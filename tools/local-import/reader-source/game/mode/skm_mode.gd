# skm_mode.gd - Mordhau's Skirmish (BP_SkirmishGameMode : BP_MordhauGameMode, game state BP_SkirmishGameState :
# BP_MordhauGameState): team rounds, no respawn inside a round. Shared rules (native OnKilled scoring, auto-assign,
# team starts, spawn queue, ControllerCanRestart) are MordhauGameMode; this class holds the Blueprint overrides.
# Ported from the bytecode (scripts/kismet; statement indices in brackets; literals: SkmMode.SkmData.kv, "skm_*" keys):
#
# Round flow, ReceiveTick (ubergraph 2550), only while IsMatchInProgress, switch GameState RoundInfo.Stage:
#   WaitingForPlayers (0): AlreadyDiedArray.Clear [1972]; CanStartRound -> StartRoundStart [1924]
#   RoundStart (1):  now >= StartTime + RoundStartDuration (5) -> CanStartRound ? StartRound : StartWaitingForPlayers
#   RoundPlay (2):   now >= StartTime + RoundDuration (150) -> EndRound(false) [3138]; else: now >= StartTime +
#                    LateRoundSpawnDuration (10) -> bAllowSpawning = false [204]; !MoreThanOneTeamAlive -> EndRound(true)
#                    [355]; else a capture point owned by a team (OwningTeam != 255): RoundPointsPerTeam[team] += 1,
#                    UpdateStateReplicatedPoints, >= CapturePointTimeToWin -> EndRound(false) [431..884]
#   RoundEnd (3):    CanStartRound ? (now >= StartTime + RoundEndDuration (5) -> StartRoundStart) : StartWaitingForPlayers
# StartWaitingForPlayers: scoring off, AlreadyDied cleared, RoundInfo {0, 0, now}, bAllowSpawning true
# StartRoundStart: scoring off; [578] TeamSwapAtHalftime && !HasSwappedTeams && TeamScores[0] + TeamScores[1] == 6 ->
#   SwapTeams; [936] world cleanup (dropped equipment, projectiles, deployables, destroyables / doors reset: event);
#   [2024] RoundPointsPerTeam[i] = Temp_float_Variable (never written in the graph: 0), UpdateStateReplicatedPoints;
#   [2355] CapturePoint.RoundStarted; [2431] AlreadyDied cleared, RoundInfo {1, 0, now}, bAllowSpawning true, every
#   controller with Team >= 0: UnpossessAndDestroyPawn(c, RestartPlayer = true)
# StartRound: scoring on, AlreadyDied cleared, RoundInfo {2, 0, now}
# EndRound(IsWipe): wipe, or GameState RoundPointsPerTeam[0] == [1]: Winner = GetMaxIndexWithDraw(living players per
#   team) (draw -> 255); else Winner = [0] > [1] ? 0 : 1. Winner 255: RoundInfo {3, 255, now}, bAllowSpawning false.
#   Otherwise AddTeamScore(Winner, 1); TeamScores[Winner] >= WinConditionRounds (7) -> EndMatch, MatchEndInfo {null,
#   Winner, TeamScores[Winner], 0, false} (RoundInfo is left in RoundPlay); TeamSwapAtHalftime && TeamScores sum >=
#   (WinConditionRounds - 1) * 2 -> EndMatch, MatchEndInfo {null, 0, 0, 0, Draw}; else RoundInfo {3, Winner, now},
#   bAllowSpawning false. Always: scoring off.
# MoreThanOneTeamAlive: bAllowSpawning -> true; else >= 2 teams with living players (GetPlayerCountsPerTeam(true, false))
# CanStartRound: >= 2 teams with players (GetPlayerCountsPerTeam(false, true))
# OnKilled (ubergraph 915): Stage != 0 and in progress -> AlreadyDiedArray.Add(killed); a non-friendly killer gets
#   CoinsPerKill (economy: event); then the parent OnKilled (MordhauGameMode.on_killed)
# ControllerCanRestart: parent && !AlreadyDiedArray.Contains(c)  -> a dead player waits for the next round
# Game state (BP_SkirmishGameState):
#   OnRep_RoundInfo (the server calls it after every RoundInfo write): on a stage change, LastObservedRoundStage = Stage;
#     RoundStart -> destroy dead characters and fire fields (event); RoundEnd -> LastWinner != 255 && == Winner ?
#     LossStreak + 1 : 0; LastWinner = Winner; HUD "Draw!" (Winner 255) or Format("{a} has won the round.", a =
#     GetTeamName(Winner)) for RoundEndDuration s; round coins (economy: events)
#   ReceiveTick (local player, InProgress): LastObservedRoundStage 0 -> "Waiting for players" 0.1 s; 1 ->
#     FCeil(StartTime + RoundStartDuration - now) != 0 -> "Round starting" Format("-{0}-") 0.1 s
#   ShouldBlockPawnInput: parent || LastObservedRoundStage == 1
#   HandleMatchEndInfo: Team == -1 or Team == WinnerTeam -> ShowMatchResult(true, "victory") else (false, "defeat")
#   GetScoreboardTimeInProgress: stage 0 -> 200 days; 1 / 2 / 3 -> FCeil(FMax(StartTime + duration - now, 0)); else parent
# Not ported (UNCONFIRMED / out of scope): the economy (BP_EconomyPlayerState coins, StoreGear, ResetEconomy: events
# only). The capture point is a ControlPoint (control_point.gd: AControlPoint + BP_SkirmishCapturePoint lock); no
# shipped SKM map places one (no BP_SkirmishCapturePoint_C actor in any map package), so set_capture_point is for
# adapters / tests.
class_name SkmMode
extends MordhauGameMode

var sd: SkmMode.SkmData						# d, typed
var round_info := {"stage": 0, "winner": 0, "start_time": 0.0}	# GameState RoundInfo (CDO: Stage 0, Winner 0, 0)
var round_points_per_team: Array = []	# BP_SkirmishGameMode RoundPointsPerTeam (float)
var state_round_points: Array = []		# BP_SkirmishGameState RoundPointsPerTeam (bytes)
var already_died: Array = []			# AlreadyDiedArray
var capture_point = null				# BP_SkirmishCapturePoint: {owning_team} or null
var has_swapped_teams := false
var team_swap_at_halftime := false		# GameState TeamSwapAtHalftime
var last_observed_round_stage := 0		# GameState LastObservedRoundStage
var last_winner := 255					# GameState LastWinner
var loss_streak := 0					# GameState LossStreak

func _init(data: SkmMode.SkmData = null, rand: UeRand = null) -> void:
	sd = data if data != null else SkmMode.SkmData.shared()
	super(sd, rand)
	round_points_per_team = sd.round_points_per_team.duplicate()
	for v in round_points_per_team:
		state_round_points.append(0)
	team_swap_at_halftime = sd.team_swap_at_halftime
	last_winner = sd.last_winner

# the round's capture point (BP_SkirmishGameMode CapturePoint): ticked with the world's control points
func set_capture_point(cp: ControlPoint) -> void:
	capture_point = cp
	add_control_point(cp)

func stage() -> int:
	return int(round_info.stage)

func stage_name(st: int) -> String:
	for n in sd.stage:
		if sd.stage[n] == st:
			return n
	return "?"

# ---- server ----------------------------------------------------------------------------------------------------
func receive_tick(_dt: float) -> void:
	if not in_progress():
		return
	var st := stage()
	var t0 := float(round_info.start_time)
	if st == sd.stage.WaitingForPlayers:
		already_died.clear()
		if can_start_round():
			start_round_start()
	elif st == sd.stage.RoundStart:
		if now >= t0 + sd.round_start_duration:
			if can_start_round():
				start_round()
			else:
				start_waiting_for_players()
	elif st == sd.stage.RoundPlay:
		if now >= t0 + sd.round_duration:
			end_round(false)
			return
		if now >= t0 + sd.late_round_spawn_duration:
			allow_spawning = false
		if not more_than_one_team_alive():
			end_round(true)
		elif capture_point != null and int(capture_point.owning_team) != int(sd.kv("skm_cp_no_team")):
			var team := int(capture_point.owning_team)
			round_points_per_team[team] = float(round_points_per_team[team]) + float(sd.kv("skm_cp_points_per_tick"))
			update_state_replicated_points()
			if float(round_points_per_team[team]) >= sd.capture_point_time_to_win:
				end_round(false)
	elif st == sd.stage.RoundEnd:
		if can_start_round():
			if now >= t0 + sd.round_end_duration:
				start_round_start()
		else:
			start_waiting_for_players()

func _counts(args_key: String) -> Array:
	var a: Array = sd.kv(args_key)
	return player_counts_per_team(bool(a[0]), bool(a[1]))

# MoreThanOneTeamAlive
func more_than_one_team_alive() -> bool:
	if allow_spawning:
		return true
	return _teams_with_players(_counts("skm_alive_counts_args")) >= 2

# CanStartRound
func can_start_round() -> bool:
	return _teams_with_players(_counts("skm_start_counts_args")) >= 2

static func _teams_with_players(counts: Array) -> int:
	return counts.filter(func(n): return int(n) > 0).size()

# UMordhauUtilityLibrary::GetMaxIndexWithDraw 0x1623f00: [index of the first maximum, draw] (a later value equal to
# the running maximum sets draw, a larger one clears it); an empty array -> -1 (draw untouched: false here)
static func max_index_with_draw(a: Array) -> Array:
	if a.is_empty():
		return [-1, false]
	var mx: int = a[0]
	var mi := 0
	var draw := false
	for i in range(1, a.size()):
		if int(a[i]) == mx:
			draw = true
		elif int(a[i]) > mx:
			draw = false
			mx = a[i]
			mi = i
	return [mi, draw]

func set_round_info(st: int, winner: int, t: float) -> void:
	round_info = {"stage": st, "winner": winner, "start_time": t}
	_ev("round_info", round_info.duplicate())
	on_rep_round_info()

# StartWaitingForPlayers
func start_waiting_for_players() -> void:
	scoring_disabled = true
	already_died.clear()
	set_round_info(int(sd.kv("skm_wfp_stage")), 0, now)
	allow_spawning = true

# StartRoundStart (header)
func start_round_start() -> void:
	scoring_disabled = true
	_ev("store_gear")								# [0] BP_EconomyCharacter StoreGear per controller's pawn
	if team_swap_at_halftime and not has_swapped_teams \
			and float(team_scores[0]) + float(team_scores[1]) == float(sd.kv("skm_halftime_rounds")):
		swap_teams()
	_ev("cleanup_world")							# [936]
	for i in round_points_per_team.size():			# [2024]
		round_points_per_team[i] = 0.0
	update_state_replicated_points()
	if capture_point != null:						# [2355] CapturePoint.RoundStarted
		_ev("capture_point_round_started")
		if capture_point is ControlPoint:
			capture_point.round_started(now)
	already_died.clear()							# [2431]
	set_round_info(int(sd.kv("skm_round_start_stage")), 0, now)
	allow_spawning = true
	for c in controllers.duplicate():
		if c.team >= 0:
			unpossess_and_destroy_pawn(c, true)

# StartRound
func start_round() -> void:
	scoring_disabled = false
	already_died.clear()
	set_round_info(int(sd.kv("skm_play_stage")), 0, now)

# EndRound(IsWipe) (header)
func end_round(is_wipe: bool) -> void:
	var winner := 0
	var by_counts := is_wipe or int(state_round_points[0]) == int(state_round_points[1])
	if by_counts:
		var r := max_index_with_draw(_counts("skm_end_counts_args"))
		winner = int(sd.kv("skm_draw_winner")) if bool(r[1]) else int(r[0]) & 0xff
	else:
		winner = 0 if int(state_round_points[0]) > int(state_round_points[1]) else 1
	var to_round_end := true
	if not (by_counts and winner == int(sd.kv("skm_draw_winner"))):
		add_team_score(winner, float(sd.kv("skm_round_win_points")))
		if float(team_scores[winner]) >= float(sd.win_condition_rounds):
			end_match()
			_set_end_info(null, winner, float(team_scores[winner]), false)
			to_round_end = false
		elif team_swap_at_halftime and float(team_scores[0]) + float(team_scores[1]) >= \
				(float(sd.win_condition_rounds) - float(sd.kv("skm_halftime_draw_sub"))) * float(sd.kv("skm_halftime_draw_mul")):
			end_match()
			_set_end_info(null, 0, 0.0, true)
			to_round_end = false
	if to_round_end:
		set_round_info(int(sd.kv("skm_end_stage")), winner, now)
		allow_spawning = false
	scoring_disabled = true

# UpdateStateReplicatedPoints: GameState RoundPointsPerTeam = Conv_IntToByte(FFloor(points)) per team
func update_state_replicated_points() -> void:
	state_round_points.clear()
	for v in round_points_per_team:
		state_round_points.append(int(floorf(float(v))) & 0xff)

# SwapTeams: every controller with Team >= 0 to the other team (0 <-> 1), economy reset; then TeamScores swapped,
# LastWinner 255, LossStreak 0
func swap_teams() -> void:
	has_swapped_teams = true
	for c in controllers:
		if c.team >= 0:
			set_team(c, 1 if c.team == 0 else 0)
			_ev("reset_economy", {"who": c.name})
	var old := team_scores.duplicate()
	team_scores[0] = old[1]
	team_scores[1] = old[0]
	last_winner = 255
	loss_streak = 0

# OnKilled (ubergraph 915; header)
func on_killed(killer: Ctrl, killed: Ctrl, damage_type := 0, weapon := "", kick := false) -> void:
	if killed != null and stage() != int(sd.kv("skm_kill_waiting_stage")) and in_progress():
		already_died.append(killed)
		if killer != null and not is_friendly(killer, killed, false):
			_ev("give_coins", {"who": killer.name, "coins": sd.coins_per_kill})
	super(killer, killed, damage_type, weapon, kick)

# ControllerCanRestart
func controller_can_restart(c: Ctrl) -> bool:
	return super(c) and not already_died.has(c)

# ---- game state ------------------------------------------------------------------------------------------------
# OnRep_RoundInfo (header)
func on_rep_round_info() -> void:
	var st := int(round_info.stage)
	if st == last_observed_round_stage:
		return
	last_observed_round_stage = st
	if st == sd.stage.RoundStart:
		_ev("destroy_corpses")
	elif st == sd.stage.RoundEnd:
		var w := int(round_info.winner)
		if last_winner != 255 and last_winner == w:
			loss_streak += 1
		else:
			loss_streak = 0
		last_winner = w
		if w == int(sd.kv("skm_draw_winner_gs")):
			_ev("announce", {"text": String(sd.kv("skm_draw_text")), "subtext": "", "duration": sd.round_end_duration,
				"src": sd.src("skm_draw_text")})
		else:
			_ev("announce", {"text": String(sd.kv("skm_round_won_format")).replace("{a}", team_name(w)), "subtext": "",
				"duration": sd.round_end_duration, "src": sd.src("skm_round_won_format")})
		for c in controllers:
			if c.team == 0 or c.team == 1:
				var coins: int = sd.coins_per_round_win if (c.team & 0xff) == w else \
					sd.coins_per_round_loss + sd.extra_coins_per_loss_streak * mini(sd.max_loss_streak, loss_streak)
				_ev("give_coins", {"who": c.name, "coins": coins})

# ReceiveTick on the local machine (header)
func client_tick(_me: Ctrl) -> Array:
	if not in_progress():
		return []
	if last_observed_round_stage == int(sd.kv("skm_hud_waiting_stage")):
		var v: Array = sd.kv("skm_waiting")
		return [{"kind": "announce", "text": String(v[0]), "subtext": "", "duration": float(v[1]), "src": sd.src("skm_waiting")}]
	if last_observed_round_stage == int(sd.kv("skm_hud_round_start_stage")):
		var n := int(ceil(float(round_info.start_time) + sd.round_start_duration - now))
		if n != 0:
			var v: Array = sd.kv("skm_round_starting")
			return [{"kind": "announce", "text": String(v[0]), "duration": float(v[1]), "src": sd.src("skm_round_starting"),
				"subtext": String(sd.kv("skm_count_format")).replace("{0}", str(n))}]
	return []

func should_block_input(c: Ctrl) -> bool:
	return super(c) or last_observed_round_stage == int(sd.kv("skm_gs_block_stage"))

func match_result_for(me: Ctrl, e: Dictionary) -> Dictionary:
	if me.team == int(sd.kv("skm_spectator_team")) or me.team == int(e.winner_team):
		return {"kind": "match_result", "victory": true, "text": String(sd.kv("skm_victory")), "subtext": ""}
	return {"kind": "match_result", "victory": false, "text": String(sd.kv("skm_defeat")), "subtext": ""}

func scoreboard_seconds_in_progress() -> int:
	var t0 := float(round_info.start_time)
	match stage():
		0: return int(sd.kv("skm_sb_no_limit_days")) * 86400
		1: return int(ceil(maxf(t0 + sd.round_start_duration - now, 0.0)))
		2: return int(ceil(maxf(t0 + sd.round_duration - now, 0.0)))
		3: return int(ceil(maxf(t0 + sd.round_end_duration - now, 0.0)))
	return super()

# ==== SkmData (the mode's constants; was skm_mode_data.gd) ========================================
# the Skirmish constants: MordhauModeData over BP_SkirmishGameMode / BP_SkirmishGameState /
# BP_SkirmishGameModeMetadata on SKM_Arena, plus the variables the Skirmish Blueprints declare (CDO values):
#   BP_SkirmishGameMode   RoundPointsPerTeam [0, 0], CapturePointTimeToWin 1, CoinsPerKill 300, CoinsPerRoundLoss 1200,
#                         CoinsPerRoundWin 2100, ExtraCoinsPerLossStreak 300, MaxLossStreak 3 (+ KillTeamScoreChange 0 /
#                         TeamKillTeamScoreChange 0 over the ctor's 10 / -10: team scores count rounds only)
#   BP_SkirmishGameState  RoundDuration 150, LateRoundSpawnDuration 10, RoundEndDuration 5, RoundStartDuration 5,
#                         WinConditionRounds 7, LastWinner 255, bUsesAutoAssign true, TeamSwapAtHalftime (not in the
#                         CDO: false); TeamCount 2 / bIsTeamMode true from BP_MordhauGameState
#   E_SkirmishRoundStage  WaitingForPlayers 0, RoundStart 1, RoundPlay 2, RoundEnd 3
#   ModeData.player_starts(SKM_Arena): 16 BP_MordhauPlayerStart_C, Team 0 x8 / Team 1 x8 (+ the Arena LevelLoadSpawn)
# Map prefix: DefaultEngine.ini:454 +GameModeMapPrefixes=(Name="SKM",GameMode=.../BP_SkirmishGameMode...).
class SkmData extends MordhauModeData:

	const SKM := GM + "Skirmish/"
	const GAME_MODE := SKM + "BP_SkirmishGameMode"
	const GAME_STATE := SKM + "BP_SkirmishGameState"
	const METADATA := SKM + "BP_SkirmishGameModeMetadata"
	const STAGE_ENUM := SKM + "E_SkirmishRoundStage"
	const MAP_JSON := ARENA + "SKM_Arena"

	# BP_SkirmishGameMode
	var round_points_per_team: Array = []
	var capture_point_time_to_win := 0.0
	var coins_per_kill := 0
	var coins_per_round_loss := 0
	var coins_per_round_win := 0
	var extra_coins_per_loss_streak := 0
	var max_loss_streak := 0
	# BP_SkirmishGameState
	var round_duration := 0.0
	var late_round_spawn_duration := 0.0
	var round_end_duration := 0.0
	var round_start_duration := 0.0
	var win_condition_rounds := 0
	var last_winner := 0
	var team_swap_at_halftime := false
	var stage := {}					# E_SkirmishRoundStage display name -> value

	static var _shared: SkmMode.SkmData

	static func shared() -> SkmMode.SkmData:
		if _shared == null:
			_shared = SkmMode.SkmData.new()
		return _shared

	func _init(map_json := MAP_JSON) -> void:
		super(map_json, GAME_MODE, GAME_STATE, METADATA)
		round_points_per_team = ModeData.cdo_floats(GAME_MODE, "RoundPointsPerTeam")
		capture_point_time_to_win = ModeData.cdo_float(GAME_MODE, "CapturePointTimeToWin")
		coins_per_kill = ModeData.cdo_int(GAME_MODE, "CoinsPerKill")
		coins_per_round_loss = ModeData.cdo_int(GAME_MODE, "CoinsPerRoundLoss")
		coins_per_round_win = ModeData.cdo_int(GAME_MODE, "CoinsPerRoundWin")
		extra_coins_per_loss_streak = ModeData.cdo_int(GAME_MODE, "ExtraCoinsPerLossStreak")
		max_loss_streak = ModeData.cdo_int(GAME_MODE, "MaxLossStreak")
		round_duration = ModeData.cdo_float(GAME_STATE, "RoundDuration")
		late_round_spawn_duration = ModeData.cdo_float(GAME_STATE, "LateRoundSpawnDuration")
		round_end_duration = ModeData.cdo_float(GAME_STATE, "RoundEndDuration")
		round_start_duration = ModeData.cdo_float(GAME_STATE, "RoundStartDuration")
		win_condition_rounds = ModeData.cdo_int(GAME_STATE, "WinConditionRounds")
		last_winner = ModeData.cdo_int(GAME_STATE, "LastWinner")
		team_swap_at_halftime = ModeData.cdo_bool_or(GAME_STATE, "TeamSwapAtHalftime", false)
		stage = ModeData.user_enum(STAGE_ENUM)
		ok = ok and stage.size() == 4 and win_condition_rounds > 0 and round_duration > 0.0 \
			and round_points_per_team.size() == state.team_count and state.b_is_team_mode

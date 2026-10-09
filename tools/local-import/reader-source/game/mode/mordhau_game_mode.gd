# mordhau_game_mode.gd - the base every game mode port extends, as every mode Blueprint extends BP_MordhauGameMode :
# AMordhauGameMode (native) : AGameMode. Engine-agnostic: plain data + tick(dt); the scene adapter (MordhauMatch and
# its subclasses, or a server) logs controllers in, reports deaths / damage / pawn locations, and acts on the events
# that come out (spawn a pawn at a start, possess, destroy, kill feed, match end). The game state's server-side fields
# (TeamScores, ElapsedTime, MatchEndInfo, bAllowSpawning) live here too: the port keeps GameMode + GameState together.
#
# Class tree (extract/decomp Blueprint parents):
#   MordhauGameMode   AMordhauGameMode (extract/native/decomp/AMordhauGameMode.cpp) + BP_MordhauGameMode
#     FfaMode         BP_DeathmatchGameMode       : BP_MordhauGameMode
#     TdmMode         BP_TeamDeathmatchGameMode   : BP_MordhauGameMode
#     SkmMode         BP_SkirmishGameMode         : BP_MordhauGameMode
#     DuelMode        BP_DuelGameMode             : BP_MordhauGameMode
#       TfMode        BP_Group3v3GameMode         : BP_DuelGameMode
# A subclass overrides exactly the functions its Blueprint overrides (k2_post_login, receive_tick, on_killed,
# controller_can_restart, on_score_changed, on_team_score_changed, match_time_ran_out_, should_block_input, ...).
#
# Native sources (constants: MordhauModeData / ModeConstants):
#   tick          AMordhauGameMode::Tick rva=0x15ab000: Super::Tick (AActor::Tick -> ReceiveTick = the Blueprint tick,
#                 AGameMode match state), one spawn-queue step, then InProgress with MatchDurationMax >= 1 and
#                 ElapsedTime >= MatchDurationMax -> MatchTimeRanOut
#   scoring       AMordhauGameMode::OnKilled_Implementation rva=0x159d6f0 (+ BP_MordhauGameMode OnKilled -> AddKillNotify);
#                 AMordhauPlayerState::AddScore rva=0x15c3bf0 -> OnScoreChanged; AddTeamScore 0x1582410 / SetTeamScore
#                 0x15a7960 -> OnTeamScoreChanged
#   time limit    native AMordhauGameMode::MatchTimeRanOut_Implementation 0x159d1c0 = bMatchTimeRanOut + vtable +0x830
#                 (AGameMode::EndMatch, resolved from the vftable)
#   respawn       OnKilled: NextRespawnTime = now + PlayerRespawnTime (waves: ceil(now / t) * t); bots:
#                 AMordhauAIController::Tick 0x15185b0 (bAutoRespawn, no pawn -> bWantsRespawn; NextRespawnTime < now and
#                 ControllerCanRestart -> RestartPlayer); players: AMordhauPlayerController::CanAskForSpawn 0x15c8a90
#                 (NextRespawnTime < server time, LastAskedForSpawnTime + 1 < real time or never asked) -> AskForSpawn
#   restart gate  AMordhauGameMode::ControllerCanRestart_Implementation 0x1587e50: a MordhauPlayerState with Team != -1
#                 and GameState bAllowSpawning (0x141587f2d `cmp byte ptr [rbx + 0x696], 0`); RestartPlayer
#                 (AMordhauGameMode::RestartPlayer 0x15a6d50) itself does not check it
#   spawn point   AMordhauGameMode::ChoosePlayerStart_Implementation 0x15876e0 / IsSpawnpointAllowed_Implementation 0x159c1b0 /
#                 AMordhauPlayerStart::IsAllowedSpawnFor_Implementation 0x15dee60 / GetSpawnpointPreference_Implementation 0x1599660 /
#                 AMordhauPlayerStart::GetSpawnPreferenceFor_Implementation 0x15da470
#   match start   AMordhauGameMode::ReadyToStartMatch_Implementation 0x15a42b0 (WarmupEnd -1 or <= now, NumPlayers + NumBots > 0),
#                 HandleMatchHasStarted 0x1599ee0 (bIsScoringDisabled = false); HandleMatchHasEnded 0x1599e10 sets it back
#   teams         AMordhauGameMode::RequestedAssignTeam_Implementation 0x15a6070 (parties not ported)
#   counts        AMordhauGameState::GetPlayerCountsPerTeam 0x1598050 (server: the world's controllers)
#   spawn queue   SpawnQueue (spawn_queue.gd)
# Port adaptations / not ported (UNCONFIRMED):
#   - AGameState::DefaultTimer (++ElapsedTime once a second while in progress) and the AGameMode::Tick match-state order
#     are UE 4.26 engine code (not disassembled); EndMatch -> WaitingPostMatch likewise
#   - the player's team at login: an auto-assign request (RequestedAssignTeam(-2)) as the FFA / TDM ports did; in the
#     game AMordhauPlayerController::ServerSetPartyInfo_Implementation 0x15f9630 asks when the GameState's auto-assign
#     flag is set and AMordhauGameState::BeginPlay 0x1584900 clears it on a non-dedicated server (team select screen)
#   - the player's spawn screen: the player asks for a spawn as soon as CanAskForSpawn allows (no button press)
#   - UWorld::EncroachingBlockingGeometry in the spawn preference (-10) tests only live pawns' capsules (radius 50 cm,
#     half height 96 cm, MordhauCharacter) against the start; level geometry is not tested
#   - spawn token distance (frontline tokens; arena starts serialize none) is equal for every start, so it never decides
#   - start iteration order = the map package's export order (TActorIterator order not reproduced)
#   - the bot "thanks" voice line of assists; map vote / ChangeLevel after the match (events only)
#   - ShouldBlockPawnInput's warmup branch (AMordhauGameState::ShouldBlockPawnInput_Implementation 0x15a8340): local play
#     has no warmup (WarmupEnd -1 from BeginPlay), so the base never blocks; the round modes override it
#   - GetPlayerCountsPerTeam's bOnlyWithValidProfiles flag (AMordhauPlayerController byte) is taken as set
class_name MordhauGameMode
extends RefCounted

# a controller + its MordhauPlayerState (+ the Blueprint controller fields the modes read), as far as the modes read them
class Ctrl extends RefCounted:
	var name := ""
	var is_bot := false				# an AMordhauAIController (takes the AI tick)
	# APlayerState / AMordhauPlayerState
	var player_id := 0				# APlayerState PlayerId (post_login: the session's RegisterPlayer id, else login order)
	var team := -1					# Team (-1 until assigned)
	var alive := false				# bIsAlive
	var score := 0.0				# Score (float)
	var kills := 0
	var deaths := 0
	var assists := 0
	# the pawn
	var has_pawn := false
	var location := Vector3.ZERO	# pawn location (Godot metres; the adapter keeps it current)
	var damage_history: Array = []	# AMordhauCharacter DamageHistory: [{who: Ctrl, t, damage}]
	var capture_point = null		# AMordhauCharacter CurrentCapturePoint +0xd80 (a ControlPoint)
	var capture_point_time := 0.0	# AMordhauCharacter CurrentCapturePointTime +0xd88
	# AMordhauPlayerController / AMordhauAIController
	var next_respawn_time := 0.0	# AMordhauPlayerController +0x9c0 / AMordhauAIController +0x568 NextRespawnTime
	var wants_respawn := false		# AMordhauAIController bWantsRespawn
	var last_asked_for_spawn := 0.0	# AMordhauPlayerController LastAskedForSpawnTime
	var match_id := ""				# MatchmakingMatchID ("" offline)
	# BP_DuelPlayerController
	var new_mmr := 0				# NewMMR
	var in_instance_with := []		# InInstanceWithControllers (AddControllerToRoom)
	var replicated_room_game := {}	# ReplicatedRoomGame (copied every TickRoom)
	var has_replicated := false		# HasReplicatedRoomGame
	func _init(n: String, bot := false) -> void:
		name = n
		is_bot = bot

var d: MordhauModeData
var rng: UeRand
var now := 0.0						# GetTimeSeconds (= GetServerWorldTimeSeconds: one process)
var match_state := "WaitingToStart"	# AGameMode MatchState: EnteringMap is passed at construction
var elapsed_time := 0				# AGameState ElapsedTime (int seconds)
var _timer := 0.0
var scoring_disabled := true		# bIsScoringDisabled
var match_time_ran_out := false		# bMatchTimeRanOut
var match_end_info := {}			# STRUCT_MatchEndInfo {winner, winner_team, winner_score, other_score, draw}
var allow_spawning := true			# GameState bAllowSpawning
var controllers: Array = []			# GameState PlayerArray order
var spawns := SpawnQueue.new()		# SpawnQueue / CurrentlySpawningController (spawn_queue.gd)
var events: Array = []				# {kind, t, ...} for the adapter
var team_scores: Array = []			# AMordhauGameState TeamScores (TeamCount entries, 0 at start - UNCONFIRMED init)
var control_points: Array = []		# AMordhauGameState AllCapturePoints [ControlPoint] (BeginPlay collects every AControlPoint)
var team1_capture_points := 0		# AMordhauGameState Team1CapturePoints
var team2_capture_points := 0		# AMordhauGameState Team2CapturePoints
var _next_id := 0

func _init(data: MordhauModeData, rand: UeRand = null) -> void:
	d = data
	rng = rand if rand != null else UeRand.new()
	scoring_disabled = d.scoring.b_is_scoring_disabled
	allow_spawning = d.state.b_allow_spawning
	for i in d.state.team_count:
		team_scores.append(0.0)

func _ev(kind: String, extra := {}) -> void:
	var e := {"kind": kind, "t": now}
	e.merge(extra)
	events.append(e)

func drain() -> Array:
	var e := events
	events = []
	return e

func by_name(n: String) -> Ctrl:
	for c in controllers:
		if c.name == n:
			return c
	return null

func in_progress() -> bool:
	return match_state == "InProgress"

# ---- login / logout ------------------------------------------------------------------------------------------
# AGameModeBase::PostLogin (PlayerArray) then the Blueprint K2_PostLogin. PlayerId: the one the game session gave the
# player state at login (AGameSession::RegisterPlayer rva=0x30e7fe0, NextPlayerID++; NetGameSession.register_player)
# when a session exists (session_player_id >= 0); offline there is no session object, so the mode keeps the same
# login-order counter itself
func post_login(c: Ctrl, session_player_id := -1) -> void:
	if session_player_id >= 0:
		c.player_id = session_player_id
	else:
		c.player_id = _next_id
		_next_id += 1
	controllers.append(c)
	if c.is_bot:
		c.wants_respawn = d.bot_auto_respawn
	k2_post_login(c)

# overridden by BP_DuelGameMode K2_PostLogin; otherwise the player's auto-assign request (header: UNCONFIRMED)
func k2_post_login(c: Ctrl) -> void:
	if not c.is_bot:
		request_assign_team(c, -2)

func logout(c: Ctrl) -> void:
	k2_on_logout(c)
	controllers.erase(c)

# BP K2_OnLogout (BP_DuelGameMode overrides)
func k2_on_logout(_c: Ctrl) -> void:
	pass

# the adapter reports deaths and respawns that do not go through on_killed: bIsAlive (the native setter is not
# ported - UNCONFIRMED)
func set_alive(c: Ctrl, v: bool) -> void:
	c.alive = v

# ---- tick ----------------------------------------------------------------------------------------------------
func tick(dt: float) -> void:
	now += dt
	# AGameState::DefaultTimer: once a second, ++ElapsedTime while the match is in progress (engine, UNCONFIRMED)
	_timer += dt
	while _timer >= 1.0:
		_timer -= 1.0
		if in_progress():
			elapsed_time += 1
	for c in controllers.duplicate():
		if c.is_bot: _bot_tick(c)
		else: _player_tick(c)
	# every AControlPoint actor ticks (AControlPoint::Tick rva=0x1517270), then AMordhauGameState::Tick rva=0x15ab790
	# recounts the points (UpdateCapturePointData); actor tick order vs the game mode: UNCONFIRMED
	for cp in control_points:
		cp.tick(dt, now, in_progress())
	update_capture_point_data()
	receive_tick(dt)
	_match_state()
	_spawn_queue_step()
	if in_progress() and d.state.match_duration_max >= 1 and elapsed_time >= d.state.match_duration_max:
		match_time_ran_out_()

# AMordhauGameState BeginPlay rva=0x1584900 adds every AControlPoint to AllCapturePoints and calls
# UpdateCapturePointData; the point's own BeginPlay (AControlPoint::BeginPlay rva=0x14f0030) runs too. Its score
# events go to AMordhauPlayerState::AddScore (add_score).
func add_control_point(cp: ControlPoint) -> void:
	cp.add_score = add_score
	control_points.append(cp)
	cp.begin_play()
	update_capture_point_data()

# AMordhauGameState::UpdateCapturePointData rva=0x15aca70: Team1CapturePoints / Team2CapturePoints = the points that
# are not bIsHiddenPoint (+0x24a) with OwningTeam (+0x2e0) 0 / 1. The topological progress (CapturePointTopologicalOrdering,
# push mode) is not ported (UNCONFIRMED: HUD only).
func update_capture_point_data() -> void:
	team1_capture_points = 0
	team2_capture_points = 0
	for cp in control_points:
		if cp.d.b_is_hidden_point:
			continue
		if cp.owning_team == 0:
			team1_capture_points += 1
		elif cp.owning_team == 1:
			team2_capture_points += 1

# the Blueprint ReceiveTick (Duel, Skirmish); nothing in BP_MordhauGameMode
func receive_tick(_dt: float) -> void:
	pass

# AMordhauGameMode::ControllerCanRestart_Implementation 0x1587e50 (Skirmish overrides)
func controller_can_restart(c: Ctrl) -> bool:
	return controllers.has(c) and c.team != -1 and allow_spawning

# AMordhauAIController::Tick 0x15185b0: no team -> RequestedAssignTeam(BehaviorProfile DefaultTeam); then respawn
func _bot_tick(c: Ctrl) -> void:
	if c.team == -1:
		request_assign_team(c, d.bot_default_team)
	if d.bot_auto_respawn and not c.has_pawn:
		c.wants_respawn = true
	if c.wants_respawn and c.next_respawn_time < now and controller_can_restart(c) and not c.has_pawn \
			and not spawns.is_queued(c):
		c.wants_respawn = false
		restart_player(c)

# AMordhauPlayerController::CanAskForSpawn 0x15c8a90 -> AskForSpawn 0x15c4f00 -> server RestartPlayer (the spawn
# screen confirms at once: UNCONFIRMED)
func _player_tick(c: Ctrl) -> void:
	if c.has_pawn or spawns.is_queued(c):
		return
	if c.next_respawn_time < now and (c.last_asked_for_spawn + 1.0 < now or c.last_asked_for_spawn == 0.0) \
			and in_progress() and controller_can_restart(c):
		c.last_asked_for_spawn = now
		restart_player(c)

# AGameMode::Tick: WaitingToStart -> InProgress on AMordhauGameMode::ReadyToStartMatch_Implementation (0x15a42b0);
# StartMatch -> HandleMatchHasStarted 0x1599ee0
func _match_state() -> void:
	if match_state != "WaitingToStart":
		return
	var we := d.state.warmup_end
	if (we == -1.0 or we <= now) and controllers.size() > 0:
		match_state = "InProgress"
		scoring_disabled = false
		_ev("match_started")

# AGameMode::EndMatch: InProgress -> WaitingPostMatch; AMordhauGameMode::HandleMatchHasEnded 0x1599e10: scoring disabled
func end_match() -> void:
	if match_state != "InProgress":
		return
	match_state = "WaitingPostMatch"
	scoring_disabled = true
	_ev("match_ended")

# native MatchTimeRanOut: bMatchTimeRanOut, EndMatch (the Blueprints extend it)
func match_time_ran_out_() -> void:
	match_time_ran_out = true
	end_match()

func _set_end_info(winner: Ctrl, winner_team: int, score: float, draw: bool) -> void:
	match_end_info = {"winner": winner.name if winner != null else "", "winner_team": winner_team,
		"winner_score": score, "other_score": 0.0, "draw": draw}
	_ev("match_end_info", match_end_info.duplicate())

# ---- teams ---------------------------------------------------------------------------------------------------
# AMordhauGameMode::RequestedAssignTeam_Implementation 0x15a6070 (parties not ported): Team -2 = auto: keep a team
# the player already has, else start at team int(rand01 * TeamCount) (clamped to TeamCount - 1) and take the first
# team with fewer players than the current pick (PlayerArray members with Team >= 0 counted). Then, when the team is
# valid (-2 < Team < TeamCount) and differs: the pawn dies (TakeDamage 1e8, adapter event), SetTeam, Controller->Reset.
func request_assign_team(c: Ctrl, team: int) -> void:
	var tc := d.state.team_count
	if team == -2:
		team = c.team
		if team == -1:
			var counts := []
			for i in tc:
				counts.append(0)
			for o in controllers:
				if o.team >= 0 and o.team < tc:
					counts[o.team] += 1
			if tc == 0:
				team = 0
			else:
				var pick := mini(int(float(rng.rand() & 0x7fff) * ModeConstants.team_rand_scale * float(tc)), tc - 1)
				var best: int = counts[pick]
				for i in tc:
					if counts[i] < best:
						best = counts[i]
						pick = i
				team = pick
	if team > -2 and team < tc and c.team != team:
		if c.has_pawn:
			on_killed(c, c)					# the pawn takes 1e8 damage instigated by its own controller
			c.has_pawn = false
			_ev("kill_pawn", {"who": c.name})
		set_team(c, team)

# AMordhauPlayerState::SetTeam
func set_team(c: Ctrl, team: int) -> void:
	c.team = team
	_ev("team", {"who": c.name, "team": team})

# AMordhauGameMode::AddTeamScore 0x1582410: 0 <= Team < TeamScores.Num -> SetTeamScore(Team, Amount + old)
func add_team_score(team: int, amount: float) -> void:
	if team >= 0 and team < team_scores.size():
		set_team_score(team, team_scores[team] + amount)

# AMordhauGameMode::SetTeamScore 0x15a7960 -> AMordhauGameMode::OnTeamScoreChanged (Blueprint event)
func set_team_score(team: int, v: float) -> void:
	var old: float = team_scores[team]
	team_scores[team] = v
	_ev("team_score", {"team": team, "score": v, "old": old})
	on_team_score_changed(team, old)

# Blueprint events (TdmMode / FfaMode override)
func on_team_score_changed(_team: int, _old: float) -> void:
	pass

func on_score_changed(_c: Ctrl, _old: float) -> void:
	pass

# AMordhauGameState::GetPlayerCountsPerTeam 0x1598050 (server branch): per team, the controllers whose
# MordhauPlayerState has Team >= 0 (and bIsAlive when bOnlyLiving); bOnlyWithValidProfiles: header
func player_counts_per_team(only_living: bool, _only_with_valid_profiles: bool) -> Array:
	var out := []
	for i in d.state.team_count:
		out.append(0)
	for c in controllers:
		if c.team >= 0 and c.team < out.size() and (c.alive or not only_living):
			out[c.team] += 1
	return out

# AMordhauGameState::IsFriendly_Implementation 0x159bc10 (player states): same state -> bIsFriendlyIfSelf; else
# team mode and the same team (neither 255)
func is_friendly(a: Ctrl, b: Ctrl, friendly_if_self := false) -> bool:
	if a == null or b == null:
		return false
	if a == b:
		return friendly_if_self
	return d.state.b_is_team_mode and a.team == b.team and a.team != 255

# UMordhauUtilityLibrary::MordhauPlayerStateSortPredicate 0x162e7d0: Score desc, Kills desc, Deaths asc, player name
# (case-sensitive code-unit compare) asc, PlayerId asc (UMordhauUtilityLibrary::SortPlayers 0x163f310)
static func sort_players(a: Array) -> Array:
	var s := a.duplicate()
	s.sort_custom(func(x, y):
		if x.score != y.score: return x.score > y.score
		if x.kills != y.kills: return x.kills > y.kills
		if x.deaths != y.deaths: return x.deaths < y.deaths
		if x.name != y.name: return x.name < y.name
		return x.player_id < y.player_id)
	return s

# ---- scoring -------------------------------------------------------------------------------------------------
# AMordhauPlayerState::AddScore 0x15c3bf0 -> AMordhauGameMode::OnScoreChanged
func add_score(c: Ctrl, amount: int) -> void:
	var old := c.score
	c.score += float(amount)
	_ev("score", {"who": c.name, "score": c.score, "old": old})
	on_score_changed(c, old)

# AMordhauCharacter::TakeDamage 0x156e980 DamageHistory: alive pawn, an instigator, damage > 0: the instigator's
# entry gets damage added when still fresh (now <= time + 20) or replaced when stale, time = now; other stale entries
# are dropped; a new instigator is appended
func on_damage(victim: Ctrl, instigator: Ctrl, damage: float) -> void:
	if victim == null or instigator == null or damage <= 0.0 or not victim.alive:
		return
	var found := false
	var keep := []
	for e in victim.damage_history:
		var fresh: bool = now <= float(e.t) + ModeConstants.assist_window
		if e.who == instigator:
			e.damage = float(e.damage) + damage if fresh else damage
			e.t = now
			found = true
			keep.append(e)
		elif fresh:
			keep.append(e)
	victim.damage_history = keep
	if not found:
		victim.damage_history.append({"who": instigator, "t": now, "damage": damage})

# AMordhauGameMode::OnKilled_Implementation 0x159d6f0, then BP_MordhauGameMode OnKilled -> GameState AddKillNotify.
# killer: null for a death without an instigating controller. damage_type: EMordhauDamageType byte; weapon: the
# DamageAgent's display name (EquipmentName), kick: the agent is a KickWeapon. (SkmMode overrides: its Blueprint
# OnKilled runs first, then calls this parent.)
func on_killed(killer: Ctrl, killed: Ctrl, damage_type := 0, weapon := "", kick := false) -> void:
	if killed == null:
		return
	killed.alive = false
	var sc := d.scoring
	# NextRespawnTime (waves: ceil(now / t) * t)
	var nr := now
	if sc.player_respawn_time > 0.0:
		nr = now + sc.player_respawn_time if not sc.b_players_spawn_in_waves \
			else ceil(now / sc.player_respawn_time) * sc.player_respawn_time
	killed.next_respawn_time = nr
	if not scoring_disabled:
		_assists(killer, killed)
		var k := killer if killer != null else killed
		var friendly := is_friendly(k, killed, false)
		var counts_death := true
		if killer != null:
			if k == killed or friendly:				# suicide / team kill
				if k == killed:
					if sc.b_suicide_decrements_kills:
						k.kills -= 1
				elif sc.b_team_kills_decrement_killer_kills:
					k.kills -= 1
				if d.state.b_is_team_mode:
					add_team_score(k.team, sc.team_kill_team_score_change)
				add_score(k, int(sc.team_kill_score_change))
			else:									# kill
				k.kills += 1
				if d.state.b_is_team_mode:
					add_team_score(k.team, sc.kill_team_score_change)
				add_score(k, int(sc.kill_score_change))
		if friendly and not sc.b_team_kills_increment_killed_deaths:
			counts_death = false
		if counts_death:
			killed.deaths += 1
	# BP_MordhauGameMode OnKilled (ubergraph 791): killer with a player state -> AddKillNotify(killer, killed), else
	# AddKillNotify(killed, killed). AddKillNotify: DamageType 3 -> Flags 2, a KickWeapon agent -> Flags 1, else 0
	var kn: Ctrl = killer if killer != null else killed
	var flags := 0
	if damage_type == int(d.kv("kf_flag_fall_damage_type")):
		flags = int(d.kv("kf_flag_fall"))
	elif kick:
		flags = int(d.kv("kf_flag_kick"))
	_ev("kill_notify", {"killer": kn.name, "killed": killed.name, "flags": flags, "weapon": weapon})

# OnKilled's assist loop over the killed pawn's DamageHistory: an entry that is not the killer, not the killed
# controller, still fresh (now <= time + 20) and not friendly to the killed player: fraction = min(damage * 0.01, 1);
# points = RoundToInt(fraction * KillScoreChange) * AssistScoreFactor ((int)ROUND((f + f) * K + 0.5) >> 1, cvtss2si:
# round half to even); points > 0 in team mode -> AddScore(points), and damage < AssistDamageToCountAsKill -> +1
# assist, else +1 kill
func _assists(killer: Ctrl, killed: Ctrl) -> void:
	var sc := d.scoring
	for e in killed.damage_history:
		var a: Ctrl = e.who
		if a == null or a == killer or a == killed or now > float(e.t) + ModeConstants.assist_window:
			continue
		if not controllers.has(a) or is_friendly(killed, a, false):
			continue
		var frac := minf(float(e.damage) * ModeConstants.assist_damage_scale, ModeConstants.assist_fraction_max)
		var pts := float(_round_half_even(2.0 * frac * sc.kill_score_change + 0.5) >> 1) * sc.assist_score_factor
		if pts > 0.0 and d.state.b_is_team_mode:
			add_score(a, int(pts))
			if float(e.damage) < float(sc.assist_damage_to_count_as_kill):
				a.assists += 1
			else:
				a.kills += 1
			_ev("assist", {"who": a.name, "points": int(pts)})

static func _round_half_even(x: float) -> int:
	var f := floorf(x)
	var r := x - f
	if r > 0.5: return int(f) + 1
	if r < 0.5: return int(f)
	return int(f) + (int(f) & 1)

# ---- spawning ------------------------------------------------------------------------------------------------
# AMordhauGameMode::RestartPlayer 0x15a6d50 (no ControllerCanRestart check)
func restart_player(c: Ctrl) -> void:
	spawns.restart_player(c)

# BP_MordhauGameMode UnpossessAndDestroyPawn: StopDriving if in a vehicle; Pawn.K2_DestroyActor; UnPossess;
# RestartPlayer(Controller) if asked
func unpossess_and_destroy_pawn(c: Ctrl, restart: bool) -> void:
	if c.has_pawn:
		_ev("destroy_pawn", {"who": c.name})
	c.has_pawn = false
	c.alive = false
	if restart:
		restart_player(c)

func _spawn_queue_step() -> void:
	spawns.tick(func(c): return controllers.has(c), _spawn_pawn, func(kind: String, c, extra: Dictionary):
		if kind == "possess":
			c.damage_history.clear()		# a new pawn: its DamageHistory starts empty
		var e := {"who": c.name}
		e.merge(extra)
		_ev(kind, e))

# step 0 of the queue: SpawnDefaultPawnFor -> ChoosePlayerStart
func _spawn_pawn(c: Ctrl) -> Dictionary:
	var ps: ModeData.PlayerStartDef = choose_player_start(c)
	if ps == null:
		return {"start": "", "xf": Transform3D.IDENTITY, "team": c.team}
	c.location = ps.xf.origin
	return {"start": ps.name, "xf": ps.xf, "team": c.team}

# AMordhauPlayerStart::IsAllowedSpawnFor_Implementation 0x15dee60: not disabled, and Team == the player's team or
# Team == -2 (any) with a team >= 0
static func is_allowed_spawn_for(ps: ModeData.PlayerStartDef, c: Ctrl) -> bool:
	if ps.b_is_spawn_disabled:
		return false
	return c.team == ps.team or (ps.team == -2 and c.team >= 0)

# AMordhauGameMode::GetSpawnpointPreference_Implementation 0x1599660
func spawn_preference(ps: ModeData.PlayerStartDef, c: Ctrl) -> float:
	var pref := float(rng.rand() & 0x7fff) * ModeConstants.spawn_rand_scale	# GetSpawnPreferenceFor
	var prox := 0.0
	var total := 0.0
	for o in controllers:
		if not (o.alive and o.has_pawn):
			continue
		var dist_cm: float = (o.location - ps.xf.origin).length() * 100.0
		var p := maxf(ModeConstants.spawn_proximity_range - dist_cm, 0.0) * ModeConstants.spawn_proximity_scale
		if p <= 0.0:
			continue
		total += p
		if d.state.b_is_team_mode and o.team == c.team:
			prox += p
		else:
			prox -= p
	pref += clampf(prox, ModeConstants.spawn_proximity_min, ModeConstants.spawn_proximity_max) \
		- total * ModeConstants.spawn_crowd_weight
	if _encroached(ps, c):
		pref += ModeConstants.spawn_blocked_penalty
	return pref

# UWorld::EncroachingBlockingGeometry for the pawn at the start: another live pawn's capsule overlaps (header)
func _encroached(ps: ModeData.PlayerStartDef, c: Ctrl) -> bool:
	const R := MordhauCharacter.CAPSULE_RADIUS * 2.0 * MordhauCharacter.CM
	const H := MordhauCharacter.CAPSULE_HALF_HEIGHT * 2.0 * MordhauCharacter.CM
	for o in controllers:
		if o == c or not (o.alive and o.has_pawn):
			continue
		var dv: Vector3 = o.location - ps.xf.origin
		if Vector2(dv.x, dv.z).length() < R and absf(dv.y) < H:
			return true
	return false

# ChoosePlayerStart_Implementation 0x15876e0: over the allowed starts, the first one, replaced by any later one with a
# higher preference (token distance equal for all: header)
func choose_player_start(c: Ctrl) -> ModeData.PlayerStartDef:
	var best: ModeData.PlayerStartDef = null
	var best_pref := 0.0
	for ps in d.starts:
		if not is_allowed_spawn_for(ps, c):
			continue
		var p := spawn_preference(ps, c)
		if best == null or p > best_pref:
			best = ps
			best_pref = p
	return best

# ---- client-side queries (GameState) -------------------------------------------------------------------------
# AMordhauGameState::ShouldBlockPawnInput_Implementation 0x15a8340 (header: no warmup in local play)
func should_block_input(_c: Ctrl) -> bool:
	return false

# BP_MordhauGameState GetScoreboardTime(InProgress) -> GetScoreboardTimeInProgress: MatchDurationMax > 0 ->
# max(MatchDurationMax - ElapsedTime, 0) s, else 200 days; WaitingToStart: WarmupEnd - now (0 here)
func scoreboard_seconds() -> int:
	if match_state == "InProgress":
		return scoreboard_seconds_in_progress()
	return 0

# BP_MordhauGameState GetScoreboardTimeInProgress (BP_SkirmishGameState overrides)
func scoreboard_seconds_in_progress() -> int:
	if d.state.match_duration_max > 0:
		return maxi(d.state.match_duration_max - elapsed_time, 0)
	return int(d.kv("sb_time_no_limit_days")) * 86400

# AMordhauGameState::GetTeamName_Implementation 0x1599c20: 0 <= Team < TeamNames.Num -> TeamNames[Team], else "%d"
# (Team -1's literal is not read: no caller here passes -1)
func team_name(team: int) -> String:
	if team >= 0 and team < d.state.team_names.size():
		return d.state.team_names[team]
	return str(team)

# BP HandleMatchEndInfo (OnRep_MatchEndInfo) of the mode's game state, for the local player `me`: the HUD command
# {kind: "match_result", victory, text, subtext}, or {} (each mode overrides)
func match_result_for(_me: Ctrl, _e: Dictionary) -> Dictionary:
	return {}

# the game state's Blueprint ReceiveTick on the local player's machine: HUD commands (SkmMode overrides)
func client_tick(_me: Ctrl) -> Array:
	return []

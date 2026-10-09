# fl_mode.gd - Mordhau's Frontline (BP_FrontlineGameMode : BP_MordhauGameMode, game state BP_FrontlineGameState):
# two teams, team scores are tickets, capture points (ControlPoint: AControlPoint) with prerequisites and point-bound
# spawns. Shared rules (native OnKilled scoring, auto-assign, spawn queue, start choice) are MordhauGameMode; the
# capture points tick there (AMordhauGameState::UpdateCapturePointData counts them). Blueprint overrides, ported from
# the bytecode (scripts/kismet; statement indices in brackets; literals: ModeKismet "fl_*"):
#   BP_FrontlineGameState ReceiveBeginPlay [1695]: authority -> TeamScores[0] = Team1StartingTickets, [1] =
#     Team2StartingTickets (a BP_PushModeInfo actor switches to push mode: not ported)
#   BP_FrontlineGameState ReceiveTick [2339] (authority, IsMatchInProgress): Team1CapturePoints == 0 or
#     Team2CapturePoints == 0 -> (not push) [965] Team2CapturePoints == 0 ? SetTeamScore(1, 0) : SetTeamScore(0, 0);
#     else TicketDrainCounter += dt, >= TicketDrainInterval -> -= interval, DrainTickets [2884]
#   DrainTickets: the team holding fewer points loses TicketDrainAmount (equal: both lose 1 [975]); both <= 0 ->
#     EndMatch, MatchEndInfo Draw [420]; then SetTeamScore(0, max(T1, 0)), SetTeamScore(1, max(T2, 0)) [672]
#   BP_FrontlineGameMode OnKilled [2684]: parent, then the killed player's team (0 / 1): SetTeamScore(team,
#     max(TeamScores[team] - DeathTicketCost, 0))
#   OnTeamScoreChanged [1579]: TeamScores[Team] == 0 and in progress -> EndMatch; MatchEndInfo winner = the other team,
#     WinnerScore = its tickets
#   MatchTimeRanOut [10]: parent; [0] == [1] -> Draw (WinnerTeam 0); else the team with more tickets wins
#   HandleMatchEndInfo [3102] (local): Draw -> ShowMatchResult(false, "draw"); Team == WinnerTeam or Team == -1 ->
#     (true, "victory"); else (false, "defeat")
# Not ported (UNCONFIRMED / out of scope): push mode (StageEndTime, TriggerWinDelayed, PushWinDelay), objectives
# (destroyables / deliveries / pushables of BP_CapturePoint), the spawn select screen, vehicles / horses, the HUD.
class_name FlMode
extends MordhauGameMode

var fd: FlMode.FlData
var ticket_drain_counter := 0.0		# BP_FrontlineGameState TicketDrainCounter

func _init(data: FlMode.FlData = null, rand: UeRand = null) -> void:
	fd = data if data != null else FlMode.FlData.new()
	super(fd, rand)
	# ReceiveBeginPlay [1730] / [1786]
	team_scores[0] = fd.team1_starting_tickets
	team_scores[1] = fd.team2_starting_tickets
	# the placed capture points; prerequisites and spawn points resolved by actor name
	var by_name := {}
	for def in fd.control_points:
		by_name[def.name] = ControlPoint.new(def, d.kv)
	var starts := {}
	for ps in d.starts:
		starts[ps.name] = ps
	for def in fd.control_points:
		var cp: ControlPoint = by_name[def.name]
		for n in def.team1_prerequisites:
			cp.team1_prerequisites.append(by_name.get(n))
		for n in def.team2_prerequisites:
			cp.team2_prerequisites.append(by_name.get(n))
		for n in def.spawn_points:
			cp.spawn_points.append(starts.get(n))
	for def in fd.control_points:
		add_control_point(by_name[def.name])

func point(n: String) -> ControlPoint:
	for cp in control_points:
		if cp.name == n:
			return cp
	return null

# BP_FrontlineGameState ReceiveTick (header)
func receive_tick(dt: float) -> void:
	if not in_progress():
		return
	if team1_capture_points == 0 or team2_capture_points == 0:
		if team2_capture_points == 0:
			set_team_score(1, float(d.kv("fl_no_points_score")))
		else:
			set_team_score(0, float(d.kv("fl_no_points_score")))
		return
	ticket_drain_counter += dt
	if ticket_drain_counter >= fd.ticket_drain_interval:
		ticket_drain_counter -= fd.ticket_drain_interval
		drain_tickets()

# BP_FrontlineGameState DrainTickets (header)
func drain_tickets() -> void:
	var t1: float = team_scores[0]
	var t2: float = team_scores[1]
	if team1_capture_points > team2_capture_points:
		t2 -= fd.ticket_drain_amount
	elif team2_capture_points > team1_capture_points:
		t1 -= fd.ticket_drain_amount
	else:
		t1 -= float(d.kv("fl_tie_drain"))
		t2 -= float(d.kv("fl_tie_drain"))
	var floor_v := float(d.kv("fl_ticket_floor"))
	if t1 <= floor_v and t2 <= floor_v:
		end_match()
		_set_end_info(null, 0, 0.0, true)
	set_team_score(0, maxf(t1, floor_v))
	set_team_score(1, maxf(t2, floor_v))

# BP_FrontlineGameMode OnKilled (header): parent first, then the death ticket
func on_killed(killer: Ctrl, killed: Ctrl, damage_type := 0, weapon := "", kick := false) -> void:
	super(killer, killed, damage_type, weapon, kick)
	if killed == null:
		return
	if killed.team == 0 or killed.team == 1:
		set_team_score(killed.team, maxf(float(team_scores[killed.team]) - fd.death_ticket_cost,
			float(d.kv("fl_ticket_floor_on_death"))))

# OnTeamScoreChanged (header)
func on_team_score_changed(team: int, _old: float) -> void:
	if team < 0 or team > 1:
		return
	if float(team_scores[team]) == float(d.kv("fl_out_of_tickets")) and in_progress():
		end_match()
		var other := 1 if team == 0 else 0
		_set_end_info(null, other, float(team_scores[other]), false)

# MatchTimeRanOut (header)
func match_time_ran_out_() -> void:
	super()
	var s0: float = team_scores[0]
	var s1: float = team_scores[1]
	if s0 == s1:
		_set_end_info(null, 0, 0.0, true)
	elif s0 > s1:
		_set_end_info(null, 0, s0, false)
	else:
		_set_end_info(null, 1, s1, false)

func match_result_for(me: Ctrl, e: Dictionary) -> Dictionary:
	if bool(e.draw):
		return {"kind": "match_result", "victory": false, "text": String(d.kv("fl_draw")), "subtext": ""}
	if me.team == int(e.winner_team) or me.team == int(d.kv("fl_spectator_team")):
		return {"kind": "match_result", "victory": true, "text": String(d.kv("fl_victory")), "subtext": ""}
	return {"kind": "match_result", "victory": false, "text": String(d.kv("fl_defeat")), "subtext": ""}

# ==== FlData (the mode's constants; was fl_mode_data.gd) ========================================
# the Frontline constants: MordhauModeData over BP_FrontlineGameMode / BP_FrontlineGameState /
# BP_FrontlineGameModeMetadata on a Frontline map (default FL_Camp), plus the variables those Blueprints declare
# (CDO values) and the map's placed capture points:
#   BP_FrontlineGameMode   DeathTicketCost 1 (+ PlayerRespawnTime 0, bPlayersSpawnInWaves, KillTeamScoreChange 0 /
#                          TeamKillTeamScoreChange 0 through ModeData.scoring: team scores are tickets)
#   BP_FrontlineGameState  TicketDrainInterval 2, TicketDrainAmount 1, Team1StartingTickets 1000,
#                          Team2StartingTickets 1000, DefendingTeam 1, InitialStageTime 60 (push mode only)
#   ModeData.control_points(map)  every placed AControlPoint subclass (BP_CapturePoint_C, BP_TeamBaseCapturePoint_C,
#                          BP_HiddenCapturePoint_C): OwningTeam, Team1/2PrerequisitePoints, SpawnPoints
class FlData extends MordhauModeData:

	const FL := GM + "Battle/"
	const GAME_MODE := FL + "BP_FrontlineGameMode"
	const GAME_STATE := FL + "BP_FrontlineGameState"
	const METADATA := GM + "BP_FrontlineGameModeMetadata"
	const MAP_JSON := "Mordhau/Content/Mordhau/Maps/DuelCamp/FL_Camp"

	var death_ticket_cost := 0.0
	var ticket_drain_interval := 0.0
	var ticket_drain_amount := 0.0
	var team1_starting_tickets := 0.0
	var team2_starting_tickets := 0.0
	var defending_team := 0
	var initial_stage_time := 0.0
	var control_points: Array = []		# [ModeData.ControlPointDef] placed on the map

	func _init(map_json := MAP_JSON) -> void:
		super(map_json, GAME_MODE, GAME_STATE, METADATA)
		death_ticket_cost = ModeData.cdo_float(GAME_MODE, "DeathTicketCost")
		ticket_drain_interval = ModeData.cdo_float(GAME_STATE, "TicketDrainInterval")
		ticket_drain_amount = ModeData.cdo_float(GAME_STATE, "TicketDrainAmount")
		team1_starting_tickets = ModeData.cdo_float(GAME_STATE, "Team1StartingTickets")
		team2_starting_tickets = ModeData.cdo_float(GAME_STATE, "Team2StartingTickets")
		defending_team = ModeData.cdo_int(GAME_STATE, "DefendingTeam")
		initial_stage_time = ModeData.cdo_float(GAME_STATE, "InitialStageTime")
		control_points = ModeData.control_points(map_json)
		ok = ok and state.team_count == 2 and state.b_is_team_mode and ticket_drain_interval > 0.0 \
			and not control_points.is_empty() and not control_points.has(null)

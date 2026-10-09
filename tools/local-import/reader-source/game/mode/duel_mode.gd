# duel_mode.gd - Mordhau's Duel game mode (BP_DuelGameMode : BP_MordhauGameMode : AMordhauGameMode : AGameMode),
# engine-agnostic: plain data + tick(). The shared native / BP_MordhauGameMode parts (spawn queue, OnKilled scoring,
# spawn choice, ControllerCanRestart with BP_DuelGameState bAllowSpawning false, UnpossessAndDestroyPawn) are
# MordhauGameMode; this class holds what BP_DuelGameMode / BP_DuelGameState override (K2_PostLogin, K2_OnLogout,
# ReceiveTick and the room functions, ShouldBlockPawnInput). TfMode (BP_Group3v3GameMode) extends it.
#
# Ported from the Blueprint bytecode of BP_DuelGameMode (decoded with scripts/kismet; function names below are the
# Blueprint's own). Constants: DuelMode.DuelData (CDO / enum packages / bytecode literals with their statement index).
#
# The mode is room based: every pair of players gets a "room" (ST_DuelRoom) with its own game (ST_DuelRoomGame:
# Team1Wins, Team2Wins, RoundInfo = STRUCT_DuelRoundInfo {Stage, Winner, StartTime}). Round flow (TickRoom):
#   WaitingForPlayers --2 players & stats--> WaitingToStart (+10 s) --RestartRoom--> RoundStart (+5 s, input blocked)
#   --> RoundPlay (until one team has nobody alive) --AwardRoundWin--> RoundEnd (+2 s) --RestartRoom--> RoundStart ...
#   AwardRoundWin with RoundsToWin (5) wins -> FinishRoomGame -> room Terminated -> next TickRoom destroys it.
# Every tick each room's game is copied to its controllers (ReplicatedRoomGame), which is all the HUD reads.
#
# Port adaptations (UNCONFIRMED: no shipped equivalent, needed to play offline):
#   - local play takes the IsPlayInEditor() branches the Blueprint already has (HandleUnhandledPlayers handles a
#     controller with no MatchmakingMatchID; HandleNewPlayer joins any open room) instead of PlayFab matchmaking;
#   - IsRoomReadyToStart's AreStatsAvailable() (PlayFab stats) is taken as true;
#   - the bot is logged in like a player (K2_PostLogin only takes MordhauPlayerControllers, so in the shipped game
#     bots never enter a duel room);
#   - AwardDuelMMR / reward drops / kicks are recorded as events only;
#   - a player spawns when AssignTeam gives it a team (the client's AskForSpawn is gated by bAllowSpawning false, so
#     in the game the first pawn comes with RestartRoom; spawning earlier lets the player walk the room meanwhile).
class_name DuelMode
extends MordhauGameMode

var dd: DuelMode.DuelData				# d, typed
var rooms: Array = []				# ST_DuelRoom dictionaries
var unhandled: Array = []			# UnhandledControllers
var matches := {}					# Matches (PlayFab): match id -> {members: [{entity, team_id}]}; offline: empty
var time_until_map_change := 0.0
var join_allowed := true			# GameSession AllowsJoin
const LOCAL_PLAY := true			# take the Blueprint's IsPlayInEditor() branches (see header)

func _init(data: DuelMode.DuelData = null, rand: UeRand = null) -> void:
	dd = data if data != null else DuelMode.DuelData.shared()
	super(dd, rand)
	time_until_map_change = dd.time_until_map_change

# ---- login / logout ------------------------------------------------------------------------------------------
# K2_PostLogin (ubergraph 1299): Cast<MordhauPlayerController>(NewPlayer) -> UnhandledControllers.Add
func k2_post_login(c: Ctrl) -> void:
	unhandled.append(c)

# K2_OnLogout (ubergraph 1468) -> HandlePlayerLeaving
func k2_on_logout(c: Ctrl) -> void:
	handle_player_leaving(c)

# ---- tick (MordhauGameMode.tick: Super::Tick -> this ReceiveTick, the match state, one spawn-queue step) -------
# BP_DuelGameMode ReceiveTick (ubergraph 633): only while IsMatchInProgress (else the graph pops straight to return):
#   Sequence [0] HandleUnhandledPlayers; for i = 0; i <= Rooms.Length - 1; i++ (length re-read every iteration:
#   DestroyRoom removes rooms while looping) TickRoom(i) if Rooms.IsValidIndex(i)
#   [1] TimeUntilMapChange -= dt; < 0: GameSession AllowJoin(false) if it AllowsJoin; GetNumPlayers() == 0 and no
#   reserved slots: EndMatch
func receive_tick(dt: float) -> void:
	if match_state != "InProgress":
		return
	handle_unhandled_players()
	var i := 0
	while i <= rooms.size() - 1:
		if i >= 0 and i < rooms.size():
			tick_room(i)
		i += 1
	time_until_map_change -= dt
	if time_until_map_change < 0.0:
		if join_allowed:
			join_allowed = false
			_ev("disallow_join")
		if controllers.is_empty():
			match_state = "WaitingPostMatch"
			_ev("end_match")

# HandleUnhandledPlayers: a controller with a MatchmakingMatchID waits for its PlayFab match (RequestMatchmakingMatch);
# with none, only IsPlayInEditor() handles it (here: always, see header). Handled ones get HandleNewPlayer +
# ResetController and leave UnhandledControllers.
func handle_unhandled_players() -> void:
	var to_remove := []
	for c in unhandled:
		if c.match_id != "" and not matches.has(c.match_id):
			_ev("request_match", {"match_id": c.match_id})
			continue
		handle_new_player(c)
		_ev("reset_controller", {"who": c.name})
		to_remove.append(c)
	for c in to_remove:
		unhandled.erase(c)

func _new_round(stage: int, start: float, winner := 0) -> Dictionary:
	return {"stage": stage, "winner": winner, "start_time": start}

# HandleNewPlayer: join the first room that is Initializing (RoomState 0) with 0 < Controllers < MaxPeoplePerRoom and the
# same MatchID (or IsPlayInEditor); else make a room {Controllers [NewPlayer], Game {0, 0, RoundInfo {Stage 0,
# Winner 0, StartTime now + TimeToWaitForPlayers}}, MMR 0/0, RoomState 0}. Then AssignTeam(room, player).
func handle_new_player(c: Ctrl) -> void:
	var room_idx := -1
	for i in rooms.size():
		var r: Dictionary = rooms[i]
		var n: int = r.controllers.size()
		if n > 0 and n < dd.max_people_per_room and r.state == dd.room_state.Initializing:
			if r.match_id == c.match_id or LOCAL_PLAY:		# EqualEqual_StrStr(MatchID) || IsPlayInEditor()
				room_idx = i
				break
	if room_idx == -1:
		rooms.append({"controllers": [c], "game": {"team1_wins": 0, "team2_wins": 0,
			"round": _new_round(int(dd.kv("room_wait_for_players_stage")), now + dd.time_to_wait_for_players)},
			"team1_mmr": 0, "team2_mmr": 0, "state": dd.room_state.Initializing, "match_id": c.match_id})
		room_idx = rooms.size() - 1
		_ev("room_created", {"room": room_idx, "who": c.name})
	else:
		add_controller_to_room(c, room_idx)
		_ev("room_joined", {"room": room_idx, "who": c.name})
	assign_team(room_idx, c)
	restart_player(c)		# spawn on team assignment (header: UNCONFIRMED)

# AssignTeam: SetTeam(Rooms[i].Controllers.Length - 1): the room's first player is team 0, the second team 1
func assign_team(room_idx: int, c: Ctrl) -> void:
	set_team(c, rooms[room_idx].controllers.size() - 1)

# AddControllerToRoom: every member and the newcomer add each other to InInstanceWithControllers; append
func add_controller_to_room(c: Ctrl, room_idx: int) -> void:
	var arr: Array = rooms[room_idx].controllers.duplicate()
	for o in arr:
		if not c.in_instance_with.has(o): c.in_instance_with.append(o)
		if not o.in_instance_with.has(c): o.in_instance_with.append(c)
	arr.append(c)
	rooms[room_idx].controllers = arr

# TickRoom: Sequence
#   [0] RoomState == Terminated: nothing, else switch RoundInfo.Stage (below)
#   [1] copy Game to every member's ReplicatedRoomGame (+ OnRep on the server: the adapter's client view does that);
#       then RoomState == Terminated -> DestroyRoom(i, false)
func tick_room(i: int) -> void:
	var r: Dictionary = rooms[i]
	if r.state != dd.room_state.Terminated:
		var g: Dictionary = r.game
		var st: int = g.round.stage
		if st == dd.stage.WaitingForPlayers:
			if g.round.start_time < now:
				destroy_room(i, true)
				return					# goto 2925: no replication this tick
			if is_room_ready_to_start(i):
				prepare_and_start_room(i)
		elif st == dd.stage.WaitingToStart:
			if g.round.start_time < now:
				restart_room(i)
		elif st == dd.stage.RoundStart:
			if g.round.start_time < now:
				set_room_game_round(i, _new_round(int(dd.kv("play_stage")), float(dd.kv("play_start_time"))))
		elif st == dd.stage.RoundPlay:
			# Team1Living / Team2Living: members alive, by Team == 0 or not
			var t1 := 0
			var t2 := 0
			for c in r.controllers:
				if c.team == 0:
					if c.alive: t1 += 1
				elif c.alive:
					t2 += 1
			if t1 == 0 or t2 == 0:
				award_round_win(i, 0 if t1 > t2 else 1)		# SelectInt(0, 1, Team1Living > Team2Living)
		elif st == dd.stage.RoundEnd:
			if g.round.start_time < now:
				restart_room(i)
	r = rooms[i]
	for c in r.controllers:
		c.replicated_room_game = r.game.duplicate(true)
		c.has_replicated = true
	if r.state == dd.room_state.Terminated:
		destroy_room(i, false)

# IsRoomReadyToStart: Controllers.Length == MaxPeoplePerRoom and every member AreStatsAvailable (offline: true)
func is_room_ready_to_start(i: int) -> bool:
	return rooms[i].controllers.size() == dd.max_people_per_room

# PrepareAndStartRoom: ComputeTeamMMR (1000 / 1000, Succeeded false); RoomState = Ready; RoundInfo {Stage 1, 0, now + 10}
func prepare_and_start_room(i: int) -> void:
	rooms[i].team1_mmr = int(dd.kv("team1_mmr"))
	rooms[i].team2_mmr = int(dd.kv("team2_mmr"))
	rooms[i].state = int(dd.kv("prepare_room_state"))
	set_room_game_round(i, _new_round(int(dd.kv("prepare_stage")), now + float(dd.kv("prepare_wait_s"))))
	_ev("room_ready", {"room": i})

# SetRoomGameRound: Game = {Team1Wins, Team2Wins (kept), RoundInfo = NewRound}
func set_room_game_round(i: int, rnd: Dictionary) -> void:
	var g: Dictionary = rooms[i].game
	rooms[i].game = {"team1_wins": g.team1_wins, "team2_wins": g.team2_wins, "round": rnd}
	_ev("stage", {"room": i, "stage": rnd.stage, "start_time": rnd.start_time})

# RestartRoom: Sequence [0] destroy every MordhauActor owned by a member (IsAnyInstanceOwner: dropped weapons...)
# [1] UnpossessAndDestroyPawn(member, RestartPlayer = true) [2] RoundInfo {Stage 2, 0, now + 5}
func restart_room(i: int) -> void:
	_ev("destroy_owned_actors", {"room": i})
	for c in rooms[i].controllers:
		unpossess_and_destroy_pawn(c, bool(dd.kv("restart_restart_player")))
	set_room_game_round(i, _new_round(int(dd.kv("restart_stage")), now + float(dd.kv("restart_wait_s"))))

# AwardRoundWin(Winner): wins += 1 for the winner's team; RoundInfo {Stage 4, Winner, now + 2}; SetRoomGame;
# Team1Wins == RoundsToWin or Team2Wins == RoundsToWin -> FinishRoomGame(Team1Wins == RoundsToWin ? 0 : 1)
func award_round_win(i: int, winner: int) -> void:
	var g: Dictionary = rooms[i].game
	var w1: int = (g.team1_wins + (1 if winner == 0 else 0)) & 0xff		# Add_ByteByte
	var w2: int = (g.team2_wins + (1 if winner == 1 else 0)) & 0xff
	rooms[i].game = {"team1_wins": w1, "team2_wins": w2,
		"round": _new_round(int(dd.kv("end_stage")), now + float(dd.kv("end_wait_s")), winner & 0xff)}
	_ev("round_won", {"room": i, "winner": winner, "team1_wins": w1, "team2_wins": w2})
	if w1 == dd.rounds_to_win or w2 == dd.rounds_to_win:
		finish_room_game(i, 0 if w1 == dd.rounds_to_win else 1)

# FinishRoomGame(Winner): SetRoomTerminated; the member on the winning team gets AddScore(100000); AwardDuelMMR,
# NewMMR (+OnRep -> end screen), TriggerRewardDropForPlayer for both (PlayFab: events only)
func finish_room_game(i: int, winner: int) -> void:
	set_room_terminated(i)
	var won: Ctrl = null
	var lost: Ctrl = null
	for c in rooms[i].controllers:
		if c.team == winner:
			won = c
			add_score(c, int(dd.kv("winner_score")))
		else:
			lost = c
	_ev("match_finished", {"room": i, "winner_team": winner, "winner": won.name if won else "",
		"loser": lost.name if lost else ""})
	if won != null and lost != null:
		_ev("award_mmr", {"winner": won.name, "loser": lost.name})

# SetRoomTerminated: RoomState = 2
func set_room_terminated(i: int) -> void:
	rooms[i].state = int(dd.kv("terminated_room_state"))

# DestroyRoom(i, KickPlayers): SetRoomTerminated; kick members if asked; Matches.Remove(MatchID); Rooms.Remove(i)
func destroy_room(i: int, kick: bool) -> void:
	set_room_terminated(i)
	if kick:
		for c in rooms[i].controllers:
			_ev("kick", {"who": c.name})
	matches.erase(rooms[i].match_id)
	_ev("room_destroyed", {"room": i, "kick": kick})
	rooms.remove_at(i)

# HandlePlayerLeaving: drop from UnhandledControllers; find the non-terminated room holding the player; if the room
# is Ready, look for another member on the leaver's team (none in a 1v1) - if there is one, PenalizeForLeavingActiveGame
# (empty function) and remove the leaver, else FinishRoomGame(the other team); an Initializing room is destroyed (kick).
func handle_player_leaving(c: Ctrl) -> void:
	unhandled.erase(c)
	var found := -1
	for i in rooms.size():
		if rooms[i].state != dd.room_state.Terminated and rooms[i].controllers.has(c):
			found = i
			break
	if found == -1:
		return
	if rooms[found].state == dd.room_state.Ready:
		var other_team_member := false
		for o in rooms[found].controllers:
			if o != c and o.team == c.team:
				other_team_member = true
		if other_team_member:
			var arr: Array = rooms[found].controllers.duplicate()
			arr.erase(c)
			rooms[found].controllers = arr
		else:
			finish_room_game(found, 1 if c.team == 0 else 0)
	elif rooms[found].state == dd.room_state.Initializing:
		destroy_room(found, true)

# ---- queries ---------------------------------------------------------------------------------------------------
func room_of(c: Ctrl) -> int:
	for i in rooms.size():
		if rooms[i].controllers.has(c):
			return i
	return -1

# BP_DuelGameState ShouldBlockPawnInput: the local controller's ReplicatedRoomGame stage == RoundStart (2), else super
# (AMordhauGameState::ShouldBlockPawnInput_Implementation 0x15a8340: warmup, WarmupEnd == -1 here)
func should_block_input(c: Ctrl) -> bool:
	return c.has_replicated and int(c.replicated_room_game.round.stage) == int(dd.kv("gs_block_input_stage"))

func stage_name(st: int) -> String:
	for n in dd.stage:
		if dd.stage[n] == st:
			return n
	return "?"

# ==== DuelData (the mode's constants; was duel_mode_data.gd) ========================================
# the Duel constants: MordhauModeData over BP_DuelGameMode / BP_DuelGameState /
# BP_DuelGameModeMetadata on DU_Arena, plus what the Duel Blueprints declare themselves:
#   - BP_DuelGameMode variables (CDO): RoundsToWin 5, MaxPeoplePerRoom 2, TimeToWaitForPlayers 35, TimeUntilMapChange
#     7200, MapPrefixes; BP_DuelGameState: DefaultWarmupTime 0, bAllowHealthRegen false, bAllowSpawning false
#   - enum values from the UserDefinedEnum packages (E_DuelRoundStage / E_DuelRoomState: Names + DisplayNameMap)
#   - literals inside Blueprint function bodies (timers, stage numbers, announcement texts): MordhauModeData.kv
# Which map uses this mode: DU_Arena's WorldSettings has no DefaultGameMode override (extract/json .../DU_Arena.json),
# so the map prefix decides: DefaultEngine.ini [/Script/EngineSettings.GameMapsSettings]
# +GameModeMapPrefixes=(Name="DU",GameMode=/Game/Mordhau/Blueprints/GameModes/Duel/BP_DuelGameMode.BP_DuelGameMode_C)
# (extract/config/DefaultEngine.ini:458); BP_DuelGameMode's own MapPrefixes CDO array has the same DU entry.
# TfMode.TfData (BP_Group3v3GameMode : BP_DuelGameMode) passes its own packages.
class DuelData extends MordhauModeData:

	const DUEL := GM + "Duel/"
	const GAME_MODE := DUEL + "BP_DuelGameMode"
	const GAME_STATE := DUEL + "BP_DuelGameState"
	const METADATA := DUEL + "BP_DuelGameModeMetadata"
	const STAGE_ENUM := DUEL + "E_DuelRoundStage"
	const ROOM_ENUM := DUEL + "E_DuelRoomState"
	const MAP_JSON := ARENA + "DU_Arena"

	# BP_DuelGameMode CDO
	var rounds_to_win := 0				# RoundsToWin (byte) 5
	var max_people_per_room := 0		# MaxPeoplePerRoom 2
	var time_to_wait_for_players := 0.0	# TimeToWaitForPlayers 35
	var time_until_map_change := 0.0	# TimeUntilMapChange 7200
	var map_prefixes := {}				# MapPrefixes: prefix -> game mode class path
	# game state CDO (over BP_MordhauGameState, AMordhauGameState ctor 0x157e160 DefaultWarmupTime 15)
	var default_warmup_time := 0.0		# DefaultWarmupTime 0 (BP_DuelGameState)
	var team_colors: Array = []			# TeamColors (BP_MordhauGameState), [Color]
	var allow_health_regen := true		# bAllowHealthRegen false (BP_DuelGameState)
	# metadata CDO (meta.*, kept under the names the HUD / menu read)
	var mode_name := ""					# Name "Duel"
	var mode_prefix := ""				# Prefix "DU"
	var mode_description := ""
	# enums: display name -> value
	var stage := {}						# WaitingForPlayers 0, WaitingToStart 1, RoundStart 2, RoundPlay 3, RoundEnd 4
	var room_state := {}				# Initializing 0, Ready 1, Terminated 2

	static var _shared: DuelMode.DuelData

	static func shared() -> DuelMode.DuelData:
		if _shared == null:
			_shared = DuelMode.DuelData.new()
		return _shared

	func _init(mode_path := GAME_MODE, state_path := GAME_STATE, meta_path := METADATA, map_json := MAP_JSON) -> void:
		super(map_json, mode_path, state_path, meta_path)
		var gm := ModeData.game_mode(mode_path)
		rounds_to_win = gm.rounds_to_win
		max_people_per_room = gm.max_people_per_room
		time_to_wait_for_players = gm.time_to_wait_for_players
		time_until_map_change = gm.time_until_map_change
		map_prefixes = gm.map_prefixes
		var gs := ModeData.game_state(state_path)
		default_warmup_time = gs.default_warmup_time
		allow_health_regen = gs.b_allow_health_regen
		team_colors = gs.team_colors
		mode_name = meta.name
		mode_prefix = meta.prefix
		mode_description = meta.description
		stage = ModeData.user_enum(STAGE_ENUM)
		room_state = ModeData.user_enum(ROOM_ENUM)
		ok = ok and rounds_to_win > 0 and max_people_per_room > 0 and stage.size() == 5 and room_state.size() == 3

	# game mode class for a map name by prefix (UGameMapsSettings GameModeMapPrefixes; "DU_Arena" -> BP_DuelGameMode)
	func mode_for_map(map_name: String) -> String:
		var p := map_name.get_slice("_", 0)
		return String(map_prefixes.get(p, ""))

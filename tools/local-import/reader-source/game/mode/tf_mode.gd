# tf_mode.gd - Mordhau's Teamfight 3v3 (BP_Group3v3GameMode : BP_DuelGameMode, game state BP_Group3v3GameState :
# BP_MordhauGameState, player controller BP_Group3v3PlayerController : BP_DuelPlayerController). The whole room / round
# flow is the Duel's (DuelMode: rooms of MaxPeoplePerRoom, now 6, TickRoom counting each team's living members,
# RoundsToWin 5); the Blueprint overrides only:
#   CDO            MaxPeoplePerRoom 6, KillScoreChange 100, TeamKillScoreChange -100, AssistScoreFactor 1 (so kills
#                  inside a room score through the native OnKilled), bAllowSpawning false on the game state
#   AssignTeam     (ubergraph 2587) Matches.Find(MatchmakingMatchID): found -> the member whose Entity.ID is the
#                  player's PlayFab entity: SetTeam(TeamID == "FreeGuard" ? 1 : 0); not found -> IsPlayInEditor() ?
#                  SetTeam(0) : log "AssignTeam: Failed to find match"
#   FinishRoomGame (ubergraph 3146) SetRoomTerminated; every BP_Group3v3PlayerController of the room:
#                  AddScore(Team == Winner ? 100000 : 0), AwardTeamfightMMR, NewMMR (+OnRep), TriggerRewardDropForPlayer
#   ComputeTeamMMR TeamfightRankSamples / TeamfightRank stats per member (PlayFab); with no stats every GetPlayerValue
#                  fails and the team MMRs are 0 / (MaxPeoplePerRoom / 2) = 0, Succeeded false
#   PenalizeForLeavingActiveGame  PlayFab penalty + MMR award (event only)
#   ShouldBlockPawnInput (BP_Group3v3GameState) the local BP_DuelPlayerController's ReplicatedRoomGame Stage == 2
#                  (RoundStart), else super
# Port adaptations (UNCONFIRMED):
#   - local matchmaking: AssignTeam's IsPlayInEditor() branch puts every player on team 0, which would end every round
#     at once; offline play instead fills Matches with a local match whose members carry TeamID "FreeGuard" for every
#     second login (the PlayFab team split itself is not in the game files), so the Blueprint's matched-player branch
#     assigns the teams
#   - bots are logged in like players (as DuelMode); AwardTeamfightMMR / reward drops are events only
class_name TfMode
extends DuelMode

const LOCAL_MATCH := "local"

func _init(data: TfMode.TfData = null, rand: UeRand = null) -> void:
	super(data if data != null else TfMode.TfData.shared_tf(), rand)

# local matchmaking stand-in (header): every login joins one local match; every second member is FreeGuard
func post_login(c: Ctrl, session_player_id := -1) -> void:
	if not matches.has(LOCAL_MATCH):
		matches[LOCAL_MATCH] = {"members": []}
	var mem: Array = matches[LOCAL_MATCH].members
	var tid := String(dd.kv("tf_freeguard_team_id")) if mem.size() % 2 == 1 else ""
	mem.append({"entity": c.name, "team_id": tid})
	c.match_id = LOCAL_MATCH
	super(c, session_player_id)

# AssignTeam (ubergraph 2587; header)
func assign_team(_room_idx: int, c: Ctrl) -> void:
	if matches.has(c.match_id):
		for m in matches[c.match_id].members:
			if String(m.entity) == c.name:
				var fg: bool = String(m.team_id) == String(dd.kv("tf_freeguard_team_id"))
				set_team(c, int(dd.kv("tf_team_freeguard")) if fg else int(dd.kv("tf_team_other")))
				return
	elif LOCAL_PLAY:
		set_team(c, int(dd.kv("tf_pie_team")))
	else:
		_ev("log", {"text": "AssignTeam: Failed to find match %s for player %s" % [c.match_id, c.name]})

# PrepareAndStartRoom -> ComputeTeamMMR (header: no PlayFab stats -> 0 / 0); the rest is the Duel's
func prepare_and_start_room(i: int) -> void:
	super(i)
	rooms[i].team1_mmr = 0
	rooms[i].team2_mmr = 0

# FinishRoomGame (ubergraph 3146; header)
func finish_room_game(i: int, winner: int) -> void:
	set_room_terminated(i)
	for c in rooms[i].controllers:
		var won: bool = c.team == winner
		add_score(c, int(dd.kv("tf_winner_score")) if won else int(dd.kv("tf_loser_score")))
		_ev("award_mmr", {"who": c.name, "won": won})
	_ev("match_finished", {"room": i, "winner_team": winner})

# BP_Group3v3GameState ShouldBlockPawnInput (header)
func should_block_input(c: Ctrl) -> bool:
	return c.has_replicated and int(c.replicated_room_game.round.stage) == int(dd.kv("tf_gs_block_input_stage"))

# ==== TfData (the mode's constants; was tf_mode_data.gd) ========================================
# the Teamfight 3v3 constants: DuelMode.DuelData over BP_Group3v3GameMode (CDO MaxPeoplePerRoom 6,
# KillScoreChange 100, TeamKillScoreChange -100, AssistScoreFactor 1; RoundsToWin / TimeToWaitForPlayers /
# TimeUntilMapChange inherited from BP_DuelGameMode) / BP_Group3v3GameState (bAllowSpawning false, DefaultWarmupTime 0,
# bAllowHealthRegen false; TeamCount 2 / bIsTeamMode from BP_MordhauGameState) / BP_Group3v3GameModeMetadata
# (Name "Teamfight", Prefix "TF") on TF_Arena (6 MordhauPlayerStart, Team 0 x3 / Team 1 x3).
# Map prefix: DefaultEngine.ini:460 +GameModeMapPrefixes=(Name="TF",GameMode=.../BP_Group3v3GameMode...).
class TfData extends DuelMode.DuelData:

	const TF := GM + "Group3v3/"
	const TF_GAME_MODE := TF + "BP_Group3v3GameMode"
	const TF_GAME_STATE := TF + "BP_Group3v3GameState"
	const TF_METADATA := TF + "BP_Group3v3GameModeMetadata"
	const TF_MAP_JSON := ARENA + "TF_Arena"

	static var _tf: TfMode.TfData

	static func shared_tf() -> TfMode.TfData:
		if _tf == null:
			_tf = TfMode.TfData.new()
		return _tf

	func _init(map_json := TF_MAP_JSON) -> void:
		super(TF_GAME_MODE, TF_GAME_STATE, TF_METADATA, map_json)

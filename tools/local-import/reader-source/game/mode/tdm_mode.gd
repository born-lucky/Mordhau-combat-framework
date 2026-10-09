# tdm_mode.gd - Mordhau's Team Deathmatch (BP_TeamDeathmatchGameMode : BP_MordhauGameMode, game state
# BP_TeamDeathmatchGameState). Shared rules: MordhauGameMode (team mode switches on, from the game state's
# bIsTeamMode: team scores KillTeamScoreChange 10 / TeamKillTeamScoreChange -10, friendly-fire scoring, assists with
# AssistScoreFactor 1, team starts, spawn preference toward live team mates). The Blueprint overrides:
#   OnTeamScoreChanged(Team) (ubergraph 10): in progress and TeamScores[Team] >= TeamScoreToWin (1000) -> EndMatch,
#     MatchEndInfo {Winner null, WinnerTeam = Team, WinnerScore = TeamScores[Team], OtherScore 0, Draw false}
#   MatchTimeRanOut (ubergraph 703): parent (bMatchTimeRanOut, EndMatch); TeamScores[0] == TeamScores[1] -> Draw;
#     else the team with the higher score wins (WinnerTeam 0 when [0] > [1], else 1)
#   HandleMatchEndInfo (BP_TeamDeathmatchGameState): Draw -> ShowMatchResult(false, "draw", ""); the local Team ==
#     WinnerTeam or Team == -1 -> ShowMatchResult(true, "victory", ""); else ShowMatchResult(false, "defeat", "")
#   (no OnScoreChanged override: a player's score never ends the match)
class_name TdmMode
extends MordhauGameMode

func _init(data: TdmMode.TdmData = null, rand: UeRand = null) -> void:
	super(data if data != null else TdmMode.TdmData.shared_tdm(), rand)

func tdm() -> TdmMode.TdmData:
	return d as TdmMode.TdmData

func on_team_score_changed(team: int, _old: float) -> void:
	if in_progress() and float(team_scores[team]) >= tdm().team_score_to_win:
		end_match()
		_set_end_info(null, team, float(team_scores[team]), false)

func match_time_ran_out_() -> void:
	super()
	var s0: float = team_scores[0]
	var s1: float = team_scores[1]
	if s0 == s1:
		_set_end_info(null, int(d.kv("tdm_tie_winner_team")), 0.0, true)
	elif s0 > s1:
		_set_end_info(null, 0, s0, false)
	else:
		_set_end_info(null, 1, s1, false)

func match_result_for(me: Ctrl, e: Dictionary) -> Dictionary:
	if bool(e.draw):
		return {"kind": "match_result", "victory": false, "text": String(d.kv("tdm_draw")), "subtext": ""}
	if me.team == int(e.winner_team) or me.team == int(d.kv("tdm_spectator_team")):
		return {"kind": "match_result", "victory": true, "text": String(d.kv("tdm_victory")), "subtext": ""}
	return {"kind": "match_result", "victory": false, "text": String(d.kv("tdm_defeat")), "subtext": ""}

# ==== TdmData (the mode's constants; was tdm_mode_data.gd) ========================================
# the Team Deathmatch constants: MordhauModeData over BP_TeamDeathmatchGameMode /
# BP_TeamDeathmatchGameState / BP_TeamDeathmatchGameModeMetadata on TDM_Arena, plus the Blueprint's own TeamScoreToWin:
#   ModeData.scoring        PlayerRespawnTime 10 and bPlayersSpawnInWaves true (BP_TeamDeathmatchGameMode CDO),
#                           KillTeamScoreChange 10 / TeamKillTeamScoreChange -10 (AMordhauGameMode ctor), AssistScoreFactor
#                           1 (ctor; no Blueprint overrides it here)
#   TeamScoreToWin 1000     the Blueprint's own variable
#   ModeData.match_state    TeamCount 2 / bIsTeamMode true / TeamColors / TeamNames (BP_MordhauGameState),
#                           MatchDurationMax 1200 and bUsesAutoAssign true (BP_TeamDeathmatchGameState)
#   ModeData.player_starts  TDM_Arena's 16 BP_MordhauPlayerStart_C (Team 0 / 1 serialized) + the Arena LevelLoadSpawn
# Map prefix: DefaultEngine.ini:453 +GameModeMapPrefixes=(Name="TDM",GameMode=.../BP_TeamDeathmatchGameMode...).
class TdmData extends MordhauModeData:

	const GAME_MODE := GM + "BP_TeamDeathmatchGameMode"
	const GAME_STATE := GM + "BP_TeamDeathmatchGameState"
	const METADATA := GM + "BP_TeamDeathmatchGameModeMetadata"
	const MAP_JSON := ARENA + "TDM_Arena"

	var team_score_to_win := 0.0

	static var _tdm: TdmMode.TdmData

	static func shared_tdm() -> TdmMode.TdmData:
		if _tdm == null:
			_tdm = TdmMode.TdmData.new()
		return _tdm

	func _init(map_json := MAP_JSON) -> void:
		super(map_json, GAME_MODE, GAME_STATE, METADATA)
		team_score_to_win = ModeData.mode_float(GAME_MODE, "TeamScoreToWin")
		ok = ok and team_score_to_win > 0.0 and state.team_count == 2 and state.b_is_team_mode

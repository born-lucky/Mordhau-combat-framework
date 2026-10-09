# ffa_mode.gd - Mordhau's Free-For-All (BP_DeathmatchGameMode : BP_MordhauGameMode, game state BP_DeathmatchGameState).
# Everything shared (native OnKilled scoring, respawn, spawn choice, auto-assign, spawn queue, time limit) is
# MordhauGameMode; this class holds only what the Deathmatch Blueprints override:
#   OnScoreChanged (BP_DeathmatchGameMode ubergraph 462): Score >= ScoreToWin while in progress -> EndMatch,
#     MatchEndInfo {Winner = that player, WinnerTeam 0, WinnerScore, OtherScore 0, Draw false}
#   MatchTimeRanOut: parent (bMatchTimeRanOut, EndMatch), then FindWinner -> MatchEndInfo
#   FindWinner: GameState PlayerArray members with Team == 0, SortPlayers, first
#   HandleMatchEndInfo (BP_DeathmatchGameState, OnRep_MatchEndInfo): the local player state is the winner, or has
#     Team -1 (spectator) -> ShowMatchResult(true, "victory", "") else ShowMatchResult(false, "defeat",
#     Format("{a} wins the match!", a = winner name))
# Constants: FfaMode.FfaData (ScoreToWin 3000; AssistScoreFactor 0 so no assists; MatchDurationMax 1200).
class_name FfaMode
extends MordhauGameMode

func _init(data: FfaMode.FfaData = null, rand: UeRand = null) -> void:
	super(data if data != null else FfaMode.FfaData.shared(), rand)

func ffa() -> FfaMode.FfaData:
	return d as FfaMode.FfaData

func on_score_changed(c: Ctrl, _old: float) -> void:
	if c.score >= ffa().score_to_win and in_progress():
		end_match()
		_set_end_info(c, int(d.kv("dm_end_winner_team")), c.score, false)

func match_time_ran_out_() -> void:
	super()
	var w := find_winner()
	_set_end_info(w, int(d.kv("dm_end_winner_team")), w.score if w != null else 0.0, false)

# FindWinner: team 0 members of PlayerArray, sorted, first
func find_winner() -> Ctrl:
	var s := sort_players(controllers.filter(func(c): return c.team == 0))
	return s[0] if not s.is_empty() else null

func match_result_for(me: Ctrl, e: Dictionary) -> Dictionary:
	if String(e.winner) == me.name or me.team == int(d.kv("dm_spectator_team")):
		return {"kind": "match_result", "victory": true, "text": String(d.kv("dm_victory")), "subtext": ""}
	return {"kind": "match_result", "victory": false, "text": String(d.kv("dm_defeat")),
		"subtext": String(d.kv("dm_winner_format")).replace("{a}", String(e.winner))}

# ==== FfaData (the mode's constants; was ffa_mode_data.gd) ========================================
# the Free-For-All constants: MordhauModeData over BP_DeathmatchGameMode / BP_DeathmatchGameState /
# BP_DeathmatchGameModeMetadata on FFA_Arena, plus the Blueprint's own variable ScoreToWin (3000).
#   ModeData.match_state: MatchDurationMax 1200, TeamCount 1, bIsTeamMode false (BP_DeathmatchGameState)
#   ModeData.player_starts(FFA_Arena): the 14 BP_MordhauPlayerStart_C actors (Team not serialized: the
#   AMordhauPlayerStart ctor 0x15b5450 value -2 = any team) + the Arena sub-level's LevelLoadSpawn
# Which map uses this mode: FFA_Arena's WorldSettings has no game mode override, so the prefix decides:
# DefaultEngine.ini:452 +GameModeMapPrefixes=(Name="FFA",GameMode=.../BP_DeathmatchGameMode.BP_DeathmatchGameMode_C)
# (and BP_DuelGameMode's MapPrefixes CDO array has the same FFA entry).
class FfaData extends MordhauModeData:

	const GAME_MODE := GM + "BP_DeathmatchGameMode"
	const GAME_STATE := GM + "BP_DeathmatchGameState"
	const METADATA := GM + "BP_DeathmatchGameModeMetadata"
	const MAP_JSON := ARENA + "FFA_Arena"

	var score_to_win := 0.0			# BP_DeathmatchGameMode ScoreToWin

	static var _shared: FfaMode.FfaData

	static func shared() -> FfaMode.FfaData:
		if _shared == null:
			_shared = FfaMode.FfaData.new()
		return _shared

	func _init(map_json := MAP_JSON) -> void:
		super(map_json, GAME_MODE, GAME_STATE, METADATA)
		score_to_win = ModeData.score_to_win(GAME_MODE)
		ok = ok and score_to_win > 0.0

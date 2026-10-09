# mordhau_mode_data.gd - the constants every game mode reads, shared by all modes the way AMordhauGameMode /
# AMordhauGameState hold them for every Blueprint mode. Read through the components/ue readers (typed records only,
# tests/test_boundary.gd):
#   ModeData.scoring(<game mode>)         AMordhauGameMode ctor rva=0x157a930 + the Blueprint chain (BP_MordhauGameMode
#                                         KillScoreChange 100 / TeamKillScoreChange -100, each mode's own CDO over it)
#   ModeData.match_state(<game state>)    AMordhauGameState ctor rva=0x157e160 + chain: MatchDurationMax, TeamCount,
#                                         bIsTeamMode, WarmupEnd, bAllowSpawning, TeamNames / TeamColors, bUsesAutoAssign
#   ModeData.metadata(<metadata>)         UGameModeMetadata CDO: Name, Prefix
#   ModeData.player_starts(<map>)         the map's AMordhauPlayerStart actors (Team, bIsSpawnDisabled, transform)
#   ModeData.ai_auto_respawn / bot_default_team   AMordhauAIController bAutoRespawn, UBotBehaviorProfile DefaultTeam
#   ModeKismet                            Blueprint bytecode literals ({value, src})
# Each mode's data class (FfaMode.FfaData, TdmMode.TdmData, DuelMode.DuelData, SkmMode.SkmData, TfMode.TfData) adds only the variables its
# own Blueprint declares (ScoreToWin, TeamScoreToWin, RoundsToWin, ...).
class_name MordhauModeData
extends RefCounted

const GM := "Mordhau/Content/Mordhau/Blueprints/GameModes/"
const AI_CONTROLLER := "Mordhau/Content/Mordhau/AI/BP_MordhauAIController"
const ARENA := "Mordhau/Content/Mordhau/Maps/Arena_Map/"

var game_mode := ""
var game_state := ""
var map_json := ""
var scoring: ModeData.ScoringDef
var state: ModeData.MatchStateDef
var meta: ModeData.Metadata
var starts: Array = []			# [ModeData.PlayerStartDef]
var bot_auto_respawn := false
var bot_default_team := -1		# UBotBehaviorProfile DefaultTeam (-2: auto-assign)
var ok := false

func _init(map_path: String, mode_path: String, state_path: String, meta_path: String) -> void:
	game_mode = mode_path
	game_state = state_path
	map_json = map_path
	scoring = ModeData.scoring(mode_path)
	state = ModeData.match_state(state_path)
	meta = ModeData.metadata(meta_path)
	starts = ModeData.player_starts(map_path)
	bot_auto_respawn = ModeData.ai_auto_respawn(AI_CONTROLLER)
	bot_default_team = ModeData.bot_default_team()
	ok = scoring != null and state != null and meta != null and not starts.is_empty() and ModeKismet.ok()

# bytecode literal (ModeKismet); a missing key is an error, not a default
func kv(key: String):
	return ModeKismet.kv(key)

func src(key: String) -> String:
	return ModeKismet.src(key)

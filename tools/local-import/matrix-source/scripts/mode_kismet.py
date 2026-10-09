# mode_kismet.py - constants of the Duel game mode / HUD that live only in Blueprint bytecode (literal floats, ints,
# FText literals inside function bodies), extracted from the cooked packages so godot/game/mode and godot/game/ui
# read them instead of typing them in (docs/DESIGN.md 1).
#
# `mdx json` / `mdx decomp` run with ReadScriptData off, so extract/json and extract/decomp have no function bodies;
# the bodies are decoded here from the raw .uasset/.uexp (scripts/kismet: Python port of CUE4Parse FKismetArchive).
#
# usage: python scripts/mode_kismet.py            -> godot/data_gen/mode/mode_kismet.json
#   (fetches the raw packages it needs into extract/raw with `sh scripts/mdx.sh raw` when missing)
# Every value is found by a pattern inside one named function; a pattern that does not match exactly as expected is
# an error (the build of this file fails rather than guessing). Each JSON entry carries its source:
#   "src": "<package>:<function>@<in-memory statement index>"  (the index EX_Jump offsets use; see scripts/kismet)
import json
import os
import re
import struct
import subprocess
import sys

R = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(R, 'scripts', 'kismet'))
import pp  # noqa: E402
from kis import K  # noqa: E402
from pkg import Pkg  # noqa: E402

RAW = os.path.join(R, 'extract', 'raw')
OUT = os.path.join(R, 'godot', 'data_gen', 'mode', 'mode_kismet.json')
GM = 'Mordhau/Content/Mordhau/Blueprints/GameModes/'
PKGS = {
    'DuelGameMode': GM + 'Duel/BP_DuelGameMode',
    'DuelGameState': GM + 'Duel/BP_DuelGameState',
    'DuelPC': GM + 'Duel/BP_DuelPlayerController',
    'MordhauGameMode': GM + 'BP_MordhauGameMode',
    'MordhauHUD': 'Mordhau/Content/Mordhau/Blueprints/BP_MordhauHUD',
    'StatusBar': 'Mordhau/Content/Mordhau/UI/BP_StatusBar',
    'Announcement': 'Mordhau/Content/Mordhau/UI/BP_Announcement',
    'VictoryPopup': 'Mordhau/Content/Mordhau/UI/BP_VictoryPopup',
    'DefeatPopup': 'Mordhau/Content/Mordhau/UI/BP_DefeatPopup',
    'RoundWins': 'Mordhau/Content/Mordhau/UI/BP_DuelRoundWinsWidget',
    'ScoreWidget': GM + 'Group3v3/BP_TeamfightScoreWidget',
    'DmGameMode': GM + 'BP_DeathmatchGameMode',
    'DmGameState': GM + 'BP_DeathmatchGameState',
    'TdmGameMode': GM + 'BP_TeamDeathmatchGameMode',
    'TdmGameState': GM + 'BP_TeamDeathmatchGameState',
    'MordhauGameState': GM + 'BP_MordhauGameState',
    'KillFeed': 'Mordhau/Content/Mordhau/UI/BP_KillFeed',
    'LocalPlay': 'Mordhau/Content/Mordhau/UI/BP_LocalPlay',
    'SkmGameMode': GM + 'Skirmish/BP_SkirmishGameMode',
    'SkmGameState': GM + 'Skirmish/BP_SkirmishGameState',
    'TfGameMode': GM + 'Group3v3/BP_Group3v3GameMode',
    'TfGameState': GM + 'Group3v3/BP_Group3v3GameState',
    'FlGameMode': GM + 'Battle/BP_FrontlineGameMode',
    'FlGameState': GM + 'Battle/BP_FrontlineGameState',
    'SkmCapturePoint': GM + 'Skirmish/BP_SkirmishCapturePoint',
    'AiBackOff': 'Mordhau/Content/Mordhau/AI/Tasks/BTTask_BackOff',
    'AiFindRandom': 'Mordhau/Content/Mordhau/AI/Tasks/BTTask_FindRandomLocation',
    'AiUnstuck': 'Mordhau/Content/Mordhau/AI/Tasks/BTTask_FindUnstuckSpot',
    'AiMoveTo': 'Mordhau/Content/Mordhau/AI/Tasks/BTTask_MoveToDestination',
}

_P = pp.P


def _p2(e):
    # FText literal (EX_TextConst): print its source string instead of pp.py's placeholder "text"
    if e[0] == 0x29:
        a = e[1:-1]
        if a and a[0] == 'empty':
            return 'T("")'
        return 'T(%s)' % _P(a[0])
    return _P(e)


pp.P = _p2   # pp.P recurses through the module global, so nested expressions print the same way


def _script(pkg, e):
    # kis.script() with the other function trailers tried (FUNC_Net adds RepOffset) and no negative unpack offset
    b = pkg.data[e['off']:e['off'] + e['size']]
    for tail in (12, 14, 10, 16, 8, 18, 20):
        end = len(b) - tail
        for n in range(1, end - 4):
            st = end - n
            if struct.unpack_from('<i', b, st - 4)[0] == n and b[end - 1] == 0x53:
                k = K(pkg, b[st:end])
                k.owner = True
                out = []
                try:
                    while k.o < len(k.b):
                        out.append(k.expr())
                    return out
                except Exception:
                    continue
    raise RuntimeError('no script in ' + e['name'])


def functions(path):
    ua, ue = os.path.join(RAW, path + '.uasset'), os.path.join(RAW, path + '.uexp')
    if not (os.path.exists(ua) and os.path.exists(ue)):
        subprocess.run(['sh', os.path.join(R, 'scripts', 'mdx.sh'), 'raw', RAW, path + '.uasset', path + '.uexp'],
                       check=True)
    p = Pkg(ua, ue)
    fns = {}
    for e in p.exports:
        if p.cls_name(e) != 'Function':
            continue
        # strip the generated member-name suffixes (Stage_13_7F96...) so patterns read like the editor names
        try:
            sts = _script(p, e)
        except RuntimeError:
            continue        # a body the decoder cannot read (none of the functions in SPEC); reported if referenced
        fns[e['name']] = [(st[-1], re.sub(r'_\d+_[0-9A-F]{32}', '', pp.P(st))) for st in sts]
    return fns


NUM = r'(-?[0-9.]+(?:e-?\d+)?)'
TXT = r'T\("([^"]*)"\)'

# key: (package, function, regex with one group per value, value types, which match)
SPEC = {
    # ---- BP_DuelGameMode (server flow)
    'room_wait_for_players_stage': ('DuelGameMode', 'HandleNewPlayer', r'STRUCT_DuelRoundInfo\.Stage = (\d+)$', int, 0),
    'prepare_room_state': ('DuelGameMode', 'PrepareAndStartRoom', r'\.RoomState = (\d+)$', int, 0),
    'prepare_stage': ('DuelGameMode', 'PrepareAndStartRoom', r'STRUCT_DuelRoundInfo\.Stage = (\d+)$', int, 0),
    'prepare_wait_s': ('DuelGameMode', 'PrepareAndStartRoom',
                       r'Add_FloatFloat\(CallFunc_GetTimeSeconds_ReturnValue, ' + NUM + r'\)', float, 0),
    'restart_stage': ('DuelGameMode', 'RestartRoom', r'STRUCT_DuelRoundInfo\.Stage = (\d+)$', int, 0),
    'restart_wait_s': ('DuelGameMode', 'RestartRoom',
                       r'Add_FloatFloat\(CallFunc_GetTimeSeconds_ReturnValue, ' + NUM + r'\)', float, 0),
    'restart_restart_player': ('DuelGameMode', 'RestartRoom',
                               r'UnpossessAndDestroyPawn\(CallFunc_Array_Get_Item_1, (true|false)\)', str, 0),
    'play_stage': ('DuelGameMode', 'TickRoom', r'STRUCT_DuelRoundInfo\.Stage = (\d+)$', int, 0),
    'play_start_time': ('DuelGameMode', 'TickRoom', r'STRUCT_DuelRoundInfo\.StartTime = ' + NUM + '$', float, 0),
    'end_stage': ('DuelGameMode', 'AwardRoundWin', r'STRUCT_DuelRoundInfo\.Stage = (\d+)$', int, 0),
    'end_wait_s': ('DuelGameMode', 'AwardRoundWin',
                   r'Add_FloatFloat\(CallFunc_GetTimeSeconds_ReturnValue, ' + NUM + r'\)', float, 0),
    'terminated_room_state': ('DuelGameMode', 'SetRoomTerminated', r'\.RoomState = (\d+)$', int, 0),
    'winner_score': ('DuelGameMode', 'FinishRoomGame', r'\.AddScore\((\d+)\)', int, 0),
    'team1_mmr': ('DuelGameMode', 'ComputeTeamMMR', r'^Team1MMR = (\d+)$', int, 0),
    'team2_mmr': ('DuelGameMode', 'ComputeTeamMMR', r'^Team2MMR = (\d+)$', int, 0),
    # ---- BP_DuelPlayerController (client: countdown + announcements)
    'pc_min_time_s': ('DuelPC', 'ExecuteUbergraph_BP_DuelPlayerController',
                      r'Greater_FloatFloat\(CallFunc_GetTimeSeconds_ReturnValue_1, ' + NUM + r'\)', float, 0),
    'pc_waiting_text': ('DuelPC', 'ExecuteUbergraph_BP_DuelPlayerController',
                        r'HUD\.ShowAnnouncement\(' + TXT + r', CallFunc_Format_ReturnValue, ' + NUM, (str, float), 0),
    'pc_count_format': ('DuelPC', 'ExecuteUbergraph_BP_DuelPlayerController', r'Format\(' + TXT, str, 0),
    'pc_loadout_extra_s': ('DuelPC', 'ExecuteUbergraph_BP_DuelPlayerController',
                           r'Add_FloatFloat\(CallFunc_Add_FloatFloat_ReturnValue, ' + NUM + r'\)', float, 0),
    'pc_endscreen_delay_s': ('DuelPC', 'ExecuteUbergraph_BP_DuelPlayerController', r'Delay\(self, ' + NUM, float, 0),
    'pc_tick_sound_at': ('DuelPC', 'UpdateCountdown', r'LessEqual_IntInt\(Countdown, (\d+)\)', int, 0),
    'pc_get_ready_at': ('DuelPC', 'UpdateCountdown', r'LessEqual_IntInt\(Countdown, (\d+)\)', int, 1),
    'pc_get_ready': ('DuelPC', 'UpdateCountdown', r'HUD\.ShowAnnouncement\(' + TXT + r', CallFunc_Format_ReturnValue, '
                     + NUM, (str, float), 0),
    'pc_match_starting': ('DuelPC', 'UpdateCountdown', r'HUD\.ShowAnnouncement\(' + TXT +
                          r', CallFunc_Format_ReturnValue, ' + NUM, (str, float), 1),
    'pc_fight': ('DuelPC', 'OnRep_ReplicatedRoomGame', r'HUD\.ShowAnnouncement\(' + TXT + r', T\(""\), ' + NUM,
                 (str, float), 0),
    'pc_round_won': ('DuelPC', 'OnRep_ReplicatedRoomGame', r'HUD\.ShowAnnouncement\(' + TXT + r', T\(""\), ' + NUM,
                     (str, float), 1),
    'pc_round_lost': ('DuelPC', 'OnRep_ReplicatedRoomGame', r'HUD\.ShowAnnouncement\(' + TXT + r', T\(""\), ' + NUM,
                      (str, float), 2),
    'pc_victory': ('DuelPC', 'OnRep_ReplicatedRoomGame', r'HUD\.ShowMatchResult\(true, ' + TXT, str, 0),
    'pc_defeat': ('DuelPC', 'OnRep_ReplicatedRoomGame', r'HUD\.ShowMatchResult\(false, ' + TXT, str, 0),
    'pc_match_over_wins': ('DuelPC', 'OnRep_ReplicatedRoomGame',
                           r'EqualEqual_ByteByte\(ReplicatedRoomGame\.Team2Wins, (\d+)\)', int, 0),
    'pc_drop_stage': ('DuelPC', 'OnRep_ReplicatedRoomGame',
                      r'EqualEqual_ByteByte\(ReplicatedRoomGame\.RoundInfo\.Stage, (\d+)\)', int, 0),
    'gs_block_input_stage': ('DuelGameState', 'ShouldBlockPawnInput',
                             r'EqualEqual_ByteByte\(.*RoundInfo\.Stage, (\d+)\)', int, 0),
    # ---- BP_DeathmatchGameMode / BP_DeathmatchGameState (FFA)
    'dm_end_winner_team': ('DmGameMode', 'ExecuteUbergraph_BP_DeathmatchGameMode',
                           r'STRUCT_MatchEndInfo\.WinnerTeam = (\d+)$', int, 0),
    'dm_victory': ('DmGameState', 'ExecuteUbergraph_BP_DeathmatchGameState', r'ShowMatchResult\(true, ' + TXT, str, 0),
    'dm_defeat': ('DmGameState', 'ExecuteUbergraph_BP_DeathmatchGameState', r'ShowMatchResult\(false, ' + TXT, str, 0),
    'dm_winner_format': ('DmGameState', 'ExecuteUbergraph_BP_DeathmatchGameState', r'Format\(' + TXT, str, 0),
    'dm_spectator_team': ('DmGameState', 'ExecuteUbergraph_BP_DeathmatchGameState',
                          r'EqualEqual_IntInt\(K2Node_DynamicCast_AsMordhau_Player_State\.Team, (-?\d+)\)', int, 0),
    # ---- BP_TeamDeathmatchGameState HandleMatchEndInfo
    'tdm_draw': ('TdmGameState', 'ExecuteUbergraph_BP_TeamDeathmatchGameState', r'ShowMatchResult\(false, ' + TXT, str, 0),
    'tdm_victory': ('TdmGameState', 'ExecuteUbergraph_BP_TeamDeathmatchGameState', r'ShowMatchResult\(true, ' + TXT, str,
                    0),
    'tdm_defeat': ('TdmGameState', 'ExecuteUbergraph_BP_TeamDeathmatchGameState', r'ShowMatchResult\(false, ' + TXT,
                   str, 1),
    'tdm_spectator_team': ('TdmGameState', 'ExecuteUbergraph_BP_TeamDeathmatchGameState',
                           r'EqualEqual_IntInt\(K2Node_DynamicCast_AsMordhau_Player_State\.Team, (-?\d+)\)', int, 0),
    'tdm_tie_winner_team': ('TdmGameMode', 'ExecuteUbergraph_BP_TeamDeathmatchGameMode',
                            r'STRUCT_MatchEndInfo\.WinnerTeam = (\d+)$', int, 0),
    # ---- BP_SkirmishGameMode (server round flow)
    'skm_wfp_stage': ('SkmGameMode', 'StartWaitingForPlayers', r'STRUCT_SkirmishRoundInfo\.Stage = (\d+)$', int, 0),
    'skm_round_start_stage': ('SkmGameMode', 'StartRoundStart', r'STRUCT_SkirmishRoundInfo\.Stage = (\d+)$', int, 0),
    'skm_play_stage': ('SkmGameMode', 'StartRound', r'STRUCT_SkirmishRoundInfo\.Stage = (\d+)$', int, 0),
    'skm_end_stage': ('SkmGameMode', 'EndRound', r'STRUCT_SkirmishRoundInfo\.Stage = (\d+)$', int, 0),
    'skm_draw_winner': ('SkmGameMode', 'EndRound', r'^Winner = (\d+)$', int, 0),
    'skm_round_win_points': ('SkmGameMode', 'EndRound',
                             r'AddTeamScore\(CallFunc_Conv_ByteToInt_ReturnValue, ' + NUM + r'\)', float, 0),
    'skm_halftime_draw_sub': ('SkmGameMode', 'EndRound',
                              r'Subtract_FloatFloat\(CallFunc_Conv_IntToFloat_ReturnValue, ' + NUM + r'\)', float, 0),
    'skm_halftime_draw_mul': ('SkmGameMode', 'EndRound',
                              r'Multiply_FloatFloat\(CallFunc_Subtract_FloatFloat_ReturnValue, ' + NUM + r'\)', float, 0),
    'skm_end_counts_args': ('SkmGameMode', 'EndRound', r'GetPlayerCountsPerTeam\((true|false), (true|false)\)',
                            (bool, bool), 0),
    'skm_halftime_rounds': ('SkmGameMode', 'StartRoundStart',
                            r'EqualEqual_FloatFloat\(CallFunc_Add_FloatFloat_ReturnValue, ' + NUM + r'\)', float, 0),
    'skm_alive_counts_args': ('SkmGameMode', 'MoreThanOneTeamAlive',
                              r'GetPlayerCountsPerTeam\((true|false), (true|false)\)', (bool, bool), 0),
    'skm_start_counts_args': ('SkmGameMode', 'CanStartRound',
                              r'GetPlayerCountsPerTeam\((true|false), (true|false)\)', (bool, bool), 0),
    'skm_cp_points_per_tick': ('SkmGameMode', 'ExecuteUbergraph_BP_SkirmishGameMode',
                               r'Add_FloatFloat\(CallFunc_Array_Get_Item_1, ' + NUM + r'\)', float, 0),
    'skm_cp_no_team': ('SkmGameMode', 'ExecuteUbergraph_BP_SkirmishGameMode',
                       r'NotEqual_ByteByte\(CapturePoint\.OwningTeam, (\d+)\)', int, 0),
    'skm_kill_waiting_stage': ('SkmGameMode', 'ExecuteUbergraph_BP_SkirmishGameMode',
                               r'EqualEqual_ByteByte\(K2Node_DynamicCast_AsBP_Skirmish_Game_State_2\.RoundInfo\.Stage, '
                               r'(\d+)\)', int, 0),
    # ---- BP_SkirmishGameState (client: announcements, match result, input block, scoreboard)
    'skm_waiting': ('SkmGameState', 'ExecuteUbergraph_BP_SkirmishGameState',
                    r'ShowAnnouncement\(' + TXT + r', T\(""\), ' + NUM, (str, float), 0),
    'skm_round_starting': ('SkmGameState', 'ExecuteUbergraph_BP_SkirmishGameState',
                           r'ShowAnnouncement\(' + TXT + r', CallFunc_Format_ReturnValue, ' + NUM, (str, float), 0),
    'skm_count_format': ('SkmGameState', 'ExecuteUbergraph_BP_SkirmishGameState', r'Format\(' + TXT, str, 0),
    'skm_hud_waiting_stage': ('SkmGameState', 'ExecuteUbergraph_BP_SkirmishGameState',
                              r'NotEqual_ByteByte\(LastObservedRoundStage, (\d+)\)', int, 0),
    'skm_hud_round_start_stage': ('SkmGameState', 'ExecuteUbergraph_BP_SkirmishGameState',
                                  r'NotEqual_ByteByte\(LastObservedRoundStage, (\d+)\)', int, 1),
    'skm_victory': ('SkmGameState', 'ExecuteUbergraph_BP_SkirmishGameState', r'ShowMatchResult\(true, ' + TXT, str, 0),
    'skm_defeat': ('SkmGameState', 'ExecuteUbergraph_BP_SkirmishGameState', r'ShowMatchResult\(false, ' + TXT, str, 0),
    'skm_spectator_team': ('SkmGameState', 'ExecuteUbergraph_BP_SkirmishGameState',
                           r'EqualEqual_IntInt\(K2Node_DynamicCast_AsMordhau_Player_State\.Team, (-?\d+)\)', int, 0),
    'skm_draw_text': ('SkmGameState', 'OnRep_RoundInfo', r'ShowAnnouncement\(' + TXT + r', T\(""\), RoundEndDuration',
                      str, 0),
    'skm_round_won_format': ('SkmGameState', 'OnRep_RoundInfo', r'Format\(' + TXT, str, 0),
    'skm_draw_winner_gs': ('SkmGameState', 'OnRep_RoundInfo', r'EqualEqual_ByteByte\(RoundInfo\.Winner, (\d+)\)', int, 0),
    'skm_gs_block_stage': ('SkmGameState', 'ShouldBlockPawnInput',
                           r'EqualEqual_ByteByte\(LastObservedRoundStage, (\d+)\)', int, 0),
    'skm_sb_no_limit_days': ('SkmGameState', 'GetScoreboardTimeInProgress', r'MakeTimespan\((\d+), ', int, 0),
    'skm_objective_name': ('SkmGameState', 'GetScoreboardObjectiveName', r'^NewParam = ' + TXT, str, 0),
    # ---- BP_FrontlineGameMode / BP_FrontlineGameState (Frontline tickets; modes r6)
    'fl_ticket_floor_on_death': ('FlGameMode', 'ExecuteUbergraph_BP_FrontlineGameMode',
                                 r'FMax\(CallFunc_Subtract_FloatFloat_ReturnValue, ' + NUM + r'\)', float, 0),
    'fl_out_of_tickets': ('FlGameMode', 'ExecuteUbergraph_BP_FrontlineGameMode',
                          r'EqualEqual_FloatFloat\(CallFunc_Array_Get_Item_2, ' + NUM + r'\)', float, 0),
    'fl_no_points_score': ('FlGameState', 'ExecuteUbergraph_BP_FrontlineGameState',
                           r'\.SetTeamScore\(1, ' + NUM + r'\)$', float, 0),
    'fl_tie_drain': ('FlGameState', 'DrainTickets', r'Subtract_FloatFloat\(Team1Score, ' + NUM + r'\)', float, 0),
    'fl_ticket_floor': ('FlGameState', 'DrainTickets', r'FMax\(Team1Score, ' + NUM + r'\)', float, 0),
    'fl_draw': ('FlGameState', 'ExecuteUbergraph_BP_FrontlineGameState', r'ShowMatchResult\(false, ' + TXT, str, 0),
    'fl_victory': ('FlGameState', 'ExecuteUbergraph_BP_FrontlineGameState', r'ShowMatchResult\(true, ' + TXT, str, 0),
    'fl_defeat': ('FlGameState', 'ExecuteUbergraph_BP_FrontlineGameState', r'ShowMatchResult\(false, ' + TXT, str, 1),
    'fl_spectator_team': ('FlGameState', 'ExecuteUbergraph_BP_FrontlineGameState',
                          r'EqualEqual_IntInt\(K2Node_DynamicCast_AsMordhau_Player_State\.Team, (-?\d+)\)', int, 0),
    # ---- BP_SkirmishCapturePoint (round reset / lock, capture announcements)
    'skm_cp_round_reset': ('SkmCapturePoint', 'ExecuteUbergraph_BP_SkirmishCapturePoint',
                           r'^SetCaptureProgress\(' + NUM + r', (\d+), (true|false)\)$', (float, int, bool), 0),
    'skm_cp_announce_after_s': ('SkmCapturePoint', 'ExecuteUbergraph_BP_SkirmishCapturePoint',
                                r'Greater_FloatFloat\(CallFunc_GetTimeSeconds_ReturnValue, ' + NUM + r'\)', float, 0),
    'skm_cp_we_capturing': ('SkmCapturePoint', 'ExecuteUbergraph_BP_SkirmishCapturePoint', r'Format\(' + TXT, str, 0),
    'skm_cp_enemy_capturing': ('SkmCapturePoint', 'ExecuteUbergraph_BP_SkirmishCapturePoint', r'Format\(' + TXT, str,
                               1),
    'skm_cp_announce_s': ('SkmCapturePoint', 'ExecuteUbergraph_BP_SkirmishCapturePoint',
                          r'ShowAnnouncement\(CallFunc_Format_ReturnValue, T\(""\), ' + NUM, float, 0),
    'skm_cp_no_relevance': ('SkmCapturePoint', 'GetTeamRelevance', r'^Relevance = (\d+)$', int, 0),
    # ---- AI Blueprint tasks (game/ai/bt: BTTask_BackOff / FindRandomLocation / FindUnstuckSpot / MoveToDestination)
    'ai_backoff_motion_random': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff',
                                 r'Less_FloatFloat\(CallFunc_GetMotionBasedRandom_ReturnValue, ' + NUM + r'\)', float, 0),
    'ai_backoff_enemy_frac': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'Multiply_FloatFloat\(CallFunc_Conv_ByteToFloat_ReturnValue, ' + NUM + r'\)',
                              float, 0),
    'ai_backoff_weak_rand': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff',
                             r'Multiply_FloatFloat\(K2Node_DynamicCast_AsMordhau_AIController\.RandomFloat, ' + NUM + r'\)', float, 0),
    'ai_backoff_weak_base': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'Add_FloatFloat\(CallFunc_Multiply_FloatFloat_ReturnValue_2, ' + NUM + r'\)',
                             float, 0),
    'ai_backoff_sat_rand': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff',
                            r'Multiply_FloatFloat\(K2Node_DynamicCast_AsMordhau_AIController\.RandomFloat, ' + NUM + r'\)', float, 1),
    'ai_backoff_sat_base': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'Add_FloatFloat\(CallFunc_Multiply_FloatFloat_ReturnValue_1, ' + NUM + r'\)',
                            float, 0),
    'ai_backoff_turn_deg': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'Multiply_FloatFloat\(RightOffset, ' + NUM + r'\)', float, 0),
    'ai_backoff_turn_scale': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'Multiply_FloatFloat\(CallFunc_Multiply_FloatFloat_ReturnValue_5, ' + NUM + r'\)',
                              float, 0),
    'ai_backoff_radius': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'Multiply_VectorFloat\(CallFunc_GreaterGreater_VectorRotator_ReturnValue, ' + NUM + r'\)',
                          float, 0),
    'ai_backoff_acceptance': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'MoveToLocation\(CallFunc_Add_VectorVector_ReturnValue, ' + NUM + ',', float, 0),
    'ai_backoff_face_up': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'StartFacingActor\(CallFunc_GetClosestEnemy_ReturnValue, ' + NUM + ',', float, 0),
    'ai_backoff_side_split': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff',
                              r'Greater_FloatFloat\(K2Node_DynamicCast_AsMordhau_AIController_1\.RandomFloat, ' + NUM + r'\)', float, 0),
    'ai_backoff_normal_tol': ('AiBackOff', 'ExecuteUbergraph_BTTask_BackOff', r'Normal\(CallFunc_Subtract_VectorVector_ReturnValue, ' + NUM + r'\)', float, 0),
    'ai_find_random_radius': ('AiFindRandom', 'ExecuteUbergraph_BTTask_FindRandomLocation',
                              r'K2_GetRandomReachablePointInRadius\(self, CallFunc_GetCentroid_ReturnValue, '
                              r'CallFunc_K2_GetRandomReachablePointInRadius_RandomLocation, ' + NUM, float, 0),
    'ai_unstuck_radius': ('AiUnstuck', 'ExecuteUbergraph_BTTask_FindUnstuckSpot',
                          r'K2_GetRandomReachablePointInRadius\(self, CallFunc_K2_GetActorLocation_ReturnValue, '
                          r'CallFunc_K2_GetRandomReachablePointInRadius_RandomLocation, ' + NUM, float, 0),
    'ai_moveto_repath_dist': ('AiMoveTo', 'ExecuteUbergraph_BTTask_MoveToDestination', r'^CallFunc_Square_ReturnValue = Square\(' + NUM + r'\)$', float, 0),
    'ai_moveto_progress_dist': ('AiMoveTo', 'ExecuteUbergraph_BTTask_MoveToDestination', r'Greater_FloatFloat\(CurrentMovementDistance, ' + NUM + r'\)', float, 0),
    'ai_moveto_stuck_s': ('AiMoveTo', 'ExecuteUbergraph_BTTask_MoveToDestination', r'GreaterEqual_FloatFloat\(CallFunc_Add_FloatFloat_ReturnValue, ' + NUM + r'\)', float, 0),
    'ai_moveto_unstuck_radius': ('AiMoveTo', 'ExecuteUbergraph_BTTask_MoveToDestination', r'K2_GetRandomReachablePointInRadius\(self, '
                                 r'CallFunc_K2_GetActorLocation_ReturnValue_1, CallFunc_K2_GetRandomReachablePointInRadius_RandomLocation, '
                                 + NUM, float, 0),
    'ai_moveto_obstacle_s': ('AiMoveTo', 'ExecuteUbergraph_BTTask_MoveToDestination', r'Greater_FloatFloat\(CallFunc_Add_FloatFloat_ReturnValue, ' + NUM + r'\)', float, 0),
    'ai_moveto_sprint_full': ('AiMoveTo', 'SetPath', r'EqualEqual_ByteByte\(MordhauCharacter\.Stamina, (\d+)\)', int, 0),
    # ---- BP_Group3v3GameMode / BP_Group3v3GameState (Teamfight 3v3, over BP_DuelGameMode)
    'tf_loser_score': ('TfGameMode', 'ExecuteUbergraph_BP_Group3v3GameMode', r'^Temp_int_Variable = (\d+)$', int, 0),
    'tf_winner_score': ('TfGameMode', 'ExecuteUbergraph_BP_Group3v3GameMode', r'^Temp_int_Variable_1 = (\d+)$', int, 0),
    'tf_freeguard_team_id': ('TfGameMode', 'ExecuteUbergraph_BP_Group3v3GameMode',
                             r'EqualEqual_StrStr\(CallFunc_Array_Get_Item\.TeamID, "([^"]*)"\)', str, 0),
    'tf_team_other': ('TfGameMode', 'ExecuteUbergraph_BP_Group3v3GameMode', r'^Temp_int_Variable_2 = (\d+)$', int, 0),
    'tf_team_freeguard': ('TfGameMode', 'ExecuteUbergraph_BP_Group3v3GameMode', r'^Temp_int_Variable_3 = (\d+)$', int,
                          0),
    'tf_pie_team': ('TfGameMode', 'ExecuteUbergraph_BP_Group3v3GameMode', r'\.SetTeam\((\d+)\)$', int, 0),
    'tf_gs_block_input_stage': ('TfGameState', 'ShouldBlockPawnInput',
                                r'EqualEqual_ByteByte\(.*RoundInfo\.Stage, (\d+)\)', int, 0),
    'tf_objective_name': ('TfGameState', 'GetScoreboardObjectiveName', r'^NewParam = ' + TXT, str, 0),
    # ---- BP_MordhauGameState kill feed / scoreboard time
    'kf_flag_fall_damage_type': ('MordhauGameState', 'AddKillNotify', r'EqualEqual_ByteByte\(DamageType, (\d+)\)', int, 0),
    'kf_flag_fall': ('MordhauGameState', 'AddKillNotify', r'^Flags = (\d+)$', int, 0),
    'kf_flag_kick': ('MordhauGameState', 'AddKillNotify', r'^Flags = (\d+)$', int, 1),
    'kf_kick_text': ('MordhauGameState', 'ReceiveKillNotify', r'^KilledWith = ' + TXT, str, 0),
    'kf_fall_text': ('MordhauGameState', 'ReceiveKillNotify', r'^KilledWith = ' + TXT, str, 1),
    'kf_rip_text': ('MordhauGameState', 'ReceiveKillNotify', r'^KilledWith = ' + TXT, str, 2),
    'kf_local_color': ('MordhauGameState', 'GetKillfeedColor', r'^Color = LinearColor\{' + NUM + ', ' + NUM + ', '
                       + NUM + ', ' + NUM, (float, float, float, float), 0),
    'kf_neutral_color': ('MordhauGameState', 'GetKillfeedColor', r'^Color = LinearColor\{' + NUM + ', ' + NUM + ', '
                         + NUM + ', ' + NUM, (float, float, float, float), 1),
    'kf_team0_color': ('MordhauGameState', 'GetKillfeedColor', r'^Color = LinearColor\{' + NUM + ', ' + NUM + ', '
                       + NUM + ', ' + NUM, (float, float, float, float), 2),
    'kf_team1_color': ('MordhauGameState', 'GetKillfeedColor', r'^Color = LinearColor\{' + NUM + ', ' + NUM + ', '
                       + NUM + ', ' + NUM, (float, float, float, float), 3),
    'kf_max_entries': ('KillFeed', 'AddEntry', r'^MaxNumOfEntries = (\d+)$', int, 0),
    'kf_default_color': ('KillFeed', 'AddEntry', r'^KillerColor = LinearColor\{' + NUM + ', ' + NUM + ', ' + NUM
                         + ', ' + NUM, (float, float, float, float), 0),
    'sb_time_no_limit_days': ('MordhauGameState', 'GetScoreboardTimeInProgress', r'MakeTimespan\((\d+), ', int, 0),
    # ---- BP_LocalPlay: bot count slider (Value 0, DisplayRange 0..SelectFloat(32, 64, IsConsolePlatform))
    'lp_bot_count_default': ('LocalPlay', 'ExecuteUbergraph_BP_LocalPlay', r'S_MordhauSlider\.Value = ' + NUM + '$',
                             float, 0),
    'lp_bot_range': ('LocalPlay', 'ExecuteUbergraph_BP_LocalPlay', r'SelectFloat\(' + NUM + ', ' + NUM + ', ',
                     (float, float), 0),
    'lp_travel_players_add': ('LocalPlay', 'ExecuteUbergraph_BP_LocalPlay',
                              r'Add_IntInt\(CallFunc_Get_Discrete_Slider_Value_Discretized_Value, (\d+)\)', int, 0),
    # ---- HUD popups
    'hud_result_duration_s': ('MordhauHUD', 'ShowMatchResult', r'\.Show\(MainText, SubText, ' + NUM + r'\)', float, 0),
    'victory_delay_s': ('VictoryPopup', 'ExecuteUbergraph_BP_VictoryPopup', r'Delay\(self, ' + NUM, float, 0),
    'victory_exit': ('VictoryPopup', 'ExecuteUbergraph_BP_VictoryPopup',
                     r'CallFunc_PlayAnimation_ReturnValue_1 = PlayAnimation\((.+?), 0\.0, 1, (\d), ' + NUM,
                     (str, int, float), 0),
    'defeat_exit': ('DefeatPopup', 'ExecuteUbergraph_BP_DefeatPopup',
                    r'CallFunc_PlayAnimation_ReturnValue_1 = PlayAnimation\((.+?), 0\.0, 1, (\d), ' + NUM,
                    (str, int, float), 0),
    'defeat_delay_s': ('DefeatPopup', 'ExecuteUbergraph_BP_DefeatPopup', r'Delay\(self, ' + NUM, float, 0),
    # ---- BP_StatusBar (bar animation)
    'sb_percent_div': ('StatusBar', 'getHealthPercentage', r'Divide_FloatFloat\(DisplayedHealth, ' + NUM, float, 0),
    'sb_wait_decay': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                      r'FInterpTo_Constant\(DelayedHealthWait, 0\.0, K2Node_Event_InDeltaTime, ' + NUM, float, 0),
    'sb_stam_wait_decay': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                           r'FInterpTo_Constant\(DelayedStaminaWait, 0\.0, K2Node_Event_InDeltaTime, ' + NUM, float, 0),
    'sb_health_regen_speed': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                              r'FInterpTo\(DisplayedHealth, CallFunc_Conv_ByteToFloat_ReturnValue, '
                              r'K2Node_Event_InDeltaTime, ' + NUM, float, 0),
    'sb_stam_regen_speed': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                            r'FInterpTo\(DisplayedStamina, CallFunc_Conv_ByteToFloat_ReturnValue_1, '
                            r'K2Node_Event_InDeltaTime, ' + NUM, float, 0),
    'sb_health_delayed_speed': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                                r'FInterpTo\(DelayedHealth, CallFunc_Conv_ByteToFloat_ReturnValue, '
                                r'K2Node_Event_InDeltaTime, ' + NUM, float, 0),
    'sb_health_target_speed': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                               r'FInterpTo\(DelayedHealth, DelayedHealthTarget, K2Node_Event_InDeltaTime, ' + NUM,
                               float, 0),
    'sb_stam_delayed_speed': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                              r'FInterpTo\(DelayedStamina, DisplayedStamina, K2Node_Event_InDeltaTime, ' + NUM,
                              float, 0),
    'sb_stam_target_speed': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                             r'FInterpTo\(DelayedStamina, DelayedStaminaTarget, K2Node_Event_InDeltaTime, ' + NUM,
                             float, 0),
    'sb_full_at': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar', r'Greater_FloatFloat\(DisplayedHealth, ' + NUM,
                   float, 0),
    'sb_pulse_range': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                       r'MapRangeUnclamped\(DisplayedHealth, ' + NUM + ', ' + NUM + ', ' + NUM + ', ' + NUM,
                       (float, float, float, float), 0),
    'sb_wait_on_drop': ('StatusBar', 'SetObservedCharacter', r'^DelayedHealthWait = ' + NUM + '$', float, 0),
    'sb_stam_bar_color': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                          r'createDynamicMaterial\(obj:M_stmBar_Inst, ProgressBar_DisplayedStam, LinearColor\{'
                          + NUM + ', ' + NUM + ', ' + NUM + ', ' + NUM, (float, float, float, float), 0),
    'sb_health_bar_color': ('StatusBar', 'ExecuteUbergraph_BP_StatusBar',
                            r'createDynamicMaterial\(obj:M_stmBar_Inst1, ProgressBar_DisplayedHealth, LinearColor\{'
                            + NUM + ', ' + NUM + ', ' + NUM + ', ' + NUM, (float, float, float, float), 0),
}


# UI inputs the HUD / menu views load at runtime (godot/game/ui/umg.gd): textures as PNG via `mdx exportlist` into
# extract/gltf, the MordhauFont faces' .ufont bytes (TTF) via `mdx raw` into extract/raw. Fetched only when missing.
UI_TEX = 'Mordhau/Content/Mordhau/UI/UIAssets/Textures/'
TEXTURES = ['UI_statusBarsContainer', 'IC_health', 'IC_stamina', 'UI_ScoreElementBackground', 'UI_ScoreElementFill',
            'UI_barMask', 'ic_hourglass', 'ic_crossedAxes', 'UI_VictoryRibbon-Primary', 'UI_VictoryRibbon-Secondary',
            'UI_VictoryRibbon-Tertiary', 'UI_DefeatRibbon-Primary', 'UI_Menu_Header_01',
            'UI_Serverbrowser_JoinBtn_Unpressed', 'UI_Serverbrowser_JoinBtn_Hover', 'UI_Serverbrowser_JoinBtn_Pressed',
            'UI_Borders_Thin_04', 'UI_statusBars_PulseOverlay', 'UI_MenuRibbon_03_normal']
FONTS = ['Cinzel-Regular_new', 'Cinzel-Bold_new', 'CrimsonText-Regular_new']


def assets():
    gl = os.path.join(R, 'extract', 'gltf')
    need = [UI_TEX + t + '.0' for t in TEXTURES if not os.path.exists(os.path.join(gl, UI_TEX + t + '.png'))]
    if need:
        lst = os.path.join(R, 'state', 'mode_kismet_textures.txt')
        os.makedirs(os.path.dirname(lst), exist_ok=True)
        with open(lst, 'w') as f:
            f.write('\n'.join(need) + '\n')
        subprocess.run(['sh', os.path.join(R, 'scripts', 'mdx.sh'), 'exportlist', gl, lst], check=True)
    fp = 'Mordhau/Content/Mordhau/UI/UIAssets/Fonts/'
    need = [fp + f + '.ufont' for f in FONTS if not os.path.exists(os.path.join(RAW, fp + f + '.ufont'))]
    if need:
        subprocess.run(['sh', os.path.join(R, 'scripts', 'mdx.sh'), 'raw', RAW] + need, check=True)


def conv(t, s):
    if t is bool:
        return s == 'true'
    if t is str:
        return s
    if t is int:
        return int(s)
    return float(s)


def main():
    assets()
    fns = {k: functions(v) for k, v in PKGS.items()}
    out = {}
    for key, (pk, fn, rx, ty, nth) in SPEC.items():
        if fn not in fns[pk]:
            raise SystemExit('%s: no function %s in %s' % (key, fn, PKGS[pk]))
        hits = []
        for idx, st in fns[pk][fn]:
            for m in re.finditer(rx, st):
                hits.append((idx, m))
        if len(hits) <= nth:
            raise SystemExit('%s: pattern %r matched %d times in %s:%s' % (key, rx, len(hits), PKGS[pk], fn))
        idx, m = hits[nth]
        if isinstance(ty, tuple):
            v = [conv(t, g) for t, g in zip(ty, m.groups())]
        else:
            v = conv(ty, m.group(1))
            if v in ('true', 'false'):
                v = v == 'true'
        out[key] = {'value': v, 'src': '%s:%s@%d' % (os.path.basename(PKGS[pk]), fn, idx)}
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=1, sort_keys=True)
    print('mode_kismet: %d constants -> %s' % (len(out), OUT))


if __name__ == '__main__':
    main()

# bot_constants.gd - every literal the ported bot (game/ai/*) takes from the exe's .rdata, loaded once and given a
# name, so the bot logic reads `BotConstants.parry_lead` instead of an address (same split as CombatConstants: the
# mechanics ask a store for a named value, only this loader knows where it lives).
# Source: extract/native/rdata.tsv via UeRdata (raw IEEE-754 bytes at the va; scripts/rdata_consts.py, hash-checked exe).
# Each value is declared with `_k(name, va, functions, engine)`: `functions` are the game functions whose code loads
# that va (Ghidra `_DAT_<va>`); test_ai checks every one is listed in the rdata.tsv `funcs` column for that va and
# that the value equals the raw bytes. `engine` names statically linked UE functions (not in rdata.tsv, which indexes
# game functions only) with the disassembled instruction that loads the va (scripts/ue_dis.py). One exe literal can
# back several names (the compiler pools equal floats), so names follow the rule that uses the value.
class_name BotConstants

static var cites := {}		# name -> [va, PackedStringArray game functions, value, PackedStringArray engine cites]

static func _k(name: String, va: int, functions: Array, engine: Array = []) -> float:
	var v := UeRdata.f32(va)
	cites[name] = [va, PackedStringArray(functions), v, PackedStringArray(engine)]
	return v

const _REROLL := "UBotBehaviorProfile::RerollRandomInstanceValues"
const _RANDOMIZE := "UBotBehaviorProfile::Randomize"
const _ATTACK := "UBTTask_MeleeAttack::PerformOffensiveEvaluation"
const _DEFEND := "UBTTask_MeleeDefend::PerformDefensiveEvaluation"
const _FEINT := "UBTTask_FallForFeint::PerformFeintEvaluation"
const _VOICE := "UBTTask_VoiceOrEmote::ExecuteTask"
const _ANGLE2D := "UMordhauUtilityLibrary::CalculateAngle2D"

# ---- rand() scaling (UeRand, every caller of the CRT rand) -----------------------------------------------------
# 1/32767: FRand as the game inlines it, (rand() & 0x7fff) * (1/32767)
static var frand_scale: float = _k("frand_scale", 0x1440dfae0, [_REROLL, _RANDOMIZE, _ATTACK,
	"AMordhauAIController::GetMotionBasedRandom"])
# UBTTask_Wait::ExecuteTask rva=0x38a5d40 / UBTService::ScheduleNextTick rva=0x38b8190 (engine): RandomDeviation draw
static var bt_wait_rand_scale: float = _k("bt_wait_rand_scale", 0x1440dfae0, [],
	["UBTTask_Wait::ExecuteTask disasm 0x1438a5d99 mulss xmm0, [0x1440dfae0]"])
static var bt_service_rand_scale: float = _k("bt_service_rand_scale", 0x1440dfae0, [],
	["UBTService::ScheduleNextTick disasm 0x1438b81f0 mulss xmm2, [0x1440dfae0]"])
# VoiceOrEmote: index = min(int(rand() * 1/32767 * count), count - 1)
static var voice_pick_rand_scale: float = _k("voice_pick_rand_scale", 0x1440dfae0, [_VOICE])

# ---- UBotBehaviorProfile ---------------------------------------------------------------------------------------
# 1.0: Random2DUnitVector = draw * 2 - 1; Randomize's `1 - cube` probabilities
static var profile_one: float = _k("profile_one", 0x143fe4e0c, [_REROLL, _RANDOMIZE])
# 1e-8: Random2DUnitVector squared length at or below it -> zero vector
static var profile_unit_vector_min_sq: float = _k("profile_unit_vector_min_sq", 0x144014a88, [_REROLL])
# 3/32767: IgnoreEnemiesWithAllyCount = min(int(rand() * 3/32767), 2) + 1
static var randomize_ally_count_scale: float = _k("randomize_ally_count_scale", 0x144324750, [_RANDOMIZE])
# 1/32767^3: a cubed draw
static var randomize_cube_scale: float = _k("randomize_cube_scale", 0x144324740, [_RANDOMIZE])

# ---- AMordhauAIController --------------------------------------------------------------------------------------
# 2/32767: GetAllyClearanceSides coin flip min(int(rand() * 2/32767), 1)
static var ally_clearance_rand_scale: float = _k("ally_clearance_rand_scale", 0x14432474c,
	["AMordhauAIController::GetAllyClearanceSides"])

# ---- shared task helpers (UBTTask_* inlines) -------------------------------------------------------------------
# 1e-8: FVector::GetSafeNormal squared-length floor, inlined in every task
static var safe_normal_min_sq: float = _k("safe_normal_min_sq", 0x144014a88, [_ATTACK, _DEFEND, _FEINT])
# 15: AMordhauWeapon::Length -> reach in cm (Length x 15 x mesh scale)
static var weapon_length_to_cm: float = _k("weapon_length_to_cm", 0x14402cb98, [_ATTACK, _DEFEND])
# -1: MoveToLocation acceptance radius "use default"
static var default_acceptance: float = _k("default_acceptance", 0x144014bb8, [_ATTACK, _DEFEND])

# ---- UMordhauUtilityLibrary::CalculateAngle2D rva=0x1616170 ----------------------------------------------------
# 1e-4: every |component| at or below it -> angle 0
static var angle2d_zero_component: float = _k("angle2d_zero_component", 0x144022350, [_ANGLE2D])
static var angle2d_min_sq: float = _k("angle2d_min_sq", 0x144014a88, [_ANGLE2D])
# 57.29578: acos radians -> degrees
static var angle2d_rad_to_deg: float = _k("angle2d_rad_to_deg", 0x1442713d0, [_ANGLE2D])

# ---- UBTTask_MeleeDefend::PerformDefensiveEvaluation rva=0x14940a0 ---------------------------------------------
# 0.3: the attack counts as "in front" when -dir . my forward >= 0.3
static var defend_in_front_dot: float = _k("defend_in_front_dot", 0x1441231e4, [_DEFEND])
# -0.5: an enemy whose forward . dir < -0.5 faces away
static var defend_enemy_facing_dot: float = _k("defend_enemy_facing_dot", 0x144022404, [_DEFEND])
# 100: may move while |z difference| < 100 (or the path is idle)
static var defend_move_max_zdiff: float = _k("defend_move_max_zdiff", 0x143fe4e40, [_DEFEND])
# 0.2 s: footwork crouch only after WindupEnd + 0.2
static var defend_crouch_delay: float = _k("defend_crouch_delay", 0x1442898a4, [_DEFEND])
# -70: StartFacingActor / StartFacingMovement argument while footworking
static var defend_footwork_facing: float = _k("defend_footwork_facing", 0x1443247d8, [_DEFEND])
# 25: back-off distance (reach + 25) x BackOffFactorDuringDefense; facing point 25 above the enemy
static var defend_back_off_margin: float = _k("defend_back_off_margin", 0x1442898b8, [_DEFEND])
static var defend_facing_height: float = _k("defend_facing_height", 0x1442898b8, [_DEFEND])
# 140: minimum back-off / footwork distance
static var defend_back_off_min: float = _k("defend_back_off_min", 0x144324780, [_DEFEND])
# 0.1 s: parry once now > WindupEnd + EarlyReleaseDuration + ParryTimingRandom - 0.1
static var defend_parry_lead: float = _k("defend_parry_lead", 0x143fe4dfc, [_DEFEND])
# perfect parry blade sweep t = 0, 20, ... < 160
static var defend_sweep_max: float = _k("defend_sweep_max", 0x144324788, [_DEFEND])
static var defend_sweep_step: float = _k("defend_sweep_step", 0x143fe4e30, [_DEFEND])

# ---- UBTTask_MeleeAttack::PerformOffensiveEvaluation rva=0x1495d00 ---------------------------------------------
# 0.9 s: time left when the bot is not attacking
static var attack_default_time_left: float = _k("attack_default_time_left", 0x14427b734, [_ATTACK])
# 0.6: enemy speed weight in the reach formula
static var attack_enemy_speed_factor: float = _k("attack_enemy_speed_factor", 0x144324764, [_ATTACK])
# circling distance (Length - 2) x 10 x scale x rnd + 140
static var attack_jitter_length_offset: float = _k("attack_jitter_length_offset", 0x144014b10, [_ATTACK])
static var attack_jitter_scale: float = _k("attack_jitter_scale", 0x143fe4e24, [_ATTACK])
static var attack_circle_base: float = _k("attack_circle_base", 0x144324780, [_ATTACK])
# 25: out-of-range feint once reach + 25 < dist
static var attack_out_of_range_margin: float = _k("attack_out_of_range_margin", 0x1442898b8, [_ATTACK])
# 200: face movement (not the enemy) beyond reach + 200; give up beyond it
static var attack_engage_margin: float = _k("attack_engage_margin", 0x14432478c, [_ATTACK])
# 50: may move while |z difference| < 50 (or the path is idle)
static var attack_move_max_zdiff: float = _k("attack_move_max_zdiff", 0x1440e9f94, [_ATTACK])
# hesitation side step: FRotator(0, 45 - 90 x rnd, 0).RotateVector(dir x 400)
static var attack_side_step_dist: float = _k("attack_side_step_dist", 0x1442efaa0, [_ATTACK])
static var attack_side_step_yaw: float = _k("attack_side_step_yaw", 0x14402cb9c, [_ATTACK])
static var attack_side_step_yaw_range: float = _k("attack_side_step_yaw_range", 0x1440701fc, [_ATTACK])
# 0.15 s: windup feint once now > WindupEnd - 0.15 - FeintTimingRandom
static var attack_feint_lead: float = _k("attack_feint_lead", 0x1442efa58, [_ATTACK])
# drag / accel facing offset: pitch = AngleTarget x -60, yaw = 60 - |AngleTarget| x 30, x -1 per flip
static var attack_drag_pitch_scale: float = _k("attack_drag_pitch_scale", 0x1443247d4, [_ATTACK])
static var attack_drag_yaw_base: float = _k("attack_drag_yaw_base", 0x143fe4e38, [_ATTACK])
static var attack_drag_yaw_per_angle: float = _k("attack_drag_yaw_per_angle", 0x143fe4e34, [_ATTACK])
static var attack_flip: float = _k("attack_flip", 0x144014bb8, [_ATTACK])
# 150: brawl (kick) only within 150
static var attack_brawl_range: float = _k("attack_brawl_range", 0x144324784, [_ATTACK])
# P(stab) = stab / max(strike + stab, 1)
static var attack_choice_min_weight: float = _k("attack_choice_min_weight", 0x143fe4e0c, [_ATTACK])
# 2/32767: the discarded FlipSide coin
static var attack_flip_rand_scale: float = _k("attack_flip_rand_scale", 0x14432474c, [_ATTACK])
# angle = rand() x 120/32767 - 60
static var attack_angle_rand_scale: float = _k("attack_angle_rand_scale", 0x144324758, [_ATTACK])
static var attack_angle_offset: float = _k("attack_angle_offset", 0x143fe4e38, [_ATTACK])
# circling: FLT_MAX ally distance without an ally; bearing rnd x 360 - 180; ally within 300 -> opposite it;
# step clamped to [-30, 30]
static var circle_no_ally_dist: float = _k("circle_no_ally_dist", 0x144014b94, [_ATTACK])
static var circle_full_turn: float = _k("circle_full_turn", 0x1442713d4, [_ATTACK])
static var circle_half_turn: float = _k("circle_half_turn", 0x14402cba4, [_ATTACK])
static var circle_ally_avoid_dist: float = _k("circle_ally_avoid_dist", 0x1442e2ab8, [_ATTACK])
static var circle_step_min: float = _k("circle_step_min", 0x1443247cc, [_ATTACK])
static var circle_step_max: float = _k("circle_step_max", 0x143fe4e34, [_ATTACK])

# ---- UBTTask_FallForFeint::PerformFeintEvaluation rva=0x1495720 ------------------------------------------------
static var feint_enemy_facing_dot: float = _k("feint_enemy_facing_dot", 0x144022404, [_FEINT])
# 0.2 s after a feint began: parry
static var feint_react_delay: float = _k("feint_react_delay", 0x1442898a4, [_FEINT])
# 0.1 s: stop watching once WindupEnd - 0.1 has passed
static var feint_watch_lead: float = _k("feint_watch_lead", 0x143fe4dfc, [_FEINT])
# 32400 = 180^2: enemy within 180 cm
static var feint_range_sq: float = _k("feint_range_sq", 0x1443247ac, [_FEINT])

# ---- UBTTask_VoiceOrEmote::ExecuteTask -------------------------------------------------------------------------
# 3 s: voice component last voice time + 3
static var voice_min_interval: float = _k("voice_min_interval", 0x143fe4e10, [_VOICE])

# ---- AMordhauAIController::GetClosestEnemy (modes/AI r6) -------------------------------------------------------
# 1 s: LastClosestEnemyChangedTime + 1 <= RealTimeSeconds before the closest enemy is re-evaluated (0x1414fa748)
static var closest_enemy_recheck_s: float = _k("closest_enemy_recheck_s", 0x143fe4e0c, ["AMordhauAIController::GetClosestEnemy"])
# 160000 cm^2 (400 cm): saturation margin and the ReallyCloseEnemyCached radius (0x1414fac9e)
static var closest_enemy_saturation_sq: float = _k("closest_enemy_saturation_sq", 0x1443247b4, ["AMordhauAIController::GetClosestEnemy"])

# ---- AMordhauAIController RandomFloat / moves / CanSee (ai r8) --------------------------------------------------
# 60/32767: rand * scale = the random part of NextRandomFloatAssignment (0..60 s)
static var random_float_next_scale: float = _k("random_float_next_scale", 0x14433f6c8, ["AMordhauAIController::BeginPlay", "AMordhauAIController::LODTick"])
# 120 s: the first assignment's base delay (BeginPlay)
static var random_float_first_delay: float = _k("random_float_first_delay", 0x1442efa8c, ["AMordhauAIController::BeginPlay"])
# 60 s: every later assignment's base delay (LODTick)
static var random_float_delay: float = _k("random_float_delay", 0x143fe4e38, ["AMordhauAIController::LODTick"])
# 0.5: the pawn-goal midpoint and the half-distance search radius (GetMoveMidpoint_Implementation)
static var move_midpoint_half: float = _k("move_midpoint_half", 0x143fe4e04, ["AMordhauAIController::GetMoveMidpoint_Implementation"])
# 100 cm: CanSee's second trace raises its end point by UpVector * 100
static var can_see_raise: float = _k("can_see_raise", 0x143fe4e40, ["AMordhauAIController::CanSee"])

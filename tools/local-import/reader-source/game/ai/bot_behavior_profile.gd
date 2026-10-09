# bot_behavior_profile.gd - UBotBehaviorProfile: a bot's skill numbers and the per-motion random rolls drawn from them.
# Data: BotData.Profile (components/ue/records/bot_data.gd; field offsets are cited there), this bot's own copy.
#
# Probabilities (FeintProbability, ...) and variances are class data. RerollRandomInstanceValues turns them into this
# instance's decisions (bWillFeint, ParryTimingRandom, ...); the controller calls it every time the bot's own motion
# changes (AMordhauAIController::GetMotionBasedRandom rva=0x14fc660), so every decision is re-rolled per motion.
class_name BotBehaviorProfile
extends RefCounted

var p: BotData.Profile					# class defaults, then Randomize / the instance rolls (this bot's copy)
var last_footworking_enemy_motion = null	# +0xe8 TWeakObjectPtr<UMordhauMotion> LastFootworkingEnemyMotion

func _init(defaults: BotData.Profile = null) -> void:
	p = defaults if defaults != null else BotData.profile("")

# from UBotBehaviorProfile::RerollRandomInstanceValues rva=0x149e910. One rand() per line, in this order.
# c = 1/32767 (BotConstants.frand_scale); the unit vector's zero-length guard and the clamps are BotConstants too.
func reroll(rng: UeRand) -> void:
	var c := BotConstants.frand_scale
	var x := float(rng.rand()) * c * 2.0 - BotConstants.profile_one
	var y := float(rng.rand()) * c * 2.0 - BotConstants.profile_one
	var l2 := y * y + x * x
	if l2 <= BotConstants.profile_unit_vector_min_sq:
		p.random_2d_unit_vector = Vector2.ZERO
	else:
		var inv := 1.0 / sqrt(l2)
		p.random_2d_unit_vector = Vector2(y * inv, x * inv)		# X = second draw, Y = first draw
	p.will_brawl = rng.frand() < p.brawl_probability
	p.will_riposte = rng.frand() < p.riposte_probability
	p.will_feint = rng.frand() < p.feint_probability
	p.will_gamble = rng.frand() < p.gamble_probability
	p.will_morph = rng.frand() < p.morph_probability
	p.will_chamber = rng.frand() < p.chamber_probability
	p.will_accel = rng.frand() < p.accel_probability
	p.will_drag = rng.frand() < p.drag_probability
	p.will_combo = rng.frand() < p.combo_probability
	p.will_fall_for_feint = rng.frand() < p.fall_for_feint_probability
	p.will_out_of_range_feint = rng.frand() < p.out_of_range_feint_probability
	p.will_perfect_parry = rng.frand() < p.perfect_parry_probability
	last_footworking_enemy_motion = null												# null weak ptr
	p.will_footwork = rng.frand() < p.footwork_instead_of_parry_probability
	p.will_footwork_with_crouch = rng.frand() < p.footwork_with_crouch_probability
	p.feint_timing_random = float(rng.rand()) * p.feint_timing_variance * c
	var ptv := p.parry_timing_variance
	p.parry_timing_random = (ptv.y - ptv.x) * clampf(rng.frand(), 0.0, 1.0) + ptv.x
	var hv := p.attack_hesitance_variance
	var hr := hv * float(rng.rand()) * c
	p.attack_hesitance_random = (hv + p.base_attack_hesitance_time) - (hr + hr)
	var bo := p.back_off_factor_during_defense_min_max
	p.back_off_factor_during_defense = (bo.y - bo.x) * clampf(rng.frand(), 0.0, 1.0) + bo.x

# from UBotBehaviorProfile::Randomize rva=0x149c190 (a BotProfile with bRandomizeBehavior). Draw order as decompiled.
# BotConstants.randomize_cube_scale = 1/32767^3 (cube of a draw), c = 1/32767.
func randomize(rng: UeRand) -> void:
	var c3 := BotConstants.randomize_cube_scale
	var one := BotConstants.profile_one
	var cube := func() -> float:
		var r := float(rng.rand())
		return r * r * r * c3
	p.ignore_enemies_with_ally_count = mini(int(float(rng.rand()) * BotConstants.randomize_ally_count_scale), 2) + 1
	var t := float(rng.rand()) * BotConstants.frand_scale
	p.b_prefers_alt_mode = mini(int(t + t), 1) == 1
	p.combo_probability = one - cube.call()
	p.base_attack_hesitance_time = cube.call()
	p.attack_hesitance_variance = cube.call()
	p.fall_for_feint_probability = rng.frand()
	p.brawl_probability = rng.frand()
	p.riposte_probability = one - cube.call()
	p.feint_probability = rng.frand()
	p.gamble_probability = rng.frand()
	var a := float(rng.rand())
	var bb := float(rng.rand())
	p.parry_timing_variance = Vector2(bb * bb * bb * c3, a * a * a * c3)		# Y = first draw, X = second
	p.feint_timing_variance = rng.frand()
	p.drag_probability = rng.frand()
	p.accel_probability = rng.frand()
	p.chamber_probability = one - cube.call()
	p.perfect_parry_probability = rng.frand()
	p.morph_probability = rng.frand()
	p.footwork_instead_of_parry_probability = rng.frand()
	p.footwork_with_crouch_probability = rng.frand()

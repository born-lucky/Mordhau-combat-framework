# bt_melee_attack.gd - UBTTask_MeleeAttack, ported from extract/native/decomp/UBTTask_MeleeAttack.cpp
#   PerformOffensiveEvaluation rva=0x1495d00 (ExecuteTask rva=0x147fe60 is a 5-byte jump to it; TickTask rva=0x14a5120
#   finishes the latent task on anything but InProgress). Argument values Ghidra dropped (move/attack choice, the
#   circling angle, MoveToLocation acceptance radii) are read from the x86-64 disassembly of the same function
#   (scripts/ue_dis.py), cited "disasm <address>".
class_name BtMeleeAttack
extends BtTask

const FN := "UBTTask_MeleeAttack::PerformOffensiveEvaluation rva=0x1495d00"

func execute(c) -> int:
	return perform(c)

func tick(c, _dt: float) -> int:
	return perform(c)

func perform(c) -> int:
	var me: BotBody = c.body
	if me == null or me.sys == null:
		return FAILED
	var now: float = c.now()
	var prof: BotBehaviorProfile = c.profile
	var my_loc := me.location
	var w: WeaponData = me.sys.weapon					# RightHandEquipment (+0x11f8) as AMordhauWeapon
	if w == null or not w.b_can_attack:					# AMordhauEquipment +0xcb9 bCanAttack
		return FAILED
	var en: BotBody = c.closest_enemy()					# AMordhauAIController::GetClosestEnemy rva=0x14fa570
	if en == null or en.is_dead:
		c.note("MeleeAttack", "no live enemy -> RequestFeint, Failed", FN)
		c.request_feint()
		return FAILED
	var em = en.sys.motion
	var enemy_attacking: bool = em is AttackMotion		# UAttackMotion or URangedDrawMotion (not ported) -> fStackX_10
	var ally: BotBody = c.closest_ally()				# GetClosestAlly rva=0x14fa3b0
	var rnd: float = c.get_motion_based_random()		# controller +0x32c
	var own = me.sys.motion
	var my_atk = own if own is AttackMotion else null
	var time_left: float = (my_atk.release_end - now) if my_atk != null else K.attack_default_time_left	# 0.9
	var reach: float = me.scaled_radius() + en.scaled_radius() \
		+ me.weapon_length * K.weapon_length_to_cm * me.mesh_scale_x \
		+ (me.threat_speed() - en.threat_speed() * K.attack_enemy_speed_factor) * time_left		# 15, 0.6
	var my_parry = own if own is ParryMotion else null
	var dist := (en.location - my_loc).length()			# AActor::GetDistanceTo rva=0x2e2c0b0
	var jitter := maxf((me.weapon_length - K.attack_jitter_length_offset) * K.attack_jitter_scale * me.mesh_scale_x, 0.0)
	var move_dist := jitter * rnd + K.attack_circle_base	# (Length - 2) * 10 * scale * rnd + 140
	c.note("MeleeAttack", "dist %.1f range %.1f (own %s)" % [dist, reach,
		("attack " + CombatEnums.STAGE_NAMES[my_atk.stage]) if my_atk != null else own.kind()], FN)
	if reach + K.attack_out_of_range_margin < dist and my_atk != null and my_atk.stage == CombatEnums.Stage.WINDUP \
			and prof.p.will_out_of_range_feint:
		if c.facing_mode == MordhauBotController.Facing.LOCATION:	# GetCurrentFacingMode rva=0x14fafc0
			return FAILED
		c.note("MeleeAttack", "out of range in windup, bWillOutOfRangeFeint -> RequestFeint, Failed", FN)
		c.request_feint()
		return FAILED
	var en_loc := en.location
	var bone_h := en.facing_bone_height
	if reach + K.attack_engage_margin <= dist:		# 200
		c.start_facing_movement(0.0)
	else:
		c.start_facing_bone(en, bone_h, Vector2.ZERO)
	var may_move: bool = c.get_move_status() == MordhauBotController.PathStatus.IDLE \
		or absf(en_loc.z - my_loc.z) < K.attack_move_max_zdiff	# 50
	c.start_sprinting()
	var dir := safe_normal(my_loc - en_loc)
	if may_move:
		_circle(c, me, en, ally, dir, rnd, move_dist)
	var stage: int = my_atk.stage if my_atk != null else -1
	var choose := false
	if my_atk == null or stage == CombatEnums.Stage.RECOVERY:
		if reach < dist:
			return FAILED if reach + K.attack_engage_margin < dist else IN_PROGRESS
		var in_riposte: bool = my_parry != null and not (my_parry.riposte_window_start
			+ my_parry.pd.riposte_window_base + my_parry.pd.non_held_parry_extension_and_riposte_window_extra < now)
		if not in_riposte:
			var until: float = own.start_time + prof.p.attack_hesitance_random	# Motion StartTime + AttackHesitanceRandom
			if now < until:
				c.note("MeleeAttack", "hesitating until %.4f -> Failed" % until, FN)
				if not may_move:
					return FAILED
				# side step: enemy + FRotator(0, 45 - 90 * rnd, 0).RotateVector(dir * 400), acceptance -1 (PerformOffensiveEvaluation disasm 0x141496a28: xmm13 = -1 from 0x14149648c)
				var off := _rotate_yaw(dir * K.attack_side_step_dist,
					K.attack_side_step_yaw - rnd * K.attack_side_step_yaw_range)
				c.move_to_location(en_loc + off, K.default_acceptance)
				return FAILED
		elif not prof.p.will_riposte:
			return IN_PROGRESS
		choose = true
	else:
		if stage == CombatEnums.Stage.RELEASE:
			if reach < dist:
				return IN_PROGRESS
			if not prof.p.will_combo:
				return FAILED
		# LAB_1414968d1
		if stage != CombatEnums.Stage.WINDUP or (prof.p.will_morph and not enemy_attacking):
			choose = true
	if choose:
		_choose_and_request(c, w, dist)
	# LAB after the request (rva 0x1496b7d...): look at the attack the bot is in now
	var atk = me.sys.motion if me.sys.motion is AttackMotion else null
	var chambered: bool = atk != null and atk.stage == CombatEnums.Stage.WINDUP and atk.b_has_chambered	# +0x10f4
	c.get_motion_based_random()
	if atk == null:
		return FAILED
	match atk.stage:
		CombatEnums.Stage.RECOVERY:
			return FAILED
		CombatEnums.Stage.WINDUP:
			var feint_at: float = atk.windup_end - K.attack_feint_lead - prof.p.feint_timing_random	# 0.15
			if me.stamina_byte() - en.stamina_byte() > 0 and not chambered and prof.p.will_feint \
					and not enemy_attacking and feint_at < now:
				c.note("MeleeAttack", "feint: more stamina, bWillFeint, now > %.4f -> RequestFeint" % feint_at, FN)
				c.request_feint()
				return FAILED if me.sys.motion != atk else IN_PROGRESS
		CombatEnums.Stage.RELEASE:
			var accel: bool = prof.p.will_accel or enemy_attacking
			if ((prof.p.will_drag and not enemy_attacking) or accel) and CombatEnums.is_strike(atk.move):
				var pitch: float = atk.angle_target * K.attack_drag_pitch_scale						# * -60
				var yawo: float = K.attack_drag_yaw_base - absf(atk.angle_target) * K.attack_drag_yaw_per_angle	# 60 - |a| * 30
				if CombatEnums.is_left(atk.move):
					yawo *= K.attack_flip
				if accel:
					yawo *= K.attack_flip
					pitch *= K.attack_flip
				c.note("MeleeAttack", "%s: facing offset (%.2f, %.2f)" % ["accel" if accel else "drag", yawo, pitch], FN)
				c.start_facing_bone(en, bone_h, Vector2(yawo, pitch))
	return IN_PROGRESS

# attack choice, UBTTask_MeleeAttack::PerformOffensiveEvaluation disasm 0x141496a7f-0x141496b78
func _choose_and_request(c, w: WeaponData, dist: float) -> void:
	var prof: BotBehaviorProfile = c.profile
	var sides: int = c.ally_clearance_sides()			# GetAllyClearanceSides rva=0x14fa080
	var mv := CombatEnums.Move.KICK
	if not prof.p.will_brawl or K.attack_brawl_range < dist:	# brawl (kick) only within 150
		var stab: float = w.stab.damage[0]				# StabAttack.Damage[0] (weapon +0xfa0 -> data[0])
		var strike: float = w.strike.damage[0]			# StrikeAttack.Damage[0] (weapon +0x1440)
		var p := UeMath.f32(stab / maxf(strike + stab, K.attack_choice_min_weight))
		mv = CombatEnums.Move.STAB if p > c.rng.frand() else CombatEnums.Move.RIGHT_STRIKE
		var r := mini(int(float(c.rng.rand()) * K.attack_flip_rand_scale), 1)	# 2/32767
		if r == 1:
			CombatEnums.flip_side(mv)					# PerformOffensiveEvaluation 0x141496b23: result discarded (bl unchanged)
		var left := CombatEnums.is_left(mv)
		if (sides == 0 and left) or (sides == 1 and not left):
			mv = CombatEnums.flip_side(mv)
		c.note("MeleeAttack", "P(stab)=%.4f sides=%d -> %s" % [p, sides, CombatEnums.MOVE_NAMES[mv]], FN)
	var angle := float(c.rng.rand()) * K.attack_angle_rand_scale - K.attack_angle_offset	# 120/32767 * r - 60
	c.note("MeleeAttack", "RequestAttack(%s, %.3f)" % [CombatEnums.MOVE_NAMES[mv], angle], FN)
	c.request_attack(mv, angle)

# circling (UBTTask_MeleeAttack::PerformOffensiveEvaluation disasm 0x141496594-0x141496862): target bearing around the enemy = rnd * 360 - 180 relative to its yaw
# (or opposite a close ally), step at most 30 degrees per evaluation, at move_dist from the enemy
func _circle(c, me: BotBody, en: BotBody, ally: BotBody, dir: Vector3, rnd: float, move_dist: float) -> void:
	var ally_dist := K.circle_no_ally_dist			# FLT_MAX
	if ally != null and not ally.is_dead:
		ally_dist = (ally.location - me.location).length()
	var cur := calculate_angle_2d(dir, en.yaw)
	var target := rnd * K.circle_full_turn - K.circle_half_turn	# * 360 - 180
	if ally_dist < K.circle_ally_avoid_dist:			# 300
		var a := fmod(calculate_angle_2d(safe_normal(ally.location - en.location), en.yaw) + K.circle_half_turn,
			K.circle_full_turn)
		if a < 0.0:
			a += K.circle_full_turn
		if a > K.circle_half_turn:
			a -= K.circle_full_turn
		target = a
	var step := clampf(find_delta_angle(cur, target), K.circle_step_min, K.circle_step_max)	# +-30
	var dest := en.location + _rotate_yaw(dir * move_dist, step)
	if not c.nav_blocked(en.location, dest):
		c.move_to_location(dest, K.default_acceptance)
	else:
		c.move_to_location(en.location, move_dist)

# FRotator(0, yaw, 0).RotateVector: rotation about Z, +yaw turns X toward Y (UE convention)
static func _rotate_yaw(v: Vector3, yaw_deg: float) -> Vector3:
	var r := deg_to_rad(yaw_deg)
	return Vector3(v.x * cos(r) - v.y * sin(r), v.x * sin(r) + v.y * cos(r), v.z)

# UMordhauUtilityLibrary::CalculateAngle2D rva=0x1616170: signed angle (degrees) of `d` from the forward of
# FRotator(0, yaw, 0), positive toward its right axis; all |components| <= 1e-4 (_DAT_144022350) -> 0.
# acos * _DAT_1442713d0 (57.29578)
static func calculate_angle_2d(d: Vector3, yaw: float) -> float:
	var e := K.angle2d_zero_component
	if absf(d.x) <= e and absf(d.y) <= e and absf(d.z) <= e:
		return 0.0
	var r := deg_to_rad(yaw)
	var fwd := Vector2(cos(r), sin(r))
	var right := Vector2(-sin(r), cos(r))
	var d2 := Vector2(d.x, d.y)
	if d2.length_squared() != 1.0:
		d2 = d2.normalized() if d2.length_squared() >= K.angle2d_min_sq else Vector2.ZERO
	var a := acos(clampf(fwd.dot(d2), -1.0, 1.0)) * K.angle2d_rad_to_deg
	return -a if right.dot(d2) < 0.0 else a

# FMath::FindDeltaAngleDegrees rva=0x1480350 (UE 4.26 UnrealMathUtility.h: A2 - A1 wrapped to [-180, 180])
static func find_delta_angle(a1: float, a2: float) -> float:
	var d := a2 - a1
	if d > 180.0:
		d -= 360.0
	elif d < -180.0:
		d += 360.0
	return d

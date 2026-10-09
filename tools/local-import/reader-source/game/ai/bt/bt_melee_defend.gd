# bt_melee_defend.gd - UBTTask_MeleeDefend, ported from extract/native/decomp/UBTTask_MeleeDefend.cpp
#   PerformDefensiveEvaluation rva=0x14940a0, ExecuteTask rva=0x147fe70, TickTask rva=0x14a5160, AbortTask rva=0x1456e80,
#   StopCrouching rva=0x14a3460. Call targets resolved through the PDB labels (extract/native/labels.tsv), offsets through
#   extract/native/types, literals through extract/native/rdata.tsv (UeRdata).
#
# Per perceived enemy whose motion is a UAttackMotion not in Recovery, nearest-first is NOT applied: the game walks
# GetPerceivedEnemies in order and the first enemy that qualifies decides the result.
class_name BtMeleeDefend
extends BtTask

const FN := "UBTTask_MeleeDefend::PerformDefensiveEvaluation rva=0x14940a0"

# ExecuteTask rva=0x147fe70: Perform; anything but InProgress also runs StopCrouching
func execute(c) -> int:
	var r := perform(c)
	if r != IN_PROGRESS:
		stop_crouching(c)
	return r

# TickTask rva=0x14a5160: Perform; anything but InProgress -> StopCrouching, FinishLatentTask(r)
func tick(c, _dt: float) -> int:
	return execute(c)

# AbortTask rva=0x1456e80: StopCrouching, return Aborted (2)
func abort(c) -> int:
	stop_crouching(c)
	return ABORTED

# StopCrouching rva=0x14a3460: controller's pawn is an alive AMordhauCharacter -> bWantsCrouch (+0xde4) = 0
static func stop_crouching(c) -> void:
	if c.body != null and not c.body.is_dead:
		c.body.wants_crouch = false

func perform(c) -> int:
	var me: BotBody = c.body
	if me == null or me.sys == null:					# AIOwner / Pawn casts / MotionSystemComponent (+0x688) null
		return FAILED
	var own = me.sys.motion
	var own_parry = own if own is ParryMotion else null	# Cast<UParryMotion>(Motion) (StaticClass 0x141710970)
	var now: float = c.now()							# GetWorld()->TimeSeconds (+0x598)
	var prof: BotBehaviorProfile = c.profile			# controller +0x480 BehaviorProfile
	c.get_motion_based_random()
	c.start_sprinting()									# AMordhauCharacter::StartSprinting rva=0x156dcc0
	var my_loc := me.location
	var my_fwd := me.forward()
	var pending := false								# bVar9: an attack is coming but not yet in threat range
	for en: BotBody in c.perceived_enemies():					# AMordhauAIController::GetPerceivedEnemies rva=0x14fd110
		var m = en.sys.motion if en.sys != null else null
		if not (m is AttackMotion) or m.stage == CombatEnums.Stage.RECOVERY:	# +0x10e9 Stage != 2
			continue
		var windup: bool = m.stage == CombatEnums.Stage.WINDUP			# bVar8
		if m.stage == CombatEnums.Stage.RELEASE and m.b_has_hit and en.ignore_cache.has(me):
			c.note("MeleeDefend", "%s: Release, already hit me (Weapon.ActorIgnoreCache) -> ignore" % en.name, FN)
			continue
		# threat / reach ranges (LAB_1414943af)
		var time_left: float = m.release_end - now		# ReleaseEnd (+0x1094) - now
		var threat: float = me.scaled_radius() + en.scaled_radius() \
			+ en.weapon_length * K.weapon_length_to_cm * en.mesh_scale_x + time_left * en.threat_speed()
		var reach: float = me.threat_speed() * time_left + threat
		var en_loc := en.location
		var d := safe_normal(my_loc - en_loc)
		var not_in_front: bool = (-d).dot(my_fwd) < K.defend_in_front_dot		# bVar7, 0.3
		var dist2d := Vector2(en_loc.x - my_loc.x, en_loc.y - my_loc.y).length()
		if d.dot(en.forward()) < K.defend_enemy_facing_dot or dist2d > reach:		# -0.5: enemy faces away
			c.note("MeleeDefend", "%s %s: not facing me or dist %.1f > reach %.1f -> ignore" % [en.name,
				CombatEnums.STAGE_NAMES[m.stage], dist2d, reach], FN)
			continue
		if dist2d > threat:
			pending = true
			c.note("MeleeDefend", "%s: dist %.1f in reach %.1f, outside threat %.1f -> wait" % [en.name, dist2d, reach, threat], FN)
			continue
		if not (not_in_front or not windup or not prof.p.will_gamble):
			c.note("MeleeDefend", "%s: Windup in front and bWillGamble -> ignore (gamble)" % en.name, FN)
			continue
		# footwork: only against the first enemy attack motion it was chosen for (LastFootworkingEnemyMotion +0xe8)
		var footwork := prof.p.will_footwork			# +0xe4
		if footwork:
			if prof.last_footworking_enemy_motion == null:
				prof.last_footworking_enemy_motion = m
			elif prof.last_footworking_enemy_motion != m:
				footwork = false
		if not_in_front and not footwork:
			c.note("MeleeDefend", "%s: attack not in front of me -> ignore" % en.name, FN)
			continue
		var perfect := prof.p.will_perfect_parry		# +0xdc
		var delay := 0.0 if perfect else prof.p.parry_timing_random	# +0xe0
		var zdiff := absf(en_loc.z - my_loc.z)
		var may_move: bool = c.get_move_status() == MordhauBotController.PathStatus.IDLE \
			or zdiff < K.defend_move_max_zdiff			# AAIController::GetMoveStatus, 100
		c.note("MeleeDefend", "%s %s %s: threat (dist %.1f <= %.1f), footwork=%s perfect=%s delay=%.4f" % [en.name,
			CombatEnums.MOVE_NAMES[m.move], CombatEnums.STAGE_NAMES[m.stage], dist2d, threat, footwork, perfect, delay], FN)
		if not footwork or not prof.p.will_footwork_with_crouch or now <= m.windup_end + K.defend_crouch_delay:
			c.stop_crouching()							# AMordhauCharacter::StopCrouching rva=0x156ddc0
		else:
			c.start_facing_actor(en, K.defend_footwork_facing, Vector2.ZERO)	# -70
			c.start_crouching()							# bWantsCrouch_SetBit / StartCrouching rva=0x156dc90
		if may_move:
			var target: Vector3 = en.trace_start		# Weapon (+0x10c8) CurrentTraceStart (+0xd48)
			var back := maxf((reach + K.defend_back_off_margin) * prof.p.back_off_factor_during_defense,
				K.defend_back_off_min)				# (reach + 25) * factor, at least 140
			var k := back
			if footwork:
				k = K.defend_back_off_min
				if not prof.p.will_footwork_with_crouch:	# +0xe5
					c.start_facing_movement(K.defend_footwork_facing)
					k = back
			var dest := target + safe_normal(my_loc - target) * k
			if not c.nav_blocked(en_loc, dest):			# UNavigationSystemV1::NavigationRaycast
				c.move_to_location(dest, K.default_acceptance)
			else:
				c.move_to_location(en_loc, k)
		if footwork:
			c.note("MeleeDefend", "footwork instead of parry -> InProgress", FN)
			return IN_PROGRESS
		if prof.p.will_fall_for_feint and m.type == CombatEnums.AttackType.MORPH:	# +0xd4, Type (+0x108d) == 4
			c.note("MeleeDefend", "morph and bWillFallForFeint -> Succeeded (FallForFeint parries it)", FN)
			return SUCCEEDED
		c.start_facing_location(en_loc + Vector3(0, 0, K.defend_facing_height))
		if own_parry != null:
			c.note("MeleeDefend", "already parrying -> InProgress", FN)
			return IN_PROGRESS
		# parry once now > WindupEnd + GetEarlyReleaseDuration + delay - 0.1 (_DAT_143fe4dfc)
		var at: float = m.get_early_release_duration() + m.windup_end + delay - K.defend_parry_lead
		if now <= at:
			c.note("MeleeDefend", "now %.4f <= parry time %.4f -> InProgress" % [now, at], FN)
			return IN_PROGRESS
		if perfect and not _blade_reaches(me, en):
			c.note("MeleeDefend", "perfect parry: blade sweep misses my capsule -> InProgress", FN)
			return IN_PROGRESS
		var bt := CombatEnums.BlockType.ALT_REGULAR if CombatEnums.is_left(m.move) else CombatEnums.BlockType.REGULAR
		c.note("MeleeDefend", "RequestParry(IsLeft(%s) = %d)" % [CombatEnums.MOVE_NAMES[m.move], bt], FN)
		c.request_parry(bt)								# AMordhauCharacter::RequestParry rva=0x1564860 (EBlockType, true)
		if me.sys.motion is ParryMotion:
			return IN_PROGRESS
		return FAILED
	if pending:
		return IN_PROGRESS
	var k2: String = own.kind() if own != null else ""
	if k2.begins_with("Flinch") or k2.begins_with("Blocked"):	# UFlinchMotion / UBlockedMotion StaticClass checks
		return FAILED
	return SUCCEEDED

# perfect parry (LAB_141494d80): step t = 0, 20, ... < 160 (_DAT_143fe4e30, _DAT_144324788) along
# dir = normal(CurrentTraceEnd - PreviousTraceEnd); line CurrentTraceStart -> CurrentTraceEnd + dir * t against my
# capsule (UPrimitiveComponent::LineTraceComponent, capsule vtable +0x848). The capsule test itself is engine code;
# here it is the exact segment / capsule intersection (UNCONFIRMED: collision margins of the engine's version).
static func _blade_reaches(me: BotBody, en: BotBody) -> bool:
	var dir := safe_normal(en.trace_end - en.prev_trace_end)
	var t := 0.0
	while t < K.defend_sweep_max:
		if segment_hits_capsule(en.trace_start, en.trace_end + dir * t, me.location,
				me.capsule_half_height * me.capsule_scale_min, me.scaled_radius()):
			return true
		t += K.defend_sweep_step
	return false

# vertical capsule centred at `c`: segment of half length (half_height - radius) on Z, inflated by radius.
# Closest distance between segment ab and the vertical axis segment, by clamped closest-point parameters.
static func segment_hits_capsule(a: Vector3, b: Vector3, c: Vector3, half_height: float, radius: float) -> bool:
	var h := maxf(half_height - radius, 0.0)
	var p0 := c - Vector3(0, 0, h)
	var d1 := b - a
	var d2 := Vector3(0, 0, 2.0 * h)
	var r := a - p0
	var aa := d1.dot(d1)
	var ee := d2.dot(d2)
	var ff := d2.dot(r)
	var s := 0.0
	var t := 0.0
	if aa <= 1e-12 and ee <= 1e-12:
		return r.length() <= radius
	if aa <= 1e-12:
		t = clampf(ff / ee, 0.0, 1.0)
	else:
		var cc := d1.dot(r)
		if ee <= 1e-12:
			s = clampf(-cc / aa, 0.0, 1.0)
		else:
			var bb := d1.dot(d2)
			var den := aa * ee - bb * bb
			s = clampf((bb * ff - cc * ee) / den, 0.0, 1.0) if den != 0.0 else 0.0
			t = (bb * s + ff) / ee
			if t < 0.0:
				t = 0.0
				s = clampf(-cc / aa, 0.0, 1.0)
			elif t > 1.0:
				t = 1.0
				s = clampf((bb - cc) / aa, 0.0, 1.0)
	return (a + d1 * s).distance_to(p0 + d2 * t) <= radius

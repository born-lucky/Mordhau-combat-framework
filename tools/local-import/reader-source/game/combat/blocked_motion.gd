# blocked_motion.gd - UBlockedMotion: the attacker after its attack was parried, chambered, clashed, stopped by the
# world or by a hit (extract/native/decomp/UBlockedMotion.cpp, all 8 functions; fields types/UBlockedMotion.h,
# FBlockResult.h). Constants: MotionDefs.Blocked (CombatData.motion_def(BP_BlockedMotion)); the blocked attack's
# fields (bounce curves, recoveries) from its MotionDefs.Attack.
# NetMotion for Blocked (FNetMotion::Blocked rva=0x1615f50): MotionType 6, Param0 = FBlockResult.Reason
# (EBlockedReason), Param1 = the FBlockResult bools as bits (bIsStun 1, bIsDisarm 2, bIsRanged 4, bIsCancel 8,
# bPartyFlag 16, bRequiresSelfBlockEvent 32, bClashOnParry 64), Param2 = time x 40 (CombatConstants.blocked_time_to_byte) as a byte.
# Animation: the engine calls OnBegin / OnTick / OnLeave make (StopAnim, SetAnimRate, PlayAnim, SetAnimPosition) are
# recorded as fields (stop_anim_fade, kick_hit_stop_anim_rate, bounce_montage, bounce_anim_position) for the
# animation layer; the simulation is third person (the *3P members; UMordhauMotion::GetIsFirstPerson false).
# Not ported (no gameplay effect): the disarm voice line and stats counter (OnBegin, bIsDisarm), the camera shake,
# the offhand-IK copies, the weapon's OnWasBlocked body (sparks / sound; its call is the `was_blocked` event).
class_name BlockedMotion
extends CombatMotion

# FBlockResult (+0xa9, types/FBlockResult.h) as OnBegin unpacks it from the NetMotion
var reason := 0						# Reason (+0xa9)
var b_is_stun := false				# +0xaa
var b_is_disarm := false			# +0xab
var b_is_ranged := false			# +0xac
var b_is_cancel := false			# +0xad
var b_party_flag := false			# +0xae
var b_requires_self_block_event := false	# +0xaf
var b_clash_on_parry := false		# +0xb0
var surface := 0					# +0xb1 (OnBegin stores 0)
var from_move := 0					# FromMove (+0xa0), the blocked attack's move
var from_attack = null				# MotionSystem LastAttackMotion at begin (its Montage +0x10b0 = FromAttackMontage +0xb8)
var b_has_queued_move := false		# +0xa1
var queued_move := 0				# +0xa8
var queued_angle := 0.0				# +0xa4
var original_movement_restriction := 0	# +0xca OriginalMovementRestriction
var b_has_faded_out_procedural := false	# +0xb2
var b_do_release_bounce_procedural := false	# +0xb3
# animation requests (see header): -1 / "" = none
var stop_anim_fade := -1.0			# blend time of the last AAdvancedCharacter::StopAnim this motion asked for
var kick_hit_stop_anim_rate := -1.0	# SetAnimRate(FromAttackMontage, KickHitStopAnimRate)
var bounce_montage := ""			# PlayAnim(BounceMontage, 1.0, true) (the attack's +0xae0)
var bounce_additive := ""			# UMordhauAnimInstance AttackBounce + SetAdditiveOverrideType("Bounce", 0.2)
var bounce_anim_position := -1.0	# SetAnimPosition(FromAttackMontage, position) of this tick

var bd: MotionDefs.Blocked		# this class's defaults (= def, typed)

func _init(owner_sys: MotionSystem, definition: MotionDefs.Base) -> void:
	super(owner_sys, definition)
	bd = definition as MotionDefs.Blocked

func kind() -> String:
	return "Blocked"

# from UBlockedMotion::OnBegin_Implementation rva=0x165dc20 (decomp UBlockedMotion.cpp 8-447)
func on_begin() -> void:
	from_attack = sys.last_attack_motion
	if from_attack == null:			# LastAttackMotion (MotionSystem +0x108) null -> return (EndTime stays 0)
		return
	var atk: AttackMotion = from_attack
	reason = sys.net.param0
	var flags: int = sys.net.param1		# byte 7 of the NetMotion: bit n -> FBlockResult byte n + 1 (decomp 83-98)
	b_is_stun = (flags & 1) != 0
	b_is_disarm = (flags & 2) != 0
	b_is_ranged = (flags & 4) != 0
	b_is_cancel = (flags & 8) != 0
	b_party_flag = (flags & 16) != 0
	b_requires_self_block_event = (flags & 32) != 0
	b_clash_on_parry = (flags & 64) != 0
	surface = 0
	var t := float(sys.net.param2) * CombatConstants.blocked_time_unit_s	# byte x 0.025
	from_move = atk.move
	var lim := bd.parried_recovery_time_limits
	var rec := clampf(t + bd.parried_recovery_time_offset, lim.x, lim.y)
	match reason:
		CombatEnums.BlockedReason.WORLD:
			b_can_block = false
			# a world hit at or after EarlyRelease (+0xb48 <= LastReleaseNormalizedTime +0x1084) costs
			# (int)-(AttackInfo.MissStaminaCost +0xedc x WorldMissStaminaFactor)
			if atk.early_release <= atk.last_release_norm:
				sys.offset_stamina(UeMath.cvttss2si(-(atk.ai.miss_stamina_cost * bd.world_miss_stamina_factor)))
			rec = bd.world_recovery_time
			movement_restriction = bd.movement_restriction_world
		CombatEnums.BlockedReason.CHAMBER:
			var cl := bd.chambered_recovery_time_limits
			rec = clampf(t + bd.chambered_recovery_time_offset, cl.x, cl.y)
		CombatEnums.BlockedReason.CLASH:
			rec = atk.ad.clashed_recovery		# +0xabc
		CombatEnums.BlockedReason.HIT:
			rec = atk.ad.hit_stop_recovery		# +0xab8
	# the attack's weapon (+0x10c8) is told it was blocked unless bRequiresSelfBlockEvent, for Parry / Chamber / Hit
	# (AMordhauWeapon::OnWasBlocked_Implementation rva=0x16340a0: sparks and sounds only)
	if not b_requires_self_block_event and (reason == CombatEnums.BlockedReason.PARRY \
			or reason == CombatEnums.BlockedReason.CHAMBER or reason == CombatEnums.BlockedReason.HIT):
		sys.world.emit_event({"kind": "was_blocked", "who": sys.name, "reason": reason, "move": from_move})
	if reason == CombatEnums.BlockedReason.HIT:
		movement_restriction = bd.movement_restriction_hit
	if from_move == CombatEnums.Move.KICK:
		original_movement_restriction = movement_restriction
		movement_restriction = CombatEnums.MovementRestriction.NO_MOVEMENT	# immediate 3 (UBlockedMotion::OnBegin_Implementation disasm 0x14165dfb6)
	end_time = (rec + start_time) - expected_delay
	# NextKickTime (MotionSystem +0xb0), UBlockedMotion::OnBegin_Implementation disasm 0x14165dfbf..0x14165dfe9: World / Hit leave it; Chamber / Clash
	# EndTime + 0.3; Parry EndTime + 0.2
	if reason == CombatEnums.BlockedReason.CHAMBER or reason == CombatEnums.BlockedReason.CLASH:
		sys.next_kick_time = end_time + CombatConstants.blocked_kick_delay_chamber
	elif reason == CombatEnums.BlockedReason.PARRY:
		sys.next_kick_time = end_time + CombatConstants.blocked_kick_delay_parry
	_begin_animation(atk)
	# a move the blocked attack had queued is replayed into this motion's ProcessAttack (virtual call) when the owner
	# is locally controlled (vcall +0x9c8) and the block was a Clash of a non-kick or a Hit of a kick (decomp 423-445)
	if atk.b_has_queued_move and sys.is_locally_controlled():
		var replay := (reason == CombatEnums.BlockedReason.CLASH and from_move != CombatEnums.Move.KICK) \
			or (reason == CombatEnums.BlockedReason.HIT and from_move == CombatEnums.Move.KICK)
		if replay:
			process_attack(atk.queued_move, atk.queued_angle)

# The animation half of OnBegin (decomp 196-422), third person
func _begin_animation(atk: AttackMotion) -> void:
	if atk.bounce_montage == "":
		if reason == CombatEnums.BlockedReason.CLASH or CombatEnums.is_stab(from_move):
			match reason:
				CombatEnums.BlockedReason.HIT: stop_anim_fade = bd.stab_hit_stop_fade_out_time
				CombatEnums.BlockedReason.WORLD: stop_anim_fade = bd.stab_world_fade_out_time
				CombatEnums.BlockedReason.CLASH: stop_anim_fade = bd.clash_fade_out_time
				_:
					var rng := bd.stab_parry_min_max_range
					var fade := bd.stab_parry_fade_out_time
					if reason != CombatEnums.BlockedReason.PARRY:
						rng = bd.stab_chambered_min_max_range
						fade = bd.stab_chambered_fade_out_time
					var a := UeMath.range_fraction(rng.x, rng.y, end_time - start_time)
					stop_anim_fade = (fade.y - fade.x) * a + fade.x
		else:
			b_do_release_bounce_procedural = true
	elif reason == CombatEnums.BlockedReason.HIT:
		if atk.has_montage():
			kick_hit_stop_anim_rate = bd.kick_hit_stop_anim_rate
		if bd.kick_hit_stop_blend_out_time != CombatConstants.blocked_no_blend_out:
			stop_anim_fade = bd.kick_hit_stop_blend_out_time
	else:
		bounce_montage = atk.bounce_montage
	var bounce := atk.ad.bounce_additive != ""
	if CombatEnums.is_strike(from_move):
		bounce = bounce and reason != CombatEnums.BlockedReason.HIT
	if bounce and from_move != CombatEnums.Move.KICK:
		bounce_additive = atk.ad.bounce_additive

# from UBlockedMotion::OnTick_Implementation rva=0x16671b0 (decomp 454-728). Reads the MotionSystem's
# LastAttackMotion each tick; returns at once without one or without its montage (FromAttackMontage +0xb8).
func on_tick(_dt: float) -> void:
	var atk = sys.last_attack_motion
	if atk == null or from_attack == null or not from_attack.has_montage():
		return
	var t := now()
	if from_move == CombatEnums.Move.KICK and start_time + CombatConstants.blocked_bounce_window < t:
		movement_restriction = original_movement_restriction
	if not b_do_release_bounce_procedural:
		return
	var until_fade := 0.0
	var fade_out := 0.0
	var duration := 0.0
	match reason:
		CombatEnums.BlockedReason.HIT:
			until_fade = bd.hit_stop_time_until_fade
			fade_out = bd.hit_stop_fade_out_time
			duration = bd.hit_stop_bounce_duration
		CombatEnums.BlockedReason.WORLD:
			until_fade = bd.world_time_until_fade
			fade_out = bd.world_fade_out_time
			duration = bd.world_bounce_duration
		_:
			var rng := bd.parry_min_max_range
			var tuf := bd.parry_time_until_fade
			var fo := bd.parry_fade_out_time
			var bdur := bd.parry_bounce_duration
			if reason != CombatEnums.BlockedReason.PARRY:
				rng = bd.chamber_min_max_range
				tuf = bd.chamber_time_until_fade
				fo = bd.chamber_fade_out_time
				bdur = bd.chamber_bounce_duration
			var a := UeMath.range_fraction(rng.x, rng.y, end_time - start_time)
			until_fade = (tuf.y - tuf.x) * a + tuf.x
			fade_out = (fo.y - fo.x) * a + fo.x
			duration = (bdur.y - bdur.x) * a + bdur.x
	if until_fade + start_time < t and not b_has_faded_out_procedural:
		b_has_faded_out_procedural = true
		stop_anim_fade = fade_out
	if start_time + CombatConstants.blocked_bounce_window <= t:
		return
	var nt := UeMath.normalized_time(start_time, start_time + duration, t)
	var lrn: float = atk.last_release_norm
	var length: float = sys.tracer.length		# the attack's Weapon +0x1bec Length (0 without a weapon)
	var curve := ""
	var scale := ""
	var ad: MotionDefs.Attack = atk.ad
	if reason == CombatEnums.BlockedReason.HIT:
		curve = bd.hit_stop_bounce_curve
		scale = bd.hit_stop_bounce_scale_curve
	else:
		if not CombatEnums.is_strike(atk.move):
			return
		if reason == CombatEnums.BlockedReason.WORLD:
			curve = ad.world_bounce_curve
			scale = ad.world_bounce_scale_curve
		else:
			curve = ad.parry_bounce_curve if lrn <= CombatConstants.blocked_late_bounce_release else ad.parry_late_bounce_curve
			scale = ad.parry_bounce_scale_curve
	if curve == "" or scale == "":
		return
	var release_scale := CombatConstants.blocked_unit_scale
	var rs := bd.hit_stop_release_scale_curve if reason == CombatEnums.BlockedReason.HIT else bd.release_scale_curve
	if rs != "":
		release_scale = CombatData.curve_value(rs, length)
	var pos := CombatData.curve_value(curve, nt) * release_scale * CombatData.curve_value(scale, lrn) \
		+ lrn * CombatConstants.blocked_bounce_half + CombatConstants.blocked_bounce_half
	bounce_anim_position = clampf(pos, 0.0, 1.0)

# from UBlockedMotion::OnLeave_Implementation rva=0x1665730: a procedural bounce that has not faded out yet stops the
# montage (UAdvancedCharacterAnimInstance::Montage_Stop_Wrapped, blend 0.5)
func on_leave(_interrupted: bool) -> void:
	if b_do_release_bounce_procedural and not b_has_faded_out_procedural:
		b_has_faded_out_procedural = true
		stop_anim_fade = CombatConstants.blocked_leave_fade

# from UBlockedMotion::ProcessAttack_Implementation rva=0x166b320
func process_attack(m: int, angle: float) -> bool:
	var last = sys.last_attack_motion
	if last == null:
		return false
	var from_kick := from_move == CombatEnums.Move.KICK
	var clash_from_kick := false
	if reason == CombatEnums.BlockedReason.CLASH and from_kick:
		clash_from_kick = true
		if m == CombatEnums.Move.KICK:
			return false
	else:
		if reason == CombatEnums.BlockedReason.CHAMBER:
			return false
		if reason == CombatEnums.BlockedReason.HIT and from_kick and m != CombatEnums.Move.KICK:
			sys.assign_net_attack_motion(CombatEnums.AttackType.COMBO, m, angle)
			return sys.motion != self
		if reason != CombatEnums.BlockedReason.CLASH:
			return _queue(m, angle, clash_from_kick)
	if not from_kick:
		var cv: AttackMotion.ComboMove = last.convert_to_combo(m)
		if not cv.ok or not sys.can_perform_attack(cv.move):
			return false
		var ty := CombatEnums.AttackType.POST_CLASH_SLOW if b_clash_on_parry else CombatEnums.AttackType.POST_CLASH
		sys.assign_net_attack_motion(ty, cv.move, angle)
		return sys.motion != self
	return _queue(m, angle, clash_from_kick)

func _queue(m: int, angle: float, clash_from_kick: bool) -> bool:
	var hit_window := clash_from_kick or reason == CombatEnums.BlockedReason.HIT
	var w := bd.queue_window_hit if hit_window else bd.queue_window
	if end_time - w <= now():
		queued_angle = angle
		b_has_queued_move = true
		queued_move = m
	return false

# from UBlockedMotion::OnEnded_Implementation rva=0x1663650
func on_ended() -> void:
	if b_has_queued_move:
		sys.assign_net_attack_motion(CombatEnums.AttackType.REGULAR, queued_move, queued_angle)
		if sys.motion != self:
			return
	super.on_ended()

# from UBlockedMotion::GetAttackCompensationStartTime rva=0x165c460: the blocks that take a follow-up attack (a Clash
# of a non-kick, a Hit of a kick answered by a non-kick) give 0 when bCanAttack, else EndTime; every other block EndTime
func get_attack_compensation_start_time(m: int) -> float:
	var from_kick := from_move == CombatEnums.Move.KICK
	var follow_up := (reason == CombatEnums.BlockedReason.CLASH and not from_kick) \
		or (reason == CombatEnums.BlockedReason.HIT and from_kick and m != CombatEnums.Move.KICK)
	if follow_up and b_can_attack:
		return 0.0
	return end_time

func trace_fields() -> Dictionary:
	return {"reason": reason, "end": end_time, "queued": b_has_queued_move}

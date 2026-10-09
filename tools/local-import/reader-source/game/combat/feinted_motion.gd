# feinted_motion.gd - UFeintedMotion: the lockout after a feint (extract/native/decomp/UFeintedMotion.cpp).
# Constants: MotionDefs.Feinted (CombatData.motion_def(BP_FeintedMotion)).
class_name FeintedMotion
extends CombatMotion

var lock_out_time := 0.0		# +0xd8 LockOutTime
var strike_lockout := 0.0		# +0xd4
var stab_lockout := 0.0			# +0xd0
var feint_type := 0				# +0xdc Type (EFeintType)
var from_move := 0				# +0xdd FromMove
var b_has_queued_move := false	# +0xde
var queued_move := 0			# +0xdf
var queued_angle := 0.0			# +0xe0
var queue_execute_time := 0.0	# +0xe4

var fd: MotionDefs.Feinted		# this class's defaults (= def, typed)

func _init(owner_sys: MotionSystem, definition: MotionDefs.Base) -> void:
	super(owner_sys, definition)
	fd = definition as MotionDefs.Feinted

func kind() -> String:
	return "Feinted"

# UFeintedMotion::CanInitiateMotion_Implementation rva=0x7bf3e0 (shared stub `mov al, 1; ret`): anything may start
func can_initiate_motion(_new_kind: String) -> bool:
	return true


static func _map_clamped(v: float, inr: Vector2, outr: Vector2) -> float:
	# FMath::GetMappedRangeValueClamped as inlined in OnBegin: alpha = clamp((v - In.X) / (In.Y - In.X), 0, 1),
	# degenerate In range (|In.Y - In.X| <= 1e-8, CombatConstants.small_number) -> 1 if v >= In.Y else 0
	var span: float = inr.y - inr.x
	var a := 0.0
	if absf(span) > CombatConstants.small_number:
		a = clampf((v - inr.x) / span, 0.0, 1.0)
	else:
		a = 1.0 if v >= inr.y else 0.0
	return (outr.y - outr.x) * a + outr.x

# from UFeintedMotion::OnBegin_Implementation rva=0x1660240
func on_begin() -> void:
	from_move = sys.net.param1
	feint_type = sys.net.param0
	var last = sys.last_attack_motion
	lock_out_time = 0.4		# 0x3ecccccd, the value stored before LastAttackMotion is read
	if last != null:
		# how late in the feintable part of the windup the feint came: 0 = at attack start, 1 = at the deadline
		var deadline: float = (last.windup_end - last.lag_induction) - last.ad.feint_window
		var span: float = deadline - last.start_time
		var rem := maxf(deadline - start_time, 0.0)
		var late := 1.0
		if 0.0 < span:
			late = clampf((span - rem) / span, 0.0, 1.0)
		lock_out_time = last.ai.feint_lock_out					# AttackInfo+0x20 FeintLockOut
		if last.type == CombatEnums.AttackType.COMBO:
			lock_out_time = maxf(lock_out_time, last.ai.combo_windup_increase)
		var w: WeaponData = sys.weapon						# RightHandEquipment as AMordhauWeapon
		if w != null:
			var inr := fd.strike_and_stab_lockout_in
			var outr := fd.strike_and_stab_lockout_out
			strike_lockout = _map_clamped(w.strike.windup, inr, outr)	# Weapon+0x13f0 = StrikeAttack.Windup
			stab_lockout = _map_clamped(w.stab.windup, inr, outr)		# Weapon+0xf50 = StabAttack.Windup
			if last.type == CombatEnums.AttackType.COMBO:
				strike_lockout = maxf(last.ai.combo_windup_increase, strike_lockout)
				stab_lockout = maxf(last.ai.combo_windup_increase, stab_lockout)
			var curve := fd.strike_and_stab_late_feint_adjustment_curve
			if curve != "":
				var adj := CombatData.curve_eval(CombatData.curve_keys(curve), late)
				strike_lockout += adj
				stab_lockout += adj
			stab_lockout = stab_lockout + fd.extra_stab_lockout
			strike_lockout = fd.extra_strike_lockout + strike_lockout
			lock_out_time = maxf(strike_lockout, stab_lockout)
		last.on_feinted(lock_out_time)
		sys.next_kick_time = fd.slow_kick_duration + start_time	# MotionSystem+0xb0
		end_time = lock_out_time + start_time
	else:
		sys.next_kick_time = fd.slow_kick_duration + start_time
		end_time = lock_out_time + start_time
	sys.last_feinted_motion = self

# from UFeintedMotion::OnTick_Implementation rva=0x16683a0
func on_tick(_dt: float) -> void:
	if b_has_queued_move and queue_execute_time <= now():
		sys.assign_net_attack_motion(CombatEnums.AttackType.REGULAR, queued_move, queued_angle)

# from UFeintedMotion::OnEnded_Implementation rva=0x1663810
func on_ended() -> void:
	if b_has_queued_move and sys.motion == self:
		sys.assign_net_attack_motion(CombatEnums.AttackType.REGULAR, queued_move, queued_angle)
		if sys.motion != self:
			return
	super.on_ended()

# from UFeintedMotion::ProcessAttack_Implementation rva=0x166b590
func process_attack(m: int, angle: float) -> bool:
	var req: MotionDefs.Attack = sys.attack_motion_defaults(m)
	if req != null and req.b_can_attack_from_feint_lockout:	# CDO of the requested class
		b_has_queued_move = false
		sys.assign_net_attack_motion(CombatEnums.AttackType.REGULAR, m, angle)
		if sys.motion != self:
			return true
	var t := now()
	var delay := 0.0
	if strike_lockout != 0.0 and stab_lockout != 0.0:
		if CombatEnums.is_strike(m):
			delay = strike_lockout - lock_out_time
		elif CombatEnums.is_stab(m):
			delay = stab_lockout - lock_out_time
	if delay + end_time <= t + fd.queue_window:
		queue_execute_time = delay + end_time
		queued_angle = angle
		queued_move = m
		b_has_queued_move = true
	return false

# from UFeintedMotion::ProcessBlock_Implementation rva=0x166bac0: parry at once, block side from the feinted move
func process_block(bt: int) -> bool:
	var b := bt
	if CombatEnums.is_left(from_move):
		b = 1 if bt == 0 else bt
	elif bt == 1:
		b = 0
	if not b_can_block:
		return false
	sys.assign_net_parry(b)
	return true

# from UFeintedMotion::ProcessFeint_Implementation rva=0x166bd10: drops the queued attack
func process_feint() -> bool:
	b_has_queued_move = false
	return false

func trace_fields() -> Dictionary:
	return {"lockout": lock_out_time, "end": end_time, "queued": b_has_queued_move}

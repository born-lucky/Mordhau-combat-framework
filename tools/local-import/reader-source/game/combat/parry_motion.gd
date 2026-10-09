# parry_motion.gd - UParryMotion (extract/native/decomp/UParryMotion.cpp).
# Constants: MotionDefs.Parry (CombatData.motion_def of the parry Blueprint); weapon values (ParryWindowOffset,
# bIsParryHeld, ParryHeldStaminaDrain, BlockStaminaNegation/Clamp) from the parrying weapon's WeaponData.
# The dynamic param byte (MotionSystem NetMotion+5): low nibble = parry recovery request (1 Fail, 2 Miss),
# high nibble = TotalBlocks. Field offsets: types/UParryMotion.h.
# Not ported: animation/additives, shield-wall raise animation, forward parry geometry and the parry angles (see
# check_parry), the fists ragdoll + backwards impulse of a fists stamina break (ReceiveBlock: vcalls +0x8f0 / +0x998).
class_name ParryMotion
extends CombatMotion

var block_type := 0				# +0x532
var b_is_shield_wall := false	# +0x46a
var b_is_block_holdable := false	# +0x539
var b_is_miss_parry := false	# +0x46b
var b_detected_any_non_friendly_attack := false	# +0x469
var b_requested_drop := false	# +0x533
var stage := 0					# +0x53a EParryStage
var total_blocks := 0			# +0x538
var parry_end := 0.0			# +0x53c
var recovery_start_time := 0.0	# +0x540
var riposte_window_start := 0.0	# +0x544
var b_has_queued_move := false	# +0x558
var queued_move_time := 0.0		# +0x55c
var queued_angle := 0.0			# +0x560
var queued_move := 0			# +0x564
var held_stamina_drain := 0.0	# +0x568
var cumulative_stamina_drain := 0.0	# +0x56c
var parry_up_time := 0.0		# +0x514 (instance copy, OnBegin adjusts it)
var parry_recovery_time := 0.0	# +0x500
var minimum_held_parry_time := 0.0	# +0x4f8
var non_held_parry_extension_time := 0.0	# +0x4b0
var backpedal_speed_factor := 0.0	# UMordhauMotion BackpedalSpeedFactor (movement; set from WeaponPtr)
var recovery_type := -1			# EParryRecoveryType passed to EnterParryRecovery
var last_blocked_move := -1		# EAttackMove of the attack this parry last blocked (the FNetBlock BlockedMove that
								# CheckAttackParry rva=0x1616cd0 sends the defender via AssignNetBlock; not a UParryMotion field)

var pd: MotionDefs.Parry		# this class's defaults (= def, typed)

func _init(owner_sys: MotionSystem, definition: MotionDefs.Base) -> void:
	super(owner_sys, definition)
	pd = definition as MotionDefs.Parry

func kind() -> String:
	return "Parry"

# from UParryMotion::CanInitiateMotion_Implementation rva=0x164ea00: only a parry that blocked something (TotalBlocks
# +0x538 != 0) and is not a shield wall (+0x46a) lets another motion start
func can_initiate_motion(_new_kind: String) -> bool:
	if total_blocks != 0:
		return not b_is_shield_wall
	return false

# from UParryMotion::GetMovementRestriction rva=0x165cc30 (disasm 0x14165cc45..0x14165cc83): Recovery stage (+0x53a == 1)
# -> SuccessfulParryRecoveryMovementRestriction (+0x4a4) when TotalBlocks (+0x538) > 0, else FailedParry... (+0x4a5);
# Parry stage -> WeaponPtr's BlockMovementRestriction (+0x1a34), or the motion's own field without a WeaponPtr
func get_movement_restriction() -> int:
	if stage == CombatEnums.ParryStage.RECOVERY:
		if total_blocks != 0:
			return pd.successful_parry_recovery_movement_restriction
		return pd.failed_parry_recovery_movement_restriction
	var wd: WeaponData = sys.parry_weapon()
	if wd != null:
		return CombatEnums.movement_restriction_of(wd.block_movement_restriction)
	return movement_restriction


# from UParryMotion::OnBegin_Implementation rva=0x1660f70
func on_begin() -> void:
	parry_up_time = pd.parry_up_time
	parry_recovery_time = pd.parry_recovery_time
	minimum_held_parry_time = pd.minimum_held_parry_time
	non_held_parry_extension_time = pd.non_held_parry_extension_time
	block_type = sys.net.param0
	sys.last_parry_motion = self
	if block_type == CombatEnums.BlockType.SHIELD_WALL:
		b_is_shield_wall = true
		block_type = 0
	var wd: WeaponData = sys.parry_weapon()	# WeaponPtr (+0x550)
	var offset := 0.0
	b_is_block_holdable = wd.b_is_parry_held
	if not b_is_block_holdable and not b_is_shield_wall:
		offset = wd.parry_window_offset
	held_stamina_drain = wd.parry_held_stamina_drain
	backpedal_speed_factor = wd.parry_backpedal_speed_factor	# -> UMordhauMotion BackpedalSpeedFactor
	# (all four reads are from WeaponPtr = the left-hand item when it IsA AMordhauWeapon: UParryMotion::OnBegin_Implementation disasm 0x141661212
	# [rsi+0x1a08], 0x14166121b [rsi+0x1a0c], 0x141661241 [rsi+0x19b0], 0x141661250 [rsi+0x1a10], rsi = WeaponPtr)
	# parrying out of an unfinished, unhit riposte costs that riposte's FeintCost (feint-to-parry)
	var cf = coming_from
	var from_live_riposte: bool = cf is AttackMotion and cf.type == CombatEnums.AttackType.RIPOSTE 		and cf.stage != CombatEnums.Stage.RECOVERY and not cf.b_has_hit
	if from_live_riposte:
		sys.offset_stamina(-cf.ai.feint_cost)
		if b_is_block_holdable:
			minimum_held_parry_time = pd.minimum_held_riposte_parry_time
	if b_is_shield_wall:
		parry_up_time = pd.shield_wall_raise_time
	parry_recovery_time = parry_recovery_time - offset
	parry_up_time = offset + parry_up_time
	parry_end = start_time + CombatConstants.parry_open_end
	end_time = parry_end

# from UParryMotion::OnTick_Implementation rva=0x1668980 (state part)
func on_tick(dt: float) -> void:
	var t := now()
	if stage == CombatEnums.ParryStage.PARRY and parry_up_time + start_time + non_held_parry_extension_time < t \
			and not b_is_block_holdable and not b_is_shield_wall:
		stop_holding()		# OnTick writes the same request byte StopHolding builds (see stop_holding)
	if stage != CombatEnums.ParryStage.PARRY:
		return
	if 0.0 < held_stamina_drain:
		cumulative_stamina_drain = held_stamina_drain * dt + cumulative_stamina_drain
		if 1.0 < cumulative_stamina_drain:
			cumulative_stamina_drain -= 1.0
			sys.offset_stamina(-1)
	_detect_attack_traces()
	# held parry released after MinimumHeldParryTime -> ServerDropParry. The decompiled test also requires the
	# character to be locally controlled, or the authority without a controller (vcall +0x9c8 / Role 3):
	# MotionSystem.drops_parry_locally (always true without game/net).
	if not b_is_shield_wall and b_is_block_holdable and sys.drops_parry_locally() and not sys.holding_block \
			and not b_requested_drop and minimum_held_parry_time + start_time < t:
		b_requested_drop = true
		sys.server_drop_parry()

# UParryMotion::OnTick_Implementation rva=0x1668980, Parry stage, after the held drain (decomp UParryMotion.cpp
# 1290-1381): on the authority (Role 3) while !bDetectedAnyNonFriendlyAttack, each AMordhauGameState
# AttackTracesMemory entry (attack_motion.gd maybe_store_last_trace) whose Owner is another character sharing this
# instance is line-traced against this character's BlockCollider (vcall +0x848 = UPrimitiveComponent::
# LineTraceComponent, types/UShapeComponent.h); a hit by a non-friendly owner (AMordhauGameState::IsFriendly
# rva=0x16c6740 false) sets bDetectedAnyNonFriendlyAttack, so the parry's end requests the miss-parry recovery
# (MissParryRecoveryTime) instead of ParryRecoveryTime. Needs sys.actor_xf (the collider's frame); headless fighters
# without one skip it. Friendly owners: CombatState.is_friendly (teams of the running game mode; none = not friendly).
# (CombatState.remove_fighter drops a leaving fighter's entries, so every entry's owner is a fighter in the world.)
func _detect_attack_traces() -> void:
	if b_detected_any_non_friendly_attack or sys.actor_xf == null:
		return
	var bc := CombatData.block_collider()
	var box_xf: Transform3D = (sys.actor_xf as Transform3D) * Transform3D(Basis.IDENTITY, bc.centre)
	var w := sys.world
	for e in w.attack_traces_memory:
		if e.owner == sys.id:
			continue
		if w.is_friendly(w.fighter_by_id(e.owner), sys):
			continue
		if MeleeHit.segment_box(e.start, e.end, box_xf, bc.half) >= 0.0:
			b_detected_any_non_friendly_attack = true
			return

# from UParryMotion::StopHolding rva=0x166db50 (disassembled 0x14166db50..0x14166dbc9;
# AMordhauCharacter::ServerDropParry_Implementation tail-jumps here):
#   detected = TotalBlocks (+0x538) == 0 ? bDetectedAnyNonFriendlyAttack (+0x469) : false     (cmp / cmovbe)
#   AssignNetMotionDynamicParam((detected ? 2 : 1) | (NetMotion.MotionDynamicParam +0xc9 & 0xf0))   (jmp 0x1414b35f0)
# on_dynamic_param_changed reads the low nibble back: 1 = parry recovery, 2 = miss-parry recovery.
func stop_holding() -> void:
	var detected := b_detected_any_non_friendly_attack if total_blocks == 0 else false
	var request := 2 if detected else 1
	sys.assign_net_motion_dynamic_param(request | (sys.net.dynamic_param & 0xf0))

# from UParryMotion::OnDynamicParamChanged_Implementation rva=0x1663210
func on_dynamic_param_changed(old: int, nv: int) -> void:
	var t := now()
	var req := CombatEnums.lo(nv)			# recovery request
	total_blocks = CombatEnums.hi(nv)
	if req != 0 and stage == CombatEnums.ParryStage.PARRY:
		parry_end = start_time
		if total_blocks == 0 or b_is_shield_wall:
			var rec := 0.0
			var kind_ := CombatEnums.ParryRecovery.FAIL
			if not b_is_shield_wall and not b_is_block_holdable and pd.miss_parry_recovery_time != 0.0:
				b_is_miss_parry = req == 2
				if req == 2:
					rec = pd.miss_parry_recovery_time
					kind_ = CombatEnums.ParryRecovery.MISS
				else:
					rec = parry_recovery_time
			else:
				b_is_miss_parry = false
				rec = parry_recovery_time
				if b_is_shield_wall:
					rec = pd.shield_wall_recovery_time
				elif b_is_block_holdable:
					rec = pd.held_parry_recovery_time
			end_time = (rec + t) - expected_delay
			_enter_parry_recovery(kind_)
		else:
			var rec2 := pd.held_parry_success_recovery_time if b_is_block_holdable else pd.parry_success_recovery_time
			end_time = (rec2 + t) - expected_delay
			_enter_parry_recovery(CombatEnums.ParryRecovery.SUCCESS)
	if total_blocks == CombatEnums.hi(old):
		return
	# a new block landed: open the riposte window; fire a riposte queued shortly before the block
	var up := CombatConstants.shield_wall_riposte_window	# shield-wall branch
	if not b_is_shield_wall:
		up = parry_up_time
		if not b_is_block_holdable and CombatEnums.hi(old) != 0:
			return
		riposte_window_start = t
		if not b_is_block_holdable:
			non_held_parry_extension_time = pd.non_held_parry_extension_and_riposte_window_extra
		# (vehicle rider: a separate .rdata constant; no vehicles here, not ported)
	else:
		riposte_window_start = t
	if not b_has_queued_move:
		return
	if t <= up + queued_move_time:
		sys.assign_net_attack_motion(CombatEnums.AttackType.RIPOSTE, queued_move, queued_angle)

# from UParryMotion::EnterParryRecovery rva=0x1650540 (state part; the rest sets anim rates)
func _enter_parry_recovery(kind_: int) -> void:
	recovery_type = kind_
	recovery_start_time = now()
	stage = CombatEnums.ParryStage.RECOVERY

# riposte window length of UParryMotion::ProcessAttack_Implementation rva=0x166b870
func riposte_window() -> float:
	var base := pd.riposte_window_base
	if b_is_shield_wall:
		return CombatConstants.shield_wall_riposte_window
	if not b_is_block_holdable:
		return base + pd.non_held_parry_extension_and_riposte_window_extra
	if sys.holding_block:
		return base + pd.held_riposte_window_extra
	return base

# from UParryMotion::ProcessAttack_Implementation rva=0x166b870
func process_attack(m: int, angle: float) -> bool:
	var t := now()
	b_has_queued_move = true
	queued_move = m
	if not b_is_shield_wall:
		if m == CombatEnums.Move.STAB and block_type == 0:
			queued_move = CombatEnums.Move.ALT_STAB
		elif m == CombatEnums.Move.ALT_STAB and block_type == 1:
			queued_move = CombatEnums.Move.STAB
	elif m == CombatEnums.Move.ALT_STAB:
		queued_move = CombatEnums.Move.STAB
	queued_angle = angle
	if CombatEnums.is_stab(queued_move):
		queued_angle = 0.0
	queued_move_time = t
	var win := riposte_window()
	if (b_is_shield_wall and parry_up_time + start_time < t and stage != CombatEnums.ParryStage.RECOVERY) \
			or (total_blocks != 0 and t <= win + riposte_window_start):
		sys.assign_net_attack_motion(CombatEnums.AttackType.RIPOSTE, queued_move, queued_angle)
		return sys.motion != self
	if b_is_shield_wall:
		return false
	if b_is_block_holdable:
		if not sys.holding_block and (total_blocks != 0 or minimum_held_parry_time + start_time < t):
			return super.process_attack(m, angle)
		return false
	if total_blocks != 0:
		return super.process_attack(m, angle)
	return false

# from UParryMotion::ProcessBlock_Implementation rva=0x166bbb0
func process_block(bt: int) -> bool:
	if not b_is_block_holdable:
		if total_blocks == 0:
			return false
	elif stage != CombatEnums.ParryStage.RECOVERY:
		return false
	if b_is_shield_wall and stage == CombatEnums.ParryStage.PARRY:
		return false
	var b := bt
	if bt < 2 and block_type < 2:
		b = block_type
	if not b_can_block:
		return false
	sys.assign_net_parry(b)
	return true

# from UParryMotion::ProcessFeint_Implementation rva=0x166bd40: shield wall lowered after its raise time
func process_feint() -> bool:
	if b_is_shield_wall and stage == CombatEnums.ParryStage.PARRY and not b_requested_drop and parry_up_time + start_time < now():
		sys.server_drop_parry()
		b_requested_drop = true
		return true
	return false

# from UParryMotion::CheckParry rva=0x164ed40 (decomp UParryMotion.cpp 975-1275): can this parry (the defender's
# LastParryMotion, current or not) block the `attacker`'s attack? The non-geometry gates, in the exe's order:
#   Parry stage: a shield wall only after ParryUpTime. Recovery stage (the parry has left: OnLeave enters recovery):
#   only while the current motion IsA UFlinchMotion (parry in flinch), not a shield wall, now <= LeaveTime +
#   ParryInFlinchDurationMax and, not holdable, now <= StartTime + ParryUpTime + NonHeldParryExtensionTime.
#   WeaponPtr exists; the attacker's motion IsA UAttackMotion with a Weapon; friendly = IsFriendly(defender, attacker);
#   (attack Stage == Release or not friendly); masks: (WeaponPtr ParryMask | 2 when neither holdable nor shield wall)
#   & attack Weapon AttackMask != 0; attack Stage != Windup; ModifyParryResult (Blueprint event, native pass-through
#   AMordhauCharacter::ModifyParryResult_Implementation rva=0x154fc80; only BP_HordeEnemy overrides it).
# Geometry (the BlockCollider forward test, BlockedAttacks memory for body hits, MaxParryAngle / MaxParryWeaponAngle
# directional checks) is the caller's contact: a trace or scripted contact counts as passing it. A BlockCollider hit
# by a non-friendly attack sets bDetectedAnyNonFriendlyAttack; the contact stands for that hit too.
func check_parry(attacker: MotionSystem) -> bool:
	if attacker == null:
		return false
	var t := now()
	if stage == CombatEnums.ParryStage.PARRY:
		if b_is_shield_wall and t < parry_up_time + start_time:
			return false
	else:
		if not (sys.motion is FlinchMotion) or b_is_shield_wall:
			return false
		if pd.parry_in_flinch_duration_max + leave_time < t:
			return false
		if not b_is_block_holdable and parry_up_time + start_time + non_held_parry_extension_time < t:
			return false
	var wd: WeaponData = sys.parry_weapon()		# WeaponPtr (+0x550)
	if wd == null:
		return false
	var atk = attacker.motion
	if not (atk is AttackMotion) or atk.weapon == null:
		return false
	var friendly: bool = sys.world.is_friendly(sys, attacker)
	if not (atk.stage == CombatEnums.Stage.RELEASE or not friendly):
		return false
	var mask := wd.parry_mask
	if not b_is_block_holdable and not b_is_shield_wall:
		mask = mask | 2
	if (mask & atk.weapon.attack_mask) == 0:
		return false
	if not friendly:
		b_detected_any_non_friendly_attack = true
	if atk.stage == CombatEnums.Stage.WINDUP:
		return false
	return true

# from UParryMotion::OnLeave_Implementation rva=0x1665b10 (state part): a parry left before its recovery enters it
# (EnterParryRecovery(TotalBlocks == 0): Fail without a block, else Success), which is what CheckParry's parry in
# flinch tests. (The BlockCollider is disabled unless the new motion is a UFlinchMotion: geometry, not modelled.)
func on_leave(_interrupted: bool) -> void:
	if stage != CombatEnums.ParryStage.RECOVERY:
		_enter_parry_recovery(CombatEnums.ParryRecovery.FAIL if total_blocks == 0 else CombatEnums.ParryRecovery.SUCCESS)

# from UParryMotion::ReceiveBlock rva=0x166bf90. drain = float(int(attacker AttackInfo.StaminaDrain))
# (+ ExtraStaminaDrainVsHeldBlock if this parry is holdable): disassembly 0x14164f2a3..0x14164f308 of CheckParry.
# ChamberFTPExtraStaminaDrain (decomp UParryMotion.cpp ReceiveBlock, the `while (i < 5)` loop): when the attacker's
# motion IsA UAttackMotion, walk this character's LastAttackMotion (+0x108) and its PreviousLastAttackMotion links, at
# most 5: the first one that CheckChamberIsValidIgnoreTiming(attacker) (a Morph: its morphed-from attack) and whose
# StartTime >= attacker WindupEnd (+0x1090) - its ChamberWindow costs (int)-ChamberFTPExtraStaminaDrain (+0x46c), once.
# (The port cuts motion links two hops from a root, MotionSystem._bound_history, so at most 3 links are visited here.)
func receive_block(drain: float, attacker_move: int, attacker = null) -> void:
	var t := now()
	last_blocked_move = attacker_move
	if t <= sys.easy_parry_until_time:
		drain = float(int(pd.easy_parry_stamina_cost))
	if attacker != null and attacker.motion is AttackMotion:
		var old = sys.last_attack_motion
		for _i in 5:
			if old == null:
				break
			if old.chamber_valid_ignore_timing(attacker):
				if old.type == CombatEnums.AttackType.MORPH and old.previous_last_attack != null:
					old = old.previous_last_attack
				if attacker.motion.windup_end - old.ad.chamber_window <= old.start_time:
					sys.offset_stamina(int(-pd.chamber_ftp_extra_stamina_drain))
					break
			old = old.previous_last_attack
	if drain <= 0.0:
		if drain < 0.0:
			sys.offset_stamina(int(-drain))
	elif sys.motion == self:
		var wd: WeaponData = sys.parry_weapon()	# WeaponPtr (+0x550)
		var drain_clamp := wd.block_stamina_clamp
		var v: float = drain - wd.block_stamina_negation
		var f: float = drain_clamp.x
		if drain_clamp.x <= v:
			f = v if v < drain_clamp.y else drain_clamp.y
		if b_is_shield_wall:
			f = float(-(UeMath.cvtss2si(CombatConstants.shield_wall_drain_round_bias - (f + f) * pd.shield_wall_stamina_drain_factor) >> 1))
		sys.offset_stamina(int(-f))
	if sys.stamina_byte() != 0 or attacker_move == CombatEnums.Move.RANGED:
		if sys.net_rep != null:		# game/net: AssignNetBlock(Parry) + ApplyBackwardsKnockbackIfNotInKnockback
			sys.net_rep.on_receive_block(attacker_move, b_is_shield_wall)
		if sys.motion == self:
			sys.easy_parry_until_time = t + pd.easy_parry_duration
			var step := 1 if total_blocks + 1 < 11 else -1
			sys.assign_net_motion_dynamic_param(CombatEnums.pack(step + total_blocks, CombatEnums.lo(sys.net.dynamic_param)))
	else:
		_stamina_break(attacker_move, attacker)

# The stamina-break branch of UParryMotion::ReceiveBlock rva=0x166bf90 (stamina byte 0, not ranged): stun = the
# LastBlockedCharacter HasPerk(0x15) and WeaponPtr is not a shield (perks are not ported: false); disarm = true; then
# WeaponPtr !bAllowDrop (+0xcb8), bDestroyEquipmentOnDeath (+0x11e0) or bAlwaysStunInsteadOfDisarm (+0x1219, both of
# the parrier) -> stun, no disarm. direction = -90 for a LeftStrike, else 90. Not fists: disarm -> stamina +=
# BlockStaminaRecover (+0x518) unless WeaponPtr IsA AMordhauShield, Disarmed(direction); stun -> += BlockStaminaRecover,
# Stunned(direction, 0, disarm). Fists (WeaponPtr IsA AFistsWeapon): += BlockStaminaRecover, then ragdoll and a -500
# impulse (not ported: no ragdoll). The attacker learns it through HandleWasParried (melee_hit.gd).
func _stamina_break(attacker_move: int, _attacker) -> void:
	var wd: WeaponData = sys.parry_weapon()
	var eq: EquipmentDef = sys.parry_weapon_equip()
	var stun := false
	var disarm := true
	if (wd != null and not wd.b_allow_drop) or sys.character.b_destroy_equipment_on_death 			or sys.character.b_always_stun_instead_of_disarm:
		disarm = false
		stun = true
	var direction := -90.0 if attacker_move == CombatEnums.Move.LEFT_STRIKE else 90.0
	var is_shield: bool = eq != null and eq.is_shield
	var is_fists: bool = eq != null and CombatData.is_class_of(eq.native, "AFistsWeapon")
	sys.trace_event("parry stamina break: %s" % ("fists" if is_fists else ("stun" if stun else "disarm")))
	if is_fists:
		sys.offset_stamina(pd.block_stamina_recover)
		sys.world.emit_event({"kind": "stamina_break", "who": sys.name, "fists": true, "stun": false, "disarm": false})
		return
	if not stun:
		if not is_shield:
			sys.offset_stamina(pd.block_stamina_recover)
		sys.assign_net_disarmed(direction)
	else:
		sys.offset_stamina(pd.block_stamina_recover)
		sys.assign_net_stunned(direction, 0, disarm)
	sys.world.emit_event({"kind": "stamina_break", "who": sys.name, "stun": stun, "disarm": disarm})

func trace_fields() -> Dictionary:
	return {"stage": CombatEnums.PARRY_STAGE_NAMES[stage], "blocks": total_blocks, "end": end_time,
		"recovery": recovery_type, "queued": b_has_queued_move}

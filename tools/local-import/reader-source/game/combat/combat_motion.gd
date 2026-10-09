# combat_motion.gd - UMordhauMotion (base of every motion) and UIdleMotion.
# A motion is the character's one current action. MotionSystem owns it; time is the world clock in seconds
# (UWorld TimeSeconds, read at world+0x598 by every motion function below).
# Field names are the UE ones from extract/native/types/UMordhauMotion.h; class defaults are the typed record
# MotionDefs.Base (or its Attack / Parry / ... subclass) read by CombatData.motion_def.
# Ownership (no reference cycles, so a dropped CombatState is freed by refcounting alone): the MotionSystem owns its
# motions (MotionSystem.motion / last_*_motion); a motion reaches its owner through a WeakRef (`sys`), never a strong
# reference. Links to older motions (coming_from, AttackMotion.previous_last_attack, BlockedMotion.from_attack) are
# strong but bounded: MotionSystem._bound_history cuts the links of every motion two hops from the owner's roots.
class_name CombatMotion
extends RefCounted

var _owner_ref: WeakRef			# -> MotionSystem (UMordhauMotion Outer = UMotionSystemComponent)
var sys: MotionSystem:			# the MotionSystem that owns this motion (UMordhauMotion GetOwner); null once it is freed
	get: return _owner_ref.get_ref() if _owner_ref != null else null
var def: MotionDefs.Base	# class defaults (native ctor + Blueprint CDO chain), shared read-only record
var bp := ""		# Blueprint class path, "" for a native-only motion
var start_time := 0.0		# +0x4c StartTime
var end_time := 0.0			# +0x50 EndTime
var leave_time := 0.0		# +0x54 LeaveTime
var expected_delay := 0.0	# +0x48 ExpectedDelay (network; 0 here: one machine, authority)
var coming_from = null		# +0x30 ComingFromMotion
var b_is_flinchable := true	# +0x60
var b_can_attack := true	# +0x6d
var b_can_block := true		# +0x6e
var b_can_emote := false	# +0x6c
var b_blocks_regen := false	# +0x88 bBlocksRegen (AMordhauCharacter::LODTick stops stamina regen while set)
var movement_restriction := 0	# +0x61 MovementRestriction (CombatEnums.MovementRestriction), read by the movement layer
								# through UMordhauMotion::GetMovementRestriction rva=0x165cc20 (returns the field)

func _init(owner_sys: MotionSystem, definition: MotionDefs.Base) -> void:
	_owner_ref = weakref(owner_sys)
	def = definition
	bp = def.path
	b_is_flinchable = def.b_is_flinchable
	b_can_attack = def.b_can_attack
	b_can_block = def.b_can_block
	b_can_emote = def.b_can_emote
	b_blocks_regen = def.b_blocks_regen
	movement_restriction = def.movement_restriction

func kind() -> String:
	return "Motion"

func now() -> float:
	return sys.now()

# from UMordhauMotion::Tick rva=0x166ee80: OnTick, then OnEnded once EndTime <= now
func tick(dt: float) -> void:
	on_tick(dt)
	if sys.motion == self and end_time <= now():
		on_ended()

func on_begin() -> void:
	pass

func on_tick(_dt: float) -> void:
	pass

func on_leave(_interrupted: bool) -> void:
	pass

func on_dynamic_param_changed(_old: int, _new: int) -> void:
	pass

# from UMordhauMotion::OnEnded_Implementation rva=0x16638a0: if still current, ChangeMotion(UIdleMotion)
func on_ended() -> void:
	if sys.motion == self:
		sys.change_to_idle()

# from UMordhauMotion::ProcessAttack_Implementation rva=0x166b7c0
func process_attack(move: int, angle: float) -> bool:
	if not b_can_attack:
		return false
	if now() < sys.next_attack_time:			# MotionSystem+0xb4 NextAttackTime
		return false
	sys.assign_net_attack_motion(CombatEnums.AttackType.REGULAR, move, angle)
	return sys.motion != self

# from UMordhauMotion::ProcessBlock_Implementation rva=0x166bb20: FNetMotion{MotionType 2 (Parry), Param0 block type}
func process_block(block_type: int) -> bool:
	if not b_can_block:
		return false
	sys.assign_net_parry(block_type)
	return true

# from UMordhauMotion::GetAttackCompensationStartTime rva=0x165c630: !bCanAttack -> EndTime, else 0
func get_attack_compensation_start_time(_m: int) -> float:
	return end_time if not b_can_attack else 0.0

# from UMordhauMotion::ProcessFeint_Implementation rva=0x7bf520 (shared "return 0" stub)
func process_feint() -> bool:
	return false

# UMordhauMotion::GetMovementRestriction rva=0x165cc20 returns the field; UAttackMotion / UKickMotion / UParryMotion
# override it (their classes below). The movement layer reads this, not the field.
func get_movement_restriction() -> int:
	return movement_restriction

# from UMordhauMotion::CanInitiateMotion_Implementation rva=0x7bf520 (shared stub `xor al, al; ret`): a motion lets no
# other motion start unless its class overrides this. new_kind = the requested motion's kind() name ("EquipmentModeSwitch",
# "EquipmentSwitch", "InteractWith", ...; classes not ported here are only named).
func can_initiate_motion(_new_kind: String) -> bool:
	return false

func trace_fields() -> Dictionary:
	return {}


# UIdleMotion: from UIdleMotion::OnBegin_Implementation rva=0x1660d30 (EndTime = FLT_MAX 0x7f7fffff, bCanEmote = 1,
# bBlocksRegen = 0) and OnTick_Implementation rva=0x16684c0 (ComingFromMotion cleared 0.5 s after start;
# CombatConstants.idle_coming_from_timeout = 0.5).
class Idle extends CombatMotion:
	func kind() -> String:
		return "Idle"

	func on_begin() -> void:
		end_time = 3.4028234663852886e38	# immediate 0x7f7fffff (FLT_MAX) in the decompiled store
		b_can_emote = true
		b_blocks_regen = false

	func on_tick(_dt: float) -> void:
		if start_time + CombatConstants.idle_coming_from_timeout < now():
			coming_from = null

	# UIdleMotion::CanInitiateMotion_Implementation rva=0x7bf3e0 (shared stub `mov al, 1; ret`): anything may start
	func can_initiate_motion(_new_kind: String) -> bool:
		return true

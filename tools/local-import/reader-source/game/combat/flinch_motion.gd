# flinch_motion.gd - UFlinchMotion: a hit character staggering (extract/native/decomp/UFlinchMotion.cpp).
# Constants: CombatData.class_defaults(BP_FlinchMotion) = UFlinchMotion::UFlinchMotion rva=0x1646930 (see
# NativeCtor: FlinchDuration +0xa4 = 0x3f666666 0.9, bCanAttack +0x6d = 0) + CDO.
# NetMotion (FNetMotion::Flinched rva=0x14bb820): MotionType 3, Param0 = PackFloat(angle / 180, [-1, 1]),
# Param1 = flag, Param2 = PackFloat(attack FlinchDurationModifier, [0.5, 2]), DynamicParam = PackFloat(attack
# FlinchSpeedModifier, [0, 1]).
class_name FlinchMotion
extends CombatMotion

var speed_factor := 1.0		# +0x64 SpeedFactor

func kind() -> String:
	return "Flinch"

# from UFlinchMotion::OnBegin_Implementation rva=0x16606a0 (timing part; additives, camera shake not ported)
func on_begin() -> void:
	var inv255 := CombatConstants.byte_to_unit
	# Param2 -> duration modifier in [0.5, 2] (x 1.5 + 0.5): used only for the camera shake scale in this function
	speed_factor = clampf(float(sys.net.dynamic_param) * inv255, 0.0, 1.0) * def.speed_factor
	end_time = ((def as MotionDefs.Flinch).flinch_duration + start_time) - expected_delay
	sys.next_attack_time = end_time						# MotionSystem +0xb4
	sys.next_kick_time = end_time + CombatConstants.flinch_kick_delay	# +0xb0, + 0.25
	sys.easy_parry_until_time = start_time + CombatConstants.easy_parry_window	# character +0xe94, + 0.5
	# OnBegin (decomp UFlinchMotion.cpp 197-200): bCanBlock = true, then false while ParryLockOutTime (+0xa8) > 0
	b_can_block = not (0.0 < (def as MotionDefs.Flinch).parry_lock_out_time)

# from UFlinchMotion::OnTick_Implementation rva=0x1668430 (disasm 0x141668456..0x1416684a4): bCanBlock (+0x6e) comes back
# once StartTime + ParryLockOutTime <= now; after StartTime + 0.45 (CombatConstants.flinch_recover_cosmetics, strictly
# later) bDisablesAtmospherics (+0x6f) and bDisablesOffhandIK (+0x72) are cleared, OffhandIKChangeSpeed (+0x78) = 2.5
# (immediate 0x40200000) and, once (bHasPlayedFoley +0xa1), AMordhauCharacter::PlayNonSnappyArmorFoley runs: the
# `flinch_foley` event for the audio layer. The ctor (rva=0x1646930) sets both flags and OffhandIKChangeSpeed 10.
var b_disables_atmospherics := true		# UMordhauMotion +0x6f
var b_disables_offhand_ik := true		# UMordhauMotion +0x72
var offhand_ik_change_speed := 10.0		# UMordhauMotion +0x78 (UFlinchMotion::UFlinchMotion rva=0x1646930 store 10.0)
var b_has_played_foley := false			# +0xa1

func on_tick(_dt: float) -> void:
	var t := now()
	if start_time + (def as MotionDefs.Flinch).parry_lock_out_time <= t:
		b_can_block = true
	if start_time + CombatConstants.flinch_recover_cosmetics < t:
		b_disables_atmospherics = false
		b_disables_offhand_ik = false
		offhand_ik_change_speed = 2.5		# UFlinchMotion::OnTick_Implementation rva=0x1668430 immediate 0x40200000
		if not b_has_played_foley:
			sys.world.emit_event({"kind": "flinch_foley", "who": sys.name})
			b_has_played_foley = true

# from UFlinchMotion::CanInitiateMotion_Implementation rva=0x164e6e0: from a flinch only UInteractWithMotion,
# UEquipmentSwitchMotion and UEquipmentModeSwitchMotion (and their subclasses) may start
const _FLINCH_INITIATES := ["InteractWith", "EquipmentSwitch", "EquipmentModeSwitch"]

func can_initiate_motion(new_kind: String) -> bool:
	return _FLINCH_INITIATES.has(new_kind)

func trace_fields() -> Dictionary:
	return {"end": end_time}

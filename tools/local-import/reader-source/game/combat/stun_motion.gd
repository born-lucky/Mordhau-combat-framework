# stun_motion.gd - UStunMotion: a character whose stamina broke on a block or chamber and who keeps its weapon (or
# drops it when bWillDisarm) (extract/native/decomp/UStunMotion.cpp; fields types/UStunMotion.h).
# Constants: MotionDefs.Stun = UStunMotion::UStunMotion rva=0x16492e0 (StunDuration 1.3, StunGracePeriodExtraTime 0.4,
# bCanAttack / bCanBlock 0, bBlocksRegen 1, MovementRestriction 3) + BP_StunMotion CDO (StunDuration 1.075).
# NetMotion (FNetMotion::Stunned rva=0x14d86a0): MotionType 4, Param0 = PackFloat(direction / 180, [-1, 1]), Param1 =
# bone, Param2 = bShouldDisarm.
# Not ported (no gameplay effect here): the hurt yell, camera shake, force feedback, the animation montage, the turn-rate
# vcall +0x9e8 (180, 90) / (-1, -1) on leave (movement layer), the dropped item's physics impulse.
class_name StunMotion
extends CombatMotion

var b_will_disarm := false		# +0xa0
var direction := 0.0			# Param0 unpacked: byte / 255 x 360 - 180 (degrees)

func kind() -> String:
	return "Stun"

# from UStunMotion::OnBegin_Implementation rva=0x16628e0 (gameplay part): EndTime = StunDuration + StartTime -
# ExpectedDelay; bWillDisarm = Param2 != 0; with bWillDisarm, on authority, the item is dropped the way
# UDisarmedMotion drops it (MotionSystem.drop_for_disarm: the ComingFrom attack's Weapon, else the left-hand weapon,
# else the right-hand weapon)
func on_begin() -> void:
	var sd := def as MotionDefs.Stun
	end_time = (sd.stun_duration + start_time) - expected_delay
	b_will_disarm = sys.net.param2 != 0
	direction = clampf(float(sys.net.param0) * CombatConstants.byte_to_unit, 0.0, 1.0) * 360.0 - 180.0
	if b_will_disarm:
		sys.drop_for_disarm(coming_from)

# from UStunMotion::OnTick_Implementation rva=0x166a1e0 (disasm 0x14166a227..0x14166a237): MotionSystem
# NextAvailableStunTime (+0xb8) = now + StunGracePeriodExtraTime (+0xa4), every tick
func on_tick(_dt: float) -> void:
	sys.next_available_stun_time = now() + (def as MotionDefs.Stun).stun_grace_period_extra_time

func trace_fields() -> Dictionary:
	return {"end": end_time, "disarm": b_will_disarm}

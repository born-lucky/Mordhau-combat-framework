# disarmed_motion.gd - UDisarmedMotion: a character whose stamina broke on a block or chamber and who drops its
# weapon (extract/native/decomp/UDisarmedMotion.cpp; fields types/UDisarmedMotion.h).
# Constants: MotionDefs.Disarmed = UDisarmedMotion::UDisarmedMotion rva=0x1646630 (RecoveryTime 0.3, bCanAttack /
# bCanBlock 0, bBlocksRegen 1) + BP_DisarmedMotion CDO (RecoveryTime 0.45, MovementRestriction Walk).
# NetMotion (FNetMotion::Disarmed rva=0x14ba4a0): MotionType 7, Param0 = PackFloat(direction / 180, [-1, 1]).
# Not ported (no gameplay effect here): hurt yell, the bot's "help" voice command, camera shake, force feedback,
# StopAnim / additive "Disarm", the dropped item's physics impulse.
class_name DisarmedMotion
extends CombatMotion

var direction := 0.0			# Param0 unpacked (degrees)

func kind() -> String:
	return "Disarmed"

# from UDisarmedMotion::OnBegin_Implementation rva=0x165ee70 (disasm 0x14165ef81..0x14165efa2): EndTime =
# RecoveryTime + StartTime - ExpectedDelay; NextKickTime (+0xb0) = EndTime + 0.3; then, on authority (Role 3), the
# drop (MotionSystem.drop_for_disarm) and SwitchToFists when the right hand is empty and there is no CurrentVehicle
func on_begin() -> void:
	end_time = ((def as MotionDefs.Disarmed).recovery_time + start_time) - expected_delay
	sys.next_kick_time = end_time + CombatConstants.disarmed_kick_delay
	direction = clampf(float(sys.net.param0) * CombatConstants.byte_to_unit, 0.0, 1.0) * 360.0 - 180.0
	sys.drop_for_disarm(coming_from)

func trace_fields() -> Dictionary:
	return {"end": end_time}

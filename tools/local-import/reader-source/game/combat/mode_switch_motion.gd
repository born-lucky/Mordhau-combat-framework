# mode_switch_motion.gd - UEquipmentModeSwitchMotion (fp-anim r3, 2026-10-06; changed by the Rust-port fp-anim agent,
# authorized by the orchestrator: the exe wins over the earlier instant switch). Proof: state/proofs/fp_anim_items.md
# Round 2 item 4; Rust twin: core/crates/mordhau-core/src/combat/modeswitch.rs.
# The weapon-mode switch is a motion (extract/native/decomp/UEquipmentModeSwitchMotion.cpp):
#   ctor rva=0x1646800: bCanAttack false, bDisablesAtmospherics true, Stage1Duration 0.25, Stage2Duration 0.15 (no
#     Blueprint subclass: BP_MordhauCharacter Motions[11] is the native class).
#   OnBegin_Implementation rva=0x165fbb0: FirstStageEnd = StartTime + 0.25, SecondStageEnd = FirstStageEnd + 0.15,
#     EndTime = SecondStageEnd; SwitchType 2 / 3 when the hand changes; else 1 unless ModeSwitchAnimation is set and
#     both modes are two-handed (type 1: SecondStageEnd -= 0.25 and EndTime += 0.25, disasm 0x14165fe25..0x14165fe6a).
#   OnTick_Implementation rva=0x1667e00: TimeSeconds > StartTime + 0.15 and not finished -> FinishSwitch rva=0x165b4d0
#     (authority, CheckCanEquipAlt) -> SwitchModeAndReAttach (MotionSystem.switch_mode_and_reattach).
#   OnLeave_Implementation rva=0x16659a0: FinishSwitch if not done.
# Every alternate-mode equipment in the paks sets ModeSwitchAnimation, so "no montage" is taken as never true.
# Not ported here: the animation montage, offhand IK and the weapon mesh's virtual reparent (animation / grip side).
class_name ModeSwitchMotion
extends CombatMotion

const STAGE1_DURATION := 0.25	# +0xd8 Stage1Duration (ctor)
const STAGE2_DURATION := 0.15	# +0xdc Stage2Duration (ctor)
const FINISH_SWITCH_AFTER := 0.15	# OnTick decomp 702

var b_is_switching_to_alt := false
var switch_type := 0
var first_stage_end := 0.0
var second_stage_end := 0.0
var stage := 0
var b_has_finished_switch := false

# the native class defaults: UMordhauMotion ctor rva=0x1647f80 (bIsFlinchable, bCanAttack, bCanBlock true,
# MovementRestriction 0, SpeedFactor 1) + this class's ctor (bCanAttack false)
static func make_def() -> MotionDefs.Base:
	var d := MotionDefs.Base.new()
	d.path = ""
	d.native = "UEquipmentModeSwitchMotion"
	d.b_is_flinchable = true
	d.b_can_attack = false
	d.b_can_block = true
	d.b_can_emote = false
	d.b_blocks_regen = false
	d.speed_factor = 1.0
	d.movement_restriction = 0
	return d

func kind() -> String:
	return "EquipmentModeSwitch"

func on_begin() -> void:
	first_stage_end = start_time + STAGE1_DURATION
	second_stage_end = first_stage_end + STAGE2_DURATION
	end_time = second_stage_end
	var e: EquipmentDef = sys.weapon_equip
	if e != null:
		if e.b_is_right_handed != e.b_second_is_right_handed:
			# 2 + bIsRightHanded of the current mode (the record keeps the class defaults: the Second* field while in
			# the alternate mode)
			var rh := e.b_second_is_right_handed if sys.alternate_mode else e.b_is_right_handed
			switch_type = 3 if rh else 2
		elif not e.b_is_two_handed or not e.b_second_is_two_handed:
			switch_type = 1
			# UEquipmentModeSwitchMotion::OnBegin_Implementation disasm 0x14165fe25..0x14165fe6a
			second_stage_end = (start_time - first_stage_end) + second_stage_end
			end_time = (first_stage_end - start_time) + end_time
			first_stage_end = 0.0

func on_tick(_dt: float) -> void:
	if start_time + FINISH_SWITCH_AFTER < now() and not b_has_finished_switch:
		_finish_switch()
	if stage == 0 and first_stage_end < now():
		stage = 1
	if stage == 1 and second_stage_end < now():
		stage = 2

func on_leave(_interrupted: bool) -> void:
	if not b_has_finished_switch:
		_finish_switch()

# FinishSwitch rva=0x165b4d0
func _finish_switch() -> void:
	b_has_finished_switch = true
	if sys.role != 3 or sys.weapon == null or not sys.check_can_equip_alt(sys.weapon_equip):
		return
	sys.switch_mode_and_reattach()

func trace_fields() -> Dictionary:
	return {"to_alt": b_is_switching_to_alt, "finished": b_has_finished_switch}

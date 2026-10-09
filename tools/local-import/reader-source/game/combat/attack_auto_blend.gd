# attack_auto_blend.gd - the attack's animation-dependent timing inputs, as pure functions shared by the combat rules
# (attack_motion.gd) and the animation layer (game/anim/motion_anim.gd), so both see the same numbers.
# Engine-agnostic: class defaults come in as a MotionDefs.Attack record, poses as Transform3D values.
#
# settings(): UAttackMotion::PrepareAnimationData_Implementation rva=0x1637140: Riposte or PostClash take the Riposte*
#   early-release pair; the auto-blend flag is the per-type *AttacksUseAutoBlendIn; Riposte uses
#   RiposteAutoBlendOptimizeForwardSteps.
# search(): the AutoBlend loop of UAttackMotion::OnBegin_Implementation rva=0x162eda0 (after "bUseAutomaticBlendIn"),
#   AutoBlendGetBlendTime rva=0x1615c60, AutoBlendCalculateWeaponDistance rva=0x1615930. UNCONFIRMED: the clip pose of
#   a virtual bone is the clip's component transform of its target bone, both sockets in component space.
class_name AttackAutoBlend

# PrepareAnimationData's outputs this port uses
class Settings:
	var early_release := 0.0		# EarlyRelease after PrepareAnimationData
	var early_release_tf := 1.0		# EarlyReleaseTimeFactor
	var auto := false				# bUseAutomaticBlendIn
	var steps := 0					# AutoBlendOptimizeForwardSteps

static func settings(d: MotionDefs.Attack, type: int) -> Settings:
	var out := Settings.new()
	out.early_release = d.early_release
	out.early_release_tf = d.early_release_time_factor
	out.steps = d.auto_blend_optimize_forward_steps
	out.auto = d.regular_attacks_use_auto_blend_in
	match type:
		CombatEnums.AttackType.COMBO: out.auto = d.combo_attacks_use_auto_blend_in
		CombatEnums.AttackType.POST_CLASH: out.auto = d.post_clash_attacks_use_auto_blend_in
		CombatEnums.AttackType.MORPH: out.auto = d.morph_attacks_use_auto_blend_in
		CombatEnums.AttackType.RIPOSTE:
			out.auto = d.riposte_attacks_use_auto_blend_in
			out.steps = d.riposte_auto_blend_optimize_forward_steps
	if type == CombatEnums.AttackType.RIPOSTE or type == CombatEnums.AttackType.POST_CLASH:
		out.early_release = d.riposte_early_release
		out.early_release_tf = d.riposte_early_release_time_factor
	return out

# UAttackMotion::GetSmoothedWindUpNormalizedTime rva=0x1628bf0
static func smoothed_windup(d: MotionDefs.Attack, start: float, windup_end: float, release_end: float, offset: float, x: float) -> float:
	if not d.enable_windup_smoothing:
		return x
	var rel := release_end - windup_end
	var span := 1.0 - (offset + offset)
	var eps := CombatConstants.small_number
	if absf(rel) > eps and absf(span) > eps:
		var exp_clamp := d.windup_smoothing_exponent_clamp
		return pow(x, clampf((windup_end - start) / (rel * span), exp_clamp.x, exp_clamp.y))
	return x

# UAttackMotion::AutoBlendGetBlendTime rva=0x1615c60: max(SpineCurve(angle), WeaponCurve(distance))
static func blend_time(d: MotionDefs.Attack, weapon_distance: float, spine_angle: float) -> float:
	var spine_curve := d.auto_blend_in_spine_curve
	var weapon_curve := d.auto_blend_in_weapon_curve
	var spine_time := CombatData.curve_value(spine_curve, spine_angle) if spine_curve != "" else 0.0
	var time := spine_time
	if weapon_curve != "":
		time = CombatData.curve_value(weapon_curve, weapon_distance)
		if time <= spine_time:
			time = spine_time
	return time

# The AutoBlend literals (deg->rad, 15, 0.15, 0.5, FLT_MAX) are CombatConstants.auto_blend_* (exe .rdata, cited there).

# UAttackMotion::AutoBlendCalculateWeaponDistance rva=0x1615930: how far the weapon hand of a clip pose is from the
# hand shown now. weapon_length = AMordhauWeapon Length.
#   forward_angle = angle between the two hands' -X axes, up_angle = between their +Z axes
#   angle = up_angle if up_angle > AutoBlendConsiderUpVectorIfLargerThanAngle (degrees) and > forward_angle,
#           else forward_angle
#   distance = angle x Length x 15 + |hand origin difference| (UE cm)
static func weapon_distance(d: MotionDefs.Attack, weapon_length: float, hand: Transform3D, anim_hand: Transform3D) -> float:
	var q_anim := AnimMath.quat_godot_to_ue(anim_hand.basis.get_rotation_quaternion())
	var q_hand := AnimMath.quat_godot_to_ue(hand.basis.get_rotation_quaternion())
	var forward_angle := acos(clampf((q_hand * Vector3(-1, 0, 0)).dot(q_anim * Vector3(-1, 0, 0)), -1.0, 1.0))
	var up_angle := acos(clampf((q_hand * Vector3(0, 0, 1)).dot(q_anim * Vector3(0, 0, 1)), -1.0, 1.0))
	var up_threshold := d.auto_blend_consider_up_vector_if_larger_than_angle * CombatConstants.auto_blend_deg_to_rad
	var angle := up_angle if up_angle > up_threshold and up_angle > forward_angle else forward_angle
	var offset_cm := (AnimMath.vec_godot_to_ue(hand.origin) - AnimMath.vec_godot_to_ue(anim_hand.origin)).length()
	return angle * weapon_length * CombatConstants.auto_blend_angle_to_cm + offset_cm

# Spine yaw of a pose in UE degrees (FQuat::Rotator yaw, AnimMath.quat_yaw_ue)
static func _spine_yaw(spine: Transform3D) -> float:
	return AnimMath.quat_yaw_ue(AnimMath.quat_godot_to_ue(spine.basis.get_rotation_quaternion()))

# One pose of the weapon hand and the spine, component space ("VB Global_RightWeapon" / "VB Global_Spine1")
class Pose:
	var hand := Transform3D.IDENTITY
	var spine := Transform3D.IDENTITY
	func _init(h := Transform3D.IDENTITY, s := Transform3D.IDENTITY) -> void:
		hand = h
		spine = s

# How far one clip pose is from the pose shown now
class Gap:
	var yaw := 0.0			# |spine yaw difference|, normalized to (-180, 180]
	var distance := 0.0		# weapon distance

static func _pose_gap(d: MotionDefs.Attack, weapon_length: float, pose_now: Pose, pose_clip: Pose) -> Gap:
	var g := Gap.new()
	g.yaw = absf(AnimMath.normalize_axis(_spine_yaw(pose_clip.spine) - _spine_yaw(pose_now.spine)))
	g.distance = weapon_distance(d, weapon_length, pose_now.hand, pose_clip.hand)
	return g

# The search's result: BlendIn and WindUpAnimationStartTimeOffset
class SearchResult:
	var blend_in := 0.0
	var offset := 0.0

# AutoBlend search, the loop of UAttackMotion::OnBegin_Implementation rva=0x162eda0 (exe 0x14162fb19..0x14162ff7b).
# pose_now: the pose shown now; pose_clip: Callable(t) -> Pose of the clip at t.
# For each step i = 0 .. AutoBlendOptimizeForwardSteps, candidate start t = i x StepSize:
#   sample 1 at t: blend = BlendTime(distance, yaw gap)
#   if blend >= 0.15: sample 2 at t2 = min(t + 0.5 x SmoothedWindUp(blend / Windup), 0.5), and
#                     blend = BlendTime(mean distance, mean yaw gap) of the two samples
#   the smallest blend wins; its t becomes WindUpAnimationStartTimeOffset, the blend BlendIn.
static func search(d: MotionDefs.Attack, steps: int, start: float, windup_end: float, release_end: float, weapon_length: float,
		pose_now: Pose, pose_clip: Callable) -> SearchResult:
	var step_size := d.auto_blend_optimize_forward_step_size
	var windup := windup_end - start				# FAttackInfo.Windup
	var class_offset := d.windup_animation_start_time_offset	# not yet overwritten while searching
	var best_blend := CombatConstants.auto_blend_no_blend_yet
	var best_offset := 0.0
	for i in range(0, steps + 1):
		var t := float(i) * step_size
		var first := _pose_gap(d, weapon_length, pose_now, pose_clip.call(t))
		var blend := blend_time(d, first.distance, first.yaw)
		if blend >= CombatConstants.auto_blend_min_blend:
			var smoothed := smoothed_windup(d, start, windup_end, release_end, class_offset, blend / windup)
			var t2 := minf(smoothed * CombatConstants.auto_blend_half + t, CombatConstants.auto_blend_half)
			var second := _pose_gap(d, weapon_length, pose_now, pose_clip.call(t2))
			blend = blend_time(d, (first.distance + second.distance) * CombatConstants.auto_blend_half,
				(first.yaw + second.yaw) * CombatConstants.auto_blend_half)
		if blend < best_blend:
			best_blend = blend
			best_offset = t
	var r := SearchResult.new()
	r.blend_in = best_blend
	r.offset = best_offset
	return r

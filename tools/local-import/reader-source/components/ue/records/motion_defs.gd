# motion_defs.gd - the class defaults of every motion the combat and animation ports run, as typed records:
#   Base     UMordhauMotion     (extract/native/types/UMordhauMotion.h)
#   Attack   UAttackMotion      (UAttackMotion.h; UStrikeMotion.h adds the look-up early-release terms)
#   Kick     UKickMotion        (UKickMotion.h: jump kick, tier-3 legs)
#   Parry    UParryMotion       (UParryMotion.h)
#   Feinted  UFeintedMotion     (UFeintedMotion.h)
#   Blocked  UBlockedMotion     (UBlockedMotion.h)
#   Flinch   UFlinchMotion      (UFlinchMotion.h)
#   Idle     UIdleMotion        (only Base fields)
# Source: CombatData.class_defaults(<motion Blueprint>) or CombatData.native_only(<class>): the native constructor
# replayed by NativeCtor (UAttackMotion rva=0x1612540, UStrikeMotion rva=0x16491c0, UStabMotion rva=0x1649150,
# UParryMotion rva=0x16486b0, UFeintedMotion rva=0x16468b0, UBlockedMotion rva=0x1644780, UFlinchMotion rva=0x1646930)
# with the Blueprint CDO chain merged over it. Every native field of a decodable type is in that dictionary, so every
# scalar here is a required read (UeRec): a missing key is a wrong name, never a silent 0. Object references (curves,
# animations, additives) are pointers NativeCtor never writes: absent = nullptr (UeRec.obj). FPerspective* structs are
# read on their ThirdPerson half (UeRec.tp_*). Records are shared per class (CombatData caches them): read-only.
class_name MotionDefs

# FSpineSpaceAdditive fields (types/FSpineSpaceAdditive.h: 11 FRotators)
const SPINE_FIELDS := ["Head", "Neck", "Spine1", "LeftShoulder", "LeftArm", "LeftForearm", "LeftHand", "RightShoulder",
	"RightArm", "RightForearm", "RightHand"]

# FSpineSpaceAdditive -> {field: Vector3(pitch, yaw, roll)} keyed by SPINE_FIELDS (the shape SpineAdditive blends)
static func spine(r: UeRec) -> Dictionary:
	var out := {}
	for f in SPINE_FIELDS:
		out[f] = r.rot(f)
	return out

# FHighMidLowSpineSpaceAdditive (High +0, Mid +0x84, Low +0x108)
class Hml extends Resource:
	var high := {}
	var mid := {}
	var low := {}
	static func read(r: UeRec) -> Hml:
		var h := Hml.new()
		h.high = MotionDefs.spine(r.sub("High"))
		h.mid = MotionDefs.spine(r.sub("Mid"))
		h.low = MotionDefs.spine(r.sub("Low"))
		return h

# EMovementRestriction field: NativeCtor stores the byte (int), a CDO the UHT name ("EMovementRestriction::Walk");
# required like every scalar (missing / malformed -> UeRec error). Values: CombatEnums.MovementRestriction (PDB LF_ENUM).
static func restriction(r: UeRec, k: String) -> int:
	if not r.has(k):
		r.errors.append("%s.%s: missing" % [r.what, k])
		return 0
	var v = r.d[k]
	if v is int and v >= 0 and v < CombatEnums.MOVEMENT_RESTRICTION_NAMES.size():
		return v
	if v is String and String(v).begins_with("EMovementRestriction::"):
		var i := CombatEnums.MOVEMENT_RESTRICTION_NAMES.find(String(v).get_slice("::", 1))
		if i >= 0:
			return i
	r.errors.append("%s.%s: expected EMovementRestriction, got %s" % [r.what, k, v])
	return 0

class Base extends Resource:
	var path := ""					# Blueprint class ("" for a native-only motion)
	var native := ""				# C++ class (UStrikeMotion, UStabMotion, UKickMotion, ...)
	var b_is_flinchable := false	# +0x60
	var b_can_attack := false		# +0x6d
	var b_can_block := false		# +0x6e
	var b_can_emote := false		# +0x6c
	var b_blocks_regen := false		# +0x88
	var speed_factor := 0.0			# +0x64 SpeedFactor
	var movement_restriction := 0	# +0x61 MovementRestriction (EMovementRestriction)

	func read(r: UeRec) -> void:
		path = r.s("__path")
		native = r.s("__native")
		b_is_flinchable = r.b("bIsFlinchable")
		b_can_attack = r.b("bCanAttack")
		b_can_block = r.b("bCanBlock")
		b_can_emote = r.b("bCanEmote")
		b_blocks_regen = r.b("bBlocksRegen")
		speed_factor = r.f("SpeedFactor")
		movement_restriction = MotionDefs.restriction(r, "MovementRestriction")

	func is_a(cpp_class: String) -> bool:
		return CombatData.is_class_of(native, cpp_class)

class Attack extends Base:
	# rules (attack_motion.gd, blocked_motion.gd, feinted_motion.gd, melee_hit.gd)
	var angling_limits := Vector2.ZERO				# +0xf0 AnglingLimits
	var clash_on_parry_follow_up_windup := 0.0		# +0xaa8
	var morph_windup_modifier := 0.0				# +0xba4
	var riposte_windup_modifier := 0.0				# +0xaa4
	var chamber_stamina_recover := 0				# +0xa7c ChamberStaminaRecover (UAttackMotion::CheckChamber stamina break)
	var miss_twice_stamina_cost_multiplier := 0.0
	var min_windup_time_before_morphing := 0.0
	var morph_window := 0.0
	var max_morph_total_time := 0.0
	var morph_kick_extra_time := 0.0
	var recovery_queue_window := 0.0
	var feint_window := 0.0
	var riposte_windup_can_parry_window := 0.0
	var hit_recovery := 0.0
	var clashed_recovery := 0.0
	var hit_stop_recovery := 0.0
	var chamber_window := 0.0
	var trace_memory_stay_duration := 0.0			# +0xa58
	var strike_animation_normalized_recovery_offset := 0.0
	var riposte_trade_damage_factor := 0.0			# +0xa9c
	var post_friendly_hit_modifier := 0.0			# +0xa8c PostFriendlyHitModifier
	var b_do_not_make_recovery_flinchable := false
	var b_is_riposte_feintable := false
	var b_use_seamless_cftp_in_recovery := false
	var b_can_block_from_release_after_hit := false
	var b_can_auto_feint_to_attack := false			# +0xb98
	var b_can_attack_from_feint_lockout := false	# +0xb7d
	var b_stop_on_hit_on_kills := false
	var b_can_be_parried_in_early_release := false	# +0xa6c (ProcessHitForBlocking parry gate)
	var b_no_damage_in_early_release := false		# +0xa6d (UStrikeMotion ctor rva=0x16491c0 sets 1)
	var clash_on_parry_can_parry_window := 0.0		# +0xaac (ProcessBlock PostClash windup parry)
	var to_chamber_attack_angle_tolerance := 0.0	# +0xa80 (CheckChamberIsValidIgnoreTiming)
	# blocked bounce (UBlockedMotion OnBegin / OnTick read these from the blocked attack); FPerspective* = ThirdPerson,
	# FirstPerson when null (the order UBlockedMotion picks them in for a third-person character)
	var bounce_montage := ""						# +0xae0 BounceMontage
	var world_bounce_curve := ""					# +0xaf8 WorldBounceCurve
	var world_bounce_scale_curve := ""				# +0xb08 WorldBounceScaleCurve
	var parry_bounce_curve := ""					# +0xb18 ParryBounceCurve
	var parry_late_bounce_curve := ""				# +0xb28 ParryLateBounceCurve
	var parry_bounce_scale_curve := ""				# +0xb38 ParryBounceScaleCurve
	# UStrikeMotion only (UStrikeMotion::IsInEarlyRelease rva=0x165d610); 0 on other classes
	var extra_early_release_for_look_up_non_undercuts := 0.0
	var extra_early_release_for_look_up_overheads := 0.0
	# curves (UCurveFloat*, "" = none); combo windup curve is an FPerspectiveCurve (ThirdPerson)
	var windup_curve := ""							# +0xdf8
	var release_curve := ""							# +0xe18
	var combo_windup_curve := ""					# +0xe00
	var morph_windup_curve := ""					# +0xba8
	var riposte_release_curve := ""					# +0xe20
	# PrepareAnimationData / AutoBlend (attack_auto_blend.gd)
	var early_release := 0.0						# +0xb48
	var early_release_time_factor := 0.0			# +0xb4c
	var riposte_early_release := 0.0				# +0xb50
	var riposte_early_release_time_factor := 0.0	# +0xb54
	var auto_blend_optimize_forward_steps := 0		# +0xbf0
	var riposte_auto_blend_optimize_forward_steps := 0
	var auto_blend_optimize_forward_step_size := 0.0	# +0xbf8
	var auto_blend_consider_up_vector_if_larger_than_angle := 0.0	# +0xbd8, degrees
	var regular_attacks_use_auto_blend_in := false
	var combo_attacks_use_auto_blend_in := false
	var post_clash_attacks_use_auto_blend_in := false
	var morph_attacks_use_auto_blend_in := false
	var riposte_attacks_use_auto_blend_in := false
	var enable_windup_smoothing := false
	var windup_smoothing_exponent_clamp := Vector2.ZERO
	var auto_blend_in_spine_curve := ""
	var auto_blend_in_weapon_curve := ""
	var windup_animation_start_time_offset := 0.0	# +0x10a4 class default
	# animation (motion_anim.gd)
	var animation := ""
	var clash_animation := ""
	var riposte_animation := ""
	var alt_riposte_animation := ""
	var normal_blend_in := 0.0
	var normal_parry_slow_blend_in := 0.0
	var normal_slow_blend_in := 0.0
	var combo_blend_in := 0.0
	var post_clash_blend_in := 0.0
	var morph_blend_in := 0.0
	var riposte_blend_in := 0.0
	var blend_in_curve := ""
	var combo_blend_in_curve := ""
	var morph_blend_in_curve := ""
	var riposte_blend_in_curve := ""
	var blend_out := 0.0
	var feint_anim_duration_offset := 0.0
	var feint_anim_minimum_duration := 0.0
	var feint_anim_rate := 0.0
	var miss_recovery_play_rate_clamp := Vector2.ZERO
	var miss_recovery_to_play_rate := 0.0
	var successful_hit_play_rate := 0.0
	var successful_hit_blend_out_anim_time := 0.0
	var bounce_additive := ""						# ThirdPerson, FirstPerson when null
	var angling_windup := Hml.new()					# AnglingAdditiveWindUp.ThirdPerson (+0xf8)
	var angling_release := Hml.new()				# AnglingAdditiveRelease (+0x410)
	var riposte_angling_windup := Hml.new()			# +0x59c
	var riposte_angling_release := Hml.new()		# +0x8b4

	func read(r: UeRec) -> void:
		super.read(r)
		angling_limits = r.v2("AnglingLimits")
		clash_on_parry_follow_up_windup = r.f("ClashOnParryFollowUpWindup")
		morph_windup_modifier = r.f("MorphWindupModifier")
		riposte_windup_modifier = r.f("RiposteWindupModifier")
		miss_twice_stamina_cost_multiplier = r.f("MissTwiceStaminaCostMultiplier")
		min_windup_time_before_morphing = r.f("MinWindUpTimeBeforeMorphing")
		morph_window = r.f("MorphWindow")
		max_morph_total_time = r.f("MaxMorphTotalTime")
		morph_kick_extra_time = r.f("MorphKickExtraTime")
		recovery_queue_window = r.f("RecoveryQueueWindow")
		feint_window = r.f("FeintWindow")
		riposte_windup_can_parry_window = r.f("RiposteWindUpCanParryWindow")
		hit_recovery = r.f("HitRecovery")
		clashed_recovery = r.f("ClashedRecovery")
		hit_stop_recovery = r.f("HitStopRecovery")
		chamber_window = r.f("ChamberWindow")
		chamber_stamina_recover = r.i("ChamberStaminaRecover")
		trace_memory_stay_duration = r.f("TraceMemoryStayDuration")
		strike_animation_normalized_recovery_offset = r.f("StrikeAnimationNormalizedRecoveryOffset")
		riposte_trade_damage_factor = r.f("RiposteTradeDamageFactor")
		post_friendly_hit_modifier = r.f("PostFriendlyHitModifier")
		b_do_not_make_recovery_flinchable = r.b("bDoNotMakeRecoveryFlinchable")
		b_is_riposte_feintable = r.b("bIsRiposteFeintable")
		b_use_seamless_cftp_in_recovery = r.b("bUseSeamlessCFTPInRecovery")
		b_can_block_from_release_after_hit = r.b("bCanBlockFromReleaseAfterHit")
		b_can_auto_feint_to_attack = r.b("bCanAutoFeintToAttack")
		b_can_attack_from_feint_lockout = r.b("bCanAttackFromFeintLockout")
		b_stop_on_hit_on_kills = r.b("bStopOnHitOnKills")
		b_can_be_parried_in_early_release = r.b("bCanBeParriedInEarlyRelease")
		b_no_damage_in_early_release = r.b("bNoDamageInEarlyRelease")
		clash_on_parry_can_parry_window = r.f("ClashOnParryCanParryWindow")
		to_chamber_attack_angle_tolerance = r.f("ToChamberAttackAngleTolerance")
		bounce_montage = r.obj("BounceMontage")
		world_bounce_curve = r.tp_or_fp_obj("WorldBounceCurve")
		world_bounce_scale_curve = r.tp_or_fp_obj("WorldBounceScaleCurve")
		parry_bounce_curve = r.tp_or_fp_obj("ParryBounceCurve")
		parry_late_bounce_curve = r.tp_or_fp_obj("ParryLateBounceCurve")
		parry_bounce_scale_curve = r.tp_or_fp_obj("ParryBounceScaleCurve")
		if is_a("UStrikeMotion"):
			extra_early_release_for_look_up_non_undercuts = r.f("ExtraEarlyReleaseForLookUpNonUndercuts")
			extra_early_release_for_look_up_overheads = r.f("ExtraEarlyReleaseForLookUpOverheads")
		windup_curve = r.obj("WindUpCurve")
		release_curve = r.obj("ReleaseCurve")
		combo_windup_curve = r.tp_obj("ComboWindUpCurve")
		morph_windup_curve = r.obj("MorphWindupCurve")
		riposte_release_curve = r.obj("RiposteReleaseCurve")
		early_release = r.f("EarlyRelease")
		early_release_time_factor = r.f("EarlyReleaseTimeFactor")
		riposte_early_release = r.f("RiposteEarlyRelease")
		riposte_early_release_time_factor = r.f("RiposteEarlyReleaseTimeFactor")
		auto_blend_optimize_forward_steps = r.i("AutoBlendOptimizeForwardSteps")
		riposte_auto_blend_optimize_forward_steps = r.i("RiposteAutoBlendOptimizeForwardSteps")
		auto_blend_optimize_forward_step_size = r.f("AutoBlendOptimizeForwardStepSize")
		auto_blend_consider_up_vector_if_larger_than_angle = r.f("AutoBlendConsiderUpVectorIfLargerThanAngle")
		regular_attacks_use_auto_blend_in = r.tp_b("RegularAttacksUseAutoBlendIn")
		combo_attacks_use_auto_blend_in = r.tp_b("ComboAttacksUseAutoBlendIn")
		post_clash_attacks_use_auto_blend_in = r.tp_b("PostClashAttacksUseAutoBlendIn")
		morph_attacks_use_auto_blend_in = r.tp_b("MorphAttacksUseAutoBlendIn")
		riposte_attacks_use_auto_blend_in = r.tp_b("RiposteAttacksUseAutoBlendIn")
		enable_windup_smoothing = r.tp_b("EnableWindUpSmoothing")
		windup_smoothing_exponent_clamp = r.v2("WindUpSmoothingExponentClamp")
		auto_blend_in_spine_curve = r.tp_obj("AutoBlendInSpineCurve")
		auto_blend_in_weapon_curve = r.tp_obj("AutoBlendInWeaponCurve")
		windup_animation_start_time_offset = r.f("WindUpAnimationStartTimeOffset")
		animation = r.tp_obj("Animation")
		clash_animation = r.tp_obj("ClashAnimation")
		riposte_animation = r.tp_obj("RiposteAnimation")
		alt_riposte_animation = r.tp_obj("AltRiposteAnimation")
		normal_blend_in = r.tp_f("NormalBlendIn")
		normal_parry_slow_blend_in = r.tp_f("NormalParrySlowBlendIn")
		normal_slow_blend_in = r.tp_f("NormalSlowBlendIn")
		combo_blend_in = r.tp_f("ComboBlendIn")
		post_clash_blend_in = r.tp_f("PostClashBlendIn")
		morph_blend_in = r.tp_f("MorphBlendIn")
		riposte_blend_in = r.tp_f("RiposteBlendIn")
		blend_in_curve = r.obj("BlendInCurve")
		combo_blend_in_curve = r.tp_obj("ComboBlendInCurve")
		morph_blend_in_curve = r.obj("MorphBlendInCurve")
		riposte_blend_in_curve = r.obj("RiposteBlendInCurve")
		blend_out = r.tp_f("BlendOut")
		feint_anim_duration_offset = r.tp_f("FeintAnimDurationOffset")
		feint_anim_minimum_duration = r.tp_f("FeintAnimMinimumDuration")
		feint_anim_rate = r.tp_f("FeintAnimRate")
		miss_recovery_play_rate_clamp = r.tp_v2("MissRecoveryPlayRateClamp")
		miss_recovery_to_play_rate = r.tp_f("MissRecoveryToPlayRate")
		successful_hit_play_rate = r.tp_f("SuccessfulHitPlayRate")
		successful_hit_blend_out_anim_time = r.tp_f("SuccessfulHitBlendOutAnimTime")
		bounce_additive = r.tp_or_fp_obj("BounceAdditive")
		angling_windup = Hml.read(r.sub("AnglingAdditiveWindUp").sub("ThirdPerson"))
		angling_release = Hml.read(r.sub("AnglingAdditiveRelease"))
		riposte_angling_windup = Hml.read(r.sub("RiposteAnglingAdditiveWindUp").sub("ThirdPerson"))
		riposte_angling_release = Hml.read(r.sub("RiposteAnglingAdditiveRelease"))

class Parry extends Base:
	var parry_up_time := 0.0						# +0x514
	# UParryMotion::GetMovementRestriction rva=0x165cc30 (Recovery stage, TotalBlocks != 0 / == 0)
	var successful_parry_recovery_movement_restriction := 0	# +0x4a4
	var failed_parry_recovery_movement_restriction := 0		# +0x4a5
	var parry_recovery_time := 0.0					# +0x500
	var minimum_held_parry_time := 0.0				# +0x4f8
	var non_held_parry_extension_time := 0.0		# +0x4b0
	var minimum_held_riposte_parry_time := 0.0
	var shield_wall_raise_time := 0.0
	var miss_parry_recovery_time := 0.0
	var shield_wall_recovery_time := 0.0
	var held_parry_recovery_time := 0.0
	var held_parry_success_recovery_time := 0.0
	var parry_success_recovery_time := 0.0
	var parry_in_flinch_duration_max := 0.0		# +0x490 (CheckParry, parry in flinch)
	var non_held_parry_extension_and_riposte_window_extra := 0.0
	var riposte_window_base := 0.0
	var held_riposte_window_extra := 0.0
	var easy_parry_stamina_cost := 0.0
	var shield_wall_stamina_drain_factor := 0.0
	var easy_parry_duration := 0.0
	var block_stamina_recover := 0					# +0x518 BlockStaminaRecover (ReceiveBlock stamina break, HandleWasParried)
	var chamber_ftp_extra_stamina_drain := 0.0		# +0x46c ChamberFTPExtraStaminaDrain (ReceiveBlock)
	# animation (motion_anim.gd)
	var animation := ""
	var alt_animation := ""
	var parried_additive := ""
	var alt_parried_additive := ""
	var animation_additive := ""
	var parry_up_time_delay_expected_delay := 0.0
	var shield_wall_raise_time_anim_offset := 0.0
	var b_legacy_animation_playing_method := false
	var parry_fail_play_rate := 0.0
	var held_parry_fail_play_rate := 0.0
	var parry_miss_fade_out := 0.0
	var parry_fail_fade_out := 0.0
	var held_parry_fail_fade_out := 0.0
	var angle_additive_right := Hml.new()			# AngleAdditive (+0x100) Right +0
	var angle_additive_left := Hml.new()			# Left +0x18c

	func read(r: UeRec) -> void:
		super.read(r)
		parry_up_time = r.f("ParryUpTime")
		successful_parry_recovery_movement_restriction = MotionDefs.restriction(r, "SuccessfulParryRecoveryMovementRestriction")
		failed_parry_recovery_movement_restriction = MotionDefs.restriction(r, "FailedParryRecoveryMovementRestriction")
		parry_recovery_time = r.f("ParryRecoveryTime")
		minimum_held_parry_time = r.f("MinimumHeldParryTime")
		non_held_parry_extension_time = r.f("NonHeldParryExtensionTime")
		minimum_held_riposte_parry_time = r.f("MinimumHeldRiposteParryTime")
		shield_wall_raise_time = r.f("ShieldWallRaiseTime")
		miss_parry_recovery_time = r.f("MissParryRecoveryTime")
		shield_wall_recovery_time = r.f("ShieldWallRecoveryTime")
		held_parry_recovery_time = r.f("HeldParryRecoveryTime")
		held_parry_success_recovery_time = r.f("HeldParrySuccessRecoveryTime")
		parry_success_recovery_time = r.f("ParrySuccessRecoveryTime")
		parry_in_flinch_duration_max = r.f("ParryInFlinchDurationMax")
		non_held_parry_extension_and_riposte_window_extra = r.f("NonHeldParryExtensionAndRiposteWindowExtra")
		riposte_window_base = r.f("RiposteWindowBase")
		held_riposte_window_extra = r.f("HeldRiposteWindowExtra")
		easy_parry_stamina_cost = r.f("EasyParryStaminaCost")
		shield_wall_stamina_drain_factor = r.f("ShieldWallStaminaDrainFactor")
		easy_parry_duration = r.f("EasyParryDuration")
		block_stamina_recover = r.i("BlockStaminaRecover")
		chamber_ftp_extra_stamina_drain = r.f("ChamberFTPExtraStaminaDrain")
		animation = r.tp_obj("Animation")
		alt_animation = r.tp_obj("AltAnimation")
		parried_additive = r.tp_or_fp_obj("ParriedAdditive")
		alt_parried_additive = r.tp_or_fp_obj("AltParriedAdditive")
		animation_additive = r.tp_or_fp_obj("AnimationAdditive")
		parry_up_time_delay_expected_delay = r.f("ParryUpTimeDelayExpectedDelay")
		shield_wall_raise_time_anim_offset = r.f("ShieldWallRaiseTimeAnimOffset")
		b_legacy_animation_playing_method = r.b("bLegacyAnimationPlayingMethod")
		parry_fail_play_rate = r.f("ParryFailPlayRate")
		held_parry_fail_play_rate = r.f("HeldParryFailPlayRate")
		parry_miss_fade_out = r.f("ParryMissFadeOut")
		parry_fail_fade_out = r.f("ParryFailFadeOut")
		held_parry_fail_fade_out = r.f("HeldParryFailFadeOut")
		var aa := r.sub("AngleAdditive")
		angle_additive_right = Hml.read(aa.sub("Right"))
		angle_additive_left = Hml.read(aa.sub("Left"))

class Feinted extends Base:
	var strike_and_stab_lockout_in := Vector2.ZERO
	var strike_and_stab_lockout_out := Vector2.ZERO
	var strike_and_stab_late_feint_adjustment_curve := ""
	var extra_stab_lockout := 0.0
	var extra_strike_lockout := 0.0
	var slow_kick_duration := 0.0
	var queue_window := 0.0
	var spine_space_additive_blend_out_time := 0.0

	func read(r: UeRec) -> void:
		super.read(r)
		strike_and_stab_lockout_in = r.v2("StrikeAndStabLockoutIn")
		strike_and_stab_lockout_out = r.v2("StrikeAndStabLockoutOut")
		strike_and_stab_late_feint_adjustment_curve = r.obj("StrikeAndStabLateFeintAdjustmentCurve")
		extra_stab_lockout = r.f("ExtraStabLockout")
		extra_strike_lockout = r.f("ExtraStrikeLockout")
		slow_kick_duration = r.f("SlowKickDuration")
		queue_window = r.f("QueueWindow")
		spine_space_additive_blend_out_time = r.f("SpineSpaceAdditiveBlendOutTime")

# UBlockedMotion (types/UBlockedMotion.h). Animation-facing fields are read on their third-person (*3P) member: the
# simulation's characters are third person (UBlockedMotion::OnBegin / OnTick branch on bIsFirstPerson).
class Blocked extends Base:
	var parried_recovery_time_limits := Vector2.ZERO	# +0x120
	var parried_recovery_time_offset := 0.0				# +0x11c
	var world_miss_stamina_factor := 0.0				# +0x194
	var world_recovery_time := 0.0						# +0x190
	var chambered_recovery_time_limits := Vector2.ZERO	# +0x12c
	var chambered_recovery_time_offset := 0.0			# +0x128
	var queue_window := 0.0								# +0xc0
	var queue_window_hit := 0.0							# +0xc4
	var movement_restriction_hit := 0					# +0xc8 MovementRestrictionHit
	var movement_restriction_world := 0					# +0xc9 MovementRestrictionWorld
	var clash_fade_out_time := 0.0						# +0xd0 ClashFadeOutTime3P
	var stab_world_fade_out_time := 0.0					# +0xd8 StabWorldFadeOutTime3P
	var stab_parry_min_max_range := Vector2.ZERO		# +0xec StabParryMinMaxRange3P
	var stab_parry_fade_out_time := Vector2.ZERO		# +0xf4 StabParryFadeOutTime3P
	var stab_chambered_min_max_range := Vector2.ZERO	# +0x10c StabChamberedMinMaxRange3P
	var stab_chambered_fade_out_time := Vector2.ZERO	# +0x114 StabChamberedFadeOutTime3P
	var stab_hit_stop_fade_out_time := 0.0				# +0x184 StabHitStopFadeOutTime3P
	var kick_hit_stop_blend_out_time := 0.0				# +0x188
	var kick_hit_stop_anim_rate := 0.0					# +0x18c
	var hit_stop_time_until_fade := 0.0					# +0x174 ProceduralHitStopTimeUntilFade3P
	var hit_stop_bounce_duration := 0.0					# +0x178 ProceduralHitStopBounceDuration3P
	var hit_stop_fade_out_time := 0.0					# +0x17c ProceduralHitStopFadeOutTime3P
	var hit_stop_bounce_curve := ""						# +0x150 ProceduralHitStopBounceCurve3P
	var hit_stop_bounce_scale_curve := ""				# +0x158 ProceduralHitStopBounceScaleCurve3P
	var hit_stop_release_scale_curve := ""				# +0x160 ProceduralHitStopReleaseScaleCurve3P
	var release_scale_curve := ""						# +0x1a0 ReleaseScaleCurve3P
	var world_time_until_fade := 0.0					# +0x234 ProceduralWorldTimeUntilFade3P
	var world_bounce_duration := 0.0					# +0x238 ProceduralWorldBounceDuration3P
	var world_fade_out_time := 0.0						# +0x23c ProceduralWorldFadeOutTime3P
	var parry_min_max_range := Vector2.ZERO				# +0x1f4 ProceduralParryMinMaxRange3P
	var parry_time_until_fade := Vector2.ZERO			# +0x1fc ProceduralParryTimeUntilFade3P
	var parry_bounce_duration := Vector2.ZERO			# +0x204 ProceduralParryBounceDuration3P
	var parry_fade_out_time := Vector2.ZERO				# +0x20c ProceduralParryFadeOutTime3P
	var chamber_min_max_range := Vector2.ZERO			# +0x214 ProceduralChamberMinMaxRange3P
	var chamber_time_until_fade := Vector2.ZERO			# +0x21c ProceduralChamberTimeUntilFade3P
	var chamber_bounce_duration := Vector2.ZERO			# +0x224 ProceduralChamberBounceDuration3P
	var chamber_fade_out_time := Vector2.ZERO			# +0x22c ProceduralChamberFadeOutTime3P

	func read(r: UeRec) -> void:
		super.read(r)
		parried_recovery_time_limits = r.v2("ParriedRecoveryTimeLimits")
		parried_recovery_time_offset = r.f("ParriedRecoveryTimeOffset")
		world_miss_stamina_factor = r.f("WorldMissStaminaFactor")
		world_recovery_time = r.f("WorldRecoveryTime")
		chambered_recovery_time_limits = r.v2("ChamberedRecoveryTimeLimits")
		chambered_recovery_time_offset = r.f("ChamberedRecoveryTimeOffset")
		queue_window = r.f("QueueWindow")
		queue_window_hit = r.f("QueueWindowHit")
		movement_restriction_hit = MotionDefs.restriction(r, "MovementRestrictionHit")
		movement_restriction_world = MotionDefs.restriction(r, "MovementRestrictionWorld")
		clash_fade_out_time = r.f("ClashFadeOutTime3P")
		stab_world_fade_out_time = r.f("StabWorldFadeOutTime3P")
		stab_parry_min_max_range = r.v2("StabParryMinMaxRange3P")
		stab_parry_fade_out_time = r.v2("StabParryFadeOutTime3P")
		stab_chambered_min_max_range = r.v2("StabChamberedMinMaxRange3P")
		stab_chambered_fade_out_time = r.v2("StabChamberedFadeOutTime3P")
		stab_hit_stop_fade_out_time = r.f("StabHitStopFadeOutTime3P")
		kick_hit_stop_blend_out_time = r.f("KickHitStopBlendOutTime")
		kick_hit_stop_anim_rate = r.f("KickHitStopAnimRate")
		hit_stop_time_until_fade = r.f("ProceduralHitStopTimeUntilFade3P")
		hit_stop_bounce_duration = r.f("ProceduralHitStopBounceDuration3P")
		hit_stop_fade_out_time = r.f("ProceduralHitStopFadeOutTime3P")
		hit_stop_bounce_curve = r.obj("ProceduralHitStopBounceCurve3P")
		hit_stop_bounce_scale_curve = r.obj("ProceduralHitStopBounceScaleCurve3P")
		hit_stop_release_scale_curve = r.obj("ProceduralHitStopReleaseScaleCurve3P")
		release_scale_curve = r.obj("ReleaseScaleCurve3P")
		world_time_until_fade = r.f("ProceduralWorldTimeUntilFade3P")
		world_bounce_duration = r.f("ProceduralWorldBounceDuration3P")
		world_fade_out_time = r.f("ProceduralWorldFadeOutTime3P")
		parry_min_max_range = r.v2("ProceduralParryMinMaxRange3P")
		parry_time_until_fade = r.v2("ProceduralParryTimeUntilFade3P")
		parry_bounce_duration = r.v2("ProceduralParryBounceDuration3P")
		parry_fade_out_time = r.v2("ProceduralParryFadeOutTime3P")
		chamber_min_max_range = r.v2("ProceduralChamberMinMaxRange3P")
		chamber_time_until_fade = r.v2("ProceduralChamberTimeUntilFade3P")
		chamber_bounce_duration = r.v2("ProceduralChamberBounceDuration3P")
		chamber_fade_out_time = r.v2("ProceduralChamberFadeOutTime3P")

# UKickMotion (types/UKickMotion.h, ctor UKickMotion::UKickMotion rva=0x1646c10): read by
# UKickMotion::ModifyAttackInfo_Implementation rva=0x165d6e0 (attack_motion.gd _modify_kick_attack_info)
class Kick extends Attack:
	var kick_damage_modifier_tier3_legs := 0.0		# +0x1100
	var jump_kick_stamina_drain := 0.0				# +0x1104
	var jump_kick_extra_windup := 0.0				# +0x1108
	var jump_kick_air_movement_restriction := 0		# +0x110c
	var max_airborne_time_for_jump_kick_anim := 0.0	# +0x1110

	func read(r: UeRec) -> void:
		super.read(r)
		kick_damage_modifier_tier3_legs = r.f("KickDamageModifierTier3Legs")
		jump_kick_stamina_drain = r.f("JumpKickStaminaDrain")
		jump_kick_extra_windup = r.f("JumpKickExtraWindup")
		jump_kick_air_movement_restriction = MotionDefs.restriction(r, "JumpKickAirMovementRestriction")
		max_airborne_time_for_jump_kick_anim = r.f("MaxAirborneTimeForJumpKickAnim")

class Flinch extends Base:
	var flinch_duration := 0.0		# +0xa4
	var parry_lock_out_time := 0.0	# +0xa8 (UFlinchMotion::OnBegin_Implementation / OnTick_Implementation rva=0x1668430)

	func read(r: UeRec) -> void:
		super.read(r)
		flinch_duration = r.f("FlinchDuration")
		parry_lock_out_time = r.f("ParryLockOutTime")

# UStunMotion (types/UStunMotion.h, ctor UStunMotion::UStunMotion rva=0x16492e0): stun_motion.gd
class Stun extends Base:
	var stun_duration := 0.0					# +0xa8
	var stun_grace_period_extra_time := 0.0		# +0xa4

	func read(r: UeRec) -> void:
		super.read(r)
		stun_duration = r.f("StunDuration")
		stun_grace_period_extra_time = r.f("StunGracePeriodExtraTime")

# UDisarmedMotion (types/UDisarmedMotion.h, ctor UDisarmedMotion::UDisarmedMotion rva=0x1646630): disarmed_motion.gd
class Disarmed extends Base:
	var recovery_time := 0.0					# +0xa0

	func read(r: UeRec) -> void:
		super.read(r)
		recovery_time = r.f("RecoveryTime")

# merged class defaults -> the record for its native class; null (and an error) when a field is missing or malformed
static func from_ue(d: Dictionary) -> Base:
	var r := UeRec.new(d, "MotionDefs(%s)" % (d.get("__path", "") if String(d.get("__path", "")) != "" else d.get("__native", "?")))
	var nat := String(d.get("__native", ""))
	var m: Base
	if CombatData.is_class_of(nat, "UKickMotion"): m = Kick.new()
	elif CombatData.is_class_of(nat, "UAttackMotion"): m = Attack.new()
	elif CombatData.is_class_of(nat, "UParryMotion"): m = Parry.new()
	elif CombatData.is_class_of(nat, "UFeintedMotion"): m = Feinted.new()
	elif CombatData.is_class_of(nat, "UBlockedMotion"): m = Blocked.new()
	elif CombatData.is_class_of(nat, "UFlinchMotion"): m = Flinch.new()
	elif CombatData.is_class_of(nat, "UStunMotion"): m = Stun.new()
	elif CombatData.is_class_of(nat, "UDisarmedMotion"): m = Disarmed.new()
	elif CombatData.is_class_of(nat, "UMordhauMotion"): m = Base.new()
	else:
		r.errors.append("%s: %s is not a UMordhauMotion" % [r.what, nat])
		return r.done(null)
	m.read(r)
	return r.done(m)

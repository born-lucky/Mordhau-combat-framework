# character_data.gd - the character code's (game/character/*) data adapter: BP_MordhauCharacter's exports (CDO and
# default subobjects such as CharMoveComp / CharacterMesh0), Blueprint class defaults down the Super chain (camera
# component), native constructor defaults (NativeCtor) and FText values, so game/character never calls the extract
# readers (UePkg / NativeCtor) itself (tests/test_boundary.gd). game/ reads the typed records at the end of this file
# (Character, Stat, MeshProps, Movement, Perks, Camera), never the raw dictionaries by UE property name.
class_name CharacterData

const CHARACTER := "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter"

# every export of a package, as CUE4Parse wrote them
static func exports(path: String) -> Array:
	return UePkg.load_pkg(path)

# class default object (Default__X_C properties) of a Blueprint package
static func cdo(path := CHARACTER) -> Dictionary:
	return UePkg.cdo(UePkg.load_pkg(path))

# properties of the export named `nm` (a default subobject: "CharMoveComp", "CharacterMesh0", ...); {} if none
static func subobject(nm: String, path := CHARACTER) -> Dictionary:
	return UePkg.export_named(UePkg.load_pkg(path), nm).get("Properties", {})

# merged class defaults down the Blueprint Super chain (UePkg.defaults; Blueprint values only, no native ctor)
static func class_defaults(path: String) -> Dictionary:
	return UePkg.defaults(path)

# UMordhauSingleton.Equipment[id] as an EquipmentDef (null for id 0 / None)
static func equipment_def(id: int) -> EquipmentDef:
	var p := UeWearable.equipment_path(id)
	return EquipmentDef.load_def(p) if p != "" else null

# native constructor defaults of a PDB class (NativeCtor: extract/native/decomp ctor stores, rdata resolved)
static func native_defaults(cls: String) -> Dictionary:
	return NativeCtor.defaults(cls)

# FText as mdx json writes it: {LocalizedString / SourceString} or {CultureInvariantString}; "" for none
static func text(v) -> String:
	if v is Dictionary and v.has("CultureInvariantString") and not v.has("LocalizedString") and not v.has("SourceString"):
		return String(v.CultureInvariantString)
	return UePkg.text(v)

# UMordhauSingleton.Equipment[id] (FEquipmentCustomization.Id; UeWearable.equipment_path) -> merged class defaults
# (native ctor + Blueprint chain, CombatData.class_defaults; "__native" = the native class, "__path" = the package).
# {} for id 0 / None.
static func equipment(id: int) -> Dictionary:
	var p := UeWearable.equipment_path(id)
	return CombatData.class_defaults(p) if p != "" else {}

# IsA for native classes: walks the PDB base chain (NativeCtor.layout(cls).base, extract/native/types/<cls>.h)
static func native_is_a(cls: String, base: String) -> bool:
	return CombatData.is_class_of(cls, base)

# ---- typed records -------------------------------------------------------------------------------------------------
# AMordhauCharacter (+ AAdvancedCharacter) class defaults of BP_MordhauCharacter: CombatData.class_defaults = the
# native ctors (AAdvancedCharacter, AMordhauCharacter; NativeCtor, zero-filled first) with the Blueprint CDO chain
# merged over them, so every field is present and read as required (UeRec). Field offsets: types/AMordhauCharacter.h,
# AAdvancedCharacter.h.
class Character:
	var melee_windup_modifier := 0.0				# +0xe48
	var melee_combo_extra_windup_modifier := 0.0
	var melee_release_modifier := 0.0
	var melee_miss_recovery_modifier := 0.0		# ..+0xe54
	var look_up_limit := 0.0						# +0x8a0 (degrees)
	var look_down_limit := 0.0
	var stamina_cost_modifier := 0.0				# +0xe44
	var stamina_regen_delay := 0.0				# +0xe6c
	var stamina_regen_per_tick := 0				# +0xe69
	var stamina_regen_tick_rate := 0.0			# +0xe78
	var received_ranged_damage_modifier := 0.0
	var received_damage_modifier := 0.0			# +0x9f8
	var received_team_damage_modifier := 0.0		# +0xa00 (copied to UDamageableComponent TeamDamageModifier +0xc8)
	var received_damage_absorption := 0.0		# +0xa0c
	var received_damage_max := 0.0				# +0xa10
	var b_has_last_chance := false				# +0x9f0
	var last_chance_heal_amount := 0				# +0x9f4
	var damage_armor_tier_override := 0			# +0x7f4
	var b_can_jump_kick := false
	var leg_damage_bonus_modifier_airborne := 0.0
	var extra_stamina_on_hit := 0				# +0xe40
	var b_is_hit_stop_on_team_hits_disabled := false
	var b_will_stop_melee := false
	var b_cannot_chamber := false				# +0xb8d bCannotChamber (UAttackMotion::CheckChamberIsValidIgnoreTiming)
	var b_destroy_equipment_on_death := false	# +0x11e0 (UAttackMotion::CheckChamber, UParryMotion::ReceiveBlock stamina break)
	var b_always_stun_instead_of_disarm := false	# +0x1219 (same)
	var jump_cooldown := 0.0						# AMordhauCharacter::CanJumpInternal
	var max_sprint_fov_offset := 0.0
	var max_sprint_fov_offset_interp_speed := 0.0
	var knockback_parry := 0.0					# +0xe84 KnockbackParry (UParryMotion::ReceiveBlock, game/net)
	var dodge_stamina_cost := 0					# +0xeac DodgeStaminaCost (AMordhauCharacter::OnDodged, game/net)
	var jump_stamina_cost := 0.0					# +0xe7c JumpStaminaCost (AMordhauCharacter::OnJumped_Implementation)
	var received_fall_damage_modifier := 0.0		# +0x9fc ReceivedFallDamageModifier (UDamageableComponent Fall branch)
	var falling_time_to_ragdoll := 0.0			# +0xe9c FallingTimeToRagdoll (AMordhauCharacter::LODTick)
	var crouch_cooldown := 0.0					# +0xde8 CrouchCooldown (AMordhauCharacter::LODTick crouch toggle)
	var ragdoll_falling_get_up_duration := 0.0	# +0x840 RagdollFallingGetUpDuration (AAdvancedCharacter ctor)
	var ragdoll_falling_min_time := 0.0			# +0x84c RagdollFallingMinTime
	var ragdoll_falling_min_velocity_to_get_up := 0.0	# +0x850 RagdollFallingMinVelocityToGetUp
	var ragdoll_falling_time_at_min_velocity_to_get_up := 0.0	# +0x854
	var b_disable_ragdoll_falling := false		# +0x7f8 bDisableRagdollFalling
	var dodge_duration := 0.0					# +0xea4 DodgeDuration (AMordhauCharacter::CanJumpInternal_Implementation; Rust exe mode)
	var dodge_cooldown := 0.0					# +0xea8 (UMordhauMovementComponent::LODTick dodge; Rust exe mode)
	var b_can_dodge := false						# +0xe66
	var ellipse_bubble_length := 0.0				# +0xf48 (LODTick bubble avoidance)
	var ellipse_bubble_radius := 0.0				# +0xf4c
	var ellipse_bubble_max_height_diff := 0.0	# +0xf50

	func read(r: UeRec) -> void:
		melee_windup_modifier = r.f("MeleeWindupModifier")
		melee_combo_extra_windup_modifier = r.f("MeleeComboExtraWindupModifier")
		melee_release_modifier = r.f("MeleeReleaseModifier")
		melee_miss_recovery_modifier = r.f("MeleeMissRecoveryModifier")
		look_up_limit = r.f("LookUpLimit")
		look_down_limit = r.f("LookDownLimit")
		stamina_cost_modifier = r.f("StaminaCostModifier")
		stamina_regen_delay = r.f("StaminaRegenDelay")
		stamina_regen_per_tick = r.i("StaminaRegenPerTick")
		stamina_regen_tick_rate = r.f("StaminaRegenTickRate")
		received_ranged_damage_modifier = r.f("ReceivedRangedDamageModifier")
		received_damage_modifier = r.f("ReceivedDamageModifier")
		received_team_damage_modifier = r.f("ReceivedTeamDamageModifier")
		received_damage_absorption = r.f("ReceivedDamageAbsorption")
		received_damage_max = r.f("ReceivedDamageMax")
		b_has_last_chance = r.b("bHasLastChance")
		last_chance_heal_amount = r.i("LastChanceHealAmount")
		damage_armor_tier_override = r.i("DamageArmorTierOverride")
		b_can_jump_kick = r.b("bCanJumpKick")
		leg_damage_bonus_modifier_airborne = r.f("LegDamageBonusModifierAirborne")
		extra_stamina_on_hit = r.i("ExtraStaminaOnHit")
		b_is_hit_stop_on_team_hits_disabled = r.b("bIsHitStopOnTeamHitsDisabled")
		b_will_stop_melee = r.b("bWillStopMelee")
		b_cannot_chamber = r.b("bCannotChamber")
		b_destroy_equipment_on_death = r.b("bDestroyEquipmentOnDeath")
		b_always_stun_instead_of_disarm = r.b("bAlwaysStunInsteadOfDisarm")
		jump_cooldown = r.f("JumpCooldown")
		max_sprint_fov_offset = r.f("MaxSprintFOVOffset")
		max_sprint_fov_offset_interp_speed = r.f("MaxSprintFOVOffsetInterpSpeed")
		knockback_parry = r.f("KnockbackParry")
		dodge_stamina_cost = r.i("DodgeStaminaCost")
		jump_stamina_cost = r.f("JumpStaminaCost")
		received_fall_damage_modifier = r.f("ReceivedFallDamageModifier")
		falling_time_to_ragdoll = r.f("FallingTimeToRagdoll")
		crouch_cooldown = r.f("CrouchCooldown")
		ragdoll_falling_get_up_duration = r.f("RagdollFallingGetUpDuration")
		ragdoll_falling_min_time = r.f("RagdollFallingMinTime")
		ragdoll_falling_min_velocity_to_get_up = r.f("RagdollFallingMinVelocityToGetUp")
		ragdoll_falling_time_at_min_velocity_to_get_up = r.f("RagdollFallingTimeAtMinVelocityToGetUp")
		b_disable_ragdoll_falling = r.b("bDisableRagdollFalling")
		dodge_duration = r.f("DodgeDuration")
		dodge_cooldown = r.f("DodgeCooldown")
		b_can_dodge = r.b("bCanDodge")
		ellipse_bubble_length = r.f("EllipseBubbleLength")
		ellipse_bubble_radius = r.f("EllipseBubbleRadius")
		ellipse_bubble_max_height_diff = r.f("EllipseBubbleMaxHeightDiff")

static var _character: Character

static func character() -> Character:
	if _character == null:
		var r := UeRec.new(CombatData.class_defaults(CHARACTER), "BP_MordhauCharacter")
		var c := Character.new()
		c.read(r)
		_character = r.done(c)
	return _character

# UStatComponent fields of a stat component's native ctor defaults (UStaminaStatComponent / UHealthStatComponent;
# types/UStatComponent.h): no Blueprint subclass is used for either, so the native ctor is the whole source.
class Stat:
	var min_value := 0						# MinStatValue
	var max_value := 0						# MaxStatValue
	var initial_value := 0					# InitialStatValue
	var b_is_regenerable := false

	func read(r: UeRec) -> void:
		min_value = r.i("MinStatValue")
		max_value = r.i("MaxStatValue")
		initial_value = r.i("InitialStatValue")
		b_is_regenerable = r.b("bIsRegenerable")

static var _stats := {}

static func stat(cls: String) -> Stat:
	if not _stats.has(cls):
		var r := UeRec.new(CombatData.native_only(cls), cls)
		var s := Stat.new()
		s.read(r)
		_stats[cls] = r.done(s)
	return _stats[cls]

# CharacterMesh0 (UHumanMeshComponent template of BP_MordhauCharacter): the unarmed anim set and the mesh placement.
# A template holds only serialized deltas: RelativeLocation / RelativeRotation absent = zero (USceneComponent
# defaults, as UeLevel.rel_xf reads them); animation references absent = nullptr.
class MeshProps:
	var relative_location := Vector3.ZERO		# UE cm
	var relative_rotation := Vector3.ZERO		# (pitch, yaw, roll) degrees
	var unarmed_lower_animation := ""
	var unarmed_falling_animation := ""

static var _mesh: MeshProps

static func mesh() -> MeshProps:
	if _mesh == null:
		var r := UeRec.new(subobject("CharacterMesh0"), "BP_MordhauCharacter.CharacterMesh0")
		var m := MeshProps.new()
		m.relative_location = r.v3_or("RelativeLocation", Vector3.ZERO)
		m.relative_rotation = r.rot_or("RelativeRotation", Vector3.ZERO)
		m.unarmed_lower_animation = r.obj("UnarmedLowerAnimation")
		m.unarmed_falling_animation = r.obj("UnarmedFallingAnimation")
		_mesh = r.done(m)
	return _mesh

# CharMoveComp (UMordhauMovementComponent template of BP_MordhauCharacter): every value the movement port reads.
# Engine fields (UCharacterMovementComponent: MaxWalkSpeed, GroundFriction, ...) have no PDB layout here and the
# Mordhau ones (SprintModifier, ...) have their ctor at rva=0x14af4f0; the template serializes every field below (all
# differ from those ctors), so each is a required read: a Blueprint that stopped serializing one fails here instead
# of moving at 0 cm/s. MaxWalkSpeedCrouched is per pawn (the Rat perk replaces it): MordhauMovement copies it.
class Movement:
	var max_walk_speed := 0.0
	var max_walk_speed_crouched := 0.0
	var max_walk_speed_crouched_with_rat_perk := 0.0
	var max_speed_falling := 0.0
	var max_acceleration := 0.0
	var walk_acceleration := 0.0
	var sprint_acceleration := 0.0
	var partial_sprint_acceleration := 0.0
	var sprint_modifier := 0.0
	var partial_sprint_modifier := 0.0
	var backpedal_modifier := 0.0
	var supersprint_modifier := 0.0
	var chasing_modifier := 0.0
	var sprint_time_to_reach_max_sprint := 0.0
	var braking_deceleration_falling := 0.0
	var braking_deceleration_falling_too_fast := 0.0
	var braking_friction_factor := 0.0
	var ground_friction := 0.0
	var falling_lateral_friction := 0.0
	var air_control := 0.0
	var air_control_boost_multiplier := 0.0
	var air_control_boost_velocity_threshold := 0.0
	var jump_z_velocity := 0.0
	var gravity_scale := 0.0
	var crouched_half_height := 0.0
	var walkable_floor_z := 0.0
	var max_step_height := 0.0
	var perch_radius_threshold := 0.0			# +0x1dc (the Rust exe-mode floor/perch port reads these three)
	var perch_additional_height := 0.0			# +0x1e0
	var b_can_walk_off_ledges_when_crouching := false	# +0x1f1 bit 6

	func read(r: UeRec) -> void:
		max_walk_speed = r.f("MaxWalkSpeed")
		max_walk_speed_crouched = r.f("MaxWalkSpeedCrouched")
		max_walk_speed_crouched_with_rat_perk = r.f("MaxWalkSpeedCrouchedWithRatPerk")
		max_speed_falling = r.f("MaxSpeedFalling")
		max_acceleration = r.f("MaxAcceleration")
		walk_acceleration = r.f("WalkAcceleration")
		sprint_acceleration = r.f("SprintAcceleration")
		partial_sprint_acceleration = r.f("PartialSprintAcceleration")
		sprint_modifier = r.f("SprintModifier")
		partial_sprint_modifier = r.f("PartialSprintModifier")
		backpedal_modifier = r.f("BackpedalModifier")
		supersprint_modifier = r.f("SupersprintModifier")
		chasing_modifier = r.f("ChasingModifier")
		sprint_time_to_reach_max_sprint = r.f("SprintTimeToReachMaxSprint")
		braking_deceleration_falling = r.f("BrakingDecelerationFalling")
		braking_deceleration_falling_too_fast = r.f("BrakingDecelerationFallingTooFast")
		braking_friction_factor = r.f("BrakingFrictionFactor")
		ground_friction = r.f("GroundFriction")
		falling_lateral_friction = r.f("FallingLateralFriction")
		air_control = r.f("AirControl")
		air_control_boost_multiplier = r.f("AirControlBoostMultiplier")
		air_control_boost_velocity_threshold = r.f("AirControlBoostVelocityThreshold")
		jump_z_velocity = r.f("JumpZVelocity")
		gravity_scale = r.f("GravityScale")
		crouched_half_height = r.f("CrouchedHalfHeight")
		walkable_floor_z = r.f("WalkableFloorZ")
		max_step_height = r.f("MaxStepHeight")
		perch_radius_threshold = r.f("PerchRadiusThreshold")
		perch_additional_height = r.f("PerchAdditionalHeight")
		b_can_walk_off_ledges_when_crouching = r.b("bCanWalkOffLedgesWhenCrouching")

static var _movement: Movement

static func movement() -> Movement:
	if _movement == null:
		var r := UeRec.new(subobject("CharMoveComp"), "BP_MordhauCharacter.CharMoveComp")
		var m := Movement.new()
		m.read(r)
		_movement = r.done(m)
	return _movement

# Movement-component fields CharMoveComp may leave to the native constructors: the UMordhauMovementComponent ctor
# (base UAdvancedCharacterMovement ctor first; NativeCtor) with BP_MordhauCharacter's CharMoveComp merged over it.
# The base fall-damage terms come from the UAdvancedCharacterMovement ctor only (CharMoveComp does not serialize them);
# the Knockback* and Ragdoll* terms are CharMoveComp overrides. Offsets: types/UAdvancedCharacterMovement.h,
# UMordhauMovementComponent.h.
class MoveExtra:
	var min_velocity_for_fall_damage := 0.0		# +0xb98
	var fall_damage_offset := 0.0				# +0xb9c
	var fall_damage_factor := 0.0				# +0xba0
	var ragdoll_min_velocity_for_fall_damage := 0.0	# +0xba4
	var ragdoll_fall_damage_offset := 0.0		# +0xba8
	var ragdoll_fall_damage_factor := 0.0		# +0xbac
	var knockback_ground_friction := 0.0			# +0xd20
	var knockback_falling_lateral_friction := 0.0	# +0xd24
	var knockback_up_impulse := 0.0				# +0xd28
	var knockback_duration := 0.0				# +0xd38
	var b_can_crouch := false					# NavAgentProps.bCanCrouch (UNavMovementComponent)
	# UMordhauMovementComponent::LODTick rva=0x14c4d60 terms (ctor rva=0x14af4f0 + CharMoveComp; read by the Rust exe port)
	var turn_sprint_prevention_max_accumulated_angle := 0.0	# +0xc70
	var turn_sprint_prevention_decay_curve := ""		# +0xc60 UCurveFloat
	var turn_sprint_prevention_slowdown_curve := ""		# +0xc68 UCurveFloat
	var rush_sprint_time_start := 0.0				# +0xc74
	var chasing_sprint_time_start := 0.0			# +0xc78
	var max_angle_to_chase := 0.0					# +0xc80
	var max_angle_to_stop_chasing := 0.0			# +0xc84
	var chasing_max_distance := Vector3.ZERO		# +0xc88 (UE X, Y, Z)
	var stop_chasing_max_distance := Vector3.ZERO	# +0xc94
	var time_to_break_us_chasing := 0.0			# +0xcac
	var time_to_break_us_being_chased := 0.0		# +0xcb0
	var min_time_to_start_chasing := 0.0			# +0xcb4
	var min_time_to_start_being_chased := 0.0		# +0xcb8
	var spawn_max_sprint_duration := 0.0			# +0xc4c

	func read(r: UeRec) -> void:
		min_velocity_for_fall_damage = r.f("MinVelocityForFallDamage")
		fall_damage_offset = r.f("FallDamageOffset")
		fall_damage_factor = r.f("FallDamageFactor")
		ragdoll_min_velocity_for_fall_damage = r.f("RagdollMinVelocityForFallDamage")
		ragdoll_fall_damage_offset = r.f("RagdollFallDamageOffset")
		ragdoll_fall_damage_factor = r.f("RagdollFallDamageFactor")
		knockback_ground_friction = r.f("KnockbackGroundFriction")
		knockback_falling_lateral_friction = r.f("KnockbackFallingLateralFriction")
		knockback_up_impulse = r.f("KnockbackUpImpulse")
		knockback_duration = r.f("KnockbackDuration")
		b_can_crouch = r.sub("NavAgentProps").b("bCanCrouch")
		turn_sprint_prevention_max_accumulated_angle = r.f("TurnSprintPreventionMaxAccumulatedAngle")
		turn_sprint_prevention_decay_curve = r.obj("TurnSprintPreventionDecayCurve")
		turn_sprint_prevention_slowdown_curve = r.obj("TurnSprintPreventionSlowdownCurve")
		rush_sprint_time_start = r.f("RushSprintTimeStart")
		chasing_sprint_time_start = r.f("ChasingSprintTimeStart")
		max_angle_to_chase = r.f("MaxAngleToChase")
		max_angle_to_stop_chasing = r.f("MaxAngleToStopChasing")
		chasing_max_distance = r.v3("ChasingMaxDistance")
		stop_chasing_max_distance = r.v3("StopChasingMaxDistance")
		time_to_break_us_chasing = r.f("TimeToBreakUsChasing")
		time_to_break_us_being_chased = r.f("TimeToBreakUsBeingChased")
		min_time_to_start_chasing = r.f("MinTimeToStartChasing")
		min_time_to_start_being_chased = r.f("MinTimeToStartBeingChased")
		spawn_max_sprint_duration = r.f("SpawnMaxSprintDuration")

static var _move_extra: MoveExtra

static func move_extra() -> MoveExtra:
	if _move_extra == null:
		var d = UePkg._merge(NativeCtor.defaults("UMordhauMovementComponent"), subobject("CharMoveComp"))
		var r := UeRec.new(d, "UMordhauMovementComponent+CharMoveComp")
		var m := MoveExtra.new()
		m.read(r)
		_move_extra = r.done(m)
	return _move_extra

# UPerkSystemComponent native ctor defaults (NativeCtor): the Tank perk's armor factors
class Perks:
	var tank_armor_speed_factor := 0.0
	var tank_armor_acceleration_factor := 0.0

static var _perks: Perks

static func perks() -> Perks:
	if _perks == null:
		var r := UeRec.new(NativeCtor.defaults("UPerkSystemComponent"), "UPerkSystemComponent")
		var p := Perks.new()
		p.tank_armor_speed_factor = r.f("TankArmorSpeedFactor")
		p.tank_armor_acceleration_factor = r.f("TankArmorAccelerationFactor")
		_perks = r.done(p)
	return _perks

# BP_CharacterCameraComponent class defaults (native UCharacterCameraComponent / UMordhauCameraComponent ctor, then
# the Blueprint chain BP_CharacterCameraComponent -> BP_MordhauCameraComponent)
const CAMERA := "Mordhau/Content/Mordhau/Blueprints/Characters/BP_CharacterCameraComponent"

class Camera:
	var third_person_camera_offset := Vector3.ZERO		# UE cm
	var third_person_rotation_offset := Vector3.ZERO	# (pitch, yaw, roll)
	var field_of_view := 0.0

static var _camera: Camera

static func camera() -> Camera:
	if _camera == null:
		var r := UeRec.new(CombatData.class_defaults(CAMERA), "BP_CharacterCameraComponent")
		var c := Camera.new()
		c.third_person_camera_offset = r.v3("ThirdPersonCameraOffset")
		c.third_person_rotation_offset = r.rot("ThirdPersonRotationOffset")
		c.field_of_view = r.f("FieldOfView")
		_camera = r.done(c)
	return _camera

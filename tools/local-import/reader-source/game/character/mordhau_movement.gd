# mordhau_movement.gd - UMordhauMovementComponent (Mordhau) on top of UCharacterMovementComponent (UE 4.26), ported.
# Pure model, no nodes: tests tick it headless, MordhauCharacter (CharacterBody3D) feeds it input and floor contact.
# Works in UE units (cm, cm/s, degrees) on Godot axes (+Y up, lateral = XZ); the character converts cm -> m (x0.01).
#
# Sources, in the order a value is looked for:
#  1. BP_MordhauCharacter's CharMoveComp subobject (extract/json/.../Characters/BP_MordhauCharacter.json, export
#     "CharMoveComp", class MordhauMovementComponent): every value the Blueprint overrides.
#  2. Native constructors (Ghidra C in extract/native/decomp): UMordhauMovementComponent::UMordhauMovementComponent
#     rva=0x14af4f0 and the engine's UCharacterMovementComponent ctor (see NATIVE_* below).
#  3. Config: Mordhau/Config/DefaultEngine.ini [/Script/Engine.PhysicsSettings] (repak get -> extract/config).
# Field offsets were matched to names by value: each BP override sits at the offset whose ctor default it replaces,
# in the JSON's property order (e.g. 0xcc0 SprintModifier 1.65 -> 1.8, 0xcd8 WalkAcceleration 2660 -> 1650).
class_name MordhauMovement
extends RefCounted

const CHAR_PKG := "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter"

# EMovementMode values as UMordhauMovementComponent::GetMaxSpeed switches on them (byte at +0x168):
# 1 Walking, 2 NavWalking, 3 Falling (UE 4.26 EMovementMode order: None, Walking, NavWalking, Falling, ...).
enum Mode { NONE = 0, WALKING = 1, FALLING = 3 }

# Sprint state byte at +0xd18, written by UMordhauMovementComponent::LODTick (rva=0x14c4d60) and read by
# GetMaxSpeed/GetMaxAcceleration. Names are ours; numbers and their effects are from the decompile.
enum Sprint { FORWARD = 0, SIDEWAYS = 1, BACKPEDAL = 2, PARTIAL = 3, SPRINT = 4, RUSH = 5, CHASE = 6, SUPER = 7 }

# --- native defaults not overridden by the Blueprint -------------------------------------------------------------
# UMordhauMovementComponent ctor rva=0x14af4f0: +0xcd4 = 0x3f733333 (0.95) is the GetMaxSpeed factor for state 1;
# +0xcf8, +0xcfc, +0xcf4 = 1.0 (extra speed multipliers); +0xd04/+0xd08 = 1.0 (armor speed/accel factors, reset to
# 1 minus armor by UpdateArmorSpeedAndAcceleration rva=0x14dc0c0). +0xcf0 = 1330 is the state-7 acceleration.
const NATIVE_SIDEWAYS_MODIFIER := 0.95   # +0xcd4 StrafeModifier (PDB name)
const NATIVE_SUPERSPRINT_ACCELERATION := 1330.0   # +0xcf0 SupersprintAcceleration
# .rdata floats read from the exe at the addresses LODTick / GetMaxBrakingDeceleration load them from:
const RDATA_FORWARD_CONE_DEG := 56.25      # DAT_14433116c: |move yaw - actor yaw| <= this counts as forward
const RDATA_BACKPEDAL_DEG := 101.25        # DAT_144331174: beyond this it is backpedal, between is sideways
const RDATA_SPRINT_SPEED_SLACK := 5.0      # DAT_143fe4e20: cm/s slack in the "fast enough to sprint" tests
const RDATA_FALL_TOO_FAST := 1.01          # DAT_1442efa68: GetMaxBrakingDeceleration falling-too-fast factor
# Capsule size: AMordhauCharacter ctor, see mordhau_character.gd.

# --- UCharacterMovementComponent (engine) -------------------------------------------------------------------------
# Engine code is statically linked into Mordhau-Win64-Shipping.exe. Values below are the immediates the engine ctor
# UCharacterMovementComponent::UCharacterMovementComponent (rva=0x2f6cc10, called first by
# UAdvancedCharacterMovement's ctor) stores, read by disassembling it at its PDB address (labels.tsv). Fields the
# BP JSON omits keep these values. Offsets match the reads in UMordhauMovementComponent::GetMaxBrakingDeceleration.
class EngineCtor:
	var braking_deceleration_walking := 2048.0		# +0x1b4 = 0x45000000 (UE sets it = MaxAcceleration 2048 at construction)
	var braking_friction := 0.0						# +0x1ac not written -> 0
	var b_use_separate_braking_friction := false	# bitfield at +0x1f0 bit 0 not set by the ctor
	var braking_sub_step_time := 1.0 / 33.0			# +0x1b0 = 0x3cf83e10
	var min_analog_walk_speed := 0.0				# +0x1a4 not written -> 0
	var max_simulation_time_step := 0.05			# +0x29c = 0x3d4ccccd
	var max_simulation_iterations := 8				# +0x2a0 = 8
# Engine constants from the same disassembly (.rdata loads):
const ENGINE_MIN_TICK_TIME := 1e-6             # 0x144000104, CalcVelocity / ApplyVelocityBraking early-out
const ENGINE_OVER_VELOCITY_PERCENT := 1.01     # 0x1442efa68, UMovementComponent::IsExceedingMaxSpeed rva=0x2fb3720
const ENGINE_BRAKE_TO_STOP_SQ := 100.0         # 0x143fe4e40, ApplyVelocityBraking: stop below 10 cm/s
const ENGINE_KINDA_SMALL := 1e-4               # 0x144022350
const ENGINE_BRAKING_STEP_MIN := 1.0 / 75.0    # 0x14497e65c, clamp of BrakingSubStepTime
const ENGINE_BRAKING_STEP_MAX := 0.05          # 0x144014a9c

var engine := EngineCtor.new()

# --- data (filled by load()) --------------------------------------------------------------------------------------
var cfg: CharacterData.Movement	# CharMoveComp values (this pawn's copy: the Rat perk changes MaxWalkSpeedCrouched,
								# Knockback swaps GroundFriction / FallingLateralFriction)
var fx: CharacterData.MoveExtra	# fall damage, knockback and NavAgentProps terms (native ctors + CharMoveComp)
var ch: CharacterData.Character	# BP_MordhauCharacter class-default terms the movement rules read
var gravity_z := -980.0
var terminal_velocity := 4000.0
var jump_cooldown := 0.15   # BP_MordhauCharacter CDO JumpCooldown (AAdvancedCharacter +0x864), used by AAdvancedCharacter::CanJumpInternal_Implementation

# --- state --------------------------------------------------------------------------------------------------------
var mode := Mode.WALKING
var velocity := Vector3.ZERO      # cm/s
var acceleration := Vector3.ZERO  # cm/s^2, from input (UE: Acceleration)
var crouched := false
var wants_sprint := false         # +0xd19 bWantsToSprint (AMordhauCharacter::SprintPressed rva=0x156da50)
var sprint_state := Sprint.FORWARD
var sprint_time := 0.0            # +0xce8, LODTick accumulator, 0 below state 4
var world_time := 0.0
# LastLand (AAdvancedCharacter +0x85c, written by AAdvancedCharacter::Landed at 0x141489862) is not set by either
# constructor nor the BP CDO, so the exe starts it at 0 (zeroed object): no jump in the first JumpCooldown of world time.
var last_landed_time := 0.0       # AAdvancedCharacter +0x85c LastLand: zero-filled (neither ctor writes it, not in the
                                  # BP CDO), first written by AAdvancedCharacter::Landed at 0x141489862 - so no jump in the
                                  # first JumpCooldown seconds of world time, as in the exe

# --- character-level state the movement rules read (AAdvancedCharacter / AMordhauCharacter fields) --------------
var authority := true             # Owner Role == ROLE_Authority (3): CheckFallDamage, Knockback, LODTick falling time
var player_controlled := true     # APawn::IsPlayerControlled (vcall +0x6c0 in AMordhauCharacter::LODTick)
var dead := false                 # AAdvancedCharacter bIsDead
var b_ignore_movement_input := false  # UAdvancedCharacterMovement +0xbc4 bIgnoreMovementInput (zero-filled)
var ragdoll_falling := false      # AAdvancedCharacter +0x505 bIsRagdollFalling (set_is_ragdoll_falling)
var ragdoll_falling_start_time := 0.0     # +0x848 RagdollFallingStartTime
# +0x844 RagdollFallingGetUpStartTime: zero-filled in the exe, where it is compared with world TimeSeconds. This model's
# clock is per pawn (starts at 0 on spawn), so the zero-filled value is taken as "long ago": in the exe a pawn spawned
# after world time RagdollFallingGetUpDuration (1.4 s) is never getting up at spawn. UNCONFIRMED for a pawn spawned
# earlier than that in a world's life.
var ragdoll_falling_get_up_start_time := -INF
var still_time_while_ragdoll_falling := 0.0  # UAdvancedCharacterMovement +0xb90 StillTimeWhileRagdollFalling
var b_is_airborne_from_jump := false    # AAdvancedCharacter +0x869
var b_was_last_land_from_jump := false  # AAdvancedCharacter +0x860
var motion_restriction := 0       # EMovementRestriction of the current motion (UMordhauMotion::GetMovementRestriction)
                                  # and the equipment (UEquipmentSystemComponent), max of both; set by the owner
var knockback_time := 0.0         # UMordhauMovementComponent +0xd4c KnockbackTime
var base_ground_friction := 0.0   # +0xd30 BaseGroundFriction (InitializeComponent)
var base_falling_lateral_friction := 0.0  # +0xd34 BaseFallingLateralFriction
var pending_impulse := Vector3.ZERO       # UCharacterMovementComponent PendingImpulseToApply (+0x274), cm/s
var last_falling_check_velocity_z := 0.0  # UAdvancedCharacterMovement +0xb58 LastFallingCheckVelocityZ
var falling_time := 0.0           # AMordhauCharacter +0xe98 FallingTime
var wants_crouch := false         # AMordhauCharacter +0xde4 bWantsCrouch
var last_crouch_toggle_time := 0.0  # AMordhauCharacter +0xde0 LastCrouchToggleTime (zero-filled)
# CVarToggleSprint "m.ToggleSprint" / CVarToggleCrouch "m.ToggleCrouch" are registered with default 0 by
# _dynamic_initializer_for__CVarToggleSprint__ rva=0x641940 / _dynamic_initializer_for__CVarToggleCrouch__ rva=0x6418d0
# (RegisterConsoleVariable(name, 0, ...)); UMordhauInput::ApplySettings writes the player's ToggleSprint/ToggleCrouch.
var toggle_sprint := 0
var toggle_crouch := 0
# Events for the owner (MordhauCharacter / Fighter), drained each frame: they reach the combat side (stamina, damage).
var fall_damage: Array = []       # amounts CheckFallDamage passed to TakeDamage (Fall type)
var jumps := 0                    # OnJumped count (JumpStaminaCost each)
var trips := 0                    # Trip() calls that started a ragdoll fall
var ragdoll_changes: Array = []   # true / false per SetIsRagdollFalling change (owner: RagdollFalling motion, body)

static func load_default() -> MordhauMovement:
	var m := MordhauMovement.new()
	m.load_data()
	return m

func load_data() -> void:
	cfg = _copy(CharacterData.movement())
	fx = CharacterData.move_extra()
	ch = CharacterData.character()
	jump_cooldown = ch.jump_cooldown
	initialize_component()
	var ini := ini_value("DefaultEngine.ini", "/Script/Engine.PhysicsSettings", "DefaultGravityZ")
	if ini != "": gravity_z = float(ini)
	ini = ini_value("DefaultEngine.ini", "/Script/Engine.PhysicsSettings", "DefaultTerminalVelocity")
	if ini != "": terminal_velocity = float(ini)

static func _copy(src: CharacterData.Movement) -> CharacterData.Movement:
	var c := CharacterData.Movement.new()
	for p in src.get_property_list():
		if p.usage & PROPERTY_USAGE_SCRIPT_VARIABLE:
			c.set(p.name, src.get(p.name))
	return c

# extract/config/<file>: Mordhau's own ini, pulled from pakchunk0 with `repak get` (no game-file edits).
static func ini_value(file: String, section: String, key: String) -> String:
	return UeConfig.value(file, section, key)

# ================================================================================================================
# UMordhauMovementComponent
# ================================================================================================================

# Speed/acceleration factor fields (names from the PDB layout, extract/native/types/UMordhauMovementComponent.h).
# Equipment terms are set by UpdateEquipmentSpeedAndAcceleration rva=0x14dcd30 (weapons; not wired yet, so 0 = no
# weapon); armor terms by UpdateArmorSpeedAndAcceleration rva=0x14dc0c0 (Armor.speed_factors, see apply_loadout).
var equip_speed_cap := 0.0       # +0xd00 EquipmentSpeedFactorOverride (0 = none)
var equip_speed_add := 0.0       # +0xd0c EquipmentSpeedBonusPercentage
var equip_sprint_penalty := 0.0  # +0xd10 EquipmentSubSprintSpeedBonus
var armor_speed := 1.0           # +0xd04 ArmorSpeedFactor (ctor 1.0)
var armor_accel := 1.0           # +0xd08 ArmorAccelerationFactor (ctor 1.0)
var equip_accel_add := 0.0       # +0xd14 EquipmentAccelerationBonusPercentage

# UMordhauMovementComponent::GetSpeedFactor rva=0x14bf9a0:
#   s = EquipmentSpeedBonusPercentage + ArmorSpeedFactor + (1 - t) * EquipmentSubSprintSpeedBonus
#   if EquipmentSpeedFactorOverride > 0: s = min(override, s); (perk multiplier skipped); s *= MotionSpeedFactor (+0xcf4,
#   1.0); max(s, 0). GetMaxSpeed multiplies its modifier by this (LAB_1414be327), so ArmorSpeedFactor scales walk,
#   strafe, backpedal, crouch and sprint speed, but not SupersprintModifier / ChasingModifier (they skip the factor).
func get_speed_factor(t: float) -> float:
	var s := equip_speed_add + armor_speed + (1.0 - t) * equip_sprint_penalty
	if equip_speed_cap > 0.0 and s > equip_speed_cap:
		s = equip_speed_cap
	return maxf(s * 1.0, 0.0)  # * +0xcf4 MotionSpeedFactor (1.0)

# UMordhauMovementComponent::UpdateArmorSpeedAndAcceleration rva=0x14dc0c0, as ported in armor.gd (Armor.speed_factors):
# ArmorSpeedFactor / ArmorAccelerationFactor from the head, upper chest and legs wearables (or the Tank perk / game
# override), and with the Rat perk (HasPerk 7) MaxWalkSpeedCrouched (+0x190) = MaxWalkSpeedCrouchedWithRatPerk (+0xcbc).
# Tank values: UPerkSystemComponent native ctor defaults (NativeCtor) TankArmorSpeedFactor / TankArmorAccelerationFactor.
func apply_loadout(l: Loadout, game_override: Armor.SpeedFactors = null) -> void:
	var pk := CharacterData.perks()
	var sf := Armor.speed_factors(l, pk, game_override)
	armor_speed = sf.speed
	armor_accel = sf.accel
	if l.has_perk(Armor.PERK_RAT):
		cfg.max_walk_speed_crouched = cfg.max_walk_speed_crouched_with_rat_perk

# UMordhauMovementComponent::GetMaxSpeed rva=0x14be1d0
func get_max_speed() -> float:
	var sprint := cfg.sprint_modifier
	var partial := cfg.partial_sprint_modifier
	var m := 1.0
	var apply_factor := true
	if mode == Mode.WALKING or mode == Mode.FALLING:
		if sprint_state == Sprint.BACKPEDAL:
			m = 1.0 * cfg.backpedal_modifier * 1.0  # +0xcf8 * BackpedalModifier * +0xcfc
		elif sprint_state == Sprint.SIDEWAYS:
			m = NATIVE_SIDEWAYS_MODIFIER
		elif crouched and mode != Mode.FALLING:
			m = 1.0
		elif sprint_state == Sprint.SUPER:
			if mode == Mode.FALLING:
				m = sprint
			else:
				m = cfg.supersprint_modifier
				apply_factor = false
		elif sprint_state >= Sprint.SPRINT:
			m = sprint
			var tt := sprint_time
			if sprint_state == Sprint.CHASE:
				m = cfg.chasing_modifier
			var reach := cfg.sprint_time_to_reach_max_sprint
			if absf(reach) > 1e-8:
				m = (m - partial) * clampf(tt / reach, 0.0, 1.0) + partial
			if sprint_state == Sprint.CHASE:
				apply_factor = false
		elif sprint_state == Sprint.PARTIAL:
			m = partial
		if apply_factor:
			var span := sprint - partial
			var t := 0.0
			if absf(span) > 1e-8:
				t = clampf((m - partial) / span, 0.0, 1.0)
			else:
				t = 0.0 if m < sprint else 1.0
			m *= get_speed_factor(t)
	# falling and not in knockback (+0xd4c <= 0, IsInKnockback rva=0x14c2720) -> MaxSpeedFalling (+0xd2c); falling in a
	# knockback takes UAdvancedCharacterMovement::GetMaxSpeed (LAB_1414be373: MovementMode != 3 or 0 < KnockbackTime)
	var base := cfg.max_speed_falling if mode == Mode.FALLING and knockback_time <= 0.0 else _super_max_speed()
	return m * base

# UAdvancedCharacterMovement::GetMaxSpeed rva=0x1484b20 (UE's own switch: crouched walking uses MaxWalkSpeedCrouched)
func _super_max_speed() -> float:
	if mode == Mode.WALKING:
		return cfg.max_walk_speed_crouched if crouched else cfg.max_walk_speed
	if mode == Mode.FALLING:
		return cfg.max_walk_speed
	return 0.0

# UMordhauMovementComponent::GetAccelerationFactor rva=0x14bbbe0:
#   EquipmentAccelerationBonusPercentage (+0xd14) + ArmorAccelerationFactor (+0xd08), clamped at 0 from below
func get_acceleration_factor() -> float:
	return maxf(equip_accel_add + armor_accel, 0.0)

# UMordhauMovementComponent::GetMaxAcceleration rva=0x14be0b0
func get_max_acceleration() -> float:
	if mode != Mode.WALKING:
		return cfg.max_acceleration
	if crouched:
		return cfg.walk_acceleration
	# GetAccelerationFactor scales only the sprint-class accelerations; WalkAcceleration, SupersprintAcceleration and
	# crouch come back unscaled.
	var k := get_acceleration_factor()
	match sprint_state:
		Sprint.SUPER: return NATIVE_SUPERSPRINT_ACCELERATION
		Sprint.SPRINT: return k * cfg.sprint_acceleration
		Sprint.PARTIAL, Sprint.RUSH, Sprint.CHASE: return k * cfg.partial_sprint_acceleration
	return cfg.walk_acceleration

# UMordhauMovementComponent::GetMaxBrakingDeceleration rva=0x14be160
func get_max_braking_deceleration() -> float:
	if mode == Mode.FALLING:
		var ms := maxf(get_max_speed(), 0.0)
		var v2 := velocity.x * velocity.x + velocity.z * velocity.z
		if ms * ms * RDATA_FALL_TOO_FAST < v2:
			return cfg.braking_deceleration_falling_too_fast
		return cfg.braking_deceleration_falling
	return engine.braking_deceleration_walking

# UMordhauMovementComponent::LODTick rva=0x14c4d60, the sprint-state part (decompile lines ~1010-1160, 1748-1766).
# move_dir: the input acceleration direction; facing: actor forward. Stamina, perks and chase/rush are not modelled.
# Movement restriction (AMordhauCharacter::GetMovementRestriction, read at decompile line 1201): bOnlyPartialSprint =
# (r == PartialSprint), and below PartialSprintModifier x MaxWalkSpeed it is set for any r; bSprintIsAllowed is false
# for r == Walk, otherwise it needs MaxWalkSpeed (less the slack).
func update_sprint_state(dt: float, facing: Vector3) -> void:
	var speed := velocity.length()
	var k := get_speed_factor(0.0)
	var mws := cfg.max_walk_speed
	var r := get_movement_restriction()
	var partial := r == CombatEnums.MovementRestriction.PARTIAL_SPRINT \
		or speed + RDATA_SPRINT_SPEED_SLACK < k * mws * cfg.partial_sprint_modifier   # +0xd1b
	var can_sprint := r != CombatEnums.MovementRestriction.WALK \
		and not (speed + RDATA_SPRINT_SPEED_SLACK < k * mws)                           # +0xd1a
	var dir := Vector3(acceleration.x, 0, acceleration.z)
	if dir.is_zero_approx():
		sprint_state = Sprint.FORWARD
	else:
		var a := rad_to_deg(Vector2(facing.x, facing.z).angle_to(Vector2(dir.x, dir.z)))
		sprint_state = Sprint.FORWARD
		if absf(a) <= RDATA_FORWARD_CONE_DEG:
			if wants_sprint and can_sprint:
				sprint_state = Sprint.PARTIAL if partial else Sprint.SPRINT
		else:
			sprint_state = Sprint.BACKPEDAL if absf(a) > RDATA_BACKPEDAL_DEG else Sprint.SIDEWAYS
	if sprint_state < Sprint.SPRINT:
		sprint_time = 0.0
	else:
		sprint_time += dt

# ================================================================================================================
# UCharacterMovementComponent (engine, disassembled from the shipped exe at its PDB address) / UAdvancedCharacterMovement
# ================================================================================================================

# UAdvancedCharacterMovement::CalcVelocity rva=0x1459ab0 (Ghidra C; a 33-byte override running into the engine's
# UCharacterMovementComponent::CalcVelocity rva=0x2f70490). Requested-move (AI path following), RVO avoidance and
# bForceMaxAccel are off for a player and left out.
func calc_velocity(dt: float, friction: float, fluid: bool, braking_decel: float) -> void:
	if dt < ENGINE_MIN_TICK_TIME: return
	friction = maxf(friction, 0.0)
	var max_speed := get_max_speed()
	# MaxInputSpeed = max(MaxSpeed * AnalogInputModifier, GetMinAnalogSpeed()); AnalogInputModifier is 1 for keys
	var min_analog: float = engine.min_analog_walk_speed if mode == Mode.WALKING else 0.0
	var max_input_speed := maxf(max_speed * 1.0, min_analog)
	max_speed = max_input_speed
	var zero_accel := acceleration == Vector3.ZERO
	var over_max := is_exceeding_max_speed(max_speed)
	if zero_accel or over_max:
		var old := velocity
		var bf: float = engine.braking_friction if engine.b_use_separate_braking_friction else friction
		apply_velocity_braking(dt, bf, braking_decel)
		# braking may not take us below max speed if we started above it and still push forward
		if over_max and velocity.length_squared() < max_speed * max_speed and acceleration.dot(old) > 0.0:
			velocity = old.normalized() * max_speed
	else:
		# friction limits how fast the direction can change
		var dir := acceleration.normalized()
		var vs := velocity.length()
		velocity = velocity - (velocity - dir * vs) * minf(dt * friction, 1.0)
	if fluid:
		velocity *= 1.0 - minf(friction * dt, 1.0)
	if not zero_accel:
		var new_max := velocity.length() if is_exceeding_max_speed(max_input_speed) else max_input_speed
		velocity += acceleration * dt
		velocity = velocity.limit_length(new_max) if new_max >= ENGINE_KINDA_SMALL else Vector3.ZERO

# UMovementComponent::IsExceedingMaxSpeed rva=0x2fb3720: |V|^2 > max(MaxSpeed,0)^2 * 1.01
func is_exceeding_max_speed(max_speed: float) -> bool:
	max_speed = maxf(max_speed, 0.0)
	return velocity.length_squared() > max_speed * max_speed * ENGINE_OVER_VELOCITY_PERCENT

# UCharacterMovementComponent::ApplyVelocityBraking rva=0x2f6f930
func apply_velocity_braking(dt: float, friction: float, braking_decel: float) -> void:
	if velocity == Vector3.ZERO or dt < ENGINE_MIN_TICK_TIME: return
	friction = maxf(maxf(cfg.braking_friction_factor, 0.0) * friction, 0.0)
	braking_decel = maxf(braking_decel, 0.0)
	var zero_friction := friction == 0.0
	var zero_braking := braking_decel == 0.0
	if zero_friction and zero_braking: return
	var old := velocity
	var remaining := dt
	var max_step := clampf(engine.braking_sub_step_time, ENGINE_BRAKING_STEP_MIN, ENGINE_BRAKING_STEP_MAX)
	var rev := Vector3.ZERO if zero_braking else -braking_decel * _safe_normal(velocity)
	while remaining >= ENGINE_MIN_TICK_TIME:
		var t := minf(max_step, remaining * 0.5) if (remaining > max_step and not zero_friction) else remaining
		remaining -= t
		velocity = velocity + (-friction * velocity + rev) * t
		if velocity.dot(old) <= 0.0:   # never reverse direction
			velocity = Vector3.ZERO
			return
	var vs2 := velocity.length_squared()
	if vs2 <= ENGINE_KINDA_SMALL or (not zero_braking and vs2 <= ENGINE_BRAKE_TO_STOP_SQ):
		velocity = Vector3.ZERO

# FVector::GetSafeNormal as the engine inlines it: zero below 1e-8 squared length
static func _safe_normal(v: Vector3) -> Vector3:
	return Vector3.ZERO if v.length_squared() < 1e-8 else v.normalized()

# UCharacterMovementComponent::GetFallingLateralAcceleration rva=0x2f794b0 -> GetAirControl rva=0x2f790f0 ->
# BoostAirControl rva=0x2f701e0, then ClampToMaxSize(GetMaxAcceleration()).
func get_falling_lateral_acceleration() -> Vector3:
	var fa := Vector3(acceleration.x, 0.0, acceleration.z)
	if fa.x * fa.x + fa.z * fa.z > 0.0:
		var ac := cfg.air_control
		if ac != 0.0:
			var v2 := velocity.x * velocity.x + velocity.z * velocity.z
			var thr := cfg.air_control_boost_velocity_threshold
			if cfg.air_control_boost_multiplier > 0.0 and v2 < thr * thr:
				ac = minf(cfg.air_control_boost_multiplier * ac, 1.0)
		fa = (ac * fa).limit_length(get_max_acceleration())
	return fa

# UCharacterMovementComponent::DoJump rva=0x2f775e0: Velocity.Z = max(JumpZVelocity, Velocity.Z); SetMovementMode(3).
# Gated like AMordhauCharacter::CanJumpInternal_Implementation rva=0x1532630, which tail-jumps (0x1415326ff) to
# AAdvancedCharacter::CanJumpInternal_Implementation rva=0x1459fb0: JumpCooldown (+0x864) + LastLand (+0x85c) >
# TimeSeconds -> false (0x141459fd2..0x141459fe9), i.e. allowed from LastLand + JumpCooldown on; then
# ACharacter::CanJumpInternal_Implementation (walking). Not modelled: AMordhauCharacter's own leg-disabled, attack
# window, LastDodgeTime + DodgeDuration (+0xea0 / +0xea4) and movement-restriction checks.
# AMordhauCharacter::CanJumpInternal_Implementation also refuses while GetMovementRestriction == NoMovement and while
# IsRagdollFallingOrGettingUp (vcall +0x960).
func can_jump() -> bool:
	if get_movement_restriction() == CombatEnums.MovementRestriction.NO_MOVEMENT:
		return false
	if is_ragdoll_falling_or_getting_up():
		return false
	return mode == Mode.WALKING and world_time >= last_landed_time + jump_cooldown

func do_jump() -> bool:
	if not can_jump(): return false
	velocity.y = maxf(cfg.jump_z_velocity, velocity.y)
	mode = Mode.FALLING
	on_jumped()
	return true

# AAdvancedCharacter::OnJumped_Implementation rva=0x148d580: bIsAirborneFromJump = true (and LastJump / bJumped).
# AMordhauCharacter::OnJumped_Implementation rva=0x1554610 then OffsetStamina(-(int)JumpStaminaCost) and stops the
# stamina regeneration; both live on the combat side (MotionSystem), so the jump is counted here and the owner
# applies them (Fighter.apply_movement_events).
func on_jumped() -> void:
	b_is_airborne_from_jump = true
	jumps += 1

# AAdvancedCharacter::Landed rva=0x1489820: bWasLastLandFromJump = bIsAirborneFromJump, LastLand = TimeSeconds,
# bIsAirborneFromJump = false.
func landed() -> void:
	b_was_last_land_from_jump = b_is_airborne_from_jump
	last_landed_time = world_time
	b_is_airborne_from_jump = false

# AAdvancedCharacter::IsRagdollFallingOrGettingUp rva=0x1487ca0: bIsRagdollFalling, or TimeSeconds is still before
# RagdollFallingGetUpStartTime + RagdollFallingGetUpDuration (false from that sum on, `<=`).
func is_ragdoll_falling_or_getting_up() -> bool:
	if ragdoll_falling:
		return true
	return world_time < ragdoll_falling_get_up_start_time + ch.ragdoll_falling_get_up_duration

# AMordhauCharacter::Trip rva=0x156f7b0 -> AAdvancedCharacter::Trip rva=0x14a78c0: alive, not ragdoll falling, not
# bDisableRagdollFalling (+0x7f8) and not IsRagdollFallingOrGettingUp (vcall +0x960) -> SetIsRagdollFalling(true)
# (vcall +0xa58), true. (The vehicle StopDriving of AMordhauCharacter::Trip has no vehicle here.)
func trip() -> bool:
	if dead or ragdoll_falling or ch.b_disable_ragdoll_falling:
		return false
	if is_ragdoll_falling_or_getting_up():
		return false
	set_is_ragdoll_falling(true)
	trips += 1
	return true

# AMordhauCharacter::SetIsRagdollFalling rva=0x1568ad0 -> AAdvancedCharacter::SetIsRagdollFalling rva=0x14a0980: dead
# or unchanged -> nothing; on: RagdollFallingStartTime = TimeSeconds; off: RagdollFallingGetUpStartTime = TimeSeconds.
# Also, not modelled here: the perch radius swap, the mesh physics blend (UCharacterMeshComponent), the
# ReplicatedCharacterFlags bit 4, and on authority the RagdollFalling net motion (MotionType 0x1a) for the combat
# side - the change is queued in ragdoll_changes for the owner.
func set_is_ragdoll_falling(on: bool) -> void:
	if dead or ragdoll_falling == on:
		return
	ragdoll_falling = on
	if on:
		ragdoll_falling_start_time = world_time
	else:
		ragdoll_falling_get_up_start_time = world_time
	ragdoll_changes.append(on)

# UAdvancedCharacterMovement::TickComponent rva=0x14a3e80, the ragdoll get-up (disassembly 0x1414a4365..0x1414a4419):
# alive (+0x504) and bIsRagdollFalling (+0x505) and |Velocity|^2 <= RagdollFallingMinVelocityToGetUp^2 ->
# StillTimeWhileRagdollFalling += dt; at >= RagdollFallingTimeAtMinVelocityToGetUp with RagdollFallingStartTime +
# RagdollFallingMinTime < TimeSeconds and Role == Authority -> SetIsRagdollFalling(false). Not ragdoll falling or
# faster -> StillTimeWhileRagdollFalling = 0. Dead -> untouched.
func tick_ragdoll_get_up(dt: float) -> void:
	if dead:
		return
	var v := ch.ragdoll_falling_min_velocity_to_get_up
	if not ragdoll_falling or velocity.length_squared() > v * v:
		still_time_while_ragdoll_falling = 0.0
		return
	still_time_while_ragdoll_falling += dt
	if still_time_while_ragdoll_falling >= ch.ragdoll_falling_time_at_min_velocity_to_get_up 			and ragdoll_falling_start_time + ch.ragdoll_falling_min_time < world_time and authority:
		set_is_ragdoll_falling(false)

# AMordhauCharacter::GetMovementRestriction rva=0x1540f50: the current motion's restriction; None within 0.1 s of a
# landing that ended a jump (TimeSeconds < LastLand + 0.1 and bWasLastLandFromJump) -> PartialSprint (1); then the max
# with the equipment's (UEquipmentSystemComponent::GetMovementRestriction; both folded into motion_restriction).
# The Rush perk branch (HasPerk 0xe) is not modelled (no perk system yet).
func get_movement_restriction() -> int:
	var r := motion_restriction
	if world_time < last_landed_time + 0.1 and b_was_last_land_from_jump and r == 0:
		r = CombatEnums.MovementRestriction.PARTIAL_SPRINT
	return r

# AMordhauCharacter::IsMoveInputIgnored rva=0x154afe0: GetMovementRestriction == NoMovement or
# IsRagdollFallingOrGettingUp (vcall +0x960); the controller's own IsMoveInputIgnored is the caller's input gate.
# APawn::Internal_AddMovementInput rva=0x32eed40 drops the input when it is true (vcall +0x758 at 0x1432eed58).
func is_move_input_ignored() -> bool:
	return get_movement_restriction() == CombatEnums.MovementRestriction.NO_MOVEMENT or is_ragdoll_falling_or_getting_up()

# UMordhauMovementComponent::ConstrainInputAcceleration rva=0x14b7a90 -> UAdvancedCharacterMovement::
# ConstrainInputAcceleration rva=0x145aee0: bIgnoreMovementInput -> zero, else the engine's constraint (the input
# here is already lateral).
func constrain_input_acceleration(a: Vector3) -> Vector3:
	return Vector3.ZERO if b_ignore_movement_input else a

# ---- crouch -------------------------------------------------------------------------------------------------------
# AMordhauCharacter::CrouchPressed rva=0x1538040: m.ToggleCrouch == 1 -> bWantsCrouch = !bWantsCrouch, else true.
func crouch_pressed() -> void:
	wants_crouch = (not wants_crouch) if toggle_crouch == 1 else true

# AMordhauCharacter::CrouchReleased rva=0x1538100: m.ToggleCrouch != 1 -> bWantsCrouch = false.
func crouch_released() -> void:
	if toggle_crouch != 1:
		wants_crouch = false

# AMordhauCharacter::CanCrouch rva=0x1532230: when not crouched, disabled legs (not ported) or GetMovementRestriction ==
# NoMovement -> false; then AAdvancedCharacter::CanCrouch rva=0x1459e10: not crouched and IsRagdollFallingOrGettingUp
# -> false; true when not crouched, NavAgentProps.bCanCrouch and the root is not simulating physics (no ragdoll here).
func can_crouch() -> bool:
	if not crouched:
		if get_movement_restriction() == CombatEnums.MovementRestriction.NO_MOVEMENT:
			return false
		if is_ragdoll_falling_or_getting_up():
			return false
	return not crouched and fx.b_can_crouch

# AMordhauCharacter::LODTick rva=0x154c390, crouch part (from 0x14154c4cf): bIsCrouched != bWantsCrouch and
# CrouchCooldown + LastCrouchToggleTime < TimeSeconds -> LastCrouchToggleTime = TimeSeconds, then Crouch() / UnCrouch()
# (ACharacter vtable). Returns true = crouch, false = uncrouch, null = no change. Crouch() goes through CanCrouch
# (ACharacter::Crouch); UnCrouch always proceeds.
func update_crouch():
	if crouched == wants_crouch:
		return null
	if not (ch.crouch_cooldown + last_crouch_toggle_time < world_time):
		return null
	last_crouch_toggle_time = world_time
	if wants_crouch:
		return true if can_crouch() else null
	return false

# ---- sprint -------------------------------------------------------------------------------------------------------
# AMordhauCharacter::SprintPressed rva=0x156da50 (keyboard; the gamepad branch is skipped): m.ToggleSprint != 0 and
# bWantsToSprint -> false; otherwise true.
func sprint_pressed() -> void:
	wants_sprint = false if (toggle_sprint != 0 and wants_sprint) else true

# AMordhauCharacter::SprintReleased rva=0x156dba0: m.ToggleSprint == 0 -> bWantsToSprint = false.
func sprint_released() -> void:
	if toggle_sprint == 0:
		wants_sprint = false

# AMordhauCharacter::StartSprinting rva=0x156dcc0 / StopSprinting rva=0x156deb0: bWantsToSprint (+0xd19) = 1 / 0.
func start_sprinting() -> void:
	wants_sprint = true

func stop_sprinting() -> void:
	wants_sprint = false

# AMordhauCharacter::MoveForward rva=0x1550170: a zero forward axis on keyboard with m.ToggleSprint == 1 ->
# StopSprinting, then AAdvancedCharacter::MoveForward rva=0x148a6c0 (Value x actor forward into AddMovementInput).
func move_forward_axis(value: float) -> void:
	if value == 0.0 and toggle_sprint == 1:
		stop_sprinting()

# ---- knockback ----------------------------------------------------------------------------------------------------
# UMordhauMovementComponent::InitializeComponent rva=0x14c1300: BaseGroundFriction / BaseFallingLateralFriction = the
# current GroundFriction / FallingLateralFriction (what PostCharacterMovementTick restores after a knockback).
func initialize_component() -> void:
	base_ground_friction = cfg.ground_friction
	base_falling_lateral_friction = cfg.falling_lateral_friction

# UMordhauMovementComponent::IsInKnockback rva=0x14c2720 (AMordhauCharacter::IsInKnockback rva=0x154aad0 forwards):
# 0 < KnockbackTime.
func is_in_knockback() -> bool:
	return 0.0 < knockback_time

# UMordhauMovementComponent::Knockback rva=0x14c2820. amount: cm/s on Godot axes (UE Z = Godot Y). Authority:
# AddImpulse(Amount + (0, 0, IsFalling ? 0 : KnockbackUpImpulse), bVelocityChange = true) (vcall +0x810; IsFalling
# vcall +0x560). Every role: KnockbackTime = KnockbackDuration, FallingLateralFriction =
# KnockbackFallingLateralFriction, GroundFriction = KnockbackGroundFriction.
func knockback(amount: Vector3) -> void:
	if authority:
		var up := 0.0 if mode == Mode.FALLING else fx.knockback_up_impulse
		add_impulse(Vector3(amount.x, up + amount.y, amount.z), true)
	knockback_time = fx.knockback_duration
	cfg.falling_lateral_friction = fx.knockback_falling_lateral_friction
	cfg.ground_friction = fx.knockback_ground_friction

# UCharacterMovementComponent::AddImpulse rva=0x2f6da30 with bVelocityChange: PendingImpulseToApply += Impulse (the
# mass division of the other path is not used by any ported caller).
func add_impulse(impulse: Vector3, _velocity_change: bool) -> void:
	pending_impulse += impulse

# UCharacterMovementComponent::ApplyAccumulatedForces rva=0x2f6e650: PendingImpulse.Z (+0x27c) != 0, on the ground
# (IsMovingOnGround, vcall +0x568) and (GetGravityZ + PendingForce.Z) * dt + PendingImpulse.Z > 1e-8 -> SetMovementMode
# (Falling); then Velocity += PendingImpulse (+ force x dt; no forces here) and the pending terms are cleared.
func apply_accumulated_forces(dt: float) -> void:
	if pending_impulse.y != 0.0 and mode == Mode.WALKING:
		if gravity() * dt + pending_impulse.y > 1e-8:
			mode = Mode.FALLING
	velocity += pending_impulse
	pending_impulse = Vector3.ZERO

# UMordhauMovementComponent::PostCharacterMovementTick rva=0x14d0710: KnockbackTime -= dt; at or below 0 the base
# frictions come back and KnockbackTime = 0.
func post_character_movement_tick(dt: float) -> void:
	knockback_time -= dt
	if knockback_time <= 0.0:
		cfg.falling_lateral_friction = base_falling_lateral_friction
		cfg.ground_friction = base_ground_friction
		knockback_time = 0.0

# ---- fall damage --------------------------------------------------------------------------------------------------
# UAdvancedCharacterMovement::CheckFallDamage rva=0x145a230 (authority, AAdvancedCharacter owner): the Ragdoll* terms
# when bIsRagdollFalling (+0x505), else MinVelocityForFallDamage / FallDamageOffset / FallDamageFactor;
# Delta = CurrentVelocityZ - LastFallingCheckVelocityZ, LastFallingCheckVelocityZ = CurrentVelocityZ;
# MinVelocity < Delta and (Delta + Offset) * Factor > 0 -> TakeDamage(that, EMordhauDamageType::Fall, the character
# itself as causer) - queued in fall_damage for the owner. UAdvancedCharacterMovement::PerformMovement rva=0x1495cc0
# calls it after every move with Velocity.Z (vcall +0xb38 = slot 359 in types/UAdvancedCharacterMovement.h), and
# OnMovementModeChanged rva=0x14901a0 with 0 on entering Walking: the speed lost at the landing is the Delta.
func check_fall_damage(current_velocity_z: float) -> float:
	if not authority:
		return 0.0
	var min_v := fx.min_velocity_for_fall_damage
	var off := fx.fall_damage_offset
	var fac := fx.fall_damage_factor
	if ragdoll_falling:
		min_v = fx.ragdoll_min_velocity_for_fall_damage
		off = fx.ragdoll_fall_damage_offset
		fac = fx.ragdoll_fall_damage_factor
	var delta := current_velocity_z - last_falling_check_velocity_z
	last_falling_check_velocity_z = current_velocity_z
	if min_v < delta:
		var d := (delta + off) * fac
		if 0.0 < d:
			fall_damage.append(d)
			return d
	return 0.0

# ---- falling too long -> trip -------------------------------------------------------------------------------------
# AMordhauCharacter::LODTick rva=0x154c390, falling part: authority, IsPlayerControlled (vcall +0x6c0), alive,
# Velocity.Z < -1 (comiss at 0x14154c47c on the movement component's +0xcc), IsAirborne (vcall +0x9e0) and not
# bIsRagdollFalling -> FallingTime += dt; above FallingTimeToRagdoll -> Trip() (vcall +0x8f0) and FallingTime = 0;
# any other frame resets FallingTime.
func lod_tick_falling(dt: float) -> void:
	if authority and player_controlled and not dead and velocity.y < -1.0 and is_airborne() and not ragdoll_falling:
		falling_time += dt
		if falling_time <= ch.falling_time_to_ragdoll:
			return
		trip()
	falling_time = 0.0

# AMordhauCharacter::IsAirborne rva=0x154a7f0 (no vehicle; Role > SimulatedProxy): the movement's IsFalling.
func is_airborne() -> bool:
	return mode == Mode.FALLING

# GetGravityZ = world gravity (DefaultGravityZ) * GravityScale
func gravity() -> float:
	return gravity_z * cfg.gravity_scale

# UCharacterMovementComponent::GetSimulationTimeStep, inlined in PhysWalking rva=0x2f81730: split the frame into
# steps of at most MaxSimulationTimeStep, halving the remainder, for up to MaxSimulationIterations.
func _sim_step(remaining: float, iterations: int) -> float:
	var mts: float = engine.max_simulation_time_step
	var step := remaining
	if remaining > mts and iterations < int(engine.max_simulation_iterations):
		step = minf(mts, remaining * 0.5)
	return maxf(step, ENGINE_MIN_TICK_TIME)

# One frame. input: wanted move direction in world space (lateral, length <= 1; UE AddMovementInput summed).
# facing: actor forward. Returns the position delta in cm; the body moves by it and reports floor contact back.
func tick(dt: float, input: Vector3, facing: Vector3) -> Vector3:
	world_time += dt
	input.y = 0.0
	if is_move_input_ignored():
		input = Vector3.ZERO     # APawn::Internal_AddMovementInput drops it (IsMoveInputIgnored)
	input = constrain_input_acceleration(input)
	# ScaleInputAcceleration: GetMaxAcceleration() * input clamped to 1. LODTick reads its direction for the sprint
	# state first; PerformMovement then uses the acceleration of the new state.
	acceleration = input.limit_length(1.0)
	update_sprint_state(dt, facing)
	acceleration = input.limit_length(1.0) * get_max_acceleration()
	apply_accumulated_forces(dt)
	var delta := Vector3.ZERO
	var remaining := dt
	var iters := 0
	while remaining >= ENGINE_MIN_TICK_TIME and iters < int(engine.max_simulation_iterations):
		iters += 1
		var step := _sim_step(remaining, iters)
		remaining -= step
		if mode == Mode.WALKING:
			# PhysWalking: Acceleration.Z = 0, MaintainHorizontalGroundVelocity, CalcVelocity(GroundFriction)
			velocity.y = 0.0
			calc_velocity(step, cfg.ground_friction, false, get_max_braking_deceleration())
			delta += velocity * step
		elif mode == Mode.FALLING:
			# PhysFalling rva=0x2f7ef00: lateral CalcVelocity with Velocity.Z zeroed and the air-control acceleration
			# swapped in, then NewFallVelocity rva=0x2f7cec0 (gravity, clamp to TerminalVelocity), then
			# Adjusted = 0.5 * (OldVelocity + Velocity) * dt.
			var old := velocity
			var saved := acceleration
			acceleration = get_falling_lateral_acceleration()
			velocity.y = 0.0
			calc_velocity(step, cfg.falling_lateral_friction, false, get_max_braking_deceleration())
			velocity.y = old.y
			acceleration = saved
			velocity.y += gravity() * step
			if velocity.length_squared() > terminal_velocity * terminal_velocity and -velocity.y > terminal_velocity:
				velocity.y = -terminal_velocity
			delta += 0.5 * (old + velocity) * step
	check_fall_damage(velocity.y)		# UAdvancedCharacterMovement::PerformMovement tail
	post_character_movement_tick(dt)
	tick_ragdoll_get_up(dt)
	lod_tick_falling(dt)
	return delta

# Floor contact from the body: landing -> Walking (AAdvancedCharacter::Landed: LastLand for JumpCooldown and
# bWasLastLandFromJump; OnMovementModeChanged -> CheckFallDamage(0)); losing the floor -> Falling.
func set_on_floor(on_floor: bool) -> void:
	if mode == Mode.FALLING and on_floor and velocity.y <= 0.0:
		mode = Mode.WALKING
		velocity.y = 0.0
		landed()
		check_fall_damage(0.0)
	elif mode == Mode.WALKING and not on_floor:
		mode = Mode.FALLING

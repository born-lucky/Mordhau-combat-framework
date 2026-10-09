//! Exe-exact character movement (the default mode): AMordhauCharacter / UMordhauMovementComponent /
//! UAdvancedCharacterMovement rules on top of UE 4.26's UCharacterMovementComponent, in binary32 as the shipped exe
//! computes them, in UE axes (cm, X forward, Y right, Z up), with real collision movement against a `World`.
//!
//! Each rule cites the disassembly it was read from. Where the exe differs from the GDScript reference (kept bit for
//! bit in `compat` under the `reference_compat` feature):
//!  1. Arithmetic: every scalar is binary32 (SSE ss ops) in the exe; the reference computes in f64 and rounds only at
//!     Godot Vector3 stores. Record values are the exe's f32s (the reference keeps the f64 of their shortest decimal,
//!     e.g. PartialSprintModifier 1.175 vs 1.17499995f).
//!  2. Literals: StrafeModifier is the f32 0x3f733333 (reference: f64 0.95), BrakingSubStepTime 0x3cf83e10 (f64 1/33),
//!     MaxSimulationTimeStep 0x3d4ccccd (0.05), the 1/75 and 0.05 braking clamps, 1.01, SMALL_NUMBER 1e-8f,
//!     KINDA_SMALL_NUMBER 1e-4f: all binary32 constants in .rdata.
//!  3. Vector ops: UE GetSafeNormal (== 1 shortcut, < 1e-8 -> zero, else * InvSqrt = rsqrtss + 2 Newton steps) and
//!     GetClampedToMaxSize (< 1e-4 -> zero, else * InvSqrt * Max) instead of Godot normalized() / limit_length()
//!     (divide by sqrt); ScaleInputAcceleration clamps the input the UE way.
//!  4. Axes: UE (X forward, Y right, Z up); the reference works on Godot axes (Y up), which reorders f32 sums.
//!  5. AnalogInputModifier (ComputeAnalogInputModifier rva=0x2f74f50) scales MaxInputSpeed in CalcVelocity; the
//!     reference takes 1. It is Size * (1 / MaxAccel) clamped, so it can sit an ulp under 1.
//!  6. Sprint direction angle: acosf(clamp(ActorForward | Input.GetSafeNormal(), -1, 1)) * 57.2957764f in 3D
//!     (LODTick 0x1414c7127..0x1414c7189), not a 2D atan2 angle_to + f64 rad_to_deg; bots use the velocity direction;
//!     a zero input is an exact-zero test (the reference: is_zero_approx 1e-5).
//!  7. Frame order: world TimeSeconds advances before any tick; the reference advanced it inside tick, after its crouch
//!     cooldown and jump gates, so those saw the previous frame's time. The Mordhau LODTick parts (falling -> Trip,
//!     crouch toggle) run in the actor tick before movement (the reference ran the falling check after the move).
//!  8. Jumping goes through CheckJumpInput / CanJump at the movement tick, with gates the reference lacks:
//!     !bIsCrouched, !bWantsToCrouch, JumpCurrentCount < JumpMaxCount (1), TimeSeconds > LastDodgeTime + DodgeDuration
//!     (0.375 s, so no jump in a world's first 0.375 s), plus the leg-disabled and attack-window gates (not modelled).
//!  9. Crouch: AMordhauCharacter's toggle sets the movement component's bWantsToCrouch; UpdateCharacterStateBeforeMovement
//!     crouches while walking OR falling (CanCrouchInCurrentState), keeps the base with a swept move down, and UnCrouch
//!     needs headroom (overlap tests) and can fail. The reference crouched instantly, walking only, without collision.
//! 10. Landing: the exe lands inside PhysFalling's sweep (IsValidLandingSpot -> ProcessLanded), and the landing's
//!     CheckFallDamage(0) (OnMovementModeChanged rva=0x14901a0) measures against the previous frame's Velocity.Z; the
//!     reference landed after a whole falling tick with its host's floor test, one frame of gravity later.
//! 11. Collision movement: PhysWalking with FindFloor / ComputeFloorDist / perching, MoveAlongFloor ramps, StepUp,
//!     SlideAlongSurface / TwoWallAdjust, AdjustFloorHeight, CheckFall / StartFalling, MoveComponent's pull-back, the
//!     walking velocity recomputed from the moved distance (Location delta * (1 / dt), quantized by float positions);
//!     PhysFalling's apex substep (ForceJumpPeakSubstep), air-control limiting on walls, NewFallVelocity's terminal
//!     velocity projection. The reference had a flat floor and no sweeps.
//! 12. Knockback: AddImpulse ignores a zero impulse and MOVE_None and clears the pending launch; ApplyAccumulatedForces
//!     also counts PendingForce.Z.
//! 13. RagdollFallingGetUpStartTime (zero-filled) compares with world TimeSeconds: every pawn of a world younger than
//!     RagdollFallingGetUpDuration (1.4 s) is "getting up" (no input, no jump, no crouch). The reference used -inf.
//! 14. Perks the reference skipped: the Dwarf speed modifier multiplies in both GetSpeedFactor and GetMaxSpeed.
//! 15. UMordhauMovementComponent::LODTick rva=0x14c4d60 in full (exe_lod.rs; the reference has only the sprint state):
//!     turn-sprint prevention (FC_TurnSprintPrevention curves), the enemy avoidance bubble with its input cancel and
//!     the +-90 degree input rotation, chase / being-chased timers (Chase state, ChasingSprintTimeStart), the Rush perk,
//!     the dodge (bCanDodge pawns) and its supersprint input override. Actor axes come from FRotator::Quaternion
//!     (uequat.rs), not cos / sin of the yaw. Horses in the bubble: their soft bubble. Not ported: the gamepad branch.
//! 16. Climbing (exe_climb.rs; not in the reference): AMordhauCharacter::JumpPressed rva=0x154bff0 tries
//!     TryClimbing rva=0x16a61f0 (a BlueprintImplementableEvent: BP_MordhauCharacter's TryClimbing / AttemptClimb /
//!     FindClimbSpot graph, decoded from the package bytecode) before jumping; LODTick retries it while airborne; the
//!     climb itself is UClimbingMotion (combat side, rust-combat) moving the actor through `climb_step` (OnTick
//!     rva=0x1667790) in MOVE_Custom. The Jump action is now edge-triggered (JumpPressed / JumpReleased -> StopJumping).
//!
//! Frame order (ExeCharacter::frame), as the exe ticks a locally controlled authority pawn:
//!  0. UWorld::Tick advances TimeSeconds by DeltaSeconds before the tick groups run (`world_time`, f32).
//!  1. Player input (APlayerController input processing): Sprint/Crouch Pressed/Released, Jump (ACharacter::Jump),
//!     AMordhauCharacter::MoveForward rva=0x1550170 / AAdvancedCharacter::MoveForward rva=0x148a6c0 and MoveRight
//!     rva=0x148a8d0 -> APawn::AddMovementInput rva=0x32e4390 (actor forward / right vectors x axis value).
//!     The controller ticks first: AController::AddPawnTickDependency rva=0x301d3a0 makes the pawn's movement component
//!     (and, without bTickBeforeOwner, the pawn) depend on the controller's tick (FTickFunction::AddPrerequisite at
//!     0x14301d3eb / 0x14301d450).
//!  2. The movement component, before the actor: UMovementComponent::RegisterComponentTickFunctions rva=0x2fb8b90
//!     (called by UCharacterMovementComponent's at 0x142f83f75) adds the component as a prerequisite of the owner's
//!     PrimaryActorTick when bTickBeforeOwner (+0xe8 bit 2, UMovementComponent ctor 0x142f98379; the UNav / UCMC ctors
//!     leave it set):
//!  3. UAdvancedCharacterMovement::TickComponent rva=0x14a3e80: UMordhauMovementComponent::LODTick rva=0x14c4d60
//!     (sprint state, vcall +0xb20 at 0x1414a41c7), then UCharacterMovementComponent::TickComponent rva=0x2f8ffb0 ->
//!     ControlledCharacterMove rva=0x2f75ee0 (CheckJumpInput, Acceleration = ScaleInputAcceleration(
//!     ConstrainInputAcceleration(input)), ComputeAnalogInputModifier) -> UAdvancedCharacterMovement::PerformMovement
//!     rva=0x1495cc0 (engine PerformMovement, then CheckFallDamage(Velocity.Z)); after it the ragdoll get-up
//!     (0x1414a4365..) and PostCharacterMovementTick (vcall +0xb28 at 0x1414a442f).
//!  4. Actor tick: AAdvancedCharacter::Tick rva=0x14a3d80 -> AMordhauCharacter::LODTick rva=0x154c390: the falling ->
//!     Trip part, the crouch toggle (ACharacter::Crouch / UnCrouch set the movement component's bWantsToCrouch, read by
//!     the next frame's movement), the bWantsClimb retry.
//!  The timer manager then ticks after the tick groups PrePhysics..PostPhysics (UWorld::Tick rva=0x31b0df0: the
//!  FTimerManager::Tick call at 0x1431b1739 follows the RunTickGroup calls for groups 0..4).

use crate::records::CharacterRecords;
use crate::ue::FVector;
use crate::uemath::*;
use crate::world::{HitResult, World};

/// EMovementMode (byte at +0x168)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    None = 0,
    Walking = 1,
    NavWalking = 2,
    Falling = 3,
    /// MOVE_Custom: UClimbingMotion::OnBegin_Implementation rva=0x165e550 sets it (0x14165e82d, mode 6); the climb moves
    /// the actor itself (exe_climb.rs)
    Custom = 6,
}

/// EMovementModifier at +0xd18 (UMordhauMovementComponent::LODTick rva=0x14c4d60 writes 0..7; names are ours)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sprint {
    Forward = 0,
    Sideways = 1,
    Backpedal = 2,
    Partial = 3,
    Sprint = 4,
    Rush = 5,
    Chase = 6,
    Super = 7,
}

pub use crate::restriction;

/// FFindFloorResult (+0x2f0 in the component): flags bBlockingHit (bit 0), bWalkableFloor (bit 1), bLineTrace (bit 2),
/// FloorDist +4, LineDist +8, HitResult +0xc (FindFloor rva=0x2f77af0, ComputeFloorDist rva=0x2f74fe0)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FloorResult {
    pub blocking_hit: bool,
    pub walkable_floor: bool,
    pub line_trace: bool,
    pub floor_dist: f32,
    pub line_dist: f32,
    pub hit: HitResult,
}

impl FloorResult {
    pub fn is_walkable_floor(&self) -> bool {
        self.blocking_hit && self.walkable_floor
    }
    /// FFindFloorResult::Clear (ComputeFloorDist 0x142f7519e..0x142f751d7)
    pub fn clear(&mut self) {
        *self = FloorResult { hit: HitResult::new(1.0), ..Default::default() };
    }
    /// FFindFloorResult::SetFromSweep rva=0x2f895a0
    pub fn set_from_sweep(&mut self, hit: &HitResult, sweep_floor_dist: f32, walkable: bool) {
        self.blocking_hit = hit.is_valid_blocking_hit();
        self.walkable_floor = walkable;
        self.line_trace = false;
        self.floor_dist = sweep_floor_dist;
        self.line_dist = 0.0;
        self.hit = *hit;
    }
    /// FFindFloorResult::SetFromLineTrace rva=0x2f89450: the line hit's normals / component, the sweep's Time,
    /// ImpactPoint, Location, TraceStart / TraceEnd
    pub fn set_from_line_trace(&mut self, line: &HitResult, sweep_floor_dist: f32, line_dist: f32, walkable: bool) {
        if self.hit.blocking_hit && line.blocking_hit {
            let old = self.hit;
            self.hit = *line;
            self.hit.time = old.time;
            self.hit.impact_point = old.impact_point;
            self.hit.location = old.location;
            self.hit.trace_start = old.trace_start;
            self.hit.trace_end = old.trace_end;
            self.line_trace = true;
            self.floor_dist = sweep_floor_dist;
            self.line_dist = line_dist;
            self.walkable_floor = walkable;
        }
    }
    /// FFindFloorResult::GetDistanceToFloor: bLineTrace ? LineDist : FloorDist
    pub fn distance_to_floor(&self) -> f32 {
        if self.line_trace {
            self.line_dist
        } else {
            self.floor_dist
        }
    }
}

/// FStepDownResult (bComputedFloor + FloorResult)
#[derive(Clone, Copy, Debug, Default)]
pub struct StepDownResult {
    pub computed_floor: bool,
    pub floor: FloorResult,
}

/// Engine and native constructor values the component starts with (not in any package): UCharacterMovementComponent
/// ctor rva=0x2f6cc10 immediates and bitfields, ACharacter ctor rva=0x2f27200, AMordhauCharacter ctor rva=0x1524d90,
/// UMordhauMovementComponent ctor rva=0x14af4f0. Values the Blueprint overrides come from the records instead.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineCtor {
    pub max_simulation_time_step: f32,  // +0x29c = 0x3d4ccccd (0x142f6d094)
    pub max_simulation_iterations: i32, // +0x2a0 = 8
    pub max_jump_apex_attempts: i32,    // +0x2a4 = 2 (MaxJumpApexAttemptsPerSimulation)
    pub braking_sub_step_time: f32,     // +0x1b0 = 0x3cf83e10 (0x142f6d1c4)
    pub braking_deceleration_walking: f32, // +0x1b4 = 0x45000000 (2048)
    pub braking_friction: f32,          // +0x1ac (not written: 0)
    pub use_separate_braking_friction: bool, // +0x1f0 bit 0 cleared (0x142f6d342)
    pub apply_gravity_while_jumping: bool,   // +0x1f0 bit 1 set (0x142f6cfea)
    pub maintain_horizontal_ground_velocity: bool, // +0x38b bit 0 set (0x142f6d346)
    pub always_check_floor: bool,       // +0x38d bit 0 set (0x142f6d372)
    pub can_walk_off_ledges: bool,      // +0x1f1 bit 5 set (0x142f6d2a7)
    pub max_depenetration_with_geometry: f32, // +0x2a8 = 0x43fa0000 (500)
    pub jump_max_count: i32,            // ACharacter +0x344 = 1 (0x142f273ef)
    pub jump_max_hold_time: f32,        // ACharacter +0x340 = 0 (0x142f273e9)
    pub capsule_half_height: f32,       // AMordhauCharacter ctor: CapsuleComponent +0x458 = 0x42c00000 (96)
    pub capsule_radius: f32,            // +0x45c = 0x42480000 (50)
    pub strafe_modifier: f32,           // UMordhauMovementComponent +0xcd4 = 0x3f733333 (0.95f)
    pub supersprint_acceleration: f32,  // +0xcf0 = 1330
}

impl Default for EngineCtor {
    fn default() -> Self {
        EngineCtor {
            max_simulation_time_step: f32::from_bits(0x3d4c_cccd),
            max_simulation_iterations: 8,
            max_jump_apex_attempts: 2,
            braking_sub_step_time: f32::from_bits(0x3cf8_3e10),
            braking_deceleration_walking: 2048.0,
            braking_friction: 0.0,
            use_separate_braking_friction: false,
            apply_gravity_while_jumping: true,
            maintain_horizontal_ground_velocity: true,
            always_check_floor: true,
            can_walk_off_ledges: true,
            max_depenetration_with_geometry: 500.0,
            jump_max_count: 1,
            jump_max_hold_time: 0.0,
            capsule_half_height: 96.0,
            capsule_radius: 50.0,
            strafe_modifier: f32::from_bits(0x3f73_3333),
            supersprint_acceleration: 1330.0,
        }
    }
}

/// The records as the exe holds them: every value binary32.
#[derive(Clone, Debug, PartialEq)]
pub struct Cfg {
    pub max_walk_speed: f32,                // +0x18c
    pub max_walk_speed_crouched: f32,       // +0x190
    pub max_speed_falling: f32,             // +0xd2c
    pub max_acceleration: f32,              // +0x1a0
    pub walk_acceleration: f32,             // +0xcd8
    pub partial_sprint_acceleration: f32,   // +0xcdc
    pub sprint_acceleration: f32,           // +0xce0
    pub sprint_modifier: f32,               // +0xcc0
    pub chasing_modifier: f32,              // +0xcc4
    pub partial_sprint_modifier: f32,       // +0xcc8
    pub supersprint_modifier: f32,          // +0xccc
    pub backpedal_modifier: f32,            // +0xcd0
    pub sprint_time_to_reach_max_sprint: f32, // +0xce4
    pub braking_deceleration_falling: f32,  // +0x1b8
    pub braking_deceleration_falling_too_fast: f32, // +0xd3c
    pub braking_friction_factor: f32,       // +0x1a8
    pub ground_friction: f32,               // +0x16c
    pub falling_lateral_friction: f32,      // +0x1d0
    pub air_control: f32,                   // +0x1c4
    pub air_control_boost_multiplier: f32,  // +0x1c8
    pub air_control_boost_velocity_threshold: f32, // +0x1cc
    pub jump_z_velocity: f32,               // +0x158
    pub gravity_scale: f32,                 // +0x150
    pub crouched_half_height: f32,          // +0x1d4
    pub walkable_floor_z: f32,              // +0x164
    pub max_step_height: f32,               // +0x154
    pub perch_radius_threshold: f32,        // +0x1dc
    pub perch_additional_height: f32,       // +0x1e0
    pub can_walk_off_ledges_when_crouching: bool, // +0x1f1 bit 6
    pub min_velocity_for_fall_damage: f32,  // +0xb98
    pub fall_damage_offset: f32,
    pub fall_damage_factor: f32,
    pub ragdoll_min_velocity_for_fall_damage: f32,
    pub ragdoll_fall_damage_offset: f32,
    pub ragdoll_fall_damage_factor: f32,
    pub knockback_ground_friction: f32,     // +0xd20
    pub knockback_falling_lateral_friction: f32, // +0xd24
    pub knockback_up_impulse: f32,          // +0xd28
    pub knockback_duration: f32,            // +0xd38
    pub can_crouch: bool,                   // NavAgentProps.bCanCrouch (+0xf0 bit 0)
    pub jump_cooldown: f32,                 // AAdvancedCharacter +0x864
    pub falling_time_to_ragdoll: f32,       // AMordhauCharacter +0xe9c
    pub crouch_cooldown: f32,               // +0xde8
    pub dodge_duration: f32,                // +0xea4
    pub ragdoll_falling_get_up_duration: f32,   // AAdvancedCharacter +0x840
    pub ragdoll_falling_min_time: f32,          // +0x84c
    pub ragdoll_falling_min_velocity_to_get_up: f32, // +0x850
    pub ragdoll_falling_time_at_min_velocity_to_get_up: f32, // +0x854
    pub disable_ragdoll_falling: bool,      // +0x7f8
    pub jump_stamina_cost: f32,             // +0xe7c
    // LODTick (UMordhauMovementComponent ctor rva=0x14af4f0 + CharMoveComp; AMordhauCharacter ctor + CDO)
    pub turn_max_accumulated_angle: f32,    // +0xc70
    /// FC_TurnSprintPreventionDecay / Slowdown keys (time, value, linear?)
    pub turn_decay_curve: Option<Vec<(f32, f32, bool)>>,
    pub turn_slowdown_curve: Option<Vec<(f32, f32, bool)>>,
    pub rush_sprint_time_start: f32,        // +0xc74
    pub chasing_sprint_time_start: f32,     // +0xc78
    pub max_angle_to_chase: f32,            // +0xc80
    pub max_angle_to_stop_chasing: f32,     // +0xc84
    pub chasing_max_distance: FVector,      // +0xc88
    pub stop_chasing_max_distance: FVector, // +0xc94
    pub time_to_break_us_chasing: f32,      // +0xcac
    pub time_to_break_us_being_chased: f32, // +0xcb0
    pub min_time_to_start_chasing: f32,     // +0xcb4
    pub min_time_to_start_being_chased: f32, // +0xcb8
    pub spawn_max_sprint_duration: f32,     // +0xc4c
    pub dodge_cooldown: f32,                // +0xea8
    pub dodge_stamina_cost: i64,            // +0xeac
    pub can_dodge: bool,                    // +0xe66
    pub ellipse_bubble_length: f32,         // +0xf48
    pub ellipse_bubble_radius: f32,         // +0xf4c
    pub ellipse_bubble_max_height_diff: f32, // +0xf50
    pub gravity_z: f32,                     // world DefaultGravityZ
    pub terminal_velocity: f32,             // physics volume TerminalVelocity (+0x258 of APhysicsVolume)
}

/// a curve package's keys as binary32 (None when the record names no curve: the exe then uses 1.0)
fn curve(r: &CharacterRecords, path: &str) -> Option<Vec<(f32, f32, bool)>> {
    if path.is_empty() {
        return None;
    }
    r.curves.get(path).map(|c| c.keys.iter().map(|(t, v, m)| (*t as f32, *v as f32, m != "RCIM_Constant")).collect())
}

impl Cfg {
    /// round every record value to binary32 (the records hold the f64 of each value's shortest decimal, which rounds
    /// back to the exe's exact float)
    pub fn from_records(r: &CharacterRecords) -> Self {
        let (m, x, c) = (&r.movement, &r.move_extra, &r.character);
        let f = |v: f64| v as f32;
        Cfg {
            max_walk_speed: f(m.max_walk_speed),
            max_walk_speed_crouched: f(m.max_walk_speed_crouched),
            max_speed_falling: f(m.max_speed_falling),
            max_acceleration: f(m.max_acceleration),
            walk_acceleration: f(m.walk_acceleration),
            partial_sprint_acceleration: f(m.partial_sprint_acceleration),
            sprint_acceleration: f(m.sprint_acceleration),
            sprint_modifier: f(m.sprint_modifier),
            chasing_modifier: f(m.chasing_modifier),
            partial_sprint_modifier: f(m.partial_sprint_modifier),
            supersprint_modifier: f(m.supersprint_modifier),
            backpedal_modifier: f(m.backpedal_modifier),
            sprint_time_to_reach_max_sprint: f(m.sprint_time_to_reach_max_sprint),
            braking_deceleration_falling: f(m.braking_deceleration_falling),
            braking_deceleration_falling_too_fast: f(m.braking_deceleration_falling_too_fast),
            braking_friction_factor: f(m.braking_friction_factor),
            ground_friction: f(m.ground_friction),
            falling_lateral_friction: f(m.falling_lateral_friction),
            air_control: f(m.air_control),
            air_control_boost_multiplier: f(m.air_control_boost_multiplier),
            air_control_boost_velocity_threshold: f(m.air_control_boost_velocity_threshold),
            jump_z_velocity: f(m.jump_z_velocity),
            gravity_scale: f(m.gravity_scale),
            crouched_half_height: f(m.crouched_half_height),
            walkable_floor_z: f(m.walkable_floor_z),
            max_step_height: f(m.max_step_height),
            perch_radius_threshold: f(m.perch_radius_threshold),
            perch_additional_height: f(m.perch_additional_height),
            can_walk_off_ledges_when_crouching: m.b_can_walk_off_ledges_when_crouching,
            min_velocity_for_fall_damage: f(x.min_velocity_for_fall_damage),
            fall_damage_offset: f(x.fall_damage_offset),
            fall_damage_factor: f(x.fall_damage_factor),
            ragdoll_min_velocity_for_fall_damage: f(x.ragdoll_min_velocity_for_fall_damage),
            ragdoll_fall_damage_offset: f(x.ragdoll_fall_damage_offset),
            ragdoll_fall_damage_factor: f(x.ragdoll_fall_damage_factor),
            knockback_ground_friction: f(x.knockback_ground_friction),
            knockback_falling_lateral_friction: f(x.knockback_falling_lateral_friction),
            knockback_up_impulse: f(x.knockback_up_impulse),
            knockback_duration: f(x.knockback_duration),
            can_crouch: x.b_can_crouch,
            jump_cooldown: f(c.jump_cooldown),
            falling_time_to_ragdoll: f(c.falling_time_to_ragdoll),
            crouch_cooldown: f(c.crouch_cooldown),
            dodge_duration: f(c.dodge_duration),
            ragdoll_falling_get_up_duration: f(c.ragdoll_falling_get_up_duration),
            ragdoll_falling_min_time: f(c.ragdoll_falling_min_time),
            ragdoll_falling_min_velocity_to_get_up: f(c.ragdoll_falling_min_velocity_to_get_up),
            ragdoll_falling_time_at_min_velocity_to_get_up: f(c.ragdoll_falling_time_at_min_velocity_to_get_up),
            disable_ragdoll_falling: c.b_disable_ragdoll_falling,
            jump_stamina_cost: f(c.jump_stamina_cost),
            turn_max_accumulated_angle: f(x.turn_sprint_prevention_max_accumulated_angle),
            turn_decay_curve: curve(r, &x.turn_sprint_prevention_decay_curve),
            turn_slowdown_curve: curve(r, &x.turn_sprint_prevention_slowdown_curve),
            rush_sprint_time_start: f(x.rush_sprint_time_start),
            chasing_sprint_time_start: f(x.chasing_sprint_time_start),
            max_angle_to_chase: f(x.max_angle_to_chase),
            max_angle_to_stop_chasing: f(x.max_angle_to_stop_chasing),
            chasing_max_distance: v(f(x.chasing_max_distance[0]), f(x.chasing_max_distance[1]), f(x.chasing_max_distance[2])),
            stop_chasing_max_distance: v(f(x.stop_chasing_max_distance[0]), f(x.stop_chasing_max_distance[1]), f(x.stop_chasing_max_distance[2])),
            time_to_break_us_chasing: f(x.time_to_break_us_chasing),
            time_to_break_us_being_chased: f(x.time_to_break_us_being_chased),
            min_time_to_start_chasing: f(x.min_time_to_start_chasing),
            min_time_to_start_being_chased: f(x.min_time_to_start_being_chased),
            spawn_max_sprint_duration: f(x.spawn_max_sprint_duration),
            dodge_cooldown: f(c.dodge_cooldown),
            dodge_stamina_cost: c.dodge_stamina_cost,
            can_dodge: c.b_can_dodge,
            ellipse_bubble_length: f(c.ellipse_bubble_length),
            ellipse_bubble_radius: f(c.ellipse_bubble_radius),
            ellipse_bubble_max_height_diff: f(c.ellipse_bubble_max_height_diff),
            gravity_z: f(r.physics.gravity_z),
            terminal_velocity: f(r.physics.terminal_velocity),
        }
    }
}

// .rdata literals the Mordhau rules load (addresses as the reference cites them, values read at those addresses)
pub const RDATA_FORWARD_CONE_DEG: f32 = 56.25; // DAT_14433116c
pub const RDATA_BACKPEDAL_DEG: f32 = 101.25; // DAT_144331174
pub const RDATA_SPRINT_SPEED_SLACK: f32 = 5.0; // DAT_143fe4e20
pub const RDATA_FALL_TOO_FAST: f32 = 1.01; // DAT_1442efa68 (f32 1.00999999)
/// 180 / PI as the f32 the LODTick sprint angle multiplies by (.rdata 0x1442713d0 = 57.2957764)
pub const RAD_TO_DEG: f32 = 57.295_776;

/// One pawn: the movement component, its capsule, and the AAdvancedCharacter / AMordhauCharacter fields the
/// movement rules read and write.
#[derive(Clone, Debug)]
pub struct ExeMovement {
    pub e: EngineCtor,
    pub c: Cfg,

    // ---- UCharacterMovementComponent ----
    pub mode: Mode,                   // +0x168
    pub location: FVector,            // UpdatedComponent location (capsule centre)
    pub velocity: FVector,            // +0xc4
    pub acceleration: FVector,        // +0x22c
    pub analog_input_modifier: f32,   // +0x28c
    pub pending_impulse: FVector,     // +0x274
    pub pending_force: FVector,       // +0x280
    pub current_floor: FloorResult,   // +0x2f0
    pub base: Option<u32>,            // MovementBase (component id)
    pub just_teleported: bool,        // +0x38b bit 5, set by the ctor (0x142f6d206)
    pub force_next_floor_check: bool, // +0x1f1 bit 3, set by the ctor (0x142f6d29f)
    pub crouch_maintains_base_location: bool, // +0x38c bit 5
    pub wants_to_crouch: bool,        // +0x38c bit 4 (CMC bWantsToCrouch)
    pub num_jump_apex_attempts: i32,  // +0x298
    pub last_update_location: FVector,
    pub random: RandomStream,
    pub capsule_half_height: f32,     // current unscaled half height (+0x458 of the capsule)
    pub control_input_vector: FVector, // APawn +0x264 ControlInputVector
    // ---- ACharacter ----
    pub is_crouched: bool,            // +0x330 bit 0
    pub pressed_jump: bool,           // +0x330 bit 2
    pub was_jumping: bool,            // +0x330 bit 10
    pub jump_key_hold_time: f32,      // +0x334
    pub jump_force_time_remaining: f32, // +0x338
    pub jump_current_count: i32,      // +0x348
    // ---- UMordhauMovementComponent ----
    pub sprint_state: Sprint,         // +0xd18
    pub wants_sprint: bool,           // +0xd19
    pub sprint_allowed: bool,         // +0xd1a
    pub only_partial_sprint: bool,    // +0xd1b
    pub wants_supersprint: bool,      // +0xd1c
    pub sprint_time: f32,             // +0xce8
    pub knockback_time: f32,          // +0xd4c
    pub base_ground_friction: f32,    // +0xd30
    pub base_falling_lateral_friction: f32, // +0xd34
    pub equip_speed_cap: f32,         // +0xd00
    pub armor_speed: f32,             // +0xd04 (ctor 1.0)
    pub armor_accel: f32,             // +0xd08 (ctor 1.0)
    pub equip_speed_add: f32,         // +0xd0c
    pub equip_sprint_penalty: f32,    // +0xd10
    pub equip_accel_add: f32,         // +0xd14
    pub motion_speed_factor: f32,     // +0xcf4 (ctor 1.0)
    pub motion_backpedal_speed_factor: f32, // +0xcf8 (ctor 1.0)
    pub equipment_backpedal_speed_factor: f32, // +0xcfc (ctor 1.0)
    /// What OnCharacterLODTick reads each actor tick (host / combat side): the current motion's (SpeedFactor +0x64,
    /// BackpedalSpeedFactor +0x68) when MotionSystemComponent->CurrentMotion is set (a ranged draw: the equipment's
    /// RangedDrawSpeedFactor, URangedDrawMotion::OnBegin_Implementation rva=0x1661e50), and the held equipment's
    /// BackpedalSpeedFactorEquipped (+0x704) of the left / right hand
    pub current_motion_factors: Option<(f32, f32)>,
    pub left_equipment_backpedal: Option<f32>,
    pub right_equipment_backpedal: Option<f32>,
    /// UPerkSystemComponent bIsDwarf (+0xcc) and DwarfSpeedModifier (+0x12c), read by GetMaxSpeed 0x1414be3a4 and
    /// GetSpeedFactor 0x1414bfa27 (None = no perk component / not a dwarf)
    pub dwarf_speed_modifier: Option<f32>,
    /// bots: UMordhauMovementComponent::LODTick takes the velocity direction instead of the pending input when the
    /// controller is an AAIController (0x1414c6efa / 0x1414c6fae)
    pub ai_controlled: bool,
    // ---- LODTick state and inputs (exe_lod.rs) ----
    /// actor rotation yaw (deg) and its quaternion (FRotator(0, Yaw, 0).Quaternion(), set each frame from the input)
    pub yaw: f32,
    pub actor_quat: crate::uequat::Quat,
    /// AActor::CreationTime (+0x9c): turn-sprint prevention starts 5 s after it
    pub creation_time: f32,
    pub turn_angle_counter: f32,      // +0xc54 TurnSprintPreventionAngleCounter
    pub turn_last_angle: f32,         // +0xc58 TurnSprintPreventionLastAngle
    pub is_chasing: bool,             // +0xc48
    pub is_being_chased: bool,        // +0xc49
    pub total_chased_time: f32,       // AMordhauCharacter +0xed4 TotalChasedTime (others write it)
    pub last_chased_time: f32,        // +0xed8 LastChasedTime
    /// mesh component location (chase angle), first-person camera location / rotation (chase box): host-provided,
    /// None = the capsule location / the actor rotation
    pub mesh_location: Option<FVector>,
    pub camera_location_1p: Option<FVector>,
    pub camera_rotation_1p: Option<(f32, f32, f32)>,
    /// the other pawns of the world, in TActorIterator order (host fills them each frame; chase timers written back)
    pub others: Vec<crate::exe_lod::OtherPawn>,
    /// gates the host owns: current motion's bDisablesChaseMechanic / bDisablesDodge, held equipment's
    /// bCannotChaseOthers / bDisablesDodge, standing in a smoke field
    pub motion_disables_chase: bool,
    pub motion_disables_dodge: bool,
    pub equipment_cannot_chase: bool,
    pub equipment_disables_dodge: bool,
    pub in_smoke_field: bool,
    /// Rush perk (HasPerk 0xe) with RushMovementBoostDuration and LastEnemyKilledTimeWithMeleeOrRanged
    pub rush_perk: Option<(f32, f32)>,
    /// stamina (combat side) for the dodge cost test
    pub stamina: i64,
    pub was_dodge_canceled: bool,     // +0xeb0
    pub dodge_direction: FVector,     // +0xeb4
    pub dodge_direction_local: FVector, // +0xec0
    /// ServerRequestDodge(byte) requests for the combat side, drained by the owner
    pub dodges: Vec<u8>,
    /// Some for a horse (exe_horse.rs): the UHorseMovementComponent / AHorse state and the class overrides
    pub horse: Option<Box<crate::exe_horse::HorseState>>,
    /// Some for a ladder mover (exe_ladder.rs): UOneDimensionalMovementComponent's state and overrides
    pub ladder: Option<Box<crate::exe_ladder::LadderState>>,
    /// Some for a UPseudoVehicleMovementComponent vehicle other than the horse (exe_pseudo.rs: BP_Catapult)
    pub pseudo: Option<Box<crate::exe_pseudo::PseudoState>>,
    // ---- climbing (exe_climb.rs) ----
    pub jump_held: bool,
    pub wants_climb: bool,            // AMordhauCharacter +0xb8c bWantsClimb
    pub allow_climbing: bool,         // +0xb8f bAllowClimbing (ctor 0x1415262bc: 1; BP_MordhauCharacter keeps it)
    pub climb_target_location: FVector, // +0xdf8 ClimbTargetLocation
    /// RequestClimb(ClimbTargetLocation, bIsSlowClimb) for the combat side's UClimbingMotion, drained by the owner
    pub climb_request: Option<(FVector, bool)>,
    /// AttemptClimb's gates the host owns: right-hand equipment bPreventsClimbing; the current motion is a climb, a
    /// parry, or an attack not in Stage 2
    pub equipment_prevents_climbing: bool,
    pub motion_blocks_climb: bool,
    /// hit components whose actor refuses climbing (FindClimbSpot 0x1029..: EnvironmentMovable, BP_BaseProgressActor,
    /// AMordhauActor bPreventClimbing; the hit component's attach parent's owner when it has one)
    pub climb_blocked_components: Vec<u32>,
    // ---- AAdvancedCharacter / AMordhauCharacter ----
    pub world_time: f32,              // UWorld TimeSeconds (+0x598)
    pub last_land: f32,               // +0x85c
    pub airborne_from_jump: bool,     // +0x869
    pub last_land_from_jump: bool,    // +0x860
    pub authority: bool,
    pub player_controlled: bool,
    pub dead: bool,                   // +0x504
    pub ignore_movement_input: bool,  // UAdvancedCharacterMovement +0xbc4
    pub ragdoll_falling: bool,        // +0x505
    pub ragdoll_falling_start_time: f32,     // +0x848
    pub ragdoll_falling_get_up_start_time: f32, // +0x844 (zero-filled; see is_ragdoll_falling_or_getting_up)
    pub still_time_while_ragdoll_falling: f32,  // +0xb90
    pub last_falling_check_velocity_z: f32,     // +0xb58
    pub motion_restriction: i64,
    pub falling_time: f32,            // +0xe98
    pub wants_crouch: bool,           // +0xde4 (Mordhau bWantsCrouch)
    pub last_crouch_toggle_time: f32, // +0xde0
    pub last_dodge_time: f32,         // +0xea0 (zero-filled)
    /// fp-anim r4: the current UAttackMotion's (WindupEnd +0x1090, ReleaseJumpBlockTime +0xbb8) as the owner (Sim)
    /// copies it each frame (None without an attack motion), for AMordhauCharacter::CanJumpInternal_Implementation
    pub attack_jump_block: Option<(f32, f32)>,
    pub toggle_sprint: i32,           // m.ToggleSprint
    pub toggle_crouch: i32,           // m.ToggleCrouch
    pub sprint_held: bool,
    pub crouch_held: bool,
    // ---- events for the owner (combat side), drained each frame ----
    pub fall_damage: Vec<f32>,
    pub jumps: i64,
    pub trips: i64,
    pub ragdoll_changes: Vec<bool>,
    pub landings: i64,
}

/// One frame of player input
#[derive(Clone, Copy, Debug, Default)]
pub struct ExeInput {
    pub fwd: f32,
    pub right: f32,
    pub jump: bool,
    pub sprint: bool,
    pub crouch: bool,
    /// actor rotation yaw in degrees (the control yaw the player controller applied this frame; actor rotation =
    /// FRotator(0, Yaw, 0)); forward / right come from its quaternion (uequat::actor_axes)
    pub yaw: f32,
}

impl ExeMovement {
    pub fn new(r: &CharacterRecords, location: FVector) -> Self {
        let e = EngineCtor::default();
        let c = Cfg::from_records(r);
        let hh = e.capsule_half_height;
        let mut m = ExeMovement {
            mode: Mode::Walking,
            location,
            velocity: FVector::ZERO,
            acceleration: FVector::ZERO,
            analog_input_modifier: 0.0,
            pending_impulse: FVector::ZERO,
            pending_force: FVector::ZERO,
            current_floor: FloorResult { hit: HitResult::new(1.0), ..Default::default() },
            base: None,
            just_teleported: true,
            force_next_floor_check: true,
            crouch_maintains_base_location: false,
            wants_to_crouch: false,
            num_jump_apex_attempts: 0,
            last_update_location: location,
            random: RandomStream::default(),
            capsule_half_height: hh,
            control_input_vector: FVector::ZERO,
            is_crouched: false,
            pressed_jump: false,
            was_jumping: false,
            jump_key_hold_time: 0.0,
            jump_force_time_remaining: 0.0,
            jump_current_count: 0,
            sprint_state: Sprint::Forward,
            wants_sprint: false,
            sprint_allowed: false,
            only_partial_sprint: false,
            wants_supersprint: false,
            sprint_time: 0.0,
            knockback_time: 0.0,
            base_ground_friction: 0.0,
            base_falling_lateral_friction: 0.0,
            equip_speed_cap: 0.0,
            armor_speed: 1.0,
            armor_accel: 1.0,
            equip_speed_add: 0.0,
            equip_sprint_penalty: 0.0,
            equip_accel_add: 0.0,
            motion_speed_factor: 1.0,
            motion_backpedal_speed_factor: 1.0,
            equipment_backpedal_speed_factor: 1.0,
            current_motion_factors: None,
            left_equipment_backpedal: None,
            right_equipment_backpedal: None,
            dwarf_speed_modifier: None,
            ai_controlled: false,
            yaw: 0.0,
            actor_quat: [0.0, 0.0, 0.0, 1.0],
            creation_time: 0.0,
            turn_angle_counter: 0.0,
            turn_last_angle: 0.0,
            is_chasing: false,
            is_being_chased: false,
            total_chased_time: 0.0,
            last_chased_time: 0.0,
            mesh_location: None,
            camera_location_1p: None,
            camera_rotation_1p: None,
            others: Vec::new(),
            motion_disables_chase: false,
            motion_disables_dodge: false,
            equipment_cannot_chase: false,
            equipment_disables_dodge: false,
            in_smoke_field: false,
            rush_perk: None,
            stamina: 100,
            was_dodge_canceled: false,
            dodge_direction: FVector::ZERO,
            dodge_direction_local: FVector::ZERO,
            dodges: Vec::new(),
            horse: None,
            ladder: None,
            pseudo: None,
            jump_held: false,
            wants_climb: false,
            allow_climbing: true,
            climb_target_location: FVector::ZERO,
            climb_request: None,
            equipment_prevents_climbing: false,
            motion_blocks_climb: false,
            climb_blocked_components: Vec::new(),
            world_time: 0.0,
            last_land: 0.0,
            airborne_from_jump: false,
            last_land_from_jump: false,
            authority: true,
            player_controlled: true,
            dead: false,
            ignore_movement_input: false,
            ragdoll_falling: false,
            ragdoll_falling_start_time: 0.0,
            ragdoll_falling_get_up_start_time: 0.0,
            still_time_while_ragdoll_falling: 0.0,
            last_falling_check_velocity_z: 0.0,
            motion_restriction: 0,
            falling_time: 0.0,
            wants_crouch: false,
            last_crouch_toggle_time: 0.0,
            last_dodge_time: 0.0,
            attack_jump_block: None,
            toggle_sprint: 0,
            toggle_crouch: 0,
            sprint_held: false,
            crouch_held: false,
            fall_damage: Vec::new(),
            jumps: 0,
            trips: 0,
            ragdoll_changes: Vec::new(),
            landings: 0,
            e,
            c,
        };
        m.initialize_component();
        m
    }

    /// UMordhauMovementComponent::InitializeComponent rva=0x14c1300 (BaseGroundFriction / BaseFallingLateralFriction)
    pub fn initialize_component(&mut self) {
        self.base_ground_friction = self.c.ground_friction;
        self.base_falling_lateral_friction = self.c.falling_lateral_friction;
    }

    pub fn capsule_radius(&self) -> f32 {
        self.e.capsule_radius
    }

    // ============================================================================================================
    // one frame
    // ============================================================================================================

    /// One frame of a locally controlled authority pawn (module doc: frame order).
    pub fn frame(&mut self, world: &dyn World, dt: f32, inp: &ExeInput) {
        // 0. UWorld::Tick: TimeSeconds += DeltaSeconds before the tick groups
        self.world_time += dt;
        // 1. input
        if inp.sprint != self.sprint_held {
            if inp.sprint {
                self.sprint_pressed();
            } else {
                self.sprint_released();
            }
            self.sprint_held = inp.sprint;
        }
        if inp.crouch != self.crouch_held {
            if inp.crouch {
                self.crouch_pressed();
            } else {
                self.crouch_released();
            }
            self.crouch_held = inp.crouch;
        }
        // JumpPressed (TryClimbing's forward traces) sees the actor rotation the pawn had before this frame's yaw
        self.jump_input(world, inp.jump);
        self.set_yaw(inp.yaw);
        let (_, fwd_axis, right_axis) = crate::uequat::actor_axes(inp.yaw);
        self.move_forward(inp.fwd, fwd_axis);
        self.move_right(inp.right, right_axis);
        // 2. movement component tick (before the actor: bTickBeforeOwner, module docs)
        self.lod_tick(dt);
        let input = self.consume_input_vector();
        self.controlled_character_move(world, input, dt);
        self.tick_ragdoll_get_up(dt);
        self.post_character_movement_tick(dt);
        // 3. actor tick (AAdvancedCharacter::LODTick: the OnLODTick broadcast first, which runs OnCharacterLODTick)
        self.on_character_lod_tick();
        self.character_lod_tick(dt);
        self.climb_retry(world);
    }

    // ---- input (AMordhauCharacter) ------------------------------------------------------------------------------

    /// AMordhauCharacter::SprintPressed rva=0x156da50 (keyboard): m.ToggleSprint != 0 and bWantsToSprint -> false, else true
    pub fn sprint_pressed(&mut self) {
        self.wants_sprint = !(self.toggle_sprint != 0 && self.wants_sprint);
    }
    /// AMordhauCharacter::SprintReleased rva=0x156dba0: m.ToggleSprint == 0 -> bWantsToSprint = false
    pub fn sprint_released(&mut self) {
        if self.toggle_sprint == 0 {
            self.wants_sprint = false;
        }
    }
    /// AMordhauCharacter::CrouchPressed rva=0x1538040: m.ToggleCrouch == 1 -> toggle, else true
    pub fn crouch_pressed(&mut self) {
        self.wants_crouch = if self.toggle_crouch == 1 { !self.wants_crouch } else { true };
    }
    /// AMordhauCharacter::CrouchReleased rva=0x1538100: m.ToggleCrouch != 1 -> false
    pub fn crouch_released(&mut self) {
        if self.toggle_crouch != 1 {
            self.wants_crouch = false;
        }
    }
    /// ACharacter::Jump rva=0x2f3c410: bPressedJump = true, JumpKeyHoldTime = 0
    pub fn jump(&mut self) {
        self.pressed_jump = true;
        self.jump_key_hold_time = 0.0;
    }

    /// AMordhauCharacter::MoveForward rva=0x1550170: Value == 0 on keyboard with m.ToggleSprint == 1 -> StopSprinting
    /// (0x1415501e2); then AAdvancedCharacter::MoveForward rva=0x148a6c0: Value != 0 -> AddMovementInput(
    /// GetActorForwardVector() * Value) (0x14148a867..0x14148a8a6; the +0x899 move-allowed flag is taken as set)
    pub fn move_forward(&mut self, value: f32, actor_forward: FVector) {
        if value == 0.0 && self.toggle_sprint == 1 {
            self.wants_sprint = false; // AMordhauCharacter::StopSprinting rva=0x156deb0
        }
        if value != 0.0 {
            self.add_movement_input(mul(actor_forward, value), 1.0, false);
        }
    }
    /// AAdvancedCharacter::MoveRight rva=0x148a8d0: AddMovementInput(GetActorRightVector() * Value) (no zero check
    /// before the add: 0x14148a976..0x14148a9f3; the +0x89a flag taken as set)
    pub fn move_right(&mut self, value: f32, actor_right: FVector) {
        self.add_movement_input(mul(actor_right, value), 1.0, false);
    }

    /// APawn::AddMovementInput rva=0x32e4390 -> UPawnMovementComponent::AddInputVector rva=0x2f9acc0 ->
    /// APawn::Internal_AddMovementInput rva=0x32eed40: bForce or !IsMoveInputIgnored (vcall +0x758) ->
    /// ControlInputVector += WorldDirection * ScaleValue
    pub fn add_movement_input(&mut self, dir: FVector, scale: f32, force: bool) {
        let a = mul(dir, scale);
        if force || !self.is_move_input_ignored() {
            self.control_input_vector = add(self.control_input_vector, a);
        }
    }

    /// APawn::Internal_ConsumeMovementInputVector rva=0x32eedb0: return it and zero it
    pub fn consume_input_vector(&mut self) -> FVector {
        let v = self.control_input_vector;
        self.control_input_vector = FVector::ZERO;
        v
    }

    // ---- AMordhauCharacter::LODTick ------------------------------------------------------------------------------

    /// UMordhauMovementComponent::OnCharacterLODTick rva=0x14c9ec0 (bound to the owner's OnLODTick; read off the
    /// disassembly): alive (+0x504) -> MotionSpeedFactor / MotionBackpedalSpeedFactor = the current motion's
    /// SpeedFactor / BackpedalSpeedFactor (only when there is one: the values persist otherwise); with an equipment
    /// system, EquipmentBackpedalSpeedFactor = 1, then the left hand's BackpedalSpeedFactorEquipped (+0x1200) or else the
    /// right hand's (+0x11f8)
    pub fn on_character_lod_tick(&mut self) {
        if self.dead {
            return;
        }
        if let Some((sf, bf)) = self.current_motion_factors {
            self.motion_speed_factor = sf;
            self.motion_backpedal_speed_factor = bf;
        }
        self.equipment_backpedal_speed_factor = 1.0;
        if let Some(b) = self.left_equipment_backpedal.or(self.right_equipment_backpedal) {
            self.equipment_backpedal_speed_factor = b;
        }
    }

    /// AMordhauCharacter::LODTick rva=0x154c390, the parts ported: falling too long -> Trip (authority,
    /// IsPlayerControlled vcall +0x6c0, alive, Velocity.Z < -1, IsAirborne vcall +0x9e0, not ragdoll falling ->
    /// FallingTime += dt; above FallingTimeToRagdoll -> Trip vcall +0x8f0 and 0; else 0), then the crouch toggle from
    /// 0x14154c4cf: bIsCrouched (ACharacter +0x330 bit 0) != bWantsCrouch and CrouchCooldown + LastCrouchToggleTime <
    /// TimeSeconds -> LastCrouchToggleTime = TimeSeconds, ACharacter::Crouch / UnCrouch(false).
    /// Not ported: scream, the rest of the LOD work. The bWantsClimb retry (0x14154c556) is `climb_retry` (exe_climb.rs),
    /// called right after this.
    pub fn character_lod_tick(&mut self, dt: f32) {
        if self.authority {
            if self.player_controlled && !self.dead && self.velocity.z < -1.0 && self.is_airborne() && !self.ragdoll_falling {
                self.falling_time = dt + self.falling_time;
                if !(self.falling_time <= self.c.falling_time_to_ragdoll) {
                    self.trip();
                    self.falling_time = 0.0;
                }
            } else {
                self.falling_time = 0.0;
            }
        }
        if !self.dead && self.is_crouched != self.wants_crouch && self.c.crouch_cooldown + self.last_crouch_toggle_time < self.world_time {
            self.last_crouch_toggle_time = self.world_time;
            if self.wants_crouch {
                // ACharacter::Crouch rva=0x2f31990: CanCrouch() -> CharacterMovement->bWantsToCrouch = true
                if self.can_crouch() {
                    self.wants_to_crouch = true;
                }
            } else {
                // ACharacter::UnCrouch rva=0x2f45b80: CharacterMovement->bWantsToCrouch = false
                self.wants_to_crouch = false;
            }
        }
    }

    /// AMordhauCharacter::CanCrouch rva=0x1532230 -> AAdvancedCharacter::CanCrouch rva=0x1459e10 -> ACharacter::CanCrouch:
    /// not crouched, not NoMovement, not ragdoll falling / getting up, NavAgentProps.bCanCrouch
    pub fn can_crouch(&self) -> bool {
        if !self.is_crouched {
            if self.get_movement_restriction() == restriction::NO_MOVEMENT || self.is_ragdoll_falling_or_getting_up() {
                return false;
            }
        }
        !self.is_crouched && self.c.can_crouch
    }

    /// AMordhauCharacter::IsAirborne rva=0x154a7f0 (no vehicle): IsFalling
    /// EVD_CAM_021: the crouch's HalfHeightAdjust = the default (standing) half height - the current one (0 standing;
    /// UCharacterMovementComponent::Crouch / UnCrouch pass it to AMordhauCharacter::OnStartCrouch rva=0x155bc20 /
    /// OnEndCrouch rva=0x1552140, which set the mesh's RelativeLocation.Z = the class default's Z + HalfHeightAdjust
    /// (+ DwarfMeshZOffsetCrouched for a dwarf; decomp AMordhauCharacter.cpp 9085-9133 / 9381-9427), so the feet stay on
    /// the floor while the capsule shrinks
    pub fn half_height_adjust(&self) -> f32 {
        self.e.capsule_half_height - self.capsule_half_height
    }

    pub fn is_airborne(&self) -> bool {
        self.mode == Mode::Falling
    }

    // ---- UMordhauMovementComponent speed rules ------------------------------------------------------------------

    /// UMordhauMovementComponent::GetSpeedFactor rva=0x14bf9a0: ((+0xd0c + +0xd04) + (1 - t) * +0xd10), min with
    /// +0xd00 when that is > 0, x DwarfSpeedModifier when bIsDwarf, x MotionSpeedFactor (+0xcf4), max 0
    pub fn get_speed_factor(&self, t: f32) -> f32 {
        let mut s = self.equip_speed_add + self.armor_speed;
        s = s + (1.0 - t) * self.equip_sprint_penalty;
        if self.equip_speed_cap > 0.0 {
            s = minss(self.equip_speed_cap, s);
        }
        if let Some(d) = self.dwarf_speed_modifier {
            s = s * d;
        }
        s = s * self.motion_speed_factor;
        maxss(s, 0.0)
    }

    /// UMordhauMovementComponent::GetMaxSpeed rva=0x14be1d0 (module-level notes on each branch in the disassembly)
    pub fn get_max_speed(&self) -> f32 {
        if self.horse.is_some() || self.pseudo.is_some() {
            // UAdvancedCharacterMovement::GetMaxSpeed rva=0x1484b20 (= UCharacterMovementComponent::GetMaxSpeed)
            return self.engine_max_speed();
        }
        if self.ladder.is_some() {
            return self.ladder_max_speed();
        }
        let c = &self.c;
        let mut m = 1.0f32;
        let mut apply = true;
        let st = self.sprint_state;
        let factor = if matches!(self.mode, Mode::Walking | Mode::NavWalking | Mode::Falling) {
            if st == Sprint::Backpedal {
                m = self.motion_backpedal_speed_factor * c.backpedal_modifier * self.equipment_backpedal_speed_factor;
                true
            } else if st == Sprint::Sideways {
                m = self.e.strafe_modifier;
                true
            } else if self.is_crouched && self.mode != Mode::Falling {
                true
            } else if st == Sprint::Super {
                if self.mode == Mode::Falling {
                    m = c.sprint_modifier;
                    true
                } else {
                    m = c.supersprint_modifier;
                    apply = false;
                    false
                }
            } else if matches!(st, Sprint::Sprint | Sprint::Rush | Sprint::Chase) {
                m = c.sprint_modifier;
                let mut tt = self.sprint_time;
                if st == Sprint::Chase {
                    m = c.chasing_modifier;
                    apply = false;
                    tt = tt + c.chasing_sprint_time_start; // + ChasingSprintTimeStart (+0xc78)
                }
                let reach = c.sprint_time_to_reach_max_sprint;
                if reach.abs() > SMALL_NUMBER {
                    let mut x = tt / reach;
                    x = if x >= 0.0 { minss(x, 1.0) } else { 0.0 };
                    let p = c.partial_sprint_modifier;
                    m = (m - p) * x + p;
                }
                apply
            } else {
                if st == Sprint::Partial {
                    m = c.partial_sprint_modifier;
                }
                true
            }
        } else {
            false
        };
        if factor && apply {
            let span = c.sprint_modifier - c.partial_sprint_modifier;
            let t = if span.abs() > SMALL_NUMBER {
                let t = (m - c.partial_sprint_modifier) / span;
                if t >= 0.0 {
                    minss(t, 1.0)
                } else {
                    0.0
                }
            } else if m < c.sprint_modifier {
                0.0
            } else {
                1.0
            };
            m = m * self.get_speed_factor(t);
        }
        if apply {
            if let Some(d) = self.dwarf_speed_modifier {
                m = m * d;
            }
        }
        // falling and not in knockback -> * MaxSpeedFalling (0x1414be3c6..0x1414be3d8); else
        // UAdvancedCharacterMovement::GetMaxSpeed rva=0x1484b20 = UCharacterMovementComponent::GetMaxSpeed * m
        if self.mode == Mode::Falling && !(0.0 < self.knockback_time) {
            return m * c.max_speed_falling;
        }
        self.engine_max_speed() * m
    }

    /// UCharacterMovementComponent::GetMaxSpeed rva=0x2f79a50: walking / navwalking -> crouched ? +0x190 : +0x18c;
    /// falling -> +0x18c (MaxWalkSpeed); other modes not used here
    pub fn engine_max_speed(&self) -> f32 {
        match self.mode {
            Mode::Walking | Mode::NavWalking => {
                if self.is_crouched {
                    self.c.max_walk_speed_crouched
                } else {
                    self.c.max_walk_speed
                }
            }
            Mode::Falling => self.c.max_walk_speed,
            Mode::None => 0.0,
            // MaxCustomMovementSpeed (UCharacterMovementComponent ctor 600, +0x19c at 0x142f6d08a; not read while
            // climbing: the motion sets the location)
            Mode::Custom => 600.0,
        }
    }

    /// UMordhauMovementComponent::GetAccelerationFactor rva=0x14bbbe0: max(+0xd14 + +0xd08, 0)
    pub fn get_acceleration_factor(&self) -> f32 {
        maxss(self.equip_accel_add + self.armor_accel, 0.0)
    }

    /// UMordhauMovementComponent::GetMaxAcceleration rva=0x14be0b0: modes other than Walking / NavWalking ->
    /// MaxAcceleration (+0x1a0); crouched -> WalkAcceleration; state 7 -> +0xcf0; states 3..6 -> factor x (state 4 ?
    /// SprintAcceleration +0xce0 : PartialSprintAcceleration +0xcdc); else WalkAcceleration
    pub fn get_max_acceleration(&self) -> f32 {
        if self.horse.is_some() || self.ladder.is_some() || self.pseudo.is_some() {
            return self.c.max_acceleration; // UCharacterMovementComponent::GetMaxAcceleration (horse vtable +0x6e8 -> 0x1596d00)
        }
        if !matches!(self.mode, Mode::Walking | Mode::NavWalking) {
            return self.c.max_acceleration;
        }
        if self.is_crouched {
            return self.c.walk_acceleration;
        }
        match self.sprint_state {
            Sprint::Super => self.e.supersprint_acceleration,
            Sprint::Sprint => self.get_acceleration_factor() * self.c.sprint_acceleration,
            Sprint::Partial | Sprint::Rush | Sprint::Chase => self.get_acceleration_factor() * self.c.partial_sprint_acceleration,
            _ => self.c.walk_acceleration,
        }
    }

    /// UMordhauMovementComponent::GetMaxBrakingDeceleration rva=0x14be160: falling and (Vy^2 + Vx^2) >
    /// max(GetMaxSpeed, 0)^2 * 1.01 -> BrakingDecelerationFallingTooFast; else UCharacterMovementComponent::
    /// GetMaxBrakingDeceleration rva=0x2f79940 (walking 2048 / falling BrakingDecelerationFalling)
    pub fn get_max_braking_deceleration(&self) -> f32 {
        if self.horse.is_some() || self.ladder.is_some() || self.pseudo.is_some() {
            // UCharacterMovementComponent::GetMaxBrakingDeceleration rva=0x2f79940
            return match self.mode {
                Mode::Walking | Mode::NavWalking => self.e.braking_deceleration_walking,
                Mode::Falling => self.c.braking_deceleration_falling,
                _ => 0.0,
            };
        }
        if self.mode == Mode::Falling {
            let ms = maxss(self.get_max_speed(), 0.0);
            let v2 = self.velocity.y * self.velocity.y + self.velocity.x * self.velocity.x;
            if v2 > ms * ms * RDATA_FALL_TOO_FAST {
                return self.c.braking_deceleration_falling_too_fast;
            }
            return self.c.braking_deceleration_falling;
        }
        self.e.braking_deceleration_walking
    }

    // ---- movement restriction, jump gates ------------------------------------------------------------------------

    /// AMordhauCharacter::GetMovementRestriction rva=0x1540f50 (PartialSprint for 0.1 s after a landing that ended a
    /// jump; Rush perk branch not modelled)
    pub fn get_movement_restriction(&self) -> i64 {
        let mut r = self.motion_restriction;
        if self.world_time < self.last_land + 0.1 && self.last_land_from_jump && r == 0 {
            r = restriction::PARTIAL_SPRINT;
        }
        r
    }

    /// AMordhauCharacter::IsMoveInputIgnored rva=0x154afe0
    pub fn is_move_input_ignored(&self) -> bool {
        if self.horse.is_some() {
            return self.horse_is_move_input_ignored();
        }
        self.get_movement_restriction() == restriction::NO_MOVEMENT || self.is_ragdoll_falling_or_getting_up()
    }

    /// AAdvancedCharacter::IsRagdollFallingOrGettingUp rva=0x1487ca0. RagdollFallingGetUpStartTime is zero-filled
    /// and compared with world TimeSeconds: a pawn of a world younger than RagdollFallingGetUpDuration counts as
    /// getting up (exe behaviour; the reference's per-pawn clock took it as "long ago").
    pub fn is_ragdoll_falling_or_getting_up(&self) -> bool {
        if self.ragdoll_falling {
            return true;
        }
        self.world_time < self.ragdoll_falling_get_up_start_time + self.c.ragdoll_falling_get_up_duration
    }

    /// ACharacter::CanJump rva=0x2f2eb30 -> AMordhauCharacter::CanJumpInternal_Implementation rva=0x1532630:
    /// leg flags (+0xd42 / +0xd43, not modelled = clear), the attack window (`attack_jump_block`), TimeSeconds > LastDodgeTime +
    /// DodgeDuration, not NoMovement -> AAdvancedCharacter::CanJumpInternal_Implementation rva=0x1459fb0: not
    /// ragdoll falling / getting up, JumpCooldown + LastLand <= TimeSeconds -> ACharacter::CanJumpInternal_
    /// Implementation rva=0x2f2eb40: !bIsCrouched && CanAttemptJump (rva=0x2f71610: bCanJump, !bWantsToCrouch,
    /// walking or falling), then the JumpCurrentCount / JumpMaxCount rule
    pub fn can_jump(&self) -> bool {
        let now = self.world_time;
        // a horse (AHorse vtable +0x7a8 = AAdvancedCharacter::CanJumpInternal_Implementation) skips AMordhauCharacter's
        if self.horse.is_none() {
            // fp-anim r4: the attack window (disasm 0x14153266d..0x1415326bb): the current motion IsA UAttackMotion and
            // WindupEnd (+0x1090) < TimeSeconds < WindupEnd + ReleaseJumpBlockTime (+0xbb8) -> false
            if let Some((we, bt)) = self.attack_jump_block {
                if we < now && now < we + bt {
                    return false;
                }
            }
            if !(now > self.last_dodge_time + self.c.dodge_duration) {
                return false;
            }
            if self.get_movement_restriction() == restriction::NO_MOVEMENT {
                return false;
            }
        }
        if self.is_ragdoll_falling_or_getting_up() {
            return false;
        }
        if self.c.jump_cooldown + self.last_land > now {
            return false;
        }
        let can_attempt = !self.wants_to_crouch && matches!(self.mode, Mode::Walking | Mode::NavWalking | Mode::Falling);
        if self.is_crouched || !can_attempt {
            return false;
        }
        if !self.was_jumping || self.e.jump_max_hold_time <= 0.0 {
            if self.jump_current_count == 0 && self.mode == Mode::Falling {
                self.jump_current_count + 1 < self.e.jump_max_count
            } else {
                self.jump_current_count < self.e.jump_max_count
            }
        } else {
            false // hold-time path: JumpMaxHoldTime is 0 for this pawn
        }
    }

    /// ACharacter::CheckJumpInput rva=0x2f2f1c0 (+ UCharacterMovementComponent::DoJump rva=0x2f775e0, which checks
    /// CanJump again and sets Velocity.Z = max(JumpZVelocity, Velocity.Z), Falling)
    pub fn check_jump_input(&mut self, world: &dyn World) {
        if !self.pressed_jump {
            return;
        }
        if self.jump_current_count == 0 && self.mode == Mode::Falling {
            self.jump_current_count += 1;
        }
        let did = self.can_jump() && self.do_jump(world);
        if did && !self.was_jumping {
            self.jump_current_count += 1;
            self.jump_force_time_remaining = self.e.jump_max_hold_time;
            self.on_jumped();
        }
        self.was_jumping = did;
    }

    fn do_jump(&mut self, world: &dyn World) -> bool {
        // UHorseMovementComponent::DoJump rva=0x14ba690: Gear < GearInfo.Num and !GearInfo[Gear].bAllowJump -> false
        if let Some(h) = &self.horse {
            if let Some(g) = h.cfg.gears.get(h.gear as usize) {
                if !g.allow_jump {
                    return false;
                }
            }
        }
        if !self.can_jump() {
            return false;
        }
        self.velocity.z = maxss(self.c.jump_z_velocity, self.velocity.z);
        self.set_movement_mode(world, Mode::Falling);
        true
    }

    /// ACharacter::ClearJumpInput rva=0x2f2f3d0
    pub fn clear_jump_input(&mut self, dt: f32) {
        if self.pressed_jump {
            self.jump_key_hold_time = dt + self.jump_key_hold_time;
            if !(self.e.jump_max_hold_time > self.jump_key_hold_time) {
                self.pressed_jump = false;
            }
        } else {
            self.was_jumping = false;
            self.jump_force_time_remaining = 0.0;
        }
    }

    /// AAdvancedCharacter::OnJumped_Implementation rva=0x148d580 / AMordhauCharacter::OnJumped_Implementation
    /// rva=0x1554610 (stamina on the combat side: counted in `jumps`)
    pub fn on_jumped(&mut self) {
        self.airborne_from_jump = true;
        self.jumps += 1;
    }

    /// AAdvancedCharacter::Landed rva=0x1489820 (+ ACharacter::Landed rva=0x2f3c430 resetting the jump state via
    /// OnMovementModeChanged)
    pub fn landed(&mut self) {
        self.last_land_from_jump = self.airborne_from_jump;
        self.last_land = self.world_time;
        self.airborne_from_jump = false;
        self.landings += 1;
    }

    // ---- knockback, fall damage, ragdoll ---------------------------------------------------------------------------

    /// UMordhauMovementComponent::Knockback rva=0x14c2820 (amount in UE axes). Authority: AddImpulse(Amount.X,
    /// Amount.Y, (IsFalling ? 0 : KnockbackUpImpulse) + Amount.Z) with bVelocityChange (0x1414c2852..0x1414c285a adds
    /// Amount.Z in both cases), then bHasRequestedVelocity (+0x38e bit 1) = false and RequestedVelocity (+0x3a4) = 0;
    /// every role (the authority test jumps to 0x1414c28ae): KnockbackTime, FallingLateralFriction, GroundFriction
    pub fn knockback(&mut self, amount: FVector) {
        if self.authority {
            let up = if self.mode == Mode::Falling { 0.0 } else { self.c.knockback_up_impulse };
            self.add_impulse(v(amount.x, amount.y, up + amount.z));
        }
        self.knockback_time = self.c.knockback_duration;
        self.c.falling_lateral_friction = self.c.knockback_falling_lateral_friction;
        self.c.ground_friction = self.c.knockback_ground_friction;
    }

    /// UCharacterMovementComponent::AddImpulse(Impulse, bVelocityChange = true): not zero and not MOVE_None ->
    /// PendingImpulseToApply += Impulse
    pub fn add_impulse(&mut self, impulse: FVector) {
        if !is_zero(impulse) && self.mode != Mode::None {
            self.pending_impulse = add(self.pending_impulse, impulse);
        }
    }

    pub fn is_in_knockback(&self) -> bool {
        0.0 < self.knockback_time
    }

    /// UMordhauMovementComponent::PostCharacterMovementTick rva=0x14d0710
    pub fn post_character_movement_tick(&mut self, dt: f32) {
        self.knockback_time = self.knockback_time - dt;
        if !(self.knockback_time > 0.0) {
            self.c.falling_lateral_friction = self.base_falling_lateral_friction;
            self.c.ground_friction = self.base_ground_friction;
            self.knockback_time = 0.0;
        }
    }

    /// UAdvancedCharacterMovement::CheckFallDamage rva=0x145a230 (f32: Delta = Z - Last; Last = Z; Delta > Min and
    /// (Delta + Offset) * Factor > 0 -> TakeDamage)
    pub fn check_fall_damage(&mut self, vz: f32) -> f32 {
        if !self.authority {
            return 0.0;
        }
        let (mn, off, fac) = if self.ragdoll_falling {
            (self.c.ragdoll_min_velocity_for_fall_damage, self.c.ragdoll_fall_damage_offset, self.c.ragdoll_fall_damage_factor)
        } else {
            (self.c.min_velocity_for_fall_damage, self.c.fall_damage_offset, self.c.fall_damage_factor)
        };
        let delta = vz - self.last_falling_check_velocity_z;
        self.last_falling_check_velocity_z = vz;
        if delta > mn {
            let d = (delta + off) * fac;
            if d > 0.0 {
                self.fall_damage.push(d);
                return d;
            }
        }
        0.0
    }

    /// AMordhauCharacter::Trip rva=0x156f7b0 -> AAdvancedCharacter::Trip rva=0x14a78c0
    pub fn trip(&mut self) -> bool {
        if self.dead || self.ragdoll_falling || self.c.disable_ragdoll_falling || self.is_ragdoll_falling_or_getting_up() {
            return false;
        }
        self.set_is_ragdoll_falling(true);
        self.trips += 1;
        true
    }

    /// AAdvancedCharacter::SetIsRagdollFalling rva=0x14a0980
    pub fn set_is_ragdoll_falling(&mut self, on: bool) {
        if self.dead || self.ragdoll_falling == on {
            return;
        }
        self.ragdoll_falling = on;
        if on {
            self.ragdoll_falling_start_time = self.world_time;
        } else {
            self.ragdoll_falling_get_up_start_time = self.world_time;
        }
        self.ragdoll_changes.push(on);
    }

    /// UAdvancedCharacterMovement::TickComponent rva=0x14a3e80, the get-up (0x1414a4365..0x1414a4419; f32,
    /// |V|^2 as (X^2 + Y^2) + Z^2 against MinVelocity^2)
    pub fn tick_ragdoll_get_up(&mut self, dt: f32) {
        if self.dead {
            return;
        }
        let mv = self.c.ragdoll_falling_min_velocity_to_get_up;
        if !self.ragdoll_falling || mv * mv < size_sq(self.velocity) {
            self.still_time_while_ragdoll_falling = 0.0;
            return;
        }
        self.still_time_while_ragdoll_falling = dt + self.still_time_while_ragdoll_falling;
        if self.c.ragdoll_falling_time_at_min_velocity_to_get_up <= self.still_time_while_ragdoll_falling
            && self.ragdoll_falling_start_time + self.c.ragdoll_falling_min_time < self.world_time
            && self.authority
        {
            self.set_is_ragdoll_falling(false);
        }
    }

    /// GetGravityZ = DefaultGravityZ x GravityScale (UCharacterMovementComponent::GetGravityZ vcall +0x400)
    pub fn gravity_z(&self) -> f32 {
        self.c.gravity_z * self.c.gravity_scale
    }
}

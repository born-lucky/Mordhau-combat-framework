//! UCharacterMovementComponent / UMovementComponent / UPrimitiveComponent::MoveComponent (UE 4.26, statically linked
//! into Mordhau-Win64-Shipping.exe), ported from the disassembly at each function's PDB address: walking with floor
//! finding, step-up, sliding, perching, falling with landing, crouch / uncrouch with encroachment tests.
//! Engine features the game's pawn does not use are left out and named where they would branch: root motion, nav
//! walking, swimming, flying, custom modes, based movement on moving platforms (bases are static here), avoidance,
//! plane constraints, networking (simulated proxies, saved moves), physics interaction forces.

use crate::exe::*;
use crate::ue::FVector;
use crate::uemath::*;
use crate::world::{HitResult, World};

/// MIN_TICK_TIME (.rdata 0x144000104, 1e-6f)
pub const MIN_TICK_TIME: f32 = 1e-6;
/// MAX_FLOOR_DIST (.rdata 0x1445393a4, 2.4f)
pub const MAX_FLOOR_DIST: f32 = 2.4;
/// MIN_FLOOR_DIST (.rdata 0x14497e674, 1.9f)
pub const MIN_FLOOR_DIST: f32 = 1.9;
/// (MIN_FLOOR_DIST + MAX_FLOOR_DIST) * 0.5 (.rdata 0x14497e678, 2.15f)
pub const AVG_FLOOR_DIST: f32 = 2.15;
/// MAX_FLOOR_DIST + KINDA_SMALL_NUMBER (.rdata 0x14497e67c, 2.4001f), FindFloor's walking height adjust
pub const FLOOR_HEIGHT_CHECK_ADJUST_WALKING: f32 = 2.4001;
/// SWEEP_EDGE_REJECT_DISTANCE (.rdata 0x1442efa58, 0.15f)
pub const SWEEP_EDGE_REJECT_DISTANCE: f32 = 0.15;
/// SWEEP_EDGE_REJECT_DISTANCE + KINDA_SMALL_NUMBER (.rdata 0x14497e664, 0.1501f)
pub const EDGE_REJECT_MIN: f32 = 0.1501;
/// MAX_STEP_SIDE_Z (.rdata 0x14497e68c holds -0.08f: StepUp compares ImpactNormal | GravDir against it)
pub const MAX_STEP_SIDE_Z: f32 = 0.08;
/// VERTICAL_SLOPE_NORMAL_Z (.rdata 0x144014a8c, 0.001f)
pub const VERTICAL_SLOPE_NORMAL_Z: f32 = 0.001;
/// BRAKE_TO_STOP_VELOCITY^2 (.rdata 0x143fe4e40, 100)
pub const BRAKE_TO_STOP_SQ: f32 = 100.0;
/// 1 - KINDA_SMALL_NUMBER (.rdata 0x1440e9f8c, 0.9999f), ComputeGroundMovementDelta's flat-floor test
pub const NEARLY_FLAT_Z: f32 = 0.9999;
/// 1 - DELTA (.rdata, 0.99999f), UCharacterMovementComponent::SlideAlongSurface
pub const ONE_MINUS_DELTA: f32 = 0.99999;
/// 4 * KINDA_SMALL_NUMBER^2 (.rdata 0x14498d730, 1.6e-7f): UPrimitiveComponent::MoveComponentImpl's minimum swept move
pub const MIN_MOVEMENT_DIST_SQ: f32 = 1.599_999_9e-7;
/// KINDA_SMALL_NUMBER * 10 as the exe stores it (.rdata 0x1446254fc / 0x14497e688, 0.000999999931f)
pub const KINDA_SMALL_X10: f32 = 0.000_999_999_93;
/// 0.95f + KINDA_SMALL_NUMBER * 10 (.rdata 0x14497e668, 0.951f): UnCrouch's SweepInflation + MIN_FLOOR_DIST / 2
pub const UNCROUCH_BASE_LIFT: f32 = 0.951;

impl ExeMovement {
    // ---- capsule -----------------------------------------------------------------------------------------------

    pub fn half_height(&self) -> f32 {
        self.capsule_half_height
    }

    // ============================================================================================================
    // ControlledCharacterMove / PerformMovement / StartNewPhysics
    // ============================================================================================================

    /// UCharacterMovementComponent::ControlledCharacterMove rva=0x2f75ee0
    pub fn controlled_character_move(&mut self, world: &dyn World, input: FVector, dt: f32) {
        self.check_jump_input(world); // ACharacter::CheckJumpInput rva=0x2f2f1c0 (vcall +0x840 on the owner)
        let a = self.constrain_input_acceleration(input);
        self.acceleration = self.scale_input_acceleration(a);
        self.analog_input_modifier = self.compute_analog_input_modifier();
        self.perform_movement(world, dt);
        // UAdvancedCharacterMovement::PerformMovement rva=0x1495cc0 tail: CheckFallDamage(Velocity.Z)
        let vz = self.velocity.z;
        self.check_fall_damage(vz);
    }

    /// UMordhauMovementComponent::ConstrainInputAcceleration rva=0x14b7a90 -> UAdvancedCharacterMovement::
    /// ConstrainInputAcceleration rva=0x145aee0 (bIgnoreMovementInput +0xbc4 -> zero) -> UCharacterMovementComponent::
    /// ConstrainInputAcceleration rva=0x2f75e50 (drops Z while walking / falling: input here is already planar)
    pub fn constrain_input_acceleration(&self, a: FVector) -> FVector {
        if self.ignore_movement_input {
            return FVector::ZERO;
        }
        if self.ladder.is_some() {
            return a; // UOneDimensionalMovementComponent::ConstrainInputAcceleration rva=0x14b7ab0
        }
        if a.z != 0.0 && matches!(self.mode, Mode::Walking | Mode::NavWalking | Mode::Falling) {
            return v(a.x, a.y, 0.0);
        }
        a
    }

    /// UCharacterMovementComponent::ScaleInputAcceleration rva=0x2f85bf0: GetMaxAcceleration() *
    /// Input.GetClampedToMaxSize(1) (SizeSquared > 1 -> * InvSqrt(SizeSquared))
    pub fn scale_input_acceleration(&self, a: FVector) -> FVector {
        let sq = size_sq(a);
        let c = if sq > 1.0 { mul(a, inv_sqrt(sq)) } else { a };
        mul(c, self.get_max_acceleration())
    }

    /// UCharacterMovementComponent::ComputeAnalogInputModifier rva=0x2f74f50: SizeSquared > 0 and MaxAccel >
    /// SMALL_NUMBER -> clamp(Size * (1 / MaxAccel), 0, 1) (a multiply by the reciprocal, 0x142f74fb1..0x142f74fb8)
    pub fn compute_analog_input_modifier(&self) -> f32 {
        let max_accel = self.get_max_acceleration();
        let sq = size_sq(self.acceleration);
        if sq > 0.0 && max_accel > SMALL_NUMBER {
            let r = sq.sqrt() * (1.0 / max_accel);
            return if r >= 0.0 { minss(r, 1.0) } else { 0.0 };
        }
        0.0
    }

    /// UCharacterMovementComponent::PerformMovement rva=0x2f7e220, the parts a walking / falling pawn without root
    /// motion runs: ApplyAccumulatedForces, UpdateCharacterStateBeforeMovement, HandlePendingLaunch (none),
    /// ClearAccumulatedForces, ClearJumpInput, NumJumpApexAttempts = 0, StartNewPhysics, UpdateCharacterStateAfterMovement
    pub fn perform_movement(&mut self, world: &dyn World, dt: f32) {
        if self.mode == Mode::None {
            return;
        }
        if self.is_moving_on_ground() && !eq(self.location, self.last_update_location) {
            self.force_next_floor_check = true;
        }
        self.apply_accumulated_forces(world, dt);
        self.update_character_state_before_movement(world);
        self.pending_impulse = FVector::ZERO; // ClearAccumulatedForces rva=0x2f73000
        self.pending_force = FVector::ZERO;
        self.clear_jump_input(dt);
        self.num_jump_apex_attempts = 0;
        self.start_new_physics(world, dt, 0);
        self.update_character_state_after_movement(world);
        // PhysicsRotation (vcall +0x7e0, 0x142f7ea0d): ported for horses (exe_horse.rs); a character's yaw is the input's
        if self.horse.is_some() {
            self.horse_physics_rotation(dt);
        }
        if self.pseudo.is_some() {
            self.pseudo_physics_rotation(world, dt);
        }
        self.last_update_location = self.location;
    }

    /// UCharacterMovementComponent::ApplyAccumulatedForces rva=0x2f6e650: PendingImpulse.Z or PendingForce.Z != 0, on
    /// the ground and (GetGravityZ + PendingForce.Z) * dt + PendingImpulse.Z > SMALL_NUMBER -> Falling; Velocity +=
    /// PendingImpulse + PendingForce * dt (per component: (dt * F + I) + V)
    pub fn apply_accumulated_forces(&mut self, world: &dyn World, dt: f32) {
        if (self.pending_impulse.z != 0.0 || self.pending_force.z != 0.0) && self.is_moving_on_ground() {
            let up = (self.gravity_z() + self.pending_force.z) * dt + self.pending_impulse.z;
            if up > SMALL_NUMBER {
                self.set_movement_mode(world, Mode::Falling);
            }
        }
        let (i, f) = (self.pending_impulse, self.pending_force);
        self.velocity = v(dt * f.x + i.x + self.velocity.x, f.y * dt + i.y + self.velocity.y, f.z * dt + i.z + self.velocity.z);
    }

    pub fn is_moving_on_ground(&self) -> bool {
        matches!(self.mode, Mode::Walking | Mode::NavWalking)
    }
    pub fn is_falling(&self) -> bool {
        self.mode == Mode::Falling
    }

    /// UCharacterMovementComponent::StartNewPhysics rva=0x2f8ecd0
    pub fn start_new_physics(&mut self, world: &dyn World, dt: f32, iterations: i32) {
        if dt < MIN_TICK_TIME || iterations >= self.e.max_simulation_iterations {
            return;
        }
        match self.mode {
            Mode::Walking | Mode::NavWalking if self.ladder.is_some() => self.ladder_phys_walking(dt),
            Mode::Walking | Mode::NavWalking => self.phys_walking(world, dt, iterations),
            Mode::Falling => self.phys_falling(world, dt, iterations),
            // PhysCustom: the base class only fires the K2_UpdateCustomMovement event, which BP_MordhauCharacter does
            // not implement (no UpdateCustomMovement function in its bytecode, state/character_kismet/
            // BP_MordhauCharacter.txt); the climb motion moves the actor (exe_climb.rs)
            Mode::None | Mode::Custom => {}
        }
    }

    /// GetSimulationTimeStep (inlined in PhysWalking 0x142f818fa..0x142f8192b and PhysFalling)
    fn sim_time_step(&self, remaining: f32, iterations: i32) -> f32 {
        let mts = self.e.max_simulation_time_step;
        let mut step = remaining;
        if remaining > mts && iterations < self.e.max_simulation_iterations {
            step = minss(remaining * 0.5, mts);
        }
        maxss(step, MIN_TICK_TIME)
    }

    // ============================================================================================================
    // movement mode
    // ============================================================================================================

    /// UCharacterMovementComponent::SetMovementMode rva=0x2f89c40 -> OnMovementModeChanged (UMordhauMovementComponent
    /// rva=0x14cc3f0 -> UAdvancedCharacterMovement rva=0x14901a0: entering Walking / NavWalking -> CheckFallDamage(0);
    /// then UCharacterMovementComponent::OnMovementModeChanged rva=0x2f7d680)
    pub fn set_movement_mode(&mut self, world: &dyn World, mode: Mode) {
        if self.mode == mode {
            return;
        }
        let prev = self.mode;
        self.mode = mode;
        if matches!(mode, Mode::Walking | Mode::NavWalking) {
            self.check_fall_damage(0.0);
        }
        // UCharacterMovementComponent::OnMovementModeChanged
        if self.mode == Mode::Walking {
            // 0x142f7d763..: bCrouchMaintainsBaseLocation, Velocity.Z = 0, GroundMovementMode, FindFloor, AdjustFloorHeight,
            // SetBaseFromFloor
            self.crouch_maintains_base_location = true;
            self.velocity.z = 0.0;
            let loc = self.location;
            let mut floor = self.current_floor;
            self.find_floor(world, loc, &mut floor, false, None);
            self.current_floor = floor;
            self.adjust_floor_height(world);
            self.base = if self.current_floor.is_walkable_floor() { self.current_floor.hit.component } else { None };
        } else {
            self.current_floor.clear();
            self.crouch_maintains_base_location = false;
            // falling: Velocity += GetImpartedMovementBaseVelocity (static bases: zero); CharacterOwner->Falling()
            self.base = None;
        }
        // UMordhauMovementComponent::OnMovementModeChanged rva=0x14cc3f0 then sets bCrouchMaintainsBaseLocation (+0x38c bit
        // 5) whatever the mode (0x1414cc3fe; src/Mordhau/Private/Components/MordhauMovementComponent.cpp, byte-matched)
        if self.horse.is_none() && self.pseudo.is_none() {
            self.crouch_maintains_base_location = true;
        }
        // ACharacter::OnMovementModeChanged rva=0x2f3d6b0: !IsFalling -> ResetJumpState (bPressedJump, bWasJumping,
        // JumpKeyHoldTime, JumpForceTimeRemaining, JumpCurrentCount = 0 when not falling)
        if !self.is_falling() {
            self.pressed_jump = false;
            self.was_jumping = false;
            self.jump_key_hold_time = 0.0;
            self.jump_force_time_remaining = 0.0;
            self.jump_current_count = 0;
        }
        let _ = prev;
    }

    // ============================================================================================================
    // crouch
    // ============================================================================================================

    /// UCharacterMovementComponent::UpdateCharacterStateBeforeMovement rva=0x2f92070
    pub fn update_character_state_before_movement(&mut self, world: &dyn World) {
        if self.is_crouched {
            if !self.wants_to_crouch || !self.can_crouch_in_current_state() {
                self.uncrouch(world);
            }
        } else if self.wants_to_crouch && self.can_crouch_in_current_state() {
            self.crouch(world);
        }
    }

    /// UCharacterMovementComponent::UpdateCharacterStateAfterMovement rva=0x2f92020: crouched in a mode it cannot
    /// crouch in -> UnCrouch
    pub fn update_character_state_after_movement(&mut self, world: &dyn World) {
        if self.is_crouched && !self.can_crouch_in_current_state() {
            self.uncrouch(world);
        }
    }

    /// UCharacterMovementComponent::CanCrouchInCurrentState rva=0x2f71b40: bCanCrouch and (falling or on the ground)
    pub fn can_crouch_in_current_state(&self) -> bool {
        self.c.can_crouch && (self.is_falling() || self.is_moving_on_ground())
    }

    /// UCharacterMovementComponent::Crouch rva=0x2f769e0 (bClientSimulation = false)
    pub fn crouch(&mut self, world: &dyn World) {
        if !self.can_crouch_in_current_state() {
            return;
        }
        if self.capsule_half_height == self.c.crouched_half_height {
            self.is_crouched = true;
            return;
        }
        let old_hh = self.capsule_half_height;
        let r = self.e.capsule_radius;
        // Max3(0, OldUnscaledRadius, CrouchedHalfHeight)
        let clamped = maxss(maxss(r, 0.0), self.c.crouched_half_height);
        self.capsule_half_height = clamped;
        let adjust = (old_hh - clamped) * 1.0; // * ComponentScale (1)
        if clamped > old_hh {
            // crouching to a larger height: encroachment test at the raised centre (0x142f76b5e..)
            let c = v(self.location.x, self.location.y, self.location.z - adjust);
            if world.overlap_capsule(c, r, clamped) {
                self.capsule_half_height = old_hh;
                return;
            }
        }
        if self.crouch_maintains_base_location {
            // UpdatedComponent->MoveComponent(-adjust Z, swept, TeleportPhysics) (0x142f76d1c..0x142f76d72)
            let mut hit = HitResult::new(1.0);
            self.move_component(world, v(0.0, 0.0, -adjust), true, false, &mut hit);
        }
        self.is_crouched = true;
        self.force_next_floor_check = true;
    }

    /// UCharacterMovementComponent::UnCrouch rva=0x2f906b0 (bClientSimulation = false)
    pub fn uncrouch(&mut self, world: &dyn World) {
        let default_hh = self.e.capsule_half_height;
        let r = self.e.capsule_radius;
        if self.capsule_half_height == default_hh {
            self.is_crouched = false;
            return;
        }
        let current = self.capsule_half_height;
        let adjust = (default_hh - current) * 1.0;
        let pawn = self.location;
        // GetPawnCapsuleCollisionShape(SHRINK_HeightCustom, -SweepInflation - adjust): half height grown, both >= 0.001
        let standing_hh = maxss(current - (-KINDA_SMALL_X10 - adjust), KINDA_SMALL_X10);
        let standing_r = maxss(r, KINDA_SMALL_X10);
        let mut encroached;
        if !self.crouch_maintains_base_location {
            encroached = world.overlap_capsule(pawn, standing_r, standing_hh);
            if encroached && adjust > 0.0 {
                // shrink to a short capsule, sweep down to the base, try standing up from there (0x142f90947..)
                let shrink_hh = current - r;
                let trace = current - shrink_hh;
                let short_hh = maxss(current - shrink_hh, KINDA_SMALL_X10);
                let short_r = maxss(r, KINDA_SMALL_X10);
                let down = v(pawn.x, pawn.y, pawn.z - trace);
                let hits = world.sweep_capsule(pawn, down, short_r, short_hh);
                let hit = hits.first().copied().unwrap_or_else(|| HitResult::new(1.0));
                if hit.start_penetrating {
                    encroached = true;
                } else {
                    let to_base = hit.time * trace + short_hh;
                    let z = pawn.z - to_base + standing_hh + UNCROUCH_BASE_LIFT;
                    let nl = v(pawn.x, pawn.y, z);
                    encroached = world.overlap_capsule(nl, standing_r, standing_hh);
                    if !encroached {
                        self.location = nl;
                    }
                }
            }
        } else {
            let mut standing = v(pawn.x, pawn.y, standing_hh - current + pawn.z);
            encroached = world.overlap_capsule(standing, standing_r, standing_hh);
            if encroached && self.is_moving_on_ground() {
                let fd = self.current_floor.floor_dist;
                if self.current_floor.blocking_hit && fd > KINDA_SMALL_X10 {
                    standing.z = standing.z - (fd - KINDA_SMALL_X10);
                    encroached = world.overlap_capsule(standing, standing_r, standing_hh);
                }
            }
            if !encroached {
                self.location = standing;
                self.force_next_floor_check = true;
            }
        }
        if encroached {
            return;
        }
        self.is_crouched = false;
        self.capsule_half_height = default_hh;
    }

    // ============================================================================================================
    // velocity
    // ============================================================================================================

    /// UAdvancedCharacterMovement::CalcVelocity rva=0x1459ab0 -> UCharacterMovementComponent::CalcVelocity rva=0x2f70490
    /// (no requested move, no bForceMaxAccel, no fluid)
    pub fn calc_velocity(&mut self, dt: f32, friction: f32, braking_decel: f32) {
        if !(dt >= MIN_TICK_TIME) {
            return;
        }
        let friction = maxss(friction, 0.0);
        let max_speed = self.get_max_speed();
        // MaxInputSpeed = max(MaxSpeed * AnalogInputModifier, GetMinAnalogSpeed() (0 for walking: MinAnalogWalkSpeed 0))
        let max_input_speed = maxss(max_speed * self.analog_input_modifier, 0.0);
        let max_speed2 = maxss(max_input_speed, 0.0); // RequestedSpeed
        let zero_accel = is_zero(self.acceleration);
        let over_max = self.is_exceeding_max_speed(max_speed2);
        if zero_accel || over_max {
            let old = self.velocity;
            let bf = if self.e.use_separate_braking_friction { self.e.braking_friction } else { friction };
            self.apply_velocity_braking(dt, bf, braking_decel);
            if over_max && size_sq(self.velocity) < max_speed2 * max_speed2 && dot(old, self.acceleration) > 0.0 {
                self.velocity = mul(safe_normal(old), max_speed2);
            }
        } else {
            let dir = safe_normal(self.acceleration);
            let vs = size(self.velocity);
            let k = minss(dt * friction, 1.0);
            let vv = self.velocity;
            self.velocity = v(vv.x - (vv.x - dir.x * vs) * k, vv.y - (vv.y - dir.y * vs) * k, vv.z - (vv.z - dir.z * vs) * k);
        }
        if !zero_accel {
            let new_max = if self.is_exceeding_max_speed(max_input_speed) { size(self.velocity) } else { max_input_speed };
            let a = self.acceleration;
            self.velocity = v(dt * a.x + self.velocity.x, a.y * dt + self.velocity.y, a.z * dt + self.velocity.z);
            self.velocity = clamped_to_max_size(self.velocity, new_max);
        }
    }

    /// UMovementComponent::IsExceedingMaxSpeed rva=0x2fb3720
    pub fn is_exceeding_max_speed(&self, max_speed: f32) -> bool {
        let ms = maxss(max_speed, 0.0);
        size_sq(self.velocity) > ms * ms * RDATA_FALL_TOO_FAST
    }

    /// UCharacterMovementComponent::ApplyVelocityBraking rva=0x2f6f930
    pub fn apply_velocity_braking(&mut self, dt: f32, friction: f32, braking_decel: f32) {
        if is_zero(self.velocity) || !(dt >= MIN_TICK_TIME) {
            return;
        }
        let friction = maxss(maxss(self.c.braking_friction_factor, 0.0) * friction, 0.0);
        let braking = maxss(braking_decel, 0.0);
        let zero_friction = friction == 0.0;
        let zero_braking = braking == 0.0;
        if zero_friction && zero_braking {
            return;
        }
        let old = self.velocity;
        let step_min = f32::from_bits(0x3c5a_740e); // 1/75 (.rdata 0x14497e65c)
        let max_step = if self.e.braking_sub_step_time >= step_min { minss(self.e.braking_sub_step_time, 0.05) } else { step_min };
        let rev = if zero_braking { FVector::ZERO } else { mul(safe_normal(self.velocity), -braking) };
        let nf = -friction;
        let mut remaining = dt;
        while remaining >= MIN_TICK_TIME {
            let t = if remaining > max_step && !zero_friction { minss(remaining * 0.5, max_step) } else { remaining };
            remaining = remaining - t;
            let vv = self.velocity;
            self.velocity = v((vv.x * nf + rev.x) * t + vv.x, (vv.y * nf + rev.y) * t + vv.y, (vv.z * nf + rev.z) * t + vv.z);
            if dot(old, self.velocity) <= 0.0 {
                self.velocity = FVector::ZERO;
                return;
            }
        }
        let sq = size_sq(self.velocity);
        if sq <= KINDA_SMALL_NUMBER || (!zero_braking && sq <= BRAKE_TO_STOP_SQ) {
            self.velocity = FVector::ZERO;
        }
    }

    /// UCharacterMovementComponent::MaintainHorizontalGroundVelocity rva=0x2f7bc50 (bMaintainHorizontalGroundVelocity
    /// -> Z = 0; else Velocity.GetSafeNormal2D() * Size)
    pub fn maintain_horizontal_ground_velocity(&mut self) {
        if self.velocity.z != 0.0 {
            if self.e.maintain_horizontal_ground_velocity {
                self.velocity.z = 0.0;
            } else {
                let s = size(self.velocity);
                self.velocity = mul(safe_normal_2d(self.velocity), s);
            }
        }
    }

    /// UCharacterMovementComponent::GetFallingLateralAcceleration rva=0x2f794b0 -> GetAirControl rva=0x2f790f0 ->
    /// BoostAirControl rva=0x2f701e0, then GetClampedToMaxSize(GetMaxAcceleration())
    pub fn get_falling_lateral_acceleration(&self) -> FVector {
        let mut fa = v(self.acceleration.x, self.acceleration.y, 0.0);
        if fa.x * fa.x + fa.y * fa.y > 0.0 {
            let mut ac = self.c.air_control;
            if ac != 0.0 {
                let vv = self.velocity;
                let thr = self.c.air_control_boost_velocity_threshold;
                if self.c.air_control_boost_multiplier > 0.0 && vv.x * vv.x + vv.y * vv.y < thr * thr {
                    ac = minss(self.c.air_control_boost_multiplier * ac, 1.0);
                }
            }
            fa = v(ac * fa.x, ac * fa.y, ac * fa.z);
            fa = clamped_to_max_size(fa, self.get_max_acceleration());
        }
        fa
    }

    /// UCharacterMovementComponent::NewFallVelocity rva=0x2f7cec0
    pub fn new_fall_velocity(&self, initial: FVector, gravity: FVector, dt: f32) -> FVector {
        let mut r = initial;
        if dt > 0.0 {
            r = v(dt * gravity.x + r.x, dt * gravity.y + r.y, dt * gravity.z + r.z);
            let limit = self.c.terminal_velocity.abs();
            if size_sq(r) > limit * limit {
                let gd = safe_normal(gravity);
                if dot(r, gd) > limit {
                    let d = (r.y - 0.0) * gd.y + (r.x - 0.0) * gd.x + (r.z - 0.0) * gd.z;
                    r = v((r.x - gd.x * d) + gd.x * limit, (r.y - gd.y * d) + gd.y * limit, (r.z - gd.z * d) + gd.z * limit);
                }
            }
        }
        r
    }

    // ============================================================================================================
    // moving the capsule
    // ============================================================================================================

    /// UPrimitiveComponent::MoveComponentImpl rva=0x2fb4250 (the parts a static-world capsule move uses): swept moves
    /// below MIN_MOVEMENT_DIST_SQ do nothing; each sweep hit is pulled back (PullBackHit: Time -= Clamp(0.1,
    /// 0.1 * (1/Dist), 1/Dist) + 0.001, clamped to [0, 1]); the first blocking hit wins, or among initial overlaps the
    /// one whose ImpactNormal is most opposed to the move; the unnamed code at 0x2fbf140 called from MoveComponentImpl at 0x142fb496d
    /// (ShouldIgnoreHitResult by its behaviour; no PDB name) drops initial overlaps
    /// the move leaves (ImpactNormal | MoveDir > 0) unless `never_ignore_overlaps` (SafeMoveUpdatedComponent sets
    /// MOVECOMP_NeverIgnoreBlockingOverlaps, 0x142fba4c7); the new location is TraceStart + (TraceEnd - TraceStart) *
    /// Time, snapped back to the start when that is within MIN_MOVEMENT_DIST_SQ.
    /// The p.HitDistanceTolerance / p.InitialOverlapTolerance cvars are 0: their FAutoConsoleVariableRef registrations
    /// (0x140741ef5 / 0x140741f65) point at floats in the zero-filled tail of .data (0x1458faad8 / 0x1458faad4) and
    /// no shipped ini sets them.
    pub fn move_component(&mut self, world: &dyn World, delta: FVector, sweep: bool, never_ignore_overlaps: bool, out: &mut HitResult) -> bool {
        // UPseudoVehicleMovementComponent::MoveUpdatedComponentImpl rva=0x1500230 with SecondaryComponents (exe_pseudo.rs)
        if self.pseudo.as_ref().is_some_and(|p| !p.cfg.secondaries.is_empty()) {
            return self.pseudo_move_updated_component(world, delta, sweep, never_ignore_overlaps, out);
        }
        self.move_component_root(world, delta, sweep, never_ignore_overlaps, out)
    }

    /// the root capsule's UPrimitiveComponent::MoveComponentImpl (the doc of `move_component`)
    pub(crate) fn move_component_root(&mut self, world: &dyn World, delta: FVector, sweep: bool, never_ignore_overlaps: bool, out: &mut HitResult) -> bool {
        let start = self.location;
        let end = add(start, delta);
        *out = HitResult::new(1.0);
        out.trace_start = start;
        out.trace_end = end;
        let dsq = size_sq(delta);
        let min_sq = if sweep { MIN_MOVEMENT_DIST_SQ } else { 0.0 };
        if dsq <= min_sq {
            return true;
        }
        if !sweep {
            self.location = end;
            return true;
        }
        let hits = world.sweep_capsule(start, end, self.e.capsule_radius, self.capsule_half_height);
        match select_move_hit(hits, start, delta, never_ignore_overlaps) {
            Some((h, nl)) => {
                *out = h;
                self.location = nl;
            }
            None => self.location = end,
        }
        true
    }

    /// UMovementComponent::MoveUpdatedComponentImpl rva=0x2fb5730 (ConstrainDirectionToPlane: no plane constraint)
    pub fn move_updated_component(&mut self, world: &dyn World, delta: FVector, sweep: bool, out: &mut HitResult) -> bool {
        self.move_component(world, delta, sweep, false, out)
    }

    /// UMovementComponent::SafeMoveUpdatedComponent rva=0x2fba420: move with NeverIgnoreBlockingOverlaps; an initial
    /// penetration -> GetPenetrationAdjustment, ResolvePenetration and, when that moved, the original move again
    pub fn safe_move_updated_component(&mut self, world: &dyn World, delta: FVector, out: &mut HitResult) -> bool {
        let mut moved = self.move_component(world, delta, true, true, out);
        if out.start_penetrating {
            let adj = self.get_penetration_adjustment(out);
            let h = *out;
            if self.resolve_penetration(world, adj, &h) {
                moved = self.move_component(world, delta, true, true, out);
            }
        }
        moved
    }

    /// UCharacterMovementComponent::GetPenetrationAdjustment rva=0x2f7a210 -> UMovementComponent::
    /// GetPenetrationAdjustment rva=0x2fab940 (Normal * (PenetrationDepth (or 0.125) + 0.125)), clamped to
    /// MaxDepenetrationWithGeometry (500)
    pub fn get_penetration_adjustment(&self, hit: &HitResult) -> FVector {
        if !hit.start_penetrating {
            return FVector::ZERO;
        }
        let depth = if hit.penetration_depth > 0.0 { hit.penetration_depth } else { 0.125 };
        let r = mul(hit.normal, depth + 0.125);
        clamped_to_max_size(r, self.e.max_depenetration_with_geometry)
    }

    /// UCharacterMovementComponent::ResolvePenetrationImpl rva=0x2f85400 -> UMovementComponent::ResolvePenetrationImpl
    /// rva=0x2fb9950: fits at TraceStart + Adjustment (overlap test with the shape inflated by 0.1, .rdata 0.1f) ->
    /// teleport there; else sweep the adjustment ignoring the overlap, then adjustment + second MTD, then adjustment +
    /// the attempted move. bJustTeleported |= moved. MoveComponent's return value is USceneComponent::
    /// InternalSetWorldLocationAndRotation's (the call at 0x142fb46a8, kept in r15b and returned at 0x142fb5605): whether
    /// the transform changed; these moves keep the rotation, so "the location changed".
    pub fn resolve_penetration(&mut self, world: &dyn World, adj: FVector, hit: &HitResult) -> bool {
        if is_zero(adj) {
            return false;
        }
        let test = add(hit.trace_start, adj);
        let r = self.e.capsule_radius + 0.1;
        let hh = self.capsule_half_height + 0.1;
        // OverlapTest (UPseudoVehicleMovementComponent::OverlapTest rva=0x15109f0 with SecondaryComponents)
        let blocked = if self.pseudo.as_ref().is_some_and(|p| !p.cfg.secondaries.is_empty()) {
            self.pseudo_overlap_test(world, test, self.yaw, r, hh)
        } else {
            world.overlap_capsule(test, r, hh)
        };
        let moved = if !blocked {
            let mut h = HitResult::new(1.0);
            self.move_component(world, adj, false, false, &mut h);
            true
        } else {
            let before = self.location;
            let mut out = HitResult::new(1.0);
            self.move_component(world, adj, true, false, &mut out);
            let mut moved = !eq(before, self.location);
            if !moved && out.start_penetrating {
                let second = self.get_penetration_adjustment(&out);
                let combined = add(adj, second);
                if !eq(second, adj) && !is_zero(combined) {
                    let mut h = HitResult::new(1.0);
                    self.move_component(world, combined, true, false, &mut h);
                    moved = !eq(before, self.location);
                }
            }
            if !moved {
                let md = sub(hit.trace_end, hit.trace_start);
                if !is_zero(md) {
                    let mut h = HitResult::new(1.0);
                    self.move_component(world, add(adj, md), true, false, &mut h);
                    moved = !eq(before, self.location);
                }
            }
            moved
        };
        self.just_teleported |= moved;
        self.just_teleported
    }

    // ============================================================================================================
    // sliding
    // ============================================================================================================

    /// UMovementComponent::ComputeSlideVector rva=0x2fa0b80: (Delta - Normal * (Delta | Normal)) * Time
    fn base_compute_slide_vector(delta: FVector, time: f32, normal: FVector) -> FVector {
        let d = delta.y * normal.y + delta.x * normal.x + delta.z * normal.z;
        v((delta.x - normal.x * d) * time, (delta.y - normal.y * d) * time, (delta.z - normal.z * d) * time)
    }

    /// UCharacterMovementComponent::ComputeSlideVector rva=0x2f75d50 (falling -> HandleSlopeBoosting)
    pub fn compute_slide_vector(&self, delta: FVector, time: f32, normal: FVector) -> FVector {
        let r = Self::base_compute_slide_vector(delta, time, normal);
        if self.is_falling() {
            return Self::handle_slope_boosting(r, delta, time, normal);
        }
        r
    }

    /// UCharacterMovementComponent::HandleSlopeBoosting rva=0x2f7aac0
    pub fn handle_slope_boosting(slide: FVector, delta: FVector, time: f32, normal: FVector) -> FVector {
        let mut r = slide;
        if r.z > 0.0 {
            let zlimit = time * delta.z;
            if r.z - zlimit > KINDA_SMALL_NUMBER {
                if zlimit > 0.0 {
                    let up = zlimit / r.z;
                    r = v(up * r.x, up * r.y, r.z * up);
                } else {
                    r = FVector::ZERO;
                }
                let rem = v(slide.x - r.x, slide.y - r.y, 0.0);
                let n2 = safe_normal_2d(normal);
                let adj = Self::base_compute_slide_vector(rem, 1.0, n2);
                r = v(adj.x + r.x, adj.y + r.y, adj.z + r.z);
            }
        }
        r
    }

    /// UCharacterMovementComponent::SlideAlongSurface rva=0x2f8c430 -> UMovementComponent::SlideAlongSurface rva=0x2fbf540
    pub fn slide_along_surface(&mut self, world: &dyn World, delta: FVector, time: f32, in_normal: FVector, hit: &mut HitResult) -> f32 {
        if !hit.blocking_hit {
            return 0.0;
        }
        let mut normal = in_normal;
        if self.is_moving_on_ground() {
            if normal.z > 0.0 {
                if !self.is_walkable(hit) {
                    normal = safe_normal_2d(normal);
                }
            } else if normal.z < -KINDA_SMALL_NUMBER {
                if self.current_floor.floor_dist < MIN_FLOOR_DIST && self.current_floor.blocking_hit {
                    let fl = self.current_floor.hit.normal;
                    let opposed = dot(delta, fl) < 0.0 && fl.z < ONE_MINUS_DELTA;
                    if opposed {
                        normal = fl;
                    }
                    normal = safe_normal_2d(normal);
                }
            }
        }
        // UMovementComponent::SlideAlongSurface
        let old_normal = normal;
        let mut slide = self.compute_slide_vector(delta, time, normal);
        if slide.x * delta.x + slide.y * delta.y + slide.z * delta.z > 0.0 {
            self.safe_move_updated_component(world, slide, hit);
            let first = hit.time;
            let mut applied = first;
            if hit.is_valid_blocking_hit() {
                self.two_wall_adjust(&mut slide, hit, old_normal);
                if !is_nearly_zero(slide, 0.001) && slide.x * delta.x + slide.y * delta.y + slide.z * delta.z > 0.0 {
                    self.safe_move_updated_component(world, slide, hit);
                    let second = (1.0 - first) * hit.time;
                    applied = second + first;
                }
            }
            return if applied >= 0.0 { minss(applied, 1.0) } else { 0.0 };
        }
        0.0
    }

    /// UCharacterMovementComponent::TwoWallAdjust rva=0x2f90420 -> UMovementComponent::TwoWallAdjust rva=0x2fc0c30
    pub fn two_wall_adjust(&self, delta: &mut FVector, hit: &HitResult, old_normal: FVector) {
        let in_delta = *delta;
        // UMovementComponent::TwoWallAdjust
        {
            let d = *delta;
            let hn = hit.normal;
            if old_normal.x * hn.x + old_normal.y * hn.y + old_normal.z * hn.z <= 0.0 {
                let desired = d;
                let nd = safe_normal(cross(hn, old_normal));
                let s = (d.x * nd.x + d.y * nd.y + d.z * nd.z) * (1.0 - hit.time);
                let mut nd2 = v(nd.x * s, nd.y * s, nd.z * s);
                if desired.x * nd2.x + desired.y * nd2.y + desired.z * nd2.z < 0.0 {
                    nd2 = neg(nd2);
                }
                *delta = nd2;
            } else {
                let desired = d;
                let mut nd = self.compute_slide_vector(d, 1.0 - hit.time, hn);
                if nd.y * desired.y + nd.x * desired.x + nd.z * desired.z <= 0.0 {
                    nd = FVector::ZERO;
                } else if ((hn.x * old_normal.x + hn.y * old_normal.y + hn.z * old_normal.z) - 1.0).abs() < KINDA_SMALL_NUMBER {
                    nd = v(nd.x + hn.x * 0.01, nd.y + hn.y * 0.01, nd.z + hn.z * 0.01);
                }
                *delta = nd;
            }
        }
        if self.is_moving_on_ground() {
            if delta.z > 0.0 {
                if (self.c.walkable_floor_z <= hit.normal.z || self.is_walkable(hit)) && hit.normal.z > KINDA_SMALL_NUMBER {
                    let time = 1.0 - hit.time;
                    let sz = safe_normal(*delta).z * size(in_delta);
                    *delta = v(in_delta.x * time, in_delta.y * time, sz / hit.normal.z * time);
                    let msh = self.c.max_step_height;
                    if delta.z > msh {
                        let k = msh / delta.z;
                        *delta = v(k * delta.x, k * delta.y, delta.z * k);
                    }
                } else {
                    delta.z = 0.0;
                }
            } else if delta.z < 0.0 && self.current_floor.floor_dist < MIN_FLOOR_DIST && self.current_floor.blocking_hit {
                delta.z = 0.0;
            }
        }
    }

    // ============================================================================================================
    // floor
    // ============================================================================================================

    /// UCharacterMovementComponent::IsWalkable rva=0x2f7b500 (no walkable-slope overrides in the test world)
    pub fn is_walkable(&self, hit: &HitResult) -> bool {
        if !hit.is_valid_blocking_hit() {
            return false;
        }
        if hit.impact_normal.z < KINDA_SMALL_NUMBER {
            return false;
        }
        self.c.walkable_floor_z <= hit.impact_normal.z
    }

    /// UCharacterMovementComponent::IsWithinEdgeTolerance rva=0x2f7b5c0
    pub fn is_within_edge_tolerance(&self, capsule: FVector, impact: FVector, radius: f32) -> bool {
        let dy = impact.y - capsule.y;
        let dx = impact.x - capsule.x;
        let reduced = maxss(radius - SWEEP_EDGE_REJECT_DISTANCE, EDGE_REJECT_MIN);
        dy * dy + dx * dx < reduced * reduced
    }

    /// UCharacterMovementComponent::GetPerchRadiusThreshold rva=0x2f7a3e0
    pub fn perch_radius_threshold(&self) -> f32 {
        maxss(self.c.perch_radius_threshold, 0.0)
    }

    /// UCharacterMovementComponent::GetValidPerchRadius rva=0x2f7a830: Clamp(R - threshold, 0.11, R)
    pub fn valid_perch_radius(&self) -> f32 {
        let r = self.e.capsule_radius;
        let x = r - self.perch_radius_threshold();
        if x >= 0.11 {
            minss(x, r)
        } else {
            0.11
        }
    }

    /// UCharacterMovementComponent::ShouldComputePerchResult rva=0x2f8ac40
    pub fn should_compute_perch_result(&self, hit: &HitResult, check_radius: bool) -> bool {
        if !hit.is_valid_blocking_hit() {
            return false;
        }
        if !(self.perch_radius_threshold() > SWEEP_EDGE_REJECT_DISTANCE) {
            return false;
        }
        if check_radius {
            let dy = hit.impact_point.y - hit.location.y;
            let dx = hit.impact_point.x - hit.location.x;
            let r = self.valid_perch_radius();
            return r * r < dx * dx + dy * dy;
        }
        true
    }

    /// UCharacterMovementComponent::ComputePerchResult rva=0x2f75c60
    pub fn compute_perch_result(&self, world: &dyn World, test_radius: f32, hit: &HitResult, max_floor_dist: f32, out: &mut FloorResult) -> bool {
        if !(max_floor_dist > 0.0) {
            return false;
        }
        let hh = self.capsule_half_height;
        let above = maxss(hit.impact_point.z - (hit.location.z - hh), 0.0);
        let line_dist = maxss(max_floor_dist - above, 0.0);
        let sweep_dist = self.e.capsule_radius + maxss(max_floor_dist, 0.0);
        self.compute_floor_dist(world, hit.location, line_dist, sweep_dist, out, test_radius, None);
        if !out.is_walkable_floor() {
            return false;
        }
        if above + out.floor_dist > max_floor_dist {
            out.walkable_floor = false;
            return false;
        }
        true
    }

    /// UCharacterMovementComponent::FindFloor rva=0x2f77af0
    pub fn find_floor(&mut self, world: &dyn World, capsule: FVector, out: &mut FloorResult, can_use_cached: bool, downward: Option<&HitResult>) {
        let adjust = if self.is_moving_on_ground() { FLOOR_HEIGHT_CHECK_ADJUST_WALKING } else { -MAX_FLOOR_DIST };
        let sweep_dist = maxss(adjust + self.c.max_step_height, MAX_FLOOR_DIST);
        let line_dist = sweep_dist;
        let mut validate = true;
        if sweep_dist > 0.0 {
            if self.e.always_check_floor || !can_use_cached || self.force_next_floor_check || self.just_teleported {
                self.force_next_floor_check = false;
                self.compute_floor_dist(world, capsule, line_dist, sweep_dist, out, self.e.capsule_radius, downward);
            } else if self.base.is_some() {
                // static base, no forced check: reuse the current floor (0x142f77df6..0x142f77e70)
                *out = self.current_floor;
                validate = false;
            } else {
                self.force_next_floor_check = false;
                self.compute_floor_dist(world, capsule, line_dist, sweep_dist, out, self.e.capsule_radius, downward);
            }
        }
        if validate && out.blocking_hit && !out.line_trace && self.should_compute_perch_result(&out.hit, true) {
            let mut max_perch = maxss(adjust + self.c.max_step_height, MAX_FLOOR_DIST);
            if self.is_moving_on_ground() {
                max_perch = max_perch + maxss(self.c.perch_additional_height, 0.0);
            }
            let mut perch = FloorResult { hit: HitResult::new(1.0), ..Default::default() };
            if self.compute_perch_result(world, self.valid_perch_radius(), &out.hit, max_perch, &mut perch) {
                let move_up = AVG_FLOOR_DIST - out.floor_dist;
                if move_up + perch.floor_dist >= max_perch {
                    out.floor_dist = AVG_FLOOR_DIST;
                }
                if !out.walkable_floor {
                    let fd = out.floor_dist;
                    out.set_from_line_trace(&perch.hit, fd, maxss(fd, MIN_FLOOR_DIST), true);
                }
            } else {
                out.walkable_floor = false;
            }
        }
    }

    /// UCharacterMovementComponent::ComputeFloorDist rva=0x2f74fe0
    #[allow(clippy::too_many_arguments)]
    pub fn compute_floor_dist(&self, world: &dyn World, capsule: FVector, line_distance: f32, sweep_distance: f32, out: &mut FloorResult, sweep_radius: f32, downward: Option<&HitResult>) {
        out.clear();
        let (r, hh) = (self.e.capsule_radius, self.capsule_half_height);
        let mut skip_sweep = false;
        if let Some(d) = downward {
            if d.is_valid_blocking_hit() && d.trace_start.z > d.trace_end.z {
                let dy = d.trace_start.y - d.trace_end.y;
                let dx = d.trace_start.x - d.trace_end.x;
                if !(dy * dy + dx * dx > KINDA_SMALL_NUMBER) && self.is_within_edge_tolerance(d.location, d.impact_point, r) {
                    skip_sweep = true;
                    let walk = self.is_walkable(d);
                    let fd = capsule.z - d.location.z;
                    out.set_from_sweep(d, fd, walk);
                    if walk {
                        return;
                    }
                }
            }
        }
        if sweep_distance < line_distance {
            return;
        }
        if !skip_sweep && sweep_distance > 0.0 && sweep_radius > 0.0 {
            let mut shrink = (hh - r) * f32::from_bits(0x3dcc_cce0); // * (1 - 0.9f) = 0.100000024 (.rdata 0x14497e660)
            let mut trace = shrink + sweep_distance;
            let mut cap_r = sweep_radius;
            let mut cap_hh = hh - shrink;
            let mut hit = self.floor_sweep(world, capsule, trace, cap_r, cap_hh);
            if hit.blocking_hit {
                if hit.start_penetrating || !self.is_within_edge_tolerance(capsule, hit.impact_point, cap_r) {
                    cap_r = maxss(cap_r - SWEEP_EDGE_REJECT_DISTANCE - KINDA_SMALL_NUMBER, 0.0);
                    if cap_r > KINDA_SMALL_NUMBER {
                        shrink = (hh - r) * f32::from_bits(0x3f66_6666); // * (1 - 0.1f) = 0.9 (.rdata 0x14427b734)
                        trace = shrink + sweep_distance;
                        cap_hh = maxss(hh - shrink, cap_r);
                        hit = self.floor_sweep(world, capsule, trace, cap_r, cap_hh);
                    }
                }
                let max_pen = -maxss(r, MAX_FLOOR_DIST);
                let res = maxss(max_pen, hit.time * trace - shrink);
                out.set_from_sweep(&hit, res, false);
                if hit.is_valid_blocking_hit() && self.is_walkable(&hit) && !(res > sweep_distance) {
                    out.walkable_floor = true;
                    return;
                }
            }
        }
        if !out.blocking_hit && !out.hit.start_penetrating {
            out.floor_dist = sweep_distance;
            return;
        }
        if line_distance > 0.0 {
            let shrink = hh;
            let trace = shrink + line_distance;
            let end = v(capsule.x, capsule.y, -trace + capsule.z);
            if let Some(hit) = world.line_trace(capsule, end) {
                if hit.time > 0.0 {
                    let res = maxss(-maxss(r, MAX_FLOOR_DIST), hit.time * trace - shrink);
                    out.blocking_hit = true;
                    if !(res > line_distance) && self.is_walkable(&hit) {
                        let fd = out.floor_dist;
                        out.set_from_line_trace(&hit, fd, res, true);
                        return;
                    }
                }
            }
        }
        out.walkable_floor = false;
    }

    /// UCharacterMovementComponent::FloorSweepTest rva=0x2f78590 (capsule sweep; bUseFlatBaseForFloorChecks false)
    fn floor_sweep(&self, world: &dyn World, capsule: FVector, trace: f32, radius: f32, half_height: f32) -> HitResult {
        // UPseudoVehicleMovementComponent::FloorSweepTest rva=0x14f92d0 with SecondaryStepCapableComponents
        if self.pseudo.as_ref().is_some_and(|p| p.cfg.secondaries.iter().any(|s| s.step_capable)) {
            return self.pseudo_floor_sweep(world, capsule, trace, radius, half_height);
        }
        self.floor_sweep_root(world, capsule, trace, radius, half_height)
    }

    /// UCharacterMovementComponent::FloorSweepTest rva=0x2f78590 for the root capsule
    pub(crate) fn floor_sweep_root(&self, world: &dyn World, capsule: FVector, trace: f32, radius: f32, half_height: f32) -> HitResult {
        let end = v(capsule.x, capsule.y, capsule.z + -trace);
        let mut hit = world
            .sweep_capsule(capsule, end, radius, half_height)
            .into_iter()
            .find(|h| h.blocking_hit)
            .unwrap_or_else(|| HitResult::new(1.0));
        hit.trace_start = capsule;
        hit.trace_end = end;
        hit
    }

    /// UCharacterMovementComponent::AdjustFloorHeight rva=0x2f6def0
    pub fn adjust_floor_height(&mut self, world: &dyn World) {
        if !self.current_floor.is_walkable_floor() {
            return;
        }
        let mut old = self.current_floor.floor_dist;
        if self.current_floor.line_trace {
            if old < MIN_FLOOR_DIST && self.current_floor.line_dist >= MIN_FLOOR_DIST {
                return;
            }
            old = self.current_floor.line_dist;
        }
        if old < MIN_FLOOR_DIST || old > MAX_FLOOR_DIST {
            let initial_z = self.location.z;
            let move_dist = AVG_FLOOR_DIST - old;
            let mut hit = HitResult::new(1.0);
            self.safe_move_updated_component(world, v(0.0, 0.0, move_dist), &mut hit);
            if !hit.is_valid_blocking_hit() {
                self.current_floor.floor_dist = move_dist + self.current_floor.floor_dist;
            } else if move_dist > 0.0 {
                self.current_floor.floor_dist = (self.location.z - initial_z) + self.current_floor.floor_dist;
            } else {
                self.current_floor.floor_dist = self.location.z - hit.location.z;
                if self.is_walkable(&hit) {
                    let fd = self.current_floor.floor_dist;
                    self.current_floor.set_from_sweep(&hit, fd, true);
                }
            }
            self.just_teleported |= !self.e.maintain_horizontal_ground_velocity || old < 0.0;
            self.force_next_floor_check = true;
        }
    }

    // ============================================================================================================
    // walking
    // ============================================================================================================

    /// UCharacterMovementComponent::ComputeGroundMovementDelta rva=0x2f758a0
    pub fn compute_ground_movement_delta(&self, delta: FVector, ramp: &HitResult, line_trace: bool) -> FVector {
        let fnrm = ramp.impact_normal;
        if fnrm.z < NEARLY_FLAT_Z && fnrm.z > KINDA_SMALL_NUMBER && ramp.normal.z > KINDA_SMALL_NUMBER && !line_trace && self.is_walkable(ramp) {
            let fdd = delta.y * fnrm.y + delta.x * fnrm.x + delta.z * fnrm.z;
            let ramp_move = v(delta.x, delta.y, -(fdd / fnrm.z));
            if self.e.maintain_horizontal_ground_velocity {
                return ramp_move;
            }
            let s = size(delta);
            return mul(safe_normal(ramp_move), s);
        }
        delta
    }

    /// UCharacterMovementComponent::CanStepUp rva=0x2f71c40
    pub fn can_step_up(&self, hit: &HitResult) -> bool {
        if !hit.is_valid_blocking_hit() || self.mode == Mode::Falling {
            return false;
        }
        hit.component.is_none() || hit.can_step_up
    }

    /// UCharacterMovementComponent::MoveAlongFloor rva=0x2f7bef0
    pub fn move_along_floor(&mut self, world: &dyn World, in_velocity: FVector, dt: f32, step_down: &mut StepDownResult) {
        if !self.current_floor.is_walkable_floor() {
            return;
        }
        let delta = v(dt * in_velocity.x, in_velocity.y * dt, 0.0);
        let mut hit = HitResult::new(1.0);
        let floor_hit = self.current_floor.hit;
        let mut ramp = self.compute_ground_movement_delta(delta, &floor_hit, self.current_floor.line_trace);
        self.safe_move_updated_component(world, ramp, &mut hit);
        let mut last_slice = dt;
        if hit.start_penetrating {
            let n = hit.normal;
            self.slide_along_surface(world, delta, 1.0, n, &mut hit);
        } else if hit.is_valid_blocking_hit() {
            let mut percent = hit.time;
            if hit.time > 0.0 && hit.normal.z > KINDA_SMALL_NUMBER && self.is_walkable(&hit) {
                let rem = 1.0 - percent;
                let d2 = v(delta.x * rem, delta.y * rem, delta.z * rem);
                let h0 = hit;
                ramp = self.compute_ground_movement_delta(d2, &h0, false);
                last_slice = rem * dt;
                self.safe_move_updated_component(world, ramp, &mut hit);
                let p = rem * hit.time + percent;
                percent = if p >= 0.0 { minss(p, 1.0) } else { 0.0 };
            }
            if hit.is_valid_blocking_hit() {
                let on_hit_base = self.base.is_some() && hit.component.is_some() && self.base == hit.component;
                if self.can_step_up(&hit) || on_hit_base {
                    let grav = v(0.0, 0.0, -1.0);
                    let rem = 1.0 - percent;
                    let d3 = v(delta.x * rem, delta.y * rem, delta.z * rem);
                    let h0 = hit;
                    if !self.step_up(world, grav, d3, &h0, Some(step_down)) {
                        let n = hit.normal;
                        let _ = last_slice;
                        self.slide_along_surface(world, delta, rem, n, &mut hit);
                    }
                } else if hit.component.is_some() && !hit.can_step_up {
                    let n = hit.normal;
                    self.slide_along_surface(world, delta, 1.0 - percent, n, &mut hit);
                }
            }
        }
    }

    /// UCharacterMovementComponent::StepUp rva=0x2f8f400 (scoped movement: a revert restores the location)
    pub fn step_up(&mut self, world: &dyn World, grav: FVector, delta: FVector, in_hit: &HitResult, out: Option<&mut StepDownResult>) -> bool {
        if !self.can_step_up(in_hit) || !(self.c.max_step_height > 0.0) {
            return false;
        }
        let old = self.location;
        let (r, hh) = (self.e.capsule_radius, self.capsule_half_height);
        let initial_impact_z = in_hit.impact_point.z;
        if initial_impact_z > (hh - r) + old.z {
            return false;
        }
        if is_zero(grav) {
            return false;
        }
        let mut up_h = self.c.max_step_height;
        let mut down_h = up_h;
        let step_side_z = in_hit.impact_normal.y * grav.y + in_hit.impact_normal.x * grav.x + in_hit.impact_normal.z * grav.z;
        let mut base_z = old.z - hh;
        let mut floor_point_z = base_z;
        if self.is_moving_on_ground() && self.current_floor.is_walkable_floor() {
            let fd = maxss(self.current_floor.distance_to_floor(), 0.0);
            base_z = base_z - fd;
            up_h = maxss(up_h - fd, 0.0);
            down_h = self.c.max_step_height + MAX_FLOOR_DIST * 2.0; // 4.8 (.rdata 0x14497e684)
            let vertical_face = !self.is_within_edge_tolerance(in_hit.location, in_hit.impact_point, r);
            if !self.current_floor.line_trace && !vertical_face {
                floor_point_z = self.current_floor.hit.impact_point.z;
            } else {
                floor_point_z = floor_point_z - self.current_floor.floor_dist;
            }
        }
        if initial_impact_z <= base_z {
            return false;
        }
        let saved = (self.location, self.just_teleported);
        let revert = |m: &mut Self| {
            m.location = saved.0;
            false
        };
        // step up: MoveUpdatedComponent(-GravDir * StepTravelUpHeight)
        let mut up_hit = HitResult::new(1.0);
        let up = v(-(up_h * grav.x), -(up_h * grav.y), -(up_h * grav.z));
        self.move_updated_component(world, up, true, &mut up_hit);
        if up_hit.start_penetrating {
            return revert(self);
        }
        // step forward
        let mut hit = HitResult::new(1.0);
        self.move_updated_component(world, delta, true, &mut hit);
        if hit.blocking_hit {
            if hit.start_penetrating {
                return revert(self);
            }
            if self.is_falling() {
                return true;
            }
            let fwd_time = hit.time;
            let n = hit.normal;
            let slide = self.slide_along_surface(world, delta, 1.0 - fwd_time, n, &mut hit);
            if self.is_falling() {
                return revert(self);
            }
            if fwd_time == 0.0 && slide == 0.0 {
                return revert(self);
            }
        }
        // step down
        let down = v(down_h * grav.x, down_h * grav.y, down_h * grav.z);
        self.move_updated_component(world, down, true, &mut hit);
        if hit.start_penetrating {
            return revert(self);
        }
        let mut sd = StepDownResult::default();
        if hit.is_valid_blocking_hit() {
            let dz = hit.impact_point.z - floor_point_z;
            if dz > self.c.max_step_height {
                return revert(self);
            }
            if !self.is_walkable(&hit) {
                let towards = delta.x * hit.impact_normal.x + delta.y * hit.impact_normal.y + delta.z * hit.impact_normal.z < 0.0;
                if towards {
                    return revert(self);
                }
                if hit.location.z > old.z {
                    return revert(self);
                }
            }
            if !self.is_within_edge_tolerance(hit.location, hit.impact_point, r) {
                return revert(self);
            }
            if dz > 0.0 && !self.can_step_up(&hit) {
                return revert(self);
            }
            if out.is_some() {
                let loc = self.location;
                let h0 = hit;
                let mut fr = FloorResult { hit: HitResult::new(1.0), ..Default::default() };
                self.find_floor(world, loc, &mut fr, false, Some(&h0));
                sd.floor = fr;
                if hit.location.z > old.z && !fr.blocking_hit && step_side_z > -MAX_STEP_SIDE_Z {
                    return revert(self);
                }
                sd.computed_floor = true;
            }
        }
        if let Some(o) = out {
            *o = sd;
        }
        self.just_teleported |= !self.e.maintain_horizontal_ground_velocity;
        true
    }

    /// UCharacterMovementComponent::PhysWalking rva=0x2f81730
    pub fn phys_walking(&mut self, world: &dyn World, dt: f32, mut iterations: i32) {
        if dt < MIN_TICK_TIME {
            return;
        }
        self.just_teleported = false;
        let mut checked_fall = false;
        let mut remaining = dt;
        while remaining >= MIN_TICK_TIME && iterations < self.e.max_simulation_iterations {
            self.just_teleported = false;
            iterations += 1;
            let tick = self.sim_time_step(remaining, iterations);
            remaining = remaining - tick;
            let old_base = self.base;
            let old_location = self.location;
            let old_floor = self.current_floor;
            self.maintain_horizontal_ground_velocity();
            let old_velocity = self.velocity;
            self.acceleration.z = 0.0;
            let bd = self.get_max_braking_deceleration();
            let gf = self.c.ground_friction;
            self.calc_velocity(tick, gf, bd);
            if self.is_falling() {
                // the pawn decided to jump up (0x142f82384): StartNewPhysics(remaining + tick, iterations - 1)
                self.start_new_physics(world, remaining + tick, iterations - 1);
                return;
            }
            let move_velocity = self.velocity;
            let delta = v(move_velocity.x * tick, move_velocity.y * tick, move_velocity.z * tick);
            let zero_delta = is_nearly_zero(delta, KINDA_SMALL_NUMBER);
            let mut step_down = StepDownResult::default();
            if zero_delta {
                remaining = 0.0;
            } else {
                self.move_along_floor(world, move_velocity, tick, &mut step_down);
                if self.is_falling() {
                    // the pawn decided to jump up (0x142f82262)
                    let desired = size(delta);
                    if desired > KINDA_SMALL_NUMBER {
                        let d = sub(self.location, old_location);
                        let actual = (d.y * d.y + d.x * d.x).sqrt();
                        let k = minss(actual * (1.0 / desired), 1.0);
                        remaining = (1.0 - k) * tick + remaining;
                    }
                    self.start_new_physics(world, remaining, iterations);
                    return;
                }
            }
            if step_down.computed_floor {
                self.current_floor = step_down.floor;
            } else {
                let loc = self.location;
                let mut floor = self.current_floor;
                self.find_floor(world, loc, &mut floor, zero_delta, None);
                self.current_floor = floor;
            }
            // ledges: CanWalkOffLedges (rva=0x2f71d00): bCanWalkOffLedges && (bCanWalkOffLedgesWhenCrouching || !crouched)
            let check_ledges = !(self.e.can_walk_off_ledges && (self.c.can_walk_off_ledges_when_crouching || !self.is_crouched));
            if check_ledges && !self.current_floor.is_walkable_floor() {
                // GetLedgeMove / RevertMove path (0x142f81cdb..): not ported (this pawn always walks off ledges)
            } else if self.current_floor.is_walkable_floor() {
                // ShouldCatchAir (ICF to a return-false stub at rva=0x7bf520); AdjustFloorHeight; SetBase
                self.adjust_floor_height(world);
                self.base = self.current_floor.hit.component;
            } else if self.current_floor.hit.start_penetrating && remaining <= 0.0 {
                let mut h = self.current_floor.hit;
                h.trace_end = add(h.trace_start, v(0.0, 0.0, MAX_FLOOR_DIST));
                let adj = self.get_penetration_adjustment(&h);
                self.resolve_penetration(world, adj, &h);
                self.force_next_floor_check = true;
            }
            if !self.current_floor.is_walkable_floor() && !self.current_floor.hit.start_penetrating {
                let must_jump = self.just_teleported || zero_delta || old_base.is_none();
                if (must_jump || !checked_fall) && self.check_fall(world, &old_floor, remaining, tick, delta, old_location, iterations, must_jump) {
                    return;
                }
                checked_fall = true;
            }
            if self.is_moving_on_ground() {
                if !self.just_teleported && tick >= MIN_TICK_TIME {
                    let inv = 1.0 / tick;
                    let d = sub(self.location, old_location);
                    self.velocity = v(d.x * inv, d.y * inv, d.z * inv);
                    self.maintain_horizontal_ground_velocity();
                }
            }
            let _ = old_velocity;
            if eq(self.location, old_location) {
                break; // didn't move this iteration: later ones would not either (0x142f820a9)
            }
        }
        if self.is_moving_on_ground() {
            self.maintain_horizontal_ground_velocity();
        }
    }

    /// UCharacterMovementComponent::CheckFall rva=0x2f72260
    #[allow(clippy::too_many_arguments)]
    fn check_fall(&mut self, world: &dyn World, _old_floor: &FloorResult, remaining: f32, tick: f32, delta: FVector, sub_loc: FVector, iterations: i32, must_jump: bool) -> bool {
        let can_walk_off = self.e.can_walk_off_ledges && (self.c.can_walk_off_ledges_when_crouching || !self.is_crouched);
        if must_jump || can_walk_off {
            // HandleWalkingOffLedge rva=0x2f7acf0: owner notification only
            if self.is_moving_on_ground() {
                self.start_falling(world, iterations, remaining, tick, delta, sub_loc);
            }
            return true;
        }
        false
    }

    /// UCharacterMovementComponent::StartFalling rva=0x2f8ebd0
    fn start_falling(&mut self, world: &dyn World, iterations: i32, remaining: f32, tick: f32, delta: FVector, sub_loc: FVector) {
        let desired = size(delta);
        let d = sub(self.location, sub_loc);
        let actual = (d.x * d.x + d.y * d.y).sqrt();
        let remaining = if desired < KINDA_SMALL_NUMBER { 0.0 } else { remaining + (1.0 - minss(actual * (1.0 / desired), 1.0)) * tick };
        if self.is_moving_on_ground() {
            self.set_movement_mode(world, Mode::Falling);
        }
        self.start_new_physics(world, remaining, iterations);
    }

    // ============================================================================================================
    // falling
    // ============================================================================================================

    /// UCharacterMovementComponent::IsValidLandingSpot rva=0x2f7b3a0
    pub fn is_valid_landing_spot(&mut self, world: &dyn World, capsule: FVector, hit: &HitResult) -> bool {
        if !hit.blocking_hit {
            return false;
        }
        if !hit.start_penetrating {
            if !self.is_walkable(hit) {
                return false;
            }
            let (r, hh) = (self.e.capsule_radius, self.capsule_half_height);
            let lower = hit.location.z - hh + r;
            if hit.impact_point.z >= lower {
                return false;
            }
            if !self.is_within_edge_tolerance(hit.location, hit.impact_point, r) {
                return false;
            }
        } else if hit.normal.z < KINDA_SMALL_NUMBER {
            return false;
        }
        let mut fr = FloorResult { hit: HitResult::new(1.0), ..Default::default() };
        self.find_floor(world, capsule, &mut fr, false, Some(hit));
        fr.is_walkable_floor()
    }

    /// UCharacterMovementComponent::ShouldCheckForValidLandingSpot rva=0x2f8ab10
    pub fn should_check_for_valid_landing_spot(&self, hit: &HitResult) -> bool {
        if hit.normal.z > KINDA_SMALL_NUMBER && !equals(hit.normal, hit.impact_normal, KINDA_SMALL_NUMBER) {
            return self.is_within_edge_tolerance(self.location, hit.impact_point, self.e.capsule_radius);
        }
        false
    }

    /// UCharacterMovementComponent::LimitAirControl rva=0x2f7b990
    pub fn limit_air_control(&mut self, world: &dyn World, accel: FVector, hit: &HitResult, check_landing: bool) -> FVector {
        let mut r = accel;
        if hit.is_valid_blocking_hit() && hit.normal.z > VERTICAL_SLOPE_NORMAL_Z {
            if !check_landing || !self.is_valid_landing_spot(world, hit.location, hit) {
                if dot(accel, hit.normal) < 0.0 {
                    let n2 = safe_normal_2d(hit.normal);
                    r = plane_project(accel, n2);
                }
            }
        } else if hit.start_penetrating {
            return if dot(r, hit.normal) > 0.0 { r } else { FVector::ZERO };
        }
        r
    }

    /// UCharacterMovementComponent::ProcessLanded rva=0x2f83670 -> ACharacter::Landed (AAdvancedCharacter::Landed
    /// rva=0x1489820) -> SetPostLandedPhysics rva=0x2f8a140 (SetMovementMode(Walking)) -> StartNewPhysics
    fn process_landed(&mut self, world: &dyn World, remaining: f32, iterations: i32) {
        self.landed();
        if self.is_falling() {
            self.set_movement_mode(world, Mode::Walking);
        }
        self.start_new_physics(world, remaining, iterations);
    }

    /// UCharacterMovementComponent::PhysFalling rva=0x2f7ef00
    pub fn phys_falling(&mut self, world: &dyn World, dt: f32, mut iterations: i32) {
        if dt < MIN_TICK_TIME {
            return;
        }
        let mut fall_accel = self.get_falling_lateral_acceleration();
        fall_accel.z = 0.0;
        let limited_air_control = fall_accel.x * fall_accel.x + fall_accel.y * fall_accel.y > 0.0; // ShouldLimitAirControl
        let mut remaining = dt;
        while remaining >= MIN_TICK_TIME && iterations < self.e.max_simulation_iterations {
            let iter_before = iterations;
            iterations += 1;
            let mut tick = self.sim_time_step(remaining, iterations);
            remaining = remaining - tick;
            let old_location = self.location;
            self.just_teleported = false;
            let old_velocity = self.velocity;
            let max_decel = self.get_max_braking_deceleration();
            {
                let saved = self.acceleration;
                self.acceleration = fall_accel;
                self.velocity.z = 0.0;
                let f = self.c.falling_lateral_friction;
                self.calc_velocity(tick, f, max_decel);
                self.velocity.z = old_velocity.z;
                self.acceleration = saved;
            }
            let gravity = v(0.0, 0.0, self.gravity_z());
            let mut gravity_time = tick;
            let mut ending_jump_force = false;
            if self.jump_force_time_remaining > 0.0 {
                let jt = minss(self.jump_force_time_remaining, tick);
                if !self.e.apply_gravity_while_jumping {
                    gravity_time = maxss(tick - jt, 0.0);
                }
                self.jump_force_time_remaining = self.jump_force_time_remaining - jt;
                if !(self.jump_force_time_remaining > 0.0) {
                    self.jump_force_time_remaining = 0.0; // ResetJumpState (vcall +0x7b0)
                    ending_jump_force = true;
                }
            }
            self.velocity = self.new_fall_velocity(self.velocity, gravity, gravity_time);
            // apex substep (CharacterMovementCVars::ForceJumpPeakSubstep, default on: 0x142f7f215)
            if old_velocity.z > 0.0 && !(self.velocity.z > 0.0) && self.num_jump_apex_attempts < self.e.max_jump_apex_attempts {
                let inv = 1.0 / tick;
                let da = v((self.velocity.x - old_velocity.x) * inv, (self.velocity.y - old_velocity.y) * inv, (self.velocity.z - old_velocity.z) * inv);
                if da.z.abs() > SMALL_NUMBER {
                    let tta = -(old_velocity.z / da.z);
                    if tta >= KINDA_SMALL_NUMBER && tta < tick {
                        self.velocity = v(da.x * tta + old_velocity.x, da.y * tta + old_velocity.y, 0.0);
                        remaining = (tick - tta) + remaining;
                        tick = tta;
                        iterations = iter_before;
                        self.num_jump_apex_attempts += 1;
                    }
                }
            }
            // midpoint integration: 0.5 * (OldVelocity + Velocity) * timeTick
            let mut adjusted = v(
                (old_velocity.x + self.velocity.x) * 0.5 * tick,
                (old_velocity.y + self.velocity.y) * 0.5 * tick,
                (old_velocity.z + self.velocity.z) * 0.5 * tick,
            );
            if ending_jump_force && !self.e.apply_gravity_while_jumping {
                let ngt = maxss(tick - gravity_time, 0.0);
                adjusted = v(
                    ngt * old_velocity.x + (old_velocity.x + self.velocity.x) * 0.5 * gravity_time,
                    ngt * old_velocity.y + (old_velocity.y + self.velocity.y) * 0.5 * gravity_time,
                    ngt * old_velocity.z + (old_velocity.z + self.velocity.z) * 0.5 * gravity_time,
                );
            }
            let mut hit = HitResult::new(1.0);
            self.safe_move_updated_component(world, adjusted, &mut hit);
            let mut last_slice = tick;
            let mut sub_remaining = (1.0 - hit.time) * tick;
            if hit.blocking_hit {
                let loc = self.location;
                if self.is_valid_landing_spot(world, loc, &hit) {
                    remaining = sub_remaining + remaining;
                    self.process_landed(world, remaining, iterations);
                    return;
                }
                adjusted = v(tick * self.velocity.x, tick * self.velocity.y, tick * self.velocity.z);
                if !hit.start_penetrating && self.should_check_for_valid_landing_spot(&hit) {
                    let loc = self.location;
                    let mut fr = FloorResult { hit: HitResult::new(1.0), ..Default::default() };
                    self.find_floor(world, loc, &mut fr, false, None);
                    let fh = fr.hit;
                    if fr.is_walkable_floor() && self.is_valid_landing_spot(world, loc, &fh) {
                        remaining = sub_remaining + remaining;
                        self.process_landed(world, remaining, iterations);
                        return;
                    }
                }
                if !self.is_falling() {
                    return;
                }
                let mut vel_no_air = old_velocity;
                let mut air_accel = self.acceleration;
                if limited_air_control {
                    let (sv, sa) = (self.velocity, self.acceleration);
                    self.acceleration = FVector::ZERO;
                    self.velocity = old_velocity;
                    self.velocity.z = 0.0;
                    let f = self.c.falling_lateral_friction;
                    self.calc_velocity(tick, f, max_decel);
                    vel_no_air = v(self.velocity.x, self.velocity.y, old_velocity.z);
                    vel_no_air = self.new_fall_velocity(vel_no_air, gravity, gravity_time);
                    self.velocity = sv;
                    self.acceleration = sa;
                    let inv = 1.0 / tick;
                    air_accel = v((self.velocity.x - vel_no_air.x) * inv, (self.velocity.y - vel_no_air.y) * inv, (self.velocity.z - vel_no_air.z) * inv);
                    let lim = self.limit_air_control(world, air_accel, &hit, false);
                    let dv = mul(lim, last_slice);
                    adjusted = v((vel_no_air.x + dv.x) * last_slice, (vel_no_air.y + dv.y) * last_slice, (vel_no_air.z + dv.z) * last_slice);
                }
                let old_hit_normal = hit.normal;
                let old_hit_impact_normal = hit.impact_normal;
                let mut delta = self.compute_slide_vector(adjusted, 1.0 - hit.time, old_hit_normal);
                if sub_remaining > KINDA_SMALL_NUMBER && !self.just_teleported {
                    let inv = 1.0 / sub_remaining;
                    self.velocity = v(delta.x * inv, delta.y * inv, delta.z * inv);
                }
                if sub_remaining > KINDA_SMALL_NUMBER && dot(delta, adjusted) > 0.0 {
                    self.safe_move_updated_component(world, delta, &mut hit);
                    if hit.blocking_hit {
                        last_slice = sub_remaining;
                        sub_remaining = (1.0 - hit.time) * sub_remaining;
                        let loc = self.location;
                        if self.is_valid_landing_spot(world, loc, &hit) {
                            remaining = sub_remaining + remaining;
                            self.process_landed(world, remaining, iterations);
                            return;
                        }
                        if !self.is_falling() {
                            return;
                        }
                        if limited_air_control && hit.normal.z > VERTICAL_SLOPE_NORMAL_Z {
                            let lm = mul(vel_no_air, last_slice);
                            delta = self.compute_slide_vector(lm, 1.0, old_hit_normal);
                        }
                        self.two_wall_adjust(&mut delta, &hit, old_hit_normal);
                        if limited_air_control {
                            let lim = self.limit_air_control(world, air_accel, &hit, false);
                            let dv = mul(lim, sub_remaining);
                            if dot(dv, old_hit_normal) > 0.0 {
                                delta = add(delta, mul(dv, sub_remaining));
                            }
                        }
                        if sub_remaining > KINDA_SMALL_NUMBER && !self.just_teleported {
                            let inv = 1.0 / sub_remaining;
                            self.velocity = v(delta.x * inv, delta.y * inv, delta.z * inv);
                        }
                        let ditch = old_hit_impact_normal.z > 0.0
                            && hit.impact_normal.z > 0.0
                            && delta.z.abs() <= KINDA_SMALL_NUMBER
                            && dot(hit.impact_normal, old_hit_impact_normal) < 0.0;
                        self.safe_move_updated_component(world, delta, &mut hit);
                        if hit.time == 0.0 {
                            let mut side = safe_normal_2d(add(old_hit_normal, hit.impact_normal));
                            if is_nearly_zero(side, KINDA_SMALL_NUMBER) {
                                side = safe_normal(v(old_hit_normal.y, -old_hit_normal.x, 0.0));
                            }
                            self.safe_move_updated_component(world, side, &mut hit);
                        }
                        let loc = self.location;
                        if ditch || self.is_valid_landing_spot(world, loc, &hit) || hit.time == 0.0 {
                            self.process_landed(world, 0.0, iterations);
                            return;
                        } else if self.perch_radius_threshold() > 0.0 && hit.time == 1.0 && old_hit_impact_normal.z >= self.c.walkable_floor_z {
                            let pl = self.location;
                            let zm = (pl.z - old_location.z).abs();
                            let d = sub(pl, old_location);
                            let m2 = d.x * d.x + d.y * d.y;
                            if zm <= 0.2 * tick && m2 <= 4.0 * tick {
                                let ms = self.get_max_speed();
                                let rx = self.random.frand();
                                self.velocity.x = self.velocity.x + 0.25 * ms * (rx - 0.5);
                                let ry = self.random.frand();
                                self.velocity.y = self.velocity.y + 0.25 * ms * (ry - 0.5);
                                self.velocity.z = maxss(self.c.jump_z_velocity * 0.25, 1.0);
                                let dd = mul(self.velocity, tick);
                                self.safe_move_updated_component(world, dd, &mut hit);
                            }
                        }
                    }
                }
            }
            if self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y <= KINDA_SMALL_X10 {
                self.velocity.x = 0.0;
                self.velocity.y = 0.0;
            }
        }
    }
}

/// The hit selection of UPrimitiveComponent::MoveComponentImpl rva=0x2fb4250 (the doc of `ExeMovement::move_component`):
/// every hit pulled back by max(0.1 / |Delta|, min(1 / |Delta|, 0.1)) + 0.001 (PullBackHit), the first blocking hit or
/// the start-penetrating one most opposed to the move, ShouldIgnoreHitResult (unnamed code at 0x2fbf140 called from
/// `MoveComponentImpl` at 0x142fb496d) dropping initial overlaps the move leaves unless `never_ignore_overlaps`. Returns
/// the hit (TraceStart / TraceEnd set) and the new location (snapped to the start within MIN_MOVEMENT_DIST_SQ, Time 0).
/// `delta` must be longer than MIN_MOVEMENT_DIST_SQ (the caller's early return).
pub(crate) fn select_move_hit(mut hits: Vec<HitResult>, start: FVector, delta: FVector, never_ignore_overlaps: bool) -> Option<(HitResult, FVector)> {
    let end = add(start, delta);
    let dsq = size_sq(delta);
    if !hits.is_empty() {
        let inv = 1.0 / dsq.sqrt();
        let lo = inv * 0.1;
        let mut back = if lo > 0.1 { lo } else { minss(inv, 0.1) };
        back = back + 0.001;
        for h in hits.iter_mut() {
            let t = h.time - back;
            h.time = if t >= 0.0 { minss(t, 1.0) } else { 0.0 };
        }
    }
    let move_dir = safe_normal(delta);
    let mut chosen: Option<usize> = None;
    let mut best = f32::from_bits(0x7f7f_c99e); // 3.39999995e+38 (.rdata 0x14449f8dc)
    for (i, h) in hits.iter().enumerate() {
        if !h.blocking_hit {
            continue;
        }
        // ShouldIgnoreHitResult: (Distance < HitDistanceTolerance(0) or bStartPenetrating) and not
        // NeverIgnoreBlockingOverlaps -> ignore when moving out
        if (h.distance < 0.0 || h.start_penetrating) && !never_ignore_overlaps {
            let md = move_dir.y * h.impact_normal.y + move_dir.x * h.impact_normal.x + move_dir.z * h.impact_normal.z;
            if md > 0.0 {
                continue;
            }
        }
        if h.start_penetrating {
            let nd = h.impact_normal.y * delta.y + h.impact_normal.x * delta.x + h.impact_normal.z * delta.z;
            if nd < best {
                best = nd;
                chosen = Some(i);
            }
        } else if chosen.is_none() {
            chosen = Some(i);
            break;
        }
    }
    let i = chosen?;
    let mut out = hits[i];
    out.trace_start = start;
    out.trace_end = end;
    let t = out.time;
    let mut nl = v((end.x - start.x) * t + start.x, (end.y - start.y) * t + start.y, (end.z - start.z) * t + start.z);
    if size_sq(sub(nl, start)) <= MIN_MOVEMENT_DIST_SQ {
        nl = start;
        out.time = 0.0;
    }
    Some((out, nl))
}

/// RotationRate.Yaw: -1 from the UAdvancedCharacterMovement ctor rva=0x144da20 (.rdata 0x144014bb8 loaded at
/// 0x14144da2e into RotationRate (0, -1, 0), +0x1e4); no BP of a character, horse, catapult or ballista overrides it
pub const ROTATION_RATE_YAW: f32 = -1.0;

/// UCharacterMovementComponent::GetDeltaRotation rva=0x2f79430's yaw (read off the disassembly at 0x142f79458..):
/// RotationRate.Yaw >= 0 ? min(Yaw * dt, 360) : 360 - with Mordhau's -1, 360: PhysicsRotation's FixedTurn then lands
/// on the desired yaw within the frame
pub fn delta_rotation_yaw(dt: f32) -> f32 {
    let r = ROTATION_RATE_YAW;
    if r >= 0.0 {
        let x = r * dt;
        if x < 360.0 {
            x
        } else {
            360.0
        }
    } else {
        360.0
    }
}

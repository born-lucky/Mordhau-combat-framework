//! reference_compat: the GDScript reference reproduced bit for bit (f64 scalars, Godot axes, Godot Vector3 semantics,
//! flat-floor host). The exe-exact port is crate::exe. UMordhauMovementComponent (Mordhau) on top of
//! UCharacterMovementComponent (UE 4.26), ported from the GDScript
//! reference godot/game/character/mordhau_movement.gd (MordhauMovement), which ports the decomp. Pure model: the host
//! feeds input and floor contact. UE units (cm, cm/s, degrees) on Godot axes (+Y up, lateral = XZ), as the reference.
//!
//! Sources, in the order a value is looked for (records.rs):
//!  1. BP_MordhauCharacter's CharMoveComp subobject (class MordhauMovementComponent): every value the Blueprint overrides.
//!  2. Native constructors (Ghidra C in extract/native/decomp): UMordhauMovementComponent::UMordhauMovementComponent
//!     rva=0x14af4f0 and the engine's UCharacterMovementComponent ctor (see EngineCtor below).
//!  3. Config: Mordhau/Config/DefaultEngine.ini [/Script/Engine.PhysicsSettings].
//!
//! Numeric model: the reference's (ue.rs): f64 scalars, f32 vectors, scalars cast to f32 when they scale a vector.

use crate::records::{CharacterRecords, Character, MoveExtra, Movement};
use crate::ue::{angle_to, clampf, maxf, minf, rad_to_deg, FVector, V3Ext};

/// EMovementMode values as UMordhauMovementComponent::GetMaxSpeed switches on them (byte at +0x168):
/// 1 Walking, 2 NavWalking, 3 Falling (UE 4.26 EMovementMode order: None, Walking, NavWalking, Falling, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    None = 0,
    Walking = 1,
    Falling = 3,
}

/// Sprint state byte at +0xd18, written by UMordhauMovementComponent::LODTick (rva=0x14c4d60) and read by
/// GetMaxSpeed/GetMaxAcceleration. Names are the reference's; numbers and their effects are from the decompile.
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

// --- native defaults not overridden by the Blueprint ---------------------------------------------------------------
// UMordhauMovementComponent ctor rva=0x14af4f0: +0xcd4 = 0x3f733333 (0.95) is the GetMaxSpeed factor for state 1;
// +0xcf8, +0xcfc, +0xcf4 = 1.0 (extra speed multipliers); +0xd04/+0xd08 = 1.0 (armor speed/accel factors, reset to
// 1 minus armor by UpdateArmorSpeedAndAcceleration rva=0x14dc0c0). +0xcf0 = 1330 is the state-7 acceleration.
// (The reference keeps 0.95 as a GDScript float, i.e. the f64 0.95, not the f32 0x3f733333: kept for parity.)
pub const NATIVE_SIDEWAYS_MODIFIER: f64 = 0.95; // +0xcd4 StrafeModifier (PDB name)
pub const NATIVE_SUPERSPRINT_ACCELERATION: f64 = 1330.0; // +0xcf0 SupersprintAcceleration
// .rdata floats read from the exe at the addresses LODTick / GetMaxBrakingDeceleration load them from:
pub const RDATA_FORWARD_CONE_DEG: f64 = 56.25; // DAT_14433116c: |move yaw - actor yaw| <= this counts as forward
pub const RDATA_BACKPEDAL_DEG: f64 = 101.25; // DAT_144331174: beyond this it is backpedal, between is sideways
pub const RDATA_SPRINT_SPEED_SLACK: f64 = 5.0; // DAT_143fe4e20: cm/s slack in the "fast enough to sprint" tests
pub const RDATA_FALL_TOO_FAST: f64 = 1.01; // DAT_1442efa68: GetMaxBrakingDeceleration falling-too-fast factor

/// UCharacterMovementComponent (engine). Engine code is statically linked into Mordhau-Win64-Shipping.exe; values are
/// the immediates the engine ctor UCharacterMovementComponent::UCharacterMovementComponent (rva=0x2f6cc10, called
/// first by UAdvancedCharacterMovement's ctor) stores, read by disassembling it at its PDB address (labels.tsv).
/// Offsets match the reads in UMordhauMovementComponent::GetMaxBrakingDeceleration.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineCtor {
    pub braking_deceleration_walking: f64, // +0x1b4 = 0x45000000 (UE sets it = MaxAcceleration 2048 at construction)
    pub braking_friction: f64,             // +0x1ac not written -> 0
    pub b_use_separate_braking_friction: bool, // bitfield at +0x1f0 bit 0 not set by the ctor
    pub braking_sub_step_time: f64,        // +0x1b0 = 0x3cf83e10 (the reference writes 1.0 / 33.0)
    pub min_analog_walk_speed: f64,        // +0x1a4 not written -> 0
    pub max_simulation_time_step: f64,     // +0x29c = 0x3d4ccccd (the reference writes 0.05)
    pub max_simulation_iterations: i64,    // +0x2a0 = 8
}

impl Default for EngineCtor {
    fn default() -> Self {
        EngineCtor {
            braking_deceleration_walking: 2048.0,
            braking_friction: 0.0,
            b_use_separate_braking_friction: false,
            braking_sub_step_time: 1.0 / 33.0,
            min_analog_walk_speed: 0.0,
            max_simulation_time_step: 0.05,
            max_simulation_iterations: 8,
        }
    }
}

// Engine constants from the same disassembly (.rdata loads):
pub const ENGINE_MIN_TICK_TIME: f64 = 1e-6; // 0x144000104, CalcVelocity / ApplyVelocityBraking early-out
pub const ENGINE_OVER_VELOCITY_PERCENT: f64 = 1.01; // 0x1442efa68, UMovementComponent::IsExceedingMaxSpeed rva=0x2fb3720
pub const ENGINE_BRAKE_TO_STOP_SQ: f64 = 100.0; // 0x143fe4e40, ApplyVelocityBraking: stop below 10 cm/s
pub const ENGINE_KINDA_SMALL: f64 = 1e-4; // 0x144022350
pub const ENGINE_BRAKING_STEP_MIN: f64 = 1.0 / 75.0; // 0x14497e65c, clamp of BrakingSubStepTime
pub const ENGINE_BRAKING_STEP_MAX: f64 = 0.05; // 0x144014a9c

/// MordhauMovement: the movement component + the AAdvancedCharacter / AMordhauCharacter fields its rules read.
#[derive(Clone, Debug)]
pub struct MordhauMovement {
    pub engine: EngineCtor,
    /// CharMoveComp values (this pawn's copy: the Rat perk changes MaxWalkSpeedCrouched, Knockback swaps GroundFriction /
    /// FallingLateralFriction)
    pub cfg: Movement,
    /// fall damage, knockback and NavAgentProps terms (native ctors + CharMoveComp)
    pub fx: MoveExtra,
    /// BP_MordhauCharacter class-default terms the movement rules read
    pub ch: Character,
    pub gravity_z: f64,
    pub terminal_velocity: f64,
    /// BP_MordhauCharacter CDO JumpCooldown (AAdvancedCharacter +0x864), used by AAdvancedCharacter::CanJumpInternal_Implementation
    pub jump_cooldown: f64,

    // --- state ---
    pub mode: Mode,
    pub velocity: FVector,     // cm/s
    pub acceleration: FVector, // cm/s^2, from input (UE: Acceleration)
    pub crouched: bool,
    pub wants_sprint: bool, // +0xd19 bWantsToSprint (AMordhauCharacter::SprintPressed rva=0x156da50)
    pub sprint_state: Sprint,
    pub sprint_time: f64, // +0xce8, LODTick accumulator, 0 below state 4
    pub world_time: f64,
    /// AAdvancedCharacter +0x85c LastLand: zero-filled (neither ctor writes it, not in the BP CDO), first written by
    /// AAdvancedCharacter::Landed at 0x141489862 - so no jump in the first JumpCooldown seconds of world time, as in the exe
    pub last_landed_time: f64,

    // --- character-level state the movement rules read (AAdvancedCharacter / AMordhauCharacter fields) ---
    pub authority: bool,         // Owner Role == ROLE_Authority (3): CheckFallDamage, Knockback, LODTick falling time
    pub player_controlled: bool, // APawn::IsPlayerControlled (vcall +0x6c0 in AMordhauCharacter::LODTick)
    pub dead: bool,              // AAdvancedCharacter bIsDead
    pub b_ignore_movement_input: bool, // UAdvancedCharacterMovement +0xbc4 bIgnoreMovementInput (zero-filled)
    pub ragdoll_falling: bool,   // AAdvancedCharacter +0x505 bIsRagdollFalling (set_is_ragdoll_falling)
    pub ragdoll_falling_start_time: f64, // +0x848 RagdollFallingStartTime
    /// +0x844 RagdollFallingGetUpStartTime: zero-filled in the exe, where it is compared with world TimeSeconds. This
    /// model's clock is per pawn (starts at 0 on spawn), so the zero-filled value is taken as "long ago" (-inf): in the
    /// exe a pawn spawned after world time RagdollFallingGetUpDuration (1.4 s) is never getting up at spawn.
    /// UNCONFIRMED for a pawn spawned earlier than that in a world's life (as in the reference).
    pub ragdoll_falling_get_up_start_time: f64,
    pub still_time_while_ragdoll_falling: f64, // UAdvancedCharacterMovement +0xb90 StillTimeWhileRagdollFalling
    pub b_is_airborne_from_jump: bool,   // AAdvancedCharacter +0x869
    pub b_was_last_land_from_jump: bool, // AAdvancedCharacter +0x860
    /// EMovementRestriction of the current motion (UMordhauMotion::GetMovementRestriction) and the equipment
    /// (UEquipmentSystemComponent), max of both; set by the owner
    pub motion_restriction: i64,
    pub knockback_time: f64,         // UMordhauMovementComponent +0xd4c KnockbackTime
    pub base_ground_friction: f64,   // +0xd30 BaseGroundFriction (InitializeComponent)
    pub base_falling_lateral_friction: f64, // +0xd34 BaseFallingLateralFriction
    pub pending_impulse: FVector,    // UCharacterMovementComponent PendingImpulseToApply (+0x274), cm/s
    pub last_falling_check_velocity_z: f64, // UAdvancedCharacterMovement +0xb58 LastFallingCheckVelocityZ
    pub falling_time: f64,           // AMordhauCharacter +0xe98 FallingTime
    pub wants_crouch: bool,          // AMordhauCharacter +0xde4 bWantsCrouch
    pub last_crouch_toggle_time: f64, // AMordhauCharacter +0xde0 LastCrouchToggleTime (zero-filled)
    /// CVarToggleSprint "m.ToggleSprint" / CVarToggleCrouch "m.ToggleCrouch" are registered with default 0 by
    /// _dynamic_initializer_for__CVarToggleSprint__ rva=0x641940 / _dynamic_initializer_for__CVarToggleCrouch__
    /// rva=0x6418d0 (RegisterConsoleVariable(name, 0, ...)); UMordhauInput::ApplySettings writes the player's values.
    pub toggle_sprint: i64,
    pub toggle_crouch: i64,
    /// Events for the owner (MordhauCharacter / Fighter), drained each frame: they reach the combat side.
    pub fall_damage: Vec<f64>,       // amounts CheckFallDamage passed to TakeDamage (Fall type)
    pub jumps: i64,                  // OnJumped count (JumpStaminaCost each)
    pub trips: i64,                  // Trip() calls that started a ragdoll fall
    pub ragdoll_changes: Vec<bool>,  // true / false per SetIsRagdollFalling change

    // Speed/acceleration factor fields (names from the PDB layout, extract/native/types/UMordhauMovementComponent.h).
    // Equipment terms are set by UpdateEquipmentSpeedAndAcceleration rva=0x14dcd30 (weapons; not wired yet, so 0 =
    // no weapon); armor terms by UpdateArmorSpeedAndAcceleration rva=0x14dc0c0 (the host sets armor_speed / _accel).
    pub equip_speed_cap: f64,      // +0xd00 EquipmentSpeedFactorOverride (0 = none)
    pub equip_speed_add: f64,      // +0xd0c EquipmentSpeedBonusPercentage
    pub equip_sprint_penalty: f64, // +0xd10 EquipmentSubSprintSpeedBonus
    pub armor_speed: f64,          // +0xd04 ArmorSpeedFactor (ctor 1.0)
    pub armor_accel: f64,          // +0xd08 ArmorAccelerationFactor (ctor 1.0)
    pub equip_accel_add: f64,      // +0xd14 EquipmentAccelerationBonusPercentage
}

impl MordhauMovement {
    /// MordhauMovement.load_default / load_data: the records, InitializeComponent, the ini gravity.
    pub fn new(r: &CharacterRecords) -> Self {
        let mut m = MordhauMovement {
            engine: EngineCtor::default(),
            cfg: r.movement.clone(),
            fx: r.move_extra.clone(),
            ch: r.character.clone(),
            gravity_z: r.physics.gravity_z,
            terminal_velocity: r.physics.terminal_velocity,
            jump_cooldown: r.character.jump_cooldown,
            mode: Mode::Walking,
            velocity: FVector::ZERO,
            acceleration: FVector::ZERO,
            crouched: false,
            wants_sprint: false,
            sprint_state: Sprint::Forward,
            sprint_time: 0.0,
            world_time: 0.0,
            last_landed_time: 0.0,
            authority: true,
            player_controlled: true,
            dead: false,
            b_ignore_movement_input: false,
            ragdoll_falling: false,
            ragdoll_falling_start_time: 0.0,
            ragdoll_falling_get_up_start_time: f64::NEG_INFINITY,
            still_time_while_ragdoll_falling: 0.0,
            b_is_airborne_from_jump: false,
            b_was_last_land_from_jump: false,
            motion_restriction: 0,
            knockback_time: 0.0,
            base_ground_friction: 0.0,
            base_falling_lateral_friction: 0.0,
            pending_impulse: FVector::ZERO,
            last_falling_check_velocity_z: 0.0,
            falling_time: 0.0,
            wants_crouch: false,
            last_crouch_toggle_time: 0.0,
            toggle_sprint: 0,
            toggle_crouch: 0,
            fall_damage: Vec::new(),
            jumps: 0,
            trips: 0,
            ragdoll_changes: Vec::new(),
            equip_speed_cap: 0.0,
            equip_speed_add: 0.0,
            equip_sprint_penalty: 0.0,
            armor_speed: 1.0,
            armor_accel: 1.0,
            equip_accel_add: 0.0,
        };
        m.initialize_component();
        m
    }

    // ============================================================================================================
    // UMordhauMovementComponent
    // ============================================================================================================

    /// UMordhauMovementComponent::GetSpeedFactor rva=0x14bf9a0:
    ///   s = EquipmentSpeedBonusPercentage + ArmorSpeedFactor + (1 - t) * EquipmentSubSprintSpeedBonus
    ///   if EquipmentSpeedFactorOverride > 0: s = min(override, s); (perk multiplier skipped); s *= MotionSpeedFactor
    ///   (+0xcf4, 1.0); max(s, 0).
    pub fn get_speed_factor(&self, t: f64) -> f64 {
        let mut s = self.equip_speed_add + self.armor_speed + (1.0 - t) * self.equip_sprint_penalty;
        if self.equip_speed_cap > 0.0 && s > self.equip_speed_cap {
            s = self.equip_speed_cap;
        }
        maxf(s * 1.0, 0.0) // * +0xcf4 MotionSpeedFactor (1.0)
    }

    /// UMordhauMovementComponent::GetMaxSpeed rva=0x14be1d0
    pub fn get_max_speed(&self) -> f64 {
        let cfg = &self.cfg;
        let sprint = cfg.sprint_modifier;
        let partial = cfg.partial_sprint_modifier;
        let mut m = 1.0;
        let mut apply_factor = true;
        if self.mode == Mode::Walking || self.mode == Mode::Falling {
            if self.sprint_state == Sprint::Backpedal {
                m = 1.0 * cfg.backpedal_modifier * 1.0; // +0xcf8 * BackpedalModifier * +0xcfc
            } else if self.sprint_state == Sprint::Sideways {
                m = NATIVE_SIDEWAYS_MODIFIER;
            } else if self.crouched && self.mode != Mode::Falling {
                m = 1.0;
            } else if self.sprint_state == Sprint::Super {
                if self.mode == Mode::Falling {
                    m = sprint;
                } else {
                    m = cfg.supersprint_modifier;
                    apply_factor = false;
                }
            } else if self.sprint_state >= Sprint::Sprint {
                m = sprint;
                let tt = self.sprint_time;
                if self.sprint_state == Sprint::Chase {
                    m = cfg.chasing_modifier;
                }
                let reach = cfg.sprint_time_to_reach_max_sprint;
                if reach.abs() > 1e-8 {
                    m = (m - partial) * clampf(tt / reach, 0.0, 1.0) + partial;
                }
                if self.sprint_state == Sprint::Chase {
                    apply_factor = false;
                }
            } else if self.sprint_state == Sprint::Partial {
                m = partial;
            }
            if apply_factor {
                let span = sprint - partial;
                let t = if span.abs() > 1e-8 {
                    clampf((m - partial) / span, 0.0, 1.0)
                } else if m < sprint {
                    0.0
                } else {
                    1.0
                };
                m *= self.get_speed_factor(t);
            }
        }
        // falling and not in knockback (+0xd4c <= 0, IsInKnockback rva=0x14c2720) -> MaxSpeedFalling (+0xd2c); falling in
        // a knockback takes UAdvancedCharacterMovement::GetMaxSpeed (LAB_1414be373: MovementMode != 3 or 0 < KnockbackTime)
        let base = if self.mode == Mode::Falling && self.knockback_time <= 0.0 {
            cfg.max_speed_falling
        } else {
            self.super_max_speed()
        };
        m * base
    }

    /// UAdvancedCharacterMovement::GetMaxSpeed rva=0x1484b20 (UE's own switch: crouched walking uses MaxWalkSpeedCrouched)
    fn super_max_speed(&self) -> f64 {
        match self.mode {
            Mode::Walking => {
                if self.crouched {
                    self.cfg.max_walk_speed_crouched
                } else {
                    self.cfg.max_walk_speed
                }
            }
            Mode::Falling => self.cfg.max_walk_speed,
            Mode::None => 0.0,
        }
    }

    /// UMordhauMovementComponent::GetAccelerationFactor rva=0x14bbbe0:
    ///   EquipmentAccelerationBonusPercentage (+0xd14) + ArmorAccelerationFactor (+0xd08), clamped at 0 from below
    pub fn get_acceleration_factor(&self) -> f64 {
        maxf(self.equip_accel_add + self.armor_accel, 0.0)
    }

    /// UMordhauMovementComponent::GetMaxAcceleration rva=0x14be0b0. GetAccelerationFactor scales only the sprint-class
    /// accelerations; WalkAcceleration, SupersprintAcceleration and crouch come back unscaled.
    pub fn get_max_acceleration(&self) -> f64 {
        if self.mode != Mode::Walking {
            return self.cfg.max_acceleration;
        }
        if self.crouched {
            return self.cfg.walk_acceleration;
        }
        let k = self.get_acceleration_factor();
        match self.sprint_state {
            Sprint::Super => NATIVE_SUPERSPRINT_ACCELERATION,
            Sprint::Sprint => k * self.cfg.sprint_acceleration,
            Sprint::Partial | Sprint::Rush | Sprint::Chase => k * self.cfg.partial_sprint_acceleration,
            _ => self.cfg.walk_acceleration,
        }
    }

    /// UMordhauMovementComponent::GetMaxBrakingDeceleration rva=0x14be160
    pub fn get_max_braking_deceleration(&self) -> f64 {
        if self.mode == Mode::Falling {
            let ms = maxf(self.get_max_speed(), 0.0);
            let v = self.velocity;
            let v2 = v.xf() * v.xf() + v.zf() * v.zf(); // GDScript: components read as floats, product in f64
            if ms * ms * RDATA_FALL_TOO_FAST < v2 {
                return self.cfg.braking_deceleration_falling_too_fast;
            }
            return self.cfg.braking_deceleration_falling;
        }
        self.engine.braking_deceleration_walking
    }

    /// UMordhauMovementComponent::LODTick rva=0x14c4d60, the sprint-state part (decompile lines ~1010-1160, 1748-1766).
    /// facing: actor forward. Stamina, perks and chase/rush are not modelled. Movement restriction
    /// (AMordhauCharacter::GetMovementRestriction, read at decompile line 1201): bOnlyPartialSprint (+0xd1b) =
    /// (r == PartialSprint), and below PartialSprintModifier x MaxWalkSpeed it is set for any r; bSprintIsAllowed
    /// (+0xd1a) is false for r == Walk, otherwise it needs MaxWalkSpeed (less the slack).
    pub fn update_sprint_state(&mut self, dt: f64, facing: FVector) {
        let speed = self.velocity.length() as f64;
        let k = self.get_speed_factor(0.0);
        let mws = self.cfg.max_walk_speed;
        let r = self.get_movement_restriction();
        let partial = r == restriction::PARTIAL_SPRINT
            || speed + RDATA_SPRINT_SPEED_SLACK < k * mws * self.cfg.partial_sprint_modifier;
        let can_sprint = r != restriction::WALK && !(speed + RDATA_SPRINT_SPEED_SLACK < k * mws);
        let dir = FVector::new(self.acceleration.x, 0.0, self.acceleration.z);
        if dir.is_zero_approx() {
            self.sprint_state = Sprint::Forward;
        } else {
            let a = rad_to_deg(angle_to((facing.x, facing.z), (dir.x, dir.z)));
            self.sprint_state = Sprint::Forward;
            if a.abs() <= RDATA_FORWARD_CONE_DEG {
                if self.wants_sprint && can_sprint {
                    self.sprint_state = if partial { Sprint::Partial } else { Sprint::Sprint };
                }
            } else {
                self.sprint_state = if a.abs() > RDATA_BACKPEDAL_DEG { Sprint::Backpedal } else { Sprint::Sideways };
            }
        }
        if self.sprint_state < Sprint::Sprint {
            self.sprint_time = 0.0;
        } else {
            self.sprint_time += dt;
        }
    }

    // ============================================================================================================
    // UCharacterMovementComponent (engine, disassembled from the shipped exe at its PDB address) / UAdvancedCharacterMovement
    // ============================================================================================================

    /// UAdvancedCharacterMovement::CalcVelocity rva=0x1459ab0 (a 33-byte override running into the engine's
    /// UCharacterMovementComponent::CalcVelocity rva=0x2f70490). Requested-move (AI path following), RVO avoidance and
    /// bForceMaxAccel are off for a player and left out.
    pub fn calc_velocity(&mut self, dt: f64, friction: f64, fluid: bool, braking_decel: f64) {
        if dt < ENGINE_MIN_TICK_TIME {
            return;
        }
        let friction = maxf(friction, 0.0);
        let max_speed = self.get_max_speed();
        // MaxInputSpeed = max(MaxSpeed * AnalogInputModifier, GetMinAnalogSpeed()); AnalogInputModifier is 1 for keys
        let min_analog = if self.mode == Mode::Walking { self.engine.min_analog_walk_speed } else { 0.0 };
        let max_input_speed = maxf(max_speed * 1.0, min_analog);
        let max_speed = max_input_speed;
        let zero_accel = self.acceleration == FVector::ZERO;
        let over_max = self.is_exceeding_max_speed(max_speed);
        if zero_accel || over_max {
            let old = self.velocity;
            let bf = if self.engine.b_use_separate_braking_friction { self.engine.braking_friction } else { friction };
            self.apply_velocity_braking(dt, bf, braking_decel);
            // braking may not take us below max speed if we started above it and still push forward
            if over_max
                && (self.velocity.length_squared() as f64) < max_speed * max_speed
                && self.acceleration.dot(old) > 0.0
            {
                self.velocity = old.normalized().scale(max_speed);
            }
        } else {
            // friction limits how fast the direction can change
            let dir = self.acceleration.normalized();
            let vs = self.velocity.length() as f64;
            self.velocity = self.velocity - (self.velocity - dir.scale(vs)).scale(minf(dt * friction, 1.0));
        }
        if fluid {
            self.velocity = self.velocity.scale(1.0 - minf(friction * dt, 1.0));
        }
        if !zero_accel {
            let new_max = if self.is_exceeding_max_speed(max_input_speed) {
                self.velocity.length() as f64
            } else {
                max_input_speed
            };
            self.velocity = self.velocity + self.acceleration.scale(dt);
            self.velocity = if new_max >= ENGINE_KINDA_SMALL { self.velocity.limit_length(new_max) } else { FVector::ZERO };
        }
    }

    /// UMovementComponent::IsExceedingMaxSpeed rva=0x2fb3720: |V|^2 > max(MaxSpeed,0)^2 * 1.01
    pub fn is_exceeding_max_speed(&self, max_speed: f64) -> bool {
        let max_speed = maxf(max_speed, 0.0);
        (self.velocity.length_squared() as f64) > max_speed * max_speed * ENGINE_OVER_VELOCITY_PERCENT
    }

    /// UCharacterMovementComponent::ApplyVelocityBraking rva=0x2f6f930
    pub fn apply_velocity_braking(&mut self, dt: f64, friction: f64, braking_decel: f64) {
        if self.velocity == FVector::ZERO || dt < ENGINE_MIN_TICK_TIME {
            return;
        }
        let friction = maxf(maxf(self.cfg.braking_friction_factor, 0.0) * friction, 0.0);
        let braking_decel = maxf(braking_decel, 0.0);
        let zero_friction = friction == 0.0;
        let zero_braking = braking_decel == 0.0;
        if zero_friction && zero_braking {
            return;
        }
        let old = self.velocity;
        let mut remaining = dt;
        let max_step = clampf(self.engine.braking_sub_step_time, ENGINE_BRAKING_STEP_MIN, ENGINE_BRAKING_STEP_MAX);
        let rev = if zero_braking { FVector::ZERO } else { safe_normal(self.velocity).scale(-braking_decel) };
        while remaining >= ENGINE_MIN_TICK_TIME {
            let t = if remaining > max_step && !zero_friction { minf(max_step, remaining * 0.5) } else { remaining };
            remaining -= t;
            self.velocity = self.velocity + (self.velocity.scale(-friction) + rev).scale(t);
            if self.velocity.dot(old) <= 0.0 {
                // never reverse direction
                self.velocity = FVector::ZERO;
                return;
            }
        }
        let vs2 = self.velocity.length_squared() as f64;
        if vs2 <= ENGINE_KINDA_SMALL || (!zero_braking && vs2 <= ENGINE_BRAKE_TO_STOP_SQ) {
            self.velocity = FVector::ZERO;
        }
    }

    /// UCharacterMovementComponent::GetFallingLateralAcceleration rva=0x2f794b0 -> GetAirControl rva=0x2f790f0 ->
    /// BoostAirControl rva=0x2f701e0, then ClampToMaxSize(GetMaxAcceleration()).
    pub fn get_falling_lateral_acceleration(&self) -> FVector {
        let mut fa = FVector::new(self.acceleration.x, 0.0, self.acceleration.z);
        if fa.xf() * fa.xf() + fa.zf() * fa.zf() > 0.0 {
            let mut ac = self.cfg.air_control;
            if ac != 0.0 {
                let v = self.velocity;
                let v2 = v.xf() * v.xf() + v.zf() * v.zf();
                let thr = self.cfg.air_control_boost_velocity_threshold;
                if self.cfg.air_control_boost_multiplier > 0.0 && v2 < thr * thr {
                    ac = minf(self.cfg.air_control_boost_multiplier * ac, 1.0);
                }
            }
            fa = fa.scale(ac).limit_length(self.get_max_acceleration());
        }
        fa
    }

    /// UCharacterMovementComponent::DoJump rva=0x2f775e0: Velocity.Z = max(JumpZVelocity, Velocity.Z); SetMovementMode(3).
    /// Gated like AMordhauCharacter::CanJumpInternal_Implementation rva=0x1532630, which tail-jumps (0x1415326ff) to
    /// AAdvancedCharacter::CanJumpInternal_Implementation rva=0x1459fb0: JumpCooldown (+0x864) + LastLand (+0x85c) >
    /// TimeSeconds -> false (0x141459fd2..0x141459fe9), i.e. allowed from LastLand + JumpCooldown on; then
    /// ACharacter::CanJumpInternal_Implementation (walking). AMordhauCharacter::CanJumpInternal_Implementation also
    /// refuses while GetMovementRestriction == NoMovement and while IsRagdollFallingOrGettingUp (vcall +0x960).
    /// Not modelled (as the reference): leg-disabled, attack window, LastDodgeTime + DodgeDuration (+0xea0 / +0xea4).
    pub fn can_jump(&self) -> bool {
        if self.get_movement_restriction() == restriction::NO_MOVEMENT {
            return false;
        }
        if self.is_ragdoll_falling_or_getting_up() {
            return false;
        }
        self.mode == Mode::Walking && self.world_time >= self.last_landed_time + self.jump_cooldown
    }

    pub fn do_jump(&mut self) -> bool {
        if !self.can_jump() {
            return false;
        }
        self.velocity.y = maxf(self.cfg.jump_z_velocity, self.velocity.yf()) as f32;
        self.mode = Mode::Falling;
        self.on_jumped();
        true
    }

    /// AAdvancedCharacter::OnJumped_Implementation rva=0x148d580: bIsAirborneFromJump = true (and LastJump / bJumped).
    /// AMordhauCharacter::OnJumped_Implementation rva=0x1554610 then OffsetStamina(-(int)JumpStaminaCost) and stops the
    /// stamina regeneration; both live on the combat side, so the jump is counted here and the owner applies them
    /// (character::apply_movement_events).
    pub fn on_jumped(&mut self) {
        self.b_is_airborne_from_jump = true;
        self.jumps += 1;
    }

    /// AAdvancedCharacter::Landed rva=0x1489820: bWasLastLandFromJump = bIsAirborneFromJump, LastLand = TimeSeconds,
    /// bIsAirborneFromJump = false.
    pub fn landed(&mut self) {
        self.b_was_last_land_from_jump = self.b_is_airborne_from_jump;
        self.last_landed_time = self.world_time;
        self.b_is_airborne_from_jump = false;
    }

    /// AAdvancedCharacter::IsRagdollFallingOrGettingUp rva=0x1487ca0: bIsRagdollFalling, or TimeSeconds is still before
    /// RagdollFallingGetUpStartTime + RagdollFallingGetUpDuration (false from that sum on, `<=`).
    pub fn is_ragdoll_falling_or_getting_up(&self) -> bool {
        if self.ragdoll_falling {
            return true;
        }
        self.world_time < self.ragdoll_falling_get_up_start_time + self.ch.ragdoll_falling_get_up_duration
    }

    /// AMordhauCharacter::Trip rva=0x156f7b0 -> AAdvancedCharacter::Trip rva=0x14a78c0: alive, not ragdoll falling, not
    /// bDisableRagdollFalling (+0x7f8) and not IsRagdollFallingOrGettingUp (vcall +0x960) -> SetIsRagdollFalling(true)
    /// (vcall +0xa58), true. (The vehicle StopDriving of AMordhauCharacter::Trip has no vehicle here.)
    pub fn trip(&mut self) -> bool {
        if self.dead || self.ragdoll_falling || self.ch.b_disable_ragdoll_falling {
            return false;
        }
        if self.is_ragdoll_falling_or_getting_up() {
            return false;
        }
        self.set_is_ragdoll_falling(true);
        self.trips += 1;
        true
    }

    /// AMordhauCharacter::SetIsRagdollFalling rva=0x1568ad0 -> AAdvancedCharacter::SetIsRagdollFalling rva=0x14a0980:
    /// dead or unchanged -> nothing; on: RagdollFallingStartTime = TimeSeconds; off: RagdollFallingGetUpStartTime =
    /// TimeSeconds. Not modelled here: the perch radius swap, the mesh physics blend, the ReplicatedCharacterFlags bit 4,
    /// and on authority the RagdollFalling net motion (MotionType 0x1a) - the change is queued in ragdoll_changes.
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

    /// UAdvancedCharacterMovement::TickComponent rva=0x14a3e80, the ragdoll get-up (disassembly 0x1414a4365..0x1414a4419):
    /// alive (+0x504) and bIsRagdollFalling (+0x505) and |Velocity|^2 <= RagdollFallingMinVelocityToGetUp^2 ->
    /// StillTimeWhileRagdollFalling += dt; at >= RagdollFallingTimeAtMinVelocityToGetUp with RagdollFallingStartTime +
    /// RagdollFallingMinTime < TimeSeconds and Role == Authority -> SetIsRagdollFalling(false). Not ragdoll falling or
    /// faster -> StillTimeWhileRagdollFalling = 0. Dead -> untouched.
    pub fn tick_ragdoll_get_up(&mut self, dt: f64) {
        if self.dead {
            return;
        }
        let v = self.ch.ragdoll_falling_min_velocity_to_get_up;
        if !self.ragdoll_falling || (self.velocity.length_squared() as f64) > v * v {
            self.still_time_while_ragdoll_falling = 0.0;
            return;
        }
        self.still_time_while_ragdoll_falling += dt;
        if self.still_time_while_ragdoll_falling >= self.ch.ragdoll_falling_time_at_min_velocity_to_get_up
            && self.ragdoll_falling_start_time + self.ch.ragdoll_falling_min_time < self.world_time
            && self.authority
        {
            self.set_is_ragdoll_falling(false);
        }
    }

    /// AMordhauCharacter::GetMovementRestriction rva=0x1540f50: the current motion's restriction; None within 0.1 s of
    /// a landing that ended a jump (TimeSeconds < LastLand + 0.1 and bWasLastLandFromJump) -> PartialSprint (1); then
    /// the max with the equipment's (UEquipmentSystemComponent::GetMovementRestriction; both folded into
    /// motion_restriction). The Rush perk branch (HasPerk 0xe) is not modelled (no perk system yet).
    pub fn get_movement_restriction(&self) -> i64 {
        let mut r = self.motion_restriction;
        if self.world_time < self.last_landed_time + 0.1 && self.b_was_last_land_from_jump && r == 0 {
            r = restriction::PARTIAL_SPRINT;
        }
        r
    }

    /// AMordhauCharacter::IsMoveInputIgnored rva=0x154afe0: GetMovementRestriction == NoMovement or
    /// IsRagdollFallingOrGettingUp (vcall +0x960). APawn::Internal_AddMovementInput rva=0x32eed40 drops the input when
    /// it is true (vcall +0x758 at 0x1432eed58).
    pub fn is_move_input_ignored(&self) -> bool {
        self.get_movement_restriction() == restriction::NO_MOVEMENT || self.is_ragdoll_falling_or_getting_up()
    }

    /// UMordhauMovementComponent::ConstrainInputAcceleration rva=0x14b7a90 -> UAdvancedCharacterMovement::
    /// ConstrainInputAcceleration rva=0x145aee0: bIgnoreMovementInput -> zero, else the engine's constraint (the input
    /// here is already lateral).
    pub fn constrain_input_acceleration(&self, a: FVector) -> FVector {
        if self.b_ignore_movement_input {
            FVector::ZERO
        } else {
            a
        }
    }

    // ---- crouch ---------------------------------------------------------------------------------------------------

    /// AMordhauCharacter::CrouchPressed rva=0x1538040: m.ToggleCrouch == 1 -> bWantsCrouch = !bWantsCrouch, else true.
    pub fn crouch_pressed(&mut self) {
        self.wants_crouch = if self.toggle_crouch == 1 { !self.wants_crouch } else { true };
    }

    /// AMordhauCharacter::CrouchReleased rva=0x1538100: m.ToggleCrouch != 1 -> bWantsCrouch = false.
    pub fn crouch_released(&mut self) {
        if self.toggle_crouch != 1 {
            self.wants_crouch = false;
        }
    }

    /// AMordhauCharacter::CanCrouch rva=0x1532230: when not crouched, disabled legs (not ported) or
    /// GetMovementRestriction == NoMovement -> false; then AAdvancedCharacter::CanCrouch rva=0x1459e10: not crouched and
    /// IsRagdollFallingOrGettingUp -> false; true when not crouched, NavAgentProps.bCanCrouch and the root is not
    /// simulating physics (no ragdoll here).
    pub fn can_crouch(&self) -> bool {
        if !self.crouched {
            if self.get_movement_restriction() == restriction::NO_MOVEMENT {
                return false;
            }
            if self.is_ragdoll_falling_or_getting_up() {
                return false;
            }
        }
        !self.crouched && self.fx.b_can_crouch
    }

    /// AMordhauCharacter::LODTick rva=0x154c390, crouch part (from 0x14154c4cf): bIsCrouched != bWantsCrouch and
    /// CrouchCooldown + LastCrouchToggleTime < TimeSeconds -> LastCrouchToggleTime = TimeSeconds, then Crouch() /
    /// UnCrouch() (ACharacter vtable). Some(true) = crouch, Some(false) = uncrouch, None = no change. Crouch() goes
    /// through CanCrouch (ACharacter::Crouch); UnCrouch always proceeds.
    pub fn update_crouch(&mut self) -> Option<bool> {
        if self.crouched == self.wants_crouch {
            return None;
        }
        if !(self.ch.crouch_cooldown + self.last_crouch_toggle_time < self.world_time) {
            return None;
        }
        self.last_crouch_toggle_time = self.world_time;
        if self.wants_crouch {
            return if self.can_crouch() { Some(true) } else { None };
        }
        Some(false)
    }

    // ---- sprint ---------------------------------------------------------------------------------------------------

    /// AMordhauCharacter::SprintPressed rva=0x156da50 (keyboard; the gamepad branch is skipped): m.ToggleSprint != 0
    /// and bWantsToSprint -> false; otherwise true.
    pub fn sprint_pressed(&mut self) {
        self.wants_sprint = !(self.toggle_sprint != 0 && self.wants_sprint);
    }

    /// AMordhauCharacter::SprintReleased rva=0x156dba0: m.ToggleSprint == 0 -> bWantsToSprint = false.
    pub fn sprint_released(&mut self) {
        if self.toggle_sprint == 0 {
            self.wants_sprint = false;
        }
    }

    /// AMordhauCharacter::StartSprinting rva=0x156dcc0: bWantsToSprint (+0xd19) = 1
    pub fn start_sprinting(&mut self) {
        self.wants_sprint = true;
    }

    /// AMordhauCharacter::StopSprinting rva=0x156deb0: bWantsToSprint (+0xd19) = 0
    pub fn stop_sprinting(&mut self) {
        self.wants_sprint = false;
    }

    /// AMordhauCharacter::MoveForward rva=0x1550170: a zero forward axis on keyboard with m.ToggleSprint == 1 ->
    /// StopSprinting, then AAdvancedCharacter::MoveForward rva=0x148a6c0 (Value x actor forward into AddMovementInput).
    pub fn move_forward_axis(&mut self, value: f64) {
        if value == 0.0 && self.toggle_sprint == 1 {
            self.stop_sprinting();
        }
    }

    // ---- knockback ------------------------------------------------------------------------------------------------

    /// UMordhauMovementComponent::InitializeComponent rva=0x14c1300: BaseGroundFriction / BaseFallingLateralFriction =
    /// the current GroundFriction / FallingLateralFriction (what PostCharacterMovementTick restores after a knockback).
    pub fn initialize_component(&mut self) {
        self.base_ground_friction = self.cfg.ground_friction;
        self.base_falling_lateral_friction = self.cfg.falling_lateral_friction;
    }

    /// UMordhauMovementComponent::IsInKnockback rva=0x14c2720 (AMordhauCharacter::IsInKnockback rva=0x154aad0
    /// forwards): 0 < KnockbackTime.
    pub fn is_in_knockback(&self) -> bool {
        0.0 < self.knockback_time
    }

    /// UMordhauMovementComponent::Knockback rva=0x14c2820. amount: cm/s on Godot axes (UE Z = Godot Y). Authority:
    /// AddImpulse(Amount + (0, 0, IsFalling ? 0 : KnockbackUpImpulse), bVelocityChange = true) (vcall +0x810; IsFalling
    /// vcall +0x560). Every role: KnockbackTime = KnockbackDuration, FallingLateralFriction =
    /// KnockbackFallingLateralFriction, GroundFriction = KnockbackGroundFriction.
    pub fn knockback(&mut self, amount: FVector) {
        if self.authority {
            let up = if self.mode == Mode::Falling { 0.0 } else { self.fx.knockback_up_impulse };
            self.add_impulse(FVector::new(amount.x, (up + amount.yf()) as f32, amount.z), true);
        }
        self.knockback_time = self.fx.knockback_duration;
        self.cfg.falling_lateral_friction = self.fx.knockback_falling_lateral_friction;
        self.cfg.ground_friction = self.fx.knockback_ground_friction;
    }

    /// UCharacterMovementComponent::AddImpulse rva=0x2f6da30 with bVelocityChange: PendingImpulseToApply += Impulse
    /// (the mass division of the other path is not used by any ported caller).
    pub fn add_impulse(&mut self, impulse: FVector, _velocity_change: bool) {
        self.pending_impulse = self.pending_impulse + impulse;
    }

    /// UCharacterMovementComponent::ApplyAccumulatedForces rva=0x2f6e650: PendingImpulse.Z (+0x27c) != 0, on the
    /// ground (IsMovingOnGround, vcall +0x568) and (GetGravityZ + PendingForce.Z) * dt + PendingImpulse.Z > 1e-8 ->
    /// SetMovementMode(Falling); then Velocity += PendingImpulse (+ force x dt; no forces here), pending cleared.
    pub fn apply_accumulated_forces(&mut self, dt: f64) {
        if self.pending_impulse.y != 0.0 && self.mode == Mode::Walking && self.gravity() * dt + self.pending_impulse.yf() > 1e-8 {
            self.mode = Mode::Falling;
        }
        self.velocity = self.velocity + self.pending_impulse;
        self.pending_impulse = FVector::ZERO;
    }

    /// UMordhauMovementComponent::PostCharacterMovementTick rva=0x14d0710: KnockbackTime -= dt; at or below 0 the base
    /// frictions come back and KnockbackTime = 0.
    pub fn post_character_movement_tick(&mut self, dt: f64) {
        self.knockback_time -= dt;
        if self.knockback_time <= 0.0 {
            self.cfg.falling_lateral_friction = self.base_falling_lateral_friction;
            self.cfg.ground_friction = self.base_ground_friction;
            self.knockback_time = 0.0;
        }
    }

    // ---- fall damage ----------------------------------------------------------------------------------------------

    /// UAdvancedCharacterMovement::CheckFallDamage rva=0x145a230 (authority, AAdvancedCharacter owner): the Ragdoll*
    /// terms when bIsRagdollFalling (+0x505), else MinVelocityForFallDamage / FallDamageOffset / FallDamageFactor;
    /// Delta = CurrentVelocityZ - LastFallingCheckVelocityZ, LastFallingCheckVelocityZ = CurrentVelocityZ;
    /// MinVelocity < Delta and (Delta + Offset) * Factor > 0 -> TakeDamage(that, EMordhauDamageType::Fall, the character
    /// itself as causer) - queued in fall_damage for the owner. UAdvancedCharacterMovement::PerformMovement rva=0x1495cc0
    /// calls it after every move with Velocity.Z (vcall +0xb38 = slot 359 in types/UAdvancedCharacterMovement.h), and
    /// OnMovementModeChanged rva=0x14901a0 with 0 on entering Walking: the speed lost at the landing is the Delta.
    pub fn check_fall_damage(&mut self, current_velocity_z: f64) -> f64 {
        if !self.authority {
            return 0.0;
        }
        let (mut min_v, mut off, mut fac) =
            (self.fx.min_velocity_for_fall_damage, self.fx.fall_damage_offset, self.fx.fall_damage_factor);
        if self.ragdoll_falling {
            min_v = self.fx.ragdoll_min_velocity_for_fall_damage;
            off = self.fx.ragdoll_fall_damage_offset;
            fac = self.fx.ragdoll_fall_damage_factor;
        }
        let delta = current_velocity_z - self.last_falling_check_velocity_z;
        self.last_falling_check_velocity_z = current_velocity_z;
        if min_v < delta {
            let d = (delta + off) * fac;
            if 0.0 < d {
                self.fall_damage.push(d);
                return d;
            }
        }
        0.0
    }

    // ---- falling too long -> trip ---------------------------------------------------------------------------------

    /// AMordhauCharacter::LODTick rva=0x154c390, falling part: authority, IsPlayerControlled (vcall +0x6c0), alive,
    /// Velocity.Z < -1 (comiss at 0x14154c47c on the movement component's +0xcc), IsAirborne (vcall +0x9e0) and not
    /// bIsRagdollFalling -> FallingTime += dt; above FallingTimeToRagdoll -> Trip() (vcall +0x8f0) and FallingTime = 0;
    /// any other frame resets FallingTime.
    pub fn lod_tick_falling(&mut self, dt: f64) {
        if self.authority
            && self.player_controlled
            && !self.dead
            && self.velocity.yf() < -1.0
            && self.is_airborne()
            && !self.ragdoll_falling
        {
            self.falling_time += dt;
            if self.falling_time <= self.ch.falling_time_to_ragdoll {
                return;
            }
            self.trip();
        }
        self.falling_time = 0.0;
    }

    /// AMordhauCharacter::IsAirborne rva=0x154a7f0 (no vehicle; Role > SimulatedProxy): the movement's IsFalling.
    pub fn is_airborne(&self) -> bool {
        self.mode == Mode::Falling
    }

    /// GetGravityZ = world gravity (DefaultGravityZ) * GravityScale
    pub fn gravity(&self) -> f64 {
        self.gravity_z * self.cfg.gravity_scale
    }

    /// UCharacterMovementComponent::GetSimulationTimeStep, inlined in PhysWalking rva=0x2f81730: split the frame into
    /// steps of at most MaxSimulationTimeStep, halving the remainder, for up to MaxSimulationIterations.
    fn sim_step(&self, remaining: f64, iterations: i64) -> f64 {
        let mts = self.engine.max_simulation_time_step;
        let mut step = remaining;
        if remaining > mts && iterations < self.engine.max_simulation_iterations {
            step = minf(mts, remaining * 0.5);
        }
        maxf(step, ENGINE_MIN_TICK_TIME)
    }

    /// One frame. input: wanted move direction in world space (lateral, length <= 1; UE AddMovementInput summed).
    /// facing: actor forward. Returns the position delta in cm; the body moves by it and reports floor contact back.
    pub fn tick(&mut self, dt: f64, input: FVector, facing: FVector) -> FVector {
        self.world_time += dt;
        let mut input = FVector::new(input.x, 0.0, input.z);
        if self.is_move_input_ignored() {
            input = FVector::ZERO; // APawn::Internal_AddMovementInput drops it (IsMoveInputIgnored)
        }
        let input = self.constrain_input_acceleration(input);
        // ScaleInputAcceleration: GetMaxAcceleration() * input clamped to 1. LODTick reads its direction for the sprint
        // state first; PerformMovement then uses the acceleration of the new state.
        self.acceleration = input.limit_length(1.0);
        self.update_sprint_state(dt, facing);
        self.acceleration = input.limit_length(1.0).scale(self.get_max_acceleration());
        self.apply_accumulated_forces(dt);
        let mut delta = FVector::ZERO;
        let mut remaining = dt;
        let mut iters = 0;
        while remaining >= ENGINE_MIN_TICK_TIME && iters < self.engine.max_simulation_iterations {
            iters += 1;
            let step = self.sim_step(remaining, iters);
            remaining -= step;
            if self.mode == Mode::Walking {
                // PhysWalking: Acceleration.Z = 0, MaintainHorizontalGroundVelocity, CalcVelocity(GroundFriction)
                self.velocity.y = 0.0;
                let bd = self.get_max_braking_deceleration();
                self.calc_velocity(step, self.cfg.ground_friction, false, bd);
                delta = delta + self.velocity.scale(step);
            } else if self.mode == Mode::Falling {
                // PhysFalling rva=0x2f7ef00: lateral CalcVelocity with Velocity.Z zeroed and the air-control acceleration
                // swapped in, then NewFallVelocity rva=0x2f7cec0 (gravity, clamp to TerminalVelocity), then
                // Adjusted = 0.5 * (OldVelocity + Velocity) * dt.
                let old = self.velocity;
                let saved = self.acceleration;
                self.acceleration = self.get_falling_lateral_acceleration();
                self.velocity.y = 0.0;
                let bd = self.get_max_braking_deceleration();
                self.calc_velocity(step, self.cfg.falling_lateral_friction, false, bd);
                self.velocity.y = old.y;
                self.acceleration = saved;
                self.velocity.y = (self.velocity.yf() + self.gravity() * step) as f32;
                let tv = self.terminal_velocity;
                if (self.velocity.length_squared() as f64) > tv * tv && -self.velocity.yf() > tv {
                    self.velocity.y = (-tv) as f32;
                }
                delta = delta + (old + self.velocity).scale(0.5).scale(step);
            }
        }
        let vz = self.velocity.yf();
        self.check_fall_damage(vz); // UAdvancedCharacterMovement::PerformMovement tail
        self.post_character_movement_tick(dt);
        self.tick_ragdoll_get_up(dt);
        self.lod_tick_falling(dt);
        delta
    }

    /// Floor contact from the body: landing -> Walking (AAdvancedCharacter::Landed: LastLand for JumpCooldown and
    /// bWasLastLandFromJump; OnMovementModeChanged -> CheckFallDamage(0)); losing the floor -> Falling.
    pub fn set_on_floor(&mut self, on_floor: bool) {
        if self.mode == Mode::Falling && on_floor && self.velocity.y <= 0.0 {
            self.mode = Mode::Walking;
            self.velocity.y = 0.0;
            self.landed();
            self.check_fall_damage(0.0);
        } else if self.mode == Mode::Walking && !on_floor {
            self.mode = Mode::Falling;
        }
    }
}

/// FVector::GetSafeNormal as the engine inlines it: zero below 1e-8 squared length
pub fn safe_normal(v: FVector) -> FVector {
    if (v.length_squared() as f64) < 1e-8 {
        FVector::ZERO
    } else {
        v.normalized()
    }
}

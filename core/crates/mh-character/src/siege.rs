//! Siege engines (rust-character r7): the aim, the shooters and the legacy vehicle net smoothing.
//!
//! The shipped siege engines are characters: BP_Ballista (BP_MordhauVehicle -> AMordhauVehicle : AAdvancedCharacter,
//! UAdvancedCharacterMovement with MaxWalkSpeed 0, BP_VehicleBallista seat, UProjectileTurretShooterComponent) and
//! BP_Catapult (AMordhauWheeledVehicle : AMordhauCompoundVehicle : AMordhauVehicle, UPseudoVehicleMovementComponent,
//! UProjectileArmShooterComponent). The extract/json search for classes deriving from the legacy AVehicleBase family
//! (AVehicleBase : AWheeledVehicle, ACatapult, ACannon) finds none: `VehicleNet` (AVehicleBase::SyncPhysics) is ported
//! for completeness, no shipped package uses it.
//!
//! - `VehicleAim`: AAdvancedCharacter::Turn rva=0x14a8a90 / LookUp rva=0x1489930 with the rate caps
//!   (SetTurnCaps rva=0x14a15f0, the cap refill of AAdvancedCharacter::LODTick rva=0x14887b0 at 0x1414890..),
//!   UAdvancedCharacterMovement::LODTick rva=0x1489530's PendingTurnValue -> AMordhauVehicle::AddTurnDegrees
//!   rva=0x1615760 -> AAdvancedCharacter::AddTurnDegrees rva=0x14574c0 (TurnLimit around SpawnTurnValue through
//!   FMath::ClampAngle rva=0x18a27c0), UCharacterMovementComponent::PhysicsRotation rva=0x2f823e0 (the actor yaw
//!   follows the control yaw at RotationRate (0, -1, 0): instant, `exe_cmc::delta_rotation_yaw`), AMordhauVehicle::LODTick rva=0x162c590 (LookUpValue back
//!   to 0 with nobody driving).
//! - `Shooter`: UProjectileShooterComponent (the stage machine Loaded 0 -> Releasing 1 -> Recovery 2 -> Loading 3 ->
//!   Loaded, Building 4 at BeginPlay; timers; ammo) with the turret (ballista) and arm (catapult) overrides. The
//!   projectile itself is `projectile::Projectile::fire_quat` at the socket transform the host's posed mesh gives
//!   (`ShooterEvent::Spawn`); the catapult's launch speed is `catapult_initial_speed`.
//!
//! Blueprint glue (state/character_kismet/*.txt from scripts/world_kismet.py, @N = in-memory statement index):
//! BP_Ballista "Turn Right" -> Turn(AxisValue, false) @1566, "Look Up" -> LookUp(AxisValue, false) @1541, Fire ->
//! ProjectileTurretShooter.OnFirePressed() @1591; BP_Catapult Fire -> ProjectileArmShooter.OnFirePressed() @904, Lower /
//! Raise Catapult Arm -> OnLowerArmPressed @610 / OnRaiseArmPressed @643; BP_CatapultProjectile Fire @1514 ->
//! SetProjectileInitialSpeed(Creator's ProjectileArmShooter.ReplicatedArm) before the parent Fire.
//!
//! The catapult's driving (UPseudoVehicleMovementComponent with its SecondaryComponents, SecondaryTurn) is
//! exe_pseudo.rs. Not ported here: the cosmetic sounds,
//! particles, camera shakes; the mouse smoothing of LookUp (cvar read at 0x141489a.., taken as 0); the order of the
//! input, LODTick, movement and timer-manager ticks inside a frame (the host's; UE 4.26's TG_PrePhysics actor ticks,
//! then the timer manager).

use crate::ue::FVector;
use crate::uemath::*;
use crate::uequat::*;
use serde_json::Value;

// ---- small exe math ------------------------------------------------------------------------------------------------

/// FMath::FInterpTo rva=0x18aae60 (text 0x18a9e60): Speed <= 0 or Dist^2 < 1e-8 -> Target; else
/// Current + Dist * min(dt * Speed, 1) (a negative alpha gives 0)
pub fn finterp_to(current: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if !(speed > 0.0) {
        return target;
    }
    let dist = target - current;
    if dist * dist < 1e-8 {
        return target;
    }
    let a = dt * speed;
    let a = if a >= 0.0 { if a < 1.0 { a } else { 1.0 } } else { 0.0 };
    a * dist + current
}

/// FMath::ClampAngle rva=0x18a27c0 (text 0x18a17c0, read off the disassembly): MaxDelta = ClampAxis(Max - Min) * 0.5,
/// RangeCenter = ClampAxis(Min + MaxDelta), DeltaFromCenter = NormalizeAxis(Angle - RangeCenter); past +/- MaxDelta ->
/// NormalizeAxis(RangeCenter +/- MaxDelta), else NormalizeAxis(Angle)
pub fn clamp_angle(angle: f32, min: f32, max: f32) -> f32 {
    let norm = |a: f32| {
        let r = clamp_axis(a);
        if r > 180.0 {
            r - 360.0
        } else {
            r
        }
    };
    let max_delta = clamp_axis(max - min) * 0.5;
    let centre = clamp_axis(max_delta + min);
    let d = norm(angle - centre);
    let r = if d > max_delta {
        centre + max_delta
    } else if d < -max_delta {
        centre - max_delta
    } else {
        angle
    };
    norm(r)
}

/// FRotator::GetNormalized of one axis as VectorNormalizeRotator does it (VectorMod 360 with the non-fractional
/// guard, clamp to [-360, 360], + 360 when negative, - 360 above 180): the per-axis delta of
/// AVehicleBase::SyncPhysics and UCharacterMovementComponent::PhysicsRotation
pub fn normalize_axis_vm(d: f32) -> f32 {
    let q = d / 360.0;
    let t = if q.abs() >= 8_388_608.0 { q } else { q.trunc() };
    let mut r = d - t * 360.0;
    r = if r < 360.0 { r } else { 360.0 };
    r = if -360.0 > r { -360.0 } else { r };
    if !(0.0 <= r) {
        r = r + 360.0;
    }
    if 180.0 < r {
        r = r - 360.0;
    }
    r
}

/// FMath::RoundToInt as the exe inlines it: cvtss2si(2x + 0.5) >> 1 (round half to even on the doubled value)
fn round_to_int(x: f32) -> i32 {
    let d = x + x + 0.5;
    (d.round_ties_even() as i32) >> 1
}

// ---- aim -----------------------------------------------------------------------------------------------------------

/// The aim state of a siege-engine character (AAdvancedCharacter fields, offsets of this build)
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleAim {
    pub look_up_value: f32,           // +0x520
    pub look_up_limit: f32,           // +0x8a0
    pub look_down_limit: f32,         // +0x8a4
    pub turn_limit: f32,              // +0x8a8 (ctor -1 = none)
    pub spawn_turn_value: f32,        // BeginPlay: ClampAxis(actor yaw)
    pub turn_rate_cap: f32,           // +0x8c4 (ctor -1 = none)
    pub turn_rate_cap_target: f32,    // +0x8c8 (ctor: not set, 0)
    pub look_up_rate_cap: f32,        // +0x8cc (ctor -1)
    pub look_up_rate_cap_target: f32, // +0x8d0 (0)
    pub turn_cap_remaining: f32,      // +0x8d8
    pub look_up_cap_remaining: f32,
    /// bTurnUsesControllerInputYawScale / bTurnRateIgnoresCap (AAdvancedCharacter)
    pub turn_uses_input_yaw_scale: bool,
    pub turn_rate_ignores_cap: bool,
    /// AMordhauVehicle::bResetLookUpWhenNoDriver (ctor true, AMordhauVehicle.cpp 0x1088)
    pub reset_look_up_when_no_driver: bool,
    /// the controller's ControlRotation yaw (AddTurnDegrees writes it)
    pub control_yaw: f32,
    /// AAdvancedCharacter::TargetControlYaw (Turn keeps it in [0, 360))
    pub target_control_yaw: f32,
    /// UAdvancedCharacterMovement::PendingTurnValue (+0xbc0)
    pub pending_turn_value: f32,
    /// the actor (root component) yaw
    pub yaw: f32,
}

impl VehicleAim {
    /// a vehicle at `yaw` with the class's limits and caps (BeginPlay: SpawnTurnValue = ClampAxis(yaw),
    /// AAdvancedCharacter BeginPlay 0x141.. AAdvancedCharacter.cpp:3433; the control rotation starts at the actor's)
    pub fn new(yaw: f32, look_up_limit: f32, look_down_limit: f32, turn_limit: f32, turn_rate_cap: f32, look_up_rate_cap: f32) -> VehicleAim {
        VehicleAim {
            look_up_value: 0.0,
            look_up_limit,
            look_down_limit,
            turn_limit,
            spawn_turn_value: clamp_axis(yaw),
            turn_rate_cap,
            turn_rate_cap_target: 0.0,
            look_up_rate_cap,
            look_up_rate_cap_target: 0.0,
            turn_cap_remaining: 0.0,
            look_up_cap_remaining: 0.0,
            turn_uses_input_yaw_scale: false,
            turn_rate_ignores_cap: false,
            reset_look_up_when_no_driver: true,
            control_yaw: yaw,
            target_control_yaw: clamp_axis(yaw),
            pending_turn_value: 0.0,
            yaw,
        }
    }

    /// AAdvancedCharacter::Turn rva=0x14a8a90 for a UAdvancedCharacterMovement character. `input_ignored` = the
    /// vcall +0x960 IsRagdollFallingOrGettingUp, only asked when not absolute; `yaw_scale` = Some(InputYawScale) for an
    /// AMordhauPlayerController (RegisterAnglingXInput is the combat side's)
    pub fn turn(&mut self, value: f32, absolute: bool, input_ignored: bool, yaw_scale: Option<f32>) {
        if !absolute && input_ignored {
            return;
        }
        let mut k = 1.0f32;
        if let Some(s) = yaw_scale {
            if !absolute && self.turn_uses_input_yaw_scale {
                k = s;
            }
        }
        if value == 0.0 {
            return;
        }
        let x = k * value;
        let mut d = x;
        if !absolute && self.turn_rate_cap != -1.0 && !self.turn_rate_ignores_cap {
            let rem = self.turn_cap_remaining;
            d = if -rem <= x { if x <= rem { x } else { rem } } else { -rem };
            let r = rem - d.abs();
            self.turn_cap_remaining = if r <= 0.0 { 0.0 } else { r };
        }
        let mut t = fmod(d + self.target_control_yaw, 360.0);
        if t < 0.0 {
            t = t + 360.0;
        }
        self.target_control_yaw = t;
        self.pending_turn_value = d + self.pending_turn_value;
    }

    /// AAdvancedCharacter::LookUp rva=0x1489930 (mouse smoothing off; `input_ignored` = vcall +0x960
    /// IsRagdollFallingOrGettingUp when not absolute; `pitch_scale` = Some(InputPitchScale) for an
    /// AMordhauPlayerController, used when not absolute). Returns the new LookUpValue when it changed (ServerLookUp)
    pub fn look_up(&mut self, value: f32, absolute: bool, input_ignored: bool, pitch_scale: Option<f32>) -> Option<f32> {
        if !absolute && input_ignored {
            return None;
        }
        let mut k = 1.0f32;
        if let Some(s) = pitch_scale {
            if !absolute {
                k = s;
            }
        }
        if value == 0.0 {
            return None;
        }
        let x = k * value;
        let mut d = x;
        if self.look_up_rate_cap != -1.0 && !absolute {
            let rem = self.look_up_cap_remaining;
            d = if -rem <= x { if x <= rem { x } else { rem } } else { -rem };
            let r = rem - d.abs();
            self.look_up_cap_remaining = if r <= 0.0 { 0.0 } else { r };
        }
        let n = d + self.look_up_value;
        let lo = -self.look_down_limit;
        self.look_up_value = if lo <= n { if n <= self.look_up_limit { n } else { self.look_up_limit } } else { lo };
        Some(self.look_up_value)
    }

    /// AAdvancedCharacter::SetTurnCaps rva=0x14a15f0
    pub fn set_turn_caps(&mut self, turn: f32, look_up: f32) {
        if turn != -1.0 && (self.turn_rate_cap == -1.0 || turn < self.turn_rate_cap) {
            self.turn_rate_cap = turn;
            let c = turn * 0.0333;
            self.turn_cap_remaining = if self.turn_cap_remaining <= c { self.turn_cap_remaining } else { c };
        }
        self.turn_rate_cap_target = turn;
        if look_up != -1.0 && (self.look_up_rate_cap == -1.0 || look_up < self.look_up_rate_cap) {
            self.look_up_rate_cap = look_up;
            let c = look_up * 0.0333;
            self.look_up_cap_remaining = if self.look_up_cap_remaining <= c { self.look_up_cap_remaining } else { c };
        }
        self.look_up_rate_cap_target = look_up;
    }

    /// The cap refill of AAdvancedCharacter::LODTick rva=0x14887b0 (0x1414890a0..0x1414891e3): a cap with no target
    /// (-1) grows 1500 / 1050 per s and lifts past 1500 / 1050; one below its target interpolates up at 4500 / 3150;
    /// remaining = clamp(cap * dt + remaining, 0, cap * 0.0333)
    pub fn lod_tick_caps(&mut self, dt: f32) {
        let mut cap = self.turn_rate_cap;
        if cap != -1.0 {
            let mut tgt = self.turn_rate_cap_target;
            if tgt == -1.0 {
                cap = dt * 1500.0 + cap;
                self.turn_rate_cap = cap;
            } else if cap < tgt {
                cap = finterp_constant_to(cap, tgt, dt, 4500.0);
                tgt = self.turn_rate_cap_target;
                self.turn_rate_cap = cap;
            }
            let r = cap * dt + self.turn_cap_remaining;
            self.turn_cap_remaining = if 0.0 <= r { if cap * 0.0333 <= r { cap * 0.0333 } else { r } } else { 0.0 };
            if tgt == -1.0 && 1500.0 < cap {
                self.turn_rate_cap = -1.0;
            }
        }
        let mut cap = self.look_up_rate_cap;
        if cap == -1.0 {
            return;
        }
        let mut tgt = self.look_up_rate_cap_target;
        if tgt == -1.0 {
            cap = dt * 1050.0 + cap;
            self.look_up_rate_cap = cap;
        } else if cap < tgt {
            cap = finterp_constant_to(cap, tgt, dt, 3150.0);
            tgt = self.look_up_rate_cap_target;
            self.look_up_rate_cap = cap;
        }
        let r = cap * dt + self.look_up_cap_remaining;
        self.look_up_cap_remaining = if 0.0 <= r { if cap * 0.0333 <= r { cap * 0.0333 } else { r } } else { 0.0 };
        if tgt == -1.0 && 1050.0 < cap {
            self.look_up_rate_cap = -1.0;
        }
    }

    /// AMordhauVehicle::AddTurnDegrees rva=0x1615760 -> AAdvancedCharacter::AddTurnDegrees rva=0x14574c0 for a player
    /// controller: the base yaw is the actor's (the control rotation's only when TurnLimit < 0), ClampAxis(ClampAxis
    /// (base) + Delta), clamped to SpawnTurnValue +/- TurnLimit by ClampAngle when TurnLimit >= 0, written as the
    /// control yaw
    pub fn add_turn_degrees(&mut self, delta: f32) {
        let base = if self.turn_limit < 0.0 { self.control_yaw } else { self.yaw };
        let mut y = fmod(base, 360.0);
        if y < 0.0 {
            y = y + 360.0;
        }
        let mut n = fmod(y + delta, 360.0);
        if n < 0.0 {
            n = n + 360.0;
        }
        if 0.0 <= self.turn_limit {
            n = clamp_angle(n, self.spawn_turn_value - self.turn_limit, self.spawn_turn_value + self.turn_limit);
        }
        self.control_yaw = n;
    }

    /// UAdvancedCharacterMovement::LODTick rva=0x1489530: PendingTurnValue != 0 and locally controlled ->
    /// AddTurnDegrees (vcall +0xa20), then PendingTurnValue = 0
    pub fn movement_lod_tick(&mut self, locally_controlled: bool) {
        if self.pending_turn_value != 0.0 {
            if locally_controlled {
                self.add_turn_degrees(self.pending_turn_value);
            }
            self.pending_turn_value = 0.0;
        }
    }

    /// UCharacterMovementComponent::PhysicsRotation rva=0x2f823e0 (bUseControllerDesiredRotation, RotationRate
    /// (0, -1, 0); the same reading as exe_horse's): the actor yaw FixedTurns toward the control yaw
    pub fn physics_rotation(&mut self, dt: f32) {
        let mut dy = fmod(self.control_yaw, 360.0);
        if dy < 0.0 {
            dy = dy + 360.0;
        }
        if dy > 180.0 {
            dy = dy - 360.0;
        }
        let r = normalize_axis_vm(self.yaw - dy);
        if !(r.abs() > 0.001) {
            return;
        }
        let rate = crate::exe_cmc::delta_rotation_yaw(dt);
        if (self.yaw - dy).abs() > 0.001 {
            self.yaw = fixed_turn(self.yaw, dy, rate);
        }
    }

    /// AMordhauVehicle::LODTick rva=0x162c590: on the authority, with no driver and no controller,
    /// LookUpValue = FInterpTo(LookUpValue, 0, LODDeltaTime, 4)
    pub fn vehicle_lod_tick(&mut self, dt: f32, authority: bool, driven: bool) {
        if authority && !driven && self.reset_look_up_when_no_driver {
            self.look_up_value = finterp_to(self.look_up_value, 0.0, dt, 4.0);
        }
    }
}

// ---- shooters ------------------------------------------------------------------------------------------------------

/// EWeaponState
pub mod weapon_state {
    pub const LOADED: u8 = 0;
    pub const RELEASING: u8 = 1;
    pub const RECOVERY: u8 = 2;
    pub const LOADING: u8 = 3;
    pub const BUILDING: u8 = 4;
}
use weapon_state::*;

/// The class-specific half
#[derive(Clone, Debug, PartialEq)]
pub enum ShooterKind {
    /// UProjectileTurretShooterComponent (ctor rva=0x14e6de0)
    Turret {
        weapon_kick_back_look_up: f32, // +0x294 (2)
        rotation_velocity: f32,        // +0x298
        previous_actor_yaw: f32,       // +0x2a8
        previous_pitch: f32,           // +0x2ac
    },
    /// UProjectileArmShooterComponent (ctor rva=0x14b0090)
    Arm {
        loaded_arm_min: f32,                 // +0x290 (0.6)
        arm_adjustment_min: i32,             // +0x294 (0)
        arm_adjustment_max: i32,             // +0x298 (100)
        arm_adjustment_step: i32,            // +0x29c (25)
        replicated_arm: u8,                  // +0x2d4
        arm_from: f32,                       // +0x2d8
        arm_target: f32,                     // +0x2dc (OnComponentCreated: LoadedArmMin)
        arm_raised_timestamp: f32,           // +0x2e0
        arm_lowered_timestamp: f32,          // +0x2e4
        last_arm_target: f32,                // +0x2e8
        arm_target_interpolation_speed: f32, // +0x2f8 (0.125)
    },
}

/// What the host does for the shooter
#[derive(Clone, Debug, PartialEq)]
pub enum ShooterEvent {
    /// spawn `fire ? FireProjectileClass : NormalProjectileClass` at the owner mesh's `socket` (world transform) and
    /// fire it (`Projectile::fire_quat`); `arm` = the catapult's ReplicatedArm at that moment (`catapult_initial_speed`)
    Spawn { fire: bool, socket: String, arm: Option<u8> },
    /// LookUp(value, true) on the owner (UProjectileTurretShooterComponent::OnWeaponStateRecovery_Implementation,
    /// when locally player controlled)
    KickBackLookUp(f32),
    /// the replicated WeaponState changed (OnRep_WeaponStateChanged ran)
    State(u8),
}

/// The pending stage timer (FTimerManager::InternalSetTimer(handle, delegate, StageTotalTime, false, -1))
#[derive(Clone, Copy, Debug, PartialEq)]
enum StageTimer {
    Releasing,
    Recovery,
    Loading,
    Building,
}

/// UProjectileShooterComponent (ctor rva=0x14e6a80) + its subclass
#[derive(Clone, Debug, PartialEq)]
pub struct Shooter {
    pub weapon_state: u8,             // +0xb0
    pub is_fire: bool,                // +0xb1
    pub building_stage_duration: f32, // +0xb4 (0)
    pub releasing_stage_duration: f32,
    pub recovery_stage_duration: f32,
    pub reloading_stage_duration: f32,
    pub projectile_socket_name: String, // +0xd0
    pub has_projectile_mesh: bool,      // Projectile +0xc8 != null (the BP construction script sets it)
    pub has_ammo: bool,                 // +0x1b8 (false)
    pub ammo_replenish_interval: f32,   // +0x1bc (2.5)
    pub max_ammo: f32,                  // +0x1c0 (5)
    pub ammo: u8,                       // +0x1c4 (5)
    pub replenishing_ammo: bool,        // +0x280
    pub stage_total_time: f32,          // +0x250
    pub stage_remaining_time: f32,      // +0x254
    pub kind: ShooterKind,
    /// FTimerManager InternalTime (+0x140, a double; the shooter's own clock; `tick_timers` advances it)
    pub timer_time: f64,
    timer: Option<(f64, StageTimer)>,
    ammo_timer: Option<f64>,
    pub events: Vec<ShooterEvent>,
}

fn fnum(v: &Value, k: &str) -> Option<f32> {
    v.get(k).and_then(Value::as_f64).map(|x| x as f32)
}

impl Shooter {
    fn base(kind: ShooterKind) -> Shooter {
        Shooter {
            weapon_state: LOADED,
            is_fire: false,
            building_stage_duration: 0.0,
            releasing_stage_duration: 0.0,
            recovery_stage_duration: 0.0,
            reloading_stage_duration: 0.0,
            projectile_socket_name: String::new(),
            has_projectile_mesh: true,
            has_ammo: false,
            ammo_replenish_interval: 2.5,
            max_ammo: 5.0,
            ammo: 5,
            replenishing_ammo: false,
            stage_total_time: 0.0,
            stage_remaining_time: 0.0,
            kind,
            timer_time: 0.0,
            timer: None,
            ammo_timer: None,
            events: Vec::new(),
        }
    }

    /// UProjectileTurretShooterComponent with its ctor values
    pub fn turret() -> Shooter {
        Shooter::base(ShooterKind::Turret { weapon_kick_back_look_up: 2.0, rotation_velocity: 0.0, previous_actor_yaw: 0.0, previous_pitch: 0.0 })
    }

    /// UProjectileArmShooterComponent with its ctor values (ArmTarget = LoadedArmMin: OnComponentCreated rva=0x14ca370)
    pub fn arm() -> Shooter {
        Shooter::base(ShooterKind::Arm {
            loaded_arm_min: 0.6,
            arm_adjustment_min: 0,
            arm_adjustment_max: 100,
            arm_adjustment_step: 25,
            replicated_arm: 0,
            arm_from: 0.0,
            arm_target: 0.6,
            arm_raised_timestamp: 0.0,
            arm_lowered_timestamp: 0.0,
            last_arm_target: 0.0,
            arm_target_interpolation_speed: 0.125,
        })
    }

    /// The shooter component of a siege-engine Blueprint (BP_Ballista "ProjectileTurretShooter_GEN_VARIABLE",
    /// BP_Catapult "ProjectileArmShooter_GEN_VARIABLE") over the ctor values
    pub fn from_bp_json(bp: &str) -> Result<Shooter, String> {
        let ex: Value = serde_json::from_str(bp).map_err(|e| format!("shooter json: {e}"))?;
        let arr = ex.as_array().ok_or("not an export list")?;
        let (comp, mut s) = if let Some(e) = arr.iter().find(|e| e["Name"] == "ProjectileTurretShooter_GEN_VARIABLE") {
            (e, Shooter::turret())
        } else if let Some(e) = arr.iter().find(|e| e["Name"] == "ProjectileArmShooter_GEN_VARIABLE") {
            (e, Shooter::arm())
        } else {
            return Err("no projectile shooter component".into());
        };
        let p = &comp["Properties"];
        let set = |k: &str, d: &mut f32| {
            if let Some(x) = fnum(p, k) {
                *d = x;
            }
        };
        set("BuildingStageDuration", &mut s.building_stage_duration);
        set("ReleasingStageDuration", &mut s.releasing_stage_duration);
        set("RecoveryStageDuration", &mut s.recovery_stage_duration);
        set("ReloadingStageDuration", &mut s.reloading_stage_duration);
        set("AmmoReplenishInterval", &mut s.ammo_replenish_interval);
        set("MaxAmmo", &mut s.max_ammo);
        if let Some(x) = p.get("bHasAmmo").and_then(Value::as_bool) {
            s.has_ammo = x;
        }
        if let Some(x) = p.get("Ammo").and_then(Value::as_u64) {
            s.ammo = x as u8;
        }
        if let Some(x) = p.get("ProjectileSocketName").and_then(Value::as_str) {
            s.projectile_socket_name = x.to_string();
        }
        match &mut s.kind {
            ShooterKind::Turret { weapon_kick_back_look_up, .. } => {
                if let Some(x) = fnum(p, "WeaponKickBackLookUp") {
                    *weapon_kick_back_look_up = x;
                }
            }
            ShooterKind::Arm { loaded_arm_min, arm_adjustment_min, arm_adjustment_max, arm_adjustment_step, arm_target, arm_target_interpolation_speed, .. } => {
                if let Some(x) = fnum(p, "LoadedArmMin") {
                    *loaded_arm_min = x;
                    *arm_target = x;
                }
                if let Some(x) = p.get("ArmAdjustmentMin").and_then(Value::as_i64) {
                    *arm_adjustment_min = x as i32;
                }
                if let Some(x) = p.get("ArmAdjustmentMax").and_then(Value::as_i64) {
                    *arm_adjustment_max = x as i32;
                }
                if let Some(x) = p.get("ArmAdjustmentStep").and_then(Value::as_i64) {
                    *arm_adjustment_step = x as i32;
                }
                if let Some(x) = fnum(p, "ArmTargetInterpolationSpeed") {
                    *arm_target_interpolation_speed = x;
                }
            }
        }
        Ok(s)
    }

    /// UProjectileShooterComponent::BeginPlay rva=0x14f12e0: BuildingStageDuration > 0 on the authority -> Building
    /// (4) for that long (no OnRep), then OnBuildingStageCompleted rva=0x1507d60: Loaded (no OnRep either)
    pub fn begin_play(&mut self, authority: bool) {
        if 0.0 < self.building_stage_duration && authority {
            self.weapon_state = BUILDING;
            self.set_timer(self.building_stage_duration, StageTimer::Building);
        }
    }

    fn set_timer(&mut self, rate: f32, t: StageTimer) {
        self.timer = Some((self.timer_time + rate as f64, t));
    }

    fn is_arm(&self) -> bool {
        matches!(self.kind, ShooterKind::Arm { .. })
    }

    /// OnRep_WeaponStateChanged rva=0x150ef90 (the stage's OnWeaponState*_Implementation, then StageRemainingTime =
    /// StageTotalTime; the cosmetic part not ported)
    fn on_rep_weapon_state(&mut self, locally_controlled: bool) {
        match self.weapon_state {
            LOADED => {}
            RELEASING => {
                // base 0x15109e0 (+ arm 0x14cfaf0: ArmFrom = ArmTarget)
                self.stage_total_time = self.releasing_stage_duration;
                if let ShooterKind::Arm { arm_from, arm_target, .. } = &mut self.kind {
                    *arm_from = *arm_target;
                }
            }
            RECOVERY => {
                // base 0x1510880 / turret 0x1510890 (+ LookUp(WeaponKickBackLookUp, true) when locally controlled)
                self.stage_total_time = self.recovery_stage_duration;
                if let ShooterKind::Turret { weapon_kick_back_look_up, .. } = self.kind {
                    if locally_controlled {
                        self.events.push(ShooterEvent::KickBackLookUp(weapon_kick_back_look_up));
                    }
                }
            }
            LOADING => self.stage_total_time = self.reloading_stage_duration, // base 0x1510760 / turret 0x1510770
            _ => {}                                                             // Building: no-op (0x7bf350)
        }
        self.stage_remaining_time = self.stage_total_time;
        self.events.push(ShooterEvent::State(self.weapon_state));
    }

    /// UProjectileShooterComponent::OnFirePressed rva=0x15087a0: Loaded, ammo (if counted) and a projectile mesh ->
    /// FireProjectile (server RPC -> FireProjectile_Implementation); returns whether it fired
    pub fn on_fire_pressed(&mut self, locally_controlled: bool, controller: bool) -> bool {
        if self.weapon_state == LOADED && (!self.has_ammo || self.ammo != 0) && self.has_projectile_mesh {
            self.fire_projectile(locally_controlled, controller);
            return true;
        }
        false
    }

    /// FireProjectile_Implementation: base rva=0x14f8930 (ammo, Releasing), turret rva=0x14f8de0 (after the base: spawn
    /// at ProjectileSocketName), arm rva=0x14bb7e0 (FiredController = the owner's controller when Releasing).
    /// `controller` = the owner pawn has a controller (the arm's FiredController; not otherwise used here)
    pub fn fire_projectile(&mut self, locally_controlled: bool, controller: bool) {
        let _ = controller;
        if self.weapon_state != LOADED || (self.has_ammo && self.ammo == 0) {
            return;
        }
        self.base_fire_projectile(locally_controlled);
        if let ShooterKind::Turret { .. } = self.kind {
            // UpdateProjectileTransform (the FireLocation for the camera shake) then SpawnActor at the socket
            let fire = self.is_fire;
            self.events.push(ShooterEvent::Spawn { fire, socket: self.projectile_socket_name.clone(), arm: None });
            self.is_fire = false;
        }
    }

    fn base_fire_projectile(&mut self, locally_controlled: bool) {
        if self.weapon_state != LOADED {
            return;
        }
        if self.has_ammo {
            if self.ammo == 0 {
                return;
            }
            self.ammo -= 1;
            if !self.replenishing_ammo {
                self.replenishing_ammo = true;
                self.ammo_timer = Some(self.timer_time + self.ammo_replenish_interval as f64);
            }
        }
        self.weapon_state = RELEASING;
        self.on_rep_weapon_state(locally_controlled);
        if self.stage_total_time <= 0.0 {
            self.on_releasing_completed(locally_controlled);
        } else {
            self.set_timer(self.stage_total_time, StageTimer::Releasing);
        }
    }

    /// OnReleasingStageCompleted_Implementation: base rva=0x150e140 (Recovery), arm rva=0x14ce680 (after the base:
    /// spawn at "FireSocket" with the creator's ReplicatedArm)
    fn on_releasing_completed(&mut self, locally_controlled: bool) {
        self.weapon_state = RECOVERY;
        self.on_rep_weapon_state(locally_controlled);
        if self.stage_total_time <= 0.0 {
            self.on_recovery_completed(locally_controlled);
        } else {
            self.set_timer(self.stage_total_time, StageTimer::Recovery);
        }
        if let ShooterKind::Arm { replicated_arm, .. } = self.kind {
            let fire = self.is_fire;
            self.events.push(ShooterEvent::Spawn { fire, socket: "FireSocket".into(), arm: Some(replicated_arm) });
            self.is_fire = false;
        }
    }

    /// OnRecoveryStageCompleted_Implementation rva=0x150d8e0: Loading
    fn on_recovery_completed(&mut self, locally_controlled: bool) {
        self.weapon_state = LOADING;
        self.on_rep_weapon_state(locally_controlled);
        if self.stage_total_time <= 0.0 {
            self.on_loading_completed(locally_controlled);
        } else {
            self.set_timer(self.stage_total_time, StageTimer::Loading);
        }
    }

    /// OnLoadingStageCompleted_Implementation rva=0x150ccb0: Loaded
    fn on_loading_completed(&mut self, locally_controlled: bool) {
        self.weapon_state = LOADED;
        self.on_rep_weapon_state(locally_controlled);
    }

    /// The timer manager's tick for this shooter's timers: FTimerManager::Tick rva=0x34df980 (read off the disassembly):
    /// InternalTime (double) += (double)DeltaTime (0x1434df9f9), then the timer at the heap top fires while
    /// !(ExpireTime >= InternalTime) (comisd at 0x1434dfb83); InternalSetTimer's ExpireTime = InternalTime + (double)
    /// Rate. ReplenishAmmo rva=0x1514870: Ammo + 1, done at MaxAmmo, else re-armed for AmmoReplenishInterval
    pub fn tick_timers(&mut self, dt: f32, locally_controlled: bool) {
        self.timer_time = self.timer_time + dt as f64;
        while let Some((expire, t)) = self.timer {
            if !(self.timer_time > expire) {
                break;
            }
            self.timer = None;
            match t {
                StageTimer::Releasing => self.on_releasing_completed(locally_controlled),
                StageTimer::Recovery => self.on_recovery_completed(locally_controlled),
                StageTimer::Loading => self.on_loading_completed(locally_controlled),
                StageTimer::Building => self.weapon_state = LOADED,
            }
        }
        while let Some(expire) = self.ammo_timer {
            if !(self.timer_time > expire) {
                break;
            }
            self.ammo_timer = None;
            self.ammo = self.ammo.wrapping_add(1);
            if self.ammo as f32 == self.max_ammo {
                self.replenishing_ammo = false;
            } else {
                self.ammo_timer = Some(self.timer_time + self.ammo_replenish_interval as f64);
            }
        }
    }

    /// OnLODTick (the owner character's LODTick delegate): base rva=0x150c600 (StageRemainingTime counts down while
    /// not Loaded), turret rva=0x150c650 (the owner alive: RotationVelocity = (PreviousActorYaw - mesh yaw) / dt,
    /// PreviousPitch = LookUpValue), arm rva=0x14cbaf0 (dead -> nothing more; the ArmTarget by stage, as
    /// UpdateArmTarget rva=0x14dbfb0 reads off the disassembly: Loaded FInterpConstantTo(ArmTarget, ReplicatedArm *
    /// (1 - LoadedArmMin) * 0.01 + LoadedArmMin, dt, ArmTargetInterpolationSpeed); Releasing Remaining / Total *
    /// ArmFrom; Recovery 0 (and ReplicatedArm = 0 on the authority); Loading UKismetMathLibrary::Ease(LoadedArmMin, 0,
    /// Remaining / Total, EaseIn, 2) = (0 - LoadedArmMin) * powf(alpha, 2) + LoadedArmMin)
    pub fn lod_tick(&mut self, dt: f32, dead: bool, authority: bool, mesh_yaw: f32, look_up_value: f32) {
        if self.weapon_state != LOADED {
            let r = self.stage_remaining_time - dt;
            self.stage_remaining_time = if r <= 0.0 { 0.0 } else { r };
        }
        let (state, rem, total) = (self.weapon_state, self.stage_remaining_time, self.stage_total_time);
        match &mut self.kind {
            ShooterKind::Turret { rotation_velocity, previous_actor_yaw, previous_pitch, .. } => {
                if !dead {
                    let prev = *previous_actor_yaw;
                    *previous_actor_yaw = mesh_yaw;
                    *rotation_velocity = (prev - mesh_yaw) / dt;
                    *previous_pitch = look_up_value;
                }
            }
            ShooterKind::Arm { loaded_arm_min, replicated_arm, arm_from, arm_target, last_arm_target, arm_target_interpolation_speed, .. } => {
                if dead {
                    return;
                }
                match state {
                    LOADED => {
                        let goal = (*replicated_arm as i32) as f32 * (1.0 - *loaded_arm_min) * 0.01 + *loaded_arm_min;
                        *arm_target = finterp_constant_to(*arm_target, goal, dt, *arm_target_interpolation_speed);
                    }
                    RELEASING => *arm_target = (rem / total) * *arm_from,
                    RECOVERY => {
                        *arm_target = 0.0;
                        if authority {
                            *replicated_arm = 0;
                        }
                    }
                    LOADING => {
                        let a = rem / total;
                        *arm_target = (0.0 - *loaded_arm_min) * (a * a) + *loaded_arm_min;
                    }
                    _ => {}
                }
                *last_arm_target = *arm_target;
            }
        }
    }

    /// UProjectileArmShooterComponent::AdjustArm rva=0x14b2d40 / LowerArm_Implementation rva=0x14c8f20 (+ step) /
    /// RaiseArm_Implementation rva=0x14d0a80 (- step): Loaded only, clamp(ReplicatedArm + delta, Min, Max) (the
    /// upper bound tested with <, so Max itself is reached through the clamp)
    pub fn adjust_arm(&mut self, delta: i32) {
        if self.weapon_state != LOADED {
            return;
        }
        if let ShooterKind::Arm { replicated_arm, arm_adjustment_min, arm_adjustment_max, .. } = &mut self.kind {
            let n = *replicated_arm as i32 + delta;
            let r = if *arm_adjustment_min <= n { if n < *arm_adjustment_max { n } else { *arm_adjustment_max } } else { *arm_adjustment_min };
            *replicated_arm = r as u8;
        }
    }

    /// OnLowerArmPressed rva=0x14cc2b0 / OnRaiseArmPressed rva=0x14cc9a0: Loaded and 0.4 s since the last press ->
    /// LowerArm (+step) / RaiseArm (-step), timestamp = now
    pub fn on_arm_pressed(&mut self, lower: bool, now: f32) {
        if self.weapon_state != LOADED {
            return;
        }
        let step = match &self.kind {
            ShooterKind::Arm { arm_adjustment_step, arm_lowered_timestamp, arm_raised_timestamp, .. } => {
                let ts = if lower { *arm_lowered_timestamp } else { *arm_raised_timestamp };
                if !(ts + 0.4 < now) {
                    return;
                }
                *arm_adjustment_step
            }
            _ => return,
        };
        self.adjust_arm(if lower { step } else { -step });
        if let ShooterKind::Arm { arm_lowered_timestamp, arm_raised_timestamp, .. } = &mut self.kind {
            if lower {
                *arm_lowered_timestamp = now;
            } else {
                *arm_raised_timestamp = now;
            }
        }
    }

    /// the catapult arm's ReplicatedArm (0 for a turret)
    pub fn arm_value(&self) -> u8 {
        match self.kind {
            ShooterKind::Arm { replicated_arm, .. } => replicated_arm,
            _ => 0,
        }
    }

    /// the arm's animated ArmTarget (0 for a turret)
    pub fn arm_target(&self) -> f32 {
        match self.kind {
            ShooterKind::Arm { arm_target, .. } => arm_target,
            _ => 0.0,
        }
    }

    /// whether the projectile mesh shows (UProjectileArmShooterComponent::UpdateProjectileVisibility rva=0x14de9f0:
    /// hidden in Recovery / Loading or out of ammo)
    pub fn projectile_hidden(&self) -> bool {
        (self.weapon_state.wrapping_sub(2) < 2) || (self.has_ammo && self.ammo == 0)
    }

    /// whether the stage machine has a pending timer (tests / hosts)
    pub fn busy(&self) -> bool {
        self.timer.is_some()
    }

    /// drain the events
    pub fn take_events(&mut self) -> Vec<ShooterEvent> {
        std::mem::take(&mut self.events)
    }

    /// is this the catapult's arm
    pub fn is_arm_shooter(&self) -> bool {
        self.is_arm()
    }
}

/// BP_CatapultProjectile SetProjectileInitialSpeed (bytecode @0..@231): InitialSpeed * (5 + Conv_ByteToFloat(arm) /
/// 100 * 5), set before the parent Fire initializes the velocity
pub fn catapult_initial_speed(initial_speed: f32, arm: u8) -> f32 {
    let a = (arm as f32) / 100.0;
    let m = a * 5.0;
    initial_speed * (5.0 + m)
}

// ---- the ballista --------------------------------------------------------------------------------------------------

/// One frame's input to a siege engine (the driver's axes and actions, routed by the BP input bindings)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SiegeInput {
    pub turn: f32,
    pub look_up: f32,
    pub fire: bool,
    pub lower_arm: bool,
    pub raise_arm: bool,
}

/// BP_Ballista: the aim + the turret shooter
#[derive(Clone, Debug, PartialEq)]
pub struct SiegeEngine {
    pub aim: VehicleAim,
    pub shooter: Shooter,
    /// a driver sits in it (UMordhauVehicleComponent::Driver)
    pub driven: bool,
    /// the driver is the local player (IsLocallyPlayerControlled)
    pub locally_controlled: bool,
    pub authority: bool,
    pub dead: bool,
    /// the controller's InputYawScale / InputPitchScale (AMordhauPlayerController; None for AI)
    pub input_yaw_scale: Option<f32>,
    pub input_pitch_scale: Option<f32>,
    pub world_time: f32,
}

impl SiegeEngine {
    /// A siege engine from its Blueprint package JSON (BP_Ballista / BP_Catapult: the CDO's LookUpLimit,
    /// LookDownLimit, TurnLimit, TurnRateCap, LookUpRateCap over the AAdvancedCharacter ctor rva=0x144b0e0 values
    /// (-1 caps / turn limit), and the shooter component) at `yaw`
    pub fn from_bp_json(bp: &str, yaw: f32) -> Result<SiegeEngine, String> {
        let ex: Value = serde_json::from_str(bp).map_err(|e| format!("siege json: {e}"))?;
        let cdo = ex
            .as_array()
            .and_then(|a| a.iter().find(|e| e["Name"].as_str().is_some_and(|n| n.starts_with("Default__"))))
            .map(|e| &e["Properties"])
            .ok_or("no CDO")?;
        let g = |k: &str, d: f32| fnum(cdo, k).unwrap_or(d);
        // AAdvancedCharacter ctor rva=0x144b0e0 writes TurnLimit -1 (0x14144b442) but not LookUpLimit / LookDownLimit
        // (+0x8a0 / +0x8a4): 0 unless the BP sets them
        let aim = VehicleAim::new(yaw, g("LookUpLimit", 0.0), g("LookDownLimit", 0.0), g("TurnLimit", -1.0), g("TurnRateCap", -1.0), g("LookUpRateCap", -1.0));
        Ok(SiegeEngine {
            aim,
            shooter: Shooter::from_bp_json(bp)?,
            driven: false,
            locally_controlled: false,
            authority: true,
            dead: false,
            input_yaw_scale: None,
            input_pitch_scale: None,
            world_time: 0.0,
        })
    }

    /// One frame in the exe's tick order (exe.rs frame docs): the controller's input, the movement component, the
    /// actor (LODTick with its OnLODTick delegate), then the timer manager. Returns the shooter events (spawns,
    /// kickbacks, states).
    pub fn frame(&mut self, dt: f32, inp: &SiegeInput) -> Vec<ShooterEvent> {
        self.world_time = self.world_time + dt;
        let mut seen = self.shooter.events.len();
        if self.driven && !self.dead {
            self.aim.look_up(inp.look_up, false, false, self.input_pitch_scale);
            self.aim.turn(inp.turn, false, false, self.input_yaw_scale);
            if inp.fire {
                self.shooter.on_fire_pressed(self.locally_controlled, true);
            }
            if inp.lower_arm {
                self.shooter.on_arm_pressed(true, self.world_time);
            }
            if inp.raise_arm {
                self.shooter.on_arm_pressed(false, self.world_time);
            }
        }
        seen = self.apply_kickbacks(seen);
        // the movement component (before the actor: bTickBeforeOwner, exe.rs frame docs)
        self.aim.movement_lod_tick(self.locally_controlled);
        self.aim.physics_rotation(dt);
        // the actor: AMordhauVehicle::LODTick -> AAdvancedCharacter::LODTick rva=0x14887b0 (OnLODTick broadcast first,
        // the shooter's OnLODTick; then the cap refill), then the look-up reset
        self.shooter.lod_tick(dt, self.dead, self.authority, self.aim.yaw, self.aim.look_up_value);
        self.aim.lod_tick_caps(dt);
        self.aim.vehicle_lod_tick(dt, self.authority, self.driven);
        // the timer manager after the tick groups (UWorld::Tick 0x1431b1739)
        self.shooter.tick_timers(dt, self.locally_controlled);
        self.apply_kickbacks(seen);
        self.shooter.take_events()
    }

    /// LookUp(WeaponKickBackLookUp, true) for the kickback events from index `from` on; returns the new end
    fn apply_kickbacks(&mut self, from: usize) -> usize {
        let n = self.shooter.events.len();
        for i in from..n {
            if let ShooterEvent::KickBackLookUp(k) = self.shooter.events[i] {
                self.aim.look_up(k, true, false, None);
            }
        }
        n
    }
}

// ---- AVehicleBase net smoothing (legacy) ---------------------------------------------------------------------------

/// FNetState (0x28 bytes)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetState {
    pub timestamp: f32,       // +0x0
    pub local_timestamp: f32, // +0x4
    pub position: FVector,    // +0x8
    pub rotation: [f32; 3],   // +0x14 pitch, yaw, roll
    pub pitch: u16,           // +0x20
    pub yaw: u16,             // +0x22
    pub roll: u8,             // +0x24
}

impl NetState {
    /// AVehicleBase::CreateNetState rva=0x1670bf0: Timestamp = time, the mesh's world location and rotator, the
    /// compressed pitch / yaw (* 182.04445, RoundToInt, 16 bit) and roll (* 0.7111111, 8 bit)
    pub fn create(time: f32, position: FVector, rotation: [f32; 3]) -> NetState {
        let p = rotation[0] * f32::from_bits(0x4336_0b61);
        let y = rotation[1] * f32::from_bits(0x4336_0b61);
        let r = rotation[2] * f32::from_bits(0x3f36_0b61);
        NetState { timestamp: time, local_timestamp: 0.0, position, rotation, pitch: round_to_int(p) as u16, yaw: round_to_int(y) as u16, roll: round_to_int(r) as u8 }
    }
}

/// The client-side smoothing of AVehicleBase (StateQueue, LerpStartState, LerpBeginTime)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleNet {
    pub queue: Vec<NetState>,     // +0x448 StateQueue (max 10)
    pub lerp_start: NetState,     // +0x41c
    pub lerp_begin_time: f32,     // +0x490
    pub net_time_behind: f32,     // +0x480
    pub net_send_rate: f32,       // +0x484
}

impl VehicleNet {
    /// AVehicleBase::AddStateToQueue rva=0x1670360: below 10 queued, Timestamp += NetTimeBehind; the first gets
    /// LocalTimestamp = now + NetTimeBehind and sets LerpBeginTime = now + NetTimeBehind - NetSendRate, later ones
    /// LocalTimestamp = Timestamp - last.Timestamp + last.LocalTimestamp ("Flooded" otherwise)
    pub fn add_state(&mut self, mut s: NetState, now: f32) {
        if self.queue.len() >= 10 {
            return;
        }
        let ts = s.timestamp + self.net_time_behind;
        s.timestamp = ts;
        if let Some(last) = self.queue.last() {
            s.local_timestamp = (ts - last.timestamp) + last.local_timestamp;
            self.queue.push(s);
        } else {
            s.local_timestamp = now + self.net_time_behind;
            self.queue.push(s);
            self.lerp_begin_time = (now + self.net_time_behind) - self.net_send_rate;
        }
    }

    /// AVehicleBase::SyncPhysics rva=0x1675190 (module docs). `current` = the mesh's world location and rotator now
    /// (CreateNetState's source). Returns the actor location / rotation to set, None when nothing is due.
    pub fn sync_physics(&mut self, now: f32, dt: f32, current: (FVector, [f32; 3])) -> Option<(FVector, [f32; 3])> {
        if self.queue.is_empty() || now <= self.lerp_begin_time {
            return None;
        }
        if self.lerp_start.timestamp == 0.0 {
            self.lerp_start = NetState::create(now - dt, current.0, current.1);
        }
        let mut tgt = self.queue[0];
        let mut alpha = Self::alpha(now, self.lerp_start.timestamp, tgt.local_timestamp);
        loop {
            if alpha < 0.99 && now <= tgt.local_timestamp {
                let s = self.lerp_start;
                let z = (tgt.position.z - s.position.z) * alpha + s.position.z;
                let y = (tgt.position.y - s.position.y) * alpha + s.position.y;
                let x = (tgt.position.x - s.position.x) * alpha + s.position.x;
                let mut rot = [0.0f32; 3];
                for k in 0..3 {
                    rot[k] = normalize_axis_vm(tgt.rotation[k] - s.rotation[k]) * alpha + s.rotation[k];
                }
                let (dy, dx, dz) = (tgt.position.y - y, tgt.position.x - x, tgt.position.z - z);
                if 10000.0 < (dy * dy + dx * dx) + dz * dz {
                    return Some((tgt.position, tgt.rotation));
                }
                return Some((v(x, y, z), rot));
            }
            self.queue.remove(0);
            self.lerp_start = NetState::create(now - dt, current.0, current.1);
            if self.queue.is_empty() {
                return None;
            }
            tgt = self.queue[0];
            alpha = Self::alpha(now, self.lerp_start.timestamp, tgt.local_timestamp);
        }
    }

    /// (now - start) / (target - start) clamped to [0, 1]; a degenerate span gives 0 before the target, else 1
    fn alpha(now: f32, start: f32, target: f32) -> f32 {
        let d = target - start;
        let a = if 1e-8 < d.abs() {
            (now - start) / d
        } else if now < target {
            0.0
        } else {
            1.0
        };
        if a < 0.0 {
            0.0
        } else if 1.0 <= a {
            1.0
        } else {
            a
        }
    }
}

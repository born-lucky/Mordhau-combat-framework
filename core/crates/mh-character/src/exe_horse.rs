//! Horses (AHorse : AMordhauVehicle : AAdvancedCharacter, movement UHorseMovementComponent :
//! UPseudoVehicleMovementComponent : UAdvancedCharacterMovement) in exe mode. A horse is an `ExeMovement` with
//! `horse: Some(..)`: the engine movement (walking, falling, floors, steps, collision) is the same port, and the class
//! overrides switch on it:
//!  - GetMaxSpeed = UAdvancedCharacterMovement::GetMaxSpeed rva=0x1484b20 (UCharacterMovementComponent's: MaxWalkSpeed,
//!    which the gear sets), GetMaxAcceleration = UCharacterMovementComponent's (MaxAcceleration, set by the gear;
//!    rva=0x1596d00), GetMaxBrakingDeceleration = UCharacterMovementComponent's rva=0x2f79940;
//!  - UHorseMovementComponent::LODTick rva=0x14c28e0: turning (curves by speed, FInterpConstantTo of TurningVelocity),
//!    front / rear capsule sweeps and the soft-bubble avoidance of other pawns, the head-on rearing, the gear by speed;
//!  - UAdvancedCharacterMovement::LODTick rva=0x1489530: PendingTurnValue -> AMordhauVehicle::AddTurnDegrees
//!    rva=0x1615760 -> AAdvancedCharacter::AddTurnDegrees rva=0x14574c0 (control yaw);
//!  - UCharacterMovementComponent::PhysicsRotation rva=0x2f823e0 (bUseControllerDesiredRotation, set by the
//!    UAdvancedCharacterMovement ctor at 0x14144dad3): the actor yaw follows the control yaw at RotationRate (0, -1, 0): instant, `exe_cmc::delta_rotation_yaw`;
//!  - UHorseMovementComponent::DoJump rva=0x14ba690 (the gear's bAllowJump), AHorse::IsMoveInputIgnored rva=0x14fe200,
//!    AHorse::MoveForward rva=0x14ffae0 (the gear shifting), AHorse::MoveRight rva=0x14ffe70 -> AAdvancedCharacter::Turn
//!    rva=0x14a8a90, AHorse::LODTick rva=0x14fe470, the BP_Horse Jump bindings (jump when fast, rear when slow).
//! The UPseudoVehicleMovementComponent overrides (MoveUpdatedComponentImpl, OverlapTest, FloorSweepTest,
//! PhysicsRotation) fall back to their parents when SecondaryComponents is empty, which it is for BP_Horse (no native
//! code or Blueprint fills it): they are the engine's here.
//!
//! Mounting (UMordhauVehicleComponent): CanInteract rva=0x14b4c50 / UHorseVehicleComponent::CanDrive_Implementation
//! rva=0x14b4aa0, StartDriving rva=0x14d79a0 (seat transform), StopDriving rva=0x14d8270 (exit transform rva=0x14bc740).
//! The combat side (AHorse::DoKnockback rva=0x14f66e0, the rider's equipment holstering, mounted attacks) is
//! rust-combat's; this module gives it the bump speed and applies the velocity multipliers.

use crate::exe::{ExeInput, ExeMovement, Mode};
use crate::exe_lod::OtherPawn;
use crate::records::CharacterRecords;
use crate::ue::FVector;
use crate::uemath::*;
use crate::uequat::*;
use crate::world::World;
use serde_json::Value;

/// FHorseGearInfo (0x18 bytes): MaxSpeed, MaxAcceleration, bAllowJump, bCanRiderRegenHealth, bCanRiderRegenStamina,
/// bCanHorseRegen
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GearInfo {
    pub max_speed: f32,
    pub max_acceleration: f32,
    pub allow_jump: bool,
    pub can_rider_regen_health: bool,
    pub can_rider_regen_stamina: bool,
    pub can_horse_regen: bool,
}

/// The horse's data: BP_Horse (CharMoveComp, Default__BP_Horse_C) over the native ctors (UCharacterMovementComponent
/// rva=0x2f6cc10, UAdvancedCharacterMovement rva=0x144da20, UPseudoVehicleMovementComponent rva=0x14e6e90,
/// UHorseMovementComponent rva=0x14ae880, AHorse rva=0x14e36f0, AAdvancedCharacter rva=0x144a(..)), binary32.
#[derive(Clone, Debug, PartialEq)]
pub struct HorseCfg {
    pub gears: Vec<GearInfo>,                       // +0xcd8
    pub turning_brake_curve: Option<Vec<RichKey>>,  // +0xc70 TurningBrakeAccelerationByVelocity
    pub turning_factor_curve: Option<Vec<RichKey>>, // +0xc78 TurningFactorByVelocity
    pub turning_accel_curve: Option<Vec<RichKey>>,  // +0xc80 TurningAccelerationByVelocity
    pub turning_factor_scale_airborne: f32,         // +0xc88 (pseudo ctor 0.1)
    pub head_on_min_speed_to_rear: f32,             // +0xc90
    pub soft_bubble_rel: FVector,                   // +0xc94 (ctor (-80, 0, 0))
    pub soft_bubble_length: f32,                    // +0xca0 (220)
    pub soft_bubble_radius: f32,                    // +0xca4 (70)
    pub soft_bubble_max_height: f32,                // +0xca8 (150)
    pub front_rear_half_height: f32,                // +0xcac (60)
    pub front_rear_radius: f32,                     // +0xcb0 (55)
    pub front_rel: FVector,                         // +0xcb4 (110, 0, 40)
    pub rear_rel: FVector,                          // +0xcc0 (-50, 0, 20)
    pub avoidance_turning_acceleration: f32,        // +0xccc (10)
    pub speed_multiplier_on_bump: f32,              // +0xcec
    pub speed_multiplier_on_melee: f32,             // +0xcf0
    // AHorse
    pub bump_damage_curve: Option<Vec<RichKey>>,    // +0xc18 BumpDamageBySpeedModifierCurve
    pub knockback_force: f32,                       // +0xc30
    pub knockback_force_velocity_factor: f32,       // +0xc34
    pub knockback_damage: f32,                      // +0xc38
    pub rearing_duration: f32,                      // +0xc48
    pub uncontrolled_gear: i32,                     // +0xc2c
    // UMordhauVehicleComponent (BP_VehicleHorse)
    pub attach_offset: FVector,                     // AttachSocketOffset.Translation (+0xc0 ..)
    pub min_xy_distance_to_enter: f32,              // +0x128
    pub min_z_distance_to_enter: (f32, f32),        // +0x12c
    pub minimum_interactable_velocity: f32,         // +0x134
    pub mesh_relative: FVector,                     // CharacterMesh0 RelativeLocation (the seat's base)
}

/// The per-horse state the native classes keep.
#[derive(Clone, Debug, PartialEq)]
pub struct HorseState {
    pub cfg: HorseCfg,
    pub gear: u8,                    // UHorseMovementComponent +0xcd0 Gear
    pub move_desired_gear: u8,       // +0xce8 DesiredGear (written by AHorse::MoveForward)
    pub desired_gear: i32,           // AHorse +0xc28 DesiredGear (-1 = reverse)
    pub turning_velocity: f32,       // +0xc68
    pub pending_turn_value: f32,     // UAdvancedCharacterMovement +0xbc0
    pub control_yaw: f32,            // the controller's ControlRotation.Yaw
    pub target_control_yaw: f32,     // AAdvancedCharacter +0x524 TargetControlYaw
    pub last_rearing_time: f32,      // AHorse +0xc54
    pub last_rearing_real_time: f32, // +0xc58
    pub is_rearing: bool,            // +0xc5c
    pub replicated_rearing: u8,      // +0xc4c
    pub rear_requests: u32,          // ServerRequestRearing calls (for the host / net)
    /// a player controller possesses the horse (IsLocallyPlayerControlled / IsControlled)
    pub controlled: bool,
    pub jump_held: bool,
    pub driver: Option<usize>,       // UMordhauVehicleComponent +0x168 Driver (host id)
    /// the driver's current motion is a ULeaveVehicleMotion / UEnterVehicleMotion (host: mordhau-core World::cur_m(fi)
    /// kind "EnterVehicle" / "LeaveVehicle"); AHorse::RequestRearing returns early then
    pub driver_entering_or_leaving: bool,
    /// the last sweep / bubble results, for tests and the host
    pub last_front_hit: bool,
    pub last_rear_hit: bool,
    pub last_push: FVector,
}

/// The horse inputs of one frame: the forward / right axes and the Jump action button (held state).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HorseInput {
    pub fwd: f32,
    pub right: f32,
    pub jump: bool,
}

// ---- records -----------------------------------------------------------------------------------------------------

fn named<'a>(exports: &'a Value, name: &str) -> Option<&'a Value> {
    exports.as_array()?.iter().find(|e| e["Name"] == name).map(|e| &e["Properties"])
}
fn fnum(v: &Value, k: &str, d: f32) -> f32 {
    v.get(k).and_then(Value::as_f64).map_or(d, |x| x as f32)
}
fn vec3(v: &Value, k: &str, d: FVector) -> FVector {
    match v.get(k) {
        Some(x) => v3(fnum(x, "X", d.x), fnum(x, "Y", d.y), fnum(x, "Z", d.z)),
        None => d,
    }
}
fn v3(x: f32, y: f32, z: f32) -> FVector {
    v(x, y, z)
}
/// the package path of an object reference ({ObjectPath: "<pkg>.<index>"})
fn ref_path(v: &Value, k: &str) -> Option<String> {
    let p = v.get(k)?.get("ObjectPath")?.as_str()?;
    Some(p.rsplit_once('.').map_or(p, |(a, _)| a).to_string())
}

/// FloatCurve keys of a CurveFloat package export list (extract/json)
pub fn curve_keys_from_json(json: &str) -> Option<Vec<RichKey>> {
    let a: Value = serde_json::from_str(json).ok()?;
    let fc = a.as_array()?.iter().find_map(|e| e.get("Properties")?.get("FloatCurve"))?;
    let keys = fc.get("Keys")?.as_array()?;
    Some(
        keys.iter()
            .map(|k| RichKey {
                interp: match k.get("InterpMode").and_then(Value::as_str).unwrap_or("RCIM_Linear") {
                    "RCIM_Linear" => 0,
                    "RCIM_Constant" => 1,
                    "RCIM_Cubic" => 2,
                    _ => 3,
                },
                weight_mode: match k.get("TangentWeightMode").and_then(Value::as_str).unwrap_or("RCTWM_WeightedNone") {
                    "RCTWM_WeightedNone" => 0,
                    "RCTWM_WeightedArrive" => 1,
                    "RCTWM_WeightedLeave" => 2,
                    _ => 3,
                },
                time: fnum(k, "Time", 0.0),
                value: fnum(k, "Value", 0.0),
                arrive: fnum(k, "ArriveTangent", 0.0),
                leave: fnum(k, "LeaveTangent", 0.0),
            })
            .collect(),
    )
}

impl HorseCfg {
    /// BP_Horse.json (CUE4Parse export list) + BP_VehicleHorse.json; `curve(package path)` returns a CurveFloat's
    /// package JSON. Absent properties take the native ctor values (cited per field).
    pub fn from_json(bp_horse: &str, bp_vehicle: Option<&str>, curve: &dyn Fn(&str) -> Option<String>) -> Result<HorseCfg, String> {
        let ex: Value = serde_json::from_str(bp_horse).map_err(|e| format!("BP_Horse json: {e}"))?;
        let cmc = named(&ex, "CharMoveComp").ok_or("BP_Horse has no CharMoveComp")?;
        let cdo = named(&ex, "Default__BP_Horse_C").ok_or("BP_Horse has no CDO")?;
        let load_curve = |o: &Value, k: &str| ref_path(o, k).and_then(|p| curve(&p)).and_then(|j| curve_keys_from_json(&j));
        // UHorseMovementComponent ctor rva=0x14ae880: four (700, 1000, all true) gears; BP_Horse replaces them
        let gears = match cmc.get("GearInfo").and_then(Value::as_array) {
            Some(a) => a
                .iter()
                .map(|g| GearInfo {
                    max_speed: fnum(g, "MaxSpeed", 0.0),
                    max_acceleration: fnum(g, "MaxAcceleration", 0.0),
                    allow_jump: g.get("bAllowJump").and_then(Value::as_bool).unwrap_or(false),
                    can_rider_regen_health: g.get("bCanRiderRegenHealth").and_then(Value::as_bool).unwrap_or(false),
                    can_rider_regen_stamina: g.get("bCanRiderRegenStamina").and_then(Value::as_bool).unwrap_or(false),
                    can_horse_regen: g.get("bCanHorseRegen").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect(),
            None => vec![GearInfo { max_speed: 700.0, max_acceleration: 1000.0, allow_jump: true, can_rider_regen_health: true, can_rider_regen_stamina: true, can_horse_regen: true }; 4],
        };
        let veh: Value = match bp_vehicle {
            Some(j) => {
                let a: Value = serde_json::from_str(j).map_err(|e| format!("BP_VehicleHorse json: {e}"))?;
                a.as_array().and_then(|a| a.iter().find(|e| e["Name"].as_str().is_some_and(|n| n.starts_with("Default__")))).map(|e| e["Properties"].clone()).unwrap_or(Value::Null)
            }
            None => Value::Null,
        };
        let mesh = named(&ex, "CharacterMesh0").cloned().unwrap_or(Value::Null);
        Ok(HorseCfg {
            gears,
            turning_brake_curve: load_curve(cmc, "TurningBrakeAccelerationByVelocity"),
            turning_factor_curve: load_curve(cmc, "TurningFactorByVelocity"),
            turning_accel_curve: load_curve(cmc, "TurningAccelerationByVelocity"),
            turning_factor_scale_airborne: fnum(cmc, "TurningFactorScaleAirborne", f32::from_bits(0x3dcc_cccd)),
            head_on_min_speed_to_rear: fnum(cmc, "HeadOnCollisionMinSpeedToRear", 0.0),
            soft_bubble_rel: vec3(cmc, "SoftBubbleEllipseRelativeLocation", v(-80.0, 0.0, 0.0)),
            soft_bubble_length: fnum(cmc, "SoftBubbleEllipseLength", 220.0),
            soft_bubble_radius: fnum(cmc, "SoftBubbleEllipseRadius", 70.0),
            soft_bubble_max_height: fnum(cmc, "SoftBubbleMaxHeight", 150.0),
            front_rear_half_height: fnum(cmc, "FrontAndRearCapsuleHalfHeight", 60.0),
            front_rear_radius: fnum(cmc, "FrontAndRearCapsuleRadius", 55.0),
            front_rel: vec3(cmc, "FrontCapsuleRelativeLocation", v(110.0, 0.0, 40.0)),
            rear_rel: vec3(cmc, "RearCapsuleRelativeLocation", v(-50.0, 0.0, 20.0)),
            avoidance_turning_acceleration: fnum(cmc, "AvoidanceTurningAcceleration", 10.0),
            speed_multiplier_on_bump: fnum(cmc, "SpeedMultiplierOnBump", 0.75),
            speed_multiplier_on_melee: fnum(cmc, "SpeedMultiplierOnReceivedMeleeDamage", f32::from_bits(0x3f59_999a)),
            bump_damage_curve: load_curve(cdo, "BumpDamageBySpeedModifierCurve"),
            knockback_force: fnum(cdo, "KnockbackForce", 600.0),
            knockback_force_velocity_factor: fnum(cdo, "KnockbackForceVelocityFactor", 0.5),
            knockback_damage: fnum(cdo, "KnockbackDamage", 5.0),
            rearing_duration: fnum(cdo, "RearingDuration", 2.6),
            uncontrolled_gear: cdo.get("UncontrolledGear").and_then(Value::as_i64).unwrap_or(0) as i32,
            attach_offset: veh.get("AttachSocketOffset").map_or(FVector::ZERO, |o| vec3(o, "Translation", FVector::ZERO)),
            min_xy_distance_to_enter: fnum(&veh, "MinXYDistanceToEnter", 150.0),
            min_z_distance_to_enter: veh.get("MinZDistanceToEnter").map_or((-50.0, 100.0), |o| (fnum(o, "X", -50.0), fnum(o, "Y", 100.0))),
            minimum_interactable_velocity: fnum(&veh, "MinimumInteractableVelocity", -1.0),
            mesh_relative: vec3(&mesh, "RelativeLocation", FVector::ZERO),
        })
    }
}

/// The engine part of BP_Horse's CharMoveComp over the native ctors, as a `CharacterRecords` the shared `Cfg` loader
/// reads (`base` supplies the AAdvancedCharacter / UMordhauMovementComponent fields a horse does not use).
pub fn horse_character_records(base: &CharacterRecords, bp_horse: &str) -> Result<CharacterRecords, String> {
    let ex: Value = serde_json::from_str(bp_horse).map_err(|e| format!("BP_Horse json: {e}"))?;
    let cmc = named(&ex, "CharMoveComp").ok_or("BP_Horse has no CharMoveComp")?;
    let cdo = named(&ex, "Default__BP_Horse_C").ok_or("BP_Horse has no CDO")?;
    let g = |k: &str, d: f64| cmc.get(k).and_then(Value::as_f64).unwrap_or(d);
    let mut r = base.clone();
    let m = &mut r.movement;
    // UCharacterMovementComponent ctor rva=0x2f6cc10 (0x142f6cff5..0x142f6d211)
    m.gravity_scale = g("GravityScale", 1.0);
    m.ground_friction = g("GroundFriction", 8.0);
    m.jump_z_velocity = g("JumpZVelocity", 420.0);
    m.walkable_floor_z = g("WalkableFloorZ", f32::from_bits(0x3f35_c28f) as f64);
    m.max_step_height = g("MaxStepHeight", 45.0);
    m.perch_radius_threshold = g("PerchRadiusThreshold", 0.0);
    m.perch_additional_height = g("PerchAdditionalHeight", 40.0);
    m.max_walk_speed = g("MaxWalkSpeed", 600.0);
    m.max_walk_speed_crouched = g("MaxWalkSpeedCrouched", 300.0);
    m.air_control = g("AirControl", f32::from_bits(0x3d4c_cccd) as f64);
    m.air_control_boost_multiplier = g("AirControlBoostMultiplier", 2.0);
    m.air_control_boost_velocity_threshold = g("AirControlBoostVelocityThreshold", 25.0);
    m.max_acceleration = g("MaxAcceleration", 2048.0);
    m.braking_friction_factor = g("BrakingFrictionFactor", 2.0);
    m.braking_deceleration_falling = g("BrakingDecelerationFalling", 0.0);
    m.falling_lateral_friction = g("FallingLateralFriction", 0.0);
    m.crouched_half_height = g("CrouchedHalfHeight", 40.0);
    m.b_can_walk_off_ledges_when_crouching = false;
    // UAdvancedCharacterMovement ctor rva=0x144da20 (0x14144db05..0x14144db19)
    r.move_extra.min_velocity_for_fall_damage = g("MinVelocityForFallDamage", 1000.0);
    r.move_extra.fall_damage_offset = g("FallDamageOffset", -1000.0);
    r.move_extra.fall_damage_factor = g("FallDamageFactor", f32::from_bits(0x3e8c_cccd) as f64);
    // AAdvancedCharacter: JumpCooldown (ctor 0x14144b713: 0.5, BP_Horse 0.75)
    r.character.jump_cooldown = cdo.get("JumpCooldown").and_then(Value::as_f64).unwrap_or(0.5);
    Ok(r)
}

impl HorseState {
    pub fn new(cfg: HorseCfg) -> Self {
        HorseState {
            cfg,
            gear: 0,
            move_desired_gear: 0,
            desired_gear: 0,
            turning_velocity: 0.0,
            pending_turn_value: 0.0,
            control_yaw: 0.0,
            target_control_yaw: 0.0,
            last_rearing_time: f32::NEG_INFINITY,
            last_rearing_real_time: f32::NEG_INFINITY,
            is_rearing: false,
            replicated_rearing: 0,
            rear_requests: 0,
            controlled: true,
            jump_held: false,
            driver: None,
            driver_entering_or_leaving: false,
            last_front_hit: false,
            last_rear_hit: false,
            last_push: FVector::ZERO,
        }
    }
}

// ---- the horse pawn -----------------------------------------------------------------------------------------------

impl ExeMovement {
    /// A horse at `location` (capsule centre): BP_Horse's CollisionCylinder (radius 55, half height 120), the engine
    /// values from `horse_character_records`, BrakingDecelerationWalking from CharMoveComp (UCMC ctor 2048), then
    /// UHorseMovementComponent::InitializeComponent rva=0x14c12c0 (gear 0's speed / acceleration).
    pub fn new_horse(rec: &CharacterRecords, bp_horse: &str, cfg: HorseCfg, location: FVector) -> Result<ExeMovement, String> {
        let ex: Value = serde_json::from_str(bp_horse).map_err(|e| format!("BP_Horse json: {e}"))?;
        let cyl = named(&ex, "CollisionCylinder").cloned().unwrap_or(Value::Null);
        let cmc = named(&ex, "CharMoveComp").cloned().unwrap_or(Value::Null);
        let mut m = ExeMovement::new(rec, location);
        m.e.capsule_half_height = fnum(&cyl, "CapsuleHalfHeight", 88.0);
        m.e.capsule_radius = fnum(&cyl, "CapsuleRadius", 34.0);
        m.capsule_half_height = m.e.capsule_half_height;
        m.e.braking_deceleration_walking = fnum(&cmc, "BrakingDecelerationWalking", 2048.0);
        m.c.can_crouch = false; // NavAgentProps.bCanCrouch (+0xf0 bit 0): UNavMovementComponent ctor clears the bits (0x142f98425), UCharacterMovementComponent ctor sets only bits 1-3 (0x142f6d37c..0x142f6d382)
        let mut h = HorseState::new(cfg);
        if let Some(g0) = h.cfg.gears.first().copied() {
            h.gear = 0;
            m.c.max_walk_speed = g0.max_speed;
            m.c.max_acceleration = g0.max_acceleration;
        }
        m.horse = Some(Box::new(h));
        Ok(m)
    }

    fn h(&self) -> &HorseState {
        self.horse.as_deref().expect("not a horse")
    }
    fn hm(&mut self) -> &mut HorseState {
        self.horse.as_deref_mut().expect("not a horse")
    }

    /// set Gear and apply its speed / acceleration (the inlined block of LODTick, ValidateGear, UpdateFromCompressedFlags)
    fn horse_set_gear(&mut self, g: u8) {
        let gi = self.h().cfg.gears.get(g as usize).copied();
        self.hm().gear = g;
        if let Some(gi) = gi {
            self.c.max_walk_speed = gi.max_speed;
            self.c.max_acceleration = gi.max_acceleration;
        }
    }

    /// UHorseMovementComponent::ValidateGear rva=0x14e16b0: while Gear - 1 is valid and |V| <=
    /// (GearInfo[Gear - 1].MaxSpeed - ErrorThreshold) - 20, drop a gear; returns whether it was already valid
    pub fn horse_validate_gear(&mut self, error_threshold: f32) -> bool {
        let speed = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y + self.velocity.z * self.velocity.z).sqrt();
        let start = self.h().gear as i32;
        let mut g = start;
        let n = self.h().cfg.gears.len() as i32;
        loop {
            let lower = g - 1;
            if !(g > -1 && g < n && lower > -1 && lower < n) {
                break;
            }
            if !(speed <= (self.h().cfg.gears[lower as usize].max_speed - error_threshold) - 20.0) {
                break;
            }
            g -= 1;
        }
        if g != start {
            self.horse_set_gear(g as u8);
            return false;
        }
        true
    }

    /// UHorseMovementComponent::UpdateFromCompressedFlags rva=0x14dd580 (server side of a move): Gear = flag bits 4..5,
    /// then the ValidateGear(5) loop
    pub fn horse_update_from_compressed_flags(&mut self, flags: u8) {
        let g = ((flags >> 5) & 1) * 2 | ((flags >> 4) & 1);
        self.horse_set_gear(g);
        self.horse_validate_gear(5.0);
    }

    /// FSavedMove_Horse::SetMoveFor rva=0x14d5970 after the engine's SetMoveFor (the owning client's saved move; added
    /// by rust-net r5 for mh-net's movement replication): ValidateGear(0); DesiredGear (+0xce8) != Gear and below
    /// GearInfo.Num -> Gear = DesiredGear with its MaxSpeed / MaxAcceleration (+0x18c / +0x1a0), ValidateGear(-5)
    /// (.rdata 0x144331190); the move saves Gear (+0x2a8). Returns the saved gear.
    pub fn horse_set_move_for_gear(&mut self) -> u8 {
        self.horse_validate_gear(0.0);
        let dg = self.h().move_desired_gear;
        if dg != self.h().gear && (dg as usize) < self.h().cfg.gears.len() {
            self.horse_set_gear(dg);
            self.horse_validate_gear(-5.0);
        }
        self.h().gear
    }

    /// FSavedMove_Horse::PrepMoveFor rva=0x14d0930 after the engine's PrepMoveFor (a replayed move; added by rust-net
    /// r5): Gear (+0xcd0) = the saved gear, only the byte (0x1414d0982: no speed / acceleration write), then
    /// ValidateGear(0)
    pub fn horse_prep_move_for_gear(&mut self, gear: u8) {
        self.hm().gear = gear;
        self.horse_validate_gear(0.0);
    }

    /// FSavedMove_Horse::GetCompressedFlags rva=0x14bc510: the gear in bits 4 and 5 over the character flags
    pub fn horse_compressed_gear_bits(&self) -> u8 {
        let g = self.h().gear;
        ((g & 1) << 4) | (((g >> 1) & 1) << 5)
    }

    /// AHorse::GetIsInRearingMode rva=0x14fb730: TimeSeconds < LastRearingTime + RearingDuration
    pub fn horse_is_rearing(&self) -> bool {
        let h = self.h();
        let t = h.last_rearing_time + h.cfg.rearing_duration;
        self.world_time <= t && t != self.world_time
    }

    /// AHorse::IsMoveInputIgnored rva=0x14fe200: rearing, ragdoll falling / getting up (vcall +0x960); the controller's
    /// own flag is the host's
    pub fn horse_is_move_input_ignored(&self) -> bool {
        self.horse_is_rearing() || self.is_ragdoll_falling_or_getting_up()
    }

    /// AHorse::RequestRearing rva=0x1514b60: not rearing (TimeSeconds >= LastRearingTime + RearingDuration), alive, the
    /// driver not entering / leaving (combat side) -> ServerRequestRearing_Implementation rva=0x15155b0
    /// (ReplicatedRearing++, then OnRep_ReplicatedRearing rva=0x150eea0: bIsRearing, LastRearingTime = TimeSeconds,
    /// LastRearingRealTime, controlled -> DesiredGear = -1)
    pub fn horse_request_rearing(&mut self) {
        let t = self.h().last_rearing_time + self.h().cfg.rearing_duration;
        if !(t < self.world_time || t == self.world_time) || self.dead {
            return;
        }
        if self.h().driver.is_some() && self.h().driver_entering_or_leaving {
            return;
        }
        let now = self.world_time;
        let h = self.hm();
        h.replicated_rearing = h.replicated_rearing.wrapping_add(1);
        h.rear_requests += 1;
        // OnRep_ReplicatedRearing: the same window test again, then
        h.is_rearing = true;
        h.last_rearing_time = now;
        h.last_rearing_real_time = now;
        if h.controlled {
            h.desired_gear = -1;
        }
    }

    /// AHorse::MoveForward rva=0x14ffae0 (every frame with the axis value)
    pub fn horse_move_forward(&mut self, val: f32) {
        let s2 = self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y + self.velocity.z * self.velocity.z;
        let ngears = self.h().cfg.gears.len() as i32;
        let mws = self.c.max_walk_speed;
        let real = self.world_time; // RealTimeSeconds
        let rear_lock = real <= self.h().last_rearing_real_time + 0.3;
        if val != 0.0 && !rear_lock {
            let dg = self.h().desired_gear;
            let mut new = None;
            if 0.0 <= val {
                if dg < ngears {
                    if dg == -1 {
                        new = Some(0);
                    } else if dg == 0 {
                        if !(25.0 < s2.abs()) {
                            new = Some(1);
                        }
                    } else {
                        let a = mws - 5.0;
                        if !(s2 <= a * a) {
                            new = Some(dg + 1);
                        }
                    }
                }
            } else if dg >= 0 {
                if dg == 0 {
                    if s2 < 25.0 {
                        new = Some(-1);
                    }
                } else {
                    let a = mws + 5.0;
                    if !(a * a <= s2) {
                        new = Some(dg - 1);
                    }
                }
            }
            if let Some(n) = new {
                self.hm().desired_gear = n;
            }
        }
        let dg = self.h().desired_gear;
        let ad = dg.abs() - 1;
        self.hm().move_desired_gear = if -1 < ad { ad as u8 } else { 0 };
        let sign = if dg < 1 { dg >> 31 } else { 1 };
        let (_, fwd, _) = actor_axes(self.yaw);
        let s = sign as f32;
        // AddInputVector(Forward * sign, false): APawn::Internal_AddMovementInput through IsMoveInputIgnored
        self.add_movement_input_plain(v(fwd.x * s, fwd.y * s, fwd.z * s));
    }

    /// AHorse::MoveRight rva=0x14ffe70 -> AAdvancedCharacter::Turn rva=0x14a8a90 (bIsAbsolute false; no
    /// InputYawScale: bTurnUsesControllerInputYawScale is false (AHorse ctor 0x1414e37xx); bTurnRateIgnoresCap (BP)
    /// skips the turn cap) -> TargetControlYaw and PendingTurnValue += Value
    pub fn horse_move_right(&mut self, val: f32) {
        if self.is_ragdoll_falling_or_getting_up() {
            return;
        }
        let v = if val < -1.0 { -1.0 } else if val > 1.0 { 1.0 } else { val };
        if v != 0.0 {
            let h = self.hm();
            h.target_control_yaw = clamp_axis(v + h.target_control_yaw);
            h.pending_turn_value = v + h.pending_turn_value;
        }
    }

    /// AHorse::CalculateBumpDamage rva=0x14f2350: BumpDamageBySpeedModifierCurve(|V|) (0 without the curve)
    pub fn horse_bump_damage(&self, vel: FVector) -> f32 {
        match &self.h().cfg.bump_damage_curve {
            Some(k) => rich_curve_eval(k, (vel.x * vel.x + vel.y * vel.y + vel.z * vel.z).sqrt()),
            None => 0.0,
        }
    }

    /// AHorse::OnBumpCapsuleOverlapped rva=0x1507d80 after a knockback the combat side did (AHorse::DoKnockback):
    /// Velocity *= SpeedMultiplierOnBump
    pub fn horse_on_bump_knockback(&mut self) {
        let k = self.h().cfg.speed_multiplier_on_bump;
        self.velocity = v(k * self.velocity.x, k * self.velocity.y, k * self.velocity.z);
    }

    /// AHorse::OnTookDamage_Implementation rva=0x1510210: alive and melee -> Velocity *= SpeedMultiplierOnReceivedMeleeDamage
    pub fn horse_on_melee_damage(&mut self) {
        if self.dead {
            return;
        }
        let k = self.h().cfg.speed_multiplier_on_melee;
        self.velocity = v(k * self.velocity.x, k * self.velocity.y, k * self.velocity.z);
    }

    /// The Jump action pressed (BP_Horse InputActionDelegateBindings, both IE_Pressed, in this order):
    /// InpActEvt_Jump_40 -> ExecuteUbergraph 4136: MapRangeClamped(|V|, 0, MaxWalkSpeed, 0, 100) > 50 -> Jump();
    /// InpActEvt_Jump_39 -> 4825: LastRearingTime + RearingDuration + 1 < GameTime and Gear < 2 -> RequestRearing()
    pub fn horse_jump_pressed(&mut self) {
        let speed = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y + self.velocity.z * self.velocity.z).sqrt();
        let mws = self.c.max_walk_speed;
        // UKismetMathLibrary::MapRangeClamped rva=0x3188730 (read off the disassembly): |InB - InA| > 1e-8 ? (V - InA) /
        // (InB - InA) : (V >= InB ? 1 : 0); clamped to [0, 1] (a NaN gives 0); (OutB - OutA) * pct + OutA
        let d = mws - 0.0;
        let pct = if d.abs() > 1e-8 { (speed - 0.0) / d } else if speed >= mws { 1.0 } else { 0.0 };
        let pct = if pct >= 0.0 { if pct < 1.0 { pct } else { 1.0 } } else { 0.0 };
        if (100.0 - 0.0) * pct + 0.0 > 50.0 {
            self.jump();
        }
        let h = self.h();
        if h.last_rearing_time + h.cfg.rearing_duration + 1.0 < self.world_time && h.gear < 2 {
            self.horse_request_rearing();
        }
    }

    /// AHorse::LODTick rva=0x14fe470 (after AMordhauVehicle::LODTick rva=0x162c590: the look-up interpolation when
    /// nobody drives; not modelled): not controlled -> DesiredGear = UncontrolledGear. The turd, the camera shake and
    /// the regen gates (GearInfo flags) are the host's / combat side's (`horse_regen_flags`).
    pub fn horse_character_lod_tick(&mut self) {
        let h = self.hm();
        if !h.controlled {
            h.desired_gear = h.cfg.uncontrolled_gear;
        }
    }

    /// The current gear's regen flags (AHorse::LODTick 0x1414fe5..: bCanHorseRegen false -> StopHealthRegen on the
    /// horse; driver and !bCanRiderRegenHealth -> StopHealthRegen; !bCanRiderRegenStamina -> StopStaminaRegen)
    pub fn horse_regen_flags(&self) -> Option<GearInfo> {
        let h = self.h();
        h.cfg.gears.get(h.gear as usize).copied()
    }

    /// UHorseMovementComponent::LODTick rva=0x14c28e0 (module docs; the locally player controlled block
    /// 0x1414c29e8..0x1414c4854, the secondary rotation test (none: no SecondaryComponents), UAdvancedCharacterMovement::
    /// LODTick, the gear block 0x1414c49xx..)
    pub fn horse_lod_tick(&mut self, world: &dyn World, dt: f32) {
        if self.h().controlled {
            self.horse_steer(world, dt);
        }
        // UAdvancedCharacterMovement::LODTick rva=0x1489530: PendingTurnValue != 0 and locally controlled ->
        // AddTurnDegrees (vcall +0xa20); PendingTurnValue = 0 (bUsePendingRotationToOrientMovement is false)
        let ptv = self.h().pending_turn_value;
        if ptv != 0.0 {
            if self.h().controlled {
                self.horse_add_turn_degrees(ptv);
            }
            self.hm().pending_turn_value = 0.0;
        }
        // gear (authority, locally controlled or uncontrolled): ValidateGear(0); DesiredGear != Gear ->
        // Gear = DesiredGear, ValidateGear(-5)
        if self.authority {
            self.horse_validate_gear(0.0);
            let dg = self.h().move_desired_gear;
            if dg != self.h().gear && (dg as usize) < self.h().cfg.gears.len() {
                self.horse_set_gear(dg);
                self.horse_validate_gear(-5.0);
            }
        } else {
            // simulated proxy (role 1): Gear from the speed (|V| < MaxSpeed + 5)
            let speed = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y + self.velocity.z * self.velocity.z).sqrt();
            self.horse_set_gear(0);
            for i in 0..self.h().cfg.gears.len().min(4) {
                if speed < self.h().cfg.gears[i].max_speed + 5.0 {
                    self.horse_set_gear(i as u8);
                    break;
                }
            }
        }
    }

    /// AMordhauVehicle::AddTurnDegrees rva=0x1615760 -> AAdvancedCharacter::AddTurnDegrees rva=0x14574c0 for a player
    /// controller (TurnLimit -1, AAdvancedCharacter ctor 0x14144b442): the base is the control rotation's yaw
    pub fn horse_add_turn_degrees(&mut self, delta: f32) {
        let h = self.hm();
        let mut y = fmod(h.control_yaw, 360.0);
        if y < 0.0 {
            y = y + 360.0;
        }
        let mut n = fmod(y + delta, 360.0);
        if n < 0.0 {
            n = n + 360.0;
        }
        h.control_yaw = n;
    }

    /// UCharacterMovementComponent::PhysicsRotation rva=0x2f823e0 for bUseControllerDesiredRotation: the desired
    /// rotation is the control rotation (pitch / roll 0, ShouldRemainVertical while walking / falling: yaw in
    /// (-180, 180]); each axis FixedTurn'ed by GetDeltaRotation (RotationRate (0, -1, 0): 360, `exe_cmc::delta_rotation_yaw`) when the normalized
    /// difference exceeds 0.001
    pub fn horse_physics_rotation(&mut self, dt: f32) {
        if !matches!(self.mode, Mode::Walking | Mode::NavWalking | Mode::Falling) {
            return;
        }
        let mut dy = fmod(self.h().control_yaw, 360.0);
        if dy < 0.0 {
            dy = dy + 360.0;
        }
        if dy > 180.0 {
            dy = dy - 360.0;
        }
        // FRotator::GetNormalized(Current - Desired) (VectorMod 360, then into (-180, 180])
        let d = self.yaw - dy;
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
        if !(r.abs() > 0.001) {
            return;
        }
        // GetDeltaRotation: RotationRate.Yaw >= 0 -> min(Rate * dt, 360)
        let rate = crate::exe_cmc::delta_rotation_yaw(dt);
        let mut ny = self.yaw;
        if (self.yaw - dy).abs() > 0.001 {
            ny = fixed_turn(self.yaw, dy, rate);
        }
        self.set_yaw(ny); // MoveUpdatedComponent(zero, Quaternion(rot), no sweep)
    }

    /// The locally player controlled block of UHorseMovementComponent::LODTick
    fn horse_steer(&mut self, world: &dyn World, dt: f32) {
        // turning (0x1414c29e8..0x1414c2b49)
        let top = match self.h().cfg.gears.last() {
            Some(g) => g.max_speed,
            None => self.c.max_walk_speed,
        };
        let vs = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
        let ratio = if top.abs() > 1e-8 { vs / top } else { 0.0 };
        let sgn = |x: f32| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 };
        let ptv = self.h().pending_turn_value;
        let tv0 = self.h().turning_velocity;
        let curve = if ptv != 0.0 && sgn(ptv) == sgn(tv0) { &self.h().cfg.turning_accel_curve } else { &self.h().cfg.turning_brake_curve };
        let acc = curve.as_ref().map_or(1.0, |k| rich_curve_eval(k, ratio));
        if let Some(k) = self.h().cfg.turning_factor_curve.clone() {
            let f = rich_curve_eval(&k, ratio);
            self.hm().pending_turn_value = f * self.h().pending_turn_value;
        }
        if self.is_falling() {
            let s = self.h().cfg.turning_factor_scale_airborne;
            self.hm().pending_turn_value = s * self.h().pending_turn_value;
        }
        let mut tv = finterp_constant_to(tv0, self.h().pending_turn_value, dt, acc);
        let yaw_delta = (dt * tv) * 90.0;
        // the pending input, clamped to size 1 (0x1414c2b55..0x1414c2bf6)
        let input = self.control_input_vector;
        let sq = (input.y * input.y + input.x * input.x) + input.z * input.z;
        let cin = if 1.0 < sq { mul(input, inv_sqrt(sq)) } else { input };
        // the yaw after this turn, the front / rear capsule sweeps (0x1414c2c9f..0x1414c3035)
        let mut ny = fmod(self.yaw + yaw_delta, 360.0);
        if ny < 0.0 {
            ny = ny + 360.0;
        }
        let loc = self.location;
        let c = self.h().cfg.clone();
        let fr = rotator_rotate_vector(0.0, ny, 0.0, c.front_rel);
        let front = v(loc.x + fr.x, loc.y + fr.y, loc.z + fr.z);
        let rr = rotator_rotate_vector(0.0, ny, 0.0, c.rear_rel);
        let rear = v(loc.x + rr.x, loc.y + rr.y, loc.z + rr.z);
        // UWorld::SweepSingleByProfile with the owner capsule's profile (VehicleOverlapPawn; the World trait answers
        // for the Pawn profile: the profiles differ only in the Pawn channel, and pawns are not in the World)
        let first = |a: FVector, b: FVector| world.sweep_capsule(a, b, c.front_rear_radius, c.front_rear_half_height).into_iter().find(|h| h.blocking_hit);
        let mut hit_any = false;
        let mut hit_rear = false;
        let mut hit = first(loc, front);
        if hit.is_some() {
            hit_any = true;
        } else {
            hit = first(loc, rear);
            if hit.is_some() {
                hit_any = true;
                hit_rear = true;
            }
        }
        let hit_pawn = false; // a hit actor that IsA APawn: never in the World (pawns come through `others`)
        // the soft bubbles of the other pawns (0x1414c3140..0x1414c3c94)
        let spots = [loc, front, rear];
        let my_vel = self.velocity;
        let bump = self.horse_bump_damage(my_vel);
        let mut total = FVector::ZERO;
        for o in self.others.iter() {
            // TActorIterator<AAdvancedCharacter>: not ourselves, not the driver, not dead (+0x504)
            if !o.alive || o.is_driver {
                continue;
            }
            let (l, r, hh, centre) = if o.is_horse {
                if !o.mordhau_movement {
                    continue;
                }
                let rel = crate::exe_lod::rotate_twv_pub(o.quat, o.soft_bubble_rel);
                (o.ellipse_length, o.ellipse_radius, o.ellipse_max_height_diff, v(rel.x + o.location.x, rel.y + o.location.y, rel.z + o.location.z))
            } else if o.mordhau_character {
                if !(o.has_movement && bump == 0.0) {
                    continue;
                }
                (o.ellipse_length, o.ellipse_radius, o.ellipse_max_height_diff, o.location)
            } else {
                continue;
            };
            let qi = quat_inverse(o.quat);
            let mut push = FVector::ZERO;
            for s in spots.iter() {
                let dz = s.z - centre.z;
                if !(dz.abs() <= hh) {
                    continue;
                }
                let local = crate::exe_lod::rotate_twv_pub(qi, v(s.x - centre.x, s.y - centre.y, dz));
                let (lx, ly) = (local.x, local.y);
                if let Some(p) = bubble_push(lx, ly, l, r) {
                    push = p;
                    break;
                }
            }
            if push.x.abs() > 1e-4 || push.y.abs() > 1e-4 || push.z.abs() > 1e-4 {
                let w = crate::exe_lod::rotate_twv_pub(o.quat, push);
                let s2 = w.y * w.y + w.x * w.x;
                let n = if s2 == 1.0 {
                    v(w.x, w.y, 0.0)
                } else if 1e-8 <= s2 {
                    let k = inv_sqrt(s2);
                    v(w.x * k, w.y * k, 0.0)
                } else {
                    FVector::ZERO
                };
                total = v(total.x + n.x, total.y + n.y, total.z + n.z);
            }
        }
        self.hm().last_front_hit = hit_any && !hit_rear;
        self.hm().last_rear_hit = hit_rear;
        self.hm().last_push = total;
        // the hit direction: (Location - ImpactPoint).GetSafeNormal2D (0x1414c3d52..)
        let mut hd = FVector::ZERO;
        if let Some(h) = hit {
            let hz = h.location.z - h.impact_point.z;
            let hy = h.location.y - h.impact_point.y;
            let hx = h.location.x - h.impact_point.x;
            let s2 = hy * hy + hx * hx;
            hd = if s2 == 1.0 {
                v(hx, hy, if hz == 0.0 { hz } else { 0.0 })
            } else if 1e-8 <= s2 {
                let k = inv_sqrt(s2);
                v(hx * k, hy * k, 0.0)
            } else {
                FVector::ZERO
            };
        }
        // the push direction (0x1414c3e55..)
        let pd = if total.x.abs() > 1e-4 || total.y.abs() > 1e-4 || total.z.abs() > 1e-4 {
            let s2 = (total.y * total.y + total.x * total.x) + total.z * total.z;
            if s2 == 1.0 {
                total
            } else if 1e-8 <= s2 {
                let k = inv_sqrt(s2);
                v(total.x * k, total.y * k, total.z * k)
            } else {
                FVector::ZERO
            }
        } else {
            total
        };
        let mut av = v(hd.x * 0.75 + pd.x * 0.25, hd.y * 0.75 + pd.y * 0.25, hd.z * 0.75 + pd.z * 0.25);
        if av.x.abs() > 1e-4 || av.y.abs() > 1e-4 || av.z.abs() > 1e-4 {
            let s2 = (av.x * av.x + av.y * av.y) + av.z * av.z;
            if s2 != 1.0 {
                av = if 1e-8 <= s2 {
                    let k = inv_sqrt(s2);
                    v(av.x * k, av.y * k, av.z * k)
                } else {
                    FVector::ZERO
                };
            }
            let normalized_input = || {
                let s2 = (cin.y * cin.y + cin.x * cin.x) + cin.z * cin.z;
                if s2 == 1.0 {
                    cin
                } else if 1e-8 <= s2 {
                    let k = inv_sqrt(s2);
                    v(cin.x * k, cin.y * k, cin.z * k)
                } else {
                    FVector::ZERO
                }
            };
            let blend = |ni: FVector| {
                let b = v(ni.x * 0.25 + av.x * 0.75, ni.y * 0.25 + av.y * 0.75, ni.z * 0.25 + av.z * 0.75);
                let s2 = (b.x * b.x + b.y * b.y) + b.z * b.z;
                if s2 == 1.0 {
                    b
                } else if 1e-8 <= s2 {
                    let k = inv_sqrt(s2);
                    v(b.x * k, b.y * k, b.z * k)
                } else {
                    FVector::ZERO
                }
            };
            if self.h().gear > 1 || !(yaw_delta.abs() > 0.01) {
                // 0x1414c4300: the angle between the (rear-flipped) actor forward and the avoidance direction
                let s = if hit_rear { -1.0f32 } else { 1.0 };
                let f = rotator_vector(0.0, self.yaw);
                let (fx, fy, fz) = (s * f.x, s * f.y, s * f.z);
                let d = (av.x * fx + av.y * fy) + av.z * fz;
                let d = if d >= -1.0 { minss(d, 1.0) } else { -1.0 };
                let ang = d.acos() * RAD_TO_DEG;
                let cz = av.y * fx - av.x * fy;
                let cy = av.x * fz - av.z * fx;
                let cx = av.z * fy - av.y * fz;
                let up = (cx * 0.0 + cy * 0.0) + cz * 1.0;
                let side = if up >= 0.0 { f32::from_bits(0xbd63_8e39) } else { f32::from_bits(0x3d63_8e39) };
                if ang >= 150.0 || !(ang > 30.0) {
                    self.add_input_force(v(-input.x, -input.y, -input.z));
                    let speed = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y + self.velocity.z * self.velocity.z).sqrt();
                    if speed > c.head_on_min_speed_to_rear && hit_any && !hit_pawn {
                        self.add_movement_input_plain(av);
                        self.horse_request_rearing();
                    } else {
                        let b = blend(normalized_input());
                        self.add_movement_input_plain(b);
                    }
                } else {
                    tv = finterp_constant_to(self.h().turning_velocity, (90.0 - ang) * side, dt, c.avoidance_turning_acceleration);
                }
            } else {
                self.add_input_force(v(-input.x, -input.y, -input.z));
                let b = blend(normalized_input());
                self.add_movement_input_plain(b);
            }
        }
        self.hm().turning_velocity = tv;
        self.hm().pending_turn_value = (dt * tv) * 90.0;
    }

    /// UPawnMovementComponent::AddInputVector(V, bForce = false) -> APawn::Internal_AddMovementInput (IsMoveInputIgnored)
    fn add_movement_input_plain(&mut self, d: FVector) {
        if !self.is_move_input_ignored() {
            self.control_input_vector = add(self.control_input_vector, d);
        }
    }

    /// One frame of a horse, the order of `frame` (exe.rs frame docs: the controller's input, the movement component,
    /// the actor tick AHorse::LODTick). The input: the action bindings, then the axes (UPlayerInput::ProcessInputStack
    /// dispatches non-axis delegates first; UNCONFIRMED for this build)
    pub fn horse_frame(&mut self, world: &dyn World, dt: f32, inp: &HorseInput) {
        self.world_time += dt;
        if inp.jump != self.h().jump_held {
            self.hm().jump_held = inp.jump;
            if inp.jump {
                self.horse_jump_pressed();
            }
        }
        self.horse_move_forward(inp.fwd);
        self.horse_move_right(inp.right);
        // the movement component before the actor (bTickBeforeOwner, exe.rs frame docs)
        self.horse_lod_tick(world, dt);
        let input = self.consume_input_vector();
        self.controlled_character_move(world, input, dt);
        self.tick_ragdoll_get_up(dt);
        // UHorseMovementComponent::PostCharacterMovementTick rva=0xb93a60: empty
        self.horse_character_lod_tick();
    }

    // ---- mounting (UMordhauVehicleComponent) -----------------------------------------------------------------------

    /// UMordhauVehicleComponent::GetSeatLocation rva=0x14bf830: AttachSocketName is None (ctor 0x1414afa7c, BP keeps
    /// it) -> the owner's root (capsule) location
    pub fn horse_seat_location(&self) -> FVector {
        self.location
    }

    /// UMordhauVehicleComponent::CanInteract rva=0x14b4c50 + UHorseVehicleComponent::CanDrive_Implementation
    /// rva=0x14b4aa0: alive, MinXYDistanceToEnter^2 > dXY^2, MinZ.X < dZ < MinZ.Y, |V| < MinimumInteractableVelocity
    /// (unless -1), not rearing, the rider's flag (+0xb90, `rider_can_enter`) and no driver
    pub fn horse_can_interact(&self, rider: FVector, rider_can_enter: bool) -> bool {
        let h = self.h();
        let seat = self.horse_seat_location();
        let dz = rider.z - seat.z;
        let dxy = (rider.y - seat.y) * (rider.y - seat.y) + (rider.x - seat.x) * (rider.x - seat.x);
        let m = h.cfg.min_xy_distance_to_enter;
        if self.dead || m * m <= dxy || dz <= h.cfg.min_z_distance_to_enter.0 || h.cfg.min_z_distance_to_enter.1 <= dz {
            return false;
        }
        if h.cfg.minimum_interactable_velocity != -1.0 {
            let s = (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y + self.velocity.z * self.velocity.z).sqrt();
            if h.cfg.minimum_interactable_velocity <= s {
                return false;
            }
        }
        !self.horse_is_rearing() && rider_can_enter && h.driver.is_none()
    }

    /// UMordhauVehicleComponent::StartDriving rva=0x14d79a0, the movement part: the rider is placed at the mesh's
    /// socket transform (AttachSocketName None = the mesh component's transform: the capsule location + the mesh's
    /// relative location rotated by the actor yaw) composed with AttachSocketOffset, rotation the mesh's (identity:
    /// BP_Horse's CharacterMesh0 serializes no RelativeRotation and no ctor of AHorse rva=0x14e36f0, AMordhauVehicle
    /// rva=0x1610d30, AAdvancedCharacter rva=0x144b0e0 or ACharacter rva=0x2f27200 writes it (+0x128): the zero default);
    /// the controller then possesses the horse
    /// and the rider is attached (`rider_seat` each frame)
    pub fn horse_start_driving(&mut self, rider_id: usize) -> Option<(FVector, f32)> {
        if self.h().driver.is_some() {
            return None;
        }
        self.hm().driver = Some(rider_id);
        self.hm().controlled = true;
        self.hm().control_yaw = self.yaw;
        Some(self.horse_rider_seat())
    }

    /// the rider's attached transform: mesh transform * AttachSocketOffset (location, yaw)
    pub fn horse_rider_seat(&self) -> (FVector, f32) {
        let c = &self.h().cfg;
        let q = self.actor_quat;
        let ml = crate::uequat::quat_rotate(q, c.mesh_relative);
        let mesh = v(self.location.x + ml.x, self.location.y + ml.y, self.location.z + ml.z);
        let off = crate::uequat::quat_rotate(q, c.attach_offset);
        (v(mesh.x + off.x, mesh.y + off.y, mesh.z + off.z), self.yaw)
    }

    /// UMordhauVehicleComponent::StopDriving rva=0x14d8270 + GetExitTransform_Implementation rva=0x14bc740, the movement
    /// part: the exit is the mesh's DetachSocketName ("Exit") socket location when the host knows it (`exit`), else the
    /// rider's own location; rotation the rider's yaw only (pitch / roll zeroed); then UWorld::FindTeleportSpot
    /// rva=0x31a3b60 for the rider's capsule (`World::find_teleport_spot`; mh-level ports the engine table): the adjusted
    /// spot, or the exit transform as is when nothing fits (0x1414d8422). Returns the rider's new (location, yaw).
    pub fn horse_stop_driving(&mut self, world: &dyn World, rider_loc: FVector, rider_yaw: f32, exit: Option<FVector>, rider_radius: f32, rider_half_height: f32) -> (FVector, f32) {
        self.hm().driver = None;
        self.hm().controlled = false;
        let place = exit.unwrap_or(rider_loc);
        let loc = world.find_teleport_spot(place, rider_radius, rider_half_height).unwrap_or(place);
        (loc, rider_yaw)
    }
}

/// The ellipse-and-circles bubble of LODTick (0x1414c35..0x1414c36d2), in the other pawn's frame: behind its origin a
/// circle of radius R, along its forward axis to L a band of half width R, a circle of radius R around (L, 0); returns
/// the push out of it, None outside
fn bubble_push(lx: f32, ly: f32, l: f32, r: f32) -> Option<FVector> {
    let front = || -> Option<FVector> {
        let d = (-ly * -ly + (l - lx) * (l - lx)).sqrt();
        if r <= d {
            return None;
        }
        let ax = lx - l;
        let s2 = ax * ax + ly * ly;
        let (nx, ny) = if s2 == 1.0 {
            (ax, ly)
        } else if 1e-8 <= s2 {
            let k = inv_sqrt(s2);
            (k * ax, ly * k)
        } else {
            (0.0, 0.0)
        };
        let k = r - d;
        Some(v(k * nx, k * ny, 0.0 * k))
    };
    if lx <= 0.0 {
        let s2 = ly * ly + lx * lx;
        let d = s2.sqrt();
        if r <= d {
            return front();
        }
        let k = r - d;
        let (nx, ny) = if s2 == 1.0 {
            (lx, ly)
        } else if 1e-8 <= s2 {
            let q = inv_sqrt(s2);
            (q * lx, ly * q)
        } else {
            (0.0, 0.0)
        };
        return Some(v(nx * k, ny * k, 0.0 * k));
    }
    if lx <= l && ly.abs() <= r {
        let rs = if ly <= 0.0 { -r } else { r };
        return Some(v(0.0, rs - ly, 0.0));
    }
    front()
}

/// A horse in `others` (another horse's soft bubble), standing at `location` facing `yaw`
pub fn other_horse(cfg: &HorseCfg, location: FVector, yaw: f32) -> OtherPawn {
    let (q, f, _) = actor_axes(yaw);
    OtherPawn {
        alive: true,
        mordhau_character: false,
        mordhau_movement: true,
        is_horse: true,
        has_movement: true,
        location,
        quat: q,
        forward: f,
        mesh_location: location,
        lower_back: location,
        ellipse_length: cfg.soft_bubble_length,
        ellipse_radius: cfg.soft_bubble_radius,
        ellipse_max_height_diff: cfg.soft_bubble_max_height,
        soft_bubble_rel: cfg.soft_bubble_rel,
        ..Default::default()
    }
}

/// `ExeInput` has no horse meaning: a horse frame takes `HorseInput`
pub fn horse_input(e: &ExeInput) -> HorseInput {
    HorseInput { fwd: e.fwd, right: e.right, jump: e.jump }
}

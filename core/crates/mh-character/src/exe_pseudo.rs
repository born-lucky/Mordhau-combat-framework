//! UPseudoVehicleMovementComponent vehicles with secondary collision components (rust-character r8): the catapult
//! (BP_Catapult : AMordhauWheeledVehicle : AMordhauCompoundVehicle : AMordhauVehicle, CharMoveComp
//! PseudoVehicleMovementComponent; AMordhauCompoundVehicle ctor rva=0x1526380 overrides the movement class). The horse
//! is the same component class without secondaries (exe_horse.rs); everything here switches on
//! `ExeMovement::pseudo` and, like the exe, on whether the SecondaryComponents / SecondaryStepCapableComponents arrays are
//! empty.
//!
//! - BP_Catapult UserConstructionScript (state/character_kismet/BP_Catapult.txt @95..@202): SecondaryComponents =
//!   [Box, Box1, BackCapsule], SecondaryStepCapableComponents = [BackCapsule]; all three attach to CollisionCylinder
//!   (SCS_Node_4..6, ParentComponentOrVariableName CollisionCylinder) with their RelativeLocation and no relative
//!   rotation (none in the package: identity).
//! - MoveUpdatedComponentImpl rva=0x1500230: each secondary is moved by the delta with the new rotation composed with its
//!   relative rotation (its own MoveComponent: sweep, PullBackHit, hit selection, `exe_cmc::select_move_hit`) and put
//!   back (SetRelativeLocationAndRotation); the earliest blocking hit scales the delta (unless falling with a
//!   controller, when the secondaries are ignored), the root moves, and the secondary hit replaces the root's when the
//!   root did not block or blocked later.
//! - FloorSweepTest rva=0x14f92d0: the root's floor sweep, then the same shape swept from each step-capable secondary's
//!   location (Start / End offset by the component's location minus the root's), the earliest blocking hit wins.
//! - OverlapTest rva=0x15109f0: the root's, then OverlapTestOnlySecondary rva=0x1510bf0 (each enabled secondary at
//!   Location + Rotation * RelativeLocation, Rotation * RelativeRotation, its shape inflated by the requested shape's
//!   half height minus the root's).
//! - PhysicsRotation rva=0x1512310: with a controller, the control rotation (pitch / roll 0 while walking / falling) is
//!   approached by FixedTurn at GetDeltaRotation (RotationRate (0, -1, 0): instant, `exe_cmc::delta_rotation_yaw`); the rotation is applied only when
//!   OverlapTestOnlySecondary finds the secondaries free there.
//! - LODTick rva=0x14ff190: the turning (curves by |Velocity| / GetMaxAttainableSpeed = MaxWalkSpeed (rva=0x14fc520),
//!   FInterpConstantTo of TurningVelocity, PendingTurnValue = TurningVelocity * dt * 90, airborne scale), the
//!   SweepRotationTestOnlySecondary rva=0x1516880 of the turn plus 2.5 degrees (inflation 0.1): blocked -> no turn and
//!   the input bent toward the hit's ImpactNormal (0.75 / 0.25); then UAdvancedCharacterMovement::LODTick's tail
//!   (AddTurnDegrees when locally controlled, PendingTurnValue = 0).
//! - UAdvancedCharacterMovement::TickComponent rva=0x14a3e80 (AdvancedCharacterMovement.cpp 0x1414a4..: input pointing
//!   backwards with bReverseBackwardsTurning flips PendingTurnValue; the catapult clears the flag).
//! - The input: AAdvancedCharacter::MoveForward rva=0x148a6c0 (bAddForwardAxisToMovementInput, set by the
//!   AMordhauCompoundVehicle ctor), BP_Catapult "Move Right" -> Turn(FClamp(Axis, -1, 1), false) @1436..@1483 ->
//!   AAdvancedCharacter::Turn rva=0x14a8a90 (TurnRateCap -1: no cap; bTurnUsesControllerInputYawScale cleared);
//!   "Turn Right" -> UMordhauVehicleComponent::SecondaryTurn rva=0x14d28c0 (the driver's camera yaw within
//!   SecondaryTurnLimit, `pseudo_secondary_turn`).
//!
//! Queries: every shape queries on the "Vehicle" object channel (the components' BodyInstance ObjectType ECC_Vehicle)
//! with its profile's responses (DefaultEngine.ini +EditProfiles=(Name="Vehicle", Pawn Block, Visibility Ignore,
//! MordhauRagdoll Ignore, Camera Overlap) for the CollisionCylinder and the boxes (whose BP ResponseArrays say the
//! same), +Profiles=(Name="VehicleOverlapPawn", Pawn Overlap, Visibility Ignore, Camera Overlap, MordhauRagdoll Ignore)
//! for BackCapsule): the root's queries go through `world::ChannelWorld` (UCharacterMovementComponent queries on
//! UpdatedPrimitive->GetCollisionObjectType() with its response params), the secondaries' through the channel queries.

use crate::exe::{ExeMovement, Mode};
use crate::exe_cmc::{select_move_hit, MIN_MOVEMENT_DIST_SQ};
use crate::records::CharacterRecords;
use crate::ue::FVector;
use crate::uemath::*;
use crate::uequat::*;
use crate::world::{ChannelWorld, HitResult, World};
use serde_json::Value;

/// A secondary collision component's shape
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SecondaryShape {
    /// UBoxComponent: BoxExtent (half extents)
    Box(FVector),
    /// UCapsuleComponent: radius, half height
    Capsule(f32, f32),
}

/// One SecondaryComponents entry
#[derive(Clone, Debug, PartialEq)]
pub struct Secondary {
    pub name: String,
    /// RelativeLocation to the root (CollisionCylinder)
    pub rel: FVector,
    pub shape: SecondaryShape,
    /// also in SecondaryStepCapableComponents
    pub step_capable: bool,
    /// its responses to object channels that are not Block (its collision profile / BodyInstance ResponseArray)
    pub responses: Vec<(String, u8)>,
}

/// The vehicle's data: BP_Catapult (CharMoveComp, the SCS components) over the native ctors
#[derive(Clone, Debug, PartialEq)]
pub struct PseudoCfg {
    pub secondaries: Vec<Secondary>,
    pub turning_brake_curve: Option<Vec<RichKey>>,  // +0xc70 TurningBrakeAccelerationByVelocity
    pub turning_factor_curve: Option<Vec<RichKey>>, // +0xc78 TurningFactorByVelocity
    pub turning_accel_curve: Option<Vec<RichKey>>,  // +0xc80 TurningAccelerationByVelocity
    pub turning_factor_scale_airborne: f32,         // +0xc88 (ctor rva=0x14e6e90: 0.1)
    pub reverse_backwards_turning: bool,            // UAdvancedCharacterMovement bReverseBackwardsTurning (ctor true)
    /// UMordhauVehicleComponent SecondaryTurnLimit (BP_VehicleCatapult 90)
    pub secondary_turn_limit: f32,
    /// the root CollisionCylinder's non-Block responses (Vehicle profile)
    pub box_responses: Vec<(String, u8)>,
}

/// The per-vehicle state
#[derive(Clone, Debug, PartialEq)]
pub struct PseudoState {
    pub cfg: PseudoCfg,
    pub turning_velocity: f32,   // +0xc68
    pub pending_turn_value: f32, // UAdvancedCharacterMovement PendingTurnValue
    pub control_yaw: f32,        // the controller's ControlRotation yaw
    pub target_control_yaw: f32, // AAdvancedCharacter TargetControlYaw
    /// a controller possesses it (a driver sits in it); also IsLocallyControlled here
    pub controlled: bool,
    /// UMordhauVehicleComponent::SecondaryTurnValue (the driver's camera yaw offset)
    pub secondary_turn_value: f32,
    /// the last SweepRotationTestOnlySecondary hit (tests / hosts)
    pub last_rotation_block: Option<HitResult>,
    // AMordhauWheeledVehicle::LODTick rva=0x162c600 (the animation's rotation velocity)
    pub previous_rotation: (f32, f32),       // PreviousRotation (yaw, look up)
    pub rotation_velocity: (f32, f32),       // RotationVelocity
    pub rotation_velocity_interp: (f32, f32), // ctor rva=0x1611520: (30, 20)
    pub rotation_velocity_max: (f32, f32),   // (15, 15)
}

/// The catapult's frame input (the driver's axes)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PseudoInput {
    /// "Move Forward"
    pub fwd: f32,
    /// "Move Right" (turns the vehicle)
    pub right: f32,
    /// "Turn Right" (the driver's camera yaw, SecondaryTurn)
    pub turn: f32,
}

fn named<'a>(exports: &'a Value, name: &str) -> Option<&'a Value> {
    exports.as_array()?.iter().find(|e| e["Name"] == name).map(|e| &e["Properties"])
}
fn fnum(v: &Value, k: &str, d: f32) -> f32 {
    v.get(k).and_then(Value::as_f64).map_or(d, |x| x as f32)
}
fn vec3(v: &Value, k: &str) -> FVector {
    match v.get(k) {
        Some(x) => crate::uemath::v(fnum(x, "X", 0.0), fnum(x, "Y", 0.0), fnum(x, "Z", 0.0)),
        None => FVector::ZERO,
    }
}
fn ref_path(v: &Value, k: &str) -> Option<String> {
    let p = v.get(k)?.get("ObjectPath")?.as_str()?;
    Some(p.rsplit_once('.').map_or(p, |(a, _)| a).to_string())
}

impl PseudoCfg {
    /// BP_Catapult.json + BP_VehicleCatapult.json (the vehicle component, SecondaryTurnLimit); `curve(package path)`
    /// returns a CurveFloat's package JSON. The secondary lists are BP_Catapult's construction script's (module docs).
    pub fn from_bp_json(bp: &str, bp_vehicle: Option<&str>, curve: &dyn Fn(&str) -> Option<String>) -> Result<PseudoCfg, String> {
        let ex: Value = serde_json::from_str(bp).map_err(|e| format!("BP json: {e}"))?;
        let cmc = named(&ex, "CharMoveComp").ok_or("no CharMoveComp")?;
        let mut secondaries = Vec::new();
        for (name, step) in [("Box", false), ("Box1", false), ("BackCapsule", true)] {
            let comp = named(&ex, &format!("{name}_GEN_VARIABLE")).ok_or(format!("no {name}"))?;
            let shape = if comp.get("CapsuleRadius").is_some() || comp.get("CapsuleHalfHeight").is_some() {
                // UCapsuleComponent ctor rva=0x2f4e060 defaults: radius 22, half height 44 (0x142f4e0b1 / 0x142f4e0bb)
                SecondaryShape::Capsule(fnum(comp, "CapsuleRadius", 22.0), fnum(comp, "CapsuleHalfHeight", 44.0))
            } else {
                // UBoxComponent ctor rva=0x2f4df90: BoxExtent (32, 32, 32) (.rdata 0x144513738 (32), stored at 0x142f4dfd9)
                let b = comp.get("BoxExtent").map_or(crate::uemath::v(32.0, 32.0, 32.0), |x| crate::uemath::v(fnum(x, "X", 32.0), fnum(x, "Y", 32.0), fnum(x, "Z", 32.0)));
                SecondaryShape::Box(b)
            };
            let responses = comp
                .pointer("/BodyInstance/CollisionResponses/ResponseArray")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|r| {
                            let ch = r.get("Channel")?.as_str()?.to_string();
                            let resp = match r.get("Response").and_then(Value::as_str).unwrap_or("ECR_Block") {
                                "ECR_Ignore" => 0u8,
                                "ECR_Overlap" => 1,
                                _ => 2,
                            };
                            Some((ch, resp))
                        })
                        .collect()
                })
                .unwrap_or_else(vehicle_responses);
            secondaries.push(Secondary { name: name.to_string(), rel: vec3(comp, "RelativeLocation"), shape, step_capable: step, responses });
        }
        let c = |k: &str| ref_path(cmc, k).and_then(|p| curve(&p)).and_then(|j| crate::exe_horse::curve_keys_from_json(&j));
        let veh = match bp_vehicle {
            Some(j) => {
                let v: Value = serde_json::from_str(j).map_err(|e| format!("vehicle json: {e}"))?;
                v.as_array().and_then(|a| a.iter().find(|e| e["Name"].as_str().is_some_and(|n| n.starts_with("Default__")))).map(|e| e["Properties"].clone()).unwrap_or(Value::Null)
            }
            None => Value::Null,
        };
        Ok(PseudoCfg {
            secondaries,
            turning_brake_curve: c("TurningBrakeAccelerationByVelocity"),
            turning_factor_curve: c("TurningFactorByVelocity"),
            turning_accel_curve: c("TurningAccelerationByVelocity"),
            turning_factor_scale_airborne: fnum(cmc, "TurningFactorScaleAirborne", 0.1),
            reverse_backwards_turning: cmc.get("bReverseBackwardsTurning").and_then(Value::as_bool).unwrap_or(true),
            // UMordhauVehicleComponent ctor rva=0x14af9b0 does not write SecondaryTurnLimit (+0x144): 0 (BP sets 90)
            secondary_turn_limit: fnum(&veh, "SecondaryTurnLimit", 0.0),
            box_responses: vehicle_responses(),
        })
    }
}

/// The "Vehicle" profile's non-Block responses (DefaultEngine.ini +EditProfiles=(Name="Vehicle", ...))
fn vehicle_responses() -> Vec<(String, u8)> {
    vec![("Visibility".into(), 0), ("Camera".into(), 1), ("MordhauRagdoll".into(), 0)]
}

/// The engine part of a pseudo vehicle's CharMoveComp over the native ctors, as `CharacterRecords` (as
/// `exe_horse::horse_character_records`): UCharacterMovementComponent ctor rva=0x2f6cc10 defaults for what the BP
/// does not set
pub fn pseudo_character_records(base: &CharacterRecords, bp: &str) -> Result<CharacterRecords, String> {
    let ex: Value = serde_json::from_str(bp).map_err(|e| format!("BP json: {e}"))?;
    let cmc = named(&ex, "CharMoveComp").ok_or("no CharMoveComp")?;
    let g = |k: &str, d: f64| cmc.get(k).and_then(Value::as_f64).unwrap_or(d);
    let mut r = base.clone();
    let m = &mut r.movement;
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
    r.move_extra.min_velocity_for_fall_damage = g("MinVelocityForFallDamage", 1000.0);
    r.move_extra.fall_damage_offset = g("FallDamageOffset", -1000.0);
    r.move_extra.fall_damage_factor = g("FallDamageFactor", f32::from_bits(0x3e8c_cccd) as f64);
    Ok(r)
}

impl PseudoState {
    pub fn new(cfg: PseudoCfg) -> Self {
        PseudoState {
            cfg,
            turning_velocity: 0.0,
            pending_turn_value: 0.0,
            control_yaw: 0.0,
            target_control_yaw: 0.0,
            controlled: false,
            secondary_turn_value: 0.0,
            last_rotation_block: None,
            previous_rotation: (0.0, 0.0),
            rotation_velocity: (0.0, 0.0),
            rotation_velocity_interp: (30.0, 20.0),
            rotation_velocity_max: (15.0, 15.0),
        }
    }
}

/// a secondary's world location for a root at `loc` with yaw quaternion `q` (ComponentToWorld = Relative * Root)
fn world_of(loc: FVector, q: Quat, rel: FVector) -> FVector {
    let r = quat_rotate(q, rel);
    v(r.x + loc.x, r.y + loc.y, r.z + loc.z)
}

impl ExeMovement {
    /// A catapult at `location` (capsule centre) facing `yaw`: BP_Catapult's CollisionCylinder, the engine values from
    /// `pseudo_character_records`, bMaintainHorizontalGroundVelocity from CharMoveComp (BP_Catapult false)
    pub fn new_pseudo(rec: &CharacterRecords, bp: &str, cfg: PseudoCfg, location: FVector, yaw: f32) -> Result<ExeMovement, String> {
        let ex: Value = serde_json::from_str(bp).map_err(|e| format!("BP json: {e}"))?;
        let cyl = named(&ex, "CollisionCylinder").cloned().unwrap_or(Value::Null);
        let cmc = named(&ex, "CharMoveComp").cloned().unwrap_or(Value::Null);
        let mut m = ExeMovement::new(rec, location);
        m.e.capsule_half_height = fnum(&cyl, "CapsuleHalfHeight", 88.0);
        m.e.capsule_radius = fnum(&cyl, "CapsuleRadius", 34.0);
        m.capsule_half_height = m.e.capsule_half_height;
        m.e.braking_deceleration_walking = fnum(&cmc, "BrakingDecelerationWalking", 2048.0);
        if let Some(b) = cmc.get("bMaintainHorizontalGroundVelocity").and_then(Value::as_bool) {
            m.e.maintain_horizontal_ground_velocity = b;
        }
        m.c.can_crouch = false;
        m.set_yaw(yaw);
        let mut p = PseudoState::new(cfg);
        p.control_yaw = yaw;
        p.target_control_yaw = clamp_axis(yaw);
        p.previous_rotation = (yaw, 0.0);
        m.pseudo = Some(Box::new(p));
        Ok(m)
    }

    fn p(&self) -> &PseudoState {
        self.pseudo.as_deref().expect("not a pseudo vehicle")
    }
    fn pm(&mut self) -> &mut PseudoState {
        self.pseudo.as_deref_mut().expect("not a pseudo vehicle")
    }

    /// the secondaries' world locations now (tests / hosts: their collision for other queries)
    pub fn pseudo_secondary_locations(&self) -> Vec<FVector> {
        let q = self.actor_quat;
        self.p().cfg.secondaries.iter().map(|s| world_of(self.location, q, s.rel)).collect()
    }

    /// a secondary's shape swept from `start` to `end` with yaw quaternion `q`, inflated: the raw hits (earliest
    /// first)
    fn secondary_sweep(&self, world: &dyn World, s: &Secondary, start: FVector, end: FVector, q: Quat, inflation: f32) -> Vec<HitResult> {
        match s.shape {
            SecondaryShape::Box(e) => world.sweep_box_channel(start, end, v(e.x + inflation, e.y + inflation, e.z + inflation), q, "Vehicle", &s.responses),
            SecondaryShape::Capsule(r, hh) => world.sweep_capsule_channel(start, end, r + inflation, hh + inflation, "Vehicle", &s.responses),
        }
    }

    /// UPseudoVehicleMovementComponent::MoveUpdatedComponentImpl rva=0x1500230 (module docs)
    pub(crate) fn pseudo_move_updated_component(&mut self, world: &dyn World, delta: FVector, sweep: bool, never_ignore_overlaps: bool, out: &mut HitResult) -> bool {
        let mut d = delta;
        let mut best: Option<HitResult> = None;
        let mut t_best = 1.0f32;
        if sweep {
            let q = self.actor_quat;
            let secs = self.p().cfg.secondaries.clone();
            for s in &secs {
                // the component's own MoveComponent (its early-out for a short move, then sweep + PullBackHit + selection)
                if size_sq(delta) <= MIN_MOVEMENT_DIST_SQ {
                    continue;
                }
                let start = world_of(self.location, q, s.rel);
                let end = add(start, delta);
                let hits = self.secondary_sweep(world, s, start, end, q, 0.0);
                if let Some((h, _)) = select_move_hit(hits, start, delta, never_ignore_overlaps) {
                    if h.blocking_hit && h.time < t_best {
                        t_best = h.time;
                        best = Some(h);
                    }
                }
            }
            if best.is_some() {
                if !self.is_falling() || !self.p().controlled {
                    d = v(d.x * t_best, d.y * t_best, d.z * t_best);
                } else {
                    best = None;
                }
            }
        }
        let r = self.move_component_root(world, d, sweep, never_ignore_overlaps, out);
        if let Some(h) = best {
            if !out.blocking_hit || t_best < out.time {
                *out = h;
            }
        }
        r
    }

    /// UPseudoVehicleMovementComponent::FloorSweepTest rva=0x14f92d0 (module docs)
    pub(crate) fn pseudo_floor_sweep(&self, world: &dyn World, capsule: FVector, trace: f32, radius: f32, half_height: f32) -> HitResult {
        let mut best = self.floor_sweep_root(world, capsule, trace, radius, half_height);
        let q = self.actor_quat;
        for s in self.p().cfg.secondaries.iter().filter(|s| s.step_capable) {
            let off = sub(world_of(self.location, q, s.rel), self.location);
            let start = add(capsule, off);
            let end = v(start.x, start.y, start.z + -trace);
            // the floor sweep's shape and channel are the root's (FloorSweepTest's TraceChannel / CollisionShape)
            let h = world.sweep_capsule(start, end, radius, half_height).into_iter().find(|h| h.blocking_hit);
            if let Some(mut h) = h {
                if !(best.blocking_hit && best.time <= h.time) {
                    h.trace_start = start;
                    h.trace_end = end;
                    best = h;
                }
            }
        }
        best
    }

    /// UPseudoVehicleMovementComponent::OverlapTestOnlySecondary rva=0x1510bf0 at `loc` with yaw `yaw`, the shapes
    /// inflated by `inflation`
    pub fn pseudo_overlap_secondaries(&self, world: &dyn World, loc: FVector, yaw: f32, inflation: f32) -> bool {
        let q = rotator_quaternion(0.0, yaw, 0.0);
        for s in &self.p().cfg.secondaries {
            let c = world_of(loc, q, s.rel);
            let hit = match s.shape {
                SecondaryShape::Box(e) => world.overlap_box(c, v(e.x + inflation, e.y + inflation, e.z + inflation), q, "Vehicle", &s.responses),
                SecondaryShape::Capsule(r, hh) => world.overlap_capsule_channel(c, r + inflation, hh + inflation, "Vehicle", &s.responses),
            };
            if hit {
                return true;
            }
        }
        false
    }

    /// UPseudoVehicleMovementComponent::OverlapTest rva=0x15109f0: the root capsule (radius `r`, half height `hh`),
    /// then the secondaries inflated by hh - the root's half height
    pub(crate) fn pseudo_overlap_test(&self, world: &dyn World, loc: FVector, yaw: f32, r: f32, hh: f32) -> bool {
        if world.overlap_capsule(loc, r, hh) {
            return true;
        }
        self.pseudo_overlap_secondaries(world, loc, yaw, hh - self.capsule_half_height)
    }

    /// UPseudoVehicleMovementComponent::SweepRotationTestOnlySecondary rva=0x1516880: each secondary from its location
    /// now to where the new yaw puts it (rotation the new one, shape inflated), SweepSingleByChannel; the earliest
    /// blocking hit
    pub fn pseudo_sweep_rotation(&self, world: &dyn World, new_yaw: f32, inflation: f32) -> Option<HitResult> {
        let q0 = self.actor_quat;
        let q1 = rotator_quaternion(0.0, new_yaw, 0.0);
        let mut best: Option<HitResult> = None;
        for s in &self.p().cfg.secondaries {
            let start = world_of(self.location, q0, s.rel);
            let end = world_of(self.location, q1, s.rel);
            let h = self.secondary_sweep(world, s, start, end, q1, inflation).into_iter().find(|h| h.blocking_hit);
            if let Some(h) = h {
                if !(best.as_ref().is_some_and(|b| b.time <= h.time)) {
                    best = Some(h);
                }
            }
        }
        best
    }

    /// AAdvancedCharacter::Turn rva=0x14a8a90 for this vehicle (TurnRateCap -1, input scale 1): TargetControlYaw and
    /// the movement's PendingTurnValue
    pub fn pseudo_turn(&mut self, value: f32) {
        if value == 0.0 {
            return;
        }
        let p = self.pm();
        let mut t = fmod(value + p.target_control_yaw, 360.0);
        if t < 0.0 {
            t = t + 360.0;
        }
        p.target_control_yaw = t;
        p.pending_turn_value = value + p.pending_turn_value;
    }

    /// UMordhauVehicleComponent::SecondaryTurn rva=0x14d28c0 (no mouse smoothing, no cap: TurnRateCap -1): the
    /// driver's camera yaw, ClampAxis(ClampAxis(SecondaryTurnValue) + Value), clamped to +/- SecondaryTurnLimit by
    /// FMath::ClampAngle when the limit is >= 0
    pub fn pseudo_secondary_turn(&mut self, value: f32) {
        if value == 0.0 {
            return;
        }
        let p = self.pm();
        let mut cur = fmod(p.secondary_turn_value, 360.0);
        if cur < 0.0 {
            cur = cur + 360.0;
        }
        let mut n = fmod(cur + value, 360.0);
        if n < 0.0 {
            n = n + 360.0;
        }
        let lim = p.cfg.secondary_turn_limit;
        if 0.0 <= lim {
            n = crate::siege::clamp_angle(n, -lim, lim);
        }
        p.secondary_turn_value = n;
    }

    /// AMordhauVehicle::AddTurnDegrees rva=0x1615760 -> AAdvancedCharacter::AddTurnDegrees rva=0x14574c0 with TurnLimit
    /// -1 (BP_Catapult does not set it): ClampAxis(ClampAxis(control yaw) + Delta) is the new control yaw
    fn pseudo_add_turn_degrees(&mut self, delta: f32) {
        let p = self.pm();
        let mut y = fmod(p.control_yaw, 360.0);
        if y < 0.0 {
            y = y + 360.0;
        }
        let mut n = fmod(y + delta, 360.0);
        if n < 0.0 {
            n = n + 360.0;
        }
        p.control_yaw = n;
    }

    /// UAdvancedCharacterMovement::TickComponent rva=0x14a3e80's turn flip: PendingTurnValue != 0, locally controlled,
    /// the input in the actor's frame (FRotator::UnrotateVector) has X < 0 and bReverseBackwardsTurning ->
    /// PendingTurnValue = -PendingTurnValue
    pub fn pseudo_reverse_backwards_turning(&mut self) {
        let (ptv, controlled, rev) = (self.p().pending_turn_value, self.p().controlled, self.p().cfg.reverse_backwards_turning);
        if ptv == 0.0 || !controlled {
            return;
        }
        let local = quat_rotate(quat_inverse(self.actor_quat), self.control_input_vector);
        if local.x < 0.0 && rev {
            self.pm().pending_turn_value = -ptv;
        }
    }

    /// UPseudoVehicleMovementComponent::LODTick rva=0x14ff190 (module docs)
    pub fn pseudo_lod_tick(&mut self, world: &dyn World, dt: f32) {
        // the turning block: locally controlled, or the authority with no controller (both here)
        let max = self.c.max_walk_speed; // GetMaxAttainableSpeed rva=0x14fc520
        let mut ratio = 0.0f32;
        if 1e-8 < max.abs() {
            let vv = self.velocity;
            ratio = ((vv.x * vv.x + vv.y * vv.y) + vv.z * vv.z).sqrt() * (1.0 / max);
        }
        let sgn = |x: f32| if x > 0.0 { 1.0f32 } else if x < 0.0 { -1.0 } else { 0.0 };
        let p = self.p();
        let curve = if p.pending_turn_value == 0.0 || sgn(p.pending_turn_value) != sgn(p.turning_velocity) { &p.cfg.turning_brake_curve } else { &p.cfg.turning_accel_curve };
        let acc = curve.as_ref().map_or(1.0, |k| rich_curve_eval(k, ratio));
        if let Some(k) = p.cfg.turning_factor_curve.clone() {
            let f = rich_curve_eval(&k, ratio);
            let pm = self.pm();
            pm.pending_turn_value = f * pm.pending_turn_value;
        }
        let falling = self.is_falling();
        {
            let pm = self.pm();
            let tv = finterp_constant_to(pm.turning_velocity, pm.pending_turn_value, dt, acc);
            pm.turning_velocity = tv;
            pm.pending_turn_value = (tv * dt) * 90.0;
            if falling {
                pm.pending_turn_value = pm.cfg.turning_factor_scale_airborne * pm.pending_turn_value;
            }
        }
        let ptv = self.p().pending_turn_value;
        self.pm().last_rotation_block = None;
        if ptv != 0.0 {
            let s = if ptv > 0.0 { 2.5f32 } else if ptv < 0.0 { -2.5 } else { 0.0 };
            let mut ny = fmod((ptv + self.yaw) + s, 360.0);
            if ny < 0.0 {
                ny = ny + 360.0;
            }
            if let Some(h) = self.pseudo_sweep_rotation(world, ny, 0.1) {
                self.pm().pending_turn_value = 0.0;
                let n = h.impact_normal;
                if 0.0001 < n.x.abs() || 0.0001 < n.y.abs() || 0.0001 < n.z.abs() {
                    let i = self.control_input_vector;
                    let bx = n.x * 0.75 + i.x * 0.25;
                    let by = n.y * 0.75 + i.y * 0.25;
                    let bz = n.z * 0.75 + i.z * 0.25;
                    let l = ((bx * bx + by * by) + bz * bz).sqrt();
                    let i2 = self.control_input_vector;
                    let d = v(bx / l - i2.x, by / l - i2.y, bz / l - i2.z);
                    // AddInputVector(d, bForce = false)
                    self.add_movement_input(d, 1.0, false);
                }
                self.pm().last_rotation_block = Some(h);
            }
        }
        // UAdvancedCharacterMovement::LODTick rva=0x1489530's tail (bUsePendingRotationToOrientMovement false)
        let ptv = self.p().pending_turn_value;
        if ptv != 0.0 {
            if self.p().controlled {
                self.pseudo_add_turn_degrees(ptv);
            }
            self.pm().pending_turn_value = 0.0;
        }
    }

    /// UPseudoVehicleMovementComponent::PhysicsRotation rva=0x1512310 (module docs)
    pub(crate) fn pseudo_physics_rotation(&mut self, world: &dyn World, dt: f32) {
        if self.p().cfg.secondaries.is_empty() || !self.p().controlled || self.mode == Mode::None {
            return;
        }
        let vertical = matches!(self.mode, Mode::Walking | Mode::NavWalking | Mode::Falling);
        let mut dy = self.p().control_yaw;
        if vertical {
            dy = fmod(dy, 360.0);
            if dy < 0.0 {
                dy = dy + 360.0;
            }
            if 180.0 < dy {
                dy = dy - 360.0;
            }
        } else {
            dy = crate::siege::normalize_axis_vm(dy);
        }
        let r = crate::siege::normalize_axis_vm(self.yaw - dy);
        if !(0.001 < r.abs()) {
            return;
        }
        // GetDeltaRotation: RotationRate.Yaw 360 >= 0 -> min(360 * dt, 360)
        let rate = crate::exe_cmc::delta_rotation_yaw(dt);
        let mut ny = dy;
        if 0.001 < (self.yaw - dy).abs() {
            ny = fixed_turn(self.yaw, dy, rate);
        }
        // OverlapTestOnlySecondary at the location with the new rotation (the root's shape, no inflation), then
        // MoveUpdatedComponent(zero, rotation): a rotation-only move does not sweep
        if !self.pseudo_overlap_secondaries(world, self.location, ny, 0.0) {
            self.set_yaw(ny);
        }
    }

    /// AMordhauWheeledVehicle::LODTick rva=0x162c600 (alive): RotationVelocity by axis (yaw, look up) =
    /// clamp(FInterpConstantTo(RotationVelocity, (1 / dt) * unwound delta, dt, interp), +/- max)
    pub fn pseudo_wheeled_lod_tick(&mut self, dt: f32, look_up: f32) {
        let yaw = self.yaw;
        let p = self.pm();
        let wrap = |d: f32| if d <= 180.0 { if d < -180.0 { d + 360.0 } else { d } } else { d + -360.0 };
        let d0 = wrap(yaw - p.previous_rotation.0);
        let d1 = wrap(look_up - p.previous_rotation.1);
        if 0.0 < dt {
            let m = p.rotation_velocity_max.0;
            let x = finterp_constant_to(p.rotation_velocity.0, (1.0 / dt) * d0, dt, p.rotation_velocity_interp.0);
            p.rotation_velocity.0 = if -m <= x { if m <= x { m } else { x } } else { -m };
            let m = p.rotation_velocity_max.1;
            let y = finterp_constant_to(p.rotation_velocity.1, (1.0 / dt) * d1, dt, p.rotation_velocity_interp.1);
            p.rotation_velocity.1 = if -m <= y { if m <= y { m } else { y } } else { -m };
        }
        p.previous_rotation = (yaw, look_up);
    }

    /// One frame of a driven (or idle) pseudo vehicle in the exe's tick order (exe.rs frame docs): input (MoveForward
    /// along the actor forward, Turn, SecondaryTurn), the movement component (TickComponent's turn flip, LODTick, then
    /// ControlledCharacterMove with PhysicsRotation inside PerformMovement), then the actor tick
    /// (AMordhauWheeledVehicle::LODTick).
    pub fn pseudo_frame(&mut self, world: &dyn World, dt: f32, inp: &PseudoInput) {
        let resp = self.p().cfg.box_responses.clone();
        let cw = ChannelWorld { inner: world, channel: "Vehicle", responses: &resp };
        let world: &dyn World = &cw;
        self.world_time += dt;
        if self.p().controlled {
            if inp.fwd != 0.0 {
                let (_, fwd, _) = actor_axes(self.yaw);
                self.add_movement_input(mul(fwd, inp.fwd), 1.0, false);
            }
            let r = if -1.0 <= inp.right { if inp.right <= 1.0 { inp.right } else { 1.0 } } else { -1.0 };
            self.pseudo_turn(r);
            self.pseudo_secondary_turn(inp.turn);
        }
        self.pseudo_reverse_backwards_turning();
        self.pseudo_lod_tick(world, dt);
        let input = self.consume_input_vector();
        self.controlled_character_move(world, input, dt);
        self.pseudo_wheeled_lod_tick(dt, 0.0);
    }
}

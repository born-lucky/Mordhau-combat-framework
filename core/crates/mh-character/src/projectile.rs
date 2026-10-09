//! Projectiles (AMordhauProjectile with UAdvProjectileMovementComponent), the flight and collision half; the ranged
//! motions and the damage are the combat side's (rust-combat).
//!
//! Spawn: AMordhauEquipment::FireProjectile_Implementation rva=0x153d950 -> FireProjectile_Internal rva=0x153dae0
//! spawns the class at the origin / orientation URangedReleaseMotion::OnBegin_Implementation rva=0x16620d0 passes
//! (the server: the character's LastRequestedFireOrigin / Rotation; locally: CameraLocation1P and CameraRotation1P *
//! the draw motion's AimRotationOffset), then InitializeProjectile rva=0x15de060 and Fire_Implementation rva=0x15d3e40
//! activate the movement: UProjectileMovementComponent::InitializeComponent turns the default Velocity (1, 0, 0) into
//! InitialSpeed along the spawn rotation (bInitialVelocityInLocalSpace). The release speed does not depend on the
//! draw time: no code path scales InitialSpeed (the draw motion only offsets the aim).
//!
//! Flight: UProjectileMovementComponent::TickComponent rva=0x2fe4e90 (sub-stepping, ComputeMoveDelta rva=0x2fcfb30,
//! ComputeVelocity rva=0x2fd0310, LimitVelocity rva=0x2fda5a0, ComputeAcceleration rva=0x2fcf620 +
//! UAdvProjectileMovementComponent::ComputeAcceleration rva=0x145aaf0 (DragDeceleration), GetGravityZ rva=0x2fd4a20,
//! rotation following the velocity (FVector::ToOrientationQuat rva=0x18c10a0), the swept box move, HandleImpact
//! stopping the simulation (AMordhauProjectile ctor: bRotationFollowsVelocity set, bShouldBounce clear).
//!
//! Collision: the BoxComponent blocks the world (Custom profile: everything Block but Pawn Ignore and the listed
//! overlaps) - a blocking hit stops the flight and AMordhauProjectile::NotifyHit rva=0x15e2330 runs SweepProjectile
//! and ProcessProjectileHit(bIsBlocking). AMordhauProjectile::Tick rva=0x1600ff0 -> SweepProjectile rva=0x1600190
//! sweeps the box from LastProjectileLocation to the current location every frame (UWorld::ComponentSweepMulti, then
//! the hits in order through ProcessProjectileHit until the projectile terminates). Character hits are against the
//! physics-asset bodies (the host's `ProjectileTarget` capsules).
//!
//! After a hit (ProcessProjectileHit_Implementation rva=0x15ebfd0, combat side) the movement-relevant outcomes are
//! here: WillSticky rva=0x16052e0 / WillPassThrough rva=0x1605290, the surface stick of AttachProjectile_Implementation
//! rva=0x15c4f90 (rotation blend toward the surface, then attach), TerminateProjectile rva=0x1600be0.

use crate::ue::FVector;
use crate::uemath::*;
use crate::uequat::*;
use crate::world::{HitResult, World};
use serde_json::Value;

/// MIN_TICK_TIME (UE 1e-6, .rdata 0x144000104)
const MIN_TICK_TIME: f32 = f32::from_bits(0x358637bd);

/// One projectile class's data: the class chain's CDOs (BP_ArrowProjectile -> BP_MissileProjectile ->
/// BP_MordhauProjectile -> AMordhauProjectile) over the native ctors
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectileCfg {
    pub class: String,
    // UProjectileMovementComponent (ctor rva=0x2fc6c90; AMordhauProjectile ctor rva=0x15b5660)
    pub initial_speed: f32,              // +0xf0 (AMordhauProjectile ctor 3000)
    pub max_speed: f32,                  // +0xf4 (3000)
    pub gravity_scale: f32,              // +0x10c (1)
    pub rotation_follows_velocity: bool, // +0xf8 bit 0 (AMordhauProjectile ctor sets it)
    pub should_bounce: bool,             // +0xf8 bit 2 (cleared)
    pub sweep_collision: bool,           // +0xf8 bit 6 (ctor set)
    pub force_sub_stepping: bool,        // +0xf8 bit 4 (ctor clear)
    pub bounciness: f32,                 // +0x114 (0.6)
    pub friction: f32,                   // +0x118 (0.2)
    pub bounce_stop_threshold: f32,      // +0x11c (5)
    pub max_simulation_time_step: f32,   // +0x154 (0.05)
    pub max_simulation_iterations: i32,  // +0x158 (4)
    pub bounce_additional_iterations: i32, // +0x15c (1)
    pub drag_deceleration: f32,          // UAdvProjectileMovementComponent +0x1d0 (not set: 0)
    /// Velocity +0xc4 before InitializeComponent: its direction, in the spawn frame (bInitialVelocityInLocalSpace),
    /// times InitialSpeed is the launch velocity (UProjectileMovementComponent ctor (1, 0, 0); BP_CatapultProjectile
    /// (0.65, 0, 0.65))
    pub initial_velocity: FVector,
    /// the BoxComp's responses to object channels that are not Block (0 Ignore, 1 Overlap, 2 Block; any channel not
    /// listed answers Block): the most derived BodyInstance CollisionResponses.ResponseArray, else the "Projectile"
    /// profile the AMordhauProjectile ctor rva=0x15b5660 sets (DefaultEngine.ini +Profiles=(Name="Projectile"))
    pub box_responses: Vec<(String, u8)>,
    // the BoxComponent (AMordhauProjectile ctor (30, 30, 30))
    pub box_extent: FVector,
    // AMordhauProjectile
    pub rotation_spin: FVector,          // (X roll, Y pitch, Z yaw) deg/s of the SpinComponent
    pub will_sticky_on: Vec<u8>,         // EPhysicalSurface values (SurfaceType_Default = 0 = any)
    pub will_pass_through_on: Vec<u8>,
    pub sticky_surface_pitch_blend: f32, // ctor 1
    pub sticky_surface_yaw_blend: f32,   // ctor 0
    pub destroy_when_terminated: bool,   // ctor false
    pub path_blend_duration: f32,        // ctor 0.5
    // combat data carried for the combat side (by armor tier)
    pub damage: Vec<f32>,
    pub head_bonus: Vec<f32>,
    pub leg_bonus: Vec<f32>,
}

impl Default for ProjectileCfg {
    fn default() -> Self {
        ProjectileCfg {
            class: String::new(),
            initial_speed: 3000.0,
            max_speed: 3000.0,
            gravity_scale: 1.0,
            rotation_follows_velocity: true,
            should_bounce: false,
            sweep_collision: true,
            force_sub_stepping: false,
            bounciness: f32::from_bits(0x3f19_999a),
            friction: f32::from_bits(0x3e4c_cccd),
            bounce_stop_threshold: 5.0,
            max_simulation_time_step: f32::from_bits(0x3d4c_cccd),
            max_simulation_iterations: 4,
            bounce_additional_iterations: 1,
            drag_deceleration: 0.0,
            initial_velocity: v(1.0, 0.0, 0.0),
            box_responses: [("Pawn", 0u8), ("Visibility", 1), ("Camera", 1), ("PhysicsBody", 1), ("Vehicle", 0), ("Destructible", 1), ("MordhauRagdoll", 1), ("Projectile", 1)]
                .iter()
                .map(|(n, r)| (n.to_string(), *r))
                .collect(),
            box_extent: v(30.0, 30.0, 30.0),
            rotation_spin: FVector::ZERO,
            will_sticky_on: Vec::new(),
            will_pass_through_on: Vec::new(),
            sticky_surface_pitch_blend: 1.0,
            sticky_surface_yaw_blend: 0.0,
            destroy_when_terminated: false,
            path_blend_duration: 0.5,
            damage: Vec::new(),
            head_bonus: Vec::new(),
            leg_bonus: Vec::new(),
        }
    }
}

fn fnum(v: &Value, k: &str) -> Option<f32> {
    v.get(k).and_then(Value::as_f64).map(|x| x as f32)
}

/// "SurfaceType<N>" / "SurfaceType_Default" -> N
fn surface(v: &Value) -> Option<u8> {
    let s = v.as_str()?;
    if s == "SurfaceType_Default" {
        return Some(0);
    }
    s.strip_prefix("SurfaceType")?.parse().ok()
}

impl ProjectileCfg {
    /// The class chain's package JSONs, most derived first (each a CUE4Parse export list with Default__<Class>_C,
    /// ProjectileComp, BoxComp); each property takes the most derived value that sets it
    pub fn from_json_chain(chain: &[&str]) -> Result<ProjectileCfg, String> {
        let mut c = ProjectileCfg::default();
        for (i, j) in chain.iter().rev().enumerate() {
            let ex: Value = serde_json::from_str(j).map_err(|e| format!("projectile json {i}: {e}"))?;
            let arr = ex.as_array().ok_or("not an export list")?;
            for e in arr {
                let name = e["Name"].as_str().unwrap_or("");
                let p = &e["Properties"];
                if name.starts_with("Default__") {
                    if c.class.is_empty() || i == chain.len() - 1 {
                        c.class = name.trim_start_matches("Default__").to_string();
                    }
                    if let Some(r) = p.get("RotationSpin") {
                        c.rotation_spin = v(fnum(r, "X").unwrap_or(0.0), fnum(r, "Y").unwrap_or(0.0), fnum(r, "Z").unwrap_or(0.0));
                    }
                    if let Some(a) = p.get("WillStickyOn").and_then(Value::as_array) {
                        c.will_sticky_on = a.iter().filter_map(surface).collect();
                    }
                    if let Some(a) = p.get("WillPassThroughOn").and_then(Value::as_array) {
                        c.will_pass_through_on = a.iter().filter_map(surface).collect();
                    }
                    if let Some(x) = fnum(p, "StickySurfacePitchBlend") {
                        c.sticky_surface_pitch_blend = x;
                    }
                    if let Some(x) = fnum(p, "StickySurfaceYawBlend") {
                        c.sticky_surface_yaw_blend = x;
                    }
                    if let Some(x) = p.get("bDestroyWhenTerminated").and_then(Value::as_bool) {
                        c.destroy_when_terminated = x;
                    }
                    if let Some(x) = fnum(p, "PathBlendDuration") {
                        c.path_blend_duration = x;
                    }
                    let arr_f = |k: &str| p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect::<Vec<_>>());
                    if let Some(a) = arr_f("Damage") {
                        c.damage = a;
                    }
                    if let Some(a) = arr_f("HeadBonus") {
                        c.head_bonus = a;
                    }
                    if let Some(a) = arr_f("LegBonus") {
                        c.leg_bonus = a;
                    }
                } else if name == "ProjectileComp" {
                    if let Some(x) = fnum(p, "InitialSpeed") {
                        c.initial_speed = x;
                    }
                    if let Some(x) = fnum(p, "MaxSpeed") {
                        c.max_speed = x;
                    }
                    if let Some(x) = fnum(p, "ProjectileGravityScale") {
                        c.gravity_scale = x;
                    }
                    if let Some(x) = fnum(p, "DragDeceleration") {
                        c.drag_deceleration = x;
                    }
                    if let Some(x) = p.get("bShouldBounce").and_then(Value::as_bool) {
                        c.should_bounce = x;
                    }
                    if let Some(x) = p.get("bRotationFollowsVelocity").and_then(Value::as_bool) {
                        c.rotation_follows_velocity = x;
                    }
                    if let Some(x) = fnum(p, "Bounciness") {
                        c.bounciness = x;
                    }
                    if let Some(x) = fnum(p, "Friction") {
                        c.friction = x;
                    }
                    if let Some(b) = p.get("Velocity") {
                        c.initial_velocity = v(fnum(b, "X").unwrap_or(0.0), fnum(b, "Y").unwrap_or(0.0), fnum(b, "Z").unwrap_or(0.0));
                    }
                } else if name == "BoxComp" {
                    if let Some(b) = p.get("BoxExtent") {
                        c.box_extent = v(fnum(b, "X").unwrap_or(c.box_extent.x), fnum(b, "Y").unwrap_or(c.box_extent.y), fnum(b, "Z").unwrap_or(c.box_extent.z));
                    }
                    if let Some(a) = p.pointer("/BodyInstance/CollisionResponses/ResponseArray").and_then(Value::as_array) {
                        c.box_responses = a
                            .iter()
                            .filter_map(|r| {
                                let ch = r.get("Channel")?.as_str()?.to_string();
                                let resp = match r.get("Response").and_then(Value::as_str).unwrap_or("ECR_Block") {
                                    "ECR_Ignore" => 0u8,
                                    "ECR_Overlap" => 1,
                                    _ => 2,
                                };
                                Some((ch, resp))
                            })
                            .collect();
                    }
                }
            }
        }
        Ok(c)
    }
}

/// A capsule body of a character's physics asset (sphyl / sphere: radius, the two segment ends in world cm)
#[derive(Clone, Debug, PartialEq)]
pub struct TargetBody {
    pub bone: String,
    pub a: FVector,
    pub b: FVector,
    pub radius: f32,
    /// EPhysicalSurface of the body's physical material (flesh / armor), for WillSticky / WillPassThrough
    pub surface: u8,
}

/// A character (or other actor) the sweep tests: its physics-asset bodies this frame
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectileTarget {
    pub id: usize,
    pub bodies: Vec<TargetBody>,
}

/// One hit of the projectile, in sweep order
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectileHit {
    /// the target id and body bone; None = the world
    pub target: Option<(usize, String)>,
    pub time: f32,
    pub location: FVector,
    pub impact_point: FVector,
    pub impact_normal: FVector,
    /// EPhysicalSurface at the hit (bodies: their surface; world: the host's, 0 when unknown)
    pub surface: u8,
    /// a blocking hit (the world) as opposed to an overlap (bodies)
    pub blocking: bool,
}

/// A projectile in flight
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub cfg: ProjectileCfg,
    pub location: FVector,
    pub quat: Quat,
    pub velocity: FVector,
    /// bSimulationEnabled (+0xf8 bit 5): cleared by StopSimulating
    pub simulating: bool,
    pub terminated: bool, // AMordhauProjectile bHasTerminated
    pub last_location: FVector, // LastProjectileLocation
    pub fired_time: f32,
    pub previous_hit_time: f32, // +0xfc
    pub spin: FVector,          // SpinComponent relative rotation (roll, pitch, yaw) accumulated
    /// the world hit that stopped the flight, until SweepProjectile has reported it
    pending_block: Option<HitResult>,
    /// set when the projectile stuck into something (location / rotation fixed)
    pub stuck: Option<(Option<usize>, String)>,
}

impl Projectile {
    /// FireProjectile_Internal -> SpawnActor(origin, orientation) -> Fire_Implementation -> UProjectileMovementComponent::
    /// InitializeComponent: Velocity (1, 0, 0).GetSafeNormal() * InitialSpeed (InitialSpeed > 0), rotated by the
    /// component's rotation (SetVelocityInLocalSpace), rotation = Velocity.Rotation() when it follows the velocity
    pub fn fire(cfg: ProjectileCfg, origin: FVector, pitch: f32, yaw: f32, roll: f32, time: f32) -> Projectile {
        Projectile::fire_quat(cfg, origin, rotator_quaternion(pitch, yaw, roll), time)
    }

    /// `fire` with the spawn rotation as a quaternion (a socket transform: the siege engines' FireSocket / BoltSocket).
    /// The launch velocity is UProjectileMovementComponent::InitializeComponent rva=0x2fd8620, which
    /// AMordhauProjectile::Fire_Implementation rva=0x15d3e40 calls itself (vcall +0x328 at 0x1415d47e0; the ctor clears
    /// bWantsInitializeComponent, 0x1415b5bfe, and SetAutoActivate(false), 0x1415b5b9b): Velocity (`initial_velocity`)
    /// with SizeSquared > 0 -> SizeSquared == 1 ? Velocity : Velocity * InvSqrt(SizeSquared) (>= 1e-8, else zero),
    /// times InitialSpeed when InitialSpeed > 0, then rotated into the spawn frame (bInitialVelocityInLocalSpace)
    pub fn fire_quat(cfg: ProjectileCfg, origin: FVector, q: Quat, time: f32) -> Projectile {
        let iv = cfg.initial_velocity;
        let ss = (iv.x * iv.x + iv.y * iv.y) + iv.z * iv.z;
        let mut local = iv;
        if ss > 0.0 && cfg.initial_speed > 0.0 {
            let n = if ss == 1.0 {
                iv
            } else if ss >= 1e-8 {
                let k = inv_sqrt(ss);
                v(iv.x * k, iv.y * k, iv.z * k)
            } else {
                FVector::ZERO
            };
            local = v(n.x * cfg.initial_speed, n.y * cfg.initial_speed, n.z * cfg.initial_speed);
        }
        let vel = quat_rotate(q, local);
        let quat = if cfg.rotation_follows_velocity { to_orientation_quat(vel) } else { q };
        Projectile {
            cfg,
            location: origin,
            quat,
            velocity: vel,
            simulating: true,
            terminated: false,
            last_location: origin,
            fired_time: time,
            previous_hit_time: 1.0,
            spin: FVector::ZERO,
            pending_block: None,
            stuck: None,
        }
    }

    /// UProjectileMovementComponent::GetGravityZ rva=0x2fd4a20: ProjectileGravityScale != 0 -> world GravityZ * scale
    pub fn gravity_z(&self, world_gravity_z: f32) -> f32 {
        if 0.0 == self.cfg.gravity_scale {
            return 0.0;
        }
        world_gravity_z * self.cfg.gravity_scale
    }

    /// ComputeAcceleration rva=0x2fcf620 (+ (G + 0) on Z, no pending force, no homing) then
    /// UAdvProjectileMovementComponent::ComputeAcceleration rva=0x145aaf0: the drag along the velocity, clamped so it
    /// never reverses a component
    pub fn compute_acceleration(&self, vel: FVector, g: f32) -> FVector {
        let mut a = v(0.0 + 0.0, 0.0 + 0.0, (g + 0.0) + 0.0);
        let s = (vel.x * vel.x + vel.y * vel.y) + vel.z * vel.z;
        let (nx, ny, nz) = if s == 1.0 {
            (vel.x, vel.y, vel.z)
        } else if 1e-8 <= s {
            let k = inv_sqrt(s);
            (vel.x * k, vel.y * k, vel.z * k)
        } else {
            (0.0, 0.0, 0.0)
        };
        let d = self.cfg.drag_deceleration;
        let (dx, dy, dz) = (nx * d, ny * d, nz * d);
        if dx.abs() > 1e-4 || dy.abs() > 1e-4 || dz.abs() > 1e-4 {
            let clampc = |vc: f32, dc: f32| -> f32 {
                if 0.0 <= vc {
                    if vc <= 0.0 || dc < 0.0 {
                        0.0
                    } else if vc <= dc {
                        vc
                    } else {
                        dc
                    }
                } else if vc <= dc {
                    if 0.0 <= dc {
                        0.0
                    } else {
                        dc
                    }
                } else {
                    vc
                }
            };
            a = v(a.x - clampc(vel.x, dx), a.y - clampc(vel.y, dy), a.z - clampc(vel.z, dz));
        }
        a
    }

    /// LimitVelocity rva=0x2fda5a0: MaxSpeed > 0 -> GetClampedToMaxSize(MaxSpeed) (no plane constraint)
    pub fn limit_velocity(&self, nv: FVector) -> FVector {
        let ms = self.cfg.max_speed;
        if !(ms > 0.0) {
            return nv;
        }
        if !(ms >= 1e-4) {
            return FVector::ZERO;
        }
        let s = (nv.x * nv.x + nv.y * nv.y) + nv.z * nv.z;
        if s > ms * ms {
            let k = inv_sqrt(s) * ms;
            return v(nv.x * k, nv.y * k, nv.z * k);
        }
        nv
    }

    /// ComputeVelocity rva=0x2fd0310: LimitVelocity(A * dt + V)
    pub fn compute_velocity(&self, vel: FVector, dt: f32, g: f32) -> FVector {
        let a = self.compute_acceleration(vel, g);
        self.limit_velocity(v(a.x * dt + vel.x, a.y * dt + vel.y, a.z * dt + vel.z))
    }

    /// ComputeMoveDelta rva=0x2fcfb30: (V1 - V0) * (0.5 * dt) + V0 * dt
    pub fn compute_move_delta(&self, vel: FVector, dt: f32, g: f32) -> FVector {
        let nv = self.compute_velocity(vel, dt, g);
        let h = dt * 0.5;
        v((nv.x - vel.x) * h + vel.x * dt, (nv.y - vel.y) * h + vel.y * dt, (nv.z - vel.z) * h + vel.z * dt)
    }

    /// the swept box move (UPrimitiveComponent::MoveComponentImpl via MoveUpdatedComponent with
    /// MOVECOMP_NeverIgnoreBlockingOverlaps): the first blocking hit, pulled back like the capsule moves
    fn move_box(&mut self, world: &dyn World, delta: FVector, quat: Quat) -> HitResult {
        let start = self.location;
        let end = add(start, delta);
        let mut out = HitResult::new(1.0);
        out.trace_start = start;
        out.trace_end = end;
        let dsq = size_sq(delta);
        self.quat = quat;
        if dsq <= crate::exe_cmc::MIN_MOVEMENT_DIST_SQ {
            return out;
        }
        if !self.cfg.sweep_collision {
            self.location = end;
            return out;
        }
        let hits = world.sweep_box(start, end, self.cfg.box_extent, quat, &self.cfg.box_responses);
        if let Some(h) = hits.into_iter().find(|h| h.blocking_hit) {
            let inv = 1.0 / dsq.sqrt();
            let lo = inv * 0.1;
            let mut back = if lo > 0.1 { lo } else { minss(inv, 0.1) };
            back = back + 0.001;
            let t = h.time - back;
            let t = if t >= 0.0 { minss(t, 1.0) } else { 0.0 };
            out = h;
            out.time = t;
            self.location = v(start.x + delta.x * t, start.y + delta.y * t, start.z + delta.z * t);
            out.location = self.location;
        } else {
            self.location = end;
        }
        out
    }

    /// UProjectileMovementComponent::TickComponent rva=0x2fe4e90 (module docs). Returns the blocking hit that stopped
    /// the flight, if any (NotifyHit).
    pub fn tick_movement(&mut self, world: &dyn World, dt: f32, world_gravity_z: f32) -> Option<HitResult> {
        if !self.simulating || self.terminated {
            return None;
        }
        let g = self.gravity_z(world_gravity_z);
        let mut remaining = dt;
        let mut num_impacts = 0;
        let mut iterations = 0;
        let mut stopped = None;
        while self.simulating && remaining >= MIN_TICK_TIME && iterations < self.cfg.max_simulation_iterations {
            iterations += 1;
            // ShouldUseSubStepping rva=0x2fe45a0 -> GetSimulationTimeStep (inlined 0x142fe518b..0x142fe51b6)
            let sub = self.cfg.force_sub_stepping || g != 0.0;
            let mut tick = remaining;
            if sub {
                if remaining > self.cfg.max_simulation_time_step && iterations < self.cfg.max_simulation_iterations {
                    tick = minss(self.cfg.max_simulation_time_step, remaining * 0.5);
                }
                tick = maxss(tick, MIN_TICK_TIME);
            }
            remaining = remaining - tick;
            let old_v = self.velocity;
            let delta = self.compute_move_delta(old_v, tick, g);
            let k = 0.01f32;
            let quat = if self.cfg.rotation_follows_velocity && (old_v.x.abs() > k || old_v.y.abs() > k || old_v.z.abs() > k) {
                to_orientation_quat(old_v)
            } else {
                self.quat
            };
            let hit = self.move_box(world, delta, quat);
            if !hit.blocking_hit {
                self.previous_hit_time = 1.0;
                if self.velocity == old_v {
                    self.velocity = self.compute_velocity(self.velocity, tick, g);
                }
            } else {
                if self.velocity == old_v {
                    self.velocity = if hit.time > 1e-4 { self.compute_velocity(old_v, tick * hit.time, g) } else { old_v };
                }
                num_impacts += 1;
                let sub_remaining = (1.0 - hit.time) * tick;
                // HandleBlockingHit -> HandleImpact: bShouldBounce clear -> StopSimulating (Velocity zero, updated
                // component cleared, bSimulationEnabled false) -> Abort
                if !self.cfg.should_bounce {
                    self.velocity = FVector::ZERO;
                    self.simulating = false;
                    stopped = Some(hit);
                    break;
                }
                // no shipped class bounces: bShouldBounce is set by no package (extract/json) and no native ctor (the
                // AMordhauProjectile ctor clears it, `and byte ptr [rax + 0xf8], 0xfb` at 0x1415b5bf0); ComputeBounceResult /
                // HandleDeflection are not ported
                self.previous_hit_time = hit.time;
                if sub_remaining >= MIN_TICK_TIME {
                    remaining = remaining + sub_remaining;
                    if num_impacts <= self.cfg.bounce_additional_iterations {
                        iterations -= 1;
                    }
                }
            }
        }
        stopped
    }

    /// AMordhauProjectile::SweepProjectile rva=0x1600190: the box from LastProjectileLocation to the current location
    /// against the targets' bodies (overlaps) and the world (the blocking hit that ends a multi sweep), sorted by time;
    /// LastProjectileLocation = current location
    pub fn sweep(&mut self, world: &dyn World, targets: &[ProjectileTarget]) -> Vec<ProjectileHit> {
        let start = self.last_location;
        let end = self.location;
        let d = sub(end, start);
        let mut hits: Vec<ProjectileHit> = Vec::new();
        // the world: the stopping hit of this frame's movement, or a sweep over the segment
        let wh = match self.pending_block.take() {
            Some(h) => Some(h),
            None if size_sq(d) > 0.0 => world.sweep_box(start, end, self.cfg.box_extent, self.quat, &self.cfg.box_responses).into_iter().find(|h| h.blocking_hit),
            None => None,
        };
        let world_time = wh.as_ref().map_or(1.0f32, |h| {
            if size_sq(d) > 0.0 {
                let dl = (size_sq(sub(h.location, start)) / size_sq(d)).sqrt();
                if dl <= 1.0 {
                    dl
                } else {
                    1.0
                }
            } else {
                0.0
            }
        });
        // bodies: the oriented box (BoxComp extent, the projectile's rotation) swept against each capsule body, exactly
        // (UWorld::ComponentSweepMulti of the BoxComp: earliest time the box comes within the capsule radius)
        for t in targets {
            let mut best: Option<(f32, &TargetBody)> = None;
            for b in &t.bodies {
                if let Some(tt) = box_capsule_time(start, d, self.cfg.box_extent, self.quat, b.a, b.b, b.radius) {
                    if tt <= world_time && best.map_or(true, |(bt, _)| tt < bt) {
                        best = Some((tt, b));
                    }
                }
            }
            if let Some((tt, b)) = best {
                let p = v(start.x + d.x * tt, start.y + d.y * tt, start.z + d.z * tt);
                // the impact: the capsule surface point closest to the box at the hit
                let bp = closest_box_point_to_segment(p, self.cfg.box_extent, self.quat, b.a, b.b);
                let c = closest_on_segment(b.a, b.b, bp);
                let n = safe_normal(sub(p, c));
                let ip = v(c.x + n.x * b.radius, c.y + n.y * b.radius, c.z + n.z * b.radius);
                hits.push(ProjectileHit { target: Some((t.id, b.bone.clone())), time: tt, location: p, impact_point: ip, impact_normal: n, surface: b.surface, blocking: false });
            }
        }
        hits.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
        if let Some(h) = wh {
            hits.push(ProjectileHit { target: None, time: world_time, location: h.location, impact_point: h.impact_point, impact_normal: h.impact_normal, surface: 0, blocking: true });
        }
        self.last_location = self.location;
        hits
    }

    /// One frame: the movement component's tick, then AMordhauProjectile::Tick (UpdateProjectileState's spin, then
    /// SweepProjectile over the path since the last sweep). The movement ticks first: UMovementComponent::
    /// RegisterComponentTickFunctions rva=0x2fb8b90 makes the owner's PrimaryActorTick depend on the component's
    /// (FTickFunction::AddPrerequisite at 0x142fb8be2) when bTickBeforeOwner (+0xe8 bit 2, set by the UMovementComponent
    /// ctor at 0x142f98379, not cleared by UProjectileMovementComponent or AMordhauProjectile) and both can tick (the
    /// AMordhauProjectile ctor sets PrimaryActorTick.bCanEverTick). A blocking hit of the movement is swept at once
    /// (NotifyHit -> SweepProjectile), which leaves the actor tick's sweep empty. Returns the hits in order; the caller
    /// (combat side, ProcessProjectileHit) resolves each with `stick` / `terminate` / nothing (pass through) and stops
    /// at the first that terminates.
    pub fn tick(&mut self, world: &dyn World, targets: &[ProjectileTarget], dt: f32, world_gravity_z: f32) -> Vec<ProjectileHit> {
        if self.terminated {
            return Vec::new();
        }
        let mut hits = Vec::new();
        if let Some(h) = self.tick_movement(world, dt, world_gravity_z) {
            self.pending_block = Some(h);
            hits.extend(self.sweep(world, targets));
        }
        // UpdateProjectileState rva=0x16048f0: SpinComponent += RotationSpin * dt (Y pitch, Z yaw, X roll)
        self.spin = v(self.spin.x + dt * self.cfg.rotation_spin.x, self.spin.y + dt * self.cfg.rotation_spin.y, self.spin.z + dt * self.cfg.rotation_spin.z);
        hits.extend(self.sweep(world, targets));
        hits
    }

    /// AMordhauProjectile::WillSticky rva=0x16052e0: an entry 0 (any) or the surface
    pub fn will_sticky(&self, surface: u8) -> bool {
        self.cfg.will_sticky_on.iter().any(|&s| s == 0) || self.cfg.will_sticky_on.contains(&surface)
    }

    /// AMordhauProjectile::WillPassThrough rva=0x1605290
    pub fn will_pass_through(&self, surface: u8) -> bool {
        self.cfg.will_pass_through_on.iter().any(|&s| s == 0) || self.cfg.will_pass_through_on.contains(&surface)
    }

    /// AttachProjectile_Implementation rva=0x15c4f90, the surface case (read off the decomp, 0x1415c6..): Target =
    /// FRotationMatrix::MakeFromXZ(-ImpactNormal, the actor's up vector).Rotator(); Current = the actor rotation
    /// (FQuat::Rotator); the new rotation keeps Current's roll, Pitch = Current.Pitch + FindDeltaAngleDegrees(Current.
    /// Pitch, Target.Pitch) * StickySurfacePitchBlend, Yaw = FindDeltaAngleDegrees(Current.Yaw, Target.Yaw) *
    /// StickySurfaceYawBlend + Current.Yaw; the projectile is placed with its StickyPoint at the hit (`sticky_offset`:
    /// the socket's offset from the root in the projectile's frame, the host's; zero = the root at the impact point)
    /// and attached. Characters use AAdvancedCharacter::GetBestStickyLocation (combat side) for the point and normal.
    pub fn stick(&mut self, hit: &ProjectileHit, sticky_offset: FVector) {
        let up = quat_rotate(self.quat, v(0.0, 0.0, 1.0));
        let n = v(-hit.impact_normal.x, -hit.impact_normal.y, -hit.impact_normal.z);
        let (tp, ty, _) = matrix_rotator(&make_from_xz(n, up));
        let (cp, cy, cr) = quat_rotator(self.quat);
        let dp = find_delta_angle_degrees(cp, tp);
        let dy = find_delta_angle_degrees(cy, ty);
        let yaw = dy * self.cfg.sticky_surface_yaw_blend + cy;
        let pitch = cp + dp * self.cfg.sticky_surface_pitch_blend;
        self.quat = rotator_quaternion(pitch, yaw, cr);
        let off = quat_rotate(self.quat, sticky_offset);
        self.location = v(hit.impact_point.x - off.x, hit.impact_point.y - off.y, hit.impact_point.z - off.z);
        self.stuck = Some((hit.target.as_ref().map(|t| t.0), hit.target.as_ref().map_or(String::new(), |t| t.1.clone())));
        self.terminate();
    }

    /// AMordhauProjectile::TerminateProjectile rva=0x1600be0 (movement part): bHasTerminated, the movement's
    /// updated component cleared and Velocity zeroed, the collision disabled
    pub fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        self.simulating = false;
        self.velocity = FVector::ZERO;
    }
}

fn closest_on_segment(a: FVector, b: FVector, p: FVector) -> FVector {
    let ab = sub(b, a);
    let l = size_sq(ab);
    if l <= 0.0 {
        return a;
    }
    let t = dot(sub(p, a), ab) / l;
    let t = if t < 0.0 { 0.0 } else if t > 1.0 { 1.0 } else { t };
    v(a.x + ab.x * t, a.y + ab.y * t, a.z + ab.z * t)
}


/// a box (half extent `e`, rotation `q`) centred at `c` and the segment a-b: (distance, the segment point, the box
/// point) of their closest pair, f64 (the PhysX query this stands in for)
fn box_segment_closest(c: [f64; 3], e: [f64; 3], q: Quat, a: [f64; 3], b: [f64; 3]) -> (f64, [f64; 3], [f64; 3]) {
    let qi = quat_inverse(q);
    let to_local = |p: [f64; 3]| {
        let l = quat_rotate(qi, v((p[0] - c[0]) as f32, (p[1] - c[1]) as f32, (p[2] - c[2]) as f32));
        [l.x as f64, l.y as f64, l.z as f64]
    };
    let (la, lb) = (to_local(a), to_local(b));
    let at = |s: f64| [la[0] + (lb[0] - la[0]) * s, la[1] + (lb[1] - la[1]) * s, la[2] + (lb[2] - la[2]) * s];
    let clampb = |p: [f64; 3]| [p[0].clamp(-e[0], e[0]), p[1].clamp(-e[1], e[1]), p[2].clamp(-e[2], e[2])];
    let d2 = |s: f64| {
        let p = at(s);
        let k = clampb(p);
        (p[0] - k[0]).powi(2) + (p[1] - k[1]).powi(2) + (p[2] - k[2]).powi(2)
    };
    // the squared distance from a point moving along a line to a box is convex
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..80 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if d2(m1) <= d2(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let s = (lo + hi) * 0.5;
    let k = clampb(at(s));
    let w = quat_rotate(q, v(k[0] as f32, k[1] as f32, k[2] as f32));
    (d2(s).sqrt(), [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s, a[2] + (b[2] - a[2]) * s], [w.x as f64 + c[0], w.y as f64 + c[1], w.z as f64 + c[2]])
}

/// The earliest t in [0, 1] where the box (half extent `e`, rotation `q`) centred at start + d * t comes within `r` of
/// the segment a-b (a start inside gives 0). The distance between two convex sets moving linearly is convex in t:
/// the minimum, then the first crossing before it.
fn box_capsule_time(start: FVector, d: FVector, e: FVector, q: Quat, a: FVector, b: FVector, r: f32) -> Option<f32> {
    let f = |x: FVector| [x.x as f64, x.y as f64, x.z as f64];
    let (o, dv, pa, pb, ee, r) = (f(start), f(d), f(a), f(b), f(e), r as f64);
    let dist = |t: f64| box_segment_closest([o[0] + dv[0] * t, o[1] + dv[1] * t, o[2] + dv[2] * t], ee, q, pa, pb).0;
    if dist(0.0) <= r {
        return Some(0.0);
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..80 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if dist(m1) < dist(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let tmin = (lo + hi) * 0.5;
    if dist(tmin) > r {
        return None;
    }
    let (mut a0, mut b0) = (0.0f64, tmin);
    for _ in 0..60 {
        let m = (a0 + b0) * 0.5;
        if dist(m) <= r {
            b0 = m;
        } else {
            a0 = m;
        }
    }
    Some(b0 as f32)
}

/// the point of the box centred at `c` closest to the segment a-b
fn closest_box_point_to_segment(c: FVector, e: FVector, q: Quat, a: FVector, b: FVector) -> FVector {
    let f = |x: FVector| [x.x as f64, x.y as f64, x.z as f64];
    let k = box_segment_closest(f(c), f(e), q, f(a), f(b)).2;
    v(k[0] as f32, k[1] as f32, k[2] as f32)
}

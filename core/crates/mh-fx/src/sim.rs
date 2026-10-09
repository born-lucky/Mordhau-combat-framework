//! CPU simulation of a Cascade particle system (LOD 0 of every emitter), engine-neutral, UE space (cm, Z up, s).
//! A port of Mordhau's own Cascade code (Mordhau-Win64-Shipping.exe; rvas from extract/split/symbols.tsv, read with
//! scripts/ue_dis.py); distributions through mh-assets particles (FRawDistribution*::GetValue lookup tables).
//!
//! FParticleEmitterInstance::Tick 0x3299f40 order: Tick_EmitterTimeSetup 0x329a6c0 -> KillParticles 0x328b270
//! (RelativeTime > 1) -> ResetParticleParameters 0x328e350 -> Tick_ModuleUpdate 0x329af90 -> Tick_SpawnParticles
//! 0x329b090 / Spawn 0x3295c30 -> Tick_ModulePostUpdate -> UpdateBoundingBox 0x329e830 (integration) -> EmitterTime
//! += EmitterDelay. FBaseParticle layout (offsets in those functions): OldLocation 0x0, RelativeTime 0xc, Location
//! 0x10, OneOverMaxLifetime 0x1c, BaseVelocity 0x20, Rotation 0x2c, Velocity 0x30, BaseRotationRate 0x3c, BaseSize
//! 0x40, RotationRate 0x4c, Size 0x50, Flags 0x5c, Color 0x60, BaseColor 0x70. PreSpawn 0x328d360 zeroes the
//! particle: without a Size / Color / Lifetime module the size and colour are 0 and the particle never ages.
//!
//! Not simulated (counted in `unsupported`): mesh rotation, lights, events (EventGenerator / Receiver), orbit,
//! attractors, SpawnPerUnit, VelocityInheritParent, ParameterDynamic; local-space emitters are simulated in world
//! space; spawn interpolation of a moving emitter (PostSpawn 0x328ce50 first branch) is not needed (static origin).
//! Collision traces against one ground plane (`ground_z`), not the level (the exe traces the world).

use mh_assets::particles::{Dist, Lod, Module, ParticleSystem, RandomStream};
use serde_json::Value;

/// FBaseParticle Flags bits (ResetParticleParameters / UpdateBoundingBox / Collision::Update tests)
pub const JUST_SPAWNED: u32 = 0x0200_0000;
pub const FREEZE: u32 = 0x0400_0000;
pub const IGNORE_COLLISIONS: u32 = 0x0800_0000;
pub const FREEZE_TRANSLATION: u32 = 0x1000_0000;
pub const FREEZE_ROTATION: u32 = 0x2000_0000;
pub const DELAY_COLLISIONS: u32 = 0x4000_0000;

#[derive(Clone, Debug, Default)]
pub struct Particle {
    pub old_pos: [f32; 3],
    pub rel_time: f32,
    pub pos: [f32; 3],
    pub one_over_life: f32,
    pub base_vel: [f32; 3],
    pub rot: f32,
    pub vel: [f32; 3],
    pub base_rot_rate: f32,
    /// signed: a negative axis is a flipped UV (Size::SpawnEx UVFlippingMode)
    pub base_size: [f32; 3],
    pub rot_rate: f32,
    pub size: [f32; 3],
    pub flags: u32,
    pub color: [f32; 4],
    pub base_color: [f32; 4],
    /// SubUV payload ImageIndex
    pub sub_image: f32,
    /// ParameterDynamic payload (the sprite vertex factory's DynamicParameter; 1 without the module)
    pub dynp: [f32; 4],
    /// Collision payload: UsedDampingFactor, UsedDampingFactorRotation, UsedCollisions, Delay
    pub damping: [f32; 3],
    pub damping_rot: [f32; 3],
    pub collisions: i32,
    pub delay: f32,
    killed: bool,
}

#[derive(Clone, Debug)]
pub struct EmitterSim {
    pub name: String,
    pub material: String,
    pub velocity_aligned: bool,
    /// PSA_Square (the Required default), PSA_FacingCameraPosition or PSA_FacingCameraDistanceBlend: the sprite height
    /// is its width (UE 4.21 ParticleSystemRender.cpp GetParticleSize, engine source; P_spark_burst's flash has
    /// StartSize (3, 0, 0) and drew as a zero-height quad without this)
    pub square: bool,
    pub sub_images: [u32; 2],
    /// EParticleSubUVInterpMethod: 0 none, 1 linear, 2 linear blend, 3 random, 4 random blend
    pub sub_method: u8,
    pub particles: Vec<Particle>,
    /// EmitterTime (s; negative while delayed inside a tick)
    pub time: f32,
    pub seconds: f32,
    duration: f32,
    delay: f32,
    loops: i64,
    loop_count: i64,
    delay_first_loop_only: bool,
    legacy_spawning: bool,
    quality_rate: f32,
    spawn_fraction: f32,
    burst_fired: Vec<bool>,
    first_time: bool,
    last_rate: f32,
    origin_offset: [f32; 3],
    lod: Lod,
    pub finished: bool,
    pub spawned: usize,
    pub collided: usize,
}

pub struct SystemSim {
    pub package: String,
    pub origin: [f32; 3],
    /// the system's rotation (UE): rows = its X, Y, Z axes; X = the requested direction
    pub basis: [[f32; 3]; 3],
    pub emitters: Vec<EmitterSim>,
    pub rng: RandomStream,
    pub unsupported: Vec<String>,
    /// ground plane height (UE z, cm) the Collision module traces against; None = no collisions
    pub ground_z: Option<f32>,
}

const SIMULATED: &[&str] = &[
    "ParticleModuleLifetime", "ParticleModuleSize", "ParticleModuleVelocity", "ParticleModuleColor", "ParticleModuleColorOverLife",
    "ParticleModuleSizeMultiplyLife", "ParticleModuleAccelerationConstant", "ParticleModuleAccelerationDrag", "ParticleModuleLocation",
    "ParticleModuleLocationPrimitiveSphere", "ParticleModuleRotation", "ParticleModuleRotationRate", "ParticleModuleSubUV",
    "ParticleModuleVelocityOverLifetime", "ParticleModuleCollision", "ParticleModuleTypeDataGpu", "ParticleModuleParameterDynamic", "ParticleModuleColorScaleOverLife",
];

fn f(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn b(v: &Value, k: &str, d: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn add(a: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    [a[0] + c[0], a[1] + c[1], a[2] + c[2]]
}
fn sub(a: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    [a[0] - c[0], a[1] - c[1], a[2] - c[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    a.map(|x| x * s)
}
fn mul(a: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    [a[0] * c[0], a[1] * c[1], a[2] * c[2]]
}
fn dot(a: [f32; 3], c: [f32; 3]) -> f32 {
    a[0] * c[0] + a[1] * c[1] + a[2] * c[2]
}
/// emitter space -> world: v.x * X + v.y * Y + v.z * Z
fn rot(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| m[0][i] * v[0] + m[1][i] * v[1] + m[2][i] * v[2])
}
/// GetSafeNormal as Velocity::SpawnEx / LocationPrimitiveSphere::SpawnEx inline it: unit length kept, < 1e-8 -> 0
fn safe_normal(v: [f32; 3]) -> [f32; 3] {
    let l2 = dot(v, v);
    if l2 == 1.0 {
        v
    } else if l2 < 1e-8 {
        [0.0; 3]
    } else {
        scale(v, 1.0 / l2.sqrt())
    }
}

/// basis with X along `dir` (UE), Z as close to world up as possible
pub fn basis_from_dir(dir: [f32; 3]) -> [[f32; 3]; 3] {
    let x = safe_normal(dir);
    if x == [0.0; 3] {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let up = if x[2].abs() > 0.99 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let cross = |a: [f32; 3], c: [f32; 3]| [a[1] * c[2] - a[2] * c[1], a[2] * c[0] - a[0] * c[2], a[0] * c[1] - a[1] * c[0]];
    let y = safe_normal(cross(up, x));
    let z = cross(x, y);
    [x, y, z]
}

impl SystemSim {
    pub fn new(ps: &ParticleSystem, origin: [f32; 3], dir: [f32; 3], seed: u32) -> SystemSim {
        let mut unsupported = vec![];
        let mut rng = RandomStream(seed);
        let emitters = ps
            .emitters
            .iter()
            .filter_map(|e| {
                let lod = e.lods.first()?.clone();
                let req = lod.required.as_ref().map(|m| m.props.clone()).unwrap_or(Value::Null);
                if !b(&req, "bEnabled", true) {
                    return None;
                }
                for m in &lod.modules {
                    if !SIMULATED.contains(&m.class.as_str()) && !unsupported.contains(&m.class) {
                        unsupported.push(m.class.clone());
                    }
                }
                // SetupEmitterDuration 0x32927d0: CurrentDelay = EmitterDelay (Low + r * (Delay - Low) with
                // bEmitterDelayUseRange), EmitterDuration = Duration (or its range) + CurrentDelay
                let mut delay = f(&req, "EmitterDelay", 0.0) as f32;
                if b(&req, "bEmitterDelayUseRange", false) {
                    let lo = f(&req, "EmitterDelayLow", 0.0) as f32;
                    delay = lo + rng.fraction() * (delay - lo);
                }
                let mut dur = f(&req, "EmitterDuration", 1.0) as f32;
                if b(&req, "bEmitterDurationUseRange", false) {
                    let lo = f(&req, "EmitterDurationLow", 0.0) as f32;
                    dur = lo + rng.fraction() * (dur - lo);
                }
                let method = match req.get("InterpolationMethod").and_then(Value::as_str).unwrap_or("") {
                    "PSUVIM_Linear" => 1,
                    "PSUVIM_Linear_Blend" => 2,
                    "PSUVIM_Random" => 3,
                    "PSUVIM_Random_Blend" => 4,
                    _ => 0,
                };
                let o = req.get("EmitterOrigin");
                let g = |k: &str| o.and_then(|o| o.get(k)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let bursts = lod.bursts().len();
                Some(EmitterSim {
                    name: e.name.clone(),
                    material: lod.material(),
                    velocity_aligned: req.get("ScreenAlignment").and_then(Value::as_str).is_some_and(|s| s.ends_with("PSA_Velocity")),
                    square: req.get("ScreenAlignment").and_then(Value::as_str).is_none_or(|s| s.ends_with("PSA_Square") || s.ends_with("PSA_FacingCameraPosition") || s.ends_with("PSA_FacingCameraDistanceBlend")),
                    sub_images: [f(&req, "SubImages_Horizontal", 1.0).max(1.0) as u32, f(&req, "SubImages_Vertical", 1.0).max(1.0) as u32],
                    sub_method: method,
                    particles: vec![],
                    time: 0.0,
                    seconds: 0.0,
                    duration: dur + delay,
                    delay,
                    loops: f(&req, "EmitterLoops", 0.0) as i64,
                    loop_count: 0,
                    delay_first_loop_only: b(&req, "bDelayFirstLoopOnly", false),
                    legacy_spawning: b(&e.props, "bUseLegacySpawningBehavior", false),
                    quality_rate: f(&e.props, "QualityLevelSpawnRateScale", 1.0) as f32,
                    spawn_fraction: 0.0,
                    burst_fired: vec![false; bursts],
                    first_time: true,
                    last_rate: 0.0,
                    origin_offset: [g("X"), g("Y"), g("Z")],
                    lod,
                    finished: false,
                    spawned: 0,
                    collided: 0,
                })
            })
            .collect();
        SystemSim { package: ps.package.clone(), origin, basis: basis_from_dir(dir), emitters, rng, unsupported, ground_z: None }
    }

    pub fn alive(&self) -> usize {
        self.emitters.iter().map(|e| e.particles.len()).sum()
    }
    pub fn done(&self) -> bool {
        self.emitters.iter().all(|e| e.finished && e.particles.is_empty())
    }

    pub fn step(&mut self, dt: f32) {
        let ctx = Ctx { origin: self.origin, basis: self.basis, ground_z: self.ground_z };
        for e in &mut self.emitters {
            e.tick(dt, &ctx, &mut self.rng);
        }
    }
}

struct Ctx {
    origin: [f32; 3],
    basis: [[f32; 3]; 3],
    ground_z: Option<f32>,
}

fn d1(m: &Module, k: &str, t: f32, r: &mut dyn FnMut() -> f32) -> f32 {
    m.dists.get(k).map_or(0.0, |d: &Dist| d.value1(t, r))
}
fn d3(m: &Module, k: &str, t: f32, r: &mut dyn FnMut() -> f32) -> [f32; 3] {
    m.dists.get(k).map_or([0.0; 3], |d: &Dist| d.value3(t, r))
}

impl EmitterSim {
    fn tick(&mut self, dt: f32, ctx: &Ctx, rng: &mut RandomStream) {
        // Tick_EmitterTimeSetup 0x329a6c0 (the non-legacy EmitterTime branch)
        self.seconds += dt;
        self.time += dt;
        let looped = self.duration > 0.0 && self.time >= self.duration;
        let mut delay = self.delay;
        if looped {
            self.loop_count += 1;
            self.burst_fired.iter_mut().for_each(|x| *x = false); // ResetBurstList 0x328e2e0
            self.time -= self.duration;
        }
        if self.delay_first_loop_only && self.loop_count > 0 {
            delay = 0.0;
        }
        self.time -= delay;
        // KillParticles 0x328b270: RelativeTime > 1
        self.particles.retain(|p| p.rel_time <= 1.0);
        // ResetParticleParameters 0x328e350 (RelativeTime of a just-spawned particle advances only with
        // bUseLegacySpawningBehavior, UParticleEmitter +0x37)
        let skip_new = !self.legacy_spawning;
        for p in &mut self.particles {
            p.vel = p.base_vel;
            p.size = p.base_size.map(f32::abs);
            p.rot_rate = p.base_rot_rate;
            p.color = p.base_color;
            if !(skip_new && p.flags & JUST_SPAWNED != 0) {
                p.rel_time += dt * p.one_over_life;
            }
        }
        // Tick_ModuleUpdate 0x329af90 (LOD update modules in order)
        let mods = std::mem::take(&mut self.lod.modules);
        {
            let mut r = || rng.fraction();
            for m in &mods {
                self.update(m, dt, ctx, &mut r);
            }
        }
        self.lod.modules = mods;
        // Tick_SpawnParticles 0x329b090: EmitterTime >= 0 and (EmitterLoops == 0 or LoopCount < EmitterLoops or
        // SecondsSinceCreation < EmitterDuration * EmitterLoops or the first tick)
        let can = self.time >= 0.0
            && (self.loops == 0 || self.loop_count < self.loops || self.seconds < self.duration * self.loops as f32 || self.first_time);
        if can {
            self.spawn_fraction = self.spawn(dt, ctx, rng);
        }
        // nothing more will spawn: the loops are done, or a non-looping (duration 0) emitter has no rate left and has
        // fired its bursts
        if (self.loops > 0 && !can && self.time >= 0.0) || (self.duration <= 0.0 && self.last_rate <= 0.0 && self.burst_fired.iter().all(|x| *x)) {
            self.finished = true;
        }
        // UpdateBoundingBox 0x329e830: OldLocation = Location, clear JustSpawned, integrate unless frozen (a just
        // spawned particle is integrated in its first tick only with bUseLegacySpawningBehavior)
        for p in &mut self.particles {
            p.old_pos = p.pos;
            let just = p.flags & JUST_SPAWNED != 0;
            p.flags &= !JUST_SPAWNED;
            if p.flags & FREEZE != 0 || (just && skip_new) {
                continue;
            }
            if p.flags & FREEZE_TRANSLATION == 0 {
                p.pos = add(p.pos, scale(p.vel, dt));
            }
            if p.flags & FREEZE_ROTATION == 0 {
                p.rot += dt * p.rot_rate;
            }
        }
        self.time += delay;
        self.first_time = false;
    }

    /// FParticleEmitterInstance::Spawn 0x3295c30 (+ GetCurrentBurstRateOffset 0x3283d40); returns the new leftover
    fn spawn(&mut self, dt: f32, ctx: &Ctx, rng: &mut RandomStream) -> f32 {
        let et = self.time;
        let old = self.spawn_fraction;
        let Some(sp) = self.lod.spawn.clone() else { return old };
        let mut burst = 0i64;
        let rate;
        {
            let mut r = || rng.fraction();
            // SpawnRate = Rate(EmitterTime) * RateScale(EmitterTime) * GetGlobalRateScale (1: no scalability cvar)
            rate = (d1(&sp, "Rate", et, &mut r) * sp.dists.get("RateScale").map_or(1.0, |d| d.value1(et, &mut r))).max(0.0);
            let bscale = sp.dists.get("BurstScale");
            for (i, (count, low, t)) in self.lod.bursts().into_iter().enumerate() {
                if self.burst_fired[i] || et < t as f32 {
                    continue;
                }
                // CountLow > -1: CountLow + trunc(r * (Count - CountLow + 1)); x BurstScale(EmitterTime), ceil
                let n = if low > -1 {
                    let span = (count - low + 1) as f32;
                    low + if span > 0.0 { (r() * span) as i64 } else { 0 }
                } else {
                    count
                };
                let s = bscale.map_or(1.0, |d| d.value1(et, &mut r));
                burst += (n as f32 * s).ceil() as i64;
                self.burst_fired[i] = true;
            }
        }
        self.last_rate = rate;
        // GetQualityLevelSpawnRateMult: QualityLevelSpawnRateScale (1 when unset); bursts ceil
        let q = self.quality_rate;
        let rate = (rate * q).max(0.0);
        let burst = (burst as f32 * q).ceil() as i64;
        if rate <= 0.0 && burst <= 0 {
            return old;
        }
        let left = old + dt * rate;
        let number = left.floor() as i64;
        let inc = if rate > 0.0 { 1.0 / rate } else { 0.0 };
        let start = dt + old * inc - inc;
        // bursts: BurstIncrement = 1 / BurstCount and BurstStartTime = dt * BurstIncrement with
        // bUseLegacySpawningBehavior, else both 0
        let (binc, bstart) = if self.legacy_spawning && burst > 0 { (1.0 / burst as f32, dt / burst as f32) } else { (0.0, 0.0) };
        // SpawnParticles worker 0x3279d70: SpawnTime starts at StartTime and steps down by Increment
        for i in 0..number.min(2000) {
            self.spawn_one(start - i as f32 * inc, ctx, rng);
        }
        for i in 0..burst.min(2000) {
            self.spawn_one(bstart - i as f32 * binc, ctx, rng);
        }
        left - number as f32
    }

    fn spawn_one(&mut self, st: f32, ctx: &Ctx, rng: &mut RandomStream) {
        // PreSpawn 0x328d360: zeroed, Location = the emitter's location (system origin + Required EmitterOrigin)
        let mut p = Particle { pos: add(ctx.origin, rot(&ctx.basis, self.origin_offset)), dynp: [1.0; 4], ..Default::default() };
        let mods = std::mem::take(&mut self.lod.modules);
        {
            let mut r = || rng.fraction();
            for m in &mods {
                self.spawn_module(m, &mut p, st, ctx, &mut r);
            }
        }
        self.lod.modules = mods;
        // PostSpawn 0x328ce50: OldLocation = Location, Location += SpawnTime * Velocity, JustSpawned
        p.old_pos = p.pos;
        p.pos = add(p.pos, scale(p.vel, st));
        p.flags |= JUST_SPAWNED;
        self.spawned += 1;
        // the worker kills a particle born with RelativeTime > 1
        if p.rel_time <= 1.0 {
            self.particles.push(p);
        }
    }

    fn spawn_module(&self, m: &Module, p: &mut Particle, st: f32, ctx: &Ctx, r: &mut dyn FnMut() -> f32) {
        let et = self.time;
        let tau = std::f32::consts::TAU;
        match m.class.as_str() {
            // Lifetime::Spawn 0x3296980
            "ParticleModuleLifetime" => {
                let l = d1(m, "Lifetime", et, r);
                if p.one_over_life > 0.0 {
                    p.one_over_life = 1.0 / (1.0 / p.one_over_life + l);
                } else if l > 0.0 {
                    p.one_over_life = 1.0 / l;
                }
                if p.rel_time <= 1.0 {
                    p.rel_time = p.one_over_life * st;
                }
            }
            // Size::SpawnEx 0x32d4240: Size += v; BaseSize += v flipped per Required UVFlippingMode
            "ParticleModuleSize" => {
                let v = d3(m, "StartSize", et, r);
                p.size = add(p.size, v);
                let mode = self.lod.required.as_ref().and_then(|q| q.props.get("UVFlippingMode")).and_then(Value::as_str).unwrap_or("");
                let mut w = v;
                let mode = mode.rsplit("::").next().unwrap_or(mode);
                match mode {
                    "FlipUV" => w = [-w[0], -w[1], -w[2]],
                    "FlipUOnly" => w[0] = -w[0],
                    "FlipVOnly" => w[1] = -w[1],
                    "RandomFlipUV" => {
                        if r() > 0.5 {
                            w = [-w[0], -w[1], -w[2]]
                        }
                    }
                    "RandomFlipUOnly" => {
                        if r() > 0.5 {
                            w[0] = -w[0]
                        }
                    }
                    "RandomFlipVOnly" => {
                        if r() > 0.5 {
                            w[1] = -w[1]
                        }
                    }
                    "RandomFlipUVIndependent" => {
                        if r() > 0.5 {
                            w[0] = -w[0]
                        }
                        if r() > 0.5 {
                            w[1] = -w[1]
                        }
                    }
                    _ => {}
                }
                p.base_size = add(p.base_size, w);
            }
            // Velocity::SpawnEx 0x32d4470: StartVelocity (emitter space unless bInWorldSpace) + FromOrigin * Radial
            "ParticleModuleVelocity" => {
                let mut v = d3(m, "StartVelocity", et, r);
                if !b(&m.props, "bInWorldSpace", false) {
                    v = rot(&ctx.basis, v);
                }
                let from = safe_normal(sub(p.pos, ctx.origin));
                let rad = d1(m, "StartVelocityRadial", et, r);
                let v = add(v, scale(from, rad));
                p.vel = add(p.vel, v);
                p.base_vel = add(p.base_vel, v);
            }
            // Color::Spawn 0x32d08d0: Color = BaseColor = (StartColor, StartAlpha) at EmitterTime
            "ParticleModuleColor" => {
                let c = d3(m, "StartColor", et, r);
                let a = d1(m, "StartAlpha", et, r);
                p.color = [c[0], c[1], c[2], a];
                p.base_color = p.color;
            }
            // ColorOverLife::Spawn 0x32d0980: the same at RelativeTime
            "ParticleModuleColorOverLife" => {
                let c = d3(m, "ColorOverLife", p.rel_time, r);
                let a = d1(m, "AlphaOverLife", p.rel_time, r);
                p.color = [c[0], c[1], c[2], a];
                p.base_color = p.color;
            }
            // SizeMultiplyLife::Spawn 0x32d2640: Size *= LifeMultiplier(RelativeTime) per MultiplyX/Y/Z
            "ParticleModuleSizeMultiplyLife" => mul_size(m, p, r),
            // AccelerationConstant::Spawn 0x3296540: Velocity, BaseVelocity += A * SpawnTime
            "ParticleModuleAccelerationConstant" => {
                let a = accel(m);
                p.vel = add(p.vel, scale(a, st));
                p.base_vel = add(p.base_vel, scale(a, st));
            }
            // Location::SpawnEx 0x32d2ac0: Location += StartLocation (emitter space)
            "ParticleModuleLocation" => {
                let v = d3(m, "StartLocation", et, r);
                p.pos = add(p.pos, rot(&ctx.basis, v));
            }
            // LocationPrimitiveSphere::SpawnEx 0x32d34a0 + LocationPrimitiveBase::DetermineUnitDirection 0x32b2790
            "ParticleModuleLocationPrimitiveSphere" => {
                let off = d3(m, "StartLocation", et, r);
                let pr = &m.props;
                // flags +0x30: Positive X/Y/Z 1/2/4, Negative X/Y/Z 8/0x10/0x20, SurfaceOnly 0x40, Velocity 0x80
                let px = [b(pr, "Positive_X", true), b(pr, "Positive_Y", true), b(pr, "Positive_Z", true)];
                let nx = [b(pr, "Negative_X", true), b(pr, "Negative_Y", true), b(pr, "Negative_Z", true)];
                let rnd = [r(), r(), r()];
                let mut u = [0.0f32; 3];
                for i in 0..3 {
                    u[i] = match (px[i], nx[i]) {
                        (true, true) => rnd[i] * 2.0 - 1.0,
                        (true, false) => rnd[i],
                        (false, true) => -rnd[i],
                        _ => 0.0,
                    };
                }
                let n = safe_normal(u);
                if b(pr, "SurfaceOnly", false) {
                    u = n;
                }
                let rad = d1(m, "StartRadius", et, r);
                // per enabled axis: clamp(u * R, -|n| R, |n| R); a disabled axis is 0
                let mut c = [0.0f32; 3];
                for i in 0..3 {
                    if px[i] || nx[i] {
                        let mx = n[i].abs() * rad;
                        let v = u[i] * rad;
                        c[i] = if v < -mx { -mx } else { v.min(mx) };
                    }
                }
                p.pos = add(p.pos, rot(&ctx.basis, add(c, off)));
                if b(pr, "Velocity", false) {
                    let v = rot(&ctx.basis, scale(c, d1(m, "VelocityScale", et, r)));
                    p.vel = add(p.vel, v);
                    p.base_vel = add(p.base_vel, v);
                }
            }
            // Rotation::Spawn 0x3297050, RotationRate::Spawn 0x32970c0 (turns -> radians, 6.28319)
            "ParticleModuleRotation" => p.rot += d1(m, "StartRotation", et, r) * tau,
            "ParticleModuleRotationRate" => {
                let v = d1(m, "StartRotationRate", et, r) * tau;
                p.rot_rate += v;
                p.base_rot_rate += v;
            }
            // SubUV::Spawn 0x3297170 -> DetermineImageIndex 0x3280660
            "ParticleModuleSubUV" => {
                if self.sub_method != 0 {
                    p.sub_image = self.image_index(m, p, r)
                }
            }
            // VelocityOverLifetime::Spawn 0x32d29e0: Absolute -> Velocity = BaseVelocity = VelOverLife(RelativeTime)
            "ParticleModuleVelocityOverLifetime" => {
                if b(&m.props, "Absolute", false) {
                    let v = d3(m, "VelOverLife", p.rel_time, r);
                    p.vel = v;
                    p.base_vel = v;
                }
            }
            // Collision::Spawn 0x32d07b0: the payload; Delay > SpawnTime delays collisions
            // ColorScaleOverLife::Spawn 0x32d0a20: Color (not BaseColor) *= ColorScaleOverLife(t) / AlphaScaleOverLife
            // (RelativeTime); t = EmitterTime with bEmitterTime (+0xa8), else RelativeTime
            "ParticleModuleColorScaleOverLife" => color_scale(m, p, et, p.rel_time, r),
            // ParameterDynamic::SpawnEx 0x32d3f10
            "ParticleModuleParameterDynamic" => dyn_params(m, p, et, false, r),
            "ParticleModuleCollision" => {
                p.damping = d3(m, "DampingFactor", et, r);
                p.damping_rot = d3(m, "DampingFactorRotation", et, r);
                p.collisions = d1(m, "MaxCollisions", et, r).round() as i32;
                p.delay = d1(m, "DelayAmount", et, r);
                if p.delay > st {
                    p.flags = (p.flags & 0x3fff_ffff) | DELAY_COLLISIONS;
                }
            }
            _ => {}
        }
    }

    /// DetermineImageIndex 0x3280660: Linear floors SubImageIndex(RelativeTime), Linear_Blend keeps the fraction
    fn image_index(&self, m: &Module, p: &Particle, r: &mut dyn FnMut() -> f32) -> f32 {
        match self.sub_method {
            1 => d1(m, "SubImageIndex", p.rel_time, r).floor(),
            2 => d1(m, "SubImageIndex", p.rel_time, r),
            // random methods (RandomImageChanges / RandomImageTime): UNCONFIRMED, not in the combat effects
            3 | 4 => (r() * (self.sub_images[0] * self.sub_images[1]) as f32).floor(),
            _ => p.sub_image,
        }
    }

    fn update(&mut self, m: &Module, dt: f32, ctx: &Ctx, r: &mut dyn FnMut() -> f32) {
        let mut hits = 0;
        for i in 0..self.particles.len() {
            if self.particles[i].flags & FREEZE != 0 {
                continue;
            }
            if m.class == "ParticleModuleSubUV" {
                // SubUV::Update 0x329e5e0 (particles with RelativeTime <= 1)
                let q = &self.particles[i];
                if self.sub_method != 0 && q.rel_time <= 1.0 {
                    let v = self.image_index(m, q, r);
                    self.particles[i].sub_image = v;
                }
                continue;
            }
            let p = &mut self.particles[i];
            match m.class.as_str() {
                // ColorOverLife::Update 0x32d72d0
                "ParticleModuleColorOverLife" => {
                    let c = d3(m, "ColorOverLife", p.rel_time, r);
                    let a = d1(m, "AlphaOverLife", p.rel_time, r);
                    p.color = [c[0], c[1], c[2], a];
                }
                // SizeMultiplyLife::Update 0x32d9f10
                "ParticleModuleSizeMultiplyLife" => mul_size(m, p, r),
                // AccelerationConstant::Update 0x329ba10
                "ParticleModuleAccelerationConstant" => {
                    let a = scale(accel(m), dt);
                    p.vel = add(p.vel, a);
                    p.base_vel = add(p.base_vel, a);
                }
                // AccelerationDrag::Update 0x329c280: d = -Velocity * DragCoefficient(RelativeTime) * dt
                "ParticleModuleAccelerationDrag" => {
                    let k = d1(m, "DragCoefficientRaw", p.rel_time, r);
                    let d = scale(p.vel, -k * dt);
                    p.vel = add(p.vel, d);
                    p.base_vel = add(p.base_vel, d);
                }
                // VelocityOverLifetime::Update 0x32da780: Absolute -> Velocity = v, else Velocity *= v
                "ParticleModuleVelocityOverLifetime" => {
                    let v = d3(m, "VelOverLife", p.rel_time, r);
                    p.vel = if b(&m.props, "Absolute", false) { v } else { mul(p.vel, v) };
                }
                // ColorScaleOverLife::Update 0x32d74f0 (frozen particles skipped above)
                "ParticleModuleColorScaleOverLife" => {
                    let a_t = if b(&m.props, "bEmitterTime", false) { self.time } else { p.rel_time };
                    color_scale(m, p, self.time, a_t, r)
                }
                // ParameterDynamic::Update 0x32d8be0: the parameters that are not bSpawnTimeOnly, the same way
                "ParticleModuleParameterDynamic" => dyn_params(m, p, self.time, true, r),
                // Collision::Update 0x32d5c20 against the ground plane
                "ParticleModuleCollision" => {
                    if let Some(gz) = ctx.ground_z {
                        if collide(m, p, gz) {
                            hits += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        if m.class == "ParticleModuleCollision" {
            self.collided += hits;
            // EPCC_Kill: the exe removes the particle from the active list inside the update
            self.particles.retain(|p| !p.killed);
        }
    }
}

/// UParticleModuleParameterDynamic::SpawnEx 0x32d3f10 per FEmitterDynamicParameter (+0x48 apart): ValueMethod (+0xc)
/// 1 AutoSet -> untouched; 0 UserSet -> scale 1; 2 / 3 / 4 VelocityX / Y / Z; 5 VelocityMag -> scale = that velocity
/// component (+0x30..0x38) / its length; value = ParamValue(t) x scale when UserSet or bScaleVelocityByParamValue (+0x10),
/// else scale; t = EmitterTime (+0x12c) with bUseEmitterTime (+0x8), else RelativeTime
fn dyn_params(m: &Module, p: &mut Particle, emitter_time: f32, update: bool, r: &mut dyn FnMut() -> f32) {
    let Some(ps) = m.props.get("DynamicParams").and_then(Value::as_array) else { return };
    for (i, dp) in ps.iter().enumerate().take(4) {
        if update && b(dp, "bSpawnTimeOnly", false) {
            continue;
        }
        let method = dp.get("ValueMethod").and_then(Value::as_str).unwrap_or("").rsplit("::").next().unwrap_or("").to_string();
        let scale = match method.as_str() {
            "EDPV_AutoSet" => continue,
            "EDPV_VelocityX" => p.vel[0],
            "EDPV_VelocityY" => p.vel[1],
            "EDPV_VelocityZ" => p.vel[2],
            "EDPV_VelocityMag" => dot(p.vel, p.vel).sqrt(),
            _ => 1.0,
        };
        let user = method == "EDPV_UserSet" || method.is_empty();
        let v = if user || b(dp, "bScaleVelocityByParamValue", false) {
            let t = if b(dp, "bUseEmitterTime", false) { emitter_time } else { p.rel_time };
            m.dists.get(&format!("DynamicParams[{i}].ParamValue")).map_or(0.0, |d| d.value1(t, r))
        } else {
            1.0
        };
        p.dynp[i] = v * scale;
    }
}

fn color_scale(m: &Module, p: &mut Particle, emitter_time: f32, alpha_t: f32, r: &mut dyn FnMut() -> f32) {
    let t = if b(&m.props, "bEmitterTime", false) { emitter_time } else { p.rel_time };
    let c = d3(m, "ColorScaleOverLife", t, r);
    let a = m.dists.get("AlphaScaleOverLife").map_or(1.0, |d| d.value1(alpha_t, r));
    p.color = [p.color[0] * c[0], p.color[1] * c[1], p.color[2] * c[2], p.color[3] * a];
}

fn mul_size(m: &Module, p: &mut Particle, r: &mut dyn FnMut() -> f32) {
    let k = d3(m, "LifeMultiplier", p.rel_time, r);
    let flags = [b(&m.props, "MultiplyX", true), b(&m.props, "MultiplyY", true), b(&m.props, "MultiplyZ", true)];
    for i in 0..3 {
        if flags[i] {
            p.size[i] *= k[i];
        }
    }
}

fn accel(m: &Module) -> [f32; 3] {
    let a = m.props.get("Acceleration");
    let g = |k: &str| a.and_then(|a| a.get(k)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [g("X"), g("Y"), g("Z")]
}

/// Collision::Update 0x32d5c20 for one particle and the plane z = gz (normal +Z, the hit time along the trace)
pub fn collide(m: &Module, p: &mut Particle, gz: f32) -> bool {
    if p.flags & (FREEZE | IGNORE_COLLISIONS | FREEZE_TRANSLATION | FREEZE_ROTATION) != 0 {
        return false;
    }
    if p.flags & DELAY_COLLISIONS != 0 {
        if p.rel_time < p.delay {
            return false;
        }
        p.flags &= !DELAY_COLLISIONS;
    }
    // trace OldLocation -> Location + Direction * Size / DirScalar (DirScalar +0x148)
    let step = sub(p.pos, p.old_pos);
    let dir = safe_normal(step);
    let ds = f(&m.props, "DirScalar", 3.5) as f32;
    let end = add(p.pos, scale(mul(dir, p.size), 1.0 / ds));
    let (z0, z1) = (p.old_pos[2], end[2]);
    if !(z0 >= gz && z1 < gz) {
        return false;
    }
    let t = if z0 - z1 > 0.0 { (z0 - gz) / (z0 - z1) } else { 0.0 };
    let hit = add(p.old_pos, scale(sub(end, p.old_pos), t));
    let n = [0.0f32, 0.0, 1.0];
    // the count drops unless bOnlyVerticalNormalsDecrementCount and |N.z| + VerticalFudgeFactor < 1
    let fudge = f(&m.props, "VerticalFudgeFactor", 0.1) as f32;
    if !b(&m.props, "bOnlyVerticalNormalsDecrementCount", false) || n[2].abs() + fudge >= 1.0 {
        p.collisions -= 1;
    }
    let mirror = |v: [f32; 3]| sub(v, scale(n, 2.0 * dot(v, n)));
    if p.collisions > 0 {
        // bounce: BaseVelocity mirrored x damping, BaseRotationRate x damping rotation X, Velocity 0, the rest of
        // the step continues along the mirrored direction (x |step| x damping x (1 - hit time))
        p.base_vel = mul(mirror(p.base_vel), p.damping);
        p.base_rot_rate *= p.damping_rot[0];
        let travel = dot(step, step).sqrt();
        let rest = mul(scale(mirror(dir), travel), p.damping);
        p.vel = [0.0; 3];
        p.pos = add(hit, scale(rest, 1.0 - t));
        return true;
    }
    // collisions used up: Location = the hit, then CollisionCompletionOption (jump table at +0xf0)
    p.pos = hit;
    let opt = m.props.get("CollisionCompletionOption").and_then(Value::as_str).unwrap_or("EPCC_Kill");
    match opt.rsplit("::").next().unwrap_or(opt) {
        "EPCC_Freeze" => p.flags |= FREEZE,
        "EPCC_HaltCollisions" => p.flags |= IGNORE_COLLISIONS,
        "EPCC_FreezeTranslation" => p.flags |= FREEZE_TRANSLATION,
        "EPCC_FreezeRotation" => p.flags |= FREEZE_ROTATION,
        "EPCC_FreezeMovement" => p.flags |= FREEZE_TRANSLATION | FREEZE_ROTATION,
        _ => p.killed = true,
    }
    true
}

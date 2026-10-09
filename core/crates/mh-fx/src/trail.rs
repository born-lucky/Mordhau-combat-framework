//! Weapon swing / blood trails: AMordhauWeapon's trail state and the anim-trail emitter it drives.
//!
//! The weapon (exe, decomp AMordhauWeapon.cpp):
//! - `AMordhauWeapon::StartTrail` 0x163fd40 (vtable +0x840): Duration = InDuration x TrailLifeTimeFactor (swing) or
//!   SwingTrailOriginalLifeTime (blood); gated by ShouldShowBlood (blood) and GameState ConsumeBudget(0.25 s,
//!   1000 cm); SpawnEmitterAttached(SwingTrailParticles | BloodTrailParticles) on the weapon mesh, then
//!   BeginTrails("TraceStart", "TraceEnd" (alternate mode: "SecondTraceStart" / "SecondTraceEnd"), width mode 0,
//!   width 1). Blood: BloodTrailEndTime = now + InDuration; swing: SwingTrailOriginalLifeTime = Duration,
//!   SwingTrailEndTime = now + InDuration + TrailExtraTime.
//!   Callers: UAttackMotion's release start (the woosh, UAttackMotion.cpp 2806: StartTrail(false, Release)); a flesh
//!   hit (surface 1) of the current attack, once (AMordhauWeapon.cpp 459: StartTrail(true, BloodTrailMaxDuration)).
//! - `AMordhauWeapon::UpdateParticleTrails` 0x1641540: every update the emitters' "TrailLifeTime" instance parameter
//!   = min(stop fade, 1 - end fade) x SwingTrailOriginalLifeTime + 0.001, the end fade a smoothstep over the last
//!   SwingTrailFadeOutDuration before the end time (swing / blood each), the stop fade smoothstep(5 x
//!   TrailTimeBeforeStop) while a delayed stop runs.
//! - `AMordhauWeapon::StopTrails` 0x1640370 (+0x848): Delay <= 0: lifetime 0.001, EndTrails, deactivate, forget both
//!   components; else TrailTimeBeforeStop = min(the running one, Delay). `AMordhauWeapon::Tick` 0x1640db0 counts it
//!   down and stops at 0. Callers: UAttackMotion::OnLeave (0.15 / 0.25), parries / interrupts 0.3, 0.
//! - Weapon CDO defaults (ctor 0x1610e30 0x1611407..): TrailLifeTimeFactor 0.4, TrailExtraTime 0.2,
//!   SwingTrailFadeOutDuration 0.4, BloodTrailMaxDuration 0.25 (BP_MordhauWeapon 0.35).
//!
//! The emitter (P_WeaponTrailDistort / P_BloodWeaponTrailBlood: one AnimTrail emitter, Lifetime = the
//! "TrailLifeTime" parameter, ColorOverLife constant colour with AlphaOverLife 1 -> 0, material M_DistortTrail /
//! M_BloodTrail run as their translated FParticleBeamTrailVertexFactory shaders): while trails are active one
//! sample per update holds the two socket positions; consecutive samples form the ribbon's quads, V across (0 at
//! the first socket, 1 at the second; width mode FromCentre, width 1 = the sockets themselves), U along (0 at the
//! newest sample, 1 at the oldest). UNCONFIRMED: FParticleAnimTrailEmitterInstance's tessellation / interpolation
//! between updates and its exact U parametrisation were not read; one sample per frame.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::collections::HashMap;

/// AMordhauWeapon's trail properties (ctor 0x1610e30 defaults, CDO overrides)
#[derive(Clone, Debug)]
pub struct TrailConfig {
    pub swing_particles: String,
    pub blood_particles: String,
    pub lifetime_factor: f32,
    pub extra_time: f32,
    pub fade_out: f32,
    pub blood_max_duration: f32,
}

impl Default for TrailConfig {
    fn default() -> Self {
        TrailConfig { swing_particles: String::new(), blood_particles: String::new(), lifetime_factor: 0.4, extra_time: 0.2, fade_out: 0.4, blood_max_duration: 0.25 }
    }
}

impl TrailConfig {
    /// a weapon Blueprint's CDO chain over the native defaults
    pub fn read(rd: &mh_pak::Reader, weapon_bp: &str) -> TrailConfig {
        let d = rd.defaults(weapon_bp);
        let p = |k: &str| mh_assets::material::strip_index(&mh_assets::material::ue_pkg_path(d.get(k).unwrap_or(&serde_json::Value::Null))).to_string();
        let f = |k: &str, x: f32| d.get(k).and_then(|v| v.as_f64()).map_or(x, |v| v as f32);
        let def = TrailConfig::default();
        TrailConfig {
            swing_particles: p("SwingTrailParticles"),
            blood_particles: p("BloodTrailParticles"),
            lifetime_factor: f("TrailLifeTimeFactor", def.lifetime_factor),
            extra_time: f("TrailExtraTime", def.extra_time),
            fade_out: f("SwingTrailFadeOutDuration", def.fade_out),
            blood_max_duration: f("BloodTrailMaxDuration", def.blood_max_duration),
        }
    }
}

/// what the host reports for a weapon (ids are the host's, e.g. the fighter index)
#[derive(Message, Clone, Debug)]
pub enum TrailEvent {
    /// StartTrail(bIsBloodTrail, InDuration)
    Start { weapon: u64, blood: bool, duration: f32, config: TrailConfig },
    /// StopTrails(Delay)
    Stop { weapon: u64, delay: f32 },
    /// this frame's trail sockets (TraceStart / TraceEnd, or the Second* pair in alternate mode), Bevy world m
    Sockets { weapon: u64, a: Vec3, b: Vec3 },
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    a: Vec3,
    b: Vec3,
    age: f32,
    life: f32,
}

/// one anim-trail emitter instance
struct Ribbon {
    samples: Vec<Sample>,
    active: bool,
    lifetime_param: f32,
    material: Option<Handle<crate::ue_material::UeParticleMaterial>>,
    color: [f32; 3],
    /// AlphaOverLife table (linear over the particle's relative time)
    alpha: Vec<f32>,
    entity: Option<Entity>,
}

impl Ribbon {
    fn alpha_at(&self, t: f32) -> f32 {
        let n = self.alpha.len();
        if n == 0 {
            return 1.0;
        }
        if n == 1 {
            return self.alpha[0];
        }
        let x = t.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (x.floor() as usize).min(n - 2);
        self.alpha[i] + (self.alpha[i + 1] - self.alpha[i]) * (x - i as f32)
    }
}

#[derive(Default)]
struct WeaponTrail {
    cfg: TrailConfig,
    swing: Option<Ribbon>,
    blood: Option<Ribbon>,
    swing_end: f32,
    blood_end: f32,
    original_life: f32,
    time_before_stop: f32,
    /// ribbons stopped by StopTrails, left to fade
    dying: Vec<Ribbon>,
}

#[derive(Default)]
pub struct Trails {
    weapons: HashMap<u64, WeaponTrail>,
}

/// smoothstep of UpdateParticleTrails' fades: 0 before end - fade, 1 at / after end
fn end_fade(now: f32, end: f32, fade: f32) -> f32 {
    if end <= now {
        return 1.0;
    }
    let start = end - fade;
    if now <= start {
        return 0.0;
    }
    let x = ((now - start) / fade).clamp(0.0, 1.0);
    (3.0 - 2.0 * x) * x * x
}

fn stop_fade(tbs: f32) -> f32 {
    if tbs <= 0.0 {
        return 1.0;
    }
    let t = tbs * 5.0;
    if t < 1.0 { (3.0 - 2.0 * t) * t * t } else { 1.0 }
}

pub(crate) fn ribbon_def(rd: &mh_pak::Reader, ps: &str) -> Option<(String, [f32; 3], Vec<f32>)> {
    let sys = mh_assets::particles::read(rd, ps)?;
    let lod = sys.emitters.first()?.lods.first()?;
    let mut r = || 0.5f32;
    let col = lod.module("ParticleModuleColorOverLife");
    let color = col.and_then(|m| m.dist("ColorOverLife")).map_or([1.0; 3], |d| d.value3(0.0, &mut r));
    let alpha = col
        .and_then(|m| m.props.pointer("/AlphaOverLife/Table/Values"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect())
        .unwrap_or_else(|| vec![1.0]);
    Some((lod.material(), color, alpha))
}

impl Trails {
    pub(crate) fn event(&mut self, ev: &TrailEvent, now: f32, mut make: impl FnMut(&str) -> Option<Ribbon0>) {
        match ev {
            TrailEvent::Start { weapon, blood, duration, config } => {
                let w = self.weapons.entry(*weapon).or_default();
                w.cfg = config.clone();
                let d = duration.max(0.0);
                let ps = if *blood { &config.blood_particles } else { &config.swing_particles };
                let Some(r0) = make(ps) else { return };
                let life = if *blood { w.original_life } else { d * config.lifetime_factor };
                let rb = Ribbon { samples: vec![], active: true, lifetime_param: life, material: r0.material, color: r0.color, alpha: r0.alpha, entity: None };
                if *blood {
                    if let Some(old) = w.blood.replace(rb) {
                        w.dying.push(old);
                    }
                    w.blood_end = now + d;
                } else {
                    if let Some(old) = w.swing.replace(rb) {
                        w.dying.push(old);
                    }
                    w.original_life = life;
                    w.swing_end = now + d + config.extra_time;
                }
            }
            TrailEvent::Stop { weapon, delay } => {
                if let Some(w) = self.weapons.get_mut(weapon) {
                    stop(w, *delay);
                }
            }
            TrailEvent::Sockets { weapon, a, b } => {
                if let Some(w) = self.weapons.get_mut(weapon) {
                    for r in [w.swing.as_mut(), w.blood.as_mut()].into_iter().flatten() {
                        if r.active {
                            r.samples.insert(0, Sample { a: *a, b: *b, age: 0.0, life: r.lifetime_param.max(0.001) });
                        }
                    }
                }
            }
        }
    }

    /// AMordhauWeapon::Tick + UpdateParticleTrails, then the samples age
    pub(crate) fn tick(&mut self, now: f32, dt: f32) {
        for w in self.weapons.values_mut() {
            if w.time_before_stop > 0.0 {
                w.time_before_stop -= dt;
                if w.time_before_stop <= 0.0 {
                    stop(w, 0.0);
                }
            }
            let fs = end_fade(now, w.swing_end, w.cfg.fade_out);
            let fb = end_fade(now, w.blood_end, w.cfg.fade_out);
            let sf = stop_fade(w.time_before_stop);
            if let Some(r) = w.swing.as_mut() {
                r.lifetime_param = sf.min(1.0 - fs) * w.original_life + 0.001;
            }
            if let Some(r) = w.blood.as_mut() {
                r.lifetime_param = sf.min(1.0 - fb) * w.original_life + 0.001;
            }
            for r in w.swing.iter_mut().chain(w.blood.iter_mut()).chain(w.dying.iter_mut()) {
                for s in &mut r.samples {
                    s.age += dt;
                }
                r.samples.retain(|s| s.age < s.life);
            }
        }
    }
}

/// the constant part of a ribbon (its system's material and colour)
pub(crate) struct Ribbon0 {
    pub material: Option<Handle<crate::ue_material::UeParticleMaterial>>,
    pub color: [f32; 3],
    pub alpha: Vec<f32>,
}

/// StopTrails 0x1640370
fn stop(w: &mut WeaponTrail, delay: f32) {
    if delay <= 0.0 {
        w.time_before_stop = -1.0;
        for r in [w.swing.take(), w.blood.take()].into_iter().flatten() {
            let mut r = r;
            r.active = false;
            w.dying.push(r);
        }
        return;
    }
    w.time_before_stop = if w.time_before_stop > 0.0 { w.time_before_stop.min(delay) } else { delay };
}

/// the ribbon meshes: (entity, mesh) per ribbon; hidden / despawned when empty
pub(crate) fn build(trails: &mut Trails, commands: &mut Commands, meshes: &mut Assets<Mesh>, cam_ue: Vec3) {
    for w in trails.weapons.values_mut() {
        for r in w.swing.iter_mut().chain(w.blood.iter_mut()).chain(w.dying.iter_mut()) {
            let n = r.samples.len();
            if std::env::var("MH_FX_TRAIL_DEBUG").is_ok() {
                eprintln!("trail ribbon: samples {} material {} life {:.3} color {:?}", r.samples.len(), r.material.is_some(), r.lifetime_param, r.color);
            }
            let Some(mat) = r.material.clone() else { continue };
            if n < 2 {
                if let Some(e) = r.entity {
                    commands.entity(e).insert(Visibility::Hidden);
                }
                continue;
            }
            let mut pos = vec![];
            let mut uv = vec![];
            let mut col = vec![];
            let mut zero = vec![];
            let mut ppos = vec![];
            let mut idx = vec![];
            for (i, s) in r.samples.iter().enumerate() {
                let u = i as f32 / (n - 1) as f32;
                let a = r.alpha_at(s.age / s.life);
                let k = [r.color[0], r.color[1], r.color[2], a];
                for (p, v) in [(s.a, 0.0f32), (s.b, 1.0)] {
                    pos.push(p.to_array());
                    uv.push([u, v]);
                    col.push(k);
                    zero.push([0.0f32; 4]);
                    let ue = Vec3::new(p.x, p.z, p.y) * 100.0 - cam_ue;
                    ppos.push([ue.x, ue.y, ue.z, 0.0]);
                }
                if i + 1 < n {
                    let b = (2 * i) as u32;
                    idx.extend([b, b + 1, b + 3, b, b + 3, b + 2]);
                }
            }
            let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
            mesh.insert_attribute(crate::ue_material::ATTR_SUBUV, zero.clone());
            mesh.insert_attribute(crate::ue_material::ATTR_TW0, zero.clone());
            mesh.insert_attribute(crate::ue_material::ATTR_TW2, zero.clone());
            mesh.insert_attribute(crate::ue_material::ATTR_DYNP, vec![[1.0f32; 4]; zero.len()]);
            mesh.insert_attribute(crate::ue_material::ATTR_PPOS, ppos);
            mesh.insert_attribute(crate::ue_material::ATTR_PVEL, zero);
            mesh.insert_indices(Indices::U32(idx));
            let h = meshes.add(mesh);
            match r.entity {
                Some(e) => {
                    commands.entity(e).insert((Mesh3d(h), Visibility::Inherited));
                }
                None => {
                    r.entity = Some(commands.spawn((Mesh3d(h), MeshMaterial3d(mat), Transform::IDENTITY, Visibility::Inherited, bevy::light::NotShadowCaster)).id());
                }
            }
        }
        // finished dying ribbons go away
        w.dying.retain(|r| {
            let gone = r.samples.is_empty();
            if gone {
                if let Some(e) = r.entity {
                    commands.entity(e).despawn();
                }
            }
            !gone
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a swing of release 0.5 s: lifetime 0.2 s (0.4 x), samples die after it; a 0.15 s delayed stop ends the
    /// trail and its samples fade out
    #[test]
    fn swing_trail_lifecycle() {
        let mut t = Trails::default();
        let cfg = TrailConfig::default();
        let mk = |_: &str| Some(Ribbon0 { material: None, color: [0.61; 3], alpha: vec![1.0, 0.0] });
        t.event(&TrailEvent::Start { weapon: 1, blood: false, duration: 0.5, config: cfg.clone() }, 0.0, mk);
        let dt = 1.0 / 60.0;
        let mut now = 0.0;
        for i in 0..30 {
            t.event(&TrailEvent::Sockets { weapon: 1, a: Vec3::X * i as f32, b: Vec3::Y }, now, mk);
            t.tick(now, dt);
            now += dt;
        }
        let w = &t.weapons[&1];
        let r = w.swing.as_ref().unwrap();
        // 0.2 s lifetime at 60 Hz: 12 samples
        assert!((11..=13).contains(&r.samples.len()), "{}", r.samples.len());
        t.event(&TrailEvent::Stop { weapon: 1, delay: 0.15 }, now, mk);
        for _ in 0..60 {
            t.tick(now, dt);
            now += dt;
        }
        let w = &t.weapons[&1];
        assert!(w.swing.is_none() && w.dying.iter().all(|r| r.samples.is_empty()));
    }
}

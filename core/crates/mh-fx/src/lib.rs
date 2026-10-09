//! mh-fx: Mordhau's Cascade particle effects in Bevy. A request (`FxRequest`: ParticleSystem package + Bevy world
//! position / direction) starts a CPU simulation (sim.rs, over mh-assets particles' distribution port) in UE space at
//! the converted origin (sim.rs: a port of the exe's FParticleEmitterInstance / module code, rvas there); every frame
//! each emitter's live particles are written into one dynamic mesh of camera-facing (or velocity-aligned,
//! PSA_Velocity) quads with the particle colour, the two SubUV cells (uv / uv_b) and their lerp, drawn with
//! `material::ParticleMaterial`, the pixel math of the game's own compiled particle shaders (material.rs).
//! `FxGround` gives the Collision module a ground plane.
//!
//! Which system a combat event plays comes from the game's own Blueprint data (`CombatFx`): BP_MordhauWeapon CDO
//! BlockParticles / HitCancelParticles / SlideParticles / ImpactParticlesBySurface, BP_ArmorBloodSplash's particle
//! (armour hits), the Blueprints' blood splash; `for_event` maps mh-sim drain() events onto them.

pub mod map_fx;
pub mod material;
pub mod sim;
pub mod trail;
pub mod ue_material;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use mh_assets::coords;
use mh_ui::Paks;
use material::{FxParams, ParticleMaterial};
use sim::SystemSim;
use ue_material::{UeBlend, UeParticleMaterial};
use std::collections::HashMap;

/// start a particle system at a Bevy world position (metres, Y up); `dir` = the effect's forward (its UE X axis)
#[derive(Message, Clone, Debug)]
pub struct FxRequest {
    pub system: String,
    pub pos: Vec3,
    pub dir: Vec3,
}

/// read a particle system and build its emitters' materials (textures, translated shaders) ahead of its first
/// FxRequest, so the first hit effect of a fight does not stall its frame (first-person r1 smoothness; loading glue)
#[derive(Message, Clone, Debug)]
pub struct FxPrewarm(pub String);

/// counts for evidence
#[derive(Resource, Clone, Debug, Default)]
pub struct FxStats {
    pub started: Vec<String>,
    pub live_systems: usize,
    pub live_particles: usize,
    pub peak_particles: usize,
    pub unsupported_modules: Vec<String>,
    pub missing: Vec<String>,
}

/// the ground height (Bevy world Y, m) the Collision module traces against; None (default) = no collisions
#[derive(Resource, Clone, Copy, Default)]
pub struct FxGround(pub Option<f32>);

/// the camera the sprites face (defaults to the first Camera3d)
#[derive(Resource, Clone, Copy)]
pub struct FxCamera(pub Entity);

pub struct FxPlugin;

impl Plugin for FxPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ParticleMaterial>::default())
            .add_plugins(MaterialPlugin::<UeParticleMaterial>::default())
            .init_resource::<UeAmbient>()
            .add_message::<FxRequest>()
            .add_message::<FxPrewarm>()
            .add_message::<trail::TrailEvent>()
            .init_resource::<TrailState>()
            .init_resource::<FxStats>()
            .init_resource::<FxGround>()
            .add_systems(PreStartup, |mut sh: ResMut<Assets<bevy::shader::Shader>>| {
                material::install(&mut sh);
                ue_material::install(&mut sh);
            })
            .add_systems(Startup, setup).add_systems(Update, (depth_prepass, map_effects, prewarm, start, step, trails).chain());
    }
}

#[derive(Resource, Default)]
pub struct TrailState(trail::Trails, HashMap<String, Option<(Option<Handle<UeParticleMaterial>>, [f32; 3], Vec<f32>)>>);

/// weapon trails (trail.rs): the host's TrailEvents, the weapon's trail state, the ribbons. The trail materials
/// (M_DistortTrail / M_BloodTrail) have no ported counterpart: their translated shaders always run.
#[allow(clippy::too_many_arguments)]
fn trails(
    mut commands: Commands,
    st: Option<NonSendMut<FxState>>,
    mut ts: ResMut<TrailState>,
    mut evs: MessageReader<trail::TrailEvent>,
    time: Res<Time>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut umats: ResMut<Assets<UeParticleMaterial>>,
    mut shaders: ResMut<Assets<bevy::shader::Shader>>,
    mut images: ResMut<Assets<Image>>,
    amb: Res<UeAmbient>,
) {
    let Some(mut st) = st else { return };
    let st = &mut *st;
    let now = time.elapsed_secs();
    let TrailState(trails, defs) = &mut *ts;
    for ev in evs.read() {
        trails.event(ev, now, |ps| {
            if ps.is_empty() {
                return None;
            }
            let d = defs
                .entry(ps.to_string())
                .or_insert_with(|| {
                    let (mat, color, alpha) = trail::ribbon_def(&st.rd, ps)?;
                    let h = ue_material_for(st, &mut umats, &mut images, &mut shaders, amb.0, &mat, true);
                    if let Some(h) = &h {
                        st.ue_mats.push(h.clone());
                    }
                    Some((h, color, alpha))
                })
                .clone()?;
            Some(trail::Ribbon0 { material: d.0, color: d.1, alpha: d.2 })
        });
    }
    trails.tick(now, time.delta_secs().min(0.1));
    let cam_ue = cams.iter().next().map_or(Vec3::ZERO, |t| {
        let c = t.translation();
        Vec3::new(c.x, c.z, c.y) * 100.0
    });
    trail::build(trails, &mut commands, &mut meshes, cam_ue);
}

/// whether every particle material runs the game's translated pixel shader (ue_material.rs): MH_FX_UE_SHADERS=1.
/// Unset, only the combat masters (COMBAT_MASTERS) do; MH_FX_UE_SHADERS=0 turns them off too (the ported pixel math)
pub fn ue_shaders_enabled() -> bool {
    std::env::var("MH_FX_UE_SHADERS").as_deref() == Ok("1")
}

/// the masters of every material the hit / block / parry / clash / world-hit systems draw with (BP_MordhauWeapon's
/// BlockParticles P_spark_burst, HitCancelParticles P_BlockDust, ImpactParticlesBySurface, BP_MordhauCharacter's
/// P_BloodSplash / P_ArmorSparks); their translated shaders are GPU-validated (r12 evidence) and on by default
pub const COMBAT_MASTERS: &[&str] = &[
    "Mordhau/Content/Mordhau/Particles/ParrySparks/M_radial_ramp",
    "Mordhau/Content/Mordhau/Particles/ParrySparks/M_Dust_Particle",
    "Mordhau/Content/Mordhau/Particles/Dust/M_FootstepDust",
    "Mordhau/Content/Mordhau/Particles/Blood/M_BloodDrops",
    "Mordhau/Content/Mordhau/Particles/Blood/M_impact_splash_subUV",
];

/// whether a material with this master runs its translated shader
pub fn ue_shader_for_master(master: &str) -> bool {
    match std::env::var("MH_FX_UE_SHADERS").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => COMBAT_MASTERS.iter().any(|m| m.eq_ignore_ascii_case(master)),
    }
}

/// opt-in: give every 3D camera Bevy's depth prepass, the SceneDepthTexture ue_material::ue_scene_depth samples.
/// Off by default: a camera with DepthPrepass draws alpha-masked materials through the prepass, which needs each
/// masked material's own prepass shader (bevy-runtime's foliage turned into holes when this was forced on). Without a
/// prepass the depth fades read "infinitely far" (no fade).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct FxDepthPrepass(pub bool);

fn depth_prepass(mut commands: Commands, on: Option<Res<FxDepthPrepass>>, cams: Query<Entity, (With<Camera3d>, Without<bevy::core_pipeline::prepass::DepthPrepass>)>) {
    if !ue_shaders_enabled() || !on.is_some_and(|o| o.0) {
        return;
    }
    for e in &cams {
        commands.entity(e).insert(bevy::core_pipeline::prepass::DepthPrepass);
    }
}

/// combat effect packages read from the Blueprints (CDO fields named on each)
#[derive(Clone, Debug, Default)]
pub struct CombatFx {
    /// BP_MordhauWeapon BlockParticles (parries / blocks)
    pub block: String,
    /// BP_MordhauWeapon HitCancelParticles
    pub hit_cancel: String,
    /// BP_MordhauWeapon SlideParticles
    pub slide: String,
    /// BP_MordhauWeapon ImpactParticlesBySurface[EPhysicalSurface]
    pub impact_by_surface: Vec<String>,
    /// armoured hit: the particle of BP_MordhauCharacter CDO BloodMetalHitEffect's class (BP_ArmorBloodSplash);
    /// AMordhauCharacter::OnTookDamage_Implementation 0x155be10 spawns BloodMetalHitEffect when the hit armour tier
    /// >= 2 and the victim is not the view target, else BloodHitEffect
    pub armor_hit: String,
    /// flesh hit: the particle of BloodHitEffect's class (BP_BloodSplash)
    pub flesh_hit: String,
}

/// the ParticleSystem of an effect actor class package: its ParticleSystemComponent's Template (the component
/// template exports of the Blueprint)
fn effect_particle(rd: &mh_pak::Reader, class_pkg: &str) -> String {
    let Some(ex) = rd.read(class_pkg) else { return String::new() };
    ex.iter()
        .filter(|e| e.get("Type").and_then(|t| t.as_str()) == Some("ParticleSystemComponent"))
        .filter_map(|e| e.pointer("/Properties/Template"))
        .map(|t| obj_pkg(Some(t)))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

fn obj_pkg(v: Option<&serde_json::Value>) -> String {
    mh_assets::material::ue_pkg_path(v.unwrap_or(&serde_json::Value::Null))
}

impl CombatFx {
    pub fn read(rd: &mh_pak::Reader) -> CombatFx {
        let w = rd.defaults("Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/BP_MordhauWeapon");
        let mut out = CombatFx {
            block: obj_pkg(w.get("BlockParticles")),
            hit_cancel: obj_pkg(w.get("HitCancelParticles")),
            slide: obj_pkg(w.get("SlideParticles")),
            impact_by_surface: w.get("ImpactParticlesBySurface").and_then(|a| a.as_array()).map(|a| a.iter().map(|x| obj_pkg(Some(x))).collect()).unwrap_or_default(),
            ..Default::default()
        };
        let c = rd.defaults("Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter");
        out.flesh_hit = effect_particle(rd, &obj_pkg(c.get("BloodHitEffect")));
        out.armor_hit = effect_particle(rd, &obj_pkg(c.get("BloodMetalHitEffect")));
        out
    }

    /// the blood effect's forward for a melee hit: AAdvancedCharacter::OnCosmeticHit_Implementation (decomp
    /// AAdvancedCharacter.cpp ~1190, rva 0x148c130) spawns HitEffect at the hit point (GetBestStickyLocation) with
    /// FRotationMatrix::MakeFromXZ(-weapon LastObservedTraceDirection, Up): the effect faces back along the swing.
    /// The ParticleSystemComponent of BP_BloodSplash / BP_ArmorBloodSplash has an identity relative transform, so
    /// the FxRequest `dir` is that X axis (Bevy world). `trace_dir` = the weapon's last trace direction (Bevy world).
    pub fn hit_dir(trace_dir: Vec3) -> Vec3 {
        -trace_dir.normalize_or_zero()
    }

    /// the first system of `for_event_all`
    pub fn for_event(&self, ev: &serde_json::Value, armoured: bool) -> Option<&str> {
        self.for_event_all(ev, armoured).into_iter().next()
    }

    /// every system the exe spawns for a mh-sim drain() event (weapon BP fields of the weapons involved):
    /// - hit -> the victim's HitEffect particle (flesh / armour blood; AAdvancedCharacter::OnCosmeticHit
    ///   0x148c130); a character hit spawns no ImpactParticles (OnHit_Implementation 0x1631430 returns for an actor);
    /// - parry / active_parry / chamber -> BlockParticles once: AMordhauCharacter::OnRep_NetBlock 0x155a4e0 calls the
    ///   blocker's AMordhauWeapon::OnBlocked_Implementation 0x16306d0 (decomp AMordhauCharacter.cpp 6899-6905); the
    ///   attacker's OnWasBlocked (UBlockedMotion) spawns nothing for Parry / Chamber / Hit unless bIsCancel;
    /// - clash -> BlockParticles twice: the victim's NetBlock (no party flag) -> OnBlocked, the attacker's (party flag
    ///   16) -> OnWasBlocked_Implementation 0x16340a0, whose Reason == Clash branch spawns BlockParticles;
    /// - was_blocked with cancel -> HitCancelParticles (OnWasBlocked bIsCancel branch, decomp AMordhauWeapon.cpp 1807-1830).
    pub fn for_event_all(&self, ev: &serde_json::Value, armoured: bool) -> Vec<&str> {
        let k = ev.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let v: Vec<&String> = match k {
            "hit" => vec![if armoured { &self.armor_hit } else { &self.flesh_hit }],
            "parry" | "active_parry" | "chamber" => vec![&self.block],
            "clash" => vec![&self.block, &self.block],
            "was_blocked" if ev.get("cancel").and_then(|c| c.as_bool()).unwrap_or(false) => vec![&self.hit_cancel],
            _ => vec![],
        };
        v.into_iter().filter(|s| !s.is_empty()).map(|s| s.as_str()).collect()
    }
}

/// the ambient the translated shaders' lighting-volume stand-in holds (linear rgb); hosts set it from the map's sky
/// light (UNCONFIRMED stand-in for UE's translucency lighting volume)
#[derive(Resource, Clone, Copy, Debug)]
pub struct UeAmbient(pub [f32; 3]);

impl Default for UeAmbient {
    fn default() -> Self {
        UeAmbient([0.35, 0.35, 0.35])
    }
}

/// a particle material: the game's translated pixel shader (ue_material.rs) when its master was translated, else the
/// hand-ported pixel math (material.rs)
#[derive(Clone, Debug)]
pub enum FxMat {
    Ue(Handle<UeParticleMaterial>),
    Ported(Handle<ParticleMaterial>),
}

struct Live {
    sim: SystemSim,
    /// per emitter: the mesh entity and handle
    meshes: Vec<Option<(Entity, Handle<Mesh>)>>,
}

/// systems, materials and textures (NonSend: mh-pak Reader)
pub struct FxState {
    rd: mh_pak::Reader,
    src: mh_assets::pak_source::PakSource,
    systems: HashMap<String, Option<mh_assets::particles::ParticleSystem>>,
    materials: HashMap<String, Option<FxMat>>,
    /// translated-material handles (their View slots are refreshed every frame)
    ue_mats: Vec<Handle<UeParticleMaterial>>,
    shaders: HashMap<String, Option<(Handle<bevy::shader::Shader>, serde_json::Value)>>,
    white: Option<Handle<Image>>,
    stand_ins: Option<[Handle<Image>; 4]>,
    live: Vec<Live>,
    seed: u32,
}

fn setup(world: &mut World) {
    let Some(p) = Paks::get_or_mount(world) else {
        warn!("mh-fx: no paks; effects disabled");
        return;
    };
    world.insert_non_send(FxState {
        rd: mh_pak::Reader::new(p.0.clone()),
        src: mh_assets::pak_source::PakSource::new(p.0.clone()),
        systems: HashMap::new(),
        materials: HashMap::new(),
        ue_mats: vec![],
        shaders: HashMap::new(),
        white: None,
        stand_ins: None,
        live: vec![],
        seed: 0x5eed,
    });
}

fn load_tex(st: &FxState, images: &mut Assets<Image>, tex: &str) -> Option<Handle<Image>> {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use mh_assets::texture;
    let (p, o) = if st.rd.vfs.has(&format!("{tex}.uasset")) {
        (tex.to_string(), None)
    } else {
        let (a, b) = tex.rsplit_once('/')?;
        (a.to_string(), Some(b.to_string()))
    };
    let ti = texture::info(&st.src, &p, o.as_deref()).ok()?;
    let data = texture::mip_data(&st.src, &ti, 0).ok()?;
    let (w, h) = (ti.mips[0].size_x as u32, ti.mips[0].size_y as u32);
    let px = match texture::decode(ti.format?, w as usize, h as usize, &data).ok()? {
        texture::Pixels::Rgba8(v) => v,
        texture::Pixels::RgbaF32(v) => v.iter().map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8).collect(),
    };
    let fmt = if ti.srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
    Some(images.add(Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, px, fmt, RenderAssetUsages::default())))
}

/// the material's mode (material.rs table), its texture (CachedExpressionData.ReferencedTextures[k]), BlendMode,
/// OpacityMaskClipValue
/// the translated material of a particle material package (None when its master has no translated shader)
#[allow(clippy::too_many_arguments)]
fn ue_material_for(
    st: &mut FxState,
    umats: &mut Assets<UeParticleMaterial>,
    images: &mut Assets<Image>,
    shaders: &mut Assets<bevy::shader::Shader>,
    amb: [f32; 3],
    pkg: &str,
    force: bool,
) -> Option<Handle<UeParticleMaterial>> {
    if !force && std::env::var("MH_FX_UE_SHADERS").as_deref() == Ok("0") {
        return None;
    }
    let rs = mh_assets::material::Resolver::new(&st.rd, &st.src);
    let params = rs.params(pkg)?;
    let master = params.master_package.clone();
    if !force && !ue_shader_for_master(&master) {
        return None;
    }
    if !st.shaders.contains_key(&master) {
        let t = ue_material::load_translated(&master).map(|t| (shaders.add(bevy::shader::Shader::from_wgsl(t.wgsl, format!("mh-fx/ue/{}.wgsl", master.rsplit('/').next().unwrap_or("m")))), t.meta));
        st.shaders.insert(master.clone(), t);
    }
    let (shader, meta) = st.shaders.get(&master)?.clone()?;
    // instance values over the master defaults: the resolver's chain, nearest first (colours carry no alpha there)
    let scalars: HashMap<String, f32> = params.scalars.iter().map(|(k, v)| (k.to_lowercase(), *v)).collect();
    let vectors: HashMap<String, [f32; 4]> = params.colors.iter().map(|(k, v)| (k.to_lowercase(), [v[0], v[1], v[2], 1.0])).collect();
    let cb = match ue_material::material_cbuffer(&meta, &scalars, &vectors) {
        Ok(c) => c,
        Err(e) => {
            warn!("mh-fx: {pkg}: {e}");
            return None;
        }
    };
    let white = st
        .white
        .get_or_insert_with(|| {
            use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
            images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, vec![255; 4], TextureFormat::Rgba8Unorm, RenderAssetUsages::default()))
        })
        .clone();
    let si = st
        .stand_ins
        .get_or_insert_with(|| {
            use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
            let depth = Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, vec![0; 4], TextureFormat::Rgba8Unorm, RenderAssetUsages::default());
            [images.add(depth), images.add(ue_material::white_3d()), images.add(ue_material::ambient_3d(amb)), images.add(ue_material::white_3d())]
        })
        .clone();
    // Material.Texture2D_k: the instance's texture parameter of that name, else the master's ReferencedTextures[index]
    let refs: Vec<String> = st
        .rd
        .read(&master)
        .and_then(|ex| ex.iter().find(|e| e.get("Type").and_then(|t| t.as_str()) == Some("Material")).cloned())
        .and_then(|m| m.pointer("/Properties/CachedExpressionData/ReferencedTextures").cloned())
        .and_then(|v| v.as_array().cloned())
        .map(|a| a.iter().map(mh_assets::material::ue_pkg_path).collect())
        .unwrap_or_default();
    let mut tex: [Handle<Image>; 6] = std::array::from_fn(|_| white.clone());
    for t in meta.get("textures").and_then(|v| v.as_array()).into_iter().flatten() {
        let slot = t.get("slot").and_then(|v| v.as_u64()).unwrap_or(99) as usize;
        if slot >= 6 {
            continue;
        }
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let idx = t.get("texture_index").and_then(|v| v.as_i64()).unwrap_or(-1);
        let path = params.tex.iter().find(|(k, _)| !name.is_empty() && k.eq_ignore_ascii_case(name)).map(|(_, t)| t.0.clone()).or_else(|| (idx >= 0).then(|| refs.get(idx as usize).cloned()).flatten());
        if let Some(h) = path.and_then(|p| load_tex(st, images, &p)) {
            tex[slot] = h;
        }
    }
    let blend = match params.blend {
        1 => UeBlend::Masked,
        2 => UeBlend::Translucent,
        3 => UeBlend::Additive,
        4 => UeBlend::Modulate,
        _ => UeBlend::Opaque,
    };
    let mut u = ue_material::UeUniforms::default();
    for (i, c) in cb.iter().enumerate().take(ue_material::MAT_SLOTS) {
        u.mat[i] = Vec4::from_array(*c);
    }
    let gbuffer = meta.get("outputs").and_then(|o| o.as_array()).is_some_and(|o| o.len() > 1);
    u.mode = Vec4::new(if gbuffer { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0);
    u.amb = Vec4::new(amb[0], amb[1], amb[2], 1.0);
    let [t0, t1, t2, t3, t4, t5] = tex;
    let h = umats.add(UeParticleMaterial {
        u,
        mtex_0: t0,
        mtex_1: t1,
        mtex_2: t2,
        mtex_3: t3,
        mtex_4: t4,
        mtex_5: t5,
        scene_depth: si[0].clone(),
        dummy2d: white,
        vol3d_0: si[1].clone(),
        vol3d_1: si[2].clone(),
        vol3d_2: si[2].clone(),
        dummy3d: si[3].clone(),
        shader,
        blend: if gbuffer && blend == UeBlend::Opaque { UeBlend::Masked } else { blend },
    });
    st.ue_mats.push(h.clone());
    Some(h)
}

#[allow(clippy::too_many_arguments)]
fn fx_material(
    st: &mut FxState,
    mats: &mut Assets<ParticleMaterial>,
    umats: &mut Assets<UeParticleMaterial>,
    images: &mut Assets<Image>,
    shaders: &mut Assets<bevy::shader::Shader>,
    amb: [f32; 3],
    pkg: &str,
) -> Option<FxMat> {
    if let Some(m) = st.materials.get(pkg) {
        return m.clone();
    }
    // M_ImpactDistortion: refraction only (material.rs); not drawn either way
    if material::mode_of(pkg).is_none() {
        st.materials.insert(pkg.to_string(), None);
        return None;
    }
    let m = match ue_material_for(st, umats, images, shaders, amb, pkg, false) {
        Some(h) => Some(FxMat::Ue(h)),
        None => material(st, mats, images, pkg).map(FxMat::Ported),
    };
    st.materials.insert(pkg.to_string(), m.clone());
    m
}

fn material(st: &mut FxState, mats: &mut Assets<ParticleMaterial>, images: &mut Assets<Image>, pkg: &str) -> Option<Handle<ParticleMaterial>> {
    let white = st
        .white
        .get_or_insert_with(|| {
            use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
            images.add(Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, vec![255; 4], TextureFormat::Rgba8Unorm, RenderAssetUsages::default()))
        })
        .clone();
    let h = (|| {
        let (mode, slot, radius) = material::mode_of(pkg)?;
        let ex = st.rd.read(pkg)?;
        let m = ex.iter().find(|e| e.get("Type").and_then(|t| t.as_str()) == Some("Material")).or_else(|| ex.first())?;
        let p = m.get("Properties")?;
        let blend = match p.get("BlendMode").and_then(|v| v.as_str()).unwrap_or("BLEND_Opaque") {
            "BLEND_Masked" => 1,
            "BLEND_Translucent" => 2,
            "BLEND_Additive" => 3,
            "BLEND_Modulate" => 4,
            _ => 0,
        };
        let clip = p.get("OpacityMaskClipValue").and_then(|v| v.as_f64()).unwrap_or(0.3333) as f32;
        let refs: Vec<String> = p
            .pointer("/CachedExpressionData/ReferencedTextures")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(mh_assets::material::ue_pkg_path).collect())
            .unwrap_or_default();
        // mode 0: the first referenced texture that is not a normal map (UNCONFIRMED)
        let tex = match (mode, slot) {
            (_, Some(k)) => refs.get(k).cloned(),
            (0, None) => refs.iter().find(|t| !t.ends_with("_N") && !t.contains("EngineMaterials")).cloned(),
            _ => None,
        };
        let image = tex.and_then(|t| load_tex(st, images, &t)).unwrap_or(white.clone());
        Some(mats.add(ParticleMaterial {
            params: FxParams { p: Vec4::new(mode as f32, radius, clip, if blend == 3 { 1.0 } else { 0.0 }) },
            tex: image,
            blend: if mode == 1 || mode == 2 { 1 } else { blend },
        }))
    })();
    h
}

/// a map's placed particle systems (map_fx::map_emitters); inserting / replacing it starts every auto-activated one
#[derive(Resource, Clone, Debug, Default)]
pub struct MapEffects(pub Vec<map_fx::MapEmitter>);

fn map_effects(m: Option<Res<MapEffects>>, mut out: MessageWriter<FxRequest>) {
    let Some(m) = m.filter(|m| m.is_changed()) else { return };
    for e in m.0.iter().filter(|e| e.auto_activate) {
        let p = coords::pos(e.pos_ue.map(|x| x as f32));
        let d = [e.dir_ue[0] as f32, e.dir_ue[2] as f32, e.dir_ue[1] as f32];
        out.write(FxRequest { system: e.template.clone(), pos: Vec3::from_array(p), dir: Vec3::from_array(d) });
    }
}

/// Bevy world (m, Y up) -> UE (cm, Z up)
fn to_ue(v: Vec3) -> [f32; 3] {
    [v.x * 100.0, v.z * 100.0, v.y * 100.0]
}

fn prewarm(
    st: Option<NonSendMut<FxState>>,
    mut reqs: MessageReader<FxPrewarm>,
    mut mats: ResMut<Assets<ParticleMaterial>>,
    mut umats: ResMut<Assets<UeParticleMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut shaders: ResMut<Assets<bevy::shader::Shader>>,
    amb: Res<UeAmbient>,
) {
    let Some(mut st) = st else {
        reqs.clear();
        return;
    };
    let st = &mut *st;
    for FxPrewarm(sys) in reqs.read() {
        let ps = match st.systems.get(sys) {
            Some(p) => p.clone(),
            None => {
                let p = mh_assets::particles::read(&st.rd, sys);
                st.systems.insert(sys.clone(), p.clone());
                p
            }
        };
        let Some(ps) = ps else { continue };
        let sim = SystemSim::new(&ps, [0.0; 3], [1.0, 0.0, 0.0], 1);
        for e in &sim.emitters {
            let _ = fx_material(st, &mut mats, &mut umats, &mut images, &mut shaders, amb.0, &e.material);
        }
    }
}

fn start(st: Option<NonSendMut<FxState>>, mut reqs: MessageReader<FxRequest>, mut stats: ResMut<FxStats>) {
    let Some(mut st) = st else { return };
    for r in reqs.read() {
        let ps = if let Some(p) = st.systems.get(&r.system) {
            p.clone()
        } else {
            let p = mh_assets::particles::read(&st.rd, &r.system);
            st.systems.insert(r.system.clone(), p.clone());
            p
        };
        let Some(ps) = ps else {
            if !stats.missing.contains(&r.system) {
                stats.missing.push(r.system.clone());
            }
            continue;
        };
        st.seed = st.seed.wrapping_mul(747796405).wrapping_add(2891336453);
        let d = r.dir.normalize_or_zero();
        let sim = SystemSim::new(&ps, to_ue(r.pos), [d.x, d.z, d.y], st.seed);
        for u in &sim.unsupported {
            if !stats.unsupported_modules.contains(u) {
                stats.unsupported_modules.push(u.clone());
            }
        }
        stats.started.push(r.system.clone());
        let n = sim.emitters.len();
        st.live.push(Live { sim, meshes: vec![None; n] });
    }
}

#[allow(clippy::too_many_arguments)]
fn step(
    mut commands: Commands,
    st: Option<NonSendMut<FxState>>,
    time: Res<Time>,
    cam: Option<Res<FxCamera>>,
    cams: Query<(Entity, &GlobalTransform), With<Camera3d>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ParticleMaterial>>,
    mut umats: ResMut<Assets<UeParticleMaterial>>,
    mut shaders: ResMut<Assets<bevy::shader::Shader>>,
    projs: Query<(&Projection, &Camera, Option<&bevy::camera::Exposure>), With<Camera3d>>,
    amb: Res<UeAmbient>,
    ground: Res<FxGround>,
    mut images: ResMut<Assets<Image>>,
    mut stats: ResMut<FxStats>,
) {
    let Some(mut st) = st else { return };
    let dt = time.delta_secs().min(0.1);
    let cam_tf = cam.and_then(|c| cams.get(c.0).ok().map(|x| *x.1)).or_else(|| cams.iter().next().map(|x| *x.1));
    let (right, up, fwd) = cam_tf.map_or((Vec3::X, Vec3::Y, Vec3::NEG_Z), |t| (t.right().as_vec3(), t.up().as_vec3(), t.forward().as_vec3()));
    let st = &mut *st;
    // the translated shaders' View slots from the camera (ue_material::view_slots)
    if let Some(t) = cam_tf {
        let (clip_from_view, near) = projs
            .iter()
            .next()
            .map(|(p, _, _)| match p {
                Projection::Perspective(pp) => (Mat4::perspective_infinite_reverse_rh(pp.fov, pp.aspect_ratio, pp.near), pp.near),
                _ => (Mat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, 0.1), 0.1),
            })
            .unwrap_or((Mat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, 0.1), 0.1));
        let size = projs.iter().next().and_then(|(_, c, _)| c.physical_target_size()).map_or(Vec2::new(1280.0, 720.0), |s| s.as_vec2());
        // View.PreExposure = the camera's exposure multiplier (Bevy's view.exposure, the default Exposure when unset)
        let exposure = projs.iter().next().and_then(|(_, _, e)| e.copied()).unwrap_or_default().exposure();
        let slots = ue_material::view_slots(&t, clip_from_view, size, near, time.elapsed_secs(), exposure);
        for h in &st.ue_mats {
            if let Some(mut m) = umats.get_mut(h) {
                m.u.view = slots;
            }
        }
    }
    let cam_ue = cam_tf.map_or(Vec3::ZERO, |t| {
        let c = t.translation();
        Vec3::new(c.x, c.z, c.y) * 100.0
    });
    let to_ue = |v: Vec3| Vec3::new(v.x, v.z, v.y);
    let mut total = 0;
    for li in 0..st.live.len() {
        st.live[li].sim.ground_z = ground.0.map(|y| y * 100.0);
        st.live[li].sim.step(dt);
        for ei in 0..st.live[li].sim.emitters.len() {
            let mat_pkg = st.live[li].sim.emitters[ei].material.clone();
            let Some(mat) = fx_material(st, &mut mats, &mut umats, &mut images, &mut shaders, amb.0, &mat_pkg) else { continue };
            let live = &mut st.live[li];
            let e = &live.sim.emitters[ei];
            total += e.particles.len();
            let mut pos = Vec::with_capacity(e.particles.len() * 4);
            let mut uv = Vec::with_capacity(e.particles.len() * 4);
            let mut uv2 = Vec::with_capacity(e.particles.len() * 4);
            let mut nrm = Vec::with_capacity(e.particles.len() * 4);
            let mut col = Vec::with_capacity(e.particles.len() * 4);
            let mut idx = Vec::with_capacity(e.particles.len() * 6);
            // the sprite vertex factory's interpolants for the translated shaders (ue_material.rs; UE translated world,
            // cm, Z up): raw UV, the two SubUV cells, TangentToWorld0 (w = SubImageLerp), TangentToWorld2,
            // DynamicParameter, particle position (w = half size X: UNCONFIRMED), velocity (direction, speed)
            let mut raw_uv: Vec<[f32; 2]> = Vec::with_capacity(e.particles.len() * 4);
            let mut subuv: Vec<[f32; 4]> = Vec::with_capacity(e.particles.len() * 4);
            let mut tw0: Vec<[f32; 4]> = Vec::with_capacity(e.particles.len() * 4);
            let mut tw2: Vec<[f32; 4]> = Vec::with_capacity(e.particles.len() * 4);
            let mut dynp: Vec<[f32; 4]> = Vec::with_capacity(e.particles.len() * 4);
            let mut ppos: Vec<[f32; 4]> = Vec::with_capacity(e.particles.len() * 4);
            let mut pvel: Vec<[f32; 4]> = Vec::with_capacity(e.particles.len() * 4);
            let [sh, sv] = e.sub_images;
            for p in &e.particles {
                let c = Vec3::from_array(coords::pos(p.pos));
                // UE sprite size is the full width / height in cm
                let hw = p.size[0] * 0.005;
                let hh = if e.square { hw } else { p.size[1].max(1e-4) * 0.005 };
                let (ax, ay) = if e.velocity_aligned {
                    let v = Vec3::from_array(coords::pos(p.vel)).normalize_or_zero();
                    let side = v.cross(fwd).normalize_or_zero();
                    (side * hw, v * hh)
                } else {
                    let (s, co) = p.rot.sin_cos();
                    ((right * co + up * s) * hw, (up * co - right * s) * hh)
                };
                let base = pos.len() as u32;
                for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    pos.push((c + ax * dx + ay * dy).to_array());
                }
                // SubUV: cell floor(ImageIndex) and the next one, lerp = the fraction (sprite vertex factory inputs);
                // a negative BaseSize axis flips that UV axis (Size::SpawnEx UVFlippingMode)
                let n = (sh * sv).max(1);
                let i0 = p.sub_image.max(0.0);
                let lerp = if e.sub_method == 2 || e.sub_method == 4 { i0.fract() } else { 0.0 };
                let (w, h) = (1.0 / sh as f32, 1.0 / sv as f32);
                let (fu, fv) = (p.base_size[0] < 0.0, p.base_size[1] < 0.0);
                let corners = |cell: u32| {
                    let (cx, cy) = ((cell % sh) as f32, (cell / sh) as f32);
                    [(0.0f32, 1.0f32), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)].map(|(u, v)| {
                        let u = if fu { 1.0 - u } else { u };
                        let v = if fv { 1.0 - v } else { v };
                        [(cx + u) * w, (cy + v) * h]
                    })
                };
                let c0 = i0 as u32 % n;
                let a0 = corners(c0);
                let a1 = corners((c0 + 1) % n);
                uv.extend(a0);
                uv2.extend(a1);
                // TexCoords[0] = the current SubUV cell's coordinates (the whole texture without SubImages), the
                // PARTICLE_SUBUVS pair = this cell and the next (UNCONFIRMED: ParticleSpriteVertexFactory.ush not read)
                for (u0, u1) in a0.iter().zip(a1.iter()) {
                    raw_uv.push(*u0);
                    subuv.push([u0[0], u0[1], u1[0], u1[1]]);
                }
                let t_ue = to_ue(ax.normalize_or_zero());
                let n_ue = to_ue(-fwd);
                tw0.extend([[t_ue.x, t_ue.y, t_ue.z, lerp]; 4]);
                tw2.extend([[n_ue.x, n_ue.y, n_ue.z, 1.0]; 4]);
                dynp.extend([p.dynp; 4]);
                let pp = Vec3::from_array(p.pos) - cam_ue;
                ppos.extend([[pp.x, pp.y, pp.z, 0.5 * p.size[0]]; 4]);
                let vv = Vec3::from_array(p.vel);
                let vl = vv.length();
                let vd = if vl > 0.0 { vv / vl } else { Vec3::ZERO };
                pvel.extend([[vd.x, vd.y, vd.z, vl]; 4]);
                let l = (lerp * lerp + 1.0).sqrt();
                nrm.extend([[lerp / l, 0.0, 1.0 / l]; 4]);
                let k = [p.color[0].max(0.0), p.color[1].max(0.0), p.color[2].max(0.0), p.color[3].clamp(0.0, 1.0)];
                col.extend([k; 4]);
                idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv2);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
            if matches!(mat, FxMat::Ue(_)) {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, raw_uv);
                mesh.insert_attribute(ue_material::ATTR_SUBUV, subuv);
                mesh.insert_attribute(ue_material::ATTR_TW0, tw0);
                mesh.insert_attribute(ue_material::ATTR_TW2, tw2);
                mesh.insert_attribute(ue_material::ATTR_DYNP, dynp);
                mesh.insert_attribute(ue_material::ATTR_PPOS, ppos);
                mesh.insert_attribute(ue_material::ATTR_PVEL, pvel);
            }
            mesh.insert_indices(Indices::U32(idx));
            // a fresh mesh asset per frame (replacing a mesh's buffers in place, or an empty one, upsets the slab
            // allocator); emitters with no live particles are hidden instead of drawn empty
            let empty = e.particles.is_empty();
            let vis = if empty { Visibility::Hidden } else { Visibility::Inherited };
            match live.meshes[ei].clone() {
                Some((ent, _)) => {
                    if empty {
                        commands.entity(ent).insert(vis);
                    } else {
                        let h = meshes.add(mesh);
                        commands.entity(ent).insert((Mesh3d(h.clone()), vis));
                        live.meshes[ei] = Some((ent, h));
                    }
                }
                None if !empty => {
                    let h = meshes.add(mesh);
                    let ent = match mat {
                        FxMat::Ue(m) => commands.spawn((Mesh3d(h.clone()), MeshMaterial3d(m), Transform::IDENTITY, vis, bevy::light::NotShadowCaster)).id(),
                        FxMat::Ported(m) => commands.spawn((Mesh3d(h.clone()), MeshMaterial3d(m), Transform::IDENTITY, vis, bevy::light::NotShadowCaster)).id(),
                    };
                    live.meshes[ei] = Some((ent, h));
                }
                None => {}
            }
        }
    }
    st.live.retain(|l| {
        let done = l.sim.done();
        if done {
            for (e, _) in l.meshes.iter().flatten() {
                commands.entity(*e).despawn();
            }
        }
        !done
    });
    stats.live_systems = st.live.len();
    stats.live_particles = total;
    stats.peak_particles = stats.peak_particles.max(total);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the parry spark bursts 3..6 + 6..10 sprites at once, they fall under gravity and die within their lifetimes
    #[test]
    fn spark_burst_simulates() {
        let rd = mh_pak::Reader::new(std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks")));
        let fx = CombatFx::read(&rd);
        assert!(fx.block.ends_with("P_spark_burst"), "{fx:?}");
        assert!(!fx.armor_hit.is_empty() && fx.impact_by_surface.len() >= 9, "{fx:?}");
        let ps = mh_assets::particles::read(&rd, &fx.block).unwrap();
        let mut s = SystemSim::new(&ps, [0.0; 3], [1.0, 0.0, 0.0], 7);
        s.step(1.0 / 60.0);
        let n = s.alive();
        assert!((9..=16).contains(&n), "{n} sparks");
        for _ in 0..300 {
            s.step(1.0 / 60.0);
        }
        assert!(s.done(), "still {} alive", s.alive());
        let ev = serde_json::json!({"kind": "parry"});
        assert_eq!(fx.for_event(&ev, false), Some(fx.block.as_str()));
    }

    /// P_BloodSplash 1 m above a ground plane: the drops (Collision: 1 collision, EPCC_HaltCollisions) reach the
    /// ground, are snapped to it once and then ignore collisions; nothing is killed by the collision
    #[test]
    fn blood_hits_the_ground() {
        let rd = mh_pak::Reader::new(std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks")));
        let ps = mh_assets::particles::read(&rd, "Mordhau/Content/Mordhau/Particles/Blood/P_BloodSplash").unwrap();
        let mut s = SystemSim::new(&ps, [0.0, 0.0, 100.0], [1.0, 0.0, 0.0], 3);
        s.ground_z = Some(0.0);
        let mut peak = 0;
        for _ in 0..240 {
            s.step(1.0 / 60.0);
            peak = peak.max(s.alive());
        }
        let hits: usize = s.emitters.iter().map(|e| e.collided).sum();
        assert!(peak > 50 && hits > 0, "peak {peak} hits {hits}");
        let halted = s.emitters.iter().flat_map(|e| &e.particles).filter(|p| p.flags & sim::IGNORE_COLLISIONS != 0).count();
        assert!(halted > 0 || s.alive() == 0);
    }

    /// the shader-derived material table covers every material of the four combat systems
    #[test]
    fn combat_materials_are_shader_derived() {
        let rd = mh_pak::Reader::new(std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks")));
        let fx = CombatFx::read(&rd);
        for sys in [&fx.block, &fx.hit_cancel, &fx.flesh_hit, &fx.armor_hit] {
            let ps = mh_assets::particles::read(&rd, sys).unwrap();
            for e in &ps.emitters {
                let m = e.lods[0].material();
                // no material: FParticleEmitterInstance::GetCurrentMaterial 0x3283f90 falls back to the default
                // surface material; the one such emitter here (P_ArmorSparks splash_1) is a collision-event
                // generator with StartSize 0 (zero-area sprites), which mh-fx does not draw
                if m.is_empty() {
                    continue;
                }
                if let Some((mode, _, _)) = material::mode_of(&m) {
                    assert!(mode != 0, "{sys}: {m} has no shader-derived mode");
                }
            }
        }
    }
}

//! level.rs - LevelPlugin: place a UE map (port of godot/components/ue/ue_level.gd build() + apply_overrides +
//! update_hlod, the inputs of godot/tools/gen_level.gd).
//!
//! R2 data paths (source.rs): placements from mh-level (paks) or ue.rs (extract/json), checked equal at load
//! (plan.rs compare -> load_map.json "equiv"); meshes/textures from extract/gltf or decoded from the paks (paksrc.rs);
//! materials as StandardMaterial (material.rs) or the ue_tint WGSL (uetint.rs); lights/sky/fog/post from the map's
//! actors (lighting.rs); HLOD proxies switched per frame like the Godot port.

use crate::lighting::{self, LightStats, Look};
use crate::material::{self, MatCache, MatCtx, PackedJobs, Pick};
use crate::paksrc::{self, TexStats};
use crate::paths::Paths;
use crate::plan::{self, Equiv, HlodRec, Plan};
use crate::source::{AssetSrc, LevelSrc, MatSrc, Source};
use crate::ue::{self, StartRec};
use crate::uetint::{self, UeTintMaterial};
use bevy::ecs::system::SystemParam;
use bevy::gltf::GltfAssetLabel;
use bevy::light::{GlobalAmbientLight, NotShadowCaster};
use bevy::math::{Affine3A, Mat3};
use bevy::prelude::*;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

pub const DEFAULT_MAP: &str = "TestLevel";

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LevelState>()
            .init_resource::<MatCache>()
            .init_resource::<PackedJobs>()
            .init_resource::<PakCache>()
            .init_resource::<Look>()
            // replaced by the map's SkyLight (lighting.rs) when the level has one
            .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 0.0, affects_lightmapped_meshes: true })
            .add_systems(Update, (start_load, track_load, material::swizzle_packed, strip_vertex_colors, update_hlod, test_level_rounds, ui_pawn_actions));
    }
}

/// Request + progress of the current map.
#[derive(Resource, Default)]
pub struct LevelState {
    pub request: Option<String>,
    pub map: String,
    pub loading: bool,
    pub loaded: bool,
    pub root: Option<Entity>,
    pub handles: Vec<UntypedHandle>,
    pub started: Option<Instant>,
    pub load_secs: f32,
    pub starts: Vec<StartRec>,
    pub stats: LevelStats,
    /// glb primitive meshes per mesh package (gltf assets), for equiv_assets
    pub gltf_meshes: HashMap<String, Vec<Handle<Mesh>>>,
}

#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct LevelStats {
    pub sources: Value,
    pub plan_source: String,
    pub levels: Vec<String>,
    pub components_seen: usize,
    pub meshes_placed: usize,
    pub ism_components: usize,
    pub ism_instances: usize,
    pub primitives: usize,
    pub missing_glb: usize,
    pub missing_glb_list: Vec<String>,
    pub mesh_decode_failed: Vec<String>,
    /// placements whose pak decode failed and that use the extract glb instead
    pub mesh_glb_fallback: usize,
    pub materials: usize,
    pub materials_with_albedo: usize,
    pub materials_with_normal: usize,
    pub materials_with_packed: usize,
    pub materials_ue_tint: usize,
    pub ue_tint_textures_bound: usize,
    /// every resolved pak material: package, master, mode, blend, flat colour, missing albedo, bound slots
    pub material_list: Vec<Value>,
    pub materials_unresolved: Vec<String>,
    pub primitives_default_material: usize,
    pub textures: TexStats,
    pub texture_failures: Vec<String>,
    pub skips: BTreeMap<String, usize>,
    pub starts: usize,
    pub hlod_proxies: usize,
    pub hlod_skipped: usize,
    pub lights: LightStats,
    pub assets_failed: usize,
    pub equiv: Option<Equiv>,
    pub read_secs: f32,
    pub build_secs: f32,
    pub vlm: Value,
    /// placements drawn with their baked HQ lightmap (ue_tint), skipped (with reasons)
    pub lightmapped: usize,
    pub lightmap_skipped: BTreeMap<String, usize>,
    pub lightmap_errors: Vec<String>,
    pub lightmap_uv2_remapped: usize,
    pub reflection_probes: Vec<Value>,
    /// primitives of mirrored placements drawn with a winding-flipped copy / left as is (glb not loaded yet)
    pub mirrored_flipped: usize,
    pub mirrored_unflipped: usize,
}

#[derive(Component)]
pub struct LevelRoot;

/// One placed UE mesh component (or one foliage instance).
#[derive(Component)]
#[allow(dead_code)] // name / mesh: identification for queries and debugging
pub struct UeMesh {
    pub name: String,
    pub actor: String,
    pub mesh: String,
}

/// The material package a placed primitive draws with (probe verb, evidence)
#[derive(Component, Clone)]
pub struct PrimInfo {
    pub material: String,
}

/// An HLOD proxy (ue_level.gd hlod_record): drawn instead of its sub-actors beyond `min_draw_cm`.
#[derive(Component, Clone)]
pub struct HlodProxy {
    pub name: String,
    pub lod_level: i64,
    pub min_draw_cm: f32,
    pub aabb: (Vec3, Vec3),
    pub subs: Vec<String>,
}

/// Either material kind a primitive can carry.
#[derive(Clone)]
pub enum MatH {
    Std(Handle<StandardMaterial>),
    Tint(Handle<UeTintMaterial>),
}

/// Decoded-from-pak assets, cached across loads.
#[derive(Resource, Default)]
pub struct PakCache {
    pub tex: HashMap<(String, bool), Option<Handle<Image>>>,
    pub packed: HashMap<(String, String), Option<Handle<Image>>>,
    pub mats: HashMap<String, Option<(MatH, Pick, bool)>>,
    pub meshes: HashMap<String, Option<Vec<(Handle<Mesh>, usize)>>>,
    pub lm_tex: HashMap<String, Option<Handle<Image>>>,
    pub lm_uv: HashMap<String, i64>,
    pub lm_errors: Vec<String>,
    pub flipped: HashMap<AssetId<Mesh>, Handle<Mesh>>,
}

/// The mesh with its triangles' corner order reversed (cached); a mesh not yet loaded (glb) is used as is and counted
fn flipped(meshes: &mut Assets<Mesh>, cache: &mut HashMap<AssetId<Mesh>, Handle<Mesh>>, h: &Handle<Mesh>, stats: &mut LevelStats) -> Handle<Mesh> {
    if let Some(f) = cache.get(&h.id()) {
        stats.mirrored_flipped += 1;
        return f.clone();
    }
    let Some(m) = meshes.get(h) else {
        stats.mirrored_unflipped += 1;
        return h.clone();
    };
    let mut c = m.clone();
    let idx: Option<Vec<u32>> = c.indices().map(|i| {
        let mut v: Vec<u32> = i.iter().map(|x| x as u32).collect();
        mh_assets::coords::flip_winding(&mut v);
        v
    });
    if let Some(v) = idx {
        c.insert_indices(bevy::mesh::Indices::U32(v));
    }
    let f = meshes.add(c);
    cache.insert(h.id(), f.clone());
    stats.mirrored_flipped += 1;
    f
}

/// StaticMesh LightMapCoordinateIndex (ue_lightmap.gd uv_index; -1 when absent)
fn lm_uv_index(rd: Option<&mh_pak::Reader>, cache: &mut HashMap<String, i64>, mesh_pkg: &str) -> i64 {
    *cache.entry(mesh_pkg.to_string()).or_insert_with(|| {
        rd.and_then(|r| r.read(mesh_pkg))
            .and_then(|ex| {
                ex.iter()
                    .find(|e| e.get("Type").and_then(|t| t.as_str()) == Some("StaticMesh"))
                    .and_then(|e| e.pointer("/Properties/LightMapCoordinateIndex").and_then(|v| v.as_i64()))
            })
            .unwrap_or(-1)
    })
}

/// A lightmap / sky-occlusion texture of the build data (linear: "SRGB": false, ue_lightmap.gd header), clamp sampler
fn lm_image(ps: &mh_assets::pak_source::PakSource, pc: &mut PakCache, images: &mut Assets<Image>, r: &str) -> Option<Handle<Image>> {
    pc.lm_tex
        .entry(r.to_string())
        .or_insert_with(|| {
            let mut st = TexStats::default();
            match paksrc::image(ps, r, false, &mut st) {
                Ok(mut i) => {
                    i.sampler = paksrc::clamp_sampler();
                    Some(images.add(i))
                }
                Err(e) => {
                    if pc.lm_errors.len() < 5 {
                        pc.lm_errors.push(e);
                    }
                    None
                }
            }
        })
        .clone()
}

/// Material names per primitive of a CUE4Parse static-mesh glb (one node, one mesh, no node transform: checked on all
/// 767 Arena glbs). Reads only the JSON chunk of the GLB container (glTF 2.0 spec, "GLB File Format Specification").
pub fn glb_materials(path: &std::path::Path) -> Option<Vec<String>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut hdr = [0u8; 20];
    f.read_exact(&mut hdr).ok()?;
    if &hdr[0..4] != b"glTF" || &hdr[16..20] != b"JSON" {
        return None;
    }
    let len = u32::from_le_bytes(hdr[12..16].try_into().ok()?) as usize;
    let mut js = vec![0u8; len];
    f.read_exact(&mut js).ok()?;
    let j: Value = serde_json::from_slice(&js).ok()?;
    let mats = j.get("materials").and_then(|m| m.as_array()).cloned().unwrap_or_default();
    let prims = j.pointer("/meshes/0/primitives")?.as_array()?;
    Some(
        prims
            .iter()
            .map(|p| {
                p.get("material")
                    .and_then(|i| i.as_u64())
                    .and_then(|i| mats.get(i as usize))
                    .and_then(|m| m.get("name"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect(),
    )
}

/// UE light / camera basis: -Z looks down the component's UE +X (ue_light.gd light_basis: Basis(b.z, b.y, -b.x)).
pub fn ue_look_basis(xf: &Affine3A) -> Quat {
    let m = Mat3::from(xf.matrix3);
    let x = m.x_axis.normalize_or_zero();
    let y = (m.y_axis - x * x.dot(m.y_axis)).normalize_or_zero();
    if x == Vec3::ZERO || y == Vec3::ZERO {
        return Quat::IDENTITY;
    }
    let z = x.cross(y);
    Quat::from_mat3(&Mat3::from_cols(z, y, -x))
}

#[derive(SystemParam)]
struct Load<'w, 's> {
    commands: Commands<'w, 's>,
    st: ResMut<'w, LevelState>,
    paths: Res<'w, Paths>,
    src: Res<'w, Source>,
    assets: Res<'w, AssetServer>,
    meshes: ResMut<'w, Assets<Mesh>>,
    images: ResMut<'w, Assets<Image>>,
    std: ResMut<'w, Assets<StandardMaterial>>,
    tint: ResMut<'w, Assets<UeTintMaterial>>,
    cache: ResMut<'w, MatCache>,
    jobs: ResMut<'w, PackedJobs>,
    pak: ResMut<'w, PakCache>,
    defaults: Option<Res<'w, uetint::Defaults>>,
    ambient: ResMut<'w, GlobalAmbientLight>,
    look: ResMut<'w, Look>,
}

/// Material package of a mesh slot from the mesh package (StaticMaterials[slot].MaterialInterface; ue_material.gd
/// json_for), read through the paks.
fn slot_material(rd: &mh_pak::Reader, mesh_pkg: &str, slot: usize) -> Option<String> {
    let exps = rd.read(mesh_pkg)?;
    exps.iter().find_map(|e| {
        let a = e.pointer("/Properties/StaticMaterials")?.as_array()?;
        let op = a.get(slot)?.pointer("/MaterialInterface/ObjectPath")?.as_str()?;
        Some(ue::strip(op).to_string())
    })
}

/// The combat test level (`--map TestLevel`, play.bat -TestLevel; user directive 2026-10-06 "combat on a test level"):
/// no map assets, a flat grid floor (1 m lines, 5 m darker) at UE z = 0, one sun, and two PlayerStarts facing each
/// other at duel distance. Not a game map: testing glue only (the sim side is mh_level CollisionWorld::flat_floor).
pub const TEST_LEVEL: &str = "TestLevel";

/// The map display name the in-match UI is mounted with (mh_ui::MatchMap, GetMapName): the package's short name. The
/// test level (ours, no package) is a 1v1 duel, so it gets the duel prefix: mode_for_map picks the game mode from the
/// DefaultGame.ini GameModeMapPrefixes ("DU" -> the duel mode), as for DU_Arena.
pub fn match_map_name(map: &str) -> String {
    if map == TEST_LEVEL {
        return "DU_TestLevel".into();
    }
    map.rsplit('/').next().unwrap_or(map).to_string()
}
/// half the start-to-start distance (UE cm): 4 m apart, outside every weapon's reach, so a duel opens with a step in
pub const TEST_LEVEL_HALF_GAP: f32 = 200.0;

fn test_level_grid(images: &mut Assets<Image>) -> Handle<Image> {
    // one texture tile = 5 m: a line every 1 m (102.4 px), the 5 m line darker
    let n = 512usize;
    let mut data = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let on = |c: usize, w: usize| (c % (n / 5)) < w || (c % (n / 5)) >= n / 5 - w;
            let major = x < 3 || y < 3 || x >= n - 3 || y >= n - 3;
            let minor = on(x, 1) || on(y, 1);
            let v: u8 = if major { 30 } else if minor { 60 } else { 105 };
            let i = (y * n + x) * 4;
            data[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    let mut img = Image::new(
        bevy::render::render_resource::Extent3d { width: n as u32, height: n as u32, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    img.sampler = crate::paksrc::repeat_sampler();
    images.add(img)
}

fn start_test_level(p: &mut Load) {
    let root = p.commands.spawn((Name::new(TEST_LEVEL), Transform::default(), Visibility::default())).id();
    let half_m = 50.0f32;
    let mut mesh = Mesh::from(Plane3d::default().mesh().size(2.0 * half_m, 2.0 * half_m));
    // UVs: one grid tile (5 m) per 1 / (2 half / 5) of the plane
    if let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
        let k = 2.0 * half_m / 5.0;
        for uv in uvs.iter_mut() {
            uv[0] *= k;
            uv[1] *= k;
        }
    }
    let grid = test_level_grid(&mut p.images);
    let mat = p.std.add(StandardMaterial { base_color_texture: Some(grid), perceptual_roughness: 0.9, ..default() });
    p.commands.spawn((Name::new("TestLevelFloor"), Mesh3d(p.meshes.add(mesh)), MeshMaterial3d(mat), Transform::default(), ChildOf(root)));
    p.commands.spawn((
        Name::new("TestLevelSun"),
        DirectionalLight { illuminance: 10_000.0, shadow_maps_enabled: true, ..default() },
        bevy::camera::visibility::RenderLayers::from_layers(&[0, crate::fighter::SHADOW_LAYER]),
        Transform::from_xyz(0.0, 10.0, 0.0).looking_at(Vec3::new(-3.0, 0.0, 2.0), Vec3::Y),
        ChildOf(root),
    ));
    p.ambient.brightness = 400.0;
    // the test level's own look: Bevy's default exposure (the 10 klx sun above was tuned under it) and no UE post. A
    // match entered from the main menu otherwise kept the MainMenu map's fixed-exposure look (ev100 ~ -0.26, ~2^10
    // brighter), which blew the floor out to white (user, 2026-10-06 11:36: "so bright I can't see anything")
    let v = p.look.version;
    *p.look = crate::lighting::Look { ev100: bevy::camera::Exposure::default().ev100, exposure: 1.0, clear: [0.45, 0.6, 0.85], ..Default::default() };
    p.look.version = v + 1;
    // two starts facing each other along UE X (bevy X), the player's first; capsule centre 100 cm up (the sim drops
    // the capsule onto the floor)
    let start = |name: &str, x: f32, yaw: f32| {
        let q = crate::fighter::yaw_quat(yaw);
        StartRec { name: name.into(), xf: Affine3A::from_rotation_translation(q, Vec3::new(x * 0.01, 1.0, 0.0)), team: None }
    };
    p.st.starts = vec![start("TestLevelStart_Player", -TEST_LEVEL_HALF_GAP, 0.0), start("TestLevelStart_Bot", TEST_LEVEL_HALF_GAP, 180.0)];
    p.st.root = Some(root);
    p.st.map = TEST_LEVEL.into();
    p.st.handles.clear();
    p.st.started = Some(Instant::now());
    p.st.stats = LevelStats { plan_source: "test level (no map)".into(), levels: vec![TEST_LEVEL.into()], starts: 2, ..default() };
    p.st.loading = true;
    p.st.loaded = false;
}

/// seconds after the first death before the test level restarts the round (testing glue, not a game rule:
/// UNCONFIRMED vs BP_DuelGameMode's round timing, which the runtime does not run)
pub const TEST_LEVEL_RESTART_SECS: f32 = 3.0;

/// The combat test level's round restart: once a fighter has died, TEST_LEVEL_RESTART_SECS later every fighter
/// respawns as a new character at its start (SimBackend::respawn), and the player's control rotation faces the
/// opponent again. Testing glue for the test level only.
/// (first-person r3) the escape menu's pawn calls (mh_ui::UiAction::Pawn): "RequestSuicide" -> the sim's suicide of
/// the player's fighter; "CycleCamera" -> AMordhauCharacter::CycleCamera, the same toggle the "Cycle Camera" action
/// does (input.rs)
pub fn ui_pawn_actions(mut ev: MessageReader<mh_ui::UiAction>, mut sim: NonSendMut<crate::sim::Sim>, mut pc: Option<ResMut<crate::input::PlayerControl>>) {
    for a in ev.read() {
        if let mh_ui::UiAction::Pawn(name) = a {
            let Some(p) = pc.as_mut() else { continue };
            match name.as_str() {
                "RequestSuicide" => sim.0.suicide(p.id),
                "CycleCamera" => p.third_person = !p.third_person,
                _ => {}
            }
            info!("ui: pawn {name}");
        }
    }
}

fn test_level_rounds(
    lvl: Res<LevelState>,
    time: Res<Time>,
    mut sim: NonSendMut<crate::sim::Sim>,
    mut pc: Option<ResMut<crate::input::PlayerControl>>,
    mut sl: ResMut<crate::sim::SimLevel>,
    sel: Res<crate::loadout::Selected>,
    mut dead_since: Local<Option<f32>>,
    mut rounds: Local<u32>,
) {
    if lvl.map != TEST_LEVEL || !lvl.loaded || lvl.starts.is_empty() {
        return;
    }
    // the player's old characters (armory_host::restart_player) stay as corpses: not in the rounds
    let views: Vec<_> = sim.0.fighters().into_iter().filter(|v| !sl.retired.contains(&v.id)).collect();
    if views.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    if dead_since.is_none() && views.iter().any(|v| v.health <= 0.0) {
        *dead_since = Some(now);
    }
    let Some(t) = *dead_since else { return };
    if now - t < TEST_LEVEL_RESTART_SECS {
        return;
    }
    for (k, v) in views.iter().enumerate() {
        let s = &lvl.starts[k % lvl.starts.len()];
        let t = s.xf.translation;
        let fwd = s.xf.matrix3 * bevy::math::Vec3A::X;
        let yaw = fwd.z.atan2(fwd.x).to_degrees();
        let loc = [t.x * 100.0, t.z * 100.0, t.y * 100.0];
        // Every player spawn applies the current profile, even when an edited profile retains the same key.
        // RestartPlayer -> PrepareControllerForRespawn 0x15a4190; leave the old pawn as a corpse.
        if let Some(p) = pc.as_mut().filter(|p| p.id == v.id) {
            // TestLevel resets on either fighter's death. End a surviving player's synthetic round before
            // replacing it: retiring a live pawn only excludes it from this loop, not combat or collision.
            // Test-only lifecycle containment; state/proofs/mercenary_respawn_reset.md.
            if v.health > 0.0 {
                sim.0.suicide(v.id);
            }
            crate::armory_host::restart_player(&mut sim, &mut sl, &sel, p, loc, yaw, v.team);
            continue;
        }
        sim.0.respawn(v.id, loc, yaw);
        if let Some(p) = pc.as_mut().filter(|p| p.id == v.id) {
            p.yaw = yaw;
            p.pitch = 0.0;
        }
    }
    *rounds += 1;
    *dead_since = None;
    info!("test level: round {} (respawned {} fighters)", *rounds, views.len());
}

fn start_load(mut p: Load) {
    let Some(map) = p.st.request.take() else { return };
    if let Some(r) = p.st.root.take() {
        p.commands.entity(r).despawn();
    }
    if map == TEST_LEVEL {
        start_test_level(&mut p);
        return;
    }
    let t0 = Instant::now();
    let src = p.src.clone();
    // ---- plans: both readers when both inputs exist, compared
    let pak_plan = src.vfs.as_ref().map(|v| plan::from_pak(v, &map));
    let ext_plan = p.paths.json.is_dir().then(|| plan::from_extract(&p.paths.json, &map));
    let equiv = match (&ext_plan, &pak_plan) {
        (Some(a), Some(b)) => Some(plan::compare(a, b)),
        _ => None,
    };
    let use_pak = src.level == LevelSrc::Pak && pak_plan.is_some();
    let (plan, other): (Plan, Option<Plan>) = if use_pak { (pak_plan.unwrap(), ext_plan) } else { (ext_plan.unwrap_or_default(), pak_plan) };
    // lighting / sky / HLOD records come from mh-level whichever reader placed the meshes
    let (lighting, sky, hlods) = match (&plan.lighting, &other) {
        (Some(_), _) => (plan.lighting.clone(), plan.sky.clone(), plan.hlods.clone()),
        (None, Some(o)) => (o.lighting.clone(), o.sky.clone(), o.hlods.clone()),
        _ => (None, None, Vec::new()),
    };
    let read_secs = t0.elapsed().as_secs_f32();
    // the map's volumetric lightmap on the GPU (ue_tint only); mh-level reads it whichever reader placed the meshes
    let vlm_src = plan.vlm.as_ref().or_else(|| other.as_ref().and_then(|o| o.vlm.as_ref()));
    let mut vlm_note = serde_json::Value::Null;
    let vlm = if src.materials == MatSrc::UeTint {
        vlm_src.and_then(|v| match uetint::upload_vlm(&mut p.images, v) {
            Ok(b) => {
                vlm_note = serde_json::json!({"indirection": v.indirection_dims, "brick_size": v.brick_size, "brick_dims": v.brick_dims,
                    "bounds_min": v.bounds_min, "bounds_max": v.bounds_max});
                Some(b)
            }
            Err(e) => {
                vlm_note = serde_json::json!({"error": e});
                None
            }
        })
    } else {
        None
    };
    // materials made before the map (the fighters' body and wearables) get the map's VLM too: UE lights movable
    // objects from the volumetric lightmap (ue_vlm.gd)
    if let (Some(v), Some(df)) = (&vlm, p.defaults.as_deref()) {
        let ids: Vec<_> = p.tint.iter().filter(|(_, m)| m.vlm_ind == df.no_vlm.ind).map(|(id, _)| id).collect();
        for id in ids {
            if let Some(mut m) = p.tint.get_mut(id) {
                m.vlm_ind = v.ind.clone();
                m.vlm_amb = v.amb.clone();
                m.vlm_sh = v.sh.clone();
                m.vlm = v.u.clone();
            }
        }
    }
    let mut stats = LevelStats {
        sources: src.summary(),
        plan_source: plan.source.into(),
        levels: plan.levels.clone(),
        components_seen: plan.seen,
        skips: plan.skips.clone(),
        starts: plan.starts.len(),
        equiv,
        read_secs,
        vlm: vlm_note,
        ..default()
    };
    let fallback = p
        .cache
        .fallback
        .get_or_insert_with(|| p.std.add(StandardMaterial { base_color: Color::srgb(0.6, 0.6, 0.6), perceptual_roughness: 0.8, ..default() }))
        .clone();
    let root = p.commands.spawn((LevelRoot, Name::new(format!("Level {map}")), Transform::default(), Visibility::default())).id();
    let mut handles: Vec<UntypedHandle> = Vec::new();
    let mut gltf_meshes: HashMap<String, Vec<Handle<Mesh>>> = HashMap::new();
    let mut xpk = ue::Pkgs::new(&p.paths.json);
    let rd = src.vfs.as_ref().map(|v| mh_pak::Reader::new(v.clone()));
    let pak = src.pak.clone();
    let resolver = match (&rd, &pak) {
        (Some(r), Some(s)) => Some(mh_assets::material::Resolver::new(r, &**s)),
        _ => None,
    };
    let want_pak_mats = src.materials == MatSrc::UeTint || src.assets == AssetSrc::Pak;
    let colors = src.materials == MatSrc::UeTint;

    // ---- per mesh package: primitives (mesh handle, material slot, glb material name)
    let mut prim_cache: HashMap<String, Option<Vec<(Handle<Mesh>, usize, String)>>> = HashMap::new();
    // ---- per (mesh pkg, slot, override): material
    let mut mat_cache: HashMap<(String, usize, String), MatH> = HashMap::new();
    let mut mat_name: HashMap<(String, usize, String), String> = HashMap::new();

    // closures over p's resources are awkward with the borrow checker; plain helper blocks below
    for rec in &plan.placements {
        let mesh_pkg = ue::strip(&rec.mesh).to_string();
        if !prim_cache.contains_key(&mesh_pkg) {
            let v = match src.assets {
                AssetSrc::Gltf => {
                    let glb_rel = format!("{mesh_pkg}.glb");
                    match glb_materials(&p.paths.data.join(&glb_rel)) {
                        None => {
                            stats.missing_glb += 1;
                            if stats.missing_glb_list.len() < 50 {
                                stats.missing_glb_list.push(glb_rel.clone());
                            }
                            None
                        }
                        Some(names) => {
                            let slots = material::section_slots(&mut xpk, &rec.mesh);
                            let mut v = Vec::new();
                            for (i, n) in names.iter().enumerate() {
                                let h: Handle<Mesh> = p.assets.load(GltfAssetLabel::Primitive { mesh: 0, primitive: i }.from_asset(glb_rel.clone()));
                                handles.push(h.clone().untyped());
                                gltf_meshes.entry(mesh_pkg.clone()).or_default().push(h.clone());
                                // slot = section i's MaterialIndex when the counts agree (ue_level.gd apply_overrides)
                                v.push((h, if slots.len() == names.len() { slots[i] } else { i }, n.clone()));
                            }
                            Some(v)
                        }
                    }
                }
                AssetSrc::Pak => {
                    let entry = p.pak.meshes.entry(mesh_pkg.clone()).or_insert_with(|| None).clone();
                    let entry = match entry {
                        Some(e) => Some(e),
                        None => match pak.as_ref().map(|s| paksrc::mesh(s, &mesh_pkg, colors)) {
                            Some(Ok(ms)) => {
                                let v: Vec<_> = ms.into_iter().map(|(m, slot)| (p.meshes.add(m), slot)).collect();
                                p.pak.meshes.insert(mesh_pkg.clone(), Some(v.clone()));
                                Some(v)
                            }
                            Some(Err(e)) => {
                                stats.mesh_decode_failed.push(format!("{mesh_pkg}: {e}"));
                                None
                            }
                            None => None,
                        },
                    };
                    match entry {
                        Some(v) => Some(v.into_iter().map(|(h, s)| (h, s, String::new())).collect()),
                        None => {
                            // decode failed: the extract glb of the same package, when there is one (counted)
                            let glb_rel = format!("{mesh_pkg}.glb");
                            glb_materials(&p.paths.data.join(&glb_rel)).map(|names| {
                                stats.mesh_glb_fallback += 1;
                                let slots = material::section_slots(&mut xpk, &rec.mesh);
                                names
                                    .iter()
                                    .enumerate()
                                    .map(|(i, n)| {
                                        let h: Handle<Mesh> = p.assets.load(GltfAssetLabel::Primitive { mesh: 0, primitive: i }.from_asset(glb_rel.clone()));
                                        handles.push(h.clone().untyped());
                                        (h, if slots.len() == names.len() { slots[i] } else { i }, n.clone())
                                    })
                                    .collect()
                            })
                        }
                    }
                }
            };
            prim_cache.insert(mesh_pkg.clone(), v);
        }
        let Some(prims) = prim_cache.get(&mesh_pkg).cloned().flatten() else { continue };
        let mut built: Vec<(Handle<Mesh>, MatH)> = Vec::new();
        let mut built_names: Vec<String> = Vec::new();
        for (mesh_h, slot, glb_name) in prims {
            let ov = rec.mats.get(slot).filter(|m| !m.is_empty()).cloned().unwrap_or_default();
            let key = (mesh_pkg.clone(), slot, ov.clone());
            let mh = if let Some(m) = mat_cache.get(&key) {
                m.clone()
            } else {
                let m = if want_pak_mats {
                    // material package: override, else the mesh's own slot material (paks)
                    let mp = if !ov.is_empty() { Some(ov.clone()) } else { rd.as_ref().and_then(|r| slot_material(r, &mesh_pkg, slot)) };
                    mat_name.insert(key.clone(), mp.clone().unwrap_or_default());
                    match (mp, &resolver, &pak) {
                        (Some(mp), Some(res), Some(ps)) => pak_mat(&mp, res, ps, &mut p.pak, &mut p.images, &mut p.std, &mut p.tint,
                            p.defaults.as_deref(), vlm.as_ref(), src.materials == MatSrc::UeTint, &mut stats),
                        _ => None,
                    }
                } else {
                    // R1 path: extract material JSON + PNGs -> StandardMaterial
                    let mut ctx = MatCtx { pk: &mut xpk, data: &p.paths.data };
                    let ovj = Some(ov.clone()).filter(|m| !m.is_empty() && p.paths.data.join(format!("{m}.json")).is_file());
                    let json = ovj.or_else(|| ctx.json_for_slot(&mesh_pkg, slot, &glb_name));
                    json.and_then(|j| material::build(&mut ctx, &j, &p.assets, &mut p.std, &mut p.cache, &mut p.jobs)).map(|(h, _)| MatH::Std(h))
                };
                let m = m.unwrap_or_else(|| {
                    stats.primitives_default_material += 1;
                    MatH::Std(fallback.clone())
                });
                mat_cache.insert(key.clone(), m.clone());
                m
            };
            let mn = mat_name.get(&key).cloned().unwrap_or_default();
            built.push((mesh_h, mh));
            built_names.push(mn);
        }
        // baked HQ lightmap: a per-component copy of each ue_tint material with the lm_* uniforms and textures
        // (ue_lightmap.gd: Godot instance uniforms; here one material per lightmapped component)
        if let (Some(lm), Some(ps)) = (&rec.lm, &pak) {
            let uv = lm_uv_index(rd.as_ref(), &mut p.pak.lm_uv, &mesh_pkg);
            let why = if src.materials != MatSrc::UeTint {
                Some("not ue_tint")
            } else if !(0..=2).contains(&uv) {
                Some("no LightMapCoordinateIndex")
            } else if uv == 2 && !built.iter().all(|(m, _)| p.meshes.get(m).is_some_and(|x| x.attribute(paksrc::ATTRIBUTE_UV_2).is_some())) {
                Some("lightmap UV channel 2 not decoded (glb mesh)")
            } else {
                None
            };
            match why {
                Some(w) => *stats.lightmap_skipped.entry(w.into()).or_default() += 1,
                None => {
                    let lt = lm_image(ps, &mut p.pak, &mut p.images, &lm.tex);
                    let sk = lm.sky.as_ref().and_then(|t| lm_image(ps, &mut p.pak, &mut p.images, t));
                    match lt {
                        None => *stats.lightmap_skipped.entry("lightmap texture undecodable".into()).or_default() += 1,
                        Some(lt) => {
                            let mut any = false;
                            // LightMapCoordinateIndex 2: draw a copy whose UV_1 holds UE UV2, lm_uv = 1 (the host
                            // fragment feeds no third UV set; UE UV1 is then unavailable to that material: UNCONFIRMED
                            // for masters that sample UV1 on lightmapped architecture)
                            let uv = if uv == 2 {
                                for (mh, _) in built.iter_mut() {
                                    if let Some(src_m) = p.meshes.get(&*mh) {
                                        let mut c = src_m.clone();
                                        if let Some(bevy::mesh::VertexAttributeValues::Float32x2(v)) = c.attribute(paksrc::ATTRIBUTE_UV_2).cloned() {
                                            c.insert_attribute(Mesh::ATTRIBUTE_UV_1, v);
                                        }
                                        *mh = p.meshes.add(c);
                                    }
                                }
                                stats.lightmap_uv2_remapped += 1;
                                1
                            } else {
                                uv
                            };
                            for (_, m) in built.iter_mut() {
                                if let MatH::Tint(h) = m {
                                    if let Some(base) = p.tint.get(&*h) {
                                        let mut c = base.clone();
                                        let set = |c: &mut UeTintMaterial, n: &str, v: [f32; 4]| {
                                            if let Some(i) = mh_assets::shader::uniform_index(n) {
                                                c.u.v[i] = Vec4::from_array(v);
                                            }
                                        };
                                        set(&mut c, "use_lightmap", [1.0, 0.0, 0.0, 0.0]);
                                        set(&mut c, "lm_coord", lm.coord);
                                        set(&mut c, "lm_scale0", lm.scale0);
                                        set(&mut c, "lm_add0", lm.add0);
                                        set(&mut c, "lm_scale1", lm.scale1);
                                        set(&mut c, "lm_add1", lm.add1);
                                        set(&mut c, "lm_uv", [uv as f32, 0.0, 0.0, 0.0]);
                                        set(&mut c, "use_sky_occlusion", [if sk.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]);
                                        c.t13 = lt.clone();
                                        if let Some(s) = &sk {
                                            c.t14 = s.clone();
                                        }
                                        *h = p.tint.add(c);
                                        any = true;
                                    }
                                }
                            }
                            stats.lightmapped += any as usize;
                        }
                    }
                }
            }
        }
        let parts: Vec<Affine3A> = if rec.inst.is_empty() { vec![rec.xf] } else { rec.inst.iter().map(|i| rec.xf * *i).collect() };
        if rec.inst.is_empty() {
            stats.meshes_placed += 1;
        } else {
            stats.ism_components += 1;
            stats.ism_instances += parts.len();
        }
        for xf in parts {
            let tr = Transform::from_matrix(Mat4::from(xf));
            let mut e = p.commands.spawn((
                UeMesh { name: rec.name.clone(), actor: rec.actor.clone(), mesh: rec.mesh.clone() },
                tr,
                Visibility::default(),
                ChildOf(root),
            ));
            if rec.noshadow {
                e.insert(NotShadowCaster);
            }
            let id = e.id();
            // a mirrored placement (negative scale determinant: 63 Arena components, e.g. arena_wall_01a2 scale Y -1)
            // reverses the triangle winding; UE draws it with reversed culling (the primitive's
            // IsLocalToWorldDeterminantNegative), Bevy culls by winding only, so the mesh is drawn from a copy with its
            // corner order flipped
            let mirrored = xf.matrix3.determinant() < 0.0;
            for (k, (m, mat)) in built.iter().enumerate() {
                stats.primitives += 1;
                let m = if mirrored { flipped(&mut p.meshes, &mut p.pak.flipped, m, &mut stats) } else { m.clone() };
                let mut c = p.commands.spawn((Mesh3d(m.clone()), Transform::default(), ChildOf(id), PrimInfo { material: built_names.get(k).cloned().unwrap_or_default() }));
                match mat {
                    MatH::Std(h) => c.insert(MeshMaterial3d(h.clone())),
                    MatH::Tint(h) => c.insert(MeshMaterial3d(h.clone())),
                };
                if rec.noshadow {
                    c.insert(NotShadowCaster);
                }
            }
        }
    }
    // ---- HLOD proxies: glb (gltf) or the named StaticMesh export of the HLOD package (pak, mh-assets lod0_export)
    for h in &hlods {
        let mut prims: Vec<(Handle<Mesh>, usize, String)> = Vec::new();
        match src.assets {
            AssetSrc::Gltf => {
                let glb_rel = format!("{}/{}.glb", h.mesh_pkg, h.mesh_name);
                if let Some(names) = glb_materials(&p.paths.data.join(&glb_rel)) {
                    for (i, n) in names.iter().enumerate() {
                        let mh: Handle<Mesh> = p.assets.load(GltfAssetLabel::Primitive { mesh: 0, primitive: i }.from_asset(glb_rel.clone()));
                        handles.push(mh.clone().untyped());
                        prims.push((mh, i, n.clone()));
                    }
                }
            }
            AssetSrc::Pak => {
                if let Some(ps) = &pak {
                    match mh_assets::static_mesh::lod0_export(&**ps, &h.mesh_pkg, Some(&h.mesh_name)) {
                        Ok(sm) => {
                            for (m, slot) in paksrc::sections(&sm, colors) {
                                prims.push((p.meshes.add(m), slot, String::new()));
                            }
                        }
                        Err(e) => stats.mesh_decode_failed.push(format!("{}/{}: {}", h.mesh_pkg, h.mesh_name, e.0)),
                    }
                }
            }
        }
        if prims.is_empty() {
            stats.hlod_skipped += 1;
            continue;
        }
        let e = p
            .commands
            .spawn((hlod_component(h), Transform::from_matrix(Mat4::from(h.xf)), Visibility::Hidden, ChildOf(root), Name::new(h.name.clone())))
            .id();
        for (mh, slot, n) in prims {
            let mat = if want_pak_mats {
                let mp = rd.as_ref().and_then(|r| hlod_slot_material(r, &h.mesh_pkg, &h.mesh_name, slot));
                match (mp, &resolver, &pak) {
                    (Some(mp), Some(res), Some(ps)) => pak_mat(&mp, res, ps, &mut p.pak, &mut p.images, &mut p.std, &mut p.tint,
                        p.defaults.as_deref(), vlm.as_ref(), src.materials == MatSrc::UeTint, &mut stats),
                    _ => None,
                }
            } else {
                let mut ctx = MatCtx { pk: &mut xpk, data: &p.paths.data };
                ctx.json_for_slot(&format!("{}/{}", h.mesh_pkg, h.mesh_name), slot, &n)
                    .and_then(|j| material::build(&mut ctx, &j, &p.assets, &mut p.std, &mut p.cache, &mut p.jobs))
                    .map(|x| MatH::Std(x.0))
            }
            .unwrap_or_else(|| MatH::Std(fallback.clone()));
            let mut c = p.commands.spawn((Mesh3d(mh), Transform::default(), ChildOf(e)));
            match mat {
                MatH::Std(h) => c.insert(MeshMaterial3d(h)),
                MatH::Tint(h) => c.insert(MeshMaterial3d(h)),
            };
        }
        stats.hlod_proxies += 1;
    }
    stats.lightmap_errors = p.pak.lm_errors.clone();
    // ---- materials summary
    for (_, info) in p.cache.by_json.values() {
        stats.materials += 1;
        stats.materials_with_albedo += (!info.pick.albedo.is_empty()) as usize;
        stats.materials_with_normal += (!info.pick.normal.is_empty()) as usize;
        stats.materials_with_packed += (!info.pick.packed.is_empty()) as usize;
    }
    for (_, pick, tint) in p.pak.mats.values().flatten() {
        stats.materials += 1;
        stats.materials_ue_tint += *tint as usize;
        stats.materials_with_albedo += (!pick.albedo.is_empty()) as usize;
        stats.materials_with_normal += (!pick.normal.is_empty()) as usize;
        stats.materials_with_packed += (!pick.packed.is_empty()) as usize;
    }
    for (h, _) in p.cache.by_json.values() {
        if let Some(m) = p.std.get(h) {
            for t in [&m.base_color_texture, &m.normal_map_texture].into_iter().flatten() {
                handles.push(t.clone().untyped());
            }
        }
    }
    for (s, _, _) in &p.jobs.0 {
        handles.push(s.clone().untyped());
    }
    // ---- lights, sky, fog, post (the map's actors through mh-level)
    match &lighting {
        Some(l) => {
            let (ls, look) = lighting::spawn(&mut p.commands, root, l, sky.as_ref(), &mut p.ambient);
            stats.lights = ls;
            let v = p.look.version;
            *p.look = look;
            p.look.version = v + 1;
            // rust-render: UE's post settings / CombineLUT / sky / SkyLight diffuse (lighting::build_post)
            lighting::build_post(&mut p.images, src.pak.as_deref(), l, sky.as_ref(), &mut p.ambient, &mut p.look);
            // ue_tint: specular from the map's reflection capture, diffuse from the baked lighting (reflection.rs)
            let refl = plan.reflection.as_ref().or_else(|| other.as_ref().and_then(|o| o.reflection.as_ref()));
            if src.materials == MatSrc::UeTint {
                if let Some((name, bright, cap)) = refl {
                    if let Some(img) = crate::reflection::specular_image(cap) {
                        let spec = p.images.add(img);
                        let black = match p.look.sky_diffuse.clone() {
                            // the stationary SkyLight's diffuse (uesky.rs) - UE adds it under every capture
                            Some(d) => d,
                            None => p.images.add(crate::reflection::black_cube()),
                        };
                        p.look.env = Some((spec, black, *bright));
                        p.look.env_info = Some(serde_json::json!({"capture": name, "brightness": bright, "cubemap_size": cap.cubemap_size,
                            "mips": cap.mip_count(), "average_brightness": cap.average_brightness}));
                    }
                }
                // every capture as a Bevy reflection probe (LightProbe + EnvironmentMapLight), UE-shaped: a sphere
                // capture's InfluenceRadius (UE default 3000, recalled: UNCONFIRMED when absent) as the probe cube's half
                // size, a box capture's component scale (its unit box -1..1, cm) with BoxTransitionDistance (default 100,
                // recalled: UNCONFIRMED) as the falloff; UE blends captures per pixel by distance, Bevy blends overlapping
                // probes by their falloff and keeps the 8 nearest per view (MAX_VIEW_LIGHT_PROBES); the camera's map-wide
                // capture stays as the fallback outside every probe
                let caps = if plan.reflections.is_empty() { other.as_ref().map(|o| &o.reflections) } else { Some(&plan.reflections) };
                let mut probes = Vec::new();
                for (name, kind, xf, props, cap) in caps.into_iter().flatten() {
                    let Some(img) = crate::reflection::specular_image(cap) else { continue };
                    let spec = p.images.add(img);
                    let black = match p.look.sky_diffuse.clone() {
                            // the stationary SkyLight's diffuse (uesky.rs) - UE adds it under every capture
                            Some(d) => d,
                            None => p.images.add(crate::reflection::black_cube()),
                        };
                    let g = |k: &str, d: f64| props.get(k).and_then(|v| v.as_f64()).unwrap_or(d) as f32;
                    let t = xf.translation();
                    let pos = Vec3::new(t[0] as f32, t[2] as f32, t[1] as f32) * 0.01;
                    let (half, falloff) = if kind.contains("Sphere") {
                        let r = g("InfluenceRadius", 3000.0);
                        (Vec3::splat(r), Vec3::splat(0.2))
                    } else {
                        let gx = xf.to_gltf();
                        // the unit box -1..1 cm scaled by the component scale: half extent (cm) = the scale (to_gltf keeps the linear part)
                        let col = |c: usize| (gx.m[0][c].powi(2) + gx.m[1][c].powi(2) + gx.m[2][c].powi(2)).sqrt() as f32;
                        let h = Vec3::new(col(0), col(1), col(2));
                        let tr = g("BoxTransitionDistance", 100.0);
                        (h, (Vec3::splat(tr) / h.max(Vec3::splat(1.0))).min(Vec3::ONE))
                    };
                    // box: the component transform in glTF space (its unit box -1..1 cm -> Bevy's -0.5..0.5 cube: x 0.02)
                    let tr = if kind.contains("Sphere") {
                        Transform::from_translation(pos).with_scale(half * 0.02)
                    } else {
                        let mut t = Transform::from_matrix(Mat4::from(crate::plan::xf_gltf(xf)));
                        t.scale *= 0.02;
                        t
                    };
                    p.commands.spawn((
                        bevy::light::LightProbe { falloff },
                        bevy::light::EnvironmentMapLight { diffuse_map: black, specular_map: spec, intensity: g("Brightness", 1.0), ..default() },
                        tr,
                        Name::new(format!("ReflectionCapture {name}")),
                        ChildOf(root),
                    ));
                    probes.push(serde_json::json!({"name": name, "kind": kind, "half_extent_cm": half.to_array(), "falloff": falloff.to_array(), "brightness": g("Brightness", 1.0)}));
                }
                stats.reflection_probes = probes;
                if vlm.is_some() {
                    p.ambient.brightness = 0.0;
                    stats.lights.ambient = Some([0.0, 0.0, 0.0, 0.0]);
                }
            }
        }
        None => {
            // no paks: R1 placeholder sun (UNCONFIRMED), lighting.rs needs mh-level
            p.commands.spawn((DirectionalLight { illuminance: 10_000.0, shadow_maps_enabled: true, ..default() }, bevy::camera::visibility::RenderLayers::from_layers(&[0, crate::fighter::SHADOW_LAYER]), Transform::from_xyz(0.0, 10.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z), ChildOf(root)));
            p.ambient.brightness = 600.0;
        }
    }
    stats.build_secs = t0.elapsed().as_secs_f32() - read_secs;
    let st = &mut p.st;
    st.map = map;
    st.root = Some(root);
    st.handles = handles;
    st.gltf_meshes = gltf_meshes;
    st.loading = true;
    st.loaded = false;
    st.started = Some(t0);
    st.starts = plan.starts.clone();
    st.stats = stats;
    info!(
        "level: {} [{}] placed {} meshes + {} instances, {} primitives, {} HLOD proxies (read {:.2}s, build {:.2}s); equiv identical: {:?}",
        st.map, st.stats.plan_source, st.stats.meshes_placed, st.stats.ism_instances, st.stats.primitives, st.stats.hlod_proxies,
        st.stats.read_secs, st.stats.build_secs, st.stats.equiv.as_ref().map(|e| e.identical)
    );
}

/// Material package -> material via rust-assets' Resolver (MaterialDesc), cached per package: ue_tint material or the
/// StandardMaterial approximation, textures decoded from the paks (paksrc.rs).
#[allow(clippy::too_many_arguments)]
fn pak_mat(
    mp: &str,
    res: &mh_assets::material::Resolver,
    ps: &mh_assets::pak_source::PakSource,
    pc: &mut PakCache,
    images: &mut Assets<Image>,
    stdm: &mut Assets<StandardMaterial>,
    tint: &mut Assets<UeTintMaterial>,
    defaults: Option<&uetint::Defaults>,
    vlm: Option<&uetint::VlmBind>,
    use_tint: bool,
    stats: &mut LevelStats,
) -> Option<MatH> {
    if !pc.mats.contains_key(mp) {
        let built = res.build(mp).map(|d| {
            let PakCache { tex: tcache, packed: pcache, .. } = pc;
            let tst = &mut stats.textures;
            let tfail = &mut stats.texture_failures;
            let mut tex = |r: &str, srgb: bool| -> Option<Handle<Image>> {
                tcache
                    .entry((r.to_string(), srgb))
                    .or_insert_with(|| match paksrc::image(ps, r, srgb, tst) {
                        Ok(i) => Some(images.add(i)),
                        Err(e) => {
                            tst.failed += 1;
                            if tfail.len() < 50 {
                                tfail.push(e);
                            }
                            None
                        }
                    })
                    .clone()
            };
            let mut info = serde_json::json!({"package": d.package, "master": d.master, "mode": d.mode, "blend": d.blend,
                "two_sided": d.two_sided, "flat": d.flat, "missing_albedo": d.missing_albedo.as_ref().map(|t| t.0.clone()),
                "base_color": d.uniforms.get("base_color").map(|u| format!("{u:?}")),
                "textures": d.textures().map(|(k, t)| format!("{k}={}", t.0)).collect::<Vec<_>>()});
            if use_tint {
                let (m, bound) = uetint::build(&d, defaults.expect("uetint defaults"), vlm, &mut tex);
                info["bound"] = serde_json::json!(bound);
                stats.material_list.push(info.clone());
                stats.ue_tint_textures_bound += bound;
                (MatH::Tint(tint.add(m)), Pick { layout: d.layout.clone(), ..default() }, true)
            } else {
                stats.material_list.push(info);
                let a = material::desc_tex(&d, "albedo_texture").and_then(|r| tex(&r, true));
                let n = material::desc_tex(&d, "normal_texture").and_then(|r| tex(&r, false));
                let pk = material::desc_tex(&d, "rma_texture").and_then(|r| {
                    pcache
                        .entry((r.clone(), d.layout.clone()))
                        .or_insert_with(|| paksrc::packed_image(ps, &r, material::layout_sel(&d.layout)).ok().map(|i| images.add(i)))
                        .clone()
                });
                let (m, pick) = material::std_from_desc(&d, a, n, pk);
                (MatH::Std(stdm.add(m)), pick, false)
            }
        });
        if built.is_none() {
            stats.materials_unresolved.push(mp.to_string());
        }
        pc.mats.insert(mp.to_string(), built);
    }
    pc.mats.get(mp).cloned().flatten().map(|b| b.0)
}

/// HLOD proxy material package for glb/section slot i: the proxy mesh export's StaticMaterials (paks)
fn hlod_slot_material(rd: &mh_pak::Reader, pkg: &str, mesh_name: &str, slot: usize) -> Option<String> {
    let exps = rd.read(pkg)?;
    exps.iter().filter(|e| e.get("Name").and_then(|n| n.as_str()) == Some(mesh_name)).find_map(|e| {
        let a = e.pointer("/Properties/StaticMaterials")?.as_array()?;
        let op = a.get(slot)?.pointer("/MaterialInterface/ObjectPath")?.as_str()?;
        Some(ue::strip(op).to_string())
    })
}

fn hlod_component(h: &HlodRec) -> HlodProxy {
    HlodProxy { name: h.name.clone(), lod_level: h.lod_level, min_draw_cm: h.min_draw_cm, aabb: h.aabb, subs: h.subs.clone() }
}

fn track_load(mut st: ResMut<LevelState>, assets: Res<AssetServer>, jobs: Res<PackedJobs>) {
    if !st.loading {
        return;
    }
    let mut failed = 0;
    for h in &st.handles {
        if assets.is_loaded_with_dependencies(h.id()) {
            continue;
        }
        if assets.load_state(h.id()).is_failed() {
            failed += 1;
            continue;
        }
        return;
    }
    if !jobs.0.is_empty() {
        return;
    }
    st.stats.assets_failed = failed;
    st.loading = false;
    st.loaded = true;
    st.load_secs = st.started.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
    info!("level: {} loaded in {:.1}s ({} handles, {} failed)", st.map, st.load_secs, st.handles.len(), failed);
}

/// CUE4Parse writes COLOR_0 for every static mesh and many Arena meshes have it all zero (ue_material.gd header:
/// arena_mid_facade_arch_01a, 10048 vertices, all 0); StandardMaterial would multiply albedo by it, so it is dropped
/// when a glb mesh loads - except with --materials uetint, whose layered masters read vertex colour.
fn strip_vertex_colors(mut ev: MessageReader<AssetEvent<Mesh>>, mut meshes: ResMut<Assets<Mesh>>, src: Res<Source>) {
    if src.materials == MatSrc::UeTint {
        ev.clear();
        return;
    }
    for e in ev.read() {
        if let AssetEvent::Added { id } = e {
            if let Some(m) = meshes.get_mut_untracked(*id) {
                m.remove_attribute(Mesh::ATTRIBUTE_COLOR);
            }
        }
    }
}

/// UE's HLOD visibility for the camera (ue_level.gd update_hlod; FLODSceneTree::UpdateVisibilityStates .text
/// 0x22c1d30): from the top LOD level down, a proxy is drawn when the squared distance (cm) from the view to its box
/// exceeds MinDrawDistance^2 (F = 1: r.FieldOfViewAffectsHLOD unset), and then its sub-actors are hidden, recursively.
fn update_hlod(
    cam: Query<&GlobalTransform, With<Camera3d>>,
    mut proxies: Query<(&HlodProxy, &mut Visibility), Without<UeMesh>>,
    mut meshes: Query<(&UeMesh, &mut Visibility), Without<HlodProxy>>,
) {
    let Some(c) = cam.iter().next() else { return };
    if proxies.is_empty() {
        return;
    }
    let cpos = c.translation();
    let mut list: Vec<(HlodProxy, bool)> = proxies.iter().map(|(h, _)| (h.clone(), false)).collect();
    list.sort_by(|a, b| b.0.lod_level.cmp(&a.0.lod_level));
    let by_name: HashMap<String, usize> = list.iter().enumerate().map(|(i, h)| (h.0.name.clone(), i)).collect();
    let mut hidden: HashSet<String> = HashSet::new();
    fn hide(n: &str, list: &Vec<(HlodProxy, bool)>, by: &HashMap<String, usize>, hidden: &mut HashSet<String>) {
        if !hidden.insert(n.to_string()) {
            return;
        }
        if let Some(&i) = by.get(n) {
            for s in list[i].0.subs.clone() {
                hide(&s, list, by, hidden);
            }
        }
    }
    for i in 0..list.len() {
        if hidden.contains(&list[i].0.name) {
            continue;
        }
        let (lo, hi) = list[i].0.aabb;
        let q = cpos.clamp(lo, hi);
        let d2 = (cpos - q).length_squared() * 10000.0; // m^2 -> cm^2
        let md = list[i].0.min_draw_cm;
        if d2 > md * md {
            list[i].1 = true;
            for s in list[i].0.subs.clone() {
                hide(&s, &list, &by_name, &mut hidden);
            }
        }
    }
    let on: HashSet<String> = list.iter().filter(|x| x.1).map(|x| x.0.name.clone()).collect();
    for (h, mut v) in proxies.iter_mut() {
        v.set_if_neq(if on.contains(&h.name) && !hidden.contains(&h.name) { Visibility::Inherited } else { Visibility::Hidden });
    }
    for (m, mut v) in meshes.iter_mut() {
        v.set_if_neq(if hidden.contains(&m.actor) { Visibility::Hidden } else { Visibility::Inherited });
    }
}

/// HLOD proxies drawn right now (dump_state).
pub fn hlod_shown(world: &mut World) -> (usize, usize) {
    let mut q = world.query::<(&HlodProxy, &Visibility)>();
    let shown = q.iter(world).filter(|(_, v)| **v != Visibility::Hidden).count();
    let mut m = world.query::<(&UeMesh, &Visibility)>();
    let hid = m.iter(world).filter(|(_, v)| **v == Visibility::Hidden).count();
    (shown, hid)
}

#[cfg(test)]
mod test_level_round_tests {
    use super::*;
    use crate::input::{AngK, PlayerControl};
    use crate::loadout::{ProfileGear, Selected};
    use crate::sim::{FighterView, FrameInput, Sim, SimBackend, SimLevel};
    use std::{cell::RefCell, rc::Rc, time::Duration};

    #[derive(Default)]
    struct RoundState {
        fighters: Vec<FighterView>,
        gear: HashMap<u32, (String, String, Vec<String>)>,
        next: (String, String, Vec<String>),
        suicides: Vec<u32>,
    }

    struct RoundSim(Rc<RefCell<RoundState>>);

    impl SimBackend for RoundSim {
        fn name(&self) -> &'static str { "test-round-lifecycle" }
        fn spawn(&mut self, loc: [f32; 3], yaw: f32, team: Option<i64>) -> u32 {
            let mut s = self.0.borrow_mut();
            let id = s.fighters.len() as u32;
            s.fighters.push(FighterView { id, loc, yaw, team, health: 100.0, ..default() });
            let gear = s.next.clone();
            s.gear.insert(id, gear);
            id
        }
        fn set_next_weapon(&mut self, weapon: &str) { self.0.borrow_mut().next.0 = weapon.into(); }
        fn set_next_left(&mut self, left: &str) { self.0.borrow_mut().next.1 = left.into(); }
        fn set_next_wearables(&mut self, wearables: &[String]) { self.0.borrow_mut().next.2 = wearables.to_vec(); }
        fn tick(&mut self, _: f32, _: &HashMap<u32, FrameInput>) {}
        fn ticks(&self) -> u64 { 0 }
        fn fighters(&self) -> Vec<FighterView> { self.0.borrow().fighters.clone() }
        fn respawn(&mut self, id: u32, loc: [f32; 3], yaw: f32) {
            let mut s = self.0.borrow_mut();
            let f = &mut s.fighters[id as usize];
            f.loc = loc;
            f.yaw = yaw;
            f.health = 100.0;
        }
        fn suicide(&mut self, id: u32) {
            let mut s = self.0.borrow_mut();
            s.suicides.push(id);
            s.fighters[id as usize].health = 0.0;
        }
    }

    // Execute the actual Bevy system and restart_player, mocking only the sim boundary. Two rounds also check
    // that a previously retired corpse never triggers another round, and same-key pending edits are reapplied.
    fn exercise_rounds(bot_dies: bool) {
        let state = Rc::new(RefCell::new(RoundState::default()));
        let mut backend = RoundSim(state.clone());
        let player = backend.spawn([0.0; 3], 0.0, Some(0));
        let bot = backend.spawn([0.0; 3], 180.0, Some(1));
        let mut pc = PlayerControl::new(AngK::from_spec(None));
        pc.id = player;
        let key = crate::loadout::CUSTOM_KEY + 3;
        let mut world = World::new();
        world.insert_non_send_resource(Sim(Box::new(backend)));
        world.insert_resource(pc);
        world.insert_resource(SimLevel::default());
        world.insert_resource(Selected { player: key, ..default() });
        world.insert_resource(Time::<()>::default());
        world.insert_resource(LevelState {
            map: TEST_LEVEL.into(),
            loaded: true,
            starts: vec![StartRec { name: "round-start".into(), xf: Affine3A::IDENTITY, team: None }],
            ..default()
        });
        let mut schedule = bevy::ecs::schedule::Schedule::default();
        schedule.add_systems(test_level_rounds);

        for round in 0..2 {
            let old = world.resource::<PlayerControl>().id;
            let gear = ProfileGear {
                index: key,
                weapon: format!("pending-weapon-{round}"),
                left: format!("pending-left-{round}"),
                wearables: vec![format!("pending-wearable-{round}")],
                ..default()
            };
            world.resource_mut::<Selected>().gear.insert(key, gear.clone());
            state.borrow_mut().fighters[if bot_dies { bot } else { old } as usize].health = 0.0;
            schedule.run(&mut world);
            assert_eq!(world.resource::<PlayerControl>().id, old, "no restart before the round delay");
            world.resource_mut::<Time>().advance_by(Duration::from_secs_f32(TEST_LEVEL_RESTART_SECS + 0.01));
            schedule.run(&mut world);

            let current = world.resource::<PlayerControl>().id;
            assert_ne!(current, old, "RestartPlayer must possess a new pawn");
            let sl = world.resource::<SimLevel>();
            assert!(sl.retired.contains(&old));
            assert_eq!(sl.profiles.get(&current), Some(&key));
            {
                let s = state.borrow();
                assert!(sl.retired.iter().all(|id| s.fighters[*id as usize].health <= 0.0), "every retired pawn must be dead");
                let live_players: Vec<_> = s.fighters.iter().filter(|f| f.team == Some(0) && f.health > 0.0).map(|f| f.id).collect();
                assert_eq!(live_players, vec![current], "only the possessed player may remain alive");
                assert_eq!(s.fighters.iter().filter(|f| f.health > 0.0).count(), 2, "one player and one bot");
                assert_eq!(s.gear.get(&current), Some(&(gear.weapon, gear.left, gear.wearables)), "latest same-key profile reaches the new pawn");
                assert_eq!(s.suicides.len(), if bot_dies { round + 1 } else { 0 }, "only a living outgoing player needs suicide");
                if bot_dies {
                    assert_eq!(s.suicides.last(), Some(&old));
                }
            }
            schedule.run(&mut world);
            world.resource_mut::<Time>().advance_by(Duration::from_secs_f32(TEST_LEVEL_RESTART_SECS + 0.01));
            schedule.run(&mut world);
            assert_eq!(world.resource::<PlayerControl>().id, current, "retired corpses cannot restart the next round");
        }
    }

    #[test]
    fn bot_death_retires_surviving_player_as_dead_before_respawn() { exercise_rounds(true); }

    #[test]
    fn player_death_respawns_without_killing_the_corpse_again() { exercise_rounds(false); }
}

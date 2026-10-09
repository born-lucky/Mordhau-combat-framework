//! weapon.rs - held weapons (r4 item 2): the weapon's part meshes from the paks (mh-assets skeletal mesh LOD0, drawn
//! rigid: the weapon parts have one root bone, the weapon actor's space), materials through rust-assets' Resolver on
//! ue_tint, placed every frame at the sim's held-weapon actor transform (mh-sim Geometry::weapon_world: the
//! RightWeapon socket, the weapon's RightHandEquipOffset / rotation offset / grip, the same transform the traces use).
//!
//! Parts: AMordhauEquipment Skins[0].PartTypes[k].Parts[0] (the default part of each type: blade, guard, grip,
//! pommel ...) or the weapon's own SkeletalMesh (mh_sim::physics::weapon_mesh rules). The loadout's
//! FEquipmentCustomization.Parts choice is not applied (UNCONFIRMED: default parts).

#[path = "weapon_blood.rs"]
pub mod blood;

use crate::level::MatH;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::collections::HashMap;

/// Part meshes of a weapon class (UE paths): AMordhauEquipment Skins[skin].PartTypes[k].Parts[parts[k]] (an index out
/// of range -> 0, FEquipmentCustomization::Validate) or the weapon's own SkeletalMesh (mh_sim::physics::weapon_mesh)
pub fn weapon_part_meshes(rd: &mh_pak::Reader, weapon: &str, skin: i64, parts: &[i64]) -> Vec<String> {
    let d = rd.defaults(weapon);
    let strip = |s: &str| mh_sim::physics::strip(s);
    if let Some(m) = d.get("SkeletalMesh").and_then(|v| v["ObjectPath"].as_str()) {
        return vec![strip(m)];
    }
    let mut out = Vec::new();
    let skins = d.get("Skins").and_then(|v| v.as_array());
    let Some(sk) = skins.and_then(|a| a.get(skin.max(0) as usize).or_else(|| a.first())) else { return out };
    for (k, pt) in sk["PartTypes"].as_array().into_iter().flatten().enumerate() {
        let list = pt["Parts"].as_array();
        let i = parts.get(k).copied().unwrap_or(0).max(0) as usize;
        let Some(part) = list.and_then(|a| a.get(i).or_else(|| a.first())) else { continue };
        let cls = strip(part["ObjectPath"].as_str().unwrap_or(""));
        if cls.is_empty() {
            continue;
        }
        if let Some(m) = rd.defaults(&cls).get("SkeletalMesh").and_then(|v| v["ObjectPath"].as_str()) {
            out.push(strip(m));
        }
    }
    out
}

/// A weapon's drawable parts
#[derive(Clone)]
#[allow(dead_code)] // meshes: evidence / debugging
pub struct WeaponLook {
    pub prims: Vec<(Handle<Mesh>, MatH)>,
    pub meshes: Vec<String>,
}

#[derive(Resource, Default)]
pub struct WeaponCache {
    pub by_weapon: HashMap<String, Option<WeaponLook>>,
    pub info: Vec<serde_json::Value>,
}

/// a held weapon's part drawn with its own ue_tint instance (the smear uniforms)
#[derive(Component)]
pub struct SmearPart {
    pub fighter: u32,
    pub left: bool,
}

/// per weapon (fighter id): UpdateTrail's state and the weapon CDO's trail data
#[derive(Default)]
pub struct SmearState {
    pub st: mh_assets::weapon_trail::WeaponTrailState,
    /// (weapon class, alternate mode, TrailUp, TrailRight, DefaultTrailFactor) (alternate mode: SecondTrailUp /
    /// SecondTrailRight / SecondDefaultTrailFactor, RecalculateTracerPoints 0x163a940)
    pub cdo: Option<(String, bool, [f32; 3], [f32; 3], f32)>,
}

/// evidence of the smear: the largest TrailWeight x factor and motion seen, material writes
#[derive(Resource, Default, Clone, serde::Serialize)]
pub struct SmearStats {
    pub max_weight: f32,
    pub max_motion_cm: f32,
    pub writes: u64,
    /// Last material observation; cached sockets are not relabeled as newly produced endpoints.
    pub current: Vec<SmearProbe>,
}

#[derive(Clone, serde::Serialize)]
pub struct SmearProbe {
    pub fighter: u32,
    pub observed_sim_tick: u64,
    pub virtual_time: f64,
    pub component_position_ue_cm: [f32; 3],
    pub component_rotation_ue: [f32; 4],
    pub component_scale_ue: [f32; 3],
    pub cached_trace_start_ue_cm: Option<[f32; 3]>,
    pub cached_trace_end_ue_cm: Option<[f32; 3]>,
    pub trace_producer_tick: Option<u64>,
    pub material_trace_start_ue_cm: [f32; 3],
    pub material_trace_end_ue_cm: [f32; 3],
    pub stab: bool,
    pub input_weight: f32,
    pub computed_material_weight: f32,
    pub computed_material_motion_ue_cm: [f32; 3],
    /// Read back from present UeTint parts after every fighter's material writes; empty without those parts.
    pub material_motion_uniforms: Vec<[f32; 4]>,
}

/// AMordhauWeapon::UpdateTrail_Implementation 0x1641a50 every frame for each held (right-hand) weapon
/// (mh_assets::weapon_trail::update), written into its own materials' trail_* uniforms (ue_tint.wgsl
/// ue_tint_trail_offset, run by uetint.rs's vertex stage). TrailWeight = UAttackMotion::TrailWeight from the sim
/// (SimBackend::trail_weight: mh-sim Sim::trail_weight, rust-combat). TrailUp / TrailRight / DefaultTrailFactor from the
/// weapon CDO (AMordhauWeapon ctor rva 0x1610e30: TrailUp (0, 0, 1), TrailRight (0, 1, 0); factor: the BP's, e.g.
/// BP_Longsword 0.01; absent -> 0, UNCONFIRMED). The alternate mode uses the Second* values. TrailFactor's
/// lowering by the parts (RecalculateTracerPoints 0x163a940) is not applied (UNCONFIRMED).
#[allow(clippy::too_many_arguments)]
pub fn weapon_smear(
    sim: NonSend<crate::sim::Sim>,
    src: Res<crate::source::Source>,
    time: Res<Time>,
    weapons: Query<(&HeldWeapon, &GlobalTransform)>,
    parts: Query<(&SmearPart, &MeshMaterial3d<crate::uetint::UeTintMaterial>)>,
    mut mats: ResMut<Assets<crate::uetint::UeTintMaterial>>,
    mut state: Local<HashMap<u32, SmearState>>,
    mut stats: ResMut<SmearStats>,
) {
    stats.current.clear();
    let Some(vfs) = &src.vfs else { return };
    let ix = |n: &str| mh_assets::shader::uniform_index(n);
    let (Some(i_up), Some(i_right), Some(i_min), Some(i_motion)) = (ix("trail_mesh_up"), ix("trail_mesh_right"), ix("trail_min"), ix("trail_motion")) else { return };
    let dt = time.delta_secs();
    let mut params: HashMap<u32, mh_assets::weapon_trail::TrailParams> = HashMap::new();
    for (h, g) in weapons.iter() {
        if h.left {
            continue;
        }
        let Some(wp) = sim.0.weapon_path(h.fighter) else { continue };
        let Some((local_start, local_end)) = sim.0.weapon_trace_local(h.fighter) else { continue };
        let cached = sim.0.trace_sockets(h.fighter);
        let alt = sim.0.alternate_mode(h.fighter);
        let s = state.entry(h.fighter).or_default();
        if s.cdo.as_ref().is_none_or(|c| c.0 != wp || c.1 != alt) {
            let d = mh_pak::Reader::new(vfs.clone()).defaults(&wp);
            let v = |k: &str, def: [f32; 3]| d.get(k).map(|x| ["X", "Y", "Z"].map(|c| x.get(c).and_then(|y| y.as_f64()).unwrap_or(0.0) as f32)).unwrap_or(def);
            let p = if alt { "Second" } else { "" };
            // ctor rva 0x1610e30: TrailUp / SecondTrailUp (0, 0, 1), TrailRight / SecondTrailRight (0, 1, 0)
            let f = d.get(&format!("{p}DefaultTrailFactor")).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
            s.cdo = Some((wp.clone(), alt, v(&format!("{p}TrailUp"), [0.0, 0.0, 1.0]), v(&format!("{p}TrailRight"), [0.0, 1.0, 0.0]), f));
        }
        let (_, _, up, right, factor) = s.cdo.clone().unwrap();
        // the weapon component's world transform in UE terms (Bevy: SwapYZ, m -> cm)
        let (scale, rot, pos) = g.to_scale_rotation_translation();
        let q = crate::ue::quat([rot.x, rot.y, rot.z, rot.w]);
        let comp = mh_assets::weapon_trail::Xf { rot: [q.x, q.y, q.z, q.w], pos: [pos.x * 100.0, pos.z * 100.0, pos.y * 100.0], scale: [scale.x, scale.z, scale.y] };
        // UpdateTrail 0x1641a50 calls GetTrace (0x1629520) now, rather than the collision preparation cache.
        // The base motion's OverrideTrace is false; overriding motions remain an unported branch.
        let (ts, te) = (comp.transform(local_start), comp.transform(local_end));
        let stab = sim.0.combat().zip(sim.0.fighter_index(h.fighter)).and_then(|(w, fi)| w.cur_m(fi).and_then(|m| m.attack().map(|a| a.mv == 2 || a.mv == 3))).unwrap_or(false);
        let weight = sim.0.trail_weight(h.fighter).unwrap_or(0.0);
        let tp = mh_assets::weapon_trail::update(&mut s.st, weight, factor, stab, comp, ts, te, up, right, dt);
        stats.current.push(SmearProbe {
            fighter: h.fighter, observed_sim_tick: sim.0.ticks(), virtual_time: time.elapsed_secs_f64(),
            component_position_ue_cm: comp.pos, component_rotation_ue: comp.rot, component_scale_ue: comp.scale,
            cached_trace_start_ue_cm: cached.map(|x| x.0), cached_trace_end_ue_cm: cached.map(|x| x.1), trace_producer_tick: None,
            material_trace_start_ue_cm: ts, material_trace_end_ue_cm: te,
            stab, input_weight: weight,
            computed_material_weight: tp.trail_weight, computed_material_motion_ue_cm: tp.trail_motion,
            material_motion_uniforms: Vec::new(),
        });
        stats.max_weight = stats.max_weight.max(tp.trail_weight);
        let m = tp.trail_motion;
        stats.max_motion_cm = stats.max_motion_cm.max((m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt());
        params.insert(h.fighter, tp);
    }
    for (p, m) in parts.iter() {
        if p.left {
            continue;
        }
        let Some(tp) = params.get(&p.fighter) else { continue };
        let want = [
            Vec4::new(tp.mesh_up[0], tp.mesh_up[1], tp.mesh_up[2], 0.0),
            Vec4::new(tp.mesh_right[0], tp.mesh_right[1], tp.mesh_right[2], 0.0),
            Vec4::new(tp.trail_min_up, tp.trail_min_right, 0.0, 0.0),
            Vec4::new(tp.trail_motion[0], tp.trail_motion[1], tp.trail_motion[2], tp.trail_weight),
        ];
        let same = mats.get(&m.0).is_some_and(|mat| mat.u.v[i_up] == want[0] && mat.u.v[i_right] == want[1] && mat.u.v[i_min] == want[2] && mat.u.v[i_motion] == want[3]);
        if same {
            continue;
        }
        if let Some(mut mat) = mats.get_mut(&m.0) {
            mat.u.v[i_up] = want[0];
            mat.u.v[i_right] = want[1];
            mat.u.v[i_min] = want[2];
            mat.u.v[i_motion] = want[3];
            stats.writes += 1;
        }
    }
    for (part, material) in parts.iter().filter(|(part, _)| !part.left) {
        if let (Some(probe), Some(mat)) = (stats.current.iter_mut().find(|p| p.fighter == part.fighter), mats.get(&material.0)) {
            probe.material_motion_uniforms.push(mat.u.v[i_motion].to_array());
        }
    }
}

/// The weapon entity of a fighter
#[derive(Component)]
pub struct HeldWeapon {
    pub fighter: u32,
    /// the left-hand item (a shield on the LeftWeapon socket; fidelity-audit r5)
    pub left: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn build_look(
    weapon: &str,
    skin: i64,
    choice: &[i64],
    src: &crate::source::Source,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    tint: &mut Assets<crate::uetint::UeTintMaterial>,
    stdm: &mut Assets<StandardMaterial>,
    defaults: Option<&crate::uetint::Defaults>,
    paint: Option<&mh_assets::equipment_paint::EquipmentPaint>,
) -> (Option<WeaponLook>, serde_json::Value) {
    let (Some(vfs), Some(ps)) = (&src.vfs, &src.pak) else { return (None, serde_json::json!({"error": "no paks"})) };
    let rd = mh_pak::Reader::new(vfs.clone());
    let res = mh_assets::material::Resolver::new(&rd, &**ps);
    let parts = weapon_part_meshes(&rd, weapon, skin, choice);
    let mut prims = Vec::new();
    let mut failed = Vec::new();
    let mut st = crate::paksrc::TexStats::default();
    let mut blood_bindings = Vec::new();
    for p in &parts {
        let sm = match mh_assets::skeletal_mesh::lod0(&**ps, p) {
            Ok(sm) => sm,
            Err(e) => {
                failed.push(format!("{p}: {}", e.0));
                continue;
            }
        };
        for (mut m, mp) in crate::pak_fighter::mesh_parts(&sm) {
            // rigid: drawn without a skin (one root bone = the weapon actor's space)
            m.remove_attribute(Mesh::ATTRIBUTE_JOINT_INDEX);
            m.remove_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT);
            let mut d = if mp.is_empty() { None } else { res.build(&mp) };
            // AMordhauEquipment::UpdateMaterial 0x1572ed0: the loadout's colours / pattern / emblem on every material
            if let (Some(d), Some(p)) = (d.as_mut(), paint) {
                mh_assets::equipment_paint::apply(d, p);
            }
            let mat = match (d, defaults, src.materials) {
                (Some(d), Some(df), crate::source::MatSrc::UeTint) => {
                    let mut loaded = Vec::new();
                    let mut tex = |r: &str, srgb: bool| crate::paksrc::image(ps, r, srgb, &mut st).ok().map(|i| {
                        let format = format!("{:?}", i.texture_descriptor.format);
                        let h = images.add(i);
                        loaded.push((r.to_owned(), srgb, h.id(), format));
                        h
                    });
                    let mat = crate::uetint::build(&d, df, None, &mut tex).0;
                    let gpu = mh_assets::shader::gpu(&d);
                    for (slot, handle) in [(4, &mat.t4), (5, &mat.t5), (6, &mat.t6)] {
                        if let mh_assets::shader::SlotBinding::Texture { tex, srgb, .. } = &gpu.slots[slot] {
                            let actual = loaded.iter().find(|(path, colour, id, _)| path == &tex.0 && colour == srgb && *id == handle.id());
                            blood_bindings.push(serde_json::json!({"mesh": p, "material": mp, "slot": slot,
                                "path": tex.0, "srgb": srgb, "image": format!("{:?}", handle.id()),
                                "loaded": actual.is_some(), "format": actual.map(|x| &x.3)}));
                        }
                    }
                    MatH::Tint(tint.add(mat))
                }
                _ => MatH::Std(stdm.add(StandardMaterial { base_color: Color::srgb(0.6, 0.6, 0.6), metallic: 1.0, perceptual_roughness: 0.4, ..default() })),
            };
            prims.push((meshes.add(m), mat));
        }
    }
    let info = serde_json::json!({"weapon": weapon, "skin": skin, "choice": choice, "parts": parts, "primitives": prims.len(), "failed": failed, "textures": st, "blood_bindings": blood_bindings});
    ((!prims.is_empty()).then(|| WeaponLook { prims, meshes: parts }), info)
}

/// Actual SampleTracers swept point segments (0x163c430), consumed after the sim tick.
/// See state/proofs/frame_pose_alignment.md. No display-only blade interpolation or attack-stage gate:
/// a hit can end release in the same step that generated its final sweep.
#[derive(Resource, Default)]
pub struct Tracers {
    pub lines: Vec<TracerLine>,
    pub sampled: u64,
    pub last: Vec<crate::sim::TracerSegment>,
    world_generation: Option<u64>,
    last_world_time: Option<f64>,
}

pub struct TracerLine {
    pub start: Vec3,
    pub end: Vec3,
    pub expires_at: f64,
    pub emitted_at: f64,
    pub emitted_world_at: f64,
    pub fighter: u32,
    pub tick: u64,
    pub environment_only: bool,
}

fn ue_cm_to_bevy(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[2], v[1]) * 0.01
}

pub fn weapon_tracers(
    mut sim: NonSendMut<crate::sim::Sim>,
    us: Option<Res<crate::usersettings::UserSettings>>,
    time: Res<Time<Virtual>>,
    real_time: Res<Time<Real>>,
    mut tracers: ResMut<Tracers>,
) {
    let now = time.elapsed_secs_f64();
    let generation = sim.0.world_generation();
    if tracers.world_generation != Some(generation) || tracers.last_world_time.is_some_and(|t| now < t) {
        tracers.lines.clear();
        tracers.last.clear();
    }
    tracers.world_generation = Some(generation);
    tracers.last_world_time = Some(now);
    let sampled = sim.0.drain_tracers();
    tracers.sampled += sampled.len() as u64;
    tracers.last = sampled;
    let Some(us) = us.filter(|u| u.draw_tracers != 0) else {
        tracers.lines.clear();
        return;
    };
    // Original positive lifetimes decay with scaled world delta (ULineBatchComponent::TickComponent
    // 0x2fc06c0), not real seconds. f64 expiry is a bounded clock correction, not f32 tick-order parity.
    let stay = us.draw_tracers_stay_time.max(0.0) as f64;
    let lines: Vec<_> = tracers.last.iter().map(|t| TracerLine {
        start: ue_cm_to_bevy(t.start), end: ue_cm_to_bevy(t.end), expires_at: now + stay,
        emitted_at: real_time.elapsed_secs_f64(), emitted_world_at: now, fighter: t.fighter, tick: t.tick,
        environment_only: t.environment_only,
    }).collect();
    tracers.lines.extend(lines);
}

/// the alive tracer lines as one unlit LineList mesh (the custom post-process path keeps the gizmo pass out of the
/// final image, so the lines go through the ordinary mesh path; UE's DrawDebugLine thickness 0.5 is a 1 px line here)
pub fn draw_tracers(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    mut tracers: ResMut<Tracers>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut ent: Local<Option<(Entity, Handle<Mesh>)>>,
) {
    let now = time.elapsed_secs_f64();
    tracers.lines.retain(|l| l.expires_at > now);
    let (e, h) = match &*ent {
        Some(x) => x.clone(),
        None => {
            let h = meshes.add(Mesh::new(bevy::mesh::PrimitiveTopology::LineList, bevy::asset::RenderAssetUsages::default()).with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new()));
            let m = mats.add(StandardMaterial { base_color: Color::WHITE, unlit: true, ..default() });
            let e = commands.spawn((Name::new("WeaponTracers"), Mesh3d(h.clone()), MeshMaterial3d(m), Transform::IDENTITY, bevy::light::NotShadowCaster, bevy::light::NotShadowReceiver)).id();
            *ent = Some((e, h.clone()));
            (e, h)
        }
    };
    let _ = e;
    if let Some(mut m) = meshes.get_mut(&h) {
        let mut pts: Vec<[f32; 3]> = Vec::with_capacity(tracers.lines.len() * 2);
        let mut colors: Vec<[f32; 4]> = Vec::with_capacity(tracers.lines.len() * 2);
        for line in &tracers.lines {
            pts.push(line.start.to_array());
            pts.push(line.end.to_array());
            // SampleTracers163c430: cosmetic hand/environment BLUE;
            // ClientDrawTracer15c9a90: authoritative ordinary blade GREEN.
            // This is the combined local display, not a release-phase color split.
            let color = if line.environment_only { [0.0, 0.0, 1.0, 1.0] } else { [0.0, 1.0, 0.0, 1.0] };
            colors.extend([color; 2]);
        }
        if pts.is_empty() {
            // an empty LineList still needs a valid buffer: one degenerate line far below the level
            pts.push([0.0, -1.0e4, 0.0]);
            pts.push([0.0, -1.0e4, 0.0]);
            colors.extend([[1.0; 4]; 2]);
        }
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pts);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}

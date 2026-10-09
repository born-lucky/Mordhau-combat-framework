//! fighter.rs - FighterPlugin: the visible fighters (port target: godot/game/actor/fighter.gd + fighter_anim.gd;
//! asset rules from godot/components/ue/ue_anim.gd). R1: the UMA body with one idle clip. Each anim glb written by
//! `mdx anim` is UMA_Master + one glTF animation (ue_anim.gd header), so one file gives mesh, skeleton and clip.
//! Fighters are spawned and moved only through the sim bridge (sim.rs); this module never decides gameplay.
//!
//! R3: with a sim that poses its fighters (mh-sim: the pose its weapon traces run against), the pak body is skinned
//! with exactly that pose every frame (`pose_from_sim`): component-space UE bones -> local Y-up joint transforms by
//! bone name. The mesh sits under the actor (capsule centre, yaw) through the character BP's mesh component transform
//! the sim reports (mh-sim Geometry.mesh_xf); without one, the R1 placement (capsule bottom, yaw -90, UNCONFIRMED).
//! Body materials: rust-assets' Resolver -> the ue_tint material (`--materials uetint`) or StandardMaterial.

use crate::level::LevelState;
use crate::sim::{Sim, SpawnQueue};
use crate::ue;
use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeIndex};
use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use mh_assets::coords;
use serde_json::json;

/// The idle the Godot port's 1H sword stance starts from (RawClips/1H; fighter_anim.gd picks per weapon - R3).
pub const IDLE_GLB: &str = "Mordhau/Content/Mordhau/Animations/RawClips/1H/1H_RH_Idle_3p_var3_SwordNew.glb";

/// UE character capsule half height, cm (godot/game/actor/bot_world.gd:17: CapsuleRadius 50 / HalfHeight 96, BotBody).
/// The mesh origin is put at the capsule bottom: UNCONFIRMED until the character BP's Mesh RelativeLocation is read.
const CAPSULE_HALF_HEIGHT: f32 = 96.0;
/// Skeletal mesh component yaw relative to the actor: -90 as on BP_CrowdSystemActor's SkeletalMesh (ue_level.gd
/// crowd()); for the player character BP: UNCONFIRMED.
pub const MESH_YAW: f32 = -90.0;

#[derive(Resource)]
#[allow(dead_code)] // glb: kept for evidence / R3
pub struct FighterAssets {
    pub glb: String,
    /// the glb's body primitives (equiv_assets compares them with the pak-decoded body)
    pub body_prims: Vec<Handle<Mesh>>,
    pub scene: Handle<bevy::world_serialization::WorldAsset>,
    pub graph: Handle<AnimationGraph>,
    pub idle: AnimationNodeIndex,
}

#[derive(Component)]
pub struct Fighter {
    pub id: u32,
}

#[derive(Component)]
pub struct FighterReady;

pub struct FighterPlugin;

impl Plugin for FighterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::weapon::WeaponCache>()
            .init_resource::<crate::weapon::SmearStats>()
            .init_resource::<crate::weapon::Tracers>()
            .add_systems(Update, (crate::weapon::weapon_tracers, crate::weapon::draw_tracers).chain().after(crate::sim::tick_sim_frame))
            .add_systems(Update, sync_perspective.after(crate::input::player_input)
                .after(crate::devmenu::apply).after(crate::camera::toggle_mode).before(crate::sim::tick_sim_frame))
            .add_systems(Startup, load_assets)
            .add_systems(Update, (spawn_from_queue, bot_skins, sync_fighters, start_anim, pose_from_sim, held_weapons, spawn_shadow_capsules, first_person_parts).chain())
            // Material deformation consumes the held weapon's current propagated component transform.
            .add_systems(PostUpdate, crate::weapon::weapon_smear.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// The body + idle decoded from the paks (`--assets pak`, pak_fighter.rs), ready to instance per fighter.
#[derive(Resource)]
pub struct PakBody {
    /// (name, parent index, local rest transform, animation target)
    pub bones: Vec<(Name, i32, Transform, AnimationTargetId)>,
    pub ibp: Handle<SkinnedMeshInverseBindposes>,
    pub parts: Vec<(Handle<Mesh>, crate::level::MatH)>,
    /// per DefaultProfiles index: the loadout's parts (loadout.rs): meshes + materials, joint names, inverse bind poses
    pub skins: std::collections::HashMap<usize, Vec<(Vec<(Handle<Mesh>, crate::level::MatH)>, Vec<String>, Handle<SkinnedMeshInverseBindposes>, crate::loadout::View)>>,
    pub graph: Handle<AnimationGraph>,
    pub idle: AnimationNodeIndex,
    pub info: serde_json::Value,
}

#[allow(clippy::too_many_arguments)]
fn load_assets(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    paths: Res<crate::paths::Paths>,
    src: Res<crate::source::Source>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clips: ResMut<Assets<AnimationClip>>,
    mut ibps: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut stdm: ResMut<Assets<StandardMaterial>>,
    mut tint: ResMut<Assets<crate::uetint::UeTintMaterial>>,
    defaults: Option<Res<crate::uetint::Defaults>>,
    mut images: ResMut<Assets<Image>>,
    spec: Option<Res<crate::specdata::SpecData>>,
    sel: Res<crate::loadout::Selected>,
) {
    let clip = assets.load(GltfAssetLabel::Animation(0).from_asset(IDLE_GLB));
    let (graph, idle) = AnimationGraph::from_clip(clip);
    let n = crate::level::glb_materials(&paths.data.join(IDLE_GLB)).map(|v| v.len()).unwrap_or(0);
    commands.insert_resource(FighterAssets {
        glb: IDLE_GLB.into(),
        body_prims: (0..n).map(|i| assets.load(GltfAssetLabel::Primitive { mesh: 0, primitive: i }.from_asset(IDLE_GLB))).collect(),
        scene: assets.load(GltfAssetLabel::Scene(0).from_asset(IDLE_GLB)),
        graph: graphs.add(graph),
        idle,
    });
    if src.assets != crate::source::AssetSrc::Pak {
        return;
    }
    let (Some(vfs), Some(ps)) = (&src.vfs, &src.pak) else { return };
    let rd = mh_pak::Reader::new(vfs.clone());
    match crate::pak_fighter::build(ps, &rd, crate::pak_fighter::BODY_MESH, crate::pak_fighter::IDLE_ANIM) {
        Ok(f) => {
            // body materials: rust-assets' Resolver -> StandardMaterial (the wearable ue_tint path is R3)
            let res = mh_assets::material::Resolver::new(&rd, &**ps);
            let mut st = crate::paksrc::TexStats::default();
            let mut parts = Vec::new();
            for (m, mp) in f.parts {
                let d = if mp.is_empty() { None } else { res.build(&mp) };
                let mat = match d {
                    Some(d) => {
                        let mut tex = |r: &str, srgb: bool| crate::paksrc::image(ps, r, srgb, &mut st).ok().map(|i| images.add(i));
                        match (&defaults, src.materials) {
                            (Some(df), crate::source::MatSrc::UeTint) => {
                                crate::level::MatH::Tint(tint.add(crate::uetint::build(&d, df, None, &mut tex).0))
                            }
                            _ => {
                                let a = crate::material::desc_tex(&d, "albedo_texture").and_then(|r| tex(&r, true));
                                let nm = crate::material::desc_tex(&d, "normal_texture").and_then(|r| tex(&r, false));
                                crate::level::MatH::Std(stdm.add(crate::material::std_from_desc(&d, a, nm, None).0))
                            }
                        }
                    }
                    None => crate::level::MatH::Std(stdm.add(StandardMaterial::default())),
                };
                parts.push((meshes.add(m), mat));
            }
            // the loadouts of this run (BP_MordhauSingleton DefaultProfiles[i]: --profile / --bot-profile): face body
            // parts + wearables, one part set per profile
            let mut all_skins = std::collections::HashMap::new();
            let mut lo_infos = serde_json::Map::new();
            for &pi in &sel.list() {
                let (skins, lo_info) = build_profile_skins(pi, &mut SkinCtx {
                    vfs,
                    ps,
                    res: &res,
                    spec: spec.as_deref(),
                    defaults: defaults.as_deref(),
                    materials: src.materials,
                    meshes: &mut meshes,
                    images: &mut images,
                    tint: &mut tint,
                    stdm: &mut stdm,
                    ibps: &mut ibps,
                    st: &mut st,
                });
                all_skins.insert(pi, skins);
                lo_infos.insert(pi.to_string(), lo_info);
            }
            let bones = (0..f.bones.len())
                .map(|i| (Name::new(f.bones[i].name.clone()), f.bones[i].parent, f.bones[i].local, crate::pak_fighter::target_id(&f.bones, i)))
                .collect();
            let (graph, idle) = AnimationGraph::from_clip(clips.add(f.clip));
            let info = serde_json::json!({"mesh": crate::pak_fighter::BODY_MESH, "anim": crate::pak_fighter::IDLE_ANIM,
                "bones": f.bones.len(), "vertices": f.vertices, "parts": parts.len(), "clip_tracks": f.clip_tracks,
                "clip_tracks_unmatched": f.clip_tracks_unmatched, "textures": st, "loadouts": lo_infos});
            commands.insert_resource(PakBody { bones, ibp: ibps.add(SkinnedMeshInverseBindposes::from(f.inverse_bindposes)), parts, skins: all_skins, graph: graphs.add(graph), idle, info });
        }
        Err(e) => {
            error!("pak fighter: {e}");
        }
    }
}

pub type Skins = Vec<(Vec<(Handle<Mesh>, crate::level::MatH)>, Vec<String>, Handle<SkinnedMeshInverseBindposes>, crate::loadout::View)>;

/// what building a loadout's parts needs (load_assets at startup, bot_skins later for a roster bot's loadout)
pub struct SkinCtx<'a> {
    pub vfs: &'a std::sync::Arc<mh_pak::Vfs>,
    pub ps: &'a mh_assets::pak_source::PakSource,
    pub res: &'a mh_assets::material::Resolver<'a>,
    pub spec: Option<&'a crate::specdata::SpecData>,
    pub defaults: Option<&'a crate::uetint::Defaults>,
    pub materials: crate::source::MatSrc,
    pub meshes: &'a mut Assets<Mesh>,
    pub images: &'a mut Assets<Image>,
    pub tint: &'a mut Assets<crate::uetint::UeTintMaterial>,
    pub stdm: &'a mut Assets<StandardMaterial>,
    pub ibps: &'a mut Assets<SkinnedMeshInverseBindposes>,
    pub st: &'a mut crate::paksrc::TexStats,
}

/// One profile's (loadout::resolve key) face body parts + wearables as skinned part sets
pub fn build_profile_skins(key: usize, c: &mut SkinCtx) -> (Skins, serde_json::Value) {
            let mut skins = Vec::new();
            let mut lo_info = serde_json::Value::Null;
            if let Some(spec) = c.spec {
                let pk = mh_level::Pkgs::new(mh_pak::Reader::new(c.vfs.clone()));
                match crate::loadout::resolve(&pk, spec, key) {
                    Ok(lo) => {
                        let mut failed = Vec::new();
                        let mut mat_dump: Vec<serde_json::Value> = Vec::new();
                        let mut wear_mats = 0;
                        for part in &lo.parts {
                            let sp = match crate::pak_fighter::skin_part(c.ps, &part.mesh) {
                                Ok(sp) => sp,
                                Err(e) => {
                                    failed.push(format!("{}: {e}", part.mesh));
                                    continue;
                                }
                            };
                            let mut ms = Vec::new();
                            for (m, mp) in sp.meshes {
                                // wearable parts: the class's textures on the M_WEARABLE master (character_builder.gd _paint;
                                // masked = the slot material is a *_Masked instance); body parts: their slot material
                                let d = match &part.wearable {
                                    Some(cls) => c.res.wearable_from_class(cls, part.pattern, part.colors, mp.contains("_Masked")).inspect(|_| wear_mats += 1),
                                    None => None,
                                }
                                .or_else(|| if mp.is_empty() { None } else { c.res.build(&mp) });
                                if let Some(d) = &d {
                                    let g = |k: &str| d.uniforms.get(k).map(|u| format!("{u:?}"));
                                    mat_dump.push(serde_json::json!({"mesh": part.mesh, "class": part.wearable, "slot_material": mp,
                                        "pattern": part.pattern, "colors": part.colors, "mode": d.mode, "master": d.master, "blend": d.blend,
                                        "color_a": g("color_a"), "color_b": g("color_b"), "color_c": g("color_c"), "base_color": g("base_color"),
                                        "textures": d.textures().map(|(k, t)| format!("{k}={}", t.0)).collect::<Vec<_>>()}));
                                }
                                let mat = match d {
                                    Some(d) => {
                                        let mut tex = |r: &str, srgb: bool| crate::paksrc::image(c.ps, r, srgb, c.st).ok().map(|i| c.images.add(i));
                                        match (c.defaults, c.materials) {
                                            (Some(df), crate::source::MatSrc::UeTint) => crate::level::MatH::Tint(c.tint.add(crate::uetint::build(&d, df, None, &mut tex).0)),
                                            _ => {
                                                let a = crate::material::desc_tex(&d, "albedo_texture").and_then(|r| tex(&r, true));
                                                let nm = crate::material::desc_tex(&d, "normal_texture").and_then(|r| tex(&r, false));
                                                crate::level::MatH::Std(c.stdm.add(crate::material::std_from_desc(&d, a, nm, None).0))
                                            }
                                        }
                                    }
                                    None => crate::level::MatH::Std(c.stdm.add(StandardMaterial::default())),
                                };
                                ms.push((c.meshes.add(m), mat));
                            }
                            skins.push((ms, sp.joints, c.ibps.add(SkinnedMeshInverseBindposes::from(sp.inverse_bindposes)), part.view));
                        }
                        lo_info = serde_json::json!({"materials": mat_dump, "profile": lo.profile, "face": lo.face, "classes": lo.classes,
                            "parts": lo.parts, "parts_built": skins.len(), "wearable_materials": wear_mats, "failed": failed, "notes": lo.notes});
                    }
                    Err(e) => lo_info = serde_json::json!({"error": e}),
                }
            }
            (skins, lo_info)
}

/// a fighter whose profile (a roster bot's BotCharacterProfiles loadout) has no part set yet: build it now, before
/// sync_fighters instances the body
#[allow(clippy::too_many_arguments)]
fn bot_skins(
    pb: Option<ResMut<PakBody>>,
    sl: Res<crate::sim::SimLevel>,
    src: Res<crate::source::Source>,
    spec: Option<Res<crate::specdata::SpecData>>,
    defaults: Option<Res<crate::uetint::Defaults>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut tint: ResMut<Assets<crate::uetint::UeTintMaterial>>,
    mut stdm: ResMut<Assets<StandardMaterial>>,
    mut ibps: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    let Some(mut pb) = pb else { return };
    let missing: Vec<usize> = sl.profiles.values().copied().filter(|k| !pb.skins.contains_key(k)).collect();
    if missing.is_empty() {
        return;
    }
    let (Some(vfs), Some(ps)) = (&src.vfs, &src.pak) else { return };
    let rd = mh_pak::Reader::new(vfs.clone());
    let res = mh_assets::material::Resolver::new(&rd, &**ps);
    let mut st = crate::paksrc::TexStats::default();
    for key in missing {
        let (skins, info) = build_profile_skins(key, &mut SkinCtx {
            vfs,
            ps,
            res: &res,
            spec: spec.as_deref(),
            defaults: defaults.as_deref(),
            materials: src.materials,
            meshes: &mut meshes,
            images: &mut images,
            tint: &mut tint,
            stdm: &mut stdm,
            ibps: &mut ibps,
            st: &mut st,
        });
        if let Some(m) = pb.info.get_mut("loadouts").and_then(|l| l.as_object_mut()) {
            m.insert(key.to_string(), info);
        }
        pb.skins.insert(key, skins);
    }
}

/// The joints of one pak body, by mesh bone index (pose_from_sim writes them).
#[derive(Component)]
pub struct BodyJoints {
    pub joints: Vec<Entity>,
    pub parents: Vec<i32>,
    pub names: Vec<String>,
    pub rest: Vec<Transform>,
}

/// A loadout part's view (loadout::View) on the fighter it belongs to
#[derive(Component)]
pub struct ViewPart {
    pub fighter: u32,
    pub view: crate::loadout::View,
}

/// EVD_CAM_019 (state/proofs/fp_body_shadow.md): the real first-person body shadow is a UE capsule shadow.
/// UHumanMeshComponent::UpdateMeshVisibility rva=0x14ddc50 (decomp 4688-4726) clears bCastCapsuleDirectShadow only for
/// the third-person view target and the merged meshes carry ShadowPhysicsAsset UMA_Master_ShadowPhysicsAsset (decomp
/// 7303): in first person the directional shadow comes from its 41 capsules on 23 bones, not from the drawn mesh (which
/// has no head or torso). Port: capsule proxies under the joints on a shadow-only render layer (the camera excludes
/// SHADOW_LAYER, the directional lights include it), shown for the local first-person fighter; the drawn 1P parts stop
/// casting while they are shown. The table is the asset's SphylElems (shadow_capsules.json, UE cm, bone space).
pub const SHADOW_LAYER: usize = 7;

#[derive(Component)]
pub struct ShadowCapsule {
    pub fighter: u32,
}

#[derive(serde::Deserialize)]
struct CapsuleRow {
    bone: String,
    center: [f32; 3],
    rotation_pyr: [f32; 3],
    radius: f32,
    length: f32,
}

fn shadow_capsule_table() -> &'static [CapsuleRow] {
    static T: std::sync::OnceLock<Vec<CapsuleRow>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        // v1 distribution: extracted capsule geometry stays in the user's local cache.
        let root = std::env::var_os("MORDHAU_LOCAL_DATA").expect("local-import cache required");
        let path = std::path::PathBuf::from(root).join("shadow_capsules.json");
        let text = std::fs::read_to_string(&path).expect("locally imported shadow capsules missing");
        let v: serde_json::Value = serde_json::from_str(&text).expect("invalid local shadow capsule JSON");
        serde_json::from_value(v.get("capsules").cloned().expect("capsules missing"))
            .expect("invalid capsule records")
    })
}

/// the capsule proxies of a newly built body (one per SphylElem whose bone the skeleton has), hidden until the fighter
/// is the local first-person view
fn spawn_shadow_capsules(
    mut commands: Commands,
    bodies: Query<(Entity, &BodyJoints, &ChildOf), Added<BodyJoints>>,
    fighters: Query<&Fighter>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut mat: Local<Option<Handle<StandardMaterial>>>,
) {
    for (_, bj, parent) in bodies.iter() {
        let Ok(f) = fighters.get(parent.parent()) else { continue };
        let m = mat.get_or_insert_with(|| mats.add(StandardMaterial { base_color: Color::BLACK, unlit: true, ..default() })).clone();
        for c in shadow_capsule_table() {
            let Some(j) = bj.names.iter().position(|n| n == &c.bone) else { continue };
            let xf = mordhau_core::ue::FTransform::new(
                mordhau_core::ue::FQuat::from_rotator(c.rotation_pyr[0], c.rotation_pyr[1], c.rotation_pyr[2]),
                mordhau_core::ue::FVector::new(c.center[0], c.center[1], c.center[2]),
            );
            // FKSphylElem: Radius and the cylinder Length along the element's local Z (coords: UE Z -> Bevy Y, the
            // Capsule3d axis)
            let mesh = meshes.add(Capsule3d::new(c.radius * 0.01, c.length * 0.01));
            commands.spawn((
                Name::new(format!("ShadowCapsule {}", c.bone)),
                Mesh3d(mesh),
                MeshMaterial3d(m.clone()),
                ue_to_bevy(&xf),
                bevy::camera::visibility::RenderLayers::layer(SHADOW_LAYER),
                bevy::light::NotShadowReceiver,
                ShadowCapsule { fighter: f.id },
                Visibility::Hidden,
                ChildOf(bj.joints[j]),
            ));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CameraPoseState {
    id: u32,
    first_person: bool,
    is_view_target: bool,
    debug_override: bool,
}

fn camera_pose_state(id: u32, third_person: bool, mode: crate::camera::CamMode, fp_fly: bool) -> CameraPoseState {
    let player = mode == crate::camera::CamMode::Player;
    // fly1p is an external observer of the local 1P pose. Keep its synthetic target explicit; an ordinary fly
    // camera releases ownership. This diagnostic exception is not the native Become/EndViewTarget policy.
    let debug_override = !player && fp_fly && !third_person;
    CameraPoseState { id, first_person: !third_person && (player || fp_fly),
        is_view_target: player || debug_override, debug_override }
}

fn apply_camera_pose(last: &mut Option<u32>, now: Option<CameraPoseState>, mut apply: impl FnMut(CameraPoseState)) {
    if let Some(old) = *last {
        if now.map(|s| s.id) != Some(old) {
            apply(CameraPoseState { id: old, first_person: false, is_view_target: false, debug_override: false });
        }
    }
    if let Some(state) = now {
        // Reapply even when the tuple is unchanged: a new level or respawn can replace a fighter with the same ID.
        apply(state);
    }
    *last = now.map(|s| s.id);
}

#[cfg(test)]
mod view_target_tests {
    use super::*;
    use crate::camera::{CamMode, FpInFly};
    use crate::input::{AngK, PlayerControl};
    use crate::sim::{FighterView, FrameInput, SimBackend};
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    #[test]
    fn camera_ownership_is_distinct_from_perspective_and_labels_fly1p() {
        for (mode, third, fly1p, expected) in [
            (CamMode::Player, false, false, (true, true, false)),
            (CamMode::Player, true, false, (false, true, false)),
            (CamMode::Player, false, true, (true, true, false)),
            (CamMode::Player, true, true, (false, true, false)),
            (CamMode::Fly, false, false, (false, false, false)),
            (CamMode::Fly, true, false, (false, false, false)),
            (CamMode::Fly, false, true, (true, true, true)),
            (CamMode::Fly, true, true, (false, false, false)),
        ] {
            let s = camera_pose_state(7, third, mode, fly1p);
            assert_eq!((s.first_person, s.is_view_target, s.debug_override), expected);
            assert_eq!(s.id, 7);
        }
    }

    struct Probe {
        ids: Rc<RefCell<Vec<u32>>>,
        states: Rc<RefCell<HashMap<u32, (bool, bool, bool)>>>,
    }

    impl SimBackend for Probe {
        fn name(&self) -> &'static str { "camera ownership probe" }
        fn spawn(&mut self, _: [f32; 3], _: f32, _: Option<i64>) -> u32 { unreachable!() }
        fn tick(&mut self, _: f32, _: &HashMap<u32, FrameInput>) { unreachable!() }
        fn ticks(&self) -> u64 { 0 }
        fn fighters(&self) -> Vec<FighterView> {
            self.ids.borrow().iter().map(|&id| FighterView { id, ..Default::default() }).collect()
        }
        fn set_first_person(&mut self, id: u32, fp: bool) { self.states.borrow_mut().entry(id).or_default().0 = fp; }
        fn set_view_target(&mut self, id: u32, target: bool, debug_override: bool) {
            let mut states = self.states.borrow_mut();
            let state = states.entry(id).or_default();
            state.1 = target;
            state.2 = debug_override;
        }
    }

    #[test]
    fn host_sync_clears_old_owners_and_refreshes_reused_ids() {
        let ids = Rc::new(RefCell::new(vec![0, 1]));
        let states = Rc::new(RefCell::new(HashMap::new()));
        let mut world = World::new();
        world.insert_non_send_resource(Sim(Box::new(Probe { ids: ids.clone(), states: states.clone() })));
        let mut player = PlayerControl::new(AngK::from_spec(None));
        player.third_person = true;
        world.insert_resource(player);
        world.insert_resource(CamMode::Player);
        world.insert_resource(FpInFly(false));
        let mut schedule = Schedule::default();
        schedule.add_systems(sync_perspective);
        schedule.run(&mut world);
        assert_eq!(states.borrow()[&0], (false, true, false), "local third person owns the view target");
        states.borrow_mut().clear(); // a replacement backend/character can reuse the same external ID
        schedule.run(&mut world);
        assert_eq!(states.borrow()[&0], (false, true, false), "unchanged tuple must still reach the new character");
        world.resource_mut::<PlayerControl>().id = 1;
        schedule.run(&mut world);
        assert_eq!(states.borrow()[&0], (false, false, false), "clear old third-person target too");
        assert_eq!(states.borrow()[&1], (false, true, false));
        world.insert_resource(CamMode::Fly);
        schedule.run(&mut world);
        assert_eq!(states.borrow()[&1], (false, false, false));
        world.resource_mut::<PlayerControl>().third_person = false;
        world.insert_resource(FpInFly(true));
        schedule.run(&mut world);
        assert_eq!(states.borrow()[&1], (true, true, true), "fly1p explicitly retains local pose for observation");
        world.remove_resource::<PlayerControl>();
        schedule.run(&mut world);
        assert_eq!(states.borrow()[&1], (false, false, false));
        world.insert_resource(PlayerControl::new(AngK::from_spec(None)));
        world.insert_resource(CamMode::Player);
        schedule.run(&mut world);
        assert_eq!(states.borrow()[&0], (true, true, false));
        ids.borrow_mut().clear();
        schedule.run(&mut world);
        assert_eq!(states.borrow()[&0], (false, false, false), "a missing selected fighter clears the prior owner");
    }
}

/// BecomeViewTarget 0x1458e90 / EndViewTarget 0x14626c0 maintain camera ownership independently of CameraStyle.
/// CameraStyleChanged 0x1532190 changes perspective before NativeUpdateAnimation 0x1501930. Both flags must be
/// current before the sim tick; doing this in visibility after the tick samples the wrong pose.
fn sync_perspective(pc: Option<Res<crate::input::PlayerControl>>, mode: Res<crate::camera::CamMode>,
    fp_fly: Res<crate::camera::FpInFly>, mut sim: NonSendMut<crate::sim::Sim>, mut last: Local<Option<u32>>) {
    let now = pc.as_ref().filter(|p| sim.0.fighters().iter().any(|v| v.id == p.id))
        .map(|p| camera_pose_state(p.id, p.third_person, *mode, fp_fly.0));
    apply_camera_pose(&mut last, now, |state| {
        sim.0.set_first_person(state.id, state.first_person);
        sim.0.set_view_target(state.id, state.is_view_target, state.debug_override);
    });
}

/// UHumanMeshComponent::UpdateMeshVisibility rva=0x14ddc50: the view target's own character in first person draws
/// its FPMesh (no head, no torso, 1P overrides: loadout::View), every other character and 3P the UnifiedMesh
fn first_person_parts(
    mut commands: Commands,
    pc: Option<Res<crate::input::PlayerControl>>,
    mode: Option<Res<crate::camera::CamMode>>,
    fp_fly: Option<Res<crate::camera::FpInFly>>,
    mut q: Query<(Entity, &ViewPart, &mut Visibility, Has<bevy::light::NotShadowCaster>), Without<ShadowCapsule>>,
    mut caps: Query<(&ShadowCapsule, &mut Visibility), Without<ViewPart>>,
    sim: NonSend<crate::sim::Sim>,
) {
    let fp_of = pc.as_ref().filter(|p| !p.third_person && mode.as_deref().is_some_and(|m| *m == crate::camera::CamMode::Player || fp_fly.as_deref().is_some_and(|f| f.0))).map(|p| p.id);
    let dead: std::collections::HashSet<_> = sim.0.fighters().iter().filter(|v|v.health<=0.).map(|v|v.id).collect();
    for (e, vp, mut vis, no_cast) in q.iter_mut() {
        let fp = fp_of == Some(vp.fighter);
        let show = match vp.view {
            crate::loadout::View::Both => true,
            // UpdateMeshVisibility 0x14ddc50 selects the full FPDeadMesh for a dead first-person character.
            crate::loadout::View::Only3P => !fp || dead.contains(&vp.fighter),
            crate::loadout::View::Only1P => fp && !dead.contains(&vp.fighter),
        };
        let want = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        // bCastCapsuleDirectShadow (EVD_CAM_019): the capsules cast instead of the drawn 1P parts
        if fp != no_cast {
            if fp {
                commands.entity(e).insert(bevy::light::NotShadowCaster);
            } else {
                commands.entity(e).remove::<bevy::light::NotShadowCaster>();
            }
        }
    }
    for (c, mut vis) in caps.iter_mut() {
        let want = if fp_of == Some(c.fighter) { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
}

/// The mesh component node under a fighter (its transform = the sim's mesh_xf).
#[derive(Component)]
pub struct MeshRoot;

/// One fighter instanced from PakBody: a mesh root, joints (with animation targets) under it, skinned parts.
/// `animate`: no sim pose -> the decoded idle clip plays through Bevy's AnimationPlayer (R2 behaviour).
pub fn spawn_pak_body(commands: &mut Commands, pb: &PakBody, root: Entity, mesh_tr: Transform, animate: bool, profile: usize, fighter: u32) {
    let mroot = commands.spawn((MeshRoot, mesh_tr, Visibility::default(), ChildOf(root))).id();
    let mut joints: Vec<Entity> = Vec::with_capacity(pb.bones.len());
    for (name, parent, local, id) in &pb.bones {
        let par = if *parent >= 0 { joints[*parent as usize] } else { mroot };
        let mut e = commands.spawn((name.clone(), *local, Visibility::default(), ChildOf(par)));
        if animate {
            e.insert((*id, AnimatedBy(mroot)));
        }
        joints.push(e.id());
    }
    // loadout parts bound to the master joints by name (the master's own body mesh is then not drawn,
    // character_builder.gd: "master pose only; the visible body comes from the separated parts")
    let by_name: std::collections::HashMap<String, Entity> = pb.bones.iter().zip(joints.iter()).map(|(b, e)| (b.0.as_str().to_lowercase(), *e)).collect();
    let skins = pb.skins.get(&profile).or_else(|| pb.skins.values().next());
    for (ms, names, ibp, view) in skins.into_iter().flatten() {
        let js: Vec<Entity> = names.iter().map(|n| by_name.get(&n.to_lowercase()).copied().unwrap_or(mroot)).collect();
        for (m, mat) in ms {
            // 1P-only parts start hidden (3P is every non-local fighter's view); first_person_parts toggles them
            let vis = if *view == crate::loadout::View::Only1P { Visibility::Hidden } else { Visibility::Inherited };
            let mut c = commands.spawn((Mesh3d(m.clone()), SkinnedMesh { inverse_bindposes: ibp.clone(), joints: js.clone() }, Transform::default(), ChildOf(mroot), ViewPart { fighter, view: *view }, vis,
                // the posed arms of a 1P view lie outside the parts' bind-pose AABB (Bevy culls a skinned mesh by its
                // unskinned bounds): never frustum-cull character parts (rendering glue, fidelity-audit r3)
                bevy::camera::visibility::NoFrustumCulling));
            match mat {
                crate::level::MatH::Std(h) => c.insert(MeshMaterial3d(h.clone())),
                crate::level::MatH::Tint(h) => c.insert(MeshMaterial3d(h.clone())),
            };
        }
    }
    let master_parts: &[(Handle<Mesh>, crate::level::MatH)] = if skins.is_none_or(|v| v.is_empty()) { &pb.parts } else { &[] };
    for (m, mat) in master_parts {
        let mut c = commands.spawn((
            Mesh3d(m.clone()),
            SkinnedMesh { inverse_bindposes: pb.ibp.clone(), joints: joints.clone() },
            Transform::default(),
            ChildOf(mroot),
        ));
        match mat {
            crate::level::MatH::Std(h) => c.insert(MeshMaterial3d(h.clone())),
            crate::level::MatH::Tint(h) => c.insert(MeshMaterial3d(h.clone())),
        };
    }
    commands.entity(mroot).insert(BodyJoints {
        joints,
        parents: pb.bones.iter().map(|b| b.1).collect(),
        names: pb.bones.iter().map(|b| b.0.as_str().to_string()).collect(),
        rest: pb.bones.iter().map(|b| b.2).collect(),
    });
    if animate {
        let mut player = AnimationPlayer::default();
        player.play(pb.idle).repeat();
        commands.entity(mroot).insert((player, AnimationGraphHandle(pb.graph.clone())));
    }
    commands.entity(root).insert(FighterReady);
}

/// UE FTransform (cm, Z up) -> Y-up metres (mh_assets::coords rules)
pub fn ue_to_bevy(t: &mordhau_core::ue::FTransform) -> Transform {
    let q = coords::quat([t.rot.x, t.rot.y, t.rot.z, t.rot.w]);
    Transform { translation: Vec3::from(coords::pos([t.loc.x, t.loc.y, t.loc.z])), rotation: Quat::from_xyzw(q[0], q[1], q[2], q[3]), scale: Vec3::ONE }
}

/// Component-space UE pose (sim skeleton order) -> local Y-up joint transforms in mesh bone order.
pub fn local_pose(bj: &BodyJoints, pose_names: &[String], comp: &[mordhau_core::ue::FTransform]) -> Vec<Transform> {
    let idx: std::collections::HashMap<String, usize> = pose_names.iter().enumerate().map(|(i, n)| (n.to_lowercase(), i)).collect();
    let cg: Vec<Option<Transform>> = bj.names.iter().map(|n| idx.get(&n.to_lowercase()).and_then(|&k| comp.get(k)).map(ue_to_bevy)).collect();
    (0..bj.names.len())
        .map(|j| {
            let Some(c) = cg[j] else { return bj.rest[j] };
            let p = bj.parents[j];
            match (p >= 0).then(|| cg[p as usize]).flatten() {
                Some(pc) => {
                    let inv = pc.rotation.inverse();
                    Transform { translation: inv * (c.translation - pc.translation), rotation: inv * c.rotation, scale: Vec3::ONE }
                }
                None if p < 0 => c,
                // parent not in the sim's skeleton: keep this joint's rest transform
                None => bj.rest[j],
            }
        })
        .collect()
}

/// Every frame: skin each pak body with the pose its sim stepped with.
fn pose_from_sim(
    sim: NonSend<Sim>,
    frames: Res<crate::sim::SimFrames>,
    ftime: Res<Time<Fixed>>,
    fighters: Query<(&Fighter, &Children)>,
    roots: Query<&BodyJoints, With<MeshRoot>>,
    mut tr: Query<&mut Transform, Without<MeshRoot>>,
) {
    let Some(names) = sim.0.pose_bones() else { return };
    for (f, ch) in fighters.iter() {
        // the interpolated pose (sim.rs SimFrames), the sim's latest before its first fixed step
        let Some(pose) = frames.pose(f.id, &ftime).or_else(|| sim.0.pose_now(f.id)) else { continue };
        for c in ch.iter() {
            let Ok(bj) = roots.get(c) else { continue };
            for (j, t) in local_pose(bj, &names, &pose).into_iter().enumerate() {
                if let Ok(mut x) = tr.get_mut(bj.joints[j]) {
                    *x = t;
                }
            }
        }
    }
}

/// glTF-space transform -> UE (location cm, yaw deg): inverse of ue.rs (SwapYZ, * 0.01).
pub(crate) fn to_ue(xf: &bevy::math::Affine3A) -> ([f32; 3], f32) {
    let t = xf.translation;
    let fwd = xf.matrix3 * Vec3A::X; // UE +X in glb space
    ([t.x * 100.0, t.z * 100.0, t.y * 100.0], fwd.z.atan2(fwd.x).to_degrees())
}

pub fn yaw_quat(yaw: f32) -> Quat {
    ue::quat(ue::rot_quat(Some(&json!({ "Yaw": yaw }))))
}

/// Spawn requests wait for the level (they need its PlayerStarts); the sim is attached to the map first (mh-sim builds
/// the map's collision), then the fighters go to the sim round robin over the starts.
pub fn spawn_from_queue(
    mut q: ResMut<SpawnQueue>,
    mut bq: ResMut<crate::sim::BotQueue>,
    lvl: Res<LevelState>,
    mut sim: NonSendMut<Sim>,
    mut sl: ResMut<crate::sim::SimLevel>,
    src: Res<crate::source::Source>,
    mut sel: ResMut<crate::loadout::Selected>,
    spec: Option<Res<crate::specdata::SpecData>>,
) {
    if (q.0 == 0 && bq.0 == 0) || !lvl.loaded || lvl.starts.is_empty() {
        return;
    }
    if sl.map != lvl.map {
        let t0 = std::time::Instant::now();
        sl.error = sim.0.attach_level(src.vfs.as_ref(), &lvl.map).err();
        sl.secs = t0.elapsed().as_secs_f32();
        sl.map = lvl.map.clone();
        if let Some(e) = &sl.error {
            error!("sim attach: {e}");
        }
    }
    let have = sim.0.fighters().len();
    for k in 0..q.0 {
        let s = &lvl.starts[(have + k) % lvl.starts.len()];
        let (loc, yaw) = to_ue(&s.xf);
        if let Some(g) = sel.gear.get(&sel.player) {
            sim.0.set_next_weapon(&g.weapon);
            sim.0.set_next_wearables(&g.wearables);
            sim.0.set_next_left(&g.left);
        }
        let id = sim.0.spawn(loc, yaw, s.team);
        sl.profiles.insert(id, sel.player);
    }
    let have = sim.0.fighters().len();
    for k in 0..bq.0 {
        let s = &lvl.starts[(have + k) % lvl.starts.len()];
        let (loc, yaw) = to_ue(&s.xf);
        // a roster bot wears its BotCharacterProfiles loadout (AMordhauAIController::BeginPlay rva=0x14f0ba0,
        // rust-mode-ai's roster pick, previewed on the same rand() stream); else the --bot-profile DefaultProfiles one
        let prof = match sim.0.preview_bot_loadout() {
            Some(l) => {
                let key = crate::loadout::BOT_KEY + l;
                if !sel.gear.contains_key(&key) {
                    if let Some(v) = &src.vfs {
                        let pk = mh_level::Pkgs::new(mh_pak::Reader::new(v.clone()));
                        if let Ok(mut g) = crate::loadout::profile_gear(&pk, key) {
                            if let Some(sd) = spec.as_deref() {
                                if let Ok(lo) = crate::loadout::resolve(&pk, sd, key) {
                                    g.wearables = lo.classes.clone();
                                }
                            }
                            sel.gear.insert(key, g);
                        }
                    }
                }
                key
            }
            None => sel.bot_profile(sl.bots_spawned),
        };
        if let Some(g) = sel.gear.get(&prof) {
            sim.0.set_next_weapon(&g.weapon);
            sim.0.set_next_wearables(&g.wearables);
            sim.0.set_next_left(&g.left);
        }
        match sim.0.spawn_bot(loc, yaw, s.team) {
            Some(id) => {
                sl.profiles.insert(id, prof);
                sl.bots_spawned += 1;
            }
            None => warn!("this sim backend has no bots; spawned a plain fighter"),
        }
    }
    q.0 = 0;
    bq.0 = 0;
}

/// Sim fighter views -> entities (spawned on first sight, transform every frame).
fn sync_fighters(
    mut commands: Commands,
    sim: NonSend<Sim>,
    fa: Option<Res<FighterAssets>>,
    pb: Option<Res<PakBody>>,
    lvl: Res<LevelState>,
    sl: Res<crate::sim::SimLevel>,
    frames: Res<crate::sim::SimFrames>,
    ftime: Res<Time<Fixed>>,
    pc: Option<Res<crate::input::PlayerControl>>,
    windows: Query<(), With<Window>>,
    mut have: Query<(&Fighter, &mut Transform)>,
) {
    let Some(fa) = fa else { return };
    let views = sim.0.fighters();
    let mesh_xf = sim.0.mesh_xf();
    let mut seen = vec![false; views.len()];
    for (f, mut tr) in have.iter_mut() {
        if let Some(v) = views.get(f.id as usize) {
            seen[f.id as usize] = true;
            let mut drawn = frames.view(f.id, &ftime).unwrap_or_else(|| v.clone());
            // the local player's actor yaw is its control yaw of this frame (APawn::FaceRotation applies the control
            // rotation every rendered frame on the owning client; PlayerControl.yaw = the capped control yaw of
            // fidelity-audit's Turn port), not the last fixed step's
            if let Some(pc) = pc.as_ref().filter(|p| p.id == f.id && p.enabled && p.yaw_initialised && !windows.is_empty()) {
                drawn.yaw = pc.yaw;
            }
            *tr = actor_transform(&drawn, mesh_xf.is_some());
        }
    }
    for (i, v) in views.iter().enumerate() {
        if seen[i] {
            continue;
        }
        let mut e = commands.spawn((Fighter { id: v.id }, Name::new(format!("Fighter {}", v.id)), actor_transform(v, mesh_xf.is_some()), Visibility::default()));
        if let Some(r) = lvl.root {
            e.insert(ChildOf(r));
        }
        let id = e.id();
        let mesh_tr = mesh_xf.as_ref().map(ue_to_bevy).unwrap_or_else(|| Transform::from_rotation(yaw_quat(MESH_YAW)));
        match &pb {
            Some(pb) => {
                let prof = sl.profiles.get(&v.id).copied().unwrap_or(0);
                spawn_pak_body(&mut commands, pb, id, mesh_tr, sim.0.pose_bones().is_none(), prof, v.id)
            }
            None => {
                commands.entity(id).insert(WorldAssetRoot(fa.scene.clone()));
            }
        }
    }
}

/// The actor: capsule centre + yaw (mh-sim pose.rs actor_xf: ACharacter keeps pitch / roll at 0). Without a sim mesh
/// transform the R1 placement puts the actor at the capsule bottom.
pub(crate) fn actor_transform(v: &crate::sim::FighterView, centre: bool) -> Transform {
    // EVD_CAM_021: a crouched capsule (half height < the standing 96) carries its mesh HalfHeightAdjust higher
    // (AMordhauCharacter::OnStartCrouch rva=0x155bc20: Mesh RelativeLocation.Z = the default -97 + HalfHeightAdjust)
    let adjust = if v.half_height > 0.0 { CAPSULE_HALF_HEIGHT - v.half_height } else { 0.0 };
    let z = if centre { v.loc[2] + adjust } else { v.loc[2] - if v.half_height > 0.0 { v.half_height } else { CAPSULE_HALF_HEIGHT } };
    let p = Vec3::new(v.loc[0], z, v.loc[1]) * 0.01; // SwapYZ, cm -> m
    Transform::from_translation(p).with_rotation(yaw_quat(v.yaw))
}

/// Every AnimationPlayer the glb scene creates under a fighter gets the idle graph, looped.
fn start_anim(
    mut commands: Commands,
    fa: Option<Res<FighterAssets>>,
    mut players: Query<(Entity, &mut AnimationPlayer), (Added<AnimationPlayer>, Without<AnimationGraphHandle>)>,
    parents: Query<&ChildOf>,
    fighters: Query<(), With<Fighter>>,
) {
    let Some(fa) = fa else { return };
    for (e, mut p) in players.iter_mut() {
        p.play(fa.idle).repeat();
        commands.entity(e).insert(AnimationGraphHandle(fa.graph.clone()));
        let mut cur = e;
        while let Ok(c) = parents.get(cur) {
            cur = c.parent();
            if fighters.get(cur).is_ok() {
                commands.entity(cur).insert(FighterReady);
                break;
            }
        }
    }
}

/// Held weapons: one entity per fighter with a weapon, at the sim's weapon actor transform every frame (weapon.rs).
#[allow(clippy::too_many_arguments)]
fn held_weapons(
    mut commands: Commands,
    sim: NonSend<Sim>,
    lvl: Res<LevelState>,
    src: Res<crate::source::Source>,
    mut cache: ResMut<crate::weapon::WeaponCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut tint: ResMut<Assets<crate::uetint::UeTintMaterial>>,
    mut stdm: ResMut<Assets<StandardMaterial>>,
    defaults: Option<Res<crate::uetint::Defaults>>,
    sl: Res<crate::sim::SimLevel>,
    sel: Res<crate::loadout::Selected>,
    frames: Res<crate::sim::SimFrames>,
    ftime: Res<Time<Fixed>>,
    actors: Query<(&Fighter, &Transform), Without<crate::weapon::HeldWeapon>>,
    mut held: Query<(&crate::weapon::HeldWeapon, &mut Transform, &mut Visibility)>,
) {
    let views = sim.0.fighters();
    let mut have = vec![false; views.len()];
    let mut have_left = vec![false; views.len()];
    for (h, mut t, mut v) in held.iter_mut() {
        let slot = if h.left { &mut have_left } else { &mut have };
        if let Some(x) = slot.get_mut(h.fighter as usize) {
            *x = true;
        }
        let w = if h.left { sim.0.left_world(h.fighter) } else { frames.weapon(h.fighter, &ftime).or_else(|| sim.0.weapon_world(h.fighter)) };
        match w {
            Some(w) => {
                // the weapon follows the actor as drawn (interpolated / the local player's frame yaw): its pose
                // relative to the interpolated actor, re-placed on the drawn one
                let wt = ue_to_bevy(&w);
                let interp = frames.view(h.fighter, &ftime).map(|v| actor_transform(&v, sim.0.mesh_xf().is_some()));
                let drawn = actors.iter().find(|(f, _)| f.id == h.fighter).map(|(_, t)| *t);
                *t = match (interp, drawn) {
                    (Some(a), Some(d)) => Transform::from_matrix(d.to_matrix() * a.to_matrix().inverse() * wt.to_matrix()),
                    _ => wt,
                };
                v.set_if_neq(Visibility::Inherited);
            }
            None => {
                v.set_if_neq(Visibility::Hidden);
            }
        }
    }
    let mut todo: Vec<(u32, bool)> = Vec::new();
    for (i, f) in views.iter().enumerate() {
        if !have[i] {
            todo.push((f.id, false));
        }
        if !have_left[i] && sim.0.left_path(f.id).is_some() {
            todo.push((f.id, true));
        }
    }
    for (fid, left) in todo {
        let Some(wp) = (if left { sim.0.left_path(fid) } else { sim.0.weapon_path(fid) }) else { continue };
        // the loadout's skin / part choice when the fighter holds its profile's item
        let prof = sl.profiles.get(&fid).copied().unwrap_or(0);
        let (skin, choice, paint_in) = match sel.gear.get(&prof) {
            Some(g) if !left && g.weapon == wp => (g.skin, g.parts.clone(), Some((g.pattern, g.colors, g.emblem, g.emblem_colors))),
            Some(g) if left && g.left == wp => (g.left_skin, g.left_parts.clone(), Some((g.left_pattern, g.left_colors, g.emblem, g.emblem_colors))),
            _ => (0, Vec::new(), None),
        };
        let key = format!("{wp}|{skin}|{choice:?}|{paint_in:?}");
        if !cache.by_weapon.contains_key(&key) {
            // AMordhauEquipment::UpdateMaterial's colours / pattern / emblem (mh_assets::equipment_paint, rust-assets r11)
            let paint = match (&src.vfs, paint_in) {
                (Some(vfs), Some((pat, cols, em, emc))) => Some(mh_assets::equipment_paint::paint(&mh_pak::Reader::new(vfs.clone()), &wp, skin, pat, cols, em, emc)),
                _ => None,
            };
            let (look, info) = crate::weapon::build_look(&wp, skin, &choice, &src, &mut meshes, &mut images, &mut tint, &mut stdm, defaults.as_deref(), paint.as_ref());
            cache.info.push(info);
            cache.by_weapon.insert(key.clone(), look);
        }
        let Some(look) = cache.by_weapon.get(&key).cloned().flatten() else { continue };
        let mut e = commands.spawn((crate::weapon::HeldWeapon { fighter: fid, left }, Name::new(format!("{} {fid}", if left { "LeftHand" } else { "Weapon" })), Transform::default(), Visibility::Hidden));
        if let Some(r) = lvl.root {
            e.insert(ChildOf(r));
        }
        let id = e.id();
        for (m, mat) in &look.prims {
            let mut c = commands.spawn((Mesh3d(m.clone()), Transform::default(), ChildOf(id)));
            match mat {
                crate::level::MatH::Std(h) => c.insert(MeshMaterial3d(h.clone())),
                // the weapon's own material instance (UE: an MID per weapon) so its smear uniforms are its own
                crate::level::MatH::Tint(h) => {
                    let own = tint.get(h).cloned().map(|m| tint.add(m)).unwrap_or_else(|| h.clone());
                    c.insert((MeshMaterial3d(own), crate::weapon::SmearPart { fighter: fid, left }))
                }
            };
        }
    }
}

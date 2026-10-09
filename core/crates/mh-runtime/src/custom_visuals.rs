//! Authored training figures and weapons in the native held component frame. These primitives replace presentation only:
//! BodyJoints (including ragdoll output), weapon transforms, simulation and collision remain unchanged.
//! Dimensions/colors below are our design, not copied body or weapon mesh data. Weapon silhouettes
//! are generic training shapes, not a recreation of every original weapon's appearance.

use bevy::camera::visibility::{NoFrustumCulling, VisibilitySystems};
use bevy::prelude::*;
use crate::fighter::{BodyJoints, Fighter, ViewPart};
use crate::sim::Sim;
use crate::weapon::HeldWeapon;

#[derive(Resource, Default)]
pub struct CustomVisuals {
    /// Opt-in. The framework host may enable this after registering CustomVisualsPlugin.
    pub enabled: bool,
}

/// Only decoded body/held-weapon meshes carry this marker. Authored parts and the existing
/// locally decoded native FP shadow proxies never do. No shadow asset table is bundled here.
#[derive(Component)]
pub struct OriginalVisual;

#[derive(Component)]
pub struct CustomPart {
    owner: Entity,
    fighter: u32,
    kind: PartKind,
}

#[derive(Clone)]
enum PartKind {
    Limb { start: Entity, end: Entity, radius: f32, arm: bool },
    Joint { bone: Entity, radius: f32, arm: bool },
    Palm { wrist: Entity, knuckles: Vec<Entity> },
    FingerTip { bone: Entity, local_axis: Vec3, length: f32 },
    Blade,
    Guard,
    Grip,
    Pommel,
    Shaft,
    AxeHead,
    HammerHead,
    LeftPlate,
}

#[derive(Component)]
struct CustomBodyReady;
#[derive(Component)]
struct CustomWeaponReady;

#[derive(Resource)]
struct VisualAssets {
    capsule: Handle<Mesh>, sphere: Handle<Mesh>, blade: Handle<Mesh>,
    guard: Handle<Mesh>, grip: Handle<Mesh>, plate: Handle<Mesh>, palm: Handle<Mesh>,
    shaft: Handle<Mesh>, axe: Handle<Mesh>, hammer: Handle<Mesh>,
    steel: Handle<StandardMaterial>, ivory: Handle<StandardMaterial>, brass: Handle<StandardMaterial>,
}

pub struct CustomVisualsPlugin;
impl Plugin for CustomVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CustomVisuals>()
            .init_resource::<WeaponFrames>()
            .add_systems(Startup, assets)
            .add_systems(Update, (refresh_weapon_frames, spawn_bodies, spawn_weapons).chain())
            // Joints/held transforms are read only. Write leaf GlobalTransforms explicitly because
            // propagation already ran; visibility and renderer observe the new pose this frame.
            .add_systems(PostUpdate, (update_parts, original_visibility).chain()
                .after(bevy::transform::TransformSystems::Propagate)
                .before(VisibilitySystems::VisibilityPropagate)
                .before(VisibilitySystems::CalculateBounds));
    }
}

fn assets(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let material = |color: Color, metallic: f32| StandardMaterial {
        base_color: color, metallic, perceptual_roughness: 0.48, ..default()
    };
    commands.insert_resource(VisualAssets {
        // Capsule has total unit height 2, sphere radius 1. Leaf transforms size these authored primitives.
        capsule: meshes.add(Capsule3d::new(0.5, 1.0)), sphere: meshes.add(Sphere::new(1.0)),
        blade: meshes.add(authored_blade()),
        guard: meshes.add(Cuboid::new(0.20, 0.025, 0.035)),
        grip: meshes.add(Cylinder::new(0.014, 1.0)),
        shaft: meshes.add(Cylinder::new(0.018, 1.0)),
        axe: meshes.add(Cuboid::new(0.24, 0.28, 0.025)),
        hammer: meshes.add(Cuboid::new(0.18, 0.13, 0.13)),
        plate: meshes.add(Cuboid::new(0.38, 0.50, 0.03)),
        palm: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        steel: mats.add(material(Color::srgb(0.22, 0.34, 0.42), 0.65)),
        ivory: mats.add(material(Color::srgb(0.75, 0.79, 0.77), 0.25)),
        brass: mats.add(material(Color::srgb(0.76, 0.57, 0.25), 0.72)),
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeaponKind { Sword, Spear, Poleaxe, Axe, Hammer, Staff }

#[derive(Clone, Copy)]
struct WeaponFrame {
    start: Vec3,
    end: Vec3,
    grip_end: Vec3,
    handle_end_source: &'static str,
    head: Option<Vec3>,
    cutting_side: f32,
    kind: WeaponKind,
}

#[derive(Resource, Default)]
struct WeaponFrames {
    by_weapon: std::collections::HashMap<String, Option<WeaponFrame>>,
    fighter_weapon: std::collections::HashMap<u32, String>,
}
impl WeaponFrames {
    fn get(&self, fighter: u32) -> Option<&WeaponFrame> {
        self.by_weapon.get(self.fighter_weapon.get(&fighter)?)?.as_ref()
    }
}

fn weapon_kind(path: &str) -> Option<WeaponKind> {
    // Directory names describe animation families too: WarAxe lives in TwoHandedSword.
    // Choose the physical silhouette from the equipment class itself, never its folder.
    let path = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if ["halberd", "poleaxe", "billhook", "bardiche"].iter().any(|n| path.contains(n)) { Some(WeaponKind::Poleaxe) }
    else if ["spear", "pitchfork"].iter().any(|n| path.contains(n)) { Some(WeaponKind::Spear) }
    else if ["sword", "dagger", "knife", "rapier", "messer", "falchion", "falx", "estoc"].iter().any(|n| path.contains(n)) { Some(WeaponKind::Sword) }
    else if path.contains("axe") { Some(WeaponKind::Axe) }
    else if ["hammer", "maul", "mace", "eveningstar"].iter().any(|n| path.contains(n)) { Some(WeaponKind::Hammer) }
    else if ["staff", "club", "stick"].iter().any(|n| path.contains(n)) { Some(WeaponKind::Staff) }
    else { None }
}

fn refresh_weapon_frames(config: Res<CustomVisuals>, sim: NonSend<Sim>, src: Res<crate::source::Source>,
    held: Query<&HeldWeapon>, mut frames: ResMut<WeaponFrames>) {
    if !config.enabled { return; }
    let Some(vfs) = &src.vfs else { return };
    let rd = mh_pak::Reader::new(vfs.clone());
    frames.fighter_weapon.clear();
    for h in &held {
        if h.left { continue; }
        let Some(f) = sim.0.combat().and_then(|w| w.fighters.get(sim.0.fighter_index(h.fighter)?)) else { continue };
        let weapon = f.weapon_path.clone();
        frames.fighter_weapon.insert(h.fighter, weapon.clone());
        if frames.by_weapon.contains_key(&weapon) { continue; }
        let frame = (|| {
            let kind = weapon_kind(&weapon)?;
            let mesh = mh_sim::physics::weapon_mesh(&rd, &weapon)?;
            let sockets = mh_sim::physics::sockets(&rd, &mesh).ok()?;
            let point = |name: &str| sockets.iter().find(|(n, _)| n == name).map(|(_, s)|
                Vec3::new(s.xf.loc.x, s.xf.loc.z, s.xf.loc.y) * 0.01);
            let start = point("TraceStart")?;
            let end = point("TraceEnd")?;
            let (grip_end, handle_end_source) = if kind == WeaponKind::Sword || point("GripEnd").is_some() {
                physical_handle_end(start, end, point("GripEnd"), point("Part3"))?
            } else {
                // Short one-handed weapons often omit a rear socket. Supply an authored handle
                // behind the fixed normal trace base so the native hand grip remains on the shaft.
                (start - (end-start).try_normalize()? * 0.18, "authored rear handle extension")
            };
            let head = point("Part1");
            let cutting_side = point("StickyPoint").or_else(|| point("AdditionalTraceStart"))
                .map(|edge| (edge.z-head.unwrap_or(start).z).signum()).filter(|s| *s != 0.0).unwrap_or(-1.0);
            let frame = WeaponFrame { start, end, grip_end, handle_end_source, head, cutting_side, kind };
            segment(frame.start, frame.end, Vec3::Z)?;
            Some(frame)
        })();
        if frame.is_none() { warn!("No supported authored physical weapon frame for {weapon}; no substitute sword is drawn"); }
        frames.by_weapon.insert(weapon, frame);
    }
}

/// Some sword skeletons have no GripEnd (e.g. longsword). Their Part3 handle anchor supplies
/// the centre of our authored handle, reflected about the guard to define its bottom. This is
/// an authored span inferred from a native anchor, not a recovered original mesh boundary.
fn physical_handle_end(start: Vec3, end: Vec3, grip_end: Option<Vec3>, handle_part: Option<Vec3>)
    -> Option<(Vec3, &'static str)> {
    let (bottom, source) = if let Some(bottom) = grip_end {
        (bottom, "GripEnd")
    } else {
        (start + (handle_part? - start) * 2.0, "authored span about Part3")
    };
    (bottom.is_finite() && (bottom - start).dot(end - start) < 0.0).then_some((bottom, source))
}

/// Handcrafted double-edged blade with a diamond section, bevels and a pointed tip.
/// Vertex coordinates below are authored; no original mesh vertices are read or copied.
fn authored_blade() -> Mesh {
    use bevy::mesh::Indices;
    use bevy::render::render_resource::PrimitiveTopology;
    let mut positions = Vec::<[f32; 3]>::new();
    for (y, width, depth) in [(0.0, 0.5, 0.5), (0.72, 0.40, 0.38), (0.91, 0.20, 0.20)] {
        positions.extend([[-width,y,0.0],[0.0,y,depth],[width,y,0.0],[0.0,y,-depth]]);
    }
    positions.push([0.0,1.0,0.0]);
    let mut indices = vec![0,2,1,0,3,2];
    for ring in 0..2 {
        for i in 0..4 { let a=ring*4+i; let b=ring*4+(i+1)%4;
            indices.extend([a,b,a+4,b,b+4,a+4]); }
    }
    for i in 0..4 { indices.extend([8+i,8+(i+1)%4,12]); }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    mesh.duplicate_vertices(); mesh.compute_flat_normals();
    mesh
}

/// All parts are placed once in the physical weapon's local frame. UE +Y is the guard/width axis,
/// UE +Z is its long axis. After UE Z-up to Bevy Y-up conversion those become Bevy +Z and +Y.
fn weapon_part(frame: &WeaponFrame, kind: &PartKind) -> Option<Transform> {
    let (_, rotation, length) = segment(frame.start, frame.end, Vec3::Z)?;
    let along = rotation * Vec3::Y;
    let sword = frame.kind == WeaponKind::Sword;
    let spear = frame.kind == WeaponKind::Spear;
    let head = frame.head.unwrap_or(frame.end - along * 0.20);
    match kind {
        PartKind::Blade if sword || spear => {
            let base = if sword { frame.start } else { frame.end - along * length.min(0.28) };
            let (_, rotation, length) = segment(base, frame.end, Vec3::Z)?;
            Some(Transform { translation:base, rotation, scale:Vec3::new(if sword {0.055} else {0.040},length,0.006) })
        }
        PartKind::Guard if sword => Some(Transform { translation:frame.start, rotation, scale:Vec3::ONE }),
        PartKind::Grip if sword => {
            let (translation, rotation, length) = segment(frame.grip_end, frame.start, Vec3::Z)?;
            Some(Transform { translation, rotation, scale:Vec3::new(1.0,length,1.0) })
        }
        PartKind::Pommel if sword => Some(Transform::from_translation(frame.grip_end).with_scale(Vec3::splat(0.025))),
        PartKind::Shaft if !sword => {
            let end = if spear {frame.end-along*length.min(0.28)} else {frame.end-along*0.06};
            let (translation, rotation, length) = segment(frame.grip_end,end,Vec3::Z)?;
            Some(Transform { translation,rotation,scale:Vec3::new(1.0,length,1.0) })
        }
        PartKind::AxeHead if matches!(frame.kind,WeaponKind::Axe|WeaponKind::Poleaxe) =>
            Some(Transform { translation:head+rotation*Vec3::X*(0.09*frame.cutting_side),rotation,scale:Vec3::ONE }),
        PartKind::HammerHead if frame.kind==WeaponKind::Hammer =>
            Some(Transform { translation:head,rotation,scale:Vec3::ONE }),
        _ => None,
    }
}

fn spawn_part(commands: &mut Commands, owner: Entity, fighter: u32, kind: PartKind,
    mesh: Handle<Mesh>, material: Handle<StandardMaterial>) {
    commands.spawn((Name::new("Authored training visual"), CustomPart { owner, fighter, kind },
        Mesh3d(mesh), MeshMaterial3d(material), Transform::default(), GlobalTransform::default(),
        Visibility::Hidden, NoFrustumCulling, ChildOf(owner)));
}

fn spawn_bodies(mut commands: Commands, a: Res<VisualAssets>,
    bodies: Query<(Entity, &BodyJoints, &ChildOf), Without<CustomBodyReady>>, fighters: Query<&Fighter>) {
    // Named endpoints are pose references only. Radii are authored silhouette choices in metres.
    const LIMBS: &[(&str, &str, f32, bool)] = &[
        ("Hips", "Spine", 0.105, false), ("Spine", "Spine1", 0.135, false),
        ("Spine1", "Neck", 0.065, false),
        ("RightArm", "RightForeArm", 0.050, true), ("RightForeArm", "RightHand", 0.042, true),
        ("LeftArm", "LeftForeArm", 0.050, true), ("LeftForeArm", "LeftHand", 0.042, true),
        ("RightUpLeg", "RightLeg", 0.065, false), ("RightLeg", "RightFoot", 0.052, false),
        ("LeftUpLeg", "LeftLeg", 0.065, false), ("LeftLeg", "LeftFoot", 0.052, false),
    ];
    for (owner, b, parent) in &bodies {
        let Ok(f) = fighters.get(parent.parent()) else { continue };
        let bone = |name: &str| b.names.iter().position(|n| n.eq_ignore_ascii_case(name)).and_then(|i| b.joints.get(i).copied());
        for &(start, end, radius, arm) in LIMBS {
            let (Some(start), Some(end)) = (bone(start), bone(end)) else { continue };
            spawn_part(&mut commands, owner, f.id, PartKind::Limb { start, end, radius, arm },
                a.capsule.clone(), if arm { a.ivory.clone() } else { a.steel.clone() });
        }
        for &(name, radius, arm) in &[("Head", 0.105, false), ("Hips", 0.115, false),
            ("RightForeArm", 0.047, true), ("LeftForeArm", 0.047, true)] {
            let Some(bone) = bone(name) else { continue };
            spawn_part(&mut commands, owner, f.id, PartKind::Joint { bone, radius, arm }, a.sphere.clone(), a.brass.clone());
        }
        // Use actual hand descendants and their bind frames. No separate finger animation/clock is invented.
        for name in ["RightHand", "LeftHand"] {
            let Some(wrist_index) = b.names.iter().position(|n| n.eq_ignore_ascii_case(name)) else { continue };
            let digits: Vec<_> = b.names.iter().enumerate().filter_map(|(i, n)| {
                let n = n.to_ascii_lowercase();
                let digit = ["thumb", "index", "middle", "ring", "pinky", "little", "finger"].iter().any(|d| n.contains(d));
                let mut parent = b.parents[i];
                while parent >= 0 && parent as usize != wrist_index { parent = b.parents[parent as usize]; }
                (digit && parent == wrist_index as i32).then_some(i)
            }).collect();
            let knuckles: Vec<_> = digits.iter().filter(|&&i| b.parents[i] == wrist_index as i32)
                .map(|&i| b.joints[i]).collect();
            if knuckles.is_empty() { warn!("Authored hand has no digit roots: {name}"); continue; }
            spawn_part(&mut commands, owner, f.id, PartKind::Palm { wrist: b.joints[wrist_index], knuckles },
                a.palm.clone(), a.ivory.clone());
            for &i in &digits {
                let parent_index = b.parents[i] as usize;
                spawn_part(&mut commands, owner, f.id, PartKind::Joint { bone: b.joints[i], radius: 0.008, arm: true },
                    a.sphere.clone(), a.brass.clone());
                if parent_index != wrist_index {
                    spawn_part(&mut commands, owner, f.id, PartKind::Limb { start: b.joints[parent_index], end: b.joints[i], radius: 0.007, arm: true },
                        a.capsule.clone(), a.ivory.clone());
                }
                if !digits.iter().any(|&j| b.parents[j] == i as i32) {
                    // The terminal bone has no tip joint. Its bind direction supplies a small authored distal pad,
                    // rotated by that terminal bone's live pose, so it curls with the existing animation.
                    let local_axis = b.rest[i].rotation.inverse() * b.rest[i].translation.normalize_or_zero();
                    spawn_part(&mut commands, owner, f.id,
                        PartKind::FingerTip { bone: b.joints[i], local_axis, length: 0.018 }, a.capsule.clone(), a.ivory.clone());
                }
            }
        }
        commands.entity(owner).insert(CustomBodyReady);
    }
}

fn spawn_weapons(mut commands: Commands, a: Res<VisualAssets>, held: Query<(Entity, &HeldWeapon), Without<CustomWeaponReady>>) {
    for (owner, h) in &held {
        if h.left {
            spawn_part(&mut commands, owner, h.fighter, PartKind::LeftPlate, a.plate.clone(), a.steel.clone());
        } else {
            spawn_part(&mut commands, owner, h.fighter, PartKind::Blade, a.blade.clone(), a.steel.clone());
            spawn_part(&mut commands, owner, h.fighter, PartKind::Guard, a.guard.clone(), a.brass.clone());
            spawn_part(&mut commands, owner, h.fighter, PartKind::Grip, a.grip.clone(), a.ivory.clone());
            spawn_part(&mut commands, owner, h.fighter, PartKind::Pommel, a.sphere.clone(), a.brass.clone());
            spawn_part(&mut commands, owner, h.fighter, PartKind::Shaft, a.shaft.clone(), a.ivory.clone());
            spawn_part(&mut commands, owner, h.fighter, PartKind::AxeHead, a.axe.clone(), a.steel.clone());
            spawn_part(&mut commands, owner, h.fighter, PartKind::HammerHead, a.hammer.clone(), a.steel.clone());
        }
        commands.entity(owner).insert(CustomWeaponReady);
    }
}

/// Segment centre/orientation. Only the visual owns this threshold; it never feeds collision or traces.
fn segment(start: Vec3, end: Vec3, reference_x: Vec3) -> Option<(Vec3, Quat, f32)> {
    if !start.is_finite() || !end.is_finite() { return None; }
    let delta = end - start; let length = delta.length();
    if !length.is_finite() || length <= 0.00001 { return None; }
    let y = delta / length;
    let mut x = reference_x - y * reference_x.dot(y);
    if x.length_squared() <= 0.000001 {
        let fallback = if y.x.abs() < 0.8 { Vec3::X } else { Vec3::Z };
        x = fallback - y * fallback.dot(y);
    }
    x = x.normalize();
    let rotation = Quat::from_mat3(&Mat3::from_cols(x, y, x.cross(y))).normalize();
    Some(((start + end) * 0.5, rotation, length))
}

#[allow(clippy::too_many_arguments)]
fn update_parts(mut commands: Commands, config: Res<CustomVisuals>, sim: NonSend<Sim>, frames: Res<WeaponFrames>,
    roots: Query<&GlobalTransform, Without<CustomPart>>,
    pc: Option<Res<crate::input::PlayerControl>>, mode: Option<Res<crate::camera::CamMode>>,
    fp_fly: Option<Res<crate::camera::FpInFly>>,
    mut parts: Query<(Entity, &CustomPart, &mut Transform, &mut GlobalTransform, &mut Visibility, Has<bevy::light::NotShadowCaster>)>) {
    let fp = pc.as_ref().filter(|p| !p.third_person && mode.as_deref().is_some_and(|m|
        *m == crate::camera::CamMode::Player || fp_fly.as_deref().is_some_and(|f| f.0))).map(|p| p.id);
    let dead: std::collections::HashSet<_> = sim.0.fighters().into_iter().filter(|f| f.health <= 0.0).map(|f| f.id).collect();
    for (entity, part, mut local, mut global, mut vis, no_cast) in &mut parts {
        *vis = Visibility::Hidden;
        if !config.enabled { continue; }
        let Ok(parent) = roots.get(part.owner) else { continue };
        let desired = match part.kind.clone() {
            PartKind::Limb { start, end, radius, arm } => {
                // Arms-only FP is an authored presentation choice, not original full-body visibility parity.
                if fp == Some(part.fighter) && !dead.contains(&part.fighter) && !arm { continue; }
                let (Ok(start), Ok(end)) = (roots.get(start), roots.get(end)) else { continue };
                let Some((position, rotation, length)) = segment(start.translation(), end.translation(), Vec3::X) else { continue };
                Transform { translation: position, rotation, scale: Vec3::new(radius * 2.0, length * 0.5, radius * 2.0) }
            }
            PartKind::Joint { bone, radius, arm } => {
                if fp == Some(part.fighter) && !dead.contains(&part.fighter) && !arm { continue; }
                let Ok(bone) = roots.get(bone) else { continue };
                Transform::from_translation(bone.translation()).with_scale(Vec3::splat(radius))
            }
            PartKind::Palm { wrist, knuckles } => {
                let Ok(wrist) = roots.get(wrist) else { continue };
                let points: Vec<_> = knuckles.iter().filter_map(|&e| roots.get(e).ok().map(|g| g.translation())).collect();
                if points.is_empty() { continue; }
                let knuckle = points.iter().copied().sum::<Vec3>() / points.len() as f32;
                let across = points.last().copied().unwrap() - points[0];
                let Some((position, rotation, length)) = segment(wrist.translation(), knuckle, across) else { continue };
                Transform { translation: position, rotation, scale: Vec3::new(0.060, length.max(0.025), 0.025) }
            }
            PartKind::FingerTip { bone, local_axis, length } => {
                let Ok(bone) = roots.get(bone) else { continue };
                let start = bone.translation();
                let end = start + bone.compute_transform().rotation * local_axis * length;
                let Some((translation, rotation, length)) = segment(start, end, Vec3::X) else { continue };
                Transform { translation, rotation, scale: Vec3::new(0.014, length * 0.5, 0.014) }
            }
            PartKind::LeftPlate => {
                if sim.0.left_world(part.fighter).is_none() { continue; }
                parent.compute_transform()
            }
            kind => {
                let Some(frame) = frames.get(part.fighter) else { continue };
                // Fixed physical geometry in the native component frame. Alternate modes move that component
                // and the hands; they never rebuild a hilt at the active collision trace or active hand grip.
                let Some(relative) = weapon_part(frame, &kind) else { continue };
                Transform::from_matrix(parent.to_matrix() * relative.to_matrix())
            }
        };
        let matrix = desired.to_matrix();
        *local = Transform::from_matrix(parent.to_matrix().inverse() * matrix);
        *global = GlobalTransform::from(desired);
        // Native FP shadow proxies continue to cast, including death. Visible authored
        // FP parts must not add a second shadow; the 3P figure casts its own silhouette.
        let fp_body = fp == Some(part.fighter)
            && matches!(part.kind, PartKind::Limb { .. } | PartKind::Joint { .. } | PartKind::Palm { .. } | PartKind::FingerTip { .. });
        if fp_body != no_cast {
            if fp_body { commands.entity(entity).insert(bevy::light::NotShadowCaster); }
            else { commands.entity(entity).remove::<bevy::light::NotShadowCaster>(); }
        }
        *vis = Visibility::Inherited;
    }
}

fn original_visibility(config: Res<CustomVisuals>,
    mut originals: Query<(&mut Visibility, Option<&ViewPart>), (With<OriginalVisual>, Without<CustomPart>)>) {
    for (mut v, part) in &mut originals {
        if config.enabled { *v = Visibility::Hidden; }
        // ViewPart visibility is restored by first_person_parts earlier this frame.
        else if part.is_none() { *v = Visibility::Inherited; }
    }
}

/// Read-only evidence from the actual drawn transforms, in the physical weapon's local frame.
pub fn diagnostics(world: &mut World) -> serde_json::Value {
    let mut query = world.query::<(&CustomPart, &GlobalTransform, &Visibility)>();
    let mut fighters = std::collections::BTreeMap::<u32, serde_json::Value>::new();
    for (part, transform, visibility) in query.iter(world) {
        let row = fighters.entry(part.fighter).or_insert_with(|| serde_json::json!({
            "palms":0, "finger_joints":0, "finger_links":0, "finger_tips":0
        }));
        if *visibility != Visibility::Hidden {
            let label = match part.kind {
                PartKind::Blade => Some("blade"), PartKind::Guard => Some("guard"),
                PartKind::Grip => Some("handle"), PartKind::Pommel => Some("pommel"),
                PartKind::Shaft => Some("shaft"), PartKind::AxeHead => Some("axe_head"),
                PartKind::HammerHead => Some("hammer_head"), _ => None,
            };
            if let (Some(label), Some(parent)) = (label, world.get::<GlobalTransform>(part.owner)) {
                let drawn_local = parent.to_matrix().inverse() * transform.to_matrix();
                row["drawn_parts"][label] = serde_json::json!({
                    "local_matrix": drawn_local.to_cols_array(),
                    "base_local_m": drawn_local.transform_point3(Vec3::ZERO).to_array(),
                    "unit_tip_local_m": drawn_local.transform_point3(Vec3::Y).to_array(),
                    "bottom_local_m": drawn_local.transform_point3(-Vec3::Y * 0.5).to_array(),
                    "top_local_m": drawn_local.transform_point3(Vec3::Y * 0.5).to_array(),
                });
            }
        }
        let key = match part.kind {
            PartKind::Palm { .. } => Some("palms"),
            PartKind::FingerTip { .. } => Some("finger_tips"),
            PartKind::Joint { radius, .. } if radius <= 0.01 => Some("finger_joints"),
            PartKind::Limb { radius, .. } if radius <= 0.01 => Some("finger_links"),
            PartKind::Grip => {
                row["handle_center_metres"] = serde_json::json!(transform.translation().to_array());
                row["handle_visible"] = serde_json::json!(*visibility != Visibility::Hidden);
                None
            }
            PartKind::Guard => {
                if let Some(parent) = world.get::<GlobalTransform>(part.owner) {
                    let expected_across = parent.compute_transform().rotation * Vec3::Z; // UE +Y
                    let drawn_across = transform.compute_transform().rotation * Vec3::X;
                    row["guard_native_y_alignment"] = serde_json::json!(drawn_across.dot(expected_across).abs());
                }
                None
            }
            _ => None,
        };
        if let Some(key) = key { row[key] = (row[key].as_u64().unwrap_or(0) + 1).into(); }
    }
    if let Some(frames) = world.get_resource::<WeaponFrames>() {
        for (fighter, row) in &mut fighters {
            if let Some(frame) = frames.get(*fighter) {
                row["fixed_frame"] = serde_json::json!({ "kind":format!("{:?}",frame.kind),
                    "blade_base_local_m":frame.start.to_array(),"blade_tip_local_m":frame.end.to_array(),
                    "handle_bottom_local_m":frame.grip_end.to_array(),"handle_end_source":frame.handle_end_source,
                    "width_axis_local_m":Vec3::Z.to_array() });
            }
        }
    }
    serde_json::json!(fighters)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_kind_uses_equipment_class_not_its_animation_family_folder() {
        assert_eq!(weapon_kind("Specific/TwoHandedSword/BP_WarAxe"),Some(WeaponKind::Axe));
        assert_eq!(weapon_kind("Specific/TwoHandedSword/BP_2Hmace"),Some(WeaponKind::Hammer));
        assert_eq!(weapon_kind("Specific/TwoHandedSword/BP_Greatsword"),Some(WeaponKind::Sword));
        assert_eq!(weapon_kind("Specific/Polearms/BP_Halberd"),Some(WeaponKind::Poleaxe));
        assert_eq!(weapon_kind("Specific/TwoHandedSword/BP_Unknown"),None);
    }
    fn sword_frame() -> WeaponFrame {
        // Synthetic test sockets; actual sockets are loaded locally from the owned paks.
        WeaponFrame { start:Vec3::ZERO,end:Vec3::Y*1.2,grip_end:-Vec3::Y*0.3,
            handle_end_source:"GripEnd",head:None,cutting_side:-1.0,kind:WeaponKind::Sword }
    }
    #[test]
    fn missing_grip_end_uses_handle_anchor_and_never_the_active_alternate_trace() {
        let start = Vec3::ZERO; let end = Vec3::Y;
        let (bottom, source) = physical_handle_end(start,end,None,Some(-Vec3::Y*0.10)).unwrap();
        assert_eq!(bottom,-Vec3::Y*0.20);
        assert_eq!(source,"authored span about Part3");
        assert!(physical_handle_end(start,end,None,None).is_none());
        assert!(physical_handle_end(start,end,None,Some(Vec3::Y*0.10)).is_none());
        assert_eq!(physical_handle_end(start,end,Some(-Vec3::Y*0.30),Some(-Vec3::Y*0.10)).unwrap().0,-Vec3::Y*0.30);
    }
    #[test]
    fn guard_and_blade_width_use_native_y_and_full_hilt_reaches_grip_end() {
        let frame = sword_frame();
        let guard = weapon_part(&frame,&PartKind::Guard).unwrap();
        assert!((guard.rotation*Vec3::X-Vec3::Z).length()<1e-6,"UE Y maps to Bevy Z, not X");
        let blade = weapon_part(&frame,&PartKind::Blade).unwrap();
        assert!((blade.transform_point(Vec3::Y)-frame.end).length()<1e-6);
        let grip = weapon_part(&frame,&PartKind::Grip).unwrap();
        assert!((grip.transform_point(Vec3::Y*0.5)-frame.start).length()<1e-6);
        assert!((grip.transform_point(-Vec3::Y*0.5)-frame.grip_end).length()<1e-6);
    }
    #[test]
    fn changing_hand_grip_moves_one_rigid_weapon_without_rebuilding_its_parts() {
        let frame = sword_frame();
        let normal = Transform::from_rotation(Quat::from_rotation_x(0.4));
        let alternate = Transform::from_translation(Vec3::new(0.2,-0.3,0.5))
            .with_rotation(Quat::from_rotation_z(-0.7));
        for kind in [PartKind::Blade,PartKind::Guard,PartKind::Grip,PartKind::Pommel] {
            let part = weapon_part(&frame,&kind).unwrap().to_matrix();
            for parent in [normal,alternate] {
                let world = parent.to_matrix()*part;
                let recovered = parent.to_matrix().inverse()*world;
                assert!(recovered.abs_diff_eq(part,1e-6));
            }
        }
    }
    #[test]
    fn visual_axis_reaches_both_live_endpoints_and_retains_roll_reference() {
        let start = Vec3::new(0.8, 1.5, -0.4); let end = Vec3::new(-0.2, 2.2, 0.6);
        let (centre, q, length) = segment(start, end, Vec3::Z).unwrap();
        assert!((centre - q * Vec3::Y * (length * 0.5) - start).length() < 0.000001);
        assert!((centre + q * Vec3::Y * (length * 0.5) - end).length() < 0.000001);
        let axis = (end-start).normalize();
        assert!((q * Vec3::X - (Vec3::Z-axis*Vec3::Z.dot(axis)).normalize()).length() < 0.000001);
    }
    #[test]
    fn invalid_visual_segment_hides_instead_of_fabricating_a_weapon() {
        assert!(segment(Vec3::ZERO, Vec3::ZERO, Vec3::X).is_none());
        assert!(segment(Vec3::ZERO, Vec3::splat(f32::NAN), Vec3::X).is_none());
        assert!(segment(Vec3::ZERO, Vec3::Y, Vec3::Y).unwrap().1.is_finite());
    }
}

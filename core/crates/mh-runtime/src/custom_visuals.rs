//! Authored training figures and trace-aligned weapons. These primitives replace presentation only:
//! BodyJoints (including ragdoll output), weapon transforms, simulation and collision remain unchanged.
//! Dimensions/colors below are our design, not copied body or weapon mesh data. Weapon silhouettes
//! are generic training blades/plates, not a recreation of every original weapon's appearance.

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
    steel: Handle<StandardMaterial>, ivory: Handle<StandardMaterial>, brass: Handle<StandardMaterial>,
}

pub struct CustomVisualsPlugin;
impl Plugin for CustomVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CustomVisuals>()
            .add_systems(Startup, assets)
            .add_systems(Update, (spawn_bodies, spawn_weapons))
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
        blade: meshes.add(Cuboid::new(0.055, 1.0, 0.014)),
        guard: meshes.add(Cuboid::new(0.20, 0.025, 0.035)),
        grip: meshes.add(Cuboid::new(0.032, 0.18, 0.032)),
        plate: meshes.add(Cuboid::new(0.38, 0.50, 0.03)),
        palm: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        steel: mats.add(material(Color::srgb(0.22, 0.34, 0.42), 0.65)),
        ivory: mats.add(material(Color::srgb(0.75, 0.79, 0.77), 0.25)),
        brass: mats.add(material(Color::srgb(0.76, 0.57, 0.25), 0.72)),
    });
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
            spawn_part(&mut commands, owner, h.fighter, PartKind::Blade, a.blade.clone(), a.ivory.clone());
            spawn_part(&mut commands, owner, h.fighter, PartKind::Guard, a.guard.clone(), a.brass.clone());
            spawn_part(&mut commands, owner, h.fighter, PartKind::Grip, a.grip.clone(), a.steel.clone());
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
fn update_parts(mut commands: Commands, config: Res<CustomVisuals>, sim: NonSend<Sim>,
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
                let Some(trace) = sim.0.current_weapon_trace(part.fighter) else { continue };
                // Endpoints are current active-grip sockets in the drawn held-mesh component, not stale
                // attack history or a separate invented weapon orientation.
                let to_m = |p: [f32; 3]| Vec3::new(p[0], p[2], p[1]) * 0.01;
                let (start, end) = (parent.transform_point(to_m(trace.start_local_ue_cm)), parent.transform_point(to_m(trace.end_local_ue_cm)));
                let Some((_centre, rotation, _length)) = segment(start, end, parent.compute_transform().rotation * Vec3::X) else { continue };
                let direction = rotation * Vec3::Y;
                // Collision start is not a grip socket. Centre the authored handle at GripLocationLocal in
                // the same interpolated held component, so the fingers and sword share the drawn pose.
                let grip = parent.transform_point(to_m(trace.grip_local_ue_cm));
                let guard = grip + direction * 0.105;
                match kind {
                    PartKind::Blade => {
                        let Some((translation, rotation, length)) = segment(guard, end, rotation * Vec3::X) else { continue };
                        Transform { translation, rotation, scale: Vec3::new(1.0, length, 1.0) }
                    }
                    PartKind::Guard => Transform { translation: guard, rotation, scale: Vec3::ONE },
                    PartKind::Grip => Transform { translation: grip, rotation, scale: Vec3::ONE },
                    _ => unreachable!(),
                }
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

/// Read-only presentation evidence for captures: which authored parts exist and where the handle sits.
pub fn diagnostics(world: &mut World) -> serde_json::Value {
    let mut query = world.query::<(&CustomPart, &GlobalTransform, &Visibility)>();
    let mut fighters = std::collections::BTreeMap::<u32, serde_json::Value>::new();
    for (part, transform, visibility) in query.iter(world) {
        let row = fighters.entry(part.fighter).or_insert_with(|| serde_json::json!({
            "palms":0, "finger_joints":0, "finger_links":0, "finger_tips":0
        }));
        let key = match part.kind {
            PartKind::Palm { .. } => Some("palms"),
            PartKind::FingerTip { .. } => Some("finger_tips"),
            PartKind::Joint { radius, .. } if radius <= 0.01 => Some("finger_joints"),
            PartKind::Limb { radius, .. } if radius <= 0.01 => Some("finger_links"),
            PartKind::Grip => {
                row["grip_metres"] = serde_json::json!(transform.translation().to_array());
                row["grip_visible"] = serde_json::json!(*visibility != Visibility::Hidden);
                if let (Some(parent), Some(trace)) = (world.get::<GlobalTransform>(part.owner),
                    world.non_send::<Sim>().0.current_weapon_trace(part.fighter)) {
                    let g = trace.grip_local_ue_cm;
                    let expected = parent.transform_point(Vec3::new(g[0], g[2], g[1]) * 0.01);
                    row["grip_error_metres"] = serde_json::json!(transform.translation().distance(expected));
                }
                None
            }
            _ => None,
        };
        if let Some(key) = key { row[key] = (row[key].as_u64().unwrap_or(0) + 1).into(); }
    }
    serde_json::json!(fighters)
}

#[cfg(test)]
mod tests {
    use super::*;
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

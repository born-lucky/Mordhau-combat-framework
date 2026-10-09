//! Round 6: collision defaults read from the exe and the mirrored-landscape cook order. SKIP (pass) without the install.

use mh_level::collision::{class_default, CollisionWorld, Resp};
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::sync::Arc;

fn pkgs() -> Option<Pkgs> {
    Vfs::mount_default().ok().map(|v| Pkgs::new(Reader::new(Arc::new(v))))
}

/// Constructor defaults (exe) applied: BlockingVolume = InvisibleWall, CameraBlockingVolume = OverlapAll + Camera
/// Block unless the level overrides, PhysicsVolume subclasses OverlapAllDynamic (APhysicsVolume ctor 0x332e830),
/// info volumes NoCollision (e.g. ALightmassImportanceVolume 0x31b3ff0), TriggerVolume Trigger, other volumes
/// AVolume's OverlapAll, StaticMeshActor components through their mesh's
/// DefaultInstance
#[test]
fn class_defaults_from_the_exe() {
    assert_eq!(class_default("BlockingVolume").profile, "InvisibleWall");
    assert_eq!(class_default("CameraBlockingVolume").profile, "OverlapAll");
    assert_eq!(class_default("CameraBlockingVolume").extra, &[("Camera", Resp::Block)]);
    assert_eq!(class_default("PainCausingVolume").profile, "OverlapAllDynamic");
    assert_eq!(class_default("LightmassImportanceVolume").profile, "NoCollision");
    assert_eq!(class_default("PostProcessVolume").profile, "NoCollision");
    assert_eq!(class_default("TriggerVolume").profile, "Trigger");
    assert_eq!(class_default("MeshMergeCullingVolume").profile, "OverlapAll");
    assert_eq!(class_default("SplineMeshComponent").profile, "NoCollision");
    assert_eq!(class_default("StaticMeshComponent").profile, "BlockAllDynamic");
    assert_eq!(class_default("LandscapeHeightfieldCollisionComponent").profile, "BlockAll");
    let Some(pk) = pkgs() else { return };
    let d = read(&pk, "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena");
    let w = CollisionWorld::build(&pk, &d, None);
    let mut seen = std::collections::BTreeMap::new();
    for (i, b) in w.bodies.iter().enumerate() {
        if b.kind == "volume" {
            let k = (b.name.split('.').next().unwrap().to_string(), b.profile.clone(), w.blocks[i], b.response("Camera"));
            *seen.entry(k).or_insert(0) += 1;
        }
    }
    println!("Arena volumes (class, profile, blocks pawn, camera response): {seen:?}");
    // every BlockingVolume resolves through InvisibleWall (WorldStatic, Visibility ignored) and blocks the pawn
    assert!(seen.keys().any(|k| k.0 == "BlockingVolume" && k.1 == "InvisibleWall" && k.2));
    // Arena's CameraBlockingVolumes all override the class default (OverlapAll + Camera Block) in the level: 6 serialize
    // the BlockAll profile, 3 a Custom WorldDynamic / QueryAndPhysics body with only Visibility + Projectile ignored;
    // either way they block the pawn and the camera
    assert!(seen.keys().filter(|k| k.0 == "CameraBlockingVolume").all(|k| (k.1 == "Custom" || k.1 == "BlockAll") && k.2 && k.3 == Resp::Block));
    // the arena wall takes its mesh's DefaultInstance (AStaticMeshActor sets bUseDefaultCollision)
    assert!(w.bodies.iter().any(|b| b.mesh.contains("arena_wall_01a") && b.profile == "BlockAll (mesh default)"));
}

/// ThePit's landscape is mirrored (scale -39, 39, 19): its collision samples are cooked without the row flip, and every
/// one equals the render heightmap at the same vertex
#[test]
fn mirrored_landscape_collision_equals_render() {
    let Some(pk) = pkgs() else { return };
    let d = read(&pk, "Mordhau/Content/Mordhau/Maps/ThePit/ThePit");
    assert!(!d.landscape_collision.is_empty());
    let mut worst = 0f32;
    let mut n = 0;
    for c in &d.landscape_collision {
        assert!(c.mirrored, "{}", c.name);
        let r = d.landscape.iter().find(|r| r.section_base == c.section_base).unwrap();
        for (a, b) in c.heights.iter().zip(&r.heights) {
            worst = worst.max((a - b).abs());
        }
        n += 1;
    }
    println!("ThePit: {n} mirrored collision components, worst height diff {worst}");
    assert_eq!(worst, 0.0);
    // the spawn that looked 'under the terrain' with the unmirrored order now stands above it
    let w = CollisionWorld::build(&pk, &d, None);
    let h = w.sweep([2807.0, 1570.0, 5000.0], [2807.0, 1570.0, -5000.0], 0.0, 0.0, &|b| w.bodies[b as usize].kind == "landscape");
    let z = h.first().map(|h| h.location[2]).unwrap_or(f64::NAN);
    println!("landscape under FFA_ThePit MordhauPlayerStart15 (z 453): z {z:.0}");
    assert!(z < 453.0 - 96.0 + 1.0, "landscape at {z}");
}

/// Spline mesh bending (CalcSliceTransform): a straight spline along X with unit scale leaves a mesh point where it
/// was, and the deformed point follows the curve (start / end positions, roll, scale)
#[test]
fn spline_deform_matches_slice_transform() {
    let pk = pkgs();
    let base = mh_level::SplineMeshPlacement {
        level: 0,
        name: String::new(),
        component_path: String::new(),
        actor: String::new(),
        mesh: String::new(),
        xf: mh_level::xf::IDENTITY,
        materials: vec![],
        material_paths: vec![],
        cast_shadow: true,
        start_pos: [0.0; 3],
        start_tangent: [100.0, 0.0, 0.0],
        start_scale: [1.0, 1.0],
        start_roll: 0.0,
        start_offset: [0.0; 2],
        end_pos: [100.0, 0.0, 0.0],
        end_tangent: [100.0, 0.0, 0.0],
        end_scale: [1.0, 1.0],
        end_roll: 0.0,
        end_offset: [0.0; 2],
        forward_axis: 0,
        spline_up_dir: [0.0, 0.0, 1.0],
        spline_boundary_min: 0.0,
        spline_boundary_max: 0.0,
        smooth_interp_roll_scale: false,
        lods: vec![],
    };
    // identity: mesh spans x 0..100, the spline is the straight segment 0..100
    let p = base.deform([25.0, 10.0, 5.0], (0.0, 100.0));
    assert!((p[0] - 25.0).abs() < 1e-9 && (p[1] - 10.0).abs() < 1e-9 && (p[2] - 5.0).abs() < 1e-9, "{p:?}");
    // scale 2 at the end: the far end's cross-section doubles; a quarter turn of roll at the end rotates Y into Z
    let mut s2 = base.clone();
    s2.end_scale = [2.0, 2.0];
    let p = s2.deform([100.0, 10.0, 0.0], (0.0, 100.0));
    assert!((p[1].abs() - 20.0).abs() < 1e-9, "{p:?}");
    let mut s3 = base.clone();
    s3.end_roll = std::f64::consts::FRAC_PI_2;
    let p = s3.deform([100.0, 10.0, 0.0], (0.0, 100.0));
    assert!(p[1].abs() < 1e-9 && (p[2].abs() - 10.0).abs() < 1e-9, "{p:?}");
    // a bent spline: the start and end slices sit at StartPos / EndPos
    let mut s4 = base.clone();
    s4.end_pos = [100.0, 100.0, 0.0];
    s4.end_tangent = [0.0, 100.0, 0.0];
    let a = s4.deform([0.0, 0.0, 0.0], (0.0, 100.0));
    let b = s4.deform([100.0, 0.0, 0.0], (0.0, 100.0));
    assert!(a.iter().all(|x| x.abs() < 1e-9) && (b[0] - 100.0).abs() < 1e-9 && (b[1] - 100.0).abs() < 1e-9, "{a:?} {b:?}");
    // real data: colliding spline meshes become bent bodies
    let Some(pk) = pk else { return };
    let mut n = 0;
    for m in ["Mordhau/Content/Mordhau/Maps/Grad/Grad", "Mordhau/Content/Mordhau/Maps/FeitoriaMap/FeitoriaMap", "Mordhau/Content/Mordhau/Maps/DuelCamp/Camp"] {
        let d = read(&pk, m);
        let w = CollisionWorld::build(&pk, &d, None);
        let sb = w.bodies.iter().filter(|b| b.kind == "spline").count();
        println!("{m}: {} spline meshes, {sb} colliding", d.splines.len());
        n += sb;
    }
    println!("colliding spline bodies on the probed maps: {n}");
}

//! PhysicsAsset from the paks (mh_assets::physics) against CUE4Parse's decode (extract/json, every serialized element
//! and constraint field) and against the hitboxes the Godot port's melee trace uses (UePhysics.body_boxes, dumped by
//! godot/tools/export_physics.gd into data_gen/physics/body_boxes_godot.json).

use mh_assets::physics::{self, godot_box};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PA: &str = "Mordhau/Content/UMA/UMA/Master/UMA_Master_PhysicsAsset";

#[test]
fn native_collision_pairs_and_physical_materials_equal_cue4parse() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount")));
    for name in ["UMA_Master_PhysicsAsset", "UMA_Master_RagdollPhysicsAsset_Proper", "PA_FallingRagdoll"] {
        let pkg=format!("Mordhau/Content/UMA/UMA/Master/{name}");
        let raw: Value=serde_json::from_slice(&std::fs::read(root().join(format!("extract/json/{pkg}.json"))).unwrap()).unwrap();
        let pa=physics::read(&rd,&pkg).unwrap();
        let table=&raw.as_array().unwrap().iter().find(|e|e["Type"]=="PhysicsAsset").unwrap()["CollisionDisableTable"];
        let want:Vec<_>=table.as_array().unwrap().iter().map(|v| ([v["Key"]["Indices"][0].as_u64().unwrap() as usize,v["Key"]["Indices"][1].as_u64().unwrap() as usize],v["Value"].as_bool().unwrap())).collect();
        assert_eq!(pa.collision_disable_pairs,want,"{name} native pairs");
        for b in &pa.bodies {
            let rawbody=raw.as_array().unwrap().iter().find(|e|e["Type"]=="SkeletalBodySetup" && e["Properties"]["BoneName"]==b.bone).unwrap();
            let mat=rawbody["Properties"]["PhysMaterial"]["ObjectPath"].as_str().map(mh_assets::material::strip_index).map(str::to_string);
            assert_eq!(b.physics_material,mat,"{} material",b.bone);
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + b.abs())
}

#[test]
fn physics_asset_equals_cue4parse() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks")));
    let pa = physics::read(&rd, PA).expect("PhysicsAsset");
    let d: Value = serde_json::from_slice(&std::fs::read(root().join(format!("extract/json/{PA}.json"))).unwrap()).unwrap();
    let ex = d.as_array().unwrap();
    let bodies: Vec<&Value> = ex.iter().filter(|e| e["Type"] == "SkeletalBodySetup").collect();
    let cons: Vec<&Value> = ex.iter().filter(|e| e["Type"] == "PhysicsConstraintTemplate").collect();
    assert_eq!((pa.bodies.len(), pa.constraints.len()), (bodies.len(), cons.len()));
    assert_eq!((pa.bodies.len(), pa.constraints.len()), (16, 15));
    for j in bodies {
        let p = &j["Properties"];
        let bone = p["BoneName"].as_str().unwrap();
        let b = pa.body(bone).unwrap_or_else(|| panic!("no body {bone}"));
        let jb = p["AggGeom"]["BoxElems"].as_array().map_or(0, |a| a.len());
        assert_eq!(b.geom.boxes.len(), jb, "{bone} boxes");
        for (mine, w) in b.geom.boxes.iter().zip(p["AggGeom"]["BoxElems"].as_array().into_iter().flatten()) {
            for (k, v) in [("X", mine.x), ("Y", mine.y), ("Z", mine.z)] {
                assert!(close(v, w[k].as_f64().unwrap(), 1e-6), "{bone} {k}");
            }
            for (i, k) in ["X", "Y", "Z"].iter().enumerate() {
                assert!(close(mine.center[i], w["Center"][k].as_f64().unwrap_or(0.0), 1e-6), "{bone} centre");
            }
            for (i, k) in ["Pitch", "Yaw", "Roll"].iter().enumerate() {
                assert!(close(mine.rotation_deg[i], w["Rotation"][k].as_f64().unwrap_or(0.0), 1e-6), "{bone} rotation");
            }
        }
        for k in ["SphylElems", "SphereElems", "ConvexElems", "TaperedCapsuleElems"] {
            let n = p["AggGeom"][k].as_array().map_or(0, |a| a.len());
            let mine = match k {
                "SphylElems" => b.geom.sphyls.len(),
                "SphereElems" => b.geom.spheres.len(),
                "ConvexElems" => b.geom.convex.len(),
                _ => b.geom.tapered.len(),
            };
            assert_eq!(mine, n, "{bone} {k}");
        }
        assert_eq!(b.collision_trace_flag, p["CollisionTraceFlag"].as_str().unwrap_or("CTF_UseDefault"));
    }
    for j in cons {
        let ci = &j["Properties"]["DefaultInstance"];
        let c = pa.constraints.iter().find(|c| c.joint == ci["JointName"].as_str().unwrap()).unwrap();
        assert_eq!((c.bone1.as_str(), c.bone2.as_str()), (ci["ConstraintBone1"].as_str().unwrap(), ci["ConstraintBone2"].as_str().unwrap()));
        for (i, k) in ["X", "Y", "Z"].iter().enumerate() {
            if let Some(v) = ci["Pos2"][k].as_f64() {
                assert!(close(c.pos2[i], v, 1e-6));
            }
            if let Some(v) = ci["PriAxis2"][k].as_f64() {
                assert!(close(c.pri_axis2[i], v, 1e-6));
            }
        }
        let pi = &ci["ProfileInstance"];
        if let Some(v) = pi["ConeLimit"]["Swing1LimitDegrees"].as_f64() {
            assert_eq!(c.limits.swing1_deg, v);
        }
        if let Some(v) = pi["TwistLimit"]["TwistLimitDegrees"].as_f64() {
            assert_eq!(c.limits.twist_deg, v);
        }
        assert_eq!(c.limits.disable_collision, pi["bDisableCollision"].as_bool().unwrap_or(false));
    }
    println!("  {} bodies ({} boxes), {} constraints", pa.bodies.len(), pa.bodies.iter().map(|b| b.geom.boxes.len()).sum::<usize>(), pa.constraints.len());
}

/// The melee trace's hitboxes: godot_box of every colliding box = UePhysics.body_boxes (1e-6 m, |q.q'| >= 1 - 1e-9)
#[test]
fn hitboxes_equal_godot_body_boxes() {
    let p = root().join("data_gen/physics/body_boxes_godot.json");
    let Ok(raw) = std::fs::read(&p) else { panic!("{} missing: run godot/tools/export_physics.gd", p.display()) };
    let g: Value = serde_json::from_slice(&raw).unwrap();
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks")));
    let pa = physics::read(&rd, PA).unwrap();
    let mine: Vec<(String, [f64; 3], [f64; 4], [f64; 3])> = pa
        .bodies
        .iter()
        .flat_map(|b| b.geom.boxes.iter().filter(|x| x.shape.collides()).map(move |x| {
            let (o, q, h) = godot_box(x);
            (b.bone.clone(), o, q, h)
        }))
        .collect();
    let want = g["boxes"].as_array().unwrap();
    assert_eq!(mine.len(), want.len());
    for w in want {
        let bone = w["bone"].as_str().unwrap();
        let f3 = |k: &str| -> Vec<f64> { w[k].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect() };
        let (o, q, h) = (f3("origin"), f3("quat"), f3("half"));
        let hit = mine.iter().any(|m| {
            m.0 == bone
                && (0..3).all(|i| (m.1[i] - o[i]).abs() < 1e-6 && (m.3[i] - h[i]).abs() < 1e-6)
                && (0..4).map(|i| m.2[i] * q[i]).sum::<f64>().abs() >= 1.0 - 1e-6
        });
        assert!(hit, "{bone}: Godot box {o:?} {q:?} {h:?} not produced from the paks");
    }
    println!("  {} hitboxes = Godot", want.len());
}

/// The longsword trace sockets (ue_physics.gd sockets) = CUE4Parse's SkeletalMeshSocket exports of the mesh's skeleton
#[test]
fn weapon_trace_sockets() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks")));
    let mesh = "Mordhau/Content/Mordhau/Assets/Weapons/Longsword/SkeletalMeshes/SK_Longsword_Blade_01";
    let s = physics::sockets(&rd, mesh);
    let (a, b) = (s.get("TraceStart").expect("TraceStart"), s.get("TraceEnd").expect("TraceEnd"));
    assert_ne!(a.location, b.location);
    println!("  {mesh}: {} sockets, TraceStart {:?} on {}, TraceEnd {:?}", s.len(), a.location, a.bone, b.location);
}

/// EVD_RAG_019/020: compare the actual pak reader with independently decoded CUE4Parse exports for all three
/// assets. In particular, a missing mass/damping/drive tag must stay unknown instead of becoming a guessed default.
#[test]
fn ragdoll_dynamics_equal_cue4parse() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks")));
    for name in ["UMA_Master_PhysicsAsset", "UMA_Master_RagdollPhysicsAsset_Proper", "PA_FallingRagdoll"] {
        let pkg = format!("Mordhau/Content/UMA/UMA/Master/{name}");
        let pa = physics::read(&rd, &pkg).expect("PhysicsAsset");
        let ex: Value = serde_json::from_slice(&std::fs::read(root().join(format!("extract/json/{pkg}.json"))).unwrap()).unwrap();
        for body in &pa.bodies {
            let p = &ex.as_array().unwrap().iter().find(|e| e["Type"] == "SkeletalBodySetup" && e["Properties"]["BoneName"] == body.bone).unwrap()["Properties"]["DefaultInstance"];
            let d = &body.dynamics;
            for (key, actual) in [
                ("MassInKgOverride", d.mass_kg), ("MassScale", d.mass_scale),
                ("LinearDamping", d.linear_damping), ("AngularDamping", d.angular_damping),
                ("CustomSleepThresholdMultiplier", d.custom_sleep_threshold_multiplier),
            ] {
                let expected = p[key].as_f64();
                assert_eq!(actual.is_some(), expected.is_some(), "{name}/{} {key} presence", body.bone);
                if let (Some(a), Some(b)) = (actual, expected) {
                    assert!(close(a, b, 1e-6), "{name}/{} {key}: {a} != {b}", body.bone);
                }
            }
            assert_eq!(d.override_mass, p["bOverrideMass"].as_bool());
            assert_eq!(d.sleep_family.as_deref(), p["SleepFamily"].as_str());
            assert_eq!(d.position_solver_iterations, p["PositionSolverIterationCount"].as_u64());
            assert_eq!(d.velocity_solver_iterations, p["VelocitySolverIterationCount"].as_u64());
            assert_eq!(d.com_nudge_cm.is_some(), p["COMNudge"].is_object());
            if let Some(com) = d.com_nudge_cm {
                for (i, key) in ["X", "Y", "Z"].iter().enumerate() {
                    assert!(close(com[i], p["COMNudge"][key].as_f64().unwrap(), 1e-6));
                }
            }
        }
        for joint in &pa.constraints {
            let p = &ex.as_array().unwrap().iter().find(|e| e["Type"] == "PhysicsConstraintTemplate" && e["Properties"]["DefaultInstance"]["JointName"] == joint.joint).unwrap()["Properties"]["DefaultInstance"]["ProfileInstance"];
            let d = &joint.dynamics;
            for (key, actual) in [("LinearLimit", &d.linear_limit), ("ConeLimit", &d.cone_limit), ("TwistLimit", &d.twist_limit)] {
                assert_eq!(actual.soft, p[key]["bSoftConstraint"].as_bool());
                for (field, actual) in [("Stiffness", actual.stiffness), ("Damping", actual.damping), ("Restitution", actual.restitution), ("ContactDistance", actual.contact_distance)] {
                    let expected = p[key][field].as_f64();
                    assert_eq!(actual.is_some(), expected.is_some(), "{name}/{} {key}.{field}", joint.joint);
                    if let (Some(a), Some(b)) = (actual, expected) { assert!(close(a, b, 1e-6)); }
                }
            }
            for (group, key, actual) in [
                ("AngularDrive", "TwistDrive", &d.twist_drive), ("AngularDrive", "SwingDrive", &d.swing_drive),
                ("AngularDrive", "SlerpDrive", &d.slerp_drive), ("LinearDrive", "XDrive", &d.linear_drives[0]),
                ("LinearDrive", "YDrive", &d.linear_drives[1]), ("LinearDrive", "ZDrive", &d.linear_drives[2]),
            ] {
                assert_eq!(actual.position_enabled, p[group][key]["bEnablePositionDrive"].as_bool());
                assert_eq!(actual.velocity_enabled, p[group][key]["bEnableVelocityDrive"].as_bool());
                for (field, actual) in [("Stiffness", actual.stiffness), ("Damping", actual.damping), ("MaxForce", actual.force_limit)] {
                    let expected = p[group][key][field].as_f64();
                    assert_eq!(actual.is_some(), expected.is_some());
                    if let (Some(a), Some(b)) = (actual, expected) { assert!(close(a, b, 1e-6)); }
                }
            }
            assert_eq!(d.angular_drive_mode.as_deref(), p["AngularDrive"]["AngularDriveMode"].as_str());
            for (actual, group, field, axes) in [
                (d.orientation_target_deg, "AngularDrive", "OrientationTarget", ["Pitch", "Yaw", "Roll"]),
                (d.angular_velocity_target, "AngularDrive", "AngularVelocityTarget", ["X", "Y", "Z"]),
                (d.linear_position_target_cm, "LinearDrive", "PositionTarget", ["X", "Y", "Z"]),
                (d.linear_velocity_target, "LinearDrive", "VelocityTarget", ["X", "Y", "Z"]),
            ] {
                assert_eq!(actual.is_some(), p[group][field].is_object());
                if let Some(a) = actual {
                    for i in 0..3 { assert!(close(a[i], p[group][field][axes[i]].as_f64().unwrap(), 1e-6)); }
                }
            }
        }
    }
}

#[test]
fn death_and_knockdown_keep_their_distinct_dynamics() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks")));
    let corpse = physics::read(&rd, "Mordhau/Content/UMA/UMA/Master/UMA_Master_RagdollPhysicsAsset_Proper").unwrap();
    let fall = physics::read(&rd, "Mordhau/Content/UMA/UMA/Master/PA_FallingRagdoll").unwrap();
    assert_eq!((corpse.bodies.len(), corpse.constraints.len()), (16, 15));
    assert_eq!((fall.bodies.len(), fall.constraints.len()), (17, 16));
    let head = &corpse.body("Head").unwrap().dynamics;
    assert_eq!((head.override_mass, head.mass_kg, head.angular_damping), (Some(true), Some(7.3), Some(10.0)));
    assert_eq!(head.linear_damping, None);
    let head = &fall.body("Head").unwrap().dynamics;
    assert_eq!(head.mass_kg, None);
    assert!(close(head.mass_scale.unwrap(), 0.3, 1e-6));
    assert_eq!(head.com_nudge_cm, Some([0.0, 0.0, -10.0]));
    assert_eq!(head.linear_damping, Some(3.5));
    assert_eq!(fall.body("Position").unwrap().physics_type, "PhysType_Kinematic");
    let tether = fall.constraints.iter().find(|j| j.bone1 == "Hips" && j.bone2 == "Position").unwrap();
    assert_eq!(tether.limits.linear_limit, 50.0);
    assert_eq!(tether.dynamics.linear_limit.stiffness, Some(200.0));
    assert!(close(tether.dynamics.linear_limit.damping.unwrap(), 0.9, 1e-6));
    let corpse_head = corpse.constraints.iter().find(|j| j.bone1 == "Head").unwrap();
    let fall_head = fall.constraints.iter().find(|j| j.bone1 == "Head").unwrap();
    assert_eq!(corpse_head.dynamics.twist_drive.stiffness, Some(70.0));
    assert_eq!(corpse_head.dynamics.twist_drive.position_enabled, None);
    assert_eq!(fall_head.dynamics.twist_drive.stiffness, Some(500.0));
    assert_eq!(fall_head.dynamics.twist_drive.position_enabled, Some(true));
    assert_eq!(fall_head.dynamics.orientation_target_deg, Some([0.0, 0.0, 30.0]));
}

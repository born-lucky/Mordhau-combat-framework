//! camera1p gauntlet probe: reference skeletons of the first-person ("_high") arm / hand parts vs the 3P parts and
//! UMA_Master, to test whether the merged FP mesh's bind pose differs (which would draw the 1P arms away from the bones).
use mh_assets::pak_source::PakSource;
use mh_assets::skeletal_mesh;

fn src() -> Option<PakSource> {
    PakSource::mount_default().ok()
}

#[test]
#[ignore]
fn probe_fp_part_ref_skeletons() {
    let Some(src) = src() else { eprintln!("MORDHAU_PAKS unset"); return };
    let bones = ["Position", "Hips", "LowerBack", "Spine", "Spine1", "RightShoulder", "RightArm", "RightForeArm", "RightHand", "RightHandFinger02_01", "LeftShoulder", "LeftArm", "LeftForeArm", "LeftHand"];
    for pkg in [
        "Mordhau/Content/UMA/UMA/Master/UMA_Master",
        "Mordhau/Content/UMA/Separated/Arm_right",
        "Mordhau/Content/UMA/Separated/Arm_right_high",
        "Mordhau/Content/UMA/Separated/Hand_right",
        "Mordhau/Content/UMA/Separated/Hand_right_high",
        "Mordhau/Content/UMA/Separated/ArmAuxiliary_high",
        "Mordhau/Content/UMA/Separated/TorsoExtended",
        "Mordhau/Content/UMA/UMAEquipment/Tier1/Gambeson/SkeletalMeshes/SK_Gambeson_Arms_fps",
        "Mordhau/Content/UMA/UMAEquipment/Tier1/Gambeson/SkeletalMeshes/SK_Gambeson_Arms",
    ] {
        match skeletal_mesh::lod0(&src, pkg) {
            Ok(m) => {
                println!("## {pkg}: {} ref bones, {} verts", m.ref_skeleton.bones.len(), m.vertices.positions.len());
                for b in bones {
                    if let Some(i) = m.ref_skeleton.find(b) {
                        let t = &m.ref_skeleton.pose[i];
                        println!("  {b:22} parent {:3} loc ({:8.3},{:8.3},{:8.3}) rot ({:.4},{:.4},{:.4},{:.4})", m.ref_skeleton.bones[i].parent, t.translation[0], t.translation[1], t.translation[2], t.rotation[0], t.rotation[1], t.rotation[2], t.rotation[3]);
                    }
                }
            }
            Err(e) => println!("## {pkg}: ERR {:?}", e),
        }
    }
}

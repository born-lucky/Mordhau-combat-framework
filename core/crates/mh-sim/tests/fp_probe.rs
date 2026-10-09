//! First-person probes (first-person r1; run on demand: `cargo test -p mh-sim --test fp_probe -- --ignored --nocapture`).
//! The raw 1P idle clip on the sim skeleton: which bones it tracks, and the right-hand finger chain against the
//! reference pose (is the grip closed in the clip?).

use std::sync::Arc;

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";

#[test]
#[ignore]
fn probe_1p_clip_fingers() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clip = std::env::var("MH_PROBE_CLIP").unwrap_or("Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_Idle_1P".into());
    let a = mh_assets::anim::decode(&src, &clip).unwrap();
    println!("clip {clip}: {} tracks, skeleton {} bones", a.tracks.len(), sk.names.len());
    let tracked: Vec<String> = a.tracks.iter().map(|t| sk.names.get(t.bone as usize).cloned().unwrap_or(format!("#{}", t.bone))).collect();
    println!("tracked: {tracked:?}");
    let untracked: Vec<&String> = sk.names.iter().filter(|n| !tracked.contains(n)).collect();
    println!("untracked: {untracked:?}");
    let pose = sk.sample(&a, 0.0);
    let rf = sk.ref_pose();
    let hand = sk.find("RightHand").unwrap();
    for f in ["RightHandFinger01_03", "RightHandFinger02_03", "RightHandFinger03_03", "RightHandFinger04_03", "RightHandFinger05_03"] {
        let i = sk.find(f).unwrap();
        let l = pose[hand].inverse().apply(pose[i].loc);
        let r = rf[hand].inverse().apply(rf[i].loc);
        println!("{f}: clip hand-local {:?}  ref {:?}", (l.x, l.y, l.z), (r.x, r.y, r.z));
    }
}

/// The sim's 1P idle pose (graph) against the raw 1P idle clip: hand-local fingertips of both hands
#[test]
#[ignore]
fn probe_1p_sim_fingers() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim};
    use mordhau_core::ue::FVector;
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
    let fp = std::env::var("MH_PROBE_3P").is_err();
    s.set_first_person(a, fp);
    s.set_view_target(a, fp, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    for _ in 0..90 {
        s.step(&[]);
    }
    let sk = s.geo.skeleton.clone();
    let pose = sk.to_component(&s.local_pose(a));
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clip = mh_assets::anim::decode(&src, "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_Idle_1P").unwrap();
    let cp = sk.sample(&clip, 0.0);
    for side in ["Right", "Left"] {
        let hand = sk.find(&format!("{side}Hand")).unwrap();
        for k in 1..=5 {
            let i = sk.find(&format!("{side}HandFinger0{k}_03")).unwrap();
            let l = pose[hand].inverse().apply(pose[i].loc);
            let c = cp[hand].inverse().apply(cp[i].loc);
            println!("{side} finger{k}: sim({}) ({:6.1} {:6.1} {:6.1})  clip ({:6.1} {:6.1} {:6.1})", if fp { "1P" } else { "3P" }, l.x, l.y, l.z, c.x, c.y, c.z);
        }
        let hp = pose[hand].loc;
        let hc = cp[hand].loc;
        println!("{side}Hand component: sim ({:6.1} {:6.1} {:6.1}) clip ({:6.1} {:6.1} {:6.1})", hp.x, hp.y, hp.z, hc.x, hc.y, hc.z);
    }
}

/// Root-chain bones (Global / Position / Hips / Spine1) of 1P clips: local track values against the reference pose
#[test]
#[ignore]
fn probe_1p_clip_root() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clips = std::env::var("MH_PROBE_CLIPS").unwrap_or(
        "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_Idle_1P,Mordhau/Content/Mordhau/Animations/RawClips/2H/Polearm/2H_Polearm_Idle_1P".into(),
    );
    for clip in clips.split(',') {
        let a = mh_assets::anim::decode(&src, clip).unwrap();
        let loc = sk.sample_local(&a, 0.0);
        let cs = sk.to_component(&loc);
        println!("{clip} len {}", a.sequence_length);
        for b in ["Global", "Position", "Hips", "Spine1", "Neck", "head"] {
            let i = sk.find(b).unwrap();
            let r = sk.ref_local[i];
            println!(
                "  {b:8} local q {:?} t {:?} | ref q {:?} t {:?} | comp t {:?}",
                (loc[i].rot.x, loc[i].rot.y, loc[i].rot.z, loc[i].rot.w),
                (loc[i].loc.x, loc[i].loc.y, loc[i].loc.z),
                (r.rot.x, r.rot.y, r.rot.z, r.rot.w),
                (r.loc.x, r.loc.y, r.loc.z),
                (cs[i].loc.x, cs[i].loc.y, cs[i].loc.z)
            );
        }
    }
}

/// The 1P idle of several weapons on the sim (graph, first person): hands and weapon in the 1P camera frame
/// (UpdateFPCamera's CameraLocation1P / CameraRotation1P) and their screen position at a horizontal FOV
/// (MH_PROBE_FOV, default 78; 16:9). MH_PROBE_LOOK = look-up degrees.
#[test]
#[ignore]
fn probe_1p_screen() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim, SimInput};
    use mordhau_core::ue::{FQuat, FVector};
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let fov: f32 = std::env::var("MH_PROBE_FOV").ok().and_then(|v| v.parse().ok()).unwrap_or(78.0);
    let look: f64 = std::env::var("MH_PROBE_LOOK").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let ws = std::env::var("MH_PROBE_WEAPONS").unwrap_or("TwoHandedSword/BP_Greatsword,TwoHandedSword/BP_Longsword,OneHanded/BP_ArmingSword,Polearms/BP_Poleaxe".into());
    let base = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/";
    for w in ws.split(',') {
        let wp = format!("{base}{w}");
        let ld = mh_sim::load::load(&m, vfs.clone(), &[wp.as_str()]).unwrap();
        let mut floor = BoxWorld::new();
        floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
        let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
        s.dedicated_server = false;
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: wp.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
        s.set_first_person(a, true);
        s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
        for n in 0..90 {
            let inp = if n == 0 { vec![(a, SimInput { look_up: Some(look), ..Default::default() })] } else { vec![] };
            s.step(&inp);
        }
        let g = s.combat.fighters[a].geom.unwrap();
        let cam = mordhau_core::ue::FTransform::new(FQuat::from_rotator(g.camera_rot.0, g.camera_rot.1, 0.0), g.camera_loc);
        let inv = cam.inverse();
        let t = (fov.to_radians() * 0.5).tan();
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        println!("{w}: cam loc {:?} rot {:?} upper {:?}", (g.camera_loc.x, g.camera_loc.y, g.camera_loc.z), g.camera_rot, s.upper_lower_assets(a, true).0);
        let show = |n: &str, wl: FVector| {
            let l = inv.apply(wl);
            let (sx, sy) = (0.5 + 0.5 * l.y / (l.x * t), 0.5 - 0.5 * l.z / (l.x * t * 9.0 / 16.0));
            println!("  {n:12} fwd {:6.1} right {:6.1} up {:6.1}  screen ({sx:5.2}, {sy:5.2})", l.x, l.y, l.z);
        };
        for b in ["RightHand", "LeftHand", "RightWeapon"] {
            let i = s.geo.skeleton.find(b).unwrap();
            show(b, s.geo.bone_world(p, i).loc);
        }
        if let Some(wx) = s.geo.weapons.get(&wp).and_then(|wg| s.geo.weapon_world(p, wg)) {
            show("weapon", wx.loc);
            show("weapon+100x", wx.apply(FVector::new(100.0, 0.0, 0.0)));
            show("weapon+100z", wx.apply(FVector::new(0.0, 0.0, 100.0)));
        }
        drop(posed);
    }
}

/// The sim's 1P idle: root-chain CS transforms against the raw 1P clip and the lower animation (the LowerBack-filtered
/// mesh-space layered blend takes Global / Position / Hips from the base pose)
#[test]
#[ignore]
fn probe_1p_root_chain() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim};
    use mordhau_core::ue::FVector;
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    for _ in 0..90 {
        s.step(&[]);
    }
    let (up, lo) = s.upper_lower_assets(a, true);
    println!("upper {up} lower {lo}");
    let sk = s.geo.skeleton.clone();
    let pose = sk.to_component(&s.local_pose(a));
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clip = mh_assets::anim::decode(&src, "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_Idle_1P").unwrap();
    let cp = sk.sample(&clip, 0.0);
    let lp = mh_assets::anim::decode(&src, &lo).ok().map(|l| sk.sample(&l, 0.0));
    for b in ["Global", "Position", "Hips", "LowerBack", "Spine1", "RightHand"] {
        let i = sk.find(b).unwrap();
        let f = |t: &mordhau_core::ue::FTransform| format!("q[{:.4} {:.4} {:.4} {:.4}] t[{:.1} {:.1} {:.1}]", t.rot.x, t.rot.y, t.rot.z, t.rot.w, t.loc.x, t.loc.y, t.loc.z);
        println!("{b:9} sim {} | 1P clip {} | lower {}", f(&pose[i]), f(&cp[i]), lp.as_ref().map(|l| f(&l[i])).unwrap_or_default());
    }
}

/// A 1P strike (MH_PROBE_MOVE, default 0) and parry on the sim: the playing sequence and the hands' 1P camera-frame
/// position / screen position (MH_PROBE_FOV, default 78) over time
#[test]
#[ignore]
fn probe_1p_strike() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim, SimInput};
    use mordhau_core::ue::{FQuat, FVector};
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let fov: f32 = std::env::var("MH_PROBE_FOV").ok().and_then(|v| v.parse().ok()).unwrap_or(78.0);
    let mv: i64 = std::env::var("MH_PROBE_MOVE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let ang: f64 = std::env::var("MH_PROBE_ANGLE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let parry = std::env::var("MH_PROBE_PARRY").is_ok();
    let fp = std::env::var("MH_PROBE_3P").is_err();
    let w = std::env::var("MH_PROBE_WEAPON").unwrap_or(LS.into());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[w.as_str()]).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
    s.set_first_person(a, fp);
    s.set_view_target(a, fp, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    let t = (fov.to_radians() * 0.5).tan();
    for n in 0..150 {
        // MH_PROBE_WALK: hold MoveForward from frame 60 instead of the attack (MH_PROBE_SPRINT: and Sprint)
        let walk = std::env::var("MH_PROBE_WALK").is_ok();
        let inp = if walk && n >= 60 {
            vec![(a, SimInput { fwd: 1.0, sprint: std::env::var("MH_PROBE_SPRINT").is_ok(), ..Default::default() })]
        } else if n == 60 && !walk {
            vec![(a, if parry { SimInput { parry: Some(0), ..Default::default() } } else { SimInput { attack: Some((mv, ang)), ..Default::default() } })]
        } else {
            vec![]
        };
        s.step(&inp);
        if n < 60 || n % 5 != 0 {
            continue;
        }
        let g = s.combat.fighters[a].geom.unwrap();
        let cam = mordhau_core::ue::FTransform::new(FQuat::from_rotator(g.camera_rot.0, g.camera_rot.1, 0.0), g.camera_loc);
        let inv = cam.inverse();
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let rh = inv.apply(s.geo.bone_world(p, s.geo.skeleton.find("RightHand").unwrap()).loc);
        let lh = inv.apply(s.geo.bone_world(p, s.geo.skeleton.find("LeftHand").unwrap()).loc);
        let tip = s.geo.weapons.get(&w).and_then(|wg| s.geo.weapon_world(p, wg)).map(|x| inv.apply(x.apply(FVector::new(0.0, 0.0, 100.0))));
        // the grip: the weapon origin against the right hand (MH_PROBE_GRIP: print their distance)
        let worg = s.geo.weapons.get(&w).and_then(|wg| s.geo.weapon_world(p, wg)).map(|x| inv.apply(x.loc));
        if std::env::var("MH_PROBE_GRIP").is_ok() {
            if let Some(o) = worg {
                println!("   grip: weapon origin - RH = {:.1} cm", (o - rh).length());
            }
        }
        drop(posed);
        let sc = |l: FVector| (0.5 + 0.5 * l.y / (l.x * t), 0.5 - 0.5 * l.z / (l.x * t * 9.0 / 16.0));
        let f = &s.fanim[a];
        println!(
            "t {:.2} {:?} upper_in {:?} seq {} pos {:.3} | RH ({:5.1} {:5.1} {:5.1}) scr {:?} | LH scr {:?} | tip+100z scr {:?}",
            (n - 60) as f32 / 60.0,
            s.combat.cur_m(a).map(|m| m.kind()),
            f.upper_input,
            f.ma.seq.rsplit('/').next().unwrap_or(""),
            f.ma.position,
            rh.x, rh.y, rh.z,
            sc(rh),
            sc(lh),
            tip.map(sc)
        );
    }
}

/// A raw 1P clip over time: Global / Position / Hips / Spine1 CS rotation (as rotators) and translation
#[test]
#[ignore]
fn probe_1p_clip_root_time() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clip = std::env::var("MH_PROBE_CLIP").unwrap_or("Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_RightStrike1P".into());
    let a = mh_assets::anim::decode(&src, &clip).unwrap();
    println!("{clip} len {} frames {}", a.sequence_length, a.num_frames);
    let n = 12;
    for k in 0..=n {
        let t = a.sequence_length * k as f32 / n as f32;
        let cs = sk.sample(&a, t);
        let mut line = format!("t {t:.2}");
        for b in ["Position", "Hips", "Spine1", "RightHand"] {
            let i = sk.find(b).unwrap();
            let (p, y, r) = mordhau_core::combat::geometry::quat_rotator(cs[i].rot);
            line += &format!(" | {b} r({p:6.1} {y:6.1} {r:6.1}) t({:5.1} {:5.1} {:5.1})", cs[i].loc.x, cs[i].loc.y, cs[i].loc.z);
        }
        println!("{line}");
    }
}

/// Reference-pose translations of the arm chain on the meshes a 1P merged mesh is built from (Skeleton-mode
/// translation retarget takes them from the mesh)
#[test]
#[ignore]
fn probe_1p_part_refposes() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    for m in [
        "Mordhau/Content/UMA/UMA/Master/UMA_Master",
        "Mordhau/Content/UMA/Separated/Armslegs",
        "Mordhau/Content/UMA/Separated/Arm_right_high",
        "Mordhau/Content/UMA/Separated/Hand_right_high",
        "Mordhau/Content/UMA/UMAEquipment/Tier2/BrigandineArmor/Hands/WisbyGauntlets",
        "Mordhau/Content/UMA/UMAEquipment/Tier3/Plate/SkeletalMeshes/ItalianHarness/SK_ItalianArms",
    ] {
        let Ok(sm) = mh_assets::skeletal_mesh::lod0(&src, m) else { println!("{m}: decode failed"); continue };
        let rs = &sm.ref_skeleton;
        let mut line = format!("{} ({} bones):", m.rsplit('/').next().unwrap(), rs.bones.len());
        for b in ["Hips", "Spine1", "RightShoulder", "RightArm", "RightForeArm", "RightHand", "RightWeapon"] {
            if let Some(i) = rs.find(b) {
                let t = rs.pose[i].translation;
                line += &format!(" {b}({:.1} {:.1} {:.1})", t[0], t[1], t[2]);
            }
        }
        println!("{line}");
    }
}

/// A raw 1P clip: the right hand in the Spine1 frame and in the Position (camera reference) frame over time
#[test]
#[ignore]
fn probe_1p_clip_hand_frames() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clip = std::env::var("MH_PROBE_CLIP").unwrap_or("Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_RightStrike1P".into());
    let a = mh_assets::anim::decode(&src, &clip).unwrap();
    let (s1, pos, rh) = (sk.find("Spine1").unwrap(), sk.find("Position").unwrap(), sk.find("RightHand").unwrap());
    for k in 0..=12 {
        let t = a.sequence_length * k as f32 / 12.0;
        let cs = sk.sample(&a, t);
        let in_s1 = cs[s1].inverse().apply(cs[rh].loc);
        let in_pos = cs[pos].inverse().apply(cs[rh].loc);
        println!("t {t:.2} hand in Spine1 ({:6.1} {:6.1} {:6.1}) in Position ({:6.1} {:6.1} {:6.1})", in_s1.x, in_s1.y, in_s1.z, in_pos.x, in_pos.y, in_pos.z);
    }
}

/// Both hands in the held weapon's frame (where on the grip each hand sits), 1P and 3P idle and at a strike windup
#[test]
#[ignore]
fn probe_hands_on_grip() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim, SimInput};
    use mordhau_core::ue::FVector;
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let w = std::env::var("MH_PROBE_WEAPON").unwrap_or(LS.into());
    for fp in [false, true] {
        let ld = mh_sim::load::load(&m, vfs.clone(), &[w.as_str()]).unwrap();
        let mut floor = BoxWorld::new();
        floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
        let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
        s.dedicated_server = false;
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
        s.set_first_person(a, fp);
        s.set_view_target(a, fp, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
        for n in 0..120 {
            let inp = if n == 90 { vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })] } else { vec![] };
            s.step(&inp);
            if n == 89 || n == 115 {
                let posed = s.posed.borrow();
                let p = posed.get("A").unwrap();
                let wx = s.geo.weapons.get(&w).and_then(|wg| s.geo.weapon_world(p, wg)).unwrap();
                let inv = wx.inverse();
                let g = |b: &str| inv.apply(s.geo.bone_world(p, s.geo.skeleton.find(b).unwrap()).loc);
                let (r, l, rf, lf) = (g("RightHand"), g("LeftHand"), g("RightHandFinger03_02"), g("LeftHandFinger03_02"));
                println!(
                    "{} {}: weapon-local RightHand ({:.1} {:.1} {:.1}) RF03_02 ({:.1} {:.1} {:.1}) LeftHand ({:.1} {:.1} {:.1}) LF03_02 ({:.1} {:.1} {:.1})",
                    if fp { "1P" } else { "3P" }, if n == 89 { "idle" } else { "windup" }, r.x, r.y, r.z, rf.x, rf.y, rf.z, l.x, l.y, l.z, lf.x, lf.y, lf.z
                );
            }
        }
    }
}

/// The 1P upper additive (BP_MordhauWeapon UpperAdditive1P = Atmospheric_Additive_1P): its additive type and how far it
/// moves the hands when applied to the 1P idle (the sim's apply_additive)
#[test]
#[ignore]
fn probe_1p_upper_additive() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let add = "Mordhau/Content/Mordhau/Animations/RawClips/Misc/Atmospheric_Additive_1P";
    let a = mh_assets::anim::decode(&src, add).unwrap();
    println!("{add}: type {} len {} tracks {} props {:?}", a.additive_anim_type, a.sequence_length, a.tracks.len(), a.properties.get("RefPoseSeq").map(|v| format!("{v:?}")));
    for (k, v) in a.properties.iter() {
        if k.contains("Additive") || k.contains("RefPose") || k.contains("BasePose") {
            println!("  prop {k} = {v:?}");
        }
    }
}

/// first-person r3 (round-restart stutter): what Sim::respawn costs, split into the combat fighter rebuild and the
/// anim rebuild (`cargo test -p mh-sim --release --test fp_probe probe_respawn_cost -- --ignored --nocapture`)
#[test]
#[ignore]
fn probe_respawn_cost() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim};
    use mordhau_core::ue::FVector;
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    for _ in 0..60 {
        s.step(&[]);
    }
    for round in 0..3 {
        let t = std::time::Instant::now();
        s.combat.respawn_fighter(a);
        let t1 = t.elapsed();
        let t = std::time::Instant::now();
        s.respawn(a, FVector::new(0.0, 0.0, 100.0), 0.0);
        let t2 = t.elapsed();
        let t = std::time::Instant::now();
        s.step(&[]);
        let t3 = t.elapsed();
        println!("round {round}: combat respawn_fighter {:.2} ms, Sim::respawn {:.2} ms, next step {:.2} ms", t1.as_secs_f64() * 1e3, t2.as_secs_f64() * 1e3, t3.as_secs_f64() * 1e3);
    }
}

/// first-person r3: the AnimAssets lookups Sim::new_fanim makes per respawn, timed one by one
#[test]
#[ignore]
fn probe_fanim_parts() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let a = mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap();
    let id = mordhau_core::ue::FTransform::new(mordhau_core::ue::FQuat::IDENTITY, mordhau_core::ue::FVector::ZERO);
    for round in 0..2 {
        let t = std::time::Instant::now();
        let _ = a.weapon_cosmetic(LS);
        let t1 = t.elapsed();
        let t = std::time::Instant::now();
        let _ = a.shoulder_offsets_1p(LS);
        let t2 = t.elapsed();
        let t = std::time::Instant::now();
        let _ = (a.offhand_weapon(LS, false, id), a.offhand_weapon(LS, true, id));
        let t3 = t.elapsed();
        let t = std::time::Instant::now();
        let bs = a.upper_blend_space(LS);
        let _ = a.blend_space(&bs);
        let t4 = t.elapsed();
        println!("round {round}: cosmetic {:.2} ms, shoulder {:.2} ms, offhand x2 {:.2} ms, upper bs {:.2} ms", t1.as_secs_f64() * 1e3, t2.as_secs_f64() * 1e3, t3.as_secs_f64() * 1e3, t4.as_secs_f64() * 1e3);
    }
}

/// first-person r3 (grip roll): each part mesh of a weapon (skin 0, part 0 / MH_PROBE_PARTS): bounding box, bones and
/// their reference pose, the bones the vertices are skinned to
#[test]
#[ignore]
fn probe_weapon_parts() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rd = mh_pak::Reader::new(vfs.clone());
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let w = std::env::var("MH_PROBE_WEAPON").unwrap_or(LS.into());
    let d = rd.defaults(&w);
    let strip = |s: &str| mh_sim::physics::strip(s);
    let mut parts = vec![];
    if let Some(m) = d.get("SkeletalMesh").and_then(|v| v["ObjectPath"].as_str()) {
        parts.push(strip(m));
    }
    let choice: Vec<usize> = std::env::var("MH_PROBE_PARTS").unwrap_or_default().split(',').filter_map(|x| x.parse().ok()).collect();
    let skin: usize = std::env::var("MH_PROBE_SKIN").ok().and_then(|x| x.parse().ok()).unwrap_or(0);
    if let Some(sk) = d.get("Skins").and_then(|v| v.as_array()).and_then(|a| a.get(skin).or_else(|| a.first())) {
        for (k, pt) in sk["PartTypes"].as_array().into_iter().flatten().enumerate() {
            let i = choice.get(k).copied().unwrap_or(0);
            let Some(part) = pt["Parts"].as_array().and_then(|a| a.get(i).or_else(|| a.first())) else { continue };
            let cls = strip(part["ObjectPath"].as_str().unwrap_or(""));
            println!("part type {k}: {cls}");
            if let Some(m) = rd.defaults(&cls).get("SkeletalMesh").and_then(|v| v["ObjectPath"].as_str()) {
                parts.push(strip(m));
            }
        }
    }
    for p in parts {
        let Ok(sm) = mh_assets::skeletal_mesh::lod0(&src, &p) else { println!("{p}: no lod0"); continue };
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in &sm.vertices.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let mut used: std::collections::BTreeMap<u16, usize> = Default::default();
        for (i, b) in sm.bones.iter().enumerate() {
            if sm.weights[i] > 0.0 {
                *used.entry(*b).or_default() += 1;
            }
        }
        println!("{p}: {} verts, bbox lo {lo:?} hi {hi:?}", sm.vertices.positions.len());
        for (i, b) in sm.ref_skeleton.bones.iter().enumerate() {
            let t = &sm.ref_skeleton.pose[i];
            println!("   bone {i} {} parent {} rot {:?} loc {:?} skinned verts {:?}", b.name, b.parent, t.rotation, t.translation, used.get(&(i as u16)));
        }
    }
}

/// first-person r3 (1P anims): a raw 1P clip (MH_PROBE_CLIP) sampled alone on the sim skeleton: the right / left hand
/// relative to the clip's own 1P camera point (Spine1 + 41.625 up, rotated with Spine1) over the clip, to compare with
/// probe_1p_strike's graph output (is the hand position the clip's, or something the graph adds?)
#[test]
#[ignore]
fn probe_1p_clip_hands() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clip = std::env::var("MH_PROBE_CLIP").unwrap_or("Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_RightStrike1P".into());
    let a = mh_assets::anim::decode(&src, &clip).unwrap();
    let (sp, rh, lh) = (sk.find("Spine1").unwrap(), sk.find("RightHand").unwrap(), sk.find("LeftHand").unwrap());
    println!("{clip}: length {}", a.sequence_length);
    let n = 14;
    for k in 0..=n {
        let t = a.sequence_length * k as f32 / n as f32;
        let p = sk.sample(&a, t);
        let cam = p[sp].loc;
        let r = |i: usize| {
            let d = p[i].loc - cam;
            (d.x, d.y, d.z)
        };
        println!("t {t:5.2} spine1 {:?} RH-spine1 {:?} LH-spine1 {:?}", (cam.x, cam.y, cam.z), r(rh), r(lh));
    }
}

/// grip r1: the held weapon's axes (blade = TraceStart -> TraceEnd, guard = weapon-local Y) in the RightHand and
/// RightWeapon bone frames, 1P vs 3P idle; the RightWeapon bone relative to RightHand; the character mesh's
/// Weapon / Hand sockets (MH_PROBE_WEAPON picks the weapon)
#[test]
#[ignore]
fn probe_grip_axes() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim};
    use mordhau_core::ue::FVector;
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rd = mh_pak::Reader::new(vfs.clone());
    let (_, mesh, _) = mh_sim::physics::character_mesh(&rd).unwrap();
    for (n, s) in mh_sim::physics::sockets(&rd, &mesh).unwrap() {
        if n.contains("Weapon") || n.contains("Hand") {
            println!("3P mesh {mesh} socket {n}: bone {} rot {:?} loc {:?}", s.bone, s.xf.rot, s.xf.loc);
        }
    }
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let w = std::env::var("MH_PROBE_WEAPON").unwrap_or(LS.into());
    for fp in [false, true] {
        let ld = mh_sim::load::load(&m, vfs.clone(), &[w.as_str()]).unwrap();
        let mut floor = BoxWorld::new();
        floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
        let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
        s.dedicated_server = false;
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
        s.set_first_person(a, fp);
        s.set_view_target(a, fp, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
        for _ in 0..90 {
            s.step(&[]);
        }
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let wg = s.geo.weapons.get(&w).unwrap();
        let wx = s.geo.weapon_world(p, wg).unwrap();
        println!("geo: rot_off {:?} rhe {:?} grip {:?} pitch {} right {} eo {:?} ts {:?} te {:?}", wg.rotation_offset, wg.right_hand_equip_offset, wg.grip_location_local, s.geo.grip_pitch_right, wg.right_handed, wg.equipped_offset, wg.trace_start, wg.trace_end);
        let blade = wx.rot.rotate((wg.trace_end.unwrap() - wg.trace_start.unwrap()).normalized());
        let guard = wx.rot.rotate(FVector::new(0.0, 1.0, 0.0));
        let tag = if fp { "1P" } else { "3P" };
        for b in ["RightHand", "RightWeapon"] {
            let bw = s.geo.bone_world(p, s.geo.skeleton.find(b).unwrap());
            let inv = bw.rot.inverse();
            let (bl, gu) = (inv.rotate(blade), inv.rotate(guard));
            println!("{tag} in {b}: blade ({:5.2} {:5.2} {:5.2}) guard ({:5.2} {:5.2} {:5.2})", bl.x, bl.y, bl.z, gu.x, gu.y, gu.z);
        }
        let h = s.geo.bone_world(p, s.geo.skeleton.find("RightHand").unwrap());
        let rw = s.geo.bone_world(p, s.geo.skeleton.find("RightWeapon").unwrap());
        let rel = h.rot.inverse().mul(rw.rot);
        let l = h.inverse_apply(rw.loc);
        println!("{tag} RightWeapon in RightHand: q {:?} loc ({:.1} {:.1} {:.1})", rel, l.x, l.y, l.z);
        // world axes (actor faces +X)
        println!("{tag} world: blade ({:5.2} {:5.2} {:5.2}) guard ({:5.2} {:5.2} {:5.2})", blade.x, blade.y, blade.z, guard.x, guard.y, guard.z);
        // the 1P camera (GetFirstPersonCameraRotation rva=0x14bcfa0: Q(Position) * MakeFromEuler(-(LookUp + 5.27)) *
        // FRotator(90, 0, -90), as camera.rs) and the axes in its frame (X fwd, Y right, Z up)
        let pos = s.geo.bone_world(p, s.geo.skeleton.find("Position").unwrap());
        let cam = pos.rot.mul(mordhau_core::ue::FQuat::from_rotator(0.0, 0.0, -5.27)).mul(mordhau_core::ue::FQuat::from_rotator(90.0, 0.0, -90.0));
        let f = cam.rotate(FVector::new(1.0, 0.0, 0.0));
        let ci = cam.inverse();
        let (bc, gc) = (ci.rotate(blade), ci.rotate(guard));
        let hc = ci.rotate(s.geo.bone_world(p, s.geo.skeleton.find("RightHand").unwrap()).loc - s.geo.bone_world(p, s.geo.skeleton.find("Spine1").unwrap()).loc);
        println!("{tag} camera fwd world ({:5.2} {:5.2} {:5.2}); in camera: blade ({:5.2} {:5.2} {:5.2}) guard ({:5.2} {:5.2} {:5.2}) hand-spine1 ({:5.1} {:5.1} {:5.1})", f.x, f.y, f.z, bc.x, bc.y, bc.z, gc.x, gc.y, gc.z, hc.x, hc.y, hc.z);
    }
}

/// first-person r3 (1P camera tilt): the Position / Global / Spine1 / head local rotations of a 1P clip over its length
/// (as Euler degrees), to see whether the clip itself carries the view tilt the footage shows during swings
#[test]
#[ignore]
fn probe_1p_clip_root_over_time() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clip = std::env::var("MH_PROBE_CLIP").unwrap_or("Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_RightStrike1P".into());
    let a = mh_assets::anim::decode(&src, &clip).unwrap();
    let tracked: Vec<String> = a.tracks.iter().map(|t| sk.names.get(t.bone as usize).cloned().unwrap_or_default()).collect();
    println!("{clip}: {} tracks; root-ish tracked: {:?}", a.tracks.len(), tracked.iter().filter(|n| ["Global", "Position", "Hips", "Spine1", "Camera", "head", "Neck"].contains(&n.as_str()) || n.to_lowercase().contains("cam")).collect::<Vec<_>>());
    for k in 0..=10 {
        let t = a.sequence_length * k as f32 / 10.0;
        let cs = sk.to_component(&sk.sample_local(&a, t));
        let mut line = format!("t {t:4.2}");
        for b in ["Position", "Spine1", "head"] {
            let i = sk.find(b).unwrap();
            let r = mordhau_core::combat::geometry::quat_rotator(cs[i].rot);
            line += &format!(" | {b} p{:6.1} y{:6.1} r{:6.1}", r.0, r.1, r.2);
        }
        println!("{line}");
    }
}

/// grip r1: every SkeletalMeshSocket named *Weapon* / *Hand* / *Grip* in UMA / 1P / character assets (does a 1P mesh
/// carry its own RightWeapon socket?)
#[test]
#[ignore]
fn probe_weapon_sockets_all() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rd = mh_pak::Reader::new(vfs.clone());
    let filt = std::env::var("MH_PROBE_FILTER").unwrap_or("/UMA/|1P|Character".into());
    let pats: Vec<&str> = filt.split('|').collect();
    let paths: Vec<String> = vfs.list().filter(|p| p.ends_with(".uasset") && pats.iter().any(|q| p.contains(q))).map(|p| p.trim_end_matches(".uasset").to_string()).collect();
    println!("{} assets", paths.len());
    let (mut nread, mut nsock) = (0, 0);
    for p in paths {
        let Some(ex) = rd.read(&p) else { continue };
        nread += 1;
        nsock += ex.iter().filter(|e| e["Type"].as_str() == Some("SkeletalMeshSocket")).count();
        for e in ex.iter().filter(|e| e["Type"].as_str() == Some("SkeletalMeshSocket")) {
            let pr = &e["Properties"];
            let n = pr["SocketName"].as_str().unwrap_or("");
            if n.contains("Weapon") || n.contains("Grip") || (n.contains("Hand") && !n.contains("Dismember")) {
                println!("{p}: {n} bone {} rot {} loc {}", pr["BoneName"], pr["RelativeRotation"], pr["RelativeLocation"]);
            }
        }
    }
    println!("read {nread} packages, {nsock} sockets in all");
}

/// first-person r3 (1P locomotion): the weapon's UpperBlendSpace1P / UpperBlendSpace samples (clip, X, Y) and range
#[test]
#[ignore]
fn probe_upper_bs_1p() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let a = mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap();
    let w = std::env::var("MH_PROBE_WEAPON").unwrap_or("Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword".into());
    for fp in [true, false] {
        let p = a.upper_blend_space_for(&w, fp);
        println!("{} {p}", if fp { "1P" } else { "3P" });
        if let Some(bs) = a.blend_space(&p) {
            println!("  range {:?}..{:?} grid {:?}", bs.min, bs.max, bs.grid_n);
            for s in &bs.samples {
                println!("  {} x {:.1} y {:.1}", s.0.rsplit('/').next().unwrap_or(""), s.1, s.2);
            }
        }
    }
}

/// grip r1: component-space rotations of the root chain + right arm: sim 1P / 3P idle vs the raw 1P idle clip
#[test]
#[ignore]
fn probe_root_chain_rot() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim};
    use mordhau_core::ue::FVector;
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let bones = ["Global", "Position", "Hips", "LowerBack", "Spine", "Spine1", "RightShoulder", "RightArm", "RightForeArm", "RightHand", "RightWeapon"];
    let show = |tag: &str, sk: &mh_sim::pose::Skeleton, pose: &[mordhau_core::ue::FTransform]| {
        for b in bones {
            let i = sk.find(b).unwrap();
            let q = pose[i].rot;
            let (x, y, z) = (q.rotate(FVector::new(1.0, 0.0, 0.0)), q.rotate(FVector::new(0.0, 1.0, 0.0)), q.rotate(FVector::new(0.0, 0.0, 1.0)));
            let l = pose[i].loc;
            println!("{tag} {b:13} loc ({:6.1} {:6.1} {:6.1}) X ({:5.2} {:5.2} {:5.2}) Y ({:5.2} {:5.2} {:5.2}) Z ({:5.2} {:5.2} {:5.2})", l.x, l.y, l.z, x.x, x.y, x.z, y.x, y.y, y.z, z.x, z.y, z.z);
        }
    };
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = ld.geo.skeleton.clone();
    let clip = mh_assets::anim::decode(&src, "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_Idle_1P").unwrap();
    show("clip1P", &sk, &sk.sample(&clip, 0.0));
    show("ref   ", &sk, &sk.ref_pose());
    for fp in [true, false] {
        let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
        let mut floor = BoxWorld::new();
        floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
        let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
        s.dedicated_server = false;
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
        s.set_first_person(a, fp);
        s.set_view_target(a, fp, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
        for _ in 0..90 {
            s.step(&[]);
        }
        let pose = sk.to_component(&s.local_pose(a));
        show(if fp { "sim1P " } else { "sim3P " }, &sk, &pose);
    }
}

/// grip r1: the weapon mesh (physics::weapon_mesh) and every socket it resolves (skeleton's + mesh's), per weapon
/// (MH_PROBE_WEAPONS = comma list of BP paths)
#[test]
#[ignore]
fn probe_weapon_mesh_sockets() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rd = mh_pak::Reader::new(vfs.clone());
    let ws = std::env::var("MH_PROBE_WEAPONS").unwrap_or(LS.into());
    for w in ws.split(',') {
        let Some(mesh) = mh_sim::physics::weapon_mesh(&rd, w) else { println!("{w}: no mesh"); continue };
        let ex = rd.read(&mesh).unwrap_or_default();
        let skel = ex.iter().find(|e| e["Type"].as_str() == Some("SkeletalMesh")).map(|e| e["Properties"]["Skeleton"]["ObjectPath"].to_string());
        println!("{w}: mesh {mesh} skeleton {skel:?}");
        for (n, s) in mh_sim::physics::sockets(&rd, &mesh).unwrap() {
            let r = s.xf.rot;
            println!("   socket {n}: bone {} loc ({:.3} {:.3} {:.3}) rot q ({:.4} {:.4} {:.4} {:.4})", s.bone, s.xf.loc.x, s.xf.loc.y, s.xf.loc.z, r.x, r.y, r.z, r.w);
        }
    }
}

/// grip r1 inventory (state/grip_data.md): per weapon (MH_PROBE_WEAPONS) the class-default grip / offhand / 1P fields,
/// the sim's WeaponGeo, and in the 1P and 3P idle: the weapon actor in the RightHand / RightWeapon frames, the left
/// hand against the weapon's grip axis, and the right fingers' local rotation against the raw idle clip
#[test]
#[ignore]
fn probe_grip_inventory() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim};
    use mordhau_core::ue::FVector;
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rd = mh_pak::Reader::new(vfs.clone());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let ws = std::env::var("MH_PROBE_WEAPONS").unwrap_or(LS.into());
    let keys = ["Offset", "Grip", "Offhand", "Equip", "IK", "1P", "Cosmetic", "bIsRightHanded", "UpperBlendSpace", "LowerAnimation"];
    let ax = |q: mordhau_core::ue::FQuat| {
        let (x, y, z) = (q.rotate(FVector::new(1.0, 0.0, 0.0)), q.rotate(FVector::new(0.0, 1.0, 0.0)), q.rotate(FVector::new(0.0, 0.0, 1.0)));
        format!("X ({:6.3} {:6.3} {:6.3}) Y ({:6.3} {:6.3} {:6.3}) Z ({:6.3} {:6.3} {:6.3})", x.x, x.y, x.z, y.x, y.y, y.z, z.x, z.y, z.z)
    };
    for w in ws.split(',') {
        println!("=== {w}");
        let d = rd.defaults(w);
        for (k, v) in d.iter() {
            if keys.iter().any(|s| k.contains(s)) {
                let s = v.to_string();
                println!("  cdo {k} = {}", &s[..s.len().min(200)]);
            }
        }
        for fp in [false, true] {
            let ld = mh_sim::load::load(&m, vfs.clone(), &[w]).unwrap();
            let mut floor = BoxWorld::new();
            floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
            let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
            s.dedicated_server = false;
            let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
            s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
            s.set_first_person(a, fp);
            s.set_view_target(a, fp, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
            for _ in 0..90 {
                s.step(&[]);
            }
            let tag = if fp { "1P" } else { "3P" };
            let wg = s.geo.weapons.get(w).unwrap().clone();
            if !fp {
                println!("  geo rot_off {:?} rhe ({:.2} {:.2} {:.2}) grip ({:.2} {:.2} {:.2}) pitch {} right {} eo {:?}", wg.rotation_offset, wg.right_hand_equip_offset.x, wg.right_hand_equip_offset.y, wg.right_hand_equip_offset.z, wg.grip_location_local.x, wg.grip_location_local.y, wg.grip_location_local.z, s.geo.grip_pitch_right, wg.right_handed, wg.equipped_offset);
            }
            let posed = s.posed.borrow();
            let p = posed.get("A").unwrap();
            let wx = s.geo.weapon_world(p, &wg).unwrap();
            let bw = |b: &str| s.geo.bone_world(p, s.geo.skeleton.find(b).unwrap());
            for b in ["RightHand", "RightWeapon"] {
                let rel = wx.then(&bw(b).inverse());
                println!("  {tag} weapon in {b:11}: loc ({:7.3} {:7.3} {:7.3}) {}", rel.loc.x, rel.loc.y, rel.loc.z, ax(rel.rot));
            }
            let rw_in_h = bw("RightWeapon").then(&bw("RightHand").inverse());
            println!("  {tag} RightWeapon in RightHand: loc ({:7.3} {:7.3} {:7.3}) {}", rw_in_h.loc.x, rw_in_h.loc.y, rw_in_h.loc.z, ax(rw_in_h.rot));
            // hands in the weapon frame: distance off the blade (Z) axis and position along it
            let inv = wx.inverse();
            for h in ["RightHand", "RightHandFinger03_02", "LeftHand", "LeftHandFinger03_02"] {
                let l = inv.apply(bw(h).loc);
                println!("  {tag} {h:20} weapon-local ({:7.2} {:7.2} {:7.2}) off-axis {:5.2} cm", l.x, l.y, l.z, (l.x * l.x + l.y * l.y).sqrt());
            }
        }
    }
}

/// first-person r3 (fp_anim_audit #1, bounce): A strikes, B parries in time; A's right hand in A's 1P camera frame
/// over the blocked motion (run twice: with and without MH_NO_BOUNCE for before / after)
#[test]
#[ignore]
fn probe_bounce() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim, SimInput};
    use mordhau_core::ue::{FQuat, FVector};
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let w = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";
    let ld = mh_sim::load::load(&m, vfs.clone(), &[w]).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let fp = std::env::var("MH_PROBE_3P").is_err();
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: w.into(), location: FVector::new(150.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
    s.set_first_person(a, fp);
    s.set_view_target(a, fp, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    let parry_at: usize = std::env::var("MH_PROBE_PARRY_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(97);
    for n in 0..200 {
        let mut inp = vec![];
        if n == 60 {
            inp.push((a, SimInput { attack: Some((0, 0.0)), ..Default::default() }));
        }
        if n == parry_at {
            inp.push((b, SimInput { parry: Some(0), ..Default::default() }));
        }
        s.step(&inp);
        if n < 100 || n % 3 != 0 {
            continue;
        }
        let g = s.combat.fighters[a].geom.unwrap();
        let cam = mordhau_core::ue::FTransform::new(FQuat::from_rotator(g.camera_rot.0, g.camera_rot.1, 0.0), g.camera_loc);
        let inv = cam.inverse();
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let rh = inv.apply(s.geo.bone_world(p, s.geo.skeleton.find("RightHand").unwrap()).loc);
        let tip = s.geo.weapons.get(w).and_then(|wg| s.geo.weapon_world(p, wg)).map(|x| inv.apply(x.apply(FVector::new(0.0, 0.0, 100.0))));
        drop(posed);
        println!("t {:.2} {:?} RH ({:5.1} {:5.1} {:5.1}) tip {:?}", (n - 60) as f32 / 60.0, s.combat.cur_m(a).map(|m| m.kind()), rh.x, rh.y, rh.z, tip.map(|t| (t.x as i32, t.y as i32, t.z as i32)));
    }
}

/// first-person r3 (fp_anim_items #5 #7 #8): the perspective values the MotionAnim now reads, for the greatsword's
/// right strike and the 2H sword parry
#[test]
#[ignore]
fn probe_fp_values() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let w = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";
    let ld = mh_sim::load::load(&m, vfs.clone(), &[w]).unwrap();
    let a = mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap();
    let sbp = "Mordhau/Content/Mordhau/Blueprints/Motions/AttackMotions/BP_Greatsword_RightStrikeMotion";
    let d = ld.spec.motion_defs.get(sbp).expect("strike def");
    let x = a.attack_def_1p(d.attack(), sbp, "UStrikeMotion");
    let t = d.attack();
    println!("strike 3P: feint rate {} min {} off {} | hit rate {} blendout {} | miss rate {} clamp {:?}", t.feint_anim_rate, t.feint_anim_minimum_duration, t.feint_anim_duration_offset, t.successful_hit_play_rate, t.successful_hit_blend_out_anim_time, t.miss_recovery_to_play_rate, t.miss_recovery_play_rate_clamp);
    println!("strike 1P: feint rate {} min {} off {} | hit rate {} blendout {} | miss rate {} clamp {:?}", x.feint_anim_rate, x.feint_anim_minimum_duration, x.feint_anim_duration_offset, x.successful_hit_play_rate, x.successful_hit_blend_out_anim_time, x.miss_recovery_to_play_rate, x.miss_recovery_play_rate_clamp);
    let pbp = "Mordhau/Content/Mordhau/Blueprints/Motions/ParryMotions/BP_2HSwordParryMotion";
    if let Some(pd) = ld.spec.motion_defs.get(pbp) {
        println!("parry 3P {:?}", a.parry_rates(pbp, pd.parry(), false));
        println!("parry 1P {:?}", a.parry_rates(pbp, pd.parry(), true));
    } else {
        println!("no parry def {pbp}; keys with Parry: {:?}", ld.spec.motion_defs.keys().filter(|k| k.contains("Parry")).collect::<Vec<_>>());
    }
}

/// grip r2 / fp_view: the raw 1P idle clip with and without the USkeleton BoneTree's Skeleton-mode translation
/// retargeting (UMA_Master_Skeleton BoneTree: 29 bones incl. LowerBack, Spine, Spine1, Neck, head, RightHand,
/// LeftHand take the target mesh's reference translation): RightHand against the 1P camera
/// (Spine1 + R x (0, 0, 41.625), R = Q(Position) x Roll(-5.27) x FRotator(90, 0, -90)), camera frame X fwd / Y right / Z up
#[test]
#[ignore]
fn probe_retarget_camera_hand() {
    use mordhau_core::ue::{FQuat, FVector};
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let ld = mh_sim::load::load(&m, vfs.clone(), &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    let rd = mh_pak::Reader::new(vfs.clone());
    let ex = rd.read("Mordhau/Content/UMA/UMA/Master/UMA_Master_Skeleton").unwrap();
    let bt = ex.iter().find(|e| e["Type"].as_str() == Some("Skeleton")).unwrap()["Properties"]["BoneTree"].as_array().unwrap().clone();
    let skel_mode: Vec<bool> = bt.iter().map(|b| b["TranslationRetargetingMode"].as_str().unwrap_or("").ends_with("Skeleton")).collect();
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let clip = std::env::var("MH_PROBE_CLIP").unwrap_or("Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_Idle_1P".into());
    let a = mh_assets::anim::decode(&src, &clip).unwrap();
    let raw = sk.sample_local(&a, 0.0);
    let mut ret = raw.clone();
    for (i, t) in ret.iter_mut().enumerate() {
        if skel_mode.get(i).copied().unwrap_or(false) {
            t.loc = sk.ref_local[i].loc;
        }
    }
    for (tag, local) in [("anim translations", raw), ("Skeleton-mode retarget", ret)] {
        let cs = sk.to_component(&local);
        let w = |b: &str| cs[sk.find(b).unwrap()].then(&ld.geo.mesh_xf);
        let pos = w("Position");
        let r = pos.rot.mul(FQuat::from_rotator(0.0, 0.0, -5.27)).mul(FQuat::from_rotator(90.0, 0.0, -90.0));
        let cam = w("Spine1").loc + r.rotate(FVector::new(0.0, 0.0, 41.625));
        let ci = r.inverse();
        for b in ["RightHand", "LeftHand", "RightForeArm", "head", "Spine1"] {
            let l = ci.rotate(w(b).loc - cam);
            let down = (-l.z).atan2(l.x).to_degrees();
            let right = l.y.atan2(l.x).to_degrees();
            println!("{tag:24} {b:12} cam-frame ({:6.1} {:6.1} {:6.1})  {:5.1} deg right {:5.1} deg down  dist {:5.1}", l.x, l.y, l.z, right, down, l.length());
        }
    }
}

/// fp_anim_items FP-6b: a mace + kite shield right strike in 1P: LeftTorsoBlendWeight and the left hand (shield arm)
/// in the 1P camera frame over the attack
#[test]
#[ignore]
fn probe_left_torso() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim, SimInput};
    use mordhau_core::ue::{FQuat, FVector};
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let w = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/OneHanded/BP_Mace";
    let kite = "Mordhau/Content/Mordhau/Blueprints/Equipment/Shields/BP_KiteShield";
    let ld = mh_sim::load::load(&m, vfs.clone(), &[w, kite]).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.into(), left: kite.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
    s.set_first_person(a, std::env::var("MH_PROBE_3P").is_err());
    s.set_view_target(a, std::env::var("MH_PROBE_3P").is_err(), false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    for n in 0..170 {
        let inp = if n == 60 { vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })] } else { vec![] };
        s.step(&inp);
        if n < 57 || n % 6 != 0 {
            continue;
        }
        let g = s.combat.fighters[a].geom.unwrap();
        let cam = mordhau_core::ue::FTransform::new(FQuat::from_rotator(g.camera_rot.0, g.camera_rot.1, 0.0), g.camera_loc);
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let lh = cam.inverse().apply(s.geo.bone_world(p, s.geo.skeleton.find("LeftHand").unwrap()).loc);
        drop(posed);
        println!("t {:.2} {:?} lt_w {:.3} LH ({:5.1} {:5.1} {:5.1})", (n as f32 - 60.0) / 60.0, s.combat.cur_m(a).map(|m| m.kind()), s.fanim[a].left_torso_w, lh.x, lh.y, lh.z);
    }
}

/// first-person r3 (held parry): the parry motion's stage / recovery over a held RMB (parry input, no release) for a
/// weapon (MH_PROBE_WEAPON, default greatsword; MH_PROBE_LEFT = a shield path)
#[test]
#[ignore]
fn probe_held_parry() {
    use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
    use mh_sim::{FighterDesc, Sim, SimInput};
    use mordhau_core::ue::FVector;
    use std::rc::Rc;
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let w = std::env::var("MH_PROBE_WEAPON").unwrap_or("Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword".into());
    let left = std::env::var("MH_PROBE_LEFT").unwrap_or_default();
    let mut ws = vec![w.as_str()];
    if !left.is_empty() {
        ws.push(left.as_str());
    }
    let ld = mh_sim::load::load(&m, vfs.clone(), &ws).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.clone(), left: left.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    for n in 0..140 {
        // MH_PROBE_WALL: R (toggle_mode = the shield wall) instead of RMB
        let wall = std::env::var("MH_PROBE_WALL").is_ok();
        let inp = if n == 30 { vec![(a, if wall { SimInput { toggle_mode: true, ..Default::default() } } else { SimInput { parry: Some(0), ..Default::default() } })] } else { vec![] };
        s.step(&inp);
        if n < 30 || n % 4 != 0 {
            continue;
        }
        let mm = s.combat.cur_m(a).unwrap();
        let p = mm.parry();
        println!("t {:.2} {} stage {:?} holdable {:?} recovery_type {:?} up {:?} | anim pos {:.3}", (n - 30) as f32 / 60.0, mm.kind(), p.map(|p| p.stage), p.map(|p| p.b_is_block_holdable), p.map(|p| p.recovery_type), p.map(|p| p.parry_up_time), s.fanim[a].ma.position);
    }
}

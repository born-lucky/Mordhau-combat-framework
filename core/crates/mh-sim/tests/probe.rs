//! Contact-time probes for the parity rows (rust-combat r5; run on demand: `cargo test -p mh-sim --test probe --
//! --ignored --nocapture`). A swings at a defender `d` cm in front (straight; stabs also with the STAB_AIMS of
//! mh-parity traced.rs) on the ported graph; prints the first contact after the attack start per distance.
//! MH_PROBE_LOWER=0: reference-pose lower body (r4); MH_PROBE_SERVER=0: no dedicated-server hips override;
//! MH_PROBE_WEAPONS=a,b (base names); MH_PROBE_DT (default 0.004).

use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
use mh_sim::{FighterDesc, Sim, SimInput};
use mordhau_core::ue::FVector;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

const SPEC: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/";
const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
const AIMS: [(f32, f64); 6] = [(0.0, 0.0), (-8.0, 0.0), (8.0, 0.0), (0.0, -10.0), (-8.0, -10.0), (8.0, -10.0)];

fn path_of(w: &str) -> String {
    for d in ["Polearms", "TwoHandedSword", "OneHanded", "TwoHanded", "Spears", "Axes", "Blunt", "Swords"] {
        let p = format!("{SPEC}{d}/{w}");
        if std::path::Path::new(&format!("{}/../../../extract/json/{p}.json", env!("CARGO_MANIFEST_DIR"))).exists() {
            return p;
        }
    }
    format!("{SPEC}{w}")
}

#[test]
#[ignore]
fn probe_contacts() {
    let matrix = match mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) {
        Ok(m) => m,
        Err(_) => return eprintln!("SKIP"),
    };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let dt: f32 = std::env::var("MH_PROBE_DT").ok().and_then(|v| v.parse().ok()).unwrap_or(0.004);
    let lower = std::env::var("MH_PROBE_LOWER").map(|v| v != "0").unwrap_or(true);
    let server = std::env::var("MH_PROBE_SERVER").map(|v| v != "0").unwrap_or(true);
    let ws = std::env::var("MH_PROBE_WEAPONS").unwrap_or("BP_Poleaxe,BP_Billhook,BP_Greatsword,BP_ExecutionerSword".into());
    let moves: Vec<i64> = std::env::var("MH_PROBE_MOVES").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or(vec![0, 2]);
    for w in ws.split(',') {
        let wp = path_of(w);
        for &mv in &moves {
            let mut row = Vec::new();
            for d in [110.0f32, 130.0, 150.0, 170.0, 190.0, 210.0] {
                let aims: &[(f32, f64)] = if mv == 0 { &AIMS[..1] } else { &AIMS };
                let mut best: Option<f64> = None;
                for &aim in aims {
                    let ld = mh_sim::load::load(&matrix, vfs.clone(), &[wp.as_str(), LS]).unwrap();
                    let mut floor = BoxWorld::new();
                    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
                    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), dt);
                    s.dedicated_server = server;
                    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: wp.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
                    let _b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(d, 0.0, 100.0), yaw: 180.0, ..Default::default() });
                    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
                    if !lower {
                        for f in s.fanim.iter_mut() {
                            f.lower = None;
                        }
                    }
                    if std::env::var("MH_PROBE_NOCOSMETIC").map(|v| v == "1").unwrap_or(false) {
                        for f in s.fanim.iter_mut() {
                            f.cosmetic = ((0.0, 0.0, 0.0), FVector::ZERO);
            f.proc.weapon_base_loc = FVector::ZERO;
            f.proc.weapon_base_rot = (0.0, 0.0, 0.0);
                        }
                    }
                    let mut start = None;
                    let mut hit = None;
                    // MH_PROBE_ALT: SwitchMode first, the attack once the mode-switch motion is over
                    let alt = std::env::var("MH_PROBE_ALT").is_ok();
                    let atk_n = if alt { (2.0 / dt) as i64 } else { 1 };
                    for n in 0..((2.5 / dt) as i64 + atk_n) {
                        let mut inp = Vec::new();
                        if n == 0 && aim != (0.0, 0.0) {
                            inp.push((a, SimInput { yaw: Some(aim.0), look_up: Some(aim.1), ..Default::default() }));
                        }
                        if n == 0 && std::env::var("MH_PROBE_ALT").is_ok() {
                            inp.push((a, SimInput { switch_mode: true, ..Default::default() }));
                        }
                        if n == atk_n {
                            if alt && n == atk_n {
                                eprintln!("alt mode {} motion {:?}", s.combat.fighters[a].alternate_mode, s.combat.cur_m(a).map(|m| m.kind()));
                            }
                            inp.push((a, SimInput { attack: Some((mv, 0.0)), ..Default::default() }));
                        }
                        s.step(&inp);
                        if start.is_none() && n >= atk_n {
                            start = s.combat.cur_m(a).filter(|m| m.is_attack()).map(|m| m.start_time);
                        }
                        let (ev, _) = s.drain();
                        if ev.iter().any(|e| matches!(e["kind"].as_str(), Some("hit") | Some("parry") | Some("chamber"))) {
                            hit = start.map(|st| s.combat.now - st);
                            break;
                        }
                    }
                    if let Some(h) = hit {
                        best = Some(best.map(|b: f64| b.min(h)).unwrap_or(h));
                    }
                }
                row.push(format!("{d}:{}", best.map(|b| format!("{b:.3}")).unwrap_or("-".into())));
            }
            println!("PROBE {w} mv{mv} lower={lower} server={server}: {}", row.join(" "));
        }
    }
}

/// per-tick diagnostics of one swing (MH_PROBE_WEAPONS = one base name, MH_PROBE_MOVES = one move, MH_PROBE_DIST)
#[test]
#[ignore]
fn probe_diag() {
    let matrix = match mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) {
        Ok(m) => m,
        Err(_) => return eprintln!("SKIP"),
    };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let rec = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
    let dt = 0.004f32;
    let w = std::env::var("MH_PROBE_WEAPONS").unwrap_or("BP_Poleaxe".into());
    let mv: i64 = std::env::var("MH_PROBE_MOVES").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let d: f32 = std::env::var("MH_PROBE_DIST").ok().and_then(|v| v.parse().ok()).unwrap_or(500.0);
    let wp = path_of(&w);
    let ld = mh_sim::load::load(&matrix, vfs.clone(), &[wp.as_str(), LS]).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), dt);
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: wp.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let _b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(d, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
    if std::env::var("MH_PROBE_NOCOSMETIC").map(|v| v == "1").unwrap_or(false) {
        for f in s.fanim.iter_mut() {
            f.cosmetic = ((0.0, 0.0, 0.0), FVector::ZERO);
            f.proc.weapon_base_loc = FVector::ZERO;
            f.proc.weapon_base_rot = (0.0, 0.0, 0.0);
        }
    }
    println!("cosmetic {:?}", s.fanim[a].cosmetic);
    let wg = s.geo.weapons.get(&wp).cloned().unwrap();
    println!("weapon geo: start {:?} end {:?} grip {:?} offset {:?} rot {:?} right {}", wg.trace_start, wg.trace_end, wg.grip_location_local, wg.right_hand_equip_offset, wg.rotation_offset, wg.right_handed);
    let mut start = None;
    for n in 0..((1.6 / dt) as i64) {
        let inp = if n == 1 { vec![(a, SimInput { attack: Some((mv, 0.0)), ..Default::default() })] } else { vec![] };
        s.step(&inp);
        if start.is_none() {
            start = s.combat.cur_m(a).filter(|m| m.is_attack()).map(|m| m.start_time);
        }
        let Some(st) = start else { continue };
        if n % 10 != 0 {
            continue;
        }
        let m = s.combat.cur_m(a);
        let (stage, we, re) = m.and_then(|m| m.attack()).map(|x| (x.stage, x.windup_end - st, x.release_end - st)).unwrap_or((-1, 0.0, 0.0));
        let p = s.posed.borrow();
        let pa = &p["A"];
        let ww = s.geo.weapon_world(pa, &wg).unwrap();
        let te = wg.trace_end.map(|x| ww.apply(x)).unwrap_or_default();
        let ts = wg.trace_start.map(|x| ww.apply(x)).unwrap_or_default();
        let hips = s.geo.skeleton.find("Hips").map(|h| s.geo.bone_world(pa, h).loc).unwrap_or_default();
        let ma = &s.fanim[a].ma;
        if n == 10 {
            let am = m.unwrap();
            let at = am.attack().unwrap();
            println!("motion bp {} native {} windup_curve '{}' release_curve '{}' ty {} blend_in {:.3} offset {:.3} early {:.3}", am.bp, at.native, at.windup_curve, at.release_curve, at.ty, ma.blend_in, ma.offset, ma.early_release);
        }
        println!("  weapon base {:?} {:?}", s.fanim[a].proc.weapon_base_loc, s.fanim[a].proc.weapon_base_rot);
        println!("t {:.3} stage {stage} we {we:.3} re {re:.3} seq {} pos {:.3} start ({:.0},{:.0},{:.0}) end ({:.0},{:.0},{:.0}) hips ({:.0},{:.0},{:.0})", s.combat.now - st, ma.seq, ma.position, ts.x, ts.y, ts.z, te.x, te.y, te.z, hips.x, hips.y, hips.z);
    }
}

/// the sockets of every part mesh of a weapon (MH_PROBE_WEAPONS = one base name)
#[test]
#[ignore]
fn probe_parts() {
    let Ok(vfs) = mh_pak::Vfs::mount_default() else { return };
    let rd = mh_pak::Reader::new(Arc::new(vfs));
    for w in std::env::var("MH_PROBE_WEAPONS").unwrap_or("BP_Poleaxe".into()).split(',') {
        let wp = path_of(w);
        let d = rd.defaults(&wp);
        println!("== {w} mesh {:?}", mh_sim::physics::weapon_mesh(&rd, &wp));
        for skin in d.get("Skins").and_then(|v| v.as_array()).into_iter().flatten().take(1) {
            for pt in skin["PartTypes"].as_array().into_iter().flatten() {
                for part in pt["Parts"].as_array().into_iter().flatten().take(1) {
                    let cls = mh_sim::physics::strip(part["ObjectPath"].as_str().unwrap_or(""));
                    let pd = rd.defaults(&cls);
                    let m = pd.get("SkeletalMesh").and_then(|v| v["ObjectPath"].as_str()).map(mh_sim::physics::strip).unwrap_or_default();
                    let so = mh_sim::physics::sockets(&rd, &m).unwrap_or_default();
                    let s: Vec<String> = so.iter().map(|(n, s)| format!("{n}@{}({:.1},{:.1},{:.1})", s.bone, s.xf.loc.x, s.xf.loc.y, s.xf.loc.z)).collect();
                    println!("  {} {} {:?}", cls.rsplit('/').next().unwrap(), m.rsplit('/').next().unwrap(), s);
                }
            }
        }
    }
}

/// what the pak reader gives for an AnimSequence's curves (MH_PROBE_CLIP)
#[test]
#[ignore]
fn probe_clip_curves() {
    let Ok(vfs) = mh_pak::Vfs::mount_default() else { return };
    let rd = mh_pak::Reader::new(Arc::new(vfs));
    let clip = std::env::var("MH_PROBE_CLIP").unwrap_or("Mordhau/Content/Mordhau/Animations/RawClips/2H/Polearm/2H_Polearm_RightStrike".into());
    let src = mh_assets::pak_source::PakSource::new(Arc::new(mh_pak::Vfs::mount_default().unwrap()));
    let a = mh_assets::anim::decode(&src, &clip).unwrap();
    for c in &a.curves {
        println!("curve {} pre {} post {} keys {:?}", c.name, c.pre, c.post, c.keys);
    }
    let ex = rd.read(&clip).unwrap();
    for e in &ex {
        if e["Type"].as_str() == Some("AnimSequence") {
            let keys: Vec<&String> = e.as_object().unwrap().keys().collect();
            println!("export keys {keys:?}");
            println!("props {:?}", e["Properties"].as_object().map(|o| o.keys().collect::<Vec<_>>()));
            println!("curve data {}", serde_json::to_string(&e["CompressedCurveData"]).unwrap().chars().take(400).collect::<String>());
        }
    }
}

/// parents of some bones in the character skeleton
#[test]
#[ignore]
fn probe_bone_parents() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs, &[LS]).unwrap();
    let sk = &ld.geo.skeleton;
    for b in ["RightWeapon", "RightHand", "LeftWeapon", "LeftHand", "RightForearm", "Position", "Spine1"] {
        let i = sk.find(b).unwrap();
        let mut chain = vec![];
        let mut p = sk.parents[i];
        while p >= 0 {
            chain.push(sk.names[p as usize].clone());
            p = sk.parents[p as usize];
        }
        println!("{b}: {chain:?}");
    }
}

/// r6 item 3: the polearm stab clip at montage-relevant times: our decoded local tracks (printed for the glb compare),
/// and RightWeapon's component location with / without the Skeleton-mode translation retarget (mesh vs USkeleton
/// reference pose)
#[test]
#[ignore]
fn probe_stab_retarget() {
    use mh_sim::pose::Skeleton;
    let Ok(vfs) = mh_pak::Vfs::mount_default() else { return };
    let vfs = Arc::new(vfs);
    let rd = mh_pak::Reader::new(vfs.clone());
    let (_pa, mesh, _xf) = mh_sim::physics::character_mesh(&rd).unwrap();
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let ex = rd.read(&mesh).unwrap();
    let skel_path = mh_sim::physics::strip(ex.iter().find(|e| e["Type"].as_str() == Some("SkeletalMesh")).unwrap()["Properties"]["Skeleton"]["ObjectPath"].as_str().unwrap());
    let (rs, modes) = mh_assets::skeletal_mesh::skeleton(&src, &skel_path).unwrap();
    let sk = Skeleton::from_ref(&rs);
    let m = mh_assets::skeletal_mesh::lod0(&src, &mesh).unwrap();
    let msk = Skeleton::from_ref(&m.ref_skeleton);
    println!("mesh {mesh} ({} bones) skeleton {skel_path} ({} bones)", msk.names.len(), sk.names.len());
    let clip = std::env::var("MH_PROBE_CLIP").unwrap_or("Mordhau/Content/Mordhau/Animations/RawClips/2H/Polearm/2H_Polearm_RightStab".into());
    let a = mh_assets::anim::decode(&src, &clip).unwrap();
    println!("clip {clip}: frames {} len {} additive {} codec {} tracks {}", a.num_frames, a.sequence_length, a.additive_anim_type, a.codec, a.tracks.len());
    let bones = ["Hips", "Spine1", "RightShoulder", "RightArm", "RightForeArm", "RightHand", "RightWeapon"];
    for b in bones {
        let i = sk.find(b).unwrap();
        let mi = msk.find(b);
        println!("{b}: mode {:?} skel ref t {:?} mesh ref t {:?}", modes.get(i), sk.ref_local[i].loc, mi.map(|j| msk.ref_local[j].loc));
    }
    for t in [0.0f32, 0.3, 0.5, 0.6, 0.8] {
        let raw = sk.sample_local(&a, t);
        // the Skeleton-mode retarget (FAnimationRuntime::RetargetBoneTransform as CUE4Parse CAnimSequence.cs:111-160)
        let mut ret_mesh = raw.clone();
        let mut ret_skel = raw.clone();
        for (i, md) in modes.iter().enumerate() {
            if md.ends_with("::Skeleton") && i < raw.len() {
                if let Some(j) = msk.find(&sk.names[i]) {
                    ret_mesh[i].loc = msk.ref_local[j].loc;
                }
                ret_skel[i].loc = sk.ref_local[i].loc;
            }
        }
        let w = sk.find("RightWeapon").unwrap();
        let (c0, c1, c2) = (sk.to_component(&raw)[w].loc, sk.to_component(&ret_mesh)[w].loc, sk.to_component(&ret_skel)[w].loc);
        println!("t {t}: RightWeapon cs raw {c0:?} | retarget(mesh ref) {c1:?} | retarget(skel ref) {c2:?}");
        for b in bones {
            let i = sk.find(b).unwrap();
            let q = raw[i].rot;
            println!("  {b} q [{:.5} {:.5} {:.5} {:.5}] t [{:.3} {:.3} {:.3}]", q.x, q.y, q.z, q.w, raw[i].loc.x, raw[i].loc.y, raw[i].loc.z);
        }
    }
}

/// r10: the parry pose by look-up and block type (RightWeapon / RightHand component locations 0.3 s into the parry)
#[test]
#[ignore]
fn probe_parry_poses() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let w = std::env::var("MH_PROBE_WEAPONS").unwrap_or(LS.into());
    for bt in [0i64, 1] {
        for lu in [-50.0f64, 0.0, 50.0] {
            let ld = mh_sim::load::load(&m, vfs.clone(), &[w.as_str()]).unwrap();
            let rec = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).unwrap();
            let mut floor = BoxWorld::new();
            floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
            let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
            s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap());
            let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
            for _ in 0..30 {
                s.step(&[(a, SimInput { look_up: Some(lu), ..Default::default() })]);
            }
            s.step(&[(a, SimInput { look_up: Some(lu), parry: Some(bt), ..Default::default() })]);
            for _ in 0..18 {
                s.step(&[(a, SimInput { look_up: Some(lu), ..Default::default() })]);
            }
            let posed = s.posed.borrow();
            let p = posed.get("A").unwrap();
            let rw = s.geo.bone_world(p, s.geo.skeleton.find("RightWeapon").unwrap()).loc;
            let tip = p.tracer.cur_end;
            println!("PARRY bt {bt} look {lu}: motion {:?} RightWeapon ({:.1},{:.1},{:.1}) tip ({:.1},{:.1},{:.1})", s.combat.cur_m(a).map(|m| m.kind()), rw.x, rw.y, rw.z, tip.x, tip.y, tip.z);
        }
    }
}

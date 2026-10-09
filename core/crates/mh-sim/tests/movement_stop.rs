//! EVD_MOV_015: starting and stopping a walk vs the reader record motion1 (state/live_rec/motion1.jsonl via
//! state/gauntlet/turnfeel/m1_rows.json). The record has no key state, so the move key is taken as pressed from the
//! first frame the real speed leaves 0 and released from the first frame the real speed drops below its peak; the
//! direction is the real velocity's while it moves. Our ExeMovement (mh-character) then has to reproduce the real
//! speed curve.
use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
use mh_sim::{FighterDesc, Sim};
use mordhau_core::ue::FVector;
use std::rc::Rc;
use std::sync::Arc;

const GS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";

fn sim() -> Option<(Sim, usize)> {
    let m = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).ok()?;
    let vfs = Arc::new(mh_pak::Vfs::mount_default().ok()?);
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).ok()?;
    let ld = mh_sim::load::load(&m, vfs.clone(), &[GS]).ok()?;
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: GS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    Some((s, a))
}

/// (t, ours, real) speed over the record's first walk (t 0 .. 1.6 s)
fn replay_walk() -> Option<Vec<(f64, f64, f64)>> {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state/gauntlet/turnfeel/m1_rows.json");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    let g = |r: &serde_json::Value, k: &str| r[k].as_f64().unwrap_or(0.0);
    let (mut s, a) = sim()?;
    s.movers[a].armor_speed = 0.8134;
    // a world older than RagdollFallingGetUpDuration: IsRagdollFallingOrGettingUp rva=0x1487ca0 ignores move input
    // while TimeSeconds < 0 + that duration (exe behaviour), which delayed the start by 0.17 s with a 1 s warm-up
    for _ in 0..240 {
        s.step(&[(a, mh_sim::SimInput { yaw: Some(g(&rows[0], "yaw") as f32), turn_applied_by_client: true, ..Default::default() })]);
    }
    let (mut peak, mut released, mut dir) = (0.0f64, false, (0.0f32, 0.0f32));
    let mut out = vec![];
    for (i, r) in rows.iter().enumerate() {
        let t = g(r, "t");
        if t > 1.6 {
            break;
        }
        let dt = if i == 0 { 1.0 / 60.0 } else { (t - g(&rows[i - 1], "t")) as f32 };
        let (vx, vy) = (g(r, "vx"), g(r, "vy"));
        let real = vx.hypot(vy);
        if real > peak {
            peak = real;
        } else if peak > 100.0 && real < peak * 0.8 {
            released = true;
        }
        let yaw = g(r, "yaw") as f32;
        if real > 1.0 && !released {
            let ang = (vy as f32).atan2(vx as f32) - yaw.to_radians();
            dir = (ang.cos(), ang.sin());
        }
        let (fwd, right) = if peak > 1.0 && !released { dir } else { (0.0, 0.0) };
        s.step_dt(&[(a, mh_sim::SimInput { yaw: Some(yaw), turn_applied_by_client: true, fwd, right, ..Default::default() })], dt.max(1e-4));
        let v = s.movers[a].velocity;
        out.push((t, (v.x as f64).hypot(v.y as f64), real));
    }
    Some(out)
}

#[test]
#[ignore]
fn probe_walk_start_stop() {
    let Some(out) = replay_walk() else { return };
    for (t, ours, real) in out.iter().filter(|x| x.0 > 0.18 && x.0 < 0.45) {
        println!("t {t:.3}  ours {ours:6.1}  real {real:6.1}");
    }
}

/// the first record time a speed series crosses `v` upward (`up`) or downward
fn cross(out: &[(f64, f64, f64)], ours: bool, v: f64, up: bool) -> Option<f64> {
    out.iter().find(|x| { let s = if ours { x.1 } else { x.2 }; if up { s >= v } else { x.0 > 0.6 && s <= v } }).map(|x| x.0)
}

#[test]
fn walk_start_and_stop_match_the_record() {
    let Some(out) = replay_walk() else { return };
    let tol = 2.5 / 60.0;
    for (v, up) in [(100.0, true), (200.0, true), (100.0, false), (20.0, false)] {
        let (o, r) = (cross(&out, true, v, up).unwrap(), cross(&out, false, v, up).unwrap());
        assert!((o - r).abs() <= tol, "{} {v} cm/s: ours {o:.3} s, record {r:.3} s", if up { "reach" } else { "drop below" });
    }
}

/// Gauntlet gap 1 (critic 2026-10-07): the 1P lower-body legs over motion1's first walk vs the record, component space.
/// The walk's phase is inherited from the running idle clock (Locomotion sync group, normalized-time sync: the clips
/// carry no AuthoredSyncMarkers): across 62 walk starts in state/live_rec/everything.json the first foot to lift and
/// its delay scatter (L and R, 0.05 - 0.4 s), so there is no reset. Our idle clock starts at spawn, the record's long
/// before, so the replay sets the group's normalized time once before the walk (0.93 at t 0.205, fitted) and then
/// everything must follow. Input key-style: D from 0.221, W+D from 0.37, W from 0.90, released at 1.005.
#[test]
fn first_person_walk_legs_match_the_record_once_in_phase() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state");
    let (Ok(rows_txt), Ok(rec_txt)) = (std::fs::read_to_string(root.join("gauntlet/turnfeel/m1_rows.json")), std::fs::read_to_string(root.join("live_rec/motion1.jsonl"))) else { return };
    let rows: Vec<serde_json::Value> = serde_json::from_str(&rows_txt).unwrap();
    let mut lines = rec_txt.lines();
    let names: Vec<String> = serde_json::from_str::<serde_json::Value>(lines.next().unwrap()).unwrap()["bone_names"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    let rec: Vec<serde_json::Value> = lines.filter_map(|l| serde_json::from_str(l).ok()).collect();
    let Some((mut s, a)) = sim() else { return };
    let Ok(vfs) = mh_pak::Vfs::mount_default() else { return };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(Arc::new(vfs)).unwrap());
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    s.movers[a].armor_speed = 0.8134;
    let g = |r: &serde_json::Value, k: &str| r[k].as_f64().unwrap_or(0.0);
    for _ in 0..240 {
        s.step(&[(a, mh_sim::SimInput { yaw: Some(g(&rows[0], "yaw") as f32), look_up: Some(g(&rows[0], "look")), turn_applied_by_client: true, ..Default::default() })]);
    }
    // EVD_MOV_019: CurrentLowerBodyIdleOffset reaches the record's 33 (greatsword LowerAnimation MetaData ParamA) and
    // DirectionOffset stays 0 in first person (record 0 in every motion1 frame)
    if let Some(lb) = s.fanim[a].lower.as_ref() {
        let r0 = rec[0]["anim"]["CurrentLowerBodyIdleOffset"].as_f64().unwrap_or(-1.0);
        assert!((lb.current_idle_offset - r0).abs() < 0.01, "CurrentLowerBodyIdleOffset ours {} record {r0}", lb.current_idle_offset);
        assert_eq!(lb.direction_offset, 0.0);
    }
    let sk = s.geo.skeleton.clone();
    let bones = ["LeftFoot", "RightFoot", "LeftLeg", "RightLeg"];
    let (mut sum, mut n, mut phased) = (vec![0.0f64; 4], 0, false);
    let (mut sv_worst, mut oz_worst) = (0.0f64, 0.0f64);
    for (i, r) in rows.iter().enumerate() {
        let t = g(r, "t");
        if t > 1.2 {
            break;
        }
        if !phased && t >= 0.205 {
            if let Some(lb) = s.fanim[a].lower.as_mut() {
                lb.sync.norm = 0.93;
            }
            phased = true;
        }
        let dt = if i == 0 { 1.0 / 60.0 } else { (t - g(&rows[i - 1], "t")) as f32 };
        let (fwd, right) = if t < 0.221 { (0.0, 0.0) } else if t < 0.37 { (0.0, 1.0) } else if t < 0.90 { (1.0, 1.0) } else if t < 1.005 { (1.0, 0.0) } else { (0.0, 0.0) };
        s.step_dt(&[(a, mh_sim::SimInput { yaw: Some(g(r, "yaw") as f32), look_up: Some(g(r, "look")), turn_applied_by_client: true, fwd, right, ..Default::default() })], dt.max(1e-4));
        if t < 0.2 {
            continue;
        }
        let real = rec.iter().min_by(|x, y| (g(x, "t") - t).abs().partial_cmp(&(g(y, "t") - t).abs()).unwrap()).unwrap();
        if let Some(lb) = s.fanim[a].lower.as_ref() {
            sv_worst = sv_worst.max((lb.smoothed_velocity - real["anim"]["SmoothedVelocity"].as_f64().unwrap_or(0.0)).abs());
            oz_worst = oz_worst.max((lb.one_to_zero_at_walk_speed - real["anim"]["OneToZeroAtWalkSpeed"].as_f64().unwrap_or(1.0)).abs());
        }
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        for (k, b) in bones.iter().enumerate() {
            let (Some(i_ours), Some(i_real)) = (sk.find(b), names.iter().position(|x| x == b)) else { continue };
            let o = p.bones[i_ours].loc;
            let rb = &real["bones"][i_real];
            let rv = FVector::new(rb[4].as_f64().unwrap() as f32, rb[5].as_f64().unwrap() as f32, rb[6].as_f64().unwrap() as f32);
            sum[k] += (o - rv).length() as f64;
        }
        n += 1;
    }
    let means: Vec<f64> = sum.iter().map(|x| x / n.max(1) as f64).collect();
    println!("legs vs record (cm, mean over {n} frames): {:?}", bones.iter().zip(means.iter()).map(|(b, m)| format!("{b} {m:.1}")).collect::<Vec<_>>());
    println!("SmoothedVelocity worst {sv_worst:.2}, OneToZeroAtWalkSpeed worst {oz_worst:.3}");
    assert!(means.iter().all(|m| *m < 6.5), "legs vs record: {means:?}");
    // SmoothedVelocity / OneToZeroAtWalkSpeed (FSmoothDamp rva 0x161c9b0); the residue is the one-frame walk-start timing
    assert!(sv_worst < 4.0 && oz_worst < 0.06, "SmoothedVelocity {sv_worst:.2} OneToZero {oz_worst:.3}");
}

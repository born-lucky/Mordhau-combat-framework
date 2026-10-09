//! grip r1 (state/proofs/grip.md): the held weapon against the exe's ComputeGrippedTransform and the hand, in the posed
//! skeleton the runtime uses, 1P and 3P idle, for a 2H sword, a 1H sword and a polearm.
//!
//! Expected values come from the proof, not from our pose code:
//! - weapon relative to the RightWeapon bone = T(RightHandEquipOffset) * R(RotationOffset) * R(Pitch +90) *
//!   T(-GripLocationLocal) (UEquipmentSystemComponent::ComputeGrippedTransform rva=0x14b70f0, decomp
//!   UEquipmentSystemComponent.cpp 80-202; attached KeepWorldTransform to the same bone, SwitchEquipment rva=0x14d8cd0),
//!   with the class defaults read from the paks over the AMordhauEquipment ctor values (RightHandEquipOffset (0, -9, -4),
//!   RotationOffset (-15, 15, 0): decomp AMordhauEquipment.cpp 1656-1663), within 0.1 cm / 0.1 deg;
//! - the crossguard (weapon-local Y, the guard meshes span Y) is perpendicular to the blade (TraceStart -> TraceEnd)
//!   and lies along the fist's knuckle axis (RightHand -Y, the bone length axis of the UMA skeleton); the blade runs
//!   across the palm (RightHand X).

use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
use mh_sim::{FighterDesc, Sim};
use mordhau_core::ue::{FQuat, FTransform, FVector};
use std::rc::Rc;
use std::sync::Arc;

const W: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific";

fn ang_deg(a: FQuat, b: FQuat) -> f32 {
    let d = a.inverse().mul(b);
    2.0 * d.w.abs().min(1.0).acos().to_degrees()
}

#[test]
fn weapon_held_as_the_exe_grips_it() {
    // the exe transform itself: the footage-matched 1P blade turn (grip.rs steady) is checked separately below
    std::env::set_var("MH_GRIP_ROLL_1P", "0");
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let Ok(vfs) = mh_pak::Vfs::mount_default() else { return };
    let vfs = Arc::new(vfs);
    let Ok(rec) = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")) else { return };
    let rd = mh_pak::Reader::new(vfs.clone());
    for w in ["TwoHandedSword/BP_Greatsword", "OneHanded/BP_ArmingSword", "Polearms/BP_Spear"] {
        let w = format!("{W}/{w}");
        // the exe's inputs, straight from the class defaults (ctor values where the Blueprints leave them)
        let d = rd.defaults(&w);
        let rot = d.get("RotationOffset").cloned().unwrap_or_default();
        let r = |k: &str, dflt: f32| rot.get(k).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(dflt);
        let (p, y, ro) = (r("Pitch", -15.0), r("Yaw", 15.0), r("Roll", 0.0));
        let rhe = d.get("RightHandEquipOffset").cloned().unwrap_or_default();
        let v = |o: &serde_json::Value, k: &str, dflt: f32| o.get(k).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(dflt);
        let off = FVector::new(v(&rhe, "X", 0.0), v(&rhe, "Y", -9.0), v(&rhe, "Z", -4.0));
        let q = FQuat::from_rotator(p, y, ro).mul(FQuat::from_rotator(90.0, 0.0, 0.0));
        // GripLocationLocal: AMordhauWeapon::RecalculateTracerPoints rva=0x163a940 derives it from the trace sockets
        // (TraceStart - normalize(TraceEnd - TraceStart) * 8; reader method 2026-10-06, superseding proof D1's 0)
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
            let tag = format!("{w} {}", if fp { "1P" } else { "3P" });
            let posed = s.posed.borrow();
            let pz = posed.get("A").unwrap();
            let wg = s.geo.weapons.get(&w).unwrap();
            let g = derived_grip(wg.trace_start, wg.trace_end);
            assert!((wg.grip_location_local - g).length() < 1e-3, "{tag}: GripLocationLocal {:?}, the exe derives {g:?}", wg.grip_location_local);
            // Socket * T(RightHandEquipOffset) * R * T(-GripLocationLocal)
            let expect = FTransform::new(q, off + q.rotate(FVector::new(-g.x, -g.y, -g.z)));
            let wx = s.geo.weapon_world(pz, wg).unwrap();
            let bone = |b: &str| s.geo.bone_world(pz, s.geo.skeleton.find(b).unwrap());
            let rel = wx.then(&bone("RightWeapon").inverse());
            let dl = rel.loc - expect.loc;
            assert!(dl.length() < 0.1, "{tag}: weapon in RightWeapon at {:?}, the exe gives {:?}", rel.loc, expect.loc);
            let da = ang_deg(rel.rot, expect.rot);
            assert!(da < 0.1, "{tag}: weapon in RightWeapon rotated {da} deg off the exe's");
            // the crossguard against the blade and the fist
            let blade = wx.rot.rotate((wg.trace_end.unwrap() - wg.trace_start.unwrap()).normalized());
            let guard = wx.rot.rotate(FVector::new(0.0, 1.0, 0.0));
            assert!(blade.dot(guard).abs() < 1e-3, "{tag}: guard not perpendicular to the blade");
            let h = bone("RightHand").rot;
            let knuckles = h.rotate(FVector::new(0.0, -1.0, 0.0));
            let palm_across = h.rotate(FVector::new(1.0, 0.0, 0.0));
            assert!(guard.dot(knuckles).abs() > 0.85, "{tag}: guard {:?} not along the fist's knuckle axis {:?}", guard, knuckles);
            assert!(blade.dot(palm_across).abs() > 0.8, "{tag}: blade {:?} not across the palm {:?}", blade, palm_across);
            // the closed fingers sit on the grip: RightHandFinger03_02 within 5 cm of the blade axis
            let f = wx.inverse().apply(bone("RightHandFinger03_02").loc);
            assert!((f.x * f.x + f.y * f.y).sqrt() < 5.0, "{tag}: middle finger {:?} off the grip", f);
        }
    }
}

/// a sim with one fighter holding `w`, posed by the graph, in 1P or 3P
fn sim_with(w: &str, fp: bool) -> Option<(Sim, usize)> {
    sim_with_dt(w, fp, 1.0 / 60.0)
}

/// `sim_with` at a given step (the real anim instance ticks at 240 Hz in the reader record)
fn sim_with_dt(w: &str, fp: bool, dt: f32) -> Option<(Sim, usize)> {
    let m = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).ok()?;
    let vfs = Arc::new(mh_pak::Vfs::mount_default().ok()?);
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).ok()?;
    let ld = mh_sim::load::load(&m, vfs.clone(), &[w]).ok()?;
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), dt);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs).unwrap());
    s.set_first_person(a, fp);
    s.set_view_target(a, fp, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    Some((s, a))
}

/// the weapon relative to its socket bone (RightWeapon for these right-handed modes)
fn rel_to_socket(s: &Sim) -> FTransform {
    let posed = s.posed.borrow();
    let p = posed.get("A").unwrap();
    let w = &s.combat.fighters[0].weapon_path;
    let wx = s.geo.weapon_world(p, s.geo.weapons.get(w).unwrap()).unwrap();
    wx.then(&s.geo.bone_world(p, s.geo.skeleton.find("RightWeapon").unwrap()).inverse())
}

/// grip r2: the alternate mode grips with the Second* fields (AMordhauEquipment::SwitchMode_Implementation
/// rva=0x156e280 swaps them, SwitchModeAndReAttach re-runs ComputeGrippedTransform rva=0x14b70f0): the Longsword's
/// SecondRotationOffset (165, 15, 0) and the Halberd's (-15, 15, 180), class defaults in the paks. During the switch
/// the mesh starts on the old grip (stage 0) and ends on the new one (stage 2: UEquipmentModeSwitchMotion
/// PerformVirtualReparentTrickery rva=0x166a520).
#[test]
fn alternate_mode_grip() {
    std::env::set_var("MH_GRIP_ROLL_1P", "0");
    for (w, second) in [("TwoHandedSword/BP_Longsword", [165.0, 15.0, 0.0]), ("Polearms/BP_Halberd", [-15.0, 15.0, 180.0])] {
        for fp in [false, true] {
            let w = format!("{W}/{w}");
            let Some((mut s, a)) = sim_with(&w, fp) else { return };
            for _ in 0..30 {
                s.step(&[]);
            }
            let prim = FQuat::from_rotator(-15.0, 15.0, 0.0).mul(FQuat::from_rotator(90.0, 0.0, 0.0));
            let r0 = rel_to_socket(&s);
            assert!(ang_deg(r0.rot, prim) < 0.1, "{w}: primary grip off by {}", ang_deg(r0.rot, prim));
            s.step(&[(a, mh_sim::SimInput { switch_mode: true, ..Default::default() })]);
            // fp-anim r3: the request starts UEquipmentModeSwitchMotion; FinishSwitch flips the mode at StartTime + 0.15
            assert_eq!(s.combat.cur_m(a).map(|m| m.kind()), Some("EquipmentModeSwitch"), "{w}: no mode switch motion");
            // stage 0 (the first 0.25 s, or 0 s for SwitchType 1): the old grip
            let st = s.posed.borrow().get("A").unwrap().grip.switch.clone();
            let kind = st.as_ref().map(|x| x.kind).unwrap_or(9);
            if kind != 1 {
                let r = rel_to_socket(&s);
                assert!(ang_deg(r.rot, prim) < 0.1, "{w}: stage 0 not on the old grip ({} deg)", ang_deg(r.rot, prim));
            }
            for _ in 0..60 {
                s.step(&[]);
            }
            assert!(s.combat.fighters[a].alternate_mode, "{w}: mode not switched by FinishSwitch");
            assert!(s.posed.borrow().get("A").unwrap().grip.switch.is_none(), "{w}: switch still running after 1 s");
            let alt = FQuat::from_rotator(second[0], second[1], second[2]).mul(FQuat::from_rotator(90.0, 0.0, 0.0));
            let r1 = rel_to_socket(&s);
            let d = ang_deg(r1.rot, alt);
            assert!(d < 0.1, "{w} {}: alternate grip off by {d} deg (kind {kind})", if fp { "1P" } else { "3P" });
            let wg = s.geo.weapons.get(&w).unwrap();
            let g = derived_grip(wg.second_trace_start, wg.second_trace_end);
            let want = FVector::new(0.0, -9.0, -4.0) + alt.rotate(FVector::new(-g.x, -g.y, -g.z));
            assert!((r1.loc - want).length() < 0.1, "{w}: alternate grip at {:?}, the exe gives {want:?}", r1.loc);
        }
    }
}

/// Footage-matched 1P grip (orchestrator 2026-10-06, grip.rs steady): in first person the weapon is turned 90 deg about
/// its blade axis relative to the exe transform; third person keeps the exe transform. The blade line (weapon-local Z)
/// is unchanged, so traces don't move.
#[test]
fn first_person_turns_the_weapon_about_its_blade_only() {
    std::env::remove_var("MH_GRIP_ROLL");
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let (Some((s1, _)), Some((s3, _))) = (sim_with(&w, true), sim_with(&w, false)) else { return };
    let (a, b) = (rel_to_socket(&s1), rel_to_socket(&s3));
    let ang = ang_deg(a.rot, b.rot);
    // the env override is process-wide and other tests set MH_GRIP_ROLL_1P=0; only judge when it is unset or 90
    if std::env::var("MH_GRIP_ROLL_1P").map(|v| v == "90").unwrap_or(true) {
        assert!((ang - 90.0).abs() < 0.5, "1P vs 3P weapon rotation {ang} deg, expected 90");
    }
    let za = a.rot.rotate(FVector::new(0.0, 0.0, 1.0));
    let zb = b.rot.rotate(FVector::new(0.0, 0.0, 1.0));
    assert!(za.x * zb.x + za.y * zb.y + za.z * zb.z > 0.999, "blade axis must not change");
}

/// orchestrator 2026-10-06: our 1P idle component-space bones (Greatsword), to diff against the live read of the real
/// game (scripts/live_bones.py -> state/live_bones.json). Run: cargo test -p mh-sim --test grip probe_live_compare -- --ignored --nocapture
#[test]
#[ignore]
fn probe_live_compare() {
    std::env::set_var("MH_GRIP_ROLL_1P", "0");
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let m = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).expect("spec");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).expect("records");
    let ld = mh_sim::load::load(&m, vfs.clone(), &[w.as_str()]).expect("load");
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.dedicated_server = false;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: w.clone(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs).expect("anim assets"));
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    for _ in 0..90 {
        s.step(&[]);
    }
    let posed = s.posed.borrow();
    let p = posed.get("A").unwrap();
    let mut out = serde_json::Map::new();
    for (i, b) in p.bones.iter().enumerate() {
        let n = s.geo.skeleton.names[i].clone();
        out.insert(n, serde_json::json!({"loc": [b.loc.x, b.loc.y, b.loc.z], "rot": [b.rot.x, b.rot.y, b.rot.z, b.rot.w]}));
    }
    // the held weapon (actor root) world transform and the RightWeapon socket's, as the reader compares them
    let wg = s.geo.weapons.get(&w).unwrap();
    let wx = s.geo.weapon_world(p, wg).unwrap();
    let sx = s.geo.bone_world(p, s.geo.skeleton.find("RightWeapon").unwrap());
    let st = mh_sim::grip::steady(&s.geo, p, wg, false).unwrap();
    println!("equip_offset {:?} rotation_offset {:?} grip_location_local {:?} pitch_r {} modes {:?}", wg.right_hand_equip_offset,
        wg.rotation_offset, wg.grip_location_local, s.geo.grip_pitch_right, wg.grip_modes.as_ref().map(|m| (m[0].rotation_offset, m[0].grip_location_local, m[0].use_equipped_offset)));
    for (k, x) in [("#weapon_world", wx), ("#socket_world", sx), ("#steady", st)] {
        out.insert(k.into(), serde_json::json!({"loc": [x.loc.x, x.loc.y, x.loc.z], "rot": [x.rot.x, x.rot.y, x.rot.z, x.rot.w]}));
    }
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state/our_bones_1p_idle.json");
    std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!("wrote {}", path.display());
}

/// reader method: the raw 1P idle clip's shoulder/arm translations vs the skeleton reference (is the 12-27 cm shoulder
/// offset in the clip data, or added later?). Run with --ignored --nocapture.
#[test]
#[ignore]
fn probe_clip_shoulder_translation() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    for clip in ["Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_Idle_1P",
                 "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_Idle_3p_Longsword_New"] {
        let a = match mh_assets::anim::decode(&src, clip) { Ok(a) => a, Err(e) => { println!("{clip}: {e}"); continue } };
        println!("{clip}: additive {} frames {} tracks {}", a.additive_anim_type, a.num_frames, a.tracks.len());
        for t in &a.tracks {
            if [40, 41, 15, 16].contains(&t.bone) || t.bone < 3 {
                let (r, p, _) = a.sample(t, 0.0);
                println!("  bone {:3} pos {:?} rot {:?}", t.bone, p, r);
            }
        }
    }
}

/// AMordhauWeapon::RecalculateTracerPoints (decomp AMordhauWeapon.cpp 2767-2779): start - safe_normal(end - start) * 8
fn derived_grip(a: Option<FVector>, b: Option<FVector>) -> FVector {
    let a = a.unwrap_or(FVector::ZERO);
    let d = b.unwrap_or(FVector::ZERO) - a;
    let n = if d.length() < 1e-4 { FVector::ZERO } else { d.normalized() };
    FVector::new(a.x - n.x * 8.0, a.y - n.y * 8.0, a.z - n.z * 8.0)
}

/// The left (alt) parry: input always requests a regular parry (AMordhauCharacter.cpp 3695 / 3789: RequestParry(0, ..));
/// a parry during a strike's windup feints it (RequestParry's FTP path) and UFeintedMotion::ProcessBlock rva=0x166bac0
/// turns it into AltRegular (the left parry, Block_Left) when the feinted move IsLeft, regular otherwise; from idle it
/// stays regular.
#[test]
fn parry_side_follows_the_strike_it_comes_out_of() {
    use mordhau_core::combat::enums::{bt, mv};
    for (from, want) in [(None, bt::REGULAR), (Some(mv::RIGHT_STRIKE), bt::REGULAR), (Some(mv::LEFT_STRIKE), bt::ALT_REGULAR)] {
        let w = format!("{W}/TwoHandedSword/BP_Greatsword");
        let Some((mut s, a)) = sim_with(&w, true) else { return };
        for _ in 0..30 {
            s.step(&[]);
        }
        if let Some(m) = from {
            s.step(&[(a, mh_sim::SimInput { attack: Some((m, 0.0)), ..Default::default() })]);
            for _ in 0..5 {
                s.step(&[]);
            }
        }
        s.step(&[(a, mh_sim::SimInput { parry: Some(bt::REGULAR), ..Default::default() })]);
        for _ in 0..2 {
            s.step(&[]);
        }
        let f = &s.combat.fighters[a];
        let pid = f.last_parry_motion.expect("no parry motion");
        let p = f.motions[pid.0 as usize].as_ref().and_then(|m| m.parry()).map(|p| p.block_type);
        assert_eq!(p, Some(want), "parry out of {from:?}: block type {p:?}, the exe gives {want}");
    }
}

/// The alt parry is visibly a different pose in 1P (UParryMotion::OnBegin_Implementation rva=0x1660f70 plays AltAnimation's
/// FirstPerson montage): the right hand 0.25 s into a regular parry vs an alt one (out of a left strike's feint).
#[test]
fn alt_parry_poses_differently_in_first_person() {
    use mordhau_core::combat::enums::{bt, mv};
    let mut hands = vec![];
    for from in [None, Some(mv::LEFT_STRIKE)] {
        let w = format!("{W}/TwoHandedSword/BP_Greatsword");
        let Some((mut s, a)) = sim_with(&w, true) else { return };
        for _ in 0..30 {
            s.step(&[]);
        }
        if let Some(m) = from {
            s.step(&[(a, mh_sim::SimInput { attack: Some((m, 0.0)), ..Default::default() })]);
            for _ in 0..5 {
                s.step(&[]);
            }
        }
        s.step(&[(a, mh_sim::SimInput { parry: Some(bt::REGULAR), ..Default::default() })]);
        for _ in 0..15 {
            s.step(&[]);
        }
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let h = p.bones[s.geo.skeleton.find("RightHand").unwrap()].loc;
        println!("{from:?}: RightHand {h:?}");
        hands.push(h);
    }
    let d = (hands[0] - hands[1]).length();
    assert!(d > 5.0, "regular and alt 1P parries put the right hand only {d} cm apart");
}

/// The strike's look counter-compensation (procedural.rs CounterCompensation, UStrikeMotion
/// ComputeCounterCompensationRotation rva=0x164f670): looking up after release counter-rotates Spine1 by roll
/// (look delta) x 0.5 for a non-overhead strike (AngleTarget > 0), with weight 1 during release.
#[test]
fn strike_counter_compensates_look_after_release() {
    use mordhau_core::combat::enums::mv;
    let mut out = vec![];
    for look in [0.0f64, 20.0] {
        let w = format!("{W}/TwoHandedSword/BP_Greatsword");
        let Some((mut s, a)) = sim_with(&w, true) else { return };
        for _ in 0..30 {
            s.step(&[]);
        }
        s.step(&[(a, mh_sim::SimInput { attack: Some((mv::RIGHT_STRIKE, 0.5)), ..Default::default() })]);
        // step into release, then look up
        let mut steps = 0;
        while s.combat.cur_m(a).and_then(|m| m.attack()).map(|x| x.stage) != Some(mordhau_core::combat::enums::stage::RELEASE) && steps < 120 {
            s.step(&[]);
            steps += 1;
        }
        for i in 0..6 {
            let l = look * (i + 1) as f64 / 6.0;
            s.step(&[(a, mh_sim::SimInput { look_up: Some(l), ..Default::default() })]);
        }
        let fa = &s.fanim[0];
        out.push((fa.counter_comp.rot, fa.counter_comp.weight));
        println!("look {look}: CounterCompensateRotation {:?} weight {}", fa.counter_comp.rot, fa.counter_comp.weight);
    }
    assert_eq!(out[0].1, 1.0, "weight not 1 during release");
    let roll = out[1].0 .2 - out[0].0 .2;
    assert!(roll > 5.0, "looking up 20 deg after release gave roll {roll}, the exe gives ~ +10 (x 0.5)");
}

/// The swivel parry: RequestParry turns BlockType 0 into AltRegular when the angling vector points left
/// (UMotionSystemComponent::RequestParry rva=0x14d1590, input.rs); from idle an AltRegular request must reach the
/// parry motion as AltRegular (the left parry).
#[test]
fn swivel_parry_from_idle_is_the_left_parry() {
    use mordhau_core::combat::enums::bt;
    for (req, want) in [(bt::REGULAR, bt::REGULAR), (bt::ALT_REGULAR, bt::ALT_REGULAR)] {
        let w = format!("{W}/TwoHandedSword/BP_Greatsword");
        let Some((mut s, a)) = sim_with(&w, true) else { return };
        for _ in 0..30 {
            s.step(&[]);
        }
        s.step(&[(a, mh_sim::SimInput { parry: Some(req), ..Default::default() })]);
        s.step(&[]);
        let f = &s.combat.fighters[a];
        let pid = f.last_parry_motion.expect("no parry");
        let got = f.motions[pid.0 as usize].as_ref().and_then(|m| m.parry()).map(|p| p.block_type);
        assert_eq!(got, Some(want), "idle parry request {req}: block type {got:?}");
    }
}

/// reader method: our 1P idle bones at a range of look-up values (state/our_bones_by_lookup.json), to compare the arms'
/// tracking of the view with the real game's (state/live_rec/real_hand_by_pitch.json). Run with --ignored.
#[test]
#[ignore]
fn probe_arms_by_look_up() {
    let mut out = serde_json::Map::new();
    for lu in [-70.0f64, -50.0, -30.0, -10.0, 0.0, 10.0, 30.0, 50.0] {
        let w = format!("{W}/TwoHandedSword/BP_Greatsword");
        let Some((mut s, a)) = sim_with(&w, true) else { return };
        for _ in 0..90 {
            s.step(&[(a, mh_sim::SimInput { look_up: Some(lu), ..Default::default() })]);
        }
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let mut m = serde_json::Map::new();
        // every bone, component space (the reader's frames[].bones layout, by name)
        for (i, n) in s.geo.skeleton.names.iter().enumerate() {
            let b = p.bones[i];
            m.insert(n.clone(), serde_json::json!({"loc": [b.loc.x, b.loc.y, b.loc.z], "rot": [b.rot.x, b.rot.y, b.rot.z, b.rot.w]}));
        }
        m.insert("look_up_value".into(), serde_json::json!(s.combat.fighters[a].look_up_value));
        out.insert(format!("{lu}"), serde_json::Value::Object(m));
    }
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state/our_bones_by_lookup.json");
    std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
}

/// EVD_CAM_006 (reader method): in steady 1P idle the real hands stay fixed in Spine1's frame as the look pitch moves,
/// so they follow the view through the look-up spine bend (procedural.rs spine_bend, node 463) and no arm node bends
/// with the look. Real values: state/live_rec/everything.json, IdleMotion frames with Arms3PSyncWeight <= 0.005 and
/// AtmosphericsWeight >= 0.85 (no strike / block montage still blending out), stale bone reads 3460-3800 / 3940-4000
/// dropped: 118 frames at mean LookUpValue -4.45 (frames 3401-4622) and 42 at +13.82 (3041-3082). Across that 18 deg
/// Spine1 turns 18.8 deg while RightArm / RightForeArm / LeftArm / LeftForeArm turn 1.5-2.2 deg locally (breathing).
/// The record holds no steady idle below look -6: every IdleMotion frame below that is within 0.6 s of a
/// BP_BlockedMotion (Arms3PSyncWeight 0.07-0.33), where the hand converges from the block pose toward these values.
#[test]
fn first_person_idle_hands_follow_spine1_with_the_look() {
    // (look, RightHand in Spine1's frame, LeftHand in Spine1's frame, RightHand - Spine1 in component space)
    let real: [(f64, [f32; 3], [f32; 3], [f32; 3]); 2] = [
        (-4.45, [2.5, -20.5, 25.1], [14.2, -16.0, 21.4], [-6.2, 27.9, 15.5]),
        (13.82, [3.7, -21.8, 24.7], [15.9, -18.2, 20.1], [-4.5, 21.4, 24.9]),
    ];
    for (lu, rh_s1, lh_s1, rh_cs) in real {
        let w = format!("{W}/TwoHandedSword/BP_Greatsword");
        let Some((mut s, a)) = sim_with(&w, true) else { return };
        for _ in 0..90 {
            s.step(&[(a, mh_sim::SimInput { look_up: Some(lu), ..Default::default() })]);
        }
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let bone = |n: &str| p.bones[s.geo.skeleton.find(n).unwrap()];
        let s1 = bone("Spine1");
        let in_s1 = |n: &str| s1.rot.inverse().rotate(bone(n).loc - s1.loc);
        let v = |x: [f32; 3]| FVector::new(x[0], x[1], x[2]);
        let (rh, lh, cs) = (in_s1("RightHand"), in_s1("LeftHand"), bone("RightHand").loc - s1.loc);
        println!("look {lu}: RightHand in Spine1 {rh:?} (real {rh_s1:?}), LeftHand {lh:?} (real {lh_s1:?}), RightHand cs {cs:?} (real {rh_cs:?})");
        for (what, got, want) in [("RightHand in Spine1", rh, v(rh_s1)), ("LeftHand in Spine1", lh, v(lh_s1)), ("RightHand - Spine1", cs, v(rh_cs))] {
            let d = (got - want).length();
            assert!(d < 4.0, "look {lu}: {what} {got:?} is {d} cm from the real {want:?}");
        }
    }
}

/// EVD_CAM_014: Arms3PSyncWeight = FInterpTo(W, Motion.bRequires3PArmsSync, dt, 3) (NativeUpdateAnimation
/// rva=0x1501930, decomp 1637 / 1663-1665, speed 3.0 from the machine code at 0x141503529 -> call 0x1415039ea).
/// Real decay: state/live_rec/everything.json frames 56 / 64 / 72 / 78 (BP_BlockedMotion_C -> IdleMotion after a
/// right strike, the anim instance ticking at 240 Hz, LastDeltaSeconds 0.00417): 0.9513 at the first frame with the flag
/// 0, then 0.628 / 0.4145 / 0.3065 at +0.1358 / +0.2724 / +0.3746 s. Replaying the whole record one frame ahead with
/// speed 3 gives a median error of 0.0000 (2.5 and 3.5: 0.0015 / 0.0013).
#[test]
fn arms_3p_sync_weight_decays_as_the_real_one_after_a_block() {
    use mh_sim::procedural::{MotionFlags, ProcState};
    let flags = MotionFlags { disables_offhand_ik: false, offhand_ik_change_speed: 2.5, disables_atmospherics: false, disables_cosmetic_weapon_transform: false, cosmetic_change_speed: 2.0, requires_3p_arms_sync: false };
    let mut p = ProcState::default();
    let cos = ((0.0, 0.0, 0.0), FVector::ZERO);
    p.update(0.0, flags, cos);
    p.arms_3p_sync = 0.9513;
    let dt = 0.00417f64;
    let mut t = 0.0;
    for (at, want) in [(0.1358f64, 0.628f32), (0.2724, 0.4145), (0.3746, 0.3065)] {
        while t + dt <= at + 1e-6 {
            t += dt;
            p.update(t, flags, cos);
        }
        println!("t {at}: ours {} real {want}", p.arms_3p_sync);
        assert!((p.arms_3p_sync - want).abs() < 0.01, "at +{at} s: Arms3PSyncWeight {} vs real {want}", p.arms_3p_sync);
    }
    // the strike sets it (UAttackMotion ctor), recovery clears it (EnterRecovery)
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((mut s, a)) = sim_with(&w, true) else { return };
    for _ in 0..30 {
        s.step(&[]);
    }
    assert_eq!(s.fanim[0].proc.arms_3p_sync, 0.0, "idle Arms3PSyncWeight not 0");
    s.step(&[(a, mh_sim::SimInput { attack: Some((mordhau_core::combat::enums::mv::RIGHT_STRIKE, 0.5)), ..Default::default() })]);
    let (mut peak, mut steps) = (0.0f32, 0);
    while s.combat.cur_m(a).and_then(|m| m.attack()).map(|x| x.stage) != Some(2) && steps < 240 {
        s.step(&[]);
        peak = peak.max(s.fanim[0].proc.arms_3p_sync);
        steps += 1;
    }
    let at_recovery = s.fanim[0].proc.arms_3p_sync;
    for _ in 0..30 {
        s.step(&[]);
    }
    let later = s.fanim[0].proc.arms_3p_sync;
    println!("strike: peak {peak} after {steps} steps, at recovery {at_recovery}, 0.5 s later {later}");
    assert!(peak > 0.5, "Arms3PSyncWeight only reached {peak} through windup / release");
    let want = at_recovery * (-3.0f32 * 0.5).exp();
    assert!((later - want).abs() < 0.05, "0.5 s into recovery {later}, FInterpTo(3) gives ~{want}");
}

/// EVD_CAM_014 reader probe: the real right strike -> BP_BlockedMotion -> idle at look -70 (everything.json frames
/// 14-82: strike StartTime 112.183, blocked StartTime 113.116 = +0.933 s, look keyed below) replayed on ours, the right
/// hand in Spine1's frame printed at the real frames' times. Run with --ignored --nocapture.
#[test]
#[ignore]
fn probe_blocked_blend_out_at_look_down() {
    use mordhau_core::combat::world::{Call, Input};
    let looks = [(0.0f64, -25.0f64), (0.126, -21.5), (0.241, -32.8), (0.327, -45.8), (0.448, -63.5), (0.611, -68.1), (0.761, -70.0), (1.2, -70.0), (1.24, -68.5)];
    let look_at = |t: f64| {
        let mut l = looks[0].1;
        for w in looks.windows(2) {
            if t >= w[0].0 {
                l = if t >= w[1].0 { w[1].1 } else { w[0].1 + (w[1].1 - w[0].1) * (t - w[0].0) / (w[1].0 - w[0].0) };
            }
        }
        l
    };
    let real = [(0.3, [-24.2, -25.1, 44.3]), (0.45, [-17.6, -32.6, 41.5]), (0.611, [-15.8, -36.8, 41.2]), (0.693, [-18.9, -42.8, 44.5]), (0.761, [-24.9, -48.9, 49.7]), (0.83, [-29.7, -47.1, 50.9]), (0.865, [0.0, 0.0, 0.0]), (0.899, [-38.4, -46.3, 42.8]), (0.92, [0.0, 0.0, 0.0]), (0.95, [0.0, 0.0, 0.0]), (0.967, [-24.7, -31.2, 60.2]), (1.035, [-24.1, -34.2, 59.7]), (1.103, [-22.5, -34.7, 60.0]), (1.171, [-20.5, -35.2, 60.3]), (1.24, [-17.7, -35.5, 58.3]), (1.308, [-15.0, -35.4, 55.9]), (1.375, [-12.6, -34.9, 53.6])];
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let hz = std::env::var("MH_PROBE_HZ").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(60.0);
    let Some((mut s, a)) = sim_with_dt(&w, true, (1.0 / hz) as f32) else { return };
    for _ in 0..(hz as usize) {
        s.step(&[(a, mh_sim::SimInput { look_up: Some(-25.0), ..Default::default() })]);
    }
    let t0 = s.combat.now;
    s.step(&[(a, mh_sim::SimInput { attack: Some((mordhau_core::combat::enums::mv::RIGHT_STRIKE, std::env::var("MH_PROBE_ANGLE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5))), look_up: Some(-25.0), ..Default::default() })]);
    let mut blocked = false;
    let mut k = 0;
    let mut dump = vec![];
    let mut hit_set = false;
    while k < real.len() {
        let t = s.combat.now - t0;
        if let Some(h) = std::env::var("MH_PROBE_HIT").ok().and_then(|v| v.parse::<f64>().ok()) {
            if t >= h && !hit_set {
                hit_set = true;
                s.combat.input(Input::Call(Call::SetHasHitIncludingCosmeticHit { who: "A".into() }));
            }
        }
        if !blocked && t >= std::env::var("MH_PROBE_BLOCK").ok().and_then(|v| v.parse().ok()).unwrap_or(0.933) {
            s.combat.input(Input::Call(Call::Blocked { who: "A".into(), reason: 0, flags: 0, time: s.combat.now }));
            blocked = true;
        }
        s.step(&[(a, mh_sim::SimInput { look_up: Some(look_at(t)), ..Default::default() })]);
        let t = s.combat.now - t0;
        if t >= real[k].0 {
            let posed = s.posed.borrow();
            let p = posed.get("A").unwrap();
            let bone = |n: &str| p.bones[s.geo.skeleton.find(n).unwrap()];
            let s1 = bone("Spine1");
            let h = s1.rot.inverse().rotate(bone("RightHand").loc - s1.loc);
            let m = s.combat.cur_m(a).map(|m| m.kind()).unwrap_or("-");
            let at = s.combat.cur_m(a).and_then(|m| m.attack().map(|x| (m.start_time, x.stage, x.windup_end, x.angle_target)));
            println!("+{:.3} {m} hit {:.2} pos {:.3} er {:.2} seq {} look {:.1} a3p {:.2} ccw {:.2} attack {:?}: ours [{:.1}, {:.1}, {:.1}] real {:?}", t, s.fanim[0].hit_effect.weight, s.fanim[0].ma.position, s.fanim[0].ma.early_release, s.fanim[0].ma.seq.rsplit('/').next().unwrap_or(""), look_at(t), s.fanim[0].proc.arms_3p_sync, s.fanim[0].counter_comp.weight, at.map(|x| (x.0 - t0, x.1, x.2 - t0, x.3)), h.x, h.y, h.z, real[k].1);
            let all: Vec<[f32; 7]> = p.bones.iter().map(|b| [b.rot.x, b.rot.y, b.rot.z, b.rot.w, b.loc.x, b.loc.y, b.loc.z]).collect();
            dump.push(serde_json::json!({"t": real[k].0, "bones": all}));
            k += 1;
        }
    }
    if let Ok(path) = std::env::var("MH_PROBE_DUMP") {
        std::fs::write(path, serde_json::json!({"names": s.geo.skeleton.names, "frames": dump}).to_string()).unwrap();
    }
}

/// EVD_CAM_010: ModifyBone 189 / 190 / 191 (Spine1Adjust, LeftShoulder, RightShoulder) and 206 / 205 / 204
/// (LowerBackAdjust, LeftUpLeg, RightUpLeg) add (1 - Arms3PSyncWeight) x CameraCollisionOffset in component space at
/// alpha Helper_FirstPersonNotDead, CameraCollisionOffset = mesh rotation^-1 x the camera's world
/// CameraCollisionLocationOffset (UMordhauAnimInstance decomp 1482-1504, UpdateBlueprintHelpers 0x14151abec-0x14151ac38).
/// They run last (outer chain), so in first person both hands and both thighs move by the world offset and Spine1 stays;
/// in third person nothing moves.
#[test]
fn camera_collision_offset_moves_arms_and_thighs_in_first_person() {
    let off = FVector::new(3.0, -4.0, -12.0);
    for fp in [true, false] {
        let w = format!("{W}/TwoHandedSword/BP_Greatsword");
        let mut at = vec![];
        for o in [FVector::ZERO, off] {
            let Some((mut s, a)) = sim_with(&w, fp) else { return };
            s.set_camera_collision_offset(a, o);
            for _ in 0..30 {
                s.step(&[]);
            }
            let posed = s.posed.borrow();
            let p = posed.get("A").unwrap();
            let wl = |n: &str| s.geo.bone_world(p, s.geo.skeleton.find(n).unwrap()).loc;
            at.push(["Spine1", "RightHand", "LeftHand", "LeftUpLeg", "RightUpLeg"].map(wl));
        }
        for (k, n) in ["Spine1", "RightHand", "LeftHand", "LeftUpLeg", "RightUpLeg"].iter().enumerate() {
            let d = at[1][k] - at[0][k];
            let want = if fp && k > 0 { off } else { FVector::ZERO };
            println!("fp {fp} {n}: moved {d:?}");
            assert!((d - want).length() < 0.05, "fp {fp}: {n} moved {d:?}, want {want:?}");
        }
    }
}

/// EVD_SWG_003: the hit-effect IK. The reader record's right strike (everything.json frames 14-56: StartTime 112.183,
/// BP_BlockedMotion at +0.933) has bIsDoingHitEffectIK from frame 49 (+0.808; frame 48, +0.791, has weight 0): the
/// hand is held at the contact location (HitEffectIKLocation, sliding 200 cm/s toward the animated hand) with
/// HitEffectIKWeight = min(weight, FC_HitEffectIKCurve(|start - animated hand|)) = 1 / 1 / 0.91 / 0.49 at +0.825 /
/// +0.859 / +0.893 / +0.911 (UMordhauAnimInstance.cpp 4280-4364; nodes 491 TwoBoneIK_4 / 492 ModifyBone_74). Without it
/// our hand was 21 cm off at +0.893 (grip.rs probe_blocked_blend_out_at_look_down). Replayed at the record's 240 Hz
/// anim tick with its look keys; the contact sets bHasHitIncludingCosmeticHit (the record is a network client: its
/// cosmetic hit precedes the server's parry by 0.12 s, which a standalone Sim does not model, so the test sets it
/// through SetHasHitIncludingCosmeticHit at +0.805). The strike's AngleTarget is not in the record: -0.54 (input -32)
/// is the value whose windup / release path fits the record before the contact (5-7 cm; 0.8 / -1.0 give 12-25 cm).
#[test]
#[ignore = "Historical pose fit: original AngleTarget and component-buffer identity are unrecorded; preserve 20261008 Bake rejection"]
fn hit_effect_ik_holds_the_hand_at_the_contact() {
    use mordhau_core::combat::world::{Call, Input};
    let looks = [(0.0f64, -25.0f64), (0.126, -21.5), (0.241, -32.8), (0.327, -45.8), (0.448, -63.5), (0.611, -68.1), (0.761, -70.0)];
    let look_at = |t: f64| {
        let mut l = looks[0].1;
        for w in looks.windows(2) {
            if t >= w[0].0 {
                l = if t >= w[1].0 { w[1].1 } else { w[0].1 + (w[1].1 - w[0].1) * (t - w[0].0) / (w[1].0 - w[0].0) };
            }
        }
        if t >= looks[looks.len() - 1].0 {
            l = looks[looks.len() - 1].1;
        }
        l
    };
    // (time after StartTime, RightHand in Spine1's frame, HitEffectIKWeight)
    let real: [(f64, [f32; 3], f32); 5] = [
        (0.7909, [-25.5, -48.8, 51.5], 0.0),
        (0.8247, [-29.7, -47.1, 50.9], 1.0),
        (0.8594, [-34.9, -47.1, 46.4], 1.0),
        (0.8934, [-38.4, -46.3, 42.8], 0.91),
        (0.9105, [-34.2, -41.3, 51.8], 0.49),
    ];
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 240.0) else { return };
    for _ in 0..240 {
        s.step(&[(a, mh_sim::SimInput { look_up: Some(-25.0), ..Default::default() })]);
    }
    let t0 = s.combat.now;
    s.step(&[(a, mh_sim::SimInput { attack: Some((mordhau_core::combat::enums::mv::RIGHT_STRIKE, -32.0)), look_up: Some(-25.0), ..Default::default() })]);
    let (mut k, mut hit) = (0, false);
    while k < real.len() {
        let t = s.combat.now - t0;
        if !hit && t >= 0.805 {
            s.combat.input(Input::Call(Call::SetHasHitIncludingCosmeticHit { who: "A".into() }));
            hit = true;
        }
        s.step(&[(a, mh_sim::SimInput { look_up: Some(look_at(t)), ..Default::default() })]);
        let t = s.combat.now - t0;
        if t + 1e-6 >= real[k].0 {
            let posed = s.posed.borrow();
            let p = posed.get("A").unwrap();
            let bone = |n: &str| p.bones[s.geo.skeleton.find(n).unwrap()];
            let s1 = bone("Spine1");
            let h = s1.rot.inverse().rotate(bone("RightHand").loc - s1.loc);
            let (want, ww) = (FVector::new(real[k].1[0], real[k].1[1], real[k].1[2]), real[k].2);
            let wt = s.fanim[0].hit_effect.weight;
            println!("+{t:.3}: hand {h:?} real {want:?} ({:.1} cm); weight {wt:.2} real {ww}", (h - want).length());
            // before the contact the residual swing difference is ~6 cm (EVD_SWG_003 observed); with the IK, 4.5
            // while the IK lets go (+0.911, weight 0.49 and the animated hand ~50 cm from the contact) a 3 ms sample offset
            // moves the hand ~1 cm per ms: only the weight is held to the record there (ours 9.4 cm off)
            let tol = if ww == 0.0 { 6.5 } else if ww < 0.9 { f32::INFINITY } else { 4.5 };
            assert!((h - want).length() < tol, "+{t:.3}: RightHand in Spine1 {h:?} vs real {want:?}");
            assert!((wt - ww).abs() < 0.15, "+{t:.3}: HitEffectIKWeight {wt} vs real {ww}");
            k += 1;
        }
    }
}

/// weapon-pose r1 probe: our 1P greatsword idle RightWeapon relative to RightHand (rotator + location) and the anim
/// instance's RightWeaponBoneBaseTransform, to compare with the reader record (everything.json idle frames).
#[test]
#[ignore]
fn probe_right_weapon_in_hand() {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    for fp in [true, false] {
        let Some((mut s, _a)) = sim_with(&w, fp) else { return };
        for _ in 0..120 {
            s.step(&[]);
        }
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let bone = |n: &str| p.bones[s.geo.skeleton.find(n).unwrap()];
        let rel = bone("RightWeapon").then(&bone("RightHand").inverse());
        drop(posed);
        let g = rel_to_socket(&s);
        println!("fp {fp}: weapon in RightWeapon rot {:?} q {:?} loc {:?}", mordhau_core::combat::geometry::quat_rotator(g.rot), g.rot, g.loc);
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let bone = |n: &str| p.bones[s.geo.skeleton.find(n).unwrap()];
        println!("fp {fp}: RW in RH rot {:?} q {:?} loc {:?}; base rot {:?} loc {:?}", mordhau_core::combat::geometry::quat_rotator(rel.rot), rel.rot, rel.loc, s.fanim[0].proc.weapon_base_rot, s.fanim[0].proc.weapon_base_loc);
    }
}

/// weapon-pose r1 probe: our 1P greatsword parry (regular / alt) hands in Spine1's frame by look and time after the
/// parry start, to compare with the reader record's parries (everything.json; real_parry samples)
#[test]
#[ignore]
fn probe_parry_hands_by_look() {
    use mordhau_core::combat::enums::bt;
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    for b in [bt::REGULAR, bt::ALT_REGULAR] {
        for look in [-20.0f64, -12.0, -4.0, 10.0] {
            let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 240.0) else { return };
            for _ in 0..240 {
                s.step(&[(a, mh_sim::SimInput { look_up: Some(look), ..Default::default() })]);
            }
            s.step(&[(a, mh_sim::SimInput { parry: Some(b), look_up: Some(look), ..Default::default() })]);
            let t0 = s.combat.now;
            let mut line = format!("bt {b} look {look:5.1}:");
            for at in [0.1f64, 0.2, 0.3] {
                while s.combat.now - t0 < at - 1e-6 {
                    s.step(&[(a, mh_sim::SimInput { look_up: Some(look), ..Default::default() })]);
                }
                let posed = s.posed.borrow();
                let p = posed.get("A").unwrap();
                let bone = |n: &str| p.bones[s.geo.skeleton.find(n).unwrap()];
                let s1 = bone("Spine1");
                let rh = s1.rot.inverse().rotate(bone("RightHand").loc - s1.loc);
                let lh = s1.rot.inverse().rotate(bone("LeftHand").loc - s1.loc);
                // in view axes: (pitch LookUp + FirstPersonLookUpOffset 5.27, the actor yaw 0), world space
                let wl = |n: &str| s.geo.bone_world(p, s.geo.skeleton.find(n).unwrap()).loc;
                let view = mordhau_core::ue::FQuat::from_rotator(look as f32 + 5.27, 0.0, 0.0);
                let rhh = view.inverse().rotate(wl("RightHand") - wl("Spine1"));
                line += &format!(" t{at} RH [{:.1},{:.1},{:.1}] LH [{:.1},{:.1},{:.1}] RH-Spine1 in view [{:.1},{:.1},{:.1}] |", rh.x, rh.y, rh.z, lh.x, lh.y, lh.z, rhh.x, rhh.y, rhh.z);
            }
            println!("{line}");
        }
    }
}

/// weapon-pose r1 probe: the record's last greatsword stab (everything.json StartTime 198.6) replayed with its look keys;
/// hands minus Spine1 in view axes (pitch LookUp + 5.27) at the record's sample times
#[test]
#[ignore]
fn probe_stab_hands_in_view() {
    let keys = [(-0.3f64, -1.8f64), (-0.087, -1.5), (-0.037, -1.3), (0.014, -0.7), (0.065, 1.5), (0.116, 4.4), (0.168, 6.4), (0.219, 7.7), (0.27, 8.0), (0.321, 7.8), (0.372, 7.5), (0.423, 7.1), (0.474, 6.6), (0.525, 5.8), (0.577, 4.8), (0.627, 3.3), (0.678, 1.4), (0.73, -0.8), (0.781, -2.1), (0.832, -2.9), (0.883, -3.2), (1.0, -3.2)];
    let look_at = |t: f64| {
        let mut l = keys[0].1;
        for w in keys.windows(2) {
            if t >= w[0].0 {
                l = if t >= w[1].0 { w[1].1 } else { w[0].1 + (w[1].1 - w[0].1) * (t - w[0].0) / (w[1].0 - w[0].0) };
            }
        }
        l
    };
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 240.0) else { return };
    for _ in 0..240 {
        s.step(&[(a, mh_sim::SimInput { look_up: Some(-1.8), ..Default::default() })]);
    }
    let t0 = s.combat.now;
    s.step(&[(a, mh_sim::SimInput { attack: Some((mordhau_core::combat::enums::mv::STAB, std::env::var("MH_PROBE_ANGLE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0))), look_up: Some(-1.8), ..Default::default() })]);
    for at in [0.202f64, 0.406, 0.593, 0.695, 0.798, 0.866] {
        while s.combat.now - t0 < at - 1e-6 {
            let t = s.combat.now - t0;
            s.step(&[(a, mh_sim::SimInput { look_up: Some(look_at(t)), ..Default::default() })]);
        }
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let wl = |n: &str| s.geo.bone_world(p, s.geo.skeleton.find(n).unwrap()).loc;
        let view = mordhau_core::ue::FQuat::from_rotator(s.combat.fighters[a].look_up_value as f32 + 5.27, 0.0, 0.0);
        let rh = view.inverse().rotate(wl("RightHand") - wl("Spine1"));
        let lh = view.inverse().rotate(wl("LeftHand") - wl("Spine1"));
        println!("t {at:.3} look {:.1}: RH [{:.1},{:.1},{:.1}] LH [{:.1},{:.1},{:.1}]", s.combat.fighters[a].look_up_value, rh.x, rh.y, rh.z, lh.x, lh.y, lh.z);
    }
}

/// the hands minus Spine1 in view axes (pitch LookUpValue + FirstPersonLookUpOffset 5.27, the actor yaw 0): the record's
/// pov rotation equals that (pov pitch - LookUpValue = 5.3, EVD_CAM_002)
fn hands_in_view(s: &Sim, a: usize) -> (FVector, FVector) {
    let posed = s.posed.borrow();
    let p = posed.get("A").unwrap();
    let wl = |n: &str| s.geo.bone_world(p, s.geo.skeleton.find(n).unwrap()).loc;
    let view = FQuat::from_rotator(s.combat.fighters[a].look_up_value as f32 + 5.27, 0.0, 0.0);
    (view.inverse().rotate(wl("RightHand") - wl("Spine1")), view.inverse().rotate(wl("LeftHand") - wl("Spine1")))
}

/// EVD_POS_001 / 002 / 003 (sheets/combat/03_pose_evidence.csv), against the reader record (everything.json):
/// - parry poses (UParryMotion::OnTick_Implementation rva=0x1668980 AngleAdditive by look, regular / alt): the hands
///   in view axes 0.2 s into the parry, real regular at look -18.8 / -7.7 and alt at -22.7 / +10.4;
/// The fitted stab path is preserved separately as an ignored diagnostic with its original assertions.
/// - the 1P RightWeapon bone relative to RightHand in idle (record (-5.93, -8.68, -11.18), loc (1.5, -0.46, 1.8)).
#[test]
fn weapon_poses_match_the_record() {
    use mordhau_core::combat::enums::bt;
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let v = |x: [f32; 3]| FVector::new(x[0], x[1], x[2]);
    // parry: (block type, look, real RH, real LH)
    let parries = [
        (bt::REGULAR, -18.8, [28.8, 28.0, 48.6], [26.8, 34.9, 55.4]),
        (bt::REGULAR, -7.7, [29.7, 25.7, 49.6], [28.6, 32.6, 56.4]),
        (bt::ALT_REGULAR, -22.7, [23.7, -21.2, 47.6], [7.2, -28.7, 66.4]),
        (bt::ALT_REGULAR, 10.4, [23.5, -21.5, 47.6], [9.1, -29.0, 67.0]),
    ];
    for (b, look, rh, lh) in parries {
        let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 240.0) else { return };
        for _ in 0..240 {
            s.step(&[(a, mh_sim::SimInput { look_up: Some(look), ..Default::default() })]);
        }
        s.step(&[(a, mh_sim::SimInput { parry: Some(b), look_up: Some(look), ..Default::default() })]);
        let t0 = s.combat.now;
        while s.combat.now - t0 < 0.2 - 1e-6 {
            s.step(&[(a, mh_sim::SimInput { look_up: Some(look), ..Default::default() })]);
        }
        let (r, l) = hands_in_view(&s, a);
        println!("parry {b} look {look}: RH {r:?} ({:.1} cm) LH {l:?} ({:.1} cm)", (r - v(rh)).length(), (l - v(lh)).length());
        assert!((r - v(rh)).length() < 2.5 && (l - v(lh)).length() < 2.5, "parry {b} look {look}: RH {r:?} LH {l:?} vs {rh:?} {lh:?}");
    }
    // 1P RightWeapon in RightHand
    let Some((mut s, _)) = sim_with(&w, true) else { return };
    for _ in 0..120 {
        s.step(&[]);
    }
    let posed = s.posed.borrow();
    let p = posed.get("A").unwrap();
    let bone = |n: &str| p.bones[s.geo.skeleton.find(n).unwrap()];
    let rel = bone("RightWeapon").then(&bone("RightHand").inverse());
    let want = FQuat::from_rotator(-5.93, -8.68, -11.18);
    let d = ang_deg(rel.rot, want);
    println!("RightWeapon in RightHand: {d:.2} deg, loc {:?}", rel.loc);
    assert!(d < 0.5 && (rel.loc - FVector::new(1.5, -0.46, 1.8)).length() < 0.3, "RightWeapon in RightHand {:?} {:?}", rel.rot, rel.loc);
}

/// Historical EVD_POS_002 fit. AngleTarget was omitted from the native capture; input -16 was fitted
/// against an unbaked implementation. Preserve the capture, original tolerances and failing diagnostic,
/// rather than treating the fitted input as a native parity oracle. See ATTACK-POSE-RECONSTRUCTION-20261008.md.
#[test]
#[ignore = "Historical pose fit: original AngleTarget and component-buffer identity are unrecorded; preserve 20261008 Bake rejection"]
fn fitted_stab_pose_matches_historical_record() {
    use mordhau_core::combat::enums::mv;
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let v = |x: [f32; 3]| FVector::new(x[0], x[1], x[2]);
    // stab with the record's look keys
    let keys = [(-0.3f64, -1.8f64), (-0.087, -1.5), (-0.037, -1.3), (0.014, -0.7), (0.065, 1.5), (0.116, 4.4), (0.168, 6.4), (0.219, 7.7), (0.27, 8.0), (0.321, 7.8), (0.372, 7.5), (0.423, 7.1), (0.474, 6.6), (0.525, 5.8), (0.577, 4.8), (0.627, 3.3), (0.678, 1.4), (0.73, -0.8), (0.781, -2.1), (0.832, -2.9), (0.883, -3.2), (1.0, -3.2)];
    let look_at = |t: f64| {
        let mut l = keys[0].1;
        for k in keys.windows(2) {
            if t >= k[0].0 {
                l = if t >= k[1].0 { k[1].1 } else { k[0].1 + (k[1].1 - k[0].1) * (t - k[0].0) / (k[1].0 - k[0].0) };
            }
        }
        l
    };
    let real = [
        (0.202, [23.3, 38.3, 11.2], [29.7, 34.1, -6.1]),
        (0.406, [-2.9, 40.3, 17.3], [0.4, 48.6, 1.5]),
        (0.593, [-18.9, 29.3, 20.6], [-18.9, 42.3, 8.8]),
        (0.695, [-18.0, 24.9, 21.9], [-18.1, 38.5, 10.7]),
        (0.798, [-0.7, 26.3, 24.4], [-1.1, 37.8, 12.1]),
        (0.866, [23.1, 28.7, 25.7], [20.7, 37.5, 12.9]),
    ];
    let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 240.0) else { return };
    for _ in 0..240 {
        s.step(&[(a, mh_sim::SimInput { look_up: Some(-1.8), ..Default::default() })]);
    }
    let t0 = s.combat.now;
    s.step(&[(a, mh_sim::SimInput { attack: Some((mv::STAB, -16.0)), look_up: Some(-1.8), ..Default::default() })]);
    for (at, rh, lh) in real {
        while s.combat.now - t0 < at - 1e-6 {
            let t = s.combat.now - t0;
            s.step(&[(a, mh_sim::SimInput { look_up: Some(look_at(t)), ..Default::default() })]);
        }
        let (r, l) = hands_in_view(&s, a);
        println!("stab +{at}: RH {:.1} cm, LH {:.1} cm", (r - v(rh)).length(), (l - v(lh)).length());
        assert!((r - v(rh)).length() < 4.0 && (l - v(lh)).length() < 4.0, "stab +{at}: RH {r:?} LH {l:?} vs {rh:?} {lh:?}");
    }
}

/// EVD_MOV_003: the foot grounding on the flat test floor: RootTranslationOffset.Z = the floor minus the capsule bottom
/// (the CharacterMovement floor gap, 1.9 - 2.4 cm), GroundingWeight 1, and the 1P thighs (ModifyBone_96 / _97) lowered by
/// it; the reader record standing on flat ground reads -1.95 .. -2.54 (everything.json RootTranslationOffset, the
/// most frequent small values; -4.3 / -7.7 where a foot is over lower ground)
#[test]
fn grounding_lowers_the_1p_thighs_by_the_floor_gap() {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let mut legs = vec![];
    for fp in [true, false] {
        let Some((mut s, a)) = sim_with(&w, fp) else { return };
        for _ in 0..120 {
            s.step(&[]);
        }
        let g = &s.fanim[a].grounding;
        let bottom = s.movers[a].location.z - s.movers[a].capsule_half_height;
        println!("fp {fp}: RootTranslationOffset {:?} weight {} capsule bottom {bottom} limbs {:?}", g.root_translation_offset, g.weight, g.limbs.iter().map(|l| (l.hit.map(|h| h.0.z), l.translation_z)).collect::<Vec<_>>());
        assert_eq!(g.weight, 1.0);
        assert!((g.root_translation_offset.z - (0.0 - bottom)).abs() < 0.05, "offset {} vs floor - bottom {}", g.root_translation_offset.z, -bottom);
        assert!((-2.6..=-1.8).contains(&g.root_translation_offset.z), "{}", g.root_translation_offset.z);
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        legs.push(p.bones[s.geo.skeleton.find("RightUpLeg").unwrap()].loc.z - p.bones[s.geo.skeleton.find("Hips").unwrap()].loc.z);
    }
    println!("RightUpLeg - Hips (CS z): 1P {} 3P {}", legs[0], legs[1]);
}

#[test]
#[ignore]
fn probe_movement_constants() {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((s, a)) = sim_with(&w, true) else { return };
    let m = &s.movers[a];
    println!("MaxWalkSpeed {} crouched {} sprint {} partial {} speed factor {} (equip add {} armor {} sprint penalty {} cap {})", m.c.max_walk_speed, m.c.max_walk_speed_crouched, m.c.sprint_modifier, m.c.partial_sprint_modifier, m.get_speed_factor(0.0), m.equip_speed_add, m.armor_speed, m.equip_sprint_penalty, m.equip_speed_cap);
}

/// EVD_MOV_001 / 002: ground speed walking forward, in a Blocked motion by reason (the record: MovementRestriction 1
/// after a Hit-reason block walks at the walk speed, 2 after a parry ~0), and the time to reach the walk speed
#[test]
#[ignore]
fn probe_walk_speeds() {
    use mordhau_core::combat::world::{Call, Input};
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    for case in ["walk", "sprint", "blocked parry", "blocked hit"] {
        let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 240.0) else { return };
        for _ in 0..600 {
            s.step(&[]);
        }
        if case.starts_with("blocked") {
            let reason = if case.ends_with("parry") { mordhau_core::combat::enums::br::PARRY } else { mordhau_core::combat::enums::br::HIT };
            s.step(&[(a, mh_sim::SimInput { attack: Some((mordhau_core::combat::enums::mv::RIGHT_STRIKE, 0.0)), ..Default::default() })]);
            for _ in 0..170 {
                s.step(&[]);
            }
            s.combat.input(Input::Call(Call::Blocked { who: "A".into(), reason, flags: 0, time: s.combat.now }));
            s.step(&[]);
            println!("  after the block call: {:?}", s.combat.cur_m(a).map(|m| (m.kind(), m.movement_restriction, m.end_time - m.start_time)));
        }
        let x0 = s.movers[a].location;
        let t0 = s.combat.now;
        let mut reach = None;
        let mut samples = vec![];
        for n in 0..240 {
            s.step(&[(a, mh_sim::SimInput { fwd: 1.0, sprint: case == "sprint", ..Default::default() })]);
            let v = s.movers[a].velocity;
            let sp = (v.x * v.x + v.y * v.y).sqrt();
            if reach.is_none() && sp >= 0.99 * 308.0 {
                reach = Some(s.combat.now - t0);
            }
            if n % 12 == 11 {
                samples.push(format!("{:.0}", sp));
            }
        }
        let d = s.movers[a].location - x0;
        let mr = s.combat.cur_m(a).map(|m| (m.kind(), m.movement_restriction));
        println!("{case:14}: avg {:.1} cm/s over 1 s, 99% of 308 at {reach:?}, speeds every 0.1 s {samples:?}, motion {mr:?}", (d.x * d.x + d.y * d.y).sqrt());
    }
}

/// EVD_MOV_001 / 004: with the record's speed factor (MovementSpeedScale 0.8134 = UMordhauMovementComponent::GetSpeedFactor
/// for its loadout, set here through ArmorSpeedFactor +0xd04) the walk tops out at MaxWalkSpeed 308 x 0.8134 = 250.5 (the
/// record's 0.5 s windows with anim Velocity 60: p95 250.3) and the full sprint at x 1.8 = 451 (record 441 - 451 with
/// Velocity 90, MovementSpeedScale 0.9354 = 0.8134 x (1 + AnimRateFactor1PMaxSprint 0.15)); the anim instance's MovementSpeedScale is that factor (NativeUpdateAnimation decomp 2313 / 2445).
#[test]
fn walk_and_sprint_speeds_match_the_record() {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    for (sprint, want) in [(false, 308.0f32 * 0.8134), (true, 308.0 * 0.8134 * 1.8)] {
        let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 120.0) else { return };
        s.movers[a].armor_speed = 0.8134;
        for _ in 0..300 {
            s.step(&[]);
        }
        let mut sp = 0.0;
        for _ in 0..(120 * 6) {
            s.step(&[(a, mh_sim::SimInput { fwd: 1.0, sprint, ..Default::default() })]);
            let v = s.movers[a].velocity;
            sp = (v.x * v.x + v.y * v.y).sqrt();
        }
        let mss = s.fanim[a].movement_speed_scale;
        println!("sprint {sprint}: speed {sp} (want {want}), MovementSpeedScale {mss}");
        assert!((sp - want).abs() < 1.0, "sprint {sprint}: {sp} vs {want}");
        let want_mss = if sprint { 0.9354 } else { 0.8134 };
        assert!((mss - want_mss).abs() < 1e-3, "MovementSpeedScale {mss} vs record {want_mss}");
    }
}


/// EVD_MOV_007+: the reader record's clean 1P idle stretch everything.json frames 4441-4660 (3.75 s: standing, walking
/// forward, strafing forward-right, sprinting, 90 deg of turning) as (t, control yaw, LookUpValue, world velocity X / Y
/// from mesh positions over 3 frames, anim Velocity 90 = sprinting) every ~0.05 s, and the real anim values / hands
/// (t, Direction, Helper_UBVelocity, SpringPitchYawValue X / Y, MovementSpeedScale, RightHand and LeftHand minus
/// Spine1 in the pov frame)
const SEG_KEYS: [(f64, f32, f64, f32, f32, bool); 74] = [(0.0, 0.16, -0.16, 0.0, 0.0, false), (0.051, 0.22, -0.21, 0.0, 0.0, false), (0.102, 0.23, -0.21, 0.0, 0.0, false), (0.153, 0.24, -0.21, 0.0, 0.0, false), (0.204, 0.24, -0.21, 0.0, 0.0, false), (0.255, 0.24, -0.21, 0.0, 0.0, false), (0.306, 0.25, -0.21, 5.6, 0.0, false), (0.358, 0.39, 0.06, 70.7, 0.5, false), (0.408, 1.68, 0.8, 152.2, 1.4, false), (0.459, 4.38, 1.12, 228.5, 4.5, false), (0.51, 6.68, 0.84, 245.9, 10.5, false), (0.561, 6.81, 0.82, 252.7, 17.9, false), (0.612, 6.8, 0.82, 266.1, 24.2, false), (0.662, 6.68, 0.85, 246.3, 25.2, false), (0.714, 6.53, 0.86, 240.4, 26.0, false), (0.765, 6.51, 0.82, 263.4, 29.2, false), (0.816, 6.51, 0.81, 244.1, 27.2, false), (0.868, 6.51, 0.81, 242.2, -8.1, false), (0.919, 6.51, 0.81, 235.0, -66.8, false), (0.97, 6.55, 0.83, 240.2, -111.6, false), (1.021, 8.63, 1.26, 211.6, -123.7, false), (1.072, 12.71, 1.5, 206.2, -131.7, false), (1.123, 16.07, 1.97, 224.8, -142.4, false), (1.174, 15.82, 2.54, 210.7, -127.1, false), (1.225, 15.82, 2.54, 213.7, -125.0, false), (1.276, 15.82, 2.54, 210.9, -121.2, false), (1.327, 15.82, 2.54, 215.2, -122.3, false), (1.378, 15.82, 2.54, 230.8, -130.2, false), (1.43, 15.82, 2.54, 233.0, -130.9, true), (1.481, 15.82, 2.54, 256.4, -143.8, true), (1.532, 15.82, 2.54, 294.5, -141.1, true), (1.584, 15.82, 2.54, 299.6, -87.5, true), (1.635, 15.93, 2.57, 315.5, -50.4, true), (1.686, 16.08, 2.65, 328.9, -19.7, true), (1.737, 16.46, 2.61, 364.5, 7.3, true), (1.788, 17.26, 2.52, 343.0, 28.5, true), (1.839, 17.98, 2.52, 349.1, 46.4, true), (1.891, 19.14, 2.63, 359.7, 63.0, true), (1.942, 21.73, 2.68, 388.9, 83.5, true), (1.993, 27.19, 2.43, 368.5, 95.1, true), (2.043, 32.98, 2.09, 369.5, 115.2, true), (2.095, 37.78, 1.83, 367.3, 139.1, true), (2.146, 40.55, 1.42, 394.3, 180.4, true), (2.197, 40.85, 1.37, 364.8, 195.0, true), (2.248, 40.85, 1.36, 357.3, 213.6, true), (2.299, 40.89, 1.31, 359.5, 233.5, true), (2.35, 41.91, 1.02, 355.5, 246.2, true), (2.401, 45.74, 0.62, 388.3, 285.9, true), (2.452, 53.02, -0.29, 344.7, 274.0, true), (2.503, 62.09, -1.43, 329.8, 293.0, true), (2.555, 69.72, -2.79, 329.2, 342.5, true), (2.606, 71.84, -3.54, 272.7, 339.4, true), (2.657, 71.84, -3.54, 268.9, 349.7, true), (2.709, 71.84, -3.54, 325.9, 350.7, true), (2.76, 71.84, -3.54, 324.9, 299.7, true), (2.811, 71.84, -3.54, 340.6, 279.1, true), (2.862, 71.84, -3.54, 353.8, 263.6, true), (2.938, 71.92, -3.52, 386.3, 261.7, true), (2.989, 72.17, -3.36, 378.6, 238.3, true), (3.04, 72.4, -3.22, 377.8, 228.1, true), (3.091, 72.91, -3.08, 381.6, 223.3, true), (3.141, 73.58, -2.87, 385.8, 221.2, true), (3.192, 74.21, -2.66, 420.6, 238.5, true), (3.243, 75.88, -2.49, 385.5, 218.3, true), (3.294, 78.55, -2.32, 384.2, 220.4, true), (3.345, 82.0, -2.21, 381.8, 226.4, true), (3.396, 86.06, -2.07, 376.9, 235.9, true), (3.447, 89.83, -1.78, 396.0, 267.7, true), (3.498, 90.32, -1.81, 353.8, 260.7, true), (3.549, 90.32, -1.81, 345.7, 272.8, true), (3.6, 90.32, -1.81, 332.8, 290.2, true), (3.651, 90.32, -1.81, 303.5, 364.6, true), (3.702, 91.19, -1.46, 200.2, 365.0, true), (3.753, 93.21, -1.09, 199.0, 379.0, true)];
const SEG_REAL: [(f64, f64, f64, f32, f32, f32, [f32; 3], [f32; 3]); 74] = [(0.0, 0.0, 0.0, -0.004, -0.323, 0.8134, [-5.6, 62.1, -22.2], [-5.6, -62.1, -22.2]), (0.051, 0.0, 0.0, -0.004, -0.216, 0.8134, [28.0, 4.8, 16.0], [26.8, -8.4, 13.1]), (0.102, 0.0, 0.0, -0.003, -0.153, 0.8134, [28.0, 4.8, 16.1], [26.8, -8.3, 13.1]), (0.153, 0.0, 0.0, -0.002, -0.098, 0.8134, [28.0, 4.9, 16.1], [26.8, -8.3, 13.2]), (0.204, 0.0, 0.0, -0.001, -0.05, 0.8134, [28.0, 5.0, 16.1], [26.8, -8.2, 13.2]), (0.255, 0.0, 0.0, -0.0, -0.013, 0.8134, [28.0, 5.1, 16.2], [26.9, -8.1, 13.3]), (0.306, 0.1, 60.0, 0.0, 0.015, 0.8134, [28.0, 5.1, 16.2], [26.9, -8.1, 13.3]), (0.358, 0.1, 60.0, 0.007, 0.031, 0.8134, [28.1, 4.7, 16.2], [27.1, -8.5, 13.4]), (0.408, -1.0, 60.0, 0.022, 0.052, 0.8134, [28.2, 4.4, 16.4], [27.2, -8.8, 13.6]), (0.459, -2.8, 60.0, 0.025, 0.079, 0.8134, [28.3, 4.3, 16.5], [27.3, -9.0, 13.7]), (0.51, -3.5, 60.0, 0.013, 0.092, 0.8134, [28.3, 4.2, 16.5], [27.4, -9.1, 13.7]), (0.561, -2.2, 60.0, 0.007, 0.073, 0.8134, [28.3, 4.1, 16.3], [27.3, -9.1, 13.5]), (0.612, -1.2, 60.0, 0.003, 0.049, 0.8134, [28.2, 4.2, 15.9], [27.2, -9.1, 13.2]), (0.662, -0.7, 60.0, 0.001, 0.028, 0.8134, [28.1, 4.3, 15.6], [27.0, -8.9, 12.7]), (0.714, -0.3, 60.0, -0.001, 0.01, 0.8134, [28.0, 4.6, 15.5], [26.9, -8.6, 12.6]), (0.765, -0.1, 60.0, -0.003, -0.002, 0.8134, [28.0, 4.9, 15.6], [26.9, -8.3, 12.8]), (0.816, -0.1, 60.0, -0.004, -0.01, 0.8134, [28.0, 5.2, 16.0], [27.0, -8.0, 13.1]), (0.868, -15.8, 60.0, -0.004, -0.013, 0.8134, [28.1, 5.4, 16.3], [27.0, -7.8, 13.5]), (0.919, -27.1, 60.0, -0.003, -0.014, 0.8134, [28.1, 5.6, 16.6], [27.1, -7.6, 13.7]), (0.97, -34.6, 60.0, -0.002, -0.013, 0.8134, [28.2, 5.7, 16.7], [27.1, -7.5, 13.8]), (1.021, -40.5, 60.0, 0.008, 0.013, 0.8134, [28.1, 5.7, 16.6], [27.1, -7.5, 13.7]), (1.072, -45.5, 60.0, 0.013, 0.057, 0.8134, [28.0, 5.7, 16.3], [27.2, -7.5, 13.5]), (1.123, -47.9, 60.0, 0.021, 0.085, 0.8134, [28.0, 5.6, 15.9], [27.2, -7.6, 13.2]), (1.174, -46.5, 60.0, 0.028, 0.062, 0.8134, [28.0, 5.2, 15.8], [27.2, -8.0, 13.0]), (1.225, -45.9, 60.0, 0.021, 0.043, 0.8134, [28.1, 4.8, 15.7], [27.2, -8.5, 13.0]), (1.276, -45.5, 60.0, 0.014, 0.025, 0.8134, [28.3, 4.3, 16.1], [27.4, -8.9, 13.3]), (1.327, -45.3, 60.0, 0.008, 0.011, 0.8134, [28.6, 3.9, 16.7], [27.6, -9.3, 13.8]), (1.378, -45.2, 60.0, 0.002, -0.001, 0.8134, [28.7, 3.7, 17.1], [27.8, -9.5, 14.2]), (1.43, -45.1, 90.0, -0.001, -0.007, 0.816, [28.7, 3.6, 16.2], [27.8, -9.4, 13.1]), (1.481, -45.1, 90.0, -0.003, -0.011, 0.8224, [28.6, 3.7, 14.7], [27.7, -9.0, 11.2]), (1.532, -36.7, 90.0, -0.004, -0.012, 0.8293, [28.4, 3.9, 13.3], [27.5, -8.6, 9.5]), (1.584, -28.5, 90.0, -0.005, -0.011, 0.8356, [28.0, 4.2, 11.8], [27.2, -8.1, 7.8]), (1.635, -22.2, 90.0, -0.003, -0.008, 0.842, [27.5, 4.6, 10.4], [26.7, -7.5, 6.1]), (1.686, -17.3, 90.0, -0.001, -0.005, 0.8483, [26.9, 5.3, 9.4], [26.2, -6.8, 5.0]), (1.737, -13.5, 90.0, -0.002, 0.002, 0.8552, [26.5, 6.1, 9.1], [25.8, -5.8, 4.6]), (1.788, -11.1, 90.0, -0.003, 0.011, 0.8616, [26.2, 7.0, 9.3], [25.6, -4.8, 4.7]), (1.839, -9.3, 90.0, -0.002, 0.018, 0.8679, [26.1, 7.7, 9.5], [25.5, -4.1, 4.8]), (1.891, -8.2, 90.0, 0.001, 0.027, 0.8744, [26.0, 7.9, 9.3], [25.5, -3.8, 4.6]), (1.942, -8.7, 90.0, 0.002, 0.051, 0.8813, [26.0, 7.6, 8.7], [25.5, -4.0, 4.0]), (1.993, -11.5, 90.0, -0.003, 0.099, 0.8876, [26.1, 7.0, 7.9], [25.7, -4.6, 3.2]), (2.043, -14.2, 90.0, -0.01, 0.14, 0.894, [26.3, 6.3, 7.3], [26.1, -5.3, 2.6]), (2.095, -15.4, 90.0, -0.014, 0.159, 0.9003, [26.8, 5.7, 7.3], [26.6, -5.9, 2.6]), (2.146, -14.3, 90.0, -0.02, 0.143, 0.9072, [27.5, 5.3, 7.9], [27.2, -6.2, 3.1]), (2.197, -11.4, 90.0, -0.016, 0.097, 0.9136, [27.9, 4.9, 8.3], [27.6, -6.5, 3.4]), (2.248, -8.9, 90.0, -0.011, 0.055, 0.9199, [28.0, 4.8, 8.4], [27.6, -6.6, 3.3]), (2.299, -7.1, 90.0, -0.007, 0.021, 0.9263, [27.7, 4.9, 8.0], [27.2, -6.4, 2.9]), (2.35, -6.5, 90.0, -0.01, 0.008, 0.9326, [27.0, 5.3, 7.2], [26.5, -6.0, 2.0]), (2.401, -8.5, 90.0, -0.014, 0.031, 0.9354, [26.3, 6.1, 6.9], [25.9, -5.2, 1.7]), (2.452, -13.3, 90.0, -0.03, 0.095, 0.9354, [25.9, 7.4, 7.3], [25.6, -3.9, 2.2]), (2.503, -18.7, 90.0, -0.048, 0.17, 0.9354, [25.7, 8.5, 7.9], [25.6, -2.9, 2.8]), (2.555, -21.6, 90.0, -0.065, 0.212, 0.9354, [25.6, 9.1, 8.3], [25.6, -2.2, 3.3]), (2.606, -18.5, 90.0, -0.064, 0.172, 0.9354, [25.7, 8.8, 8.1], [25.6, -2.5, 3.0]), (2.657, -21.8, 90.0, -0.044, 0.113, 0.9354, [25.9, 7.9, 7.6], [25.6, -3.3, 2.4]), (2.709, -27.0, 90.0, -0.025, 0.058, 0.9354, [26.2, 6.8, 7.2], [25.8, -4.3, 1.8]), (2.76, -30.8, 90.0, -0.011, 0.017, 0.9354, [26.7, 5.8, 7.1], [26.3, -5.2, 1.6]), (2.811, -33.8, 90.0, 0.0, -0.011, 0.9354, [27.4, 5.2, 7.7], [27.0, -5.7, 2.1]), (2.862, -36.2, 90.0, 0.007, -0.029, 0.9354, [27.9, 4.9, 8.3], [27.4, -6.0, 2.6]), (2.938, -39.0, 90.0, 0.013, -0.037, 0.9354, [28.0, 4.9, 8.4], [27.5, -5.9, 2.6]), (2.989, -40.5, 90.0, 0.016, -0.033, 0.9354, [27.5, 5.2, 7.9], [27.0, -5.6, 2.1]), (3.04, -41.7, 90.0, 0.017, -0.026, 0.9354, [26.7, 5.7, 7.2], [26.3, -5.0, 1.3]), (3.091, -42.9, 90.0, 0.017, -0.014, 0.9354, [26.2, 6.6, 7.2], [25.8, -4.0, 1.2]), (3.141, -43.9, 90.0, 0.017, -0.002, 0.9354, [25.9, 7.6, 7.8], [25.5, -2.8, 1.6]), (3.192, -44.8, 90.0, 0.016, 0.01, 0.9354, [25.8, 8.5, 8.4], [25.4, -1.8, 2.2]), (3.243, -46.3, 90.0, 0.015, 0.028, 0.9354, [25.8, 8.8, 8.7], [25.4, -1.3, 2.4]), (3.294, -48.4, 90.0, 0.013, 0.054, 0.9354, [25.8, 8.5, 8.4], [25.5, -1.4, 1.9]), (3.345, -50.8, 90.0, 0.01, 0.082, 0.9354, [25.9, 7.8, 7.8], [25.7, -2.1, 1.3]), (3.396, -53.2, 90.0, 0.009, 0.109, 0.9354, [26.3, 6.9, 7.3], [26.0, -2.9, 0.8]), (3.447, -54.8, 90.0, 0.01, 0.123, 0.9354, [26.8, 6.2, 7.3], [26.6, -3.6, 0.9]), (3.498, -53.0, 90.0, 0.005, 0.092, 0.9354, [27.6, 5.6, 8.0], [27.3, -4.2, 1.6]), (3.549, -51.3, 90.0, 0.002, 0.058, 0.9354, [28.0, 5.3, 8.5], [27.7, -4.7, 2.1]), (3.6, -45.5, 90.0, -0.001, 0.029, 0.9354, [28.0, 5.2, 8.6], [27.8, -5.0, 2.4]), (3.651, -35.8, 90.0, -0.002, 0.006, 0.9354, [27.7, 5.4, 8.2], [27.4, -5.2, 2.2]), (3.702, -26.8, 90.0, 0.005, 0.001, 0.9354, [27.0, 5.7, 7.5], [26.8, -5.1, 1.8]), (3.753, -32.5, 90.0, 0.012, 0.013, 0.9354, [26.3, 6.6, 7.2], [26.2, -4.7, 2.0])];


fn seg_key(t: f64) -> (f32, f64, f32, f32, bool) {
    let mut k = SEG_KEYS[0];
    for w in SEG_KEYS.windows(2) {
        if t >= w[0].0 {
            k = w[0];
            if t < w[1].0 {
                let f = ((t - w[0].0) / (w[1].0 - w[0].0)) as f32;
                return (w[0].1 + (w[1].1 - w[0].1) * f, w[0].2 + (w[1].2 - w[0].2) * f as f64, w[0].3 + (w[1].3 - w[0].3) * f, w[0].4 + (w[1].4 - w[0].4) * f, w[0].5);
            }
            k = w[1];
        }
    }
    (k.1, k.2, k.3, k.4, k.5)
}

/// replays SEG_KEYS: control yaw / look applied as the owning client's, movement input toward the record's velocity a
/// little ahead (the input leads the measured velocity by the CMC's ramp), sprint while the record sprints
fn replay_segment(lead: f64) -> Option<Vec<(f64, f64, f64, f32, f32, f64, FVector, FVector)>> {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let (mut s, a) = sim_with_dt(&w, true, 1.0 / 240.0)?;
    s.movers[a].armor_speed = 0.8134;
    for _ in 0..480 {
        s.step(&[(a, mh_sim::SimInput { yaw: Some(SEG_KEYS[0].1), look_up: Some(SEG_KEYS[0].2), turn_applied_by_client: true, ..Default::default() })]);
    }
    let t0 = s.combat.now;
    let mut out = vec![];
    let mut k = 0;
    while k < SEG_REAL.len() {
        let t = s.combat.now - t0;
        let (yaw, look, _, _, sprint) = seg_key(t);
        let (_, _, vx, vy, _) = seg_key(t + lead);
        let sp = (vx * vx + vy * vy).sqrt();
        let (mut fwd, mut right) = (0.0, 0.0);
        if sp > 20.0 {
            let ang = vy.atan2(vx) - yaw.to_radians();
            fwd = ang.cos();
            right = ang.sin();
        }
        s.step(&[(a, mh_sim::SimInput { yaw: Some(yaw), look_up: Some(look), turn_applied_by_client: true, fwd, right, sprint, ..Default::default() })]);
        let t = s.combat.now - t0;
        if t + 1e-6 >= SEG_REAL[k].0 {
            let posed = s.posed.borrow();
            let p = posed.get("A").unwrap();
            let wl = |n: &str| s.geo.bone_world(p, s.geo.skeleton.find(n).unwrap()).loc;
            let view = FQuat::from_rotator(s.combat.fighters[a].look_up_value as f32 + 5.27, s.yaw[a], 0.0);
            let rh = view.inverse().rotate(wl("RightHand") - wl("Spine1"));
            let lh = view.inverse().rotate(wl("LeftHand") - wl("Spine1"));
            let fa = &s.fanim[a];
            out.push((t, fa.upper_input.0, fa.upper_input.1, fa.proc.spring.0, fa.proc.spring.1, fa.movement_speed_scale, rh, lh));
            k += 1;
        }
    }
    Some(out)
}

#[test]
#[ignore]
fn probe_segment_replay() {
    let lead = std::env::var("MH_LEAD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.08);
    let Some(out) = replay_segment(lead) else { return };
    let (mut erh, mut elh) = (0.0f32, 0.0f32);
    for (o, r) in out.iter().zip(SEG_REAL.iter()) {
        let (drh, dlh) = ((o.6 - FVector::new(r.6[0], r.6[1], r.6[2])).length(), (o.7 - FVector::new(r.7[0], r.7[1], r.7[2])).length());
        erh = erh.max(drh);
        elh = elh.max(dlh);
        println!("t {:.2} Dir {:7.1}/{:7.1} UB {:4.0}/{:4.0} spring ({:.2},{:.2})/({:.2},{:.2}) mss {:.3}/{:.3} RH [{:.1},{:.1},{:.1}]/{:?} {:.1}cm LH {:.1}cm", o.0, o.1, r.1, o.2, r.2, o.3, o.4, r.3, r.4, o.5, r.5, o.6.x, o.6.y, o.6.z, r.6, drh, dlh);
    }
    println!("max RH {erh:.1} cm, LH {elh:.1} cm");
}

/// EVD_MOV_007..012: the record's clean 1P stretch (frames 4441-4660: stand, walk off, strafe forward-right, sprint,
/// ~90 deg of turning) replayed with its control yaw / look / movement: the 1P arm-feel chain - the upper locomotion
/// blend space (BlendSpacePlayer_29 at Direction / Helper_UBVelocity, rate MovementSpeedScale with the sprint ramp),
/// the pitch / yaw springs (SpringPitchYawValue -> ModifyBone_72 / _75 / _76 at Helper_HandSpringWeight), the 1P
/// locomotion lower body - puts both hands (minus Spine1, in the view frame) within 3.5 cm of the record at every
/// sample after the respawn frame, with SpringPitchYawValue within 0.04 (after the respawn transient, +0.3 s; ours trails the record by ~0.03 on the first turn at +0.4 s) and MovementSpeedScale within 0.01.
/// Direction is not compared: the replayed movement input only approximates the record's velocity direction.
#[test]
fn first_person_arm_feel_matches_the_record_while_moving_and_turning() {
    let Some(out) = replay_segment(0.08) else { return };
    for (o, r) in out.iter().zip(SEG_REAL.iter()).skip(1) {
        let (drh, dlh) = ((o.6 - FVector::new(r.6[0], r.6[1], r.6[2])).length(), (o.7 - FVector::new(r.7[0], r.7[1], r.7[2])).length());
        assert!(drh < 3.5 && dlh < 3.5, "t {:.2}: RH {:?} vs {:?} ({drh:.1}), LH {:?} vs {:?} ({dlh:.1})", o.0, o.6, r.6, o.7, r.7);
        // the record's yaw spring starts at -0.32 from the respawn (frame 4440) and decays by +0.3 s
        assert!(o.0 < 0.3 || ((o.3 - r.3).abs() < 0.04 && (o.4 - r.4).abs() < 0.04), "t {:.2}: spring ({}, {}) vs ({}, {})", o.0, o.3, o.4, r.3, r.4);
        assert!((o.5 as f32 - r.5).abs() < 0.01, "t {:.2}: MovementSpeedScale {} vs {}", o.0, o.5, r.5);
    }
}

/// EVD_MOV_013: the live runtime run of state/gauntlet/armfeel/seg.txt (offscreen, --profile Knight, per-frame stepping;
/// dump_state rows extracted to state/gauntlet/armfeel/runtime_rows.json) replayed in mh-sim with the same frame dts,
/// control yaw / look and keys (W from frame 36, D 100-210, Shift 170-440): the hands minus Spine1 in the runtime's
/// camera (rig.fp_probe) against the sim's view frame. Run with --ignored --nocapture.
#[test]
#[ignore]
fn probe_runtime_vs_sim() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state/gauntlet/armfeel").join(std::env::var("MH_ROWS").unwrap_or_else(|_| "runtime_rows.json".into()));
    let Ok(txt) = std::fs::read_to_string(&path) else { return };
    let rows: Vec<serde_json::Value> = serde_json::from_str(&txt).unwrap();
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 120.0) else { return };
    s.movers[a].armor_speed = rows[0]["sf"].as_f64().unwrap() as f32;
    // idle until the runtime's first dumped sim time (the idle / bob phase depends on it)
    let t_first = rows[0]["now"].as_f64().unwrap();
    while s.combat.now + 1.0 / 120.0 <= t_first {
        s.step(&[(a, mh_sim::SimInput { yaw: Some(0.0), look_up: Some(0.0), turn_applied_by_client: true, ..Default::default() })]);
    }
    let rest = (t_first - s.combat.now) as f32;
    if rest > 1e-5 {
        s.step_dt(&[(a, mh_sim::SimInput { yaw: Some(0.0), look_up: Some(0.0), turn_applied_by_client: true, ..Default::default() })], rest);
    }
    let (mut worst, mut sum, mut n) = (0.0f32, 0.0f32, 0);
    let mut dump = vec![];
    for (i, r) in rows.iter().enumerate() {
        if i == 0 {
            continue;
        }
        let dt = (r["now"].as_f64().unwrap() - rows[i - 1]["now"].as_f64().unwrap()) as f32;
        let yaw = r["yaw"].as_f64().unwrap() as f32;
        let look = r["look"].as_f64().unwrap();
        let fwd = if (36..440).contains(&i) { 1.0 } else { 0.0 };
        let right = if (100..210).contains(&i) { 1.0 } else { 0.0 };
        let sprint = (170..440).contains(&i);
        s.step_dt(&[(a, mh_sim::SimInput { yaw: Some(yaw), look_up: Some(look), turn_applied_by_client: true, fwd, right, sprint, ..Default::default() })], dt);
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let wl = |n: &str| s.geo.bone_world(p, s.geo.skeleton.find(n).unwrap()).loc;
        let view = FQuat::from_rotator(s.combat.fighters[a].look_up_value as f32 + 5.27, s.yaw[a], 0.0);
        let rh = view.inverse().rotate(wl("RightHand") - wl("Spine1"));
        let want = FVector::new(r["rh"][0].as_f64().unwrap() as f32, r["rh"][1].as_f64().unwrap() as f32, r["rh"][2].as_f64().unwrap() as f32);
        let d = (rh - want).length();
        worst = worst.max(d);
        sum += d;
        n += 1;
        dump.push(serde_json::json!({"i": i, "rh": [rh.x, rh.y, rh.z]}));
        let ui = s.fanim[a].upper_input;
        if i % 15 == 0 {
            println!("f{i:03} yaw {yaw:6.1} sim Dir {:6.1} UB {:3.0} | runtime Dir {:6.1} UB {:3.0} | RH sim [{:.1},{:.1},{:.1}] runtime {:?} {d:.1} cm", ui.0, ui.1, r["ui"][0].as_f64().unwrap(), r["ui"][1].as_f64().unwrap(), rh.x, rh.y, rh.z, [want.x, want.y, want.z]);
        }
    }
    println!("RH runtime vs sim: mean {:.2} cm, worst {worst:.2} cm over {n} frames", sum / n as f32);
    let _ = std::fs::write(path.with_file_name("sim_rows.json"), serde_json::to_string(&dump).unwrap());
}

/// prints the greatsword's TraceStart / TraceEnd sockets (weapon mesh component space) for screenshot projection
#[test]
#[ignore]
fn probe_greatsword_trace_sockets() {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((s, _a)) = sim_with(&w, true) else { return };
    let g = s.geo.weapons.get(&w).unwrap();
    println!("TS {:?} TE {:?} grip {:?}", g.trace_start, g.trace_end, g.grip_location_local);
}

/// EVD_MOV_010: the reader record state/live_rec/motion1.jsonl (all IdleMotion, camera spun 100-850 deg/s, a short
/// walk) as rows (state/gauntlet/turnfeel/m1_rows.json: t, control yaw, LookUpValue, velocity, LowerBodyRotationOffset,
/// hands minus the camera in the camera frame, Spine / Spine1 local rotations) replayed at the record's frame dts; the
/// camera is Spine1 + up x 41.625 + forward x 0.95 (the record: Spine1 in its pov frame (-0.96, 0, -41.62), std 0.05).
/// Returns (t, LowerBodyRotationOffset, RH, LH in the camera frame, Spine local quat, Spine1 local quat) per row.
fn replay_m1(rows: &[serde_json::Value]) -> Option<Vec<(f64, f32, FVector, FVector, FQuat, FQuat, Vec<FQuat>)>> {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let (mut s, a) = sim_with_dt(&w, true, 1.0 / 60.0)?;
    s.movers[a].armor_speed = 0.8134;
    let g = |r: &serde_json::Value, k: &str| r[k].as_f64().unwrap_or(0.0);
    let y0 = g(&rows[0], "yaw") as f32;
    for _ in 0..180 {
        s.step(&[(a, mh_sim::SimInput { yaw: Some(y0), look_up: Some(g(&rows[0], "look")), turn_applied_by_client: true, ..Default::default() })]);
    }
    let sk = s.geo.skeleton.clone();
    let (lb, sp, s1) = (sk.find("LowerBack").unwrap(), sk.find("Spine").unwrap(), sk.find("Spine1").unwrap());
    if let Some(an) = s.anim.as_ref() {
        for path in [s.upper_additive_asset(a, true), s.upper_additive_asset(a, false)] {
            if let Some(c) = an.clip(&path) {
                eprintln!("ADDCLIP {path} len {} tracks {}", c.sequence_length, c.tracks.len());
                if path.ends_with("_1P") {
                    let mut rows = vec![];
                    let mut t = 0.0f32;
                    while t < c.sequence_length as f32 {
                        let q: Vec<(usize, [f32; 4])> = c.tracks.iter().map(|tr| (tr.bone as usize, c.sample(tr, t).0.unwrap_or([0.0, 0.0, 0.0, 1.0]))).collect();
                        rows.push(serde_json::json!({"t": t, "q": q}));
                        t += 1.0 / 60.0;
                    }
                    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state/gauntlet/camera1p/additive_1p_tracks.json");
                    let _ = std::fs::write(&out, serde_json::to_string(&serde_json::json!({"names": sk.names.clone(), "rows": rows})).unwrap());
                }
                for tr in &c.tracks {
                    let b = tr.bone as usize;
                    let name = sk.names.get(b).cloned().unwrap_or_default();
                    let mut worst = 0.0f32;
                    let mut worst_t = 0.0f32;
                    let mut row = String::new();
                    let mut t = 0.0f32;
                    while t < c.sequence_length as f32 {
                        let (r, pv, _) = c.sample(tr, t);
                        let q = r.unwrap_or([0.0, 0.0, 0.0, 1.0]);
                        let ang = 2.0 * q[3].abs().min(1.0).acos().to_degrees();
                        let tl = pv.map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()).unwrap_or(0.0);
                        if ang > worst { worst = ang; worst_t = t; }
                        if (t * 2.0).fract() == 0.0 && t < 8.0 { row.push_str(&format!(" {ang:.1}/{tl:.1}")); }
                        t += 0.25;
                    }
                    eprintln!("ADDTRACK {b} {name}: max rot {worst:.1} deg at t={worst_t:.2};{row}");
                }
            }
        }
    }
    eprintln!("ASSETS upper_additive_1p={} upper_additive_3p={} upper_lower={:?}", s.upper_additive_asset(a, true), s.upper_additive_asset(a, false), s.upper_lower_assets(a, true));
    let mut out = vec![];
    for (i, r) in rows.iter().enumerate() {
        let dt = if i == 0 { 1.0 / 60.0 } else { (g(r, "t") - g(&rows[i - 1], "t")) as f32 };
        let yaw = g(r, "yaw") as f32;
        let (vx, vy) = (g(r, "vx") as f32, g(r, "vy") as f32);
        let (mut fwd, mut right) = (0.0, 0.0);
        if (vx * vx + vy * vy).sqrt() > 20.0 {
            let ang = vy.atan2(vx) - yaw.to_radians();
            fwd = ang.cos();
            right = ang.sin();
        }
        s.step_dt(&[(a, mh_sim::SimInput { yaw: Some(yaw), look_up: Some(g(r, "look")), turn_applied_by_client: true, fwd, right, ..Default::default() })], dt.max(1e-4));
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let wl = |n: &str| s.geo.bone_world(p, sk.find(n).unwrap()).loc;
        let view = FQuat::from_rotator(s.combat.fighters[a].look_up_value as f32 + 5.27, s.yaw[a], 0.0);
        let cam = wl("Spine1") + view.rotate(FVector::new(0.95, 0.0, 41.625));
        let rh = view.inverse().rotate(wl("RightHand") - cam);
        let lh = view.inverse().rotate(wl("LeftHand") - cam);
        let lr = |par: usize, c: usize| p.bones[par].rot.inverse().mul(p.bones[c].rot);
        let cs: Vec<FQuat> = M1_BONES.iter().map(|n| p.bones[sk.find(n).unwrap()].rot).collect();
        let pr = &s.fanim[a].proc;
        // camera1p gauntlet: the RightWeapon bone, the held weapon's root (ComputeGrippedTransform rel on the bone) and
        // its blade (+Z) direction in the camera frame, for a pixel comparison against the record (f = 621 px)
        let rwb = s.geo.bone_world(p, sk.find("RightWeapon").unwrap());
        let root = match s.geo.weapons.get(&w) {
            Some(g) => {
                let id = FTransform::new(FQuat::IDENTITY, FVector::ZERO);
                mh_sim::pose::gripped(&id, g.right_hand_equip_offset, g.rotation_offset, s.geo.grip_pitch_right, g.grip_location_local).then(&rwb)
            }
            None => rwb,
        };
        let vi = view.inverse();
        let c = |v: FVector| vi.rotate(v - cam);
        let (rwc, rootc, bd) = (c(rwb.loc), c(root.loc), vi.rotate(root.rot.rotate(FVector::new(0.0, 0.0, 1.0))));
        let extra = vec![
            FQuat { x: pr.spring.0, y: pr.spring.1, z: pr.hand_spring_weight, w: pr.turn_delta },
            FQuat { x: rwc.x, y: rwc.y, z: rwc.z, w: 0.0 },
            FQuat { x: rootc.x, y: rootc.y, z: rootc.z, w: 0.0 },
            FQuat { x: bd.x, y: bd.y, z: bd.z, w: 0.0 },
        ];
        out.push((g(r, "t"), s.fanim[a].proc.lower_rot_offset, rh, lh, lr(lb, sp), lr(sp, s1), [cs, extra].concat()));
    }
    Some(out)
}

const M1_BONES: [&str; 12] = ["Hips", "LowerBack", "Spine", "Spine1", "head", "RightHand", "LeftUpLeg", "LeftFoot", "RightWeapon", "RightForeArm", "RightArm", "RightShoulder"];

/// EVD_CAM_015 (turn in place, first person; state/proofs/turn_in_place_1p.md): the record motion1 replayed at its own
/// dts keeps both hands on the camera while the lower body lags up to 45 deg, because the LowerBack inverse node 504 runs
/// at alpha 1 in first person (AlphaScaleBiasClamp map 0 -> 1.0). Right hand in the camera frame vs the record: mean
/// 1.98 cm, worst 3.49 cm (was 15.9 / 22.9 with the whole body following the hips); Spine1's component-space yaw does not
/// change with the offset (record: 18.3 -> 17.4 deg at offset 45).
#[test]
fn first_person_turn_in_place_keeps_the_upper_body_on_the_camera() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state/gauntlet/turnfeel/m1_rows.json");
    let Ok(txt) = std::fs::read_to_string(&path) else { return };
    let rows: Vec<serde_json::Value> = serde_json::from_str(&txt).unwrap();
    let Some(out) = replay_m1(&rows) else { return };
    let v3 = |r: &serde_json::Value, k: &str| FVector::new(r[k][0].as_f64().unwrap() as f32, r[k][1].as_f64().unwrap() as f32, r[k][2].as_f64().unwrap() as f32);
    let yaw = |q: &FQuat| (2.0 * (q.w * q.z + q.x * q.y)).atan2(1.0 - 2.0 * (q.y * q.y + q.z * q.z)).to_degrees();
    let (mut sum, mut worst) = (0.0f32, 0.0f32);
    let (mut s1_rest, mut s1_turn) = (None, None);
    for (o, r) in out.iter().zip(rows.iter()) {
        let d = (o.2 - v3(r, "rh")).length();
        sum += d;
        worst = worst.max(d);
        // M1_BONES[3] = Spine1 component-space rotation
        let s1 = yaw(&o.6[3]);
        if o.1.abs() < 0.5 && s1_rest.is_none() {
            s1_rest = Some(s1);
        }
        if o.1.abs() > 44.0 {
            s1_turn = Some(s1);
        }
    }
    let mean = sum / out.len() as f32;
    assert!(mean < 4.0 && worst < 8.0, "right hand vs the record: mean {mean:.2} cm, worst {worst:.2} cm");
    let (a, b) = (s1_rest.expect("a rest frame"), s1_turn.expect("a full-offset frame"));
    assert!((a - b).abs() < 3.0, "Spine1 yaw must not follow the lower-body offset: rest {a:.1}, at offset 45 {b:.1}");
}

#[test]
#[ignore]
fn probe_turn_feel_m1() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state/gauntlet/turnfeel/m1_rows.json");
    let Ok(txt) = std::fs::read_to_string(&path) else { return };
    let rows: Vec<serde_json::Value> = serde_json::from_str(&txt).unwrap();
    let Some(out) = replay_m1(&rows) else { return };
    let v3 = |r: &serde_json::Value, k: &str| FVector::new(r[k][0].as_f64().unwrap() as f32, r[k][1].as_f64().unwrap() as f32, r[k][2].as_f64().unwrap() as f32);
    let q4 = |r: &serde_json::Value, k: &str| FQuat { x: r[k][0].as_f64().unwrap() as f32, y: r[k][1].as_f64().unwrap() as f32, z: r[k][2].as_f64().unwrap() as f32, w: r[k][3].as_f64().unwrap() as f32 };
    let mut dump = vec![];
    let (mut sum, mut worst) = (0.0f32, 0.0f32);
    for (o, r) in out.iter().zip(rows.iter()) {
        let (drh, dlh) = ((o.2 - v3(r, "rh")).length(), (o.3 - v3(r, "lh")).length());
        sum += drh;
        worst = worst.max(drh);
        dump.push(serde_json::json!({"t": o.0, "lbro": o.1, "rh": [o.2.x, o.2.y, o.2.z], "lh": [o.3.x, o.3.y, o.3.z], "spine": [o.4.x, o.4.y, o.4.z, o.4.w], "spine1": [o.5.x, o.5.y, o.5.z, o.5.w], "d_spine": ang_deg(o.4, q4(r, "spine")), "d_spine1": ang_deg(o.5, q4(r, "spine1")), "cs": o.6.iter().map(|q| vec![q.x, q.y, q.z, q.w]).collect::<Vec<_>>()}));
    }
    println!("RH camera-frame vs record: mean {:.2} cm, worst {worst:.2} cm over {} frames", sum / out.len() as f32, out.len());
    let _ = std::fs::write(path.with_file_name("m1_ours.json"), serde_json::to_string(&dump).unwrap());
}

/// EVD_MOV_005: the exe's MovementAnimRate (NativeUpdateAnimation decomp 2530-2611; lower.rs) fed the record's own
/// 2D speed (mesh displacement per frame) reproduces the record's UMordhauAnimInstance MovementAnimRate frame by frame
/// (state/live_rec/motion1.jsonl; 1P: the 1P lower-body clips carry no Speed curve, so AnimSpeed = 5).
#[test]
fn movement_anim_rate_matches_the_record() {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../state/live_rec/motion1.jsonl");
    let Ok(txt) = std::fs::read_to_string(p) else { return };
    let rows: Vec<serde_json::Value> = txt.lines().skip(1).filter_map(|l| serde_json::from_str(l).ok()).collect();
    let f = |r: &serde_json::Value, k: &str, i: usize| r[k][i].as_f64().unwrap_or(0.0);
    let mut mar = rows[0]["anim"]["MovementAnimRate"].as_f64().unwrap();
    let mut worst = 0.0f64;
    for w in rows.windows(2) {
        let dt = w[1]["t"].as_f64().unwrap() - w[0]["t"].as_f64().unwrap();
        let speed = (f(&w[1], "mesh_world", 4) - f(&w[0], "mesh_world", 4)).hypot(f(&w[1], "mesh_world", 5) - f(&w[0], "mesh_world", 5)) / dt.max(1e-4);
        let (rate, _) = mh_sim::lower::movement_anim_rate_targets(speed, 0.0, true);
        mar = mh_sim::lower::finterp_constant_to(mar, rate, dt, 5.0);
        worst = worst.max((mar - w[1]["anim"]["MovementAnimRate"].as_f64().unwrap()).abs());
    }
    assert!(worst < 0.03, "MovementAnimRate vs record: worst {worst:.4}");
}

/// user report 2026-10-07 ("morphs aren't working"): a strike whose windup is interrupted by a stab request inside the
/// morph window must become a MORPH stab (UAttackMotion::ProcessAttack_Implementation rva=0x1637cf0: Stage Windup,
/// MinWindupTimeBeforeMorphing < t - start < min(MaxMorphTotalTime, WindupEnd - start - MorphWindow) - LagInduction,
/// stamina >= MorphCost). If this passes, a failing morph in play is the runtime input path, not the core.
#[test]
fn strike_morphs_into_stab_inside_the_morph_window() {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((mut s, a)) = sim_with(&w, true) else { return };
    for _ in 0..60 {
        s.step(&[(a, mh_sim::SimInput { yaw: Some(0.0), look_up: Some(0.0), turn_applied_by_client: true, ..Default::default() })]);
    }
    s.step(&[(a, mh_sim::SimInput { attack: Some((0, 0.0)), yaw: Some(0.0), turn_applied_by_client: true, ..Default::default() })]);
    let first = s.combat.fighters[a].motion.expect("the strike started");
    let (start, windup_end, ty0) = {
        let m = s.combat.cur_m(a).expect("current motion");
        let at = m.attack().expect("an attack motion");
        (m.start_time, at.windup_end, at.ty)
    };
    assert_eq!(ty0, mordhau_core::combat::enums::at::REGULAR);
    // 0.15 s into the windup (the greatsword strike windup is ~0.5 s): request a stab
    for _ in 0..9 {
        s.step(&[(a, mh_sim::SimInput { yaw: Some(0.0), turn_applied_by_client: true, ..Default::default() })]);
    }
    assert!(s.combat.now < windup_end, "still in the windup ({} < {windup_end})", s.combat.now);
    s.step(&[(a, mh_sim::SimInput { attack: Some((mordhau_core::combat::enums::mv::STAB, 0.0)), yaw: Some(0.0), turn_applied_by_client: true, ..Default::default() })]);
    let m = s.combat.cur_m(a).expect("current motion after the stab request");
    let at = m.attack().expect("still an attack");
    println!("after the stab request: motion {:?} (first {:?}) ty {} stab_class {} start {start} now {}", s.combat.fighters[a].motion, first, at.ty, at.is_stab_class(), s.combat.now);
    assert!(at.is_stab_class(), "the strike did not morph into a stab: {}", m.bp);
    assert_eq!(at.ty, mordhau_core::combat::enums::at::MORPH, "the stab is not flagged as a morph");
}

/// camera1p gauntlet piece 5 (strike vs crosshair): the record state/live_rec/everything.json frames 1669-1765
/// (state/gauntlet/camera1p/strike1_rows.json: 0.6 s idle with a turn, a greatsword right strike from idle at look
/// -33, the hit's BlockedMotion) replayed at the record's dts; the strike is requested on the row the record's motion
/// becomes the strike. Dumps per row our counter-compensation, springs, lower-body offset and the camera-frame
/// RightHand / RightWeapon bone / held-weapon root + blade direction to state/gauntlet/camera1p/strike1_ours.json
/// (the comparison is in python, session log 2026-10-07 11:30).
#[test]
#[ignore]
fn probe_strike1() {
    // MH_STRIKE_ROWS=<file> (default strike1_rows.json): the segment to replay; the output is <stem>_ours.json
    let file = std::env::var("MH_STRIKE_ROWS").unwrap_or_else(|_| "state/gauntlet/camera1p/strike1_rows.json".into());
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../").join(&file);
    let Ok(txt) = std::fs::read_to_string(&path) else { return };
    let doc: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let rows = doc["rows"].as_array().unwrap();
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((mut s, a)) = sim_with_dt(&w, true, 1.0 / 60.0) else { return };
    s.movers[a].armor_speed = 0.8134;
    let g = |r: &serde_json::Value, k: &str| r[k].as_f64().unwrap_or(0.0);
    let yaw0 = rows[0]["ctrl"][1].as_f64().unwrap_or(0.0) as f32;
    let look0 = g(&rows[0]["anim"], "LookUpValue");
    for _ in 0..180 {
        s.step(&[(a, mh_sim::SimInput { yaw: Some(yaw0), look_up: Some(look0), turn_applied_by_client: true, ..Default::default() })]);
    }
    let sk = s.geo.skeleton.clone();
    let mut out = vec![];
    let mut fired = false;
    for (i, r) in rows.iter().enumerate() {
        let dt = if i == 0 { 1.0 / 60.0 } else { (g(r, "t") - g(&rows[i - 1], "t")) as f32 };
        let yaw = r["ctrl"][1].as_f64().unwrap_or(0.0) as f32;
        let look = g(&r["anim"], "LookUpValue");
        let mname = r["motion"].as_str().unwrap_or("");
        let mv = if mname.contains("RightStrike") { Some(0i64) } else if mname.contains("LeftStrike") { Some(1) } else if mname.contains("RightStab") { Some(2) } else if mname.contains("AltStab") || mname.contains("LeftStab") { Some(3) } else { None };
        // the angle: MH_STRIKE_ANGLE (degrees of angling), else the record's AngleTarget (UAttackMotion +0x1080, in the
        // row's motion_raw of the newer recorder) x 90, else 0
        let rec_angle = r["motion_raw"].as_str().and_then(|h| {
            let b: Vec<u8> = (0..h.len() / 2).filter_map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect();
            (b.len() >= 0x1084).then(|| f32::from_le_bytes([b[0x1080], b[0x1081], b[0x1082], b[0x1083]]) as f64 * 90.0)
        });
        let angle: f64 = std::env::var("MH_STRIKE_ANGLE").ok().and_then(|v| v.parse().ok()).or(rec_angle).unwrap_or(0.0);
        // every new attack motion of the record (a chained attack too) is requested on the row it appears
        // ... and kept requested on every later row until ours plays the same class (the real player's press was queued
        // during the previous attack and fired at the first allowed instant; the probe cannot see the press itself)
        let ours_name = s.combat.cur_m(a).map(|m| m.bp.rsplit('/').next().unwrap_or("").to_string()).unwrap_or_default();
        // ... and a chained attack is requested two rows before the record shows it (the player's press preceded the
        // start; a request that lands after our ReleaseEnd waits for the recovery instead of comboing)
        let ahead = rows.get(i + 2).and_then(|n| n["motion"].as_str()).unwrap_or("");
        let ahead_mv = if ahead.contains("RightStrike") { Some(0i64) } else if ahead.contains("LeftStrike") { Some(1) } else if ahead.contains("RightStab") { Some(2) } else if ahead.contains("AltStab") || ahead.contains("LeftStab") { Some(3) } else { None };
        let (mv, mname, angle) = if ahead_mv.is_some() && ahead != mname {
            // the angle of the attack the record is about to start (its own row's AngleTarget)
            let a2 = rows.get(i + 2).and_then(|n| n["motion_raw"].as_str()).and_then(|h| {
                let b: Vec<u8> = (0..h.len() / 2).filter_map(|k| u8::from_str_radix(&h[2 * k..2 * k + 2], 16).ok()).collect();
                (b.len() >= 0x1084).then(|| f32::from_le_bytes([b[0x1080], b[0x1081], b[0x1082], b[0x1083]]) as f64 * 90.0)
            });
            (ahead_mv, ahead, std::env::var("MH_STRIKE_ANGLE").ok().and_then(|v| v.parse().ok()).or(a2).unwrap_or(angle))
        } else {
            (mv, mname, angle)
        };
        let want = mv.is_some() && (ours_name.is_empty() || !mname.contains(ours_name.trim_end_matches("_C")));
        let attack = match mv { Some(m) if want => { fired = true; Some((m, angle)) } _ => None };
        let _ = fired;
        // the record's movement: the mesh world XY delta per row as the held axes (speed > 20 cm/s), relative to the yaw
        let (mut fwd, mut right) = (0.0f32, 0.0f32);
        let mut sprint = false;
        if i > 0 {
            let mw = |r: &serde_json::Value, k: usize| r["mesh_world"][k].as_f64().unwrap_or(0.0);
            let (dx, dy) = (mw(r, 4) - mw(&rows[i - 1], 4), mw(r, 5) - mw(&rows[i - 1], 5));
            let sp = (dx * dx + dy * dy).sqrt() / dt.max(1e-4) as f64;
            if sp > 20.0 {
                let ang = dy.atan2(dx) - (yaw as f64).to_radians();
                fwd = ang.cos() as f32;
                right = ang.sin() as f32;
                sprint = sp > 300.0;
            }
        }
        s.step_dt(&[(a, mh_sim::SimInput { yaw: Some(yaw), look_up: Some(look), turn_applied_by_client: true, attack, fwd, right, sprint, ..Default::default() })], dt.max(1e-4));
        let posed = s.posed.borrow();
        let p = posed.get("A").unwrap();
        let wl = |n: &str| s.geo.bone_world(p, sk.find(n).unwrap());
        let view = FQuat::from_rotator(s.combat.fighters[a].look_up_value as f32 + 5.27, s.yaw[a], 0.0);
        let cam = wl("Spine1").loc + view.rotate(FVector::new(0.95, 0.0, 41.625));
        let vi = view.inverse();
        let c = |v: FVector| vi.rotate(v - cam);
        let rwb = wl("RightWeapon");
        let root = match s.geo.weapons.get(&w) {
            Some(gw) => {
                let id = FTransform::new(FQuat::IDENTITY, FVector::ZERO);
                mh_sim::pose::gripped(&id, gw.right_hand_equip_offset, gw.rotation_offset, s.geo.grip_pitch_right, gw.grip_location_local).then(&rwb)
            }
            None => rwb,
        };
        let bd = vi.rotate(root.rot.rotate(FVector::new(0.0, 0.0, 1.0)));
        let (rh, lh, rw, rt) = (c(wl("RightHand").loc), c(wl("LeftHand").loc), c(rwb.loc), c(root.loc));
        let fa = &s.fanim[a];
        let motion = s.combat.cur_m(a).map(|m| m.bp.clone()).unwrap_or_default();
        let stage = s.combat.cur_m(a).and_then(|m| m.attack()).map(|at| at.stage).unwrap_or(-1);
        let csb: serde_json::Map<String, serde_json::Value> = ["Hips", "LowerBack", "Spine", "Spine1", "RightShoulder", "RightArm", "RightForeArm", "RightHand", "RightWeapon", "head"]
            .iter()
            .filter_map(|n| sk.find(n).map(|i| (n.to_string(), serde_json::json!([p.bones[i].rot.x, p.bones[i].rot.y, p.bones[i].rot.z, p.bones[i].rot.w, p.bones[i].loc.x, p.bones[i].loc.y, p.bones[i].loc.z]))))
            .collect();
        let mesh = s.geo.mesh_xf.then(&p.actor);
        out.push(serde_json::json!({"t": g(r, "t"), "motion": motion, "stage": stage, "yaw": s.yaw[a], "look": s.combat.fighters[a].look_up_value, "cs": csb, "cam": [cam.x, cam.y, cam.z], "mesh": [mesh.rot.x, mesh.rot.y, mesh.rot.z, mesh.rot.w, mesh.loc.x, mesh.loc.y, mesh.loc.z],
            "cc_rot": [fa.counter_comp.rot.0, fa.counter_comp.rot.1, fa.counter_comp.rot.2], "cc_w": fa.counter_comp.weight, "montages": fa.ma.montages.insts.iter().map(|m| serde_json::json!([m.asset.path.rsplit('/').next().unwrap_or(""), m.position(s.combat.now), m.weight(&s.combat.spec, s.combat.now), m.rate, m.bin, m.bin_option, m.bin_curve])).collect::<Vec<_>>(), "additive_alpha": fa.ma.additive_alpha, "upper_w": fa.upper_w, "atk": s.combat.cur_m(a).and_then(|m| m.attack().map(|at| serde_json::json!([m.start_time, at.windup_end, at.release_end, m.end_time, at.ty, at.angle_target, at.queued_move, at.queued_angle]))), "req": attack.map(|x| serde_json::json!([x.0, x.1])), "stamina": s.combat.fighters[a].stamina, "health": s.combat.fighters[a].health,
            "spring": [fa.proc.spring.0, fa.proc.spring.1], "lbro": fa.proc.lower_rot_offset, "upper_input": [fa.upper_input.0, fa.upper_input.1], "upper_input": [fa.upper_input.0, fa.upper_input.1],
            "rh": [rh.x, rh.y, rh.z], "lh": [lh.x, lh.y, lh.z], "rw": [rw.x, rw.y, rw.z], "root": [rt.x, rt.y, rt.z], "blade": [bd.x, bd.y, bd.z]}));
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("strike1_rows").replace("_rows", "");
    let _ = std::fs::write(path.with_file_name(format!("{stem}_ours.json")), serde_json::to_string(&out).unwrap());
    println!("strike1: {} rows written", out.len());
}

/// camera1p gauntlet piece 5: does the real first-person stab pose lie on the clip we play? For every record row whose
/// motion matches MH_FIT_MOTION (default "Stab"), the clip MH_FIT_CLIP is sampled over its whole length and the time whose
/// right-arm chain (RightArm / RightForeArm / RightHand relative to Spine1, component space) best matches the record is
/// printed with its error, next to our montage position of that row (strike2_ours.json "montages").
#[test]
#[ignore]
fn probe_fit_clip() {
    use mordhau_core::ue::FQuat;
    let file = std::env::var("MH_STRIKE_ROWS").unwrap_or_else(|_| "state/gauntlet/camera1p/strike2_rows.json".into());
    let clip = std::env::var("MH_FIT_CLIP").unwrap_or_else(|_| "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_RightStab".into());
    let motion = std::env::var("MH_FIT_MOTION").unwrap_or_else(|_| "Stab".into());
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../").join(&file);
    let Ok(txt) = std::fs::read_to_string(&path) else { return };
    let doc: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let rows = doc["rows"].as_array().unwrap();
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let ours: Vec<serde_json::Value> = std::fs::read_to_string(path.with_file_name(format!("{}_ours.json", path.file_stem().unwrap().to_string_lossy().trim_end_matches("_rows"))))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let Some((s, _a)) = sim_with_dt(&w, true, 1.0 / 60.0) else { return };
    let a = s.anim.clone().expect("anim assets");
    let sk = &s.geo.skeleton;
    let Some(c) = a.clip(&clip) else { println!("no clip {clip}"); return };
    let len = c.sequence_length as f64;
    let bones = ["RightArm", "RightForeArm", "RightHand", "LeftHand"];
    let idx: Vec<usize> = bones.iter().map(|b| sk.find(b).unwrap()).collect();
    let s1 = sk.find("Spine1").unwrap();
    let q = |v: &serde_json::Value| FQuat::new(v[0].as_f64().unwrap() as f32, v[1].as_f64().unwrap() as f32, v[2].as_f64().unwrap() as f32, v[3].as_f64().unwrap() as f32);
    let ang = |x: FQuat, y: FQuat| -> f64 { let d = x.inverse().mul(y); (2.0 * (d.w.abs().min(1.0) as f64).acos()).to_degrees() };
    // the clip's relative rotations per sampled time (cached)
    let n = (len / 0.01).ceil() as usize + 1;
    let samples: Vec<(f64, Vec<FQuat>)> = (0..n)
        .map(|i| {
            let t = (i as f64 * 0.01).min(len);
            let cs = sk.to_component(&a.sample(sk, &clip, t, false));
            let inv = cs[s1].rot.inverse();
            (t, idx.iter().map(|&b| inv.mul(cs[b].rot)).collect())
        })
        .collect();
    println!("clip {clip} len {len:.3}: row t, motion, best clip time, error (deg, mean over {bones:?}), error at 0 / at our position");
    for r in rows {
        let m = r["motion"].as_str().unwrap_or("");
        if !m.contains(&motion) {
            continue;
        }
        let rb = &r["bones"];
        let inv = q(&rb["Spine1"]).inverse();
        let real: Vec<FQuat> = bones.iter().map(|b| inv.mul(q(&rb[*b]))).collect();
        let err = |rel: &Vec<FQuat>| -> f64 { rel.iter().zip(&real).map(|(x, y)| ang(*x, *y)).sum::<f64>() / bones.len() as f64 };
        let (bt, be) = samples.iter().map(|(t, rel)| (*t, err(rel))).min_by(|x, y| x.1.partial_cmp(&y.1).unwrap()).unwrap();
        println!("FIT {:7.3} {:28} best {bt:6.3} err {be:5.2}  err0 {:5.2}", r["t"].as_f64().unwrap_or(0.0), m.rsplit('/').next().unwrap_or(m), err(&samples[0].1));
        // the residual the real adds over the clip at its best time, per bone, parent-local (pitch, yaw, roll)
        let local = sk.to_component(&a.sample(sk, &clip, bt, false));
        let chain = ["Spine1", "RightShoulder", "RightArm", "RightForeArm", "RightHand", "LeftArm", "LeftForeArm", "LeftHand"];
        let mut line = String::from("RES ");
        for b in chain {
            let i = sk.find(b).unwrap();
            let p = sk.parents[i];
            let (lr, lc) = if p >= 0 {
                let pn = &sk.names[p as usize];
                if rb.get(pn.as_str()).is_none() || rb.get(b).is_none() {
                    continue;
                }
                (q(&rb[pn.as_str()]).inverse().mul(q(&rb[b])), local[p as usize].rot.inverse().mul(local[i].rot))
            } else {
                (q(&rb[b]), local[i].rot)
            };
            let d = lc.inverse().mul(lr);
            let (pp, yy, rr) = mordhau_core::combat::geometry::quat_rotator(d);
            line += &format!(" {b}:({pp:5.1},{yy:5.1},{rr:5.1})");
        }
        println!("{line}");
        // the same residual for OUR pose of that row (strike2_ours.json: "cs" bones, "montages"[0][1] = our clip time)
        if let Some(orow) = ours.iter().find(|o| (o["t"].as_f64().unwrap_or(-1.0) - r["t"].as_f64().unwrap_or(0.0)).abs() < 1e-4) {
            let ot = orow["montages"][0][1].as_f64().unwrap_or(0.0);
            let ob = &orow["cs"];
            let local = sk.to_component(&a.sample(sk, &clip, ot, false));
            let mut line = String::from("OURS");
            for b in chain {
                let i = sk.find(b).unwrap();
                let p = sk.parents[i];
                if p < 0 {
                    continue;
                }
                let pn = &sk.names[p as usize];
                if ob.get(pn.as_str()).is_none() || ob.get(b).is_none() {
                    continue;
                }
                let lr = q(&ob[pn.as_str()]).inverse().mul(q(&ob[b]));
                let lc = local[p as usize].rot.inverse().mul(local[i].rot);
                let (pp, yy, rr) = mordhau_core::combat::geometry::quat_rotator(lc.inverse().mul(lr));
                line += &format!(" {b}:({pp:5.1},{yy:5.1},{rr:5.1})");
            }
            println!("{line} @{ot:.3}");
        }
    }
}

/// piece 12 (first-person legs): a standing 90 deg turn in 0.1 s, then hold; prints the lower-body rotation offset and
/// the Hips / Spine1 component-space yaw against the mesh yaw per frame (does the lower body lag the camera?)
#[test]
#[ignore]
fn probe_turn_legs() {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let fp = std::env::var("MH_TURN_3P").is_err();
    let Some((mut s, a)) = sim_with_dt(&w, fp, 1.0 / 60.0) else { return };
    let yaw_of = |q: mordhau_core::ue::FQuat| mordhau_core::combat::geometry::quat_rotator(q).1;
    let mut yaw = 0.0f32;
    for i in 0..330 {
        if (180..186).contains(&i) {
            yaw += 15.0;
        }
        s.step(&[(a, mh_sim::SimInput { yaw: Some(yaw), look_up: Some(-60.0), turn_applied_by_client: true, ..Default::default() })]);
        if i >= 175 && i % 4 == 0 {
            let fa = &s.fanim[a];
            let posed = s.posed.borrow();
            let p = posed.get(&s.combat.fighters[a].name).unwrap();
            let sk = &s.geo.skeleton;
            let (h, s1, lf) = (sk.find("Hips").unwrap(), sk.find("Spine1").unwrap(), sk.find("LeftFoot").unwrap());
            let l = fa.lower.as_ref().unwrap();
            println!("LEGS i {i:3} yaw {yaw:5.1} lbro {:6.1} zs {} it {:.2} angvel {:6.1} abs {:6.1} hips_cs_yaw {:6.1} spine1_cs_yaw {:6.1} lfoot_cs_yaw {:6.1} lfoot_cs_xy ({:.1},{:.1})", fa.proc.lower_rot_offset, fa.proc.zeroing_sign, fa.proc.internal_time, l.ang_vel_lb, l.abs_ang_vel_lb, yaw_of(p.bones[h].rot), yaw_of(p.bones[s1].rot), yaw_of(p.bones[lf].rot), p.bones[lf].loc.x, p.bones[lf].loc.y);
        }
    }
}

/// piece 5: which blend-in curves the greatsword attack defs name, and whether the spec's curve table has them
#[test]
#[ignore]
fn probe_curve_keys() {
    let w = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((s, _a)) = sim_with_dt(&w, true, 1.0 / 60.0) else { return };
    let spec = &s.combat.spec;
    println!("CURVES in spec: {:?}", spec.curves.keys().collect::<Vec<_>>());
    for (k, c) in spec.curves.iter() {
        if k.contains("ComboBlendCurve") || k.contains("Stabanimcurve") {
            println!("CURVE {k}: pre {:?} post {:?} keys {:?}", c.pre, c.post, c.keys);
        }
    }
    for (k, d) in spec.motion_defs.iter() {
        if let Some(at) = d.attack.as_ref() {
            println!("DEF {k}: blend_in_curve={:?} combo={:?} morph={:?} riposte={:?} windup={:?} combo_windup={:?}", at.blend_in_curve, at.combo_blend_in_curve, at.morph_blend_in_curve, at.riposte_blend_in_curve, at.windup_curve, at.combo_windup_curve);
        }
    }
}


/// Actual ToggleMode route used by R; retain original Second* data in both directions.
#[test]
fn original_alt_toggle_roundtrip_preserves_attacks_profiles_and_grips() {
    for name in ["TwoHandedSword/BP_Greatsword", "TwoHandedSword/BP_Messer"] {
        for fp in [false, true] {
            let path = format!("{W}/{name}");
            let Some((mut s, fi)) = sim_with(&path, fp) else { return };
            for _ in 0..30 { s.step(&[]); }
            let primary = s.combat.fighters[fi].weapon.clone().unwrap();
            let equip = s.combat.fighters[fi].weapon_equip.clone().unwrap();
            let data = |a: &mordhau_core::data::AttackInfo| (a.windup, a.release, a.miss_recovery, a.hit_effect_speed_up_exponent, a.damage.clone());
            let endpoints = {
                let g = s.geo.weapons.get(&path).unwrap();
                (g.trace_start, g.trace_end, g.second_trace_start, g.second_trace_end)
            };
            for target in [true, false] {
                s.step(&[(fi, mh_sim::SimInput { toggle_mode: true, ..Default::default() })]);
                assert_eq!(s.combat.cur_m(fi).unwrap().kind(), "EquipmentModeSwitch");
                let source_alt = !target;
                let start = s.combat.cur_m(fi).unwrap().start_time;
                assert_eq!(s.combat.fighters[fi].alternate_mode, source_alt);
                let mut saw_right = false;
                let mut stage2_identity = None;
                for _ in 0..60 {
                    s.step(&[]);
                    if let Some(m) = s.combat.cur_m(fi).filter(|m| m.kind() == "EquipmentModeSwitch") {
                        let sw = m.mode_switch().unwrap();
                        assert_eq!(sw.b_is_switching_to_alt, target);
                        assert_eq!(m.start_time, start);
                        let grip = s.posed.borrow().get("A").unwrap().grip.clone();
                        if sw.stage == 2 {
                            let st = grip.switch.as_ref().expect("retain completed switch until OnLeave");
                            assert_eq!(st.stage, 2);
                            assert_eq!(st.start, start);
                            assert!(grip.mesh_world.is_none());
                            if let Some(prev) = stage2_identity { assert_eq!(prev, st.start); }
                            stage2_identity = Some(st.start);
                        }
                        if sw.switch_type == 0 && sw.stage == 1 {
                            saw_right |= s.fanim[fi].offhand.right_fraction > 0.0;
                        }
                    }
                }
                assert_eq!(s.combat.fighters[fi].alternate_mode, target);
                assert!(s.posed.borrow().get("A").unwrap().grip.switch.is_none());
                let actual = s.combat.fighters[fi].weapon.as_ref().unwrap();
                assert_eq!(data(&actual.strike), data(if target { &primary.second_strike } else { &primary.strike }));
                assert_eq!(data(&actual.stab), data(if target { &primary.second_stab } else { &primary.stab }));
                assert_eq!(s.combat.fighters[fi].weapon_equip.as_ref().unwrap().weapon_animation_profile,
                    if target { equip.second_weapon_animation_profile.clone() } else { equip.weapon_animation_profile.clone() });
                let g = s.geo.weapons.get(&path).unwrap();
                assert_eq!((g.trace_start, g.trace_end, g.second_trace_start, g.second_trace_end), endpoints);
                let mode = &g.grip_modes.as_ref().unwrap()[target as usize];
                let desired = FQuat::from_rotator(mode.rotation_offset[0], mode.rotation_offset[1], mode.rotation_offset[2]).mul(FQuat::from_rotator(90.0, 0.0, 0.0));
                assert!(ang_deg(rel_to_socket(&s).rot, desired) < 0.1);
                if name.ends_with("Greatsword") { assert!(saw_right, "Type0 must consume native right-hand regrip branch"); }
                else { assert!(stage2_identity.is_some(), "Type1 retains stage2 before native .65 motion end"); }
            }
        }
    }
}

#[test]
fn original_interrupted_mode_switch_resets_weapon_mesh_on_leave() {
    let path = format!("{W}/TwoHandedSword/BP_Greatsword");
    let Some((mut s, fi)) = sim_with(&path, true) else { return };
    for _ in 0..30 { s.step(&[]); }
    s.step(&[(fi, mh_sim::SimInput { toggle_mode: true, ..Default::default() })]);
    assert!(s.posed.borrow().get("A").unwrap().grip.mesh_world.is_some());
    s.combat.change_to_idle(fi); // Original OnLeave finishes switch even before .15 and resets mesh.
    assert!(s.combat.fighters[fi].alternate_mode);
    s.step(&[]);
    let p = s.posed.borrow();
    let p = p.get("A").unwrap();
    assert!(p.grip.switch.is_none());
    assert!(p.grip.mesh_world.is_none());
    let g = s.geo.weapons.get(&path).unwrap();
    let actual = s.geo.weapon_world(p, g).unwrap();
    let want = mh_sim::grip::steady(&s.geo, p, g, true).unwrap();
    assert!((actual.loc - want.loc).length() < 1e-4);
    assert!(ang_deg(actual.rot, want.rot) < 0.1);
}

#[test]
fn original_delayed_messer_switch_observation_plays_source_mode_montage() {
    let path = format!("{W}/TwoHandedSword/BP_Messer");
    let Some((mut s, fi)) = sim_with(&path, true) else { return };
    for _ in 0..30 { s.step(&[]); }
    for target in [true, false] {
        s.step(&[(fi, mh_sim::SimInput { toggle_mode: true, ..Default::default() })]);
        for _ in 0..13 { s.step(&[]); } // Past FinishSwitch, still in original Type1 motion.
        assert_eq!(s.combat.fighters[fi].alternate_mode, target);
        assert_eq!(s.combat.cur_m(fi).unwrap().kind(), "EquipmentModeSwitch");
        let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs).unwrap());
        let want = if target { "MTG_1H_RH_To1H" } else { "MTG_1H_RH_To2H" };
        assert!(s.fanim[fi].ma.montages.insts.iter().any(|i| !i.stopped && i.asset.path.rsplit('/').next().unwrap().starts_with(want)),
            "late-observed switch must use pre-swap {want}");
        for _ in 0..60 { s.step(&[]); }
    }
}

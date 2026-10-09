//! The catapult (exe_pseudo.rs: UPseudoVehicleMovementComponent with SecondaryComponents) on DU_Arena's collision:
//! driving forward reaches MaxWalkSpeed 187, turning follows the TurningFactorByVelocity curve and the
//! AddTurnDegrees -> PhysicsRotation chain, reversing into the arena wall stops on the rear secondary (BackCapsule,
//! 282 cm behind the root) with every secondary free of the wall, a turn that would swing the rear into the wall is
//! refused (SweepRotationTestOnlySecondary), and SecondaryTurn is clamped to SecondaryTurnLimit 90. Data: extract/json
//! and the install; SKIP (pass) without them.

use mh_character::exe::Mode;
use mh_character::exe_pseudo::{pseudo_character_records, PseudoCfg, PseudoInput};
use mh_character::uemath::v;
use mh_character::world::World;
use mh_character::{CharacterRecords, CharacterSource, ExeMovement, RecordsJson};
use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::path::PathBuf;
use std::sync::Arc;

const DT: f32 = 1.0 / 60.0;

fn root() -> PathBuf {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    dir.join("../../..")
}

fn bp(rel: &str) -> Option<String> {
    std::fs::read_to_string(root().join("extract/json/Mordhau/Content/Mordhau/Blueprints").join(rel)).ok()
}

fn rec() -> Option<CharacterRecords> {
    let p = root().join("core/tests/golden/character/records.json");
    Some(RecordsJson(&std::fs::read_to_string(&p).ok()?).load().unwrap())
}

fn arena() -> Option<(mh_level::LevelData, CollisionWorld)> {
    let vfs = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/du_arena.umap"))?.trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs));
    let d = read(&pk, &pkg);
    let w = CollisionWorld::build(&pk, &d, None);
    Some((d, w))
}

fn catapult(w: &dyn World, at: [f64; 3], yaw: f32) -> Option<ExeMovement> {
    let j = bp("Interactables/SiegeEngines/BP_Catapult.json")?;
    let veh = bp("VehicleComponents/BP_VehicleCatapult.json");
    let curve = |p: &str| std::fs::read_to_string(root().join("extract/json").join(format!("{p}.json"))).ok();
    let cfg = PseudoCfg::from_bp_json(&j, veh.as_deref(), &curve).unwrap();
    let r = pseudo_character_records(&rec()?, &j).unwrap();
    let top = v(at[0] as f32, at[1] as f32, at[2] as f32 + 100.0);
    let h = w.line_trace(top, v(at[0] as f32, at[1] as f32, at[2] as f32 - 2000.0))?;
    let mut m = ExeMovement::new_pseudo(&r, &j, cfg, v(at[0] as f32, at[1] as f32, h.impact_point.z + 60.0 + 2.15), yaw).unwrap();
    m.world_time = 10.0;
    m.pseudo.as_mut().unwrap().controlled = true;
    m.pseudo_frame(w, DT, &PseudoInput::default());
    Some(m)
}

#[test]
fn catapult_cfg() {
    let Some(j) = bp("Interactables/SiegeEngines/BP_Catapult.json") else { return eprintln!("SKIP: no extract/json") };
    let curve = |p: &str| std::fs::read_to_string(root().join("extract/json").join(format!("{p}.json"))).ok();
    let veh = bp("VehicleComponents/BP_VehicleCatapult.json");
    let c = PseudoCfg::from_bp_json(&j, veh.as_deref(), &curve).unwrap();
    assert_eq!(c.secondaries.len(), 3);
    assert_eq!(c.secondaries[2].rel, v(-282.0, 0.0, 0.2));
    assert!(c.secondaries[2].step_capable && !c.secondaries[0].step_capable);
    assert!(!c.reverse_backwards_turning);
    assert_eq!(c.secondary_turn_limit, 90.0);
    assert!(c.turning_factor_curve.is_some() && c.turning_accel_curve.is_none());
}

#[test]
fn catapult_drives_turns_and_stops_on_its_rear() {
    let Some((d, w)) = arena() else { return };
    let Some(_) = rec() else { return eprintln!("SKIP: no golden records") };
    let red = d.spawns.iter().find(|s| s.name == "SpawnRed").expect("SpawnRed").xf.translation();
    let blue = d.spawns.iter().find(|s| s.name == "SpawnBlue").expect("SpawnBlue").xf.translation();
    let yaw = ((blue[1] - red[1]) as f32).atan2((blue[0] - red[0]) as f32).to_degrees();
    let mid = [(red[0] + blue[0]) / 2.0, (red[1] + blue[1]) / 2.0, red[2]];
    let Some(mut m) = catapult(&w, mid, yaw) else { return eprintln!("SKIP: no extract/json") };
    assert_eq!(m.mode, Mode::Walking, "on the arena floor");
    // forward: MaxWalkSpeed 187 at MaxAcceleration 512 (0.37 s)
    for _ in 0..90 {
        m.pseudo_frame(&w, DT, &PseudoInput { fwd: 1.0, ..Default::default() });
    }
    let sp = (m.velocity.x * m.velocity.x + m.velocity.y * m.velocity.y).sqrt();
    assert!((sp - 187.0).abs() < 0.5, "{sp}");
    // turning while moving: the yaw follows (curve 0.25 -> 0.15 by speed / 187; TurningVelocity at 1 / s)
    let y0 = m.yaw;
    for _ in 0..120 {
        m.pseudo_frame(&w, DT, &PseudoInput { fwd: 1.0, right: 1.0, ..Default::default() });
    }
    let turned = mh_character::siege::normalize_axis_vm(m.yaw - y0);
    println!("turned {turned}");
    assert!(turned > 1.0 && turned < 60.0, "{turned}");
    // stop, then reverse for a long time: the wall behind stops the rear capsule
    for _ in 0..120 {
        m.pseudo_frame(&w, DT, &PseudoInput::default());
    }
    for _ in 0..1800 {
        m.pseudo_frame(&w, DT, &PseudoInput { fwd: -1.0, ..Default::default() });
    }
    let sp = (m.velocity.x * m.velocity.x + m.velocity.y * m.velocity.y).sqrt();
    println!("loc {:?} vel {:?} speed {sp} mode {:?} yaw {}", m.location, m.velocity, m.mode, m.yaw);
    let l0 = m.location;
    for _ in 0..60 {
        m.pseudo_frame(&w, DT, &PseudoInput { fwd: -1.0, ..Default::default() });
    }
    let moved = ((m.location.x - l0.x).powi(2) + (m.location.y - l0.y).powi(2)).sqrt();
    println!("moved in 1 s: {moved} -> {:?}", m.location);
    // pinned against the wall: at most a slow slide along it (the 187 cm/s drive is spent on the wall)
    assert!(moved < 20.0, "stopped against the wall: {moved}");
    // nothing of it is inside the wall, and the rear (BackCapsule, inflated 1 cm) touches it
    assert!(!m.pseudo_overlap_secondaries(&w, m.location, m.yaw, 0.0));
    assert!(!w.overlap_capsule(m.location, m.capsule_radius() - 0.1, m.half_height() - 0.1));
    assert!(m.pseudo_overlap_secondaries(&w, m.location, m.yaw, 3.0), "the rear is at the wall");
    // a hard turn there swings the 3 m rear into the wall: SweepRotationTestOnlySecondary refuses (some frame blocks)
    let mut blocked = false;
    for _ in 0..120 {
        m.pseudo_frame(&w, DT, &PseudoInput { right: 1.0, ..Default::default() });
        blocked |= m.pseudo.as_ref().unwrap().last_rotation_block.is_some();
        assert!(!m.pseudo_overlap_secondaries(&w, m.location, m.yaw, 0.0), "never turned into the wall");
    }
    assert!(blocked, "the rotation test fired");
}

#[test]
fn secondary_turn_limit() {
    let Some((d, w)) = arena() else { return };
    let Some(_) = rec() else { return eprintln!("SKIP: no golden records") };
    let red = d.spawns.iter().find(|s| s.name == "SpawnRed").expect("SpawnRed").xf.translation();
    let blue = d.spawns.iter().find(|s| s.name == "SpawnBlue").expect("SpawnBlue").xf.translation();
    let mid = [(red[0] + blue[0]) / 2.0, (red[1] + blue[1]) / 2.0, red[2]];
    let Some(mut m) = catapult(&w, mid, 0.0) else { return eprintln!("SKIP: no extract/json") };
    for _ in 0..200 {
        m.pseudo_frame(&w, DT, &PseudoInput { turn: 1.0, ..Default::default() });
    }
    assert_eq!(m.pseudo.as_ref().unwrap().secondary_turn_value, 90.0);
    for _ in 0..400 {
        m.pseudo_frame(&w, DT, &PseudoInput { turn: -1.0, ..Default::default() });
    }
    assert_eq!(m.pseudo.as_ref().unwrap().secondary_turn_value, -90.0);
}

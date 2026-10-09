//! Traced-hit parity on the user's data (skips without the spec matrix / paks / character records): a Longsword swing
//! posed by mh-sim's clip stand-in reaches a defender in front, never before its Release begins (a trace runs in
//! Release only: UAttackMotion::ExecuteAttackTracingAndLogic rva=0x161c390), and a farther defender is reached later.

use mh_parity::traced::{Anim, Traced};
use mh_parity::LS;
use std::path::PathBuf;

#[test]
fn clip_swing_contact_after_windup() {
    let recs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json");
    if !recs.exists() {
        return eprintln!("SKIP: no character records");
    }
    let t = match Traced::new(&recs, 0.001, Anim::Clip) {
        Ok(t) => t,
        Err(e) => return eprintln!("SKIP: {e}"),
    };
    let v = t.probe_weapon(LS, &[]).expect("probe");
    let s = &v["strike"];
    let c = s["contact_time"].as_f64().expect("the strike reaches some distance");
    assert!(c >= 0.561 - 1e-3, "contact {c} before the Longsword's windup (0.561) ended");
    let by = s["contact_by_dist"].as_object().unwrap();
    let times: Vec<f64> = ["70", "110", "150"].iter().filter_map(|d| by[*d].as_f64()).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "farther defenders are reached later: {times:?}");
}

/// rust-parity r7 turn sweep (graph): turning swings report the yaw change the turn caps allow over the last 0.2 s,
/// and a straight swing at the defender reports bearing 0 / turn 0
#[test]
fn turn_sweep_reports_capped_turns() {
    let recs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json");
    if !recs.exists() {
        return eprintln!("SKIP: no character records");
    }
    let t = match Traced::new(&recs, 0.001, Anim::Graph) {
        Ok(t) => t,
        Err(e) => return eprintln!("SKIP: {e}"),
    };
    let v = t.turn_sweep(LS).expect("sweep");
    let rows = v["strike"].as_array().unwrap();
    assert!(rows.len() > 20, "{} strike swings reached", rows.len());
    for r in rows {
        let (y0, rate, turn, brg) = (r[1].as_f64().unwrap(), r[2].as_f64().unwrap(), r[4].as_f64().unwrap(), r[5].as_f64().unwrap());
        assert!(turn.abs() <= rate.abs() * 0.2 + 1.0, "turn {turn} over 0.2 s exceeds the requested rate {rate}");
        if rate == 0.0 {
            assert!(turn.abs() < 1e-3 && (brg + y0).abs() < 1e-3, "straight swing: turn {turn}, bearing {brg}, y0 {y0}");
        }
    }
}

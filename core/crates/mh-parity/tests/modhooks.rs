//! modhooks.rs (rust-parity r6): the NoChamber mod's OnBlockedMelee branches as a layer over the World, on the golden
//! record dump (core/tests/golden/spec.json, git-ignored: skips when absent).
//!   experimental parry (BP_MorhauCharacter ubergraph 5897..6040): ParryUpTime = (now - StartTime) + ParryExtensionDuration
//!   -> the parry's recovery follows the block by ParryExtensionDuration + NonHeldParryExtensionAndRiposteWindowExtra,
//!   so parry_up_block grows one for one with the block offset (vanilla: constant).

use mh_parity::modhooks::ModHooks;
use mh_parity::{Mode, Prober, LS};
use mordhau_core::data::{RecordsJsonExe, SpecSource};
use std::path::PathBuf;
use std::rc::Rc;

fn prober() -> Option<Prober> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/spec.json");
    let text = std::fs::read_to_string(&p).ok()?;
    Some(Prober::new(Rc::new(RecordsJsonExe(&text).load_spec().expect("spec")), Mode::Exe))
}

#[test]
fn hooks_from_layer() {
    let h = ModHooks::from_layer(&serde_json::json!({"mod_hooks": {"experimental_parry_duration": 0.15, "chftp_stam_cost": 15.0}}));
    assert_eq!(h.exp_parry, Some(0.15));
    assert_eq!(h.chftp_stam_cost, Some(15.0));
    assert!(!h.chftp_exclude_stabs);
    assert!(!ModHooks::from_layer(&serde_json::json!({})).any(), "no mod_hooks = vanilla / pak defaults");
}

#[test]
fn experimental_parry_up_time_follows_the_block() {
    let Some(mut p) = prober() else { return eprintln!("SKIP: no golden spec") };
    // vanilla: no grid
    assert!(p.parry(LS).get("parry_up_block_at").is_none());
    p.hooks = ModHooks { exp_parry: Some(0.15), ..Default::default() };
    let m = p.parry(LS);
    let grid: Vec<(f64, f64)> = m["parry_up_block_at"].as_array().unwrap().iter()
        .map(|x| (x[0].as_f64().unwrap(), x[1].as_f64().unwrap())).collect();
    assert!(grid.len() >= 5, "grid {grid:?}");
    // recovery - block = ext + the native extension (+ < 2 ticks): the same for every block offset
    let gaps: Vec<f64> = grid.iter().map(|(b, r)| r - b).collect();
    let (lo, hi) = gaps.iter().fold((f64::MAX, f64::MIN), |a, g| (a.0.min(*g), a.1.max(*g)));
    assert!(hi - lo < 0.0025, "recovery - block not constant: {gaps:?}");
    assert!(lo > 0.15, "gap {lo} <= ParryExtensionDuration");
}

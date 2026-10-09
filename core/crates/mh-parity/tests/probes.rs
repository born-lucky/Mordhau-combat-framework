//! mh-parity probes on the golden record dump (core/tests/golden/spec.json, Triternion data, git-ignored: the tests
//! skip when it is absent). Values checked against the GDScript probes (state/parity/probe_vanilla_out.json, combat
//! r13): Longsword windup 0.561 / parry riposte window 0.15 / blocked_p2 22; Fists parried by Fists (parry_attacker).

use mh_parity::{Mode, Prober, LS};
use mordhau_core::data::{RecordsJson, RecordsJsonExe, SpecSource};
use std::path::PathBuf;
use std::rc::Rc;

const FISTS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/BP_FistsWeapon";

fn prober(mode: Mode) -> Option<Prober> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/spec.json");
    let text = std::fs::read_to_string(&p).ok()?;
    let spec = match mode {
        Mode::Compat => RecordsJson(&text).load_spec(),
        Mode::Exe => RecordsJsonExe(&text).load_spec(),
    }
    .expect("spec");
    Some(Prober::new(Rc::new(spec), mode))
}

#[test]
fn parry_attacker_mask_rule() {
    let Some(p) = prober(Mode::Compat) else { return };
    assert_eq!(p.parry_attacker(LS), LS);
    assert_eq!(p.parry_attacker(FISTS), FISTS, "held Fists (ParryMask 4) cannot block a Longsword (AttackMask 1)");
}

#[test]
fn compat_matches_gdscript_probe_values() {
    let Some(p) = prober(Mode::Compat) else { return };
    let tl = p.attack_timeline(LS, 0);
    assert!((tl.windup - 0.561).abs() < 1e-9, "Longsword strike windup {}", tl.windup);
    let fp = p.parry(FISTS);
    assert!(fp["blocked_p2"].as_f64().unwrap() >= 0.0, "Fists parry probe found no Blocked: {fp:?}");
    let rw = p.riposte_window(FISTS);
    assert!((rw - 0.15).abs() < 1e-9, "Fists riposte window {rw}");
    let lp = p.parry(LS);
    assert_eq!(lp["blocked_p2"].as_f64(), Some(22.0));
}

#[test]
fn exe_mode_finds_same_tick_rows() {
    // the f32 clock puts a row a few ulp below its tick's scheduled time; the probes compare ticks, so the parry probe
    // still finds the block row and its stamina drain (it read -1 before the tick fix)
    let Some(p) = prober(Mode::Exe) else { return };
    let lp = p.parry(LS);
    assert!(lp["block_drain"].as_i64().unwrap() > 0, "exe parry block_drain {:?}", lp["block_drain"]);
    let rw = p.riposte_window(FISTS);
    assert!(rw.is_finite() && rw > 0.0, "exe Fists riposte window {rw}");
}

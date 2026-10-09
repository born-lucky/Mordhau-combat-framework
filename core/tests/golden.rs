//! Golden-trace parity: every combat scenario in core/tests/scenarios/combat.json is replayed through mordhau-core
//! and compared, tick by tick, with the GDScript reference's trace (core/tests/golden/combat/<name>.jsonl, written by
//! godot/tools/export_golden.gd) - every fighter's state, every emitted event and every log line.
//!
//! Numbers compare EXACTLY (f64 bit-equal: the reference computes in GDScript doubles and rounds to f32 exactly where
//! the exe's single precision is emulated; the port mirrors both). MORDHAU_GOLDEN_F32=1 relaxes numbers to
//! "equal after rounding to f32" (for a spec source whose floats are stored as f32, e.g. mh-spec).
//! The golden data is Triternion-derived and git-ignored: without it the test reports SKIP and passes, unless
//! MORDHAU_GOLDEN_REQUIRED=1. MORDHAU_GOLDEN_ONLY=a,b runs just those scenarios.

use mordhau_core::combat::world::{Call, Input};
use mordhau_core::combat::World;
use mordhau_core::data::{RecordsJson, SpecSource};
use mordhau_core::snapshot;
use serde_json::Value;
use std::path::PathBuf;
use std::rc::Rc;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}
fn f(v: &Value, k: &str, d: f64) -> f64 {
    v[k].as_f64().unwrap_or(d)
}
fn i(v: &Value, k: &str, d: i64) -> i64 {
    v[k].as_f64().map(|x| x as i64).unwrap_or(d)
}

fn input_of(ev: &Value, alias: &dyn Fn(&str) -> String) -> Input {
    let who = s(ev, "who");
    match ev["kind"].as_str().unwrap() {
        "attack" => Input::Attack { who, mv: i(ev, "move", 0), angle: f(ev, "angle", 0.0) },
        "feint" => Input::Feint { who },
        "parry" => Input::Parry { who, bt: i(ev, "bt", 0) },
        "release_block" => Input::ReleaseBlock { who },
        "switch_mode" => Input::SwitchMode { who },
        "toggle_mode" => Input::ToggleMode { who },
        "contact" => Input::Contact { who, target: s(ev, "target"), bone: ev["bone"].as_str().unwrap_or("Spine1").into() },
        "set" => {
            let v = &ev["value"];
            Input::Call(match s(ev, "field").as_str() {
                "stamina" => Call::SetStamina { who, v: v.as_f64().unwrap() as i64 },
                "health" => Call::SetHealth { who, v: v.as_f64().unwrap() as i64 },
                "armor_tier_override" => Call::SetArmorTierOverride { who, v: v.as_f64().unwrap() as i64 },
                "airborne" => Call::SetAirborne { who, v: v.as_bool().unwrap() },
                "holding_block" => Call::SetHoldingBlock { who, v: v.as_bool().unwrap() },
                "preset_flip" => Call::SetPresetFlip { who, v: v.as_bool().unwrap() },
                "b_is_left_arm_disabled" => Call::SetArmDisabled { who, left: true, v: v.as_bool().unwrap() },
                "b_is_right_arm_disabled" => Call::SetArmDisabled { who, left: false, v: v.as_bool().unwrap() },
                "airborne_time" => Call::SetAirborneTime { who, v: v.as_f64().unwrap() },
                "look_up_value" => Call::SetLookUp { who, v: v.as_f64().unwrap() },
                x => panic!("set {x}"),
            })
        }
        "weapon_no_drop" => Input::Call(Call::WeaponNoDrop { who }),
        "blocked" => Input::Call(Call::Blocked { who, reason: i(ev, "reason", 0), flags: i(ev, "flags", 0), time: f(ev, "time", 0.0) }),
        "request_parry" => Input::Call(Call::RequestParry { who, bt: i(ev, "bt", 0), ftp: ev["ftp"].as_bool().unwrap_or(true) }),
        "preset_attack" => Input::Call(Call::PresetAttack { who, mv: i(ev, "move", 0), angle: f(ev, "angle", 0.0) }),
        "attack_now" => Input::Call(Call::AttackNow { who, mv: i(ev, "move", 0), angle: f(ev, "angle", 0.0) }),
        "equip_right" => Input::Call(Call::EquipRight { who, weapon: alias(&s(ev, "weapon")) }),
        k => panic!("unknown input kind {k}"),
    }
}

/// first difference between golden `g` and port `p`, as "path: golden vs port"
fn diff(path: &str, g: &Value, p: &Value, f32_mode: bool) -> Option<String> {
    match (g, p) {
        (Value::Number(a), Value::Number(b)) => {
            let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            // lwn = LastWindupNormalizedTime: the only value through pow() (GetSmoothedWindUpNormalizedTime); the
            // reference's C runtime pow (Godot build) and Rust's (MSVC UCRT) may differ by 1 ulp. Animation-only
            // value (no rule reads it), so it alone gets a 1-ulp tolerance.
            let ulp1 = path.ends_with(".lwn") && (x.to_bits() as i64 - y.to_bits() as i64).abs() <= 1;
            let eq = if f32_mode { x as f32 == y as f32 } else { x == y || ulp1 };
            (!eq).then(|| format!("{path}: {x:?} vs {y:?}"))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: len {} vs {} ({g} vs {p})", a.len(), b.len()));
            }
            a.iter().zip(b).enumerate().find_map(|(k, (x, y))| diff(&format!("{path}[{k}]"), x, y, f32_mode))
        }
        (Value::Object(a), Value::Object(b)) => {
            for k in a.keys() {
                if !b.contains_key(k) {
                    return Some(format!("{path}.{k}: missing in port"));
                }
            }
            for k in b.keys() {
                if !a.contains_key(k) {
                    return Some(format!("{path}.{k}: extra in port"));
                }
            }
            a.iter().find_map(|(k, x)| diff(&format!("{path}.{k}"), x, &b[k], f32_mode))
        }
        _ => (g != p).then(|| format!("{path}: {g} vs {p}")),
    }
}

fn run_scenario(spec: &Rc<mordhau_core::data::Spec>, sc: &Value, alias: &dyn Fn(&str) -> String, golden: &str, f32_mode: bool) -> Result<usize, String> {
    // The scenario as the reference parsed it (header line): Godot's JSON parser is not correctly rounded
    // (0.008333333333333333 -> 0.00833333333333333), so dt / t / angles are taken bit for bit from the golden header.
    let mut header: Value = serde_json::from_str(golden.lines().next().unwrap_or("{}")).map_err(|e| format!("header: {e}"))?;
    mordhau_core::data::decode_exact(&mut header);
    let sc = if header["parsed"].is_object() { &header["parsed"] } else { sc };
    let mut w = World::new_reference(spec.clone(), f(sc, "dt", 0.001));
    if sc.get("mode").is_some() {
        w.set_game_mode(&s(&sc["mode"], "game_mode"), &s(&sc["mode"], "game_state"));
    }
    for fd in sc["fighters"].as_array().unwrap() {
        let fi = w.add_fighter(&s(fd, "name"), &alias(&s(fd, "weapon")), &alias(&s(fd, "left")));
        if fd.get("team").is_some() {
            w.fighters[fi].team = i(fd, "team", 255);
        }
    }
    for ev in sc["inputs"].as_array().unwrap() {
        let t = f(ev, "t", 0.0);
        w.at(t, input_of(ev, alias));
    }
    let mut lines = golden.lines();
    lines.next(); // header
    let (mut hits0, mut log0) = (0usize, 0usize);
    let until = f(sc, "until", 0.0);
    let mut n = 0usize;
    loop {
        let row = snapshot::row(&w, hits0, log0);
        hits0 = w.hits.len();
        log0 = w.events.len();
        let g: Value = match lines.next() {
            Some(l) => {
                let mut v: Value = serde_json::from_str(l).map_err(|e| format!("golden row {n}: {e}"))?;
                mordhau_core::data::decode_exact(&mut v);
                v
            }
            None => return Err(format!("port ran past the golden trace at tick {}", w.tick_n)),
        };
        if let Some(d) = diff("", &g, &row, f32_mode) {
            return Err(format!("tick {} (t={}): {d}", w.tick_n, w.now));
        }
        n += 1;
        if !(w.now < until - w.dt * 0.5) {
            break;
        }
        w.step();
    }
    if lines.next().is_some() {
        return Err("golden trace has more ticks than the port".into());
    }
    Ok(n)
}

#[test]
fn golden_combat_parity() {
    let dir = tests_dir();
    let required = std::env::var("MORDHAU_GOLDEN_REQUIRED").map(|v| v == "1").unwrap_or(false);
    let f32_mode = std::env::var("MORDHAU_GOLDEN_F32").map(|v| v == "1").unwrap_or(false);
    let spec_path = dir.join("golden/spec.json");
    let Ok(spec_txt) = std::fs::read_to_string(&spec_path) else {
        assert!(!required, "golden spec missing: {}", spec_path.display());
        eprintln!("SKIP: no golden data ({}); run godot/tools/export_golden.gd", spec_path.display());
        return;
    };
    let spec = Rc::new(RecordsJson(&spec_txt).load_spec().expect("spec.json"));
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("scenarios/combat.json")).unwrap()).unwrap();
    let aliases = doc["weapons"].clone();
    let alias = move |x: &str| -> String { aliases[x].as_str().unwrap_or(x).to_string() };
    let only = std::env::var("MORDHAU_GOLDEN_ONLY").ok();
    let mut fails = Vec::new();
    let mut total = 0;
    for sc in doc["scenarios"].as_array().unwrap() {
        let name = s(sc, "name");
        if only.as_ref().map(|o| !o.split(',').any(|x| x == name)).unwrap_or(false) {
            continue;
        }
        let gp = dir.join("golden/combat").join(format!("{name}.jsonl"));
        let Ok(golden) = std::fs::read_to_string(&gp) else {
            fails.push(format!("{name}: no golden file {}", gp.display()));
            continue;
        };
        total += 1;
        match run_scenario(&spec, sc, &alias, &golden, f32_mode) {
            Ok(n) => eprintln!("PASS {name}: {n} ticks identical"),
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                fails.push(format!("{name}: {e}"));
            }
        }
    }
    eprintln!("golden combat parity: {}/{} scenarios", total - fails.len(), total);
    assert!(fails.is_empty(), "golden parity failures:\n{}", fails.join("\n"));
}

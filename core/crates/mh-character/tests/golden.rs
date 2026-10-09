//! reference_compat mode. Golden-trace parity: replays tests/scenarios.json exactly as godot/tools/export_golden_character.gd drives the
//! GDScript reference (MordhauCharacter.step_model + a flat floor standing in for move_and_slide), and compares every
//! tick's state with core/tests/golden/character/<scenario>.jsonl bit for bit (floats are f64 bit patterns).
//! Regenerate the traces with
//!   sh scripts/godot_import.sh --run timeout 900 godot --headless --path godot --script res://tools/export_golden_character.gd

use mh_character::records::hex_f64;
use mh_character::ue::{FVector, V3Ext};
use mh_character::compat::{CharacterInput, Mode, MordhauMovement};
use mh_character::{CharacterRecords, CharacterSource, ExtractJson, RecordsJson};
use serde_json::Value;
use std::path::PathBuf;

/// this crate's directory (MH_CHARACTER_DIR overrides it for an out-of-tree manifest)
fn crate_dir() -> PathBuf {
    std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn root() -> PathBuf {
    crate_dir().join("../../..")
}

fn golden_dir() -> PathBuf {
    root().join("core/tests/golden/character")
}

fn read(p: &PathBuf) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nrun: sh scripts/godot_import.sh --run timeout 900 godot --headless --path godot --script \
             res://tools/export_golden_character.gd",
            p.display()
        )
    })
}

fn records() -> CharacterRecords {
    RecordsJson(&read(&golden_dir().join("records.json"))).load().expect("records.json")
}

/// A float as the trace writes it (f64 bits, hex)
fn hb(x: f64) -> Value {
    Value::String(x.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect())
}
fn hv(v: FVector) -> Value {
    Value::Array(vec![hb(v.xf()), hb(v.yf()), hb(v.zf())])
}

fn num(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(d)
}
fn flag(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
}

/// the last input segment covering tick t (export_golden_character.gd seg)
fn seg(s: &Value, t: i64) -> Value {
    let mut out = Value::Null;
    for g in s["input"].as_array().into_iter().flatten() {
        if g["from"].as_f64().unwrap() as i64 <= t && t < g["to"].as_f64().unwrap() as i64 {
            out = g.clone();
        }
    }
    out
}

/// one scenario -> the trace rows, field names and encoding as the exporter writes them
fn run(rec: &CharacterRecords, s: &Value) -> Vec<Value> {
    let dt = 1.0 / num(s, "hz", 60.0);
    let mut m = MordhauMovement::new(rec);
    let mut inp = CharacterInput::default();
    m.toggle_sprint = num(s, "toggle_sprint", 0.0) as i64;
    m.toggle_crouch = num(s, "toggle_crouch", 0.0) as i64;
    inp.yaw = num(s, "yaw", 0.0);
    let mut floor_y = num(s, "floor", 0.0);
    let mut pos = FVector::new(0.0, num(s, "start_y", 0.0) as f32, 0.0);
    if s.get("mode").and_then(|x| x.as_str()) == Some("falling") {
        m.mode = Mode::Falling;
    }
    let mut out = Vec::new();
    for t in 0..num(s, "ticks", 0.0) as i64 {
        for e in s["events"].as_array().into_iter().flatten() {
            if e["tick"].as_f64().unwrap() as i64 != t {
                continue;
            }
            if let Some(v) = e.get("floor") {
                floor_y = v.as_f64().unwrap();
            }
            if let Some(v) = e.get("yaw") {
                inp.yaw = v.as_f64().unwrap();
            }
            if let Some(v) = e.get("restriction") {
                m.motion_restriction = v.as_f64().unwrap() as i64;
            }
            if let Some(v) = e.get("authority") {
                m.authority = v.as_bool().unwrap();
            }
            if let Some(v) = e.get("dead") {
                m.dead = v.as_bool().unwrap();
            }
            if let Some(v) = e.get("toggle_sprint") {
                m.toggle_sprint = v.as_f64().unwrap() as i64;
            }
            if let Some(v) = e.get("toggle_crouch") {
                m.toggle_crouch = v.as_f64().unwrap() as i64;
            }
            if e.get("trip").is_some() {
                m.trip();
            }
            if let Some(k) = e.get("knockback") {
                let c = |i: usize| k[i].as_f64().unwrap() as f32;
                m.knockback(FVector::new(c(0), c(1), c(2)));
            }
        }
        let g = seg(s, t);
        let r = inp.step(&mut m, dt, num(&g, "fwd", 0.0), num(&g, "right", 0.0), flag(&g, "jump"), flag(&g, "sprint"), flag(&g, "crouch"));
        pos = pos + r.delta;
        let on_floor = pos.yf() <= floor_y; // flat-floor stand-in for move_and_slide + is_on_floor
        if on_floor {
            pos.y = floor_y as f32;
        }
        m.set_on_floor(on_floor);
        let row = serde_json::json!({
            "t": t, "wt": hb(m.world_time), "mode": m.mode as i64, "vel": hv(m.velocity), "acc": hv(m.acceleration),
            "pos": hv(pos), "delta": hv(r.delta), "facing": hv(r.facing), "crouch": r.crouch, "on_floor": on_floor,
            "crouched": m.crouched, "wants_crouch": m.wants_crouch, "wants_sprint": m.wants_sprint,
            "sprint_state": m.sprint_state as i64, "sprint_time": hb(m.sprint_time), "max_speed": hb(m.get_max_speed()),
            "last_landed": hb(m.last_landed_time), "last_crouch_toggle": hb(m.last_crouch_toggle_time),
            "knockback_time": hb(m.knockback_time), "ground_friction": hb(m.cfg.ground_friction),
            "falling_lateral_friction": hb(m.cfg.falling_lateral_friction), "pending_impulse": hv(m.pending_impulse),
            "falling_time": hb(m.falling_time), "ragdoll": m.ragdoll_falling, "ragdoll_start": hb(m.ragdoll_falling_start_time),
            "ragdoll_get_up_start": hb(m.ragdoll_falling_get_up_start_time), "still_time": hb(m.still_time_while_ragdoll_falling),
            "lfcvz": hb(m.last_falling_check_velocity_z), "airborne_from_jump": m.b_is_airborne_from_jump,
            "last_land_from_jump": m.b_was_last_land_from_jump, "restriction": m.get_movement_restriction(),
            "input_ignored": m.is_move_input_ignored(), "getting_up": m.is_ragdoll_falling_or_getting_up(),
            "jumps": m.jumps, "fall_damage": m.fall_damage.iter().map(|x| hb(*x)).collect::<Vec<_>>(),
            "ragdoll_changes": m.ragdoll_changes.clone(),
        });
        out.push(row);
        // the owner drains the events each frame (Fighter.apply_movement_events)
        mh_character::compat::apply_movement_events::<dyn mh_character::compat::CombatSide>(&mut m, None);
    }
    out
}

/// JSON numbers from Godot's stringify are floats ("0" vs 0.0 for ints): compare numbers by value, the rest exactly
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        _ => a == b,
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::String(s) => hex_f64(s).map(|f| format!("{f:?}")).unwrap_or_else(|| s.clone()),
        Value::Array(a) => format!("[{}]", a.iter().map(show).collect::<Vec<_>>().join(", ")),
        _ => v.to_string(),
    }
}

/// every scenario against its golden trace: (first divergence per scenario, ticks compared, fields compared)
fn compare(rec: &CharacterRecords) -> (Vec<String>, usize, usize) {
    let spec: Value = serde_json::from_str(&read(&crate_dir().join("tests/scenarios.json"))).unwrap();
    let mut failures = Vec::new();
    let mut ticks = 0;
    let mut fields = 0;
    for s in spec["scenarios"].as_array().unwrap() {
        let name = s["name"].as_str().unwrap();
        let want: Vec<Value> = read(&golden_dir().join(format!("{name}.jsonl")))
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let got = run(rec, s);
        if got.len() != want.len() {
            failures.push(format!("{name}: {} ticks, golden {}", got.len(), want.len()));
            continue;
        }
        'rows: for (g, w) in got.iter().zip(&want) {
            ticks += 1;
            for (k, wv) in w.as_object().unwrap() {
                fields += 1;
                let gv = g.get(k).unwrap_or(&Value::Null);
                if !same(gv, wv) {
                    failures.push(format!("{name} tick {}: {k} = {} (golden {})", w["t"], show(gv), show(wv)));
                    break 'rows; // first divergence per scenario; later ticks follow from it
                }
            }
        }
    }
    (failures, ticks, fields)
}

#[test]
fn golden_parity_every_tick() {
    let (failures, ticks, fields) = compare(&records());
    eprintln!("character golden: {ticks} ticks, {fields} fields compared");
    assert!(failures.is_empty(), "golden mismatches:
{}", failures.join("
"));
    let spec: Value = serde_json::from_str(&read(&crate_dir().join("tests/scenarios.json"))).unwrap();
    let want: f64 = spec["scenarios"].as_array().unwrap().iter().map(|s| s["ticks"].as_f64().unwrap()).sum();
    assert_eq!(ticks, want as usize, "not every scenario tick was compared");
}

/// Tamper check: the comparison is not vacuous - one ulp off in one record value diverges every scenario that reads it.
#[test]
fn golden_parity_detects_one_ulp() {
    let mut rec = records();
    rec.movement.ground_friction = f64::from_bits(rec.movement.ground_friction.to_bits() + 1);
    let (failures, _, _) = compare(&rec);
    assert!(failures.len() >= 8, "a 1-ulp GroundFriction change went unnoticed: {failures:?}");
}

/// ExtractJson (BP_MordhauCharacter.json + DefaultEngine.ini over the native defaults) reads the same values the
/// reference's record layer produced. The native-only terms come from the dump (NativeCtor is not ported); every value
/// the Blueprint or the ini serializes must equal the dump bit for bit.
#[test]
fn extract_json_matches_reference_records() {
    let bp = root().join("extract/json/Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter.json");
    let ini = root().join("extract/config/DefaultEngine.ini");
    let (Ok(bp), Ok(ini)) = (std::fs::read_to_string(&bp), std::fs::read_to_string(&ini)) else {
        eprintln!("SKIP: extract/ not present (the user's own game export)");
        return;
    };
    let rec = records();
    // native layer = the dump with every Blueprint/ini-sourced value zeroed, so the overlay must supply them
    let mut native = rec.clone();
    native.movement = Default::default();
    native.physics = mh_character::records::Physics { gravity_z: 0.0, terminal_velocity: 0.0 };
    native.move_extra.knockback_ground_friction = 0.0;
    native.move_extra.knockback_falling_lateral_friction = 0.0;
    native.move_extra.knockback_duration = 0.0;
    native.move_extra.ragdoll_fall_damage_factor = 0.0;
    native.move_extra.b_can_crouch = false;
    native.character.jump_cooldown = 0.0;
    native.character.falling_time_to_ragdoll = 0.0;
    native.character.jump_stamina_cost = 0.0;
    let got = ExtractJson { bp_json: &bp, engine_ini: Some(&ini), native }.load().expect("ExtractJson");
    assert_eq!(got, rec, "ExtractJson records differ from the reference dump");
}

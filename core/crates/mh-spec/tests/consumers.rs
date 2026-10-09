// Consumers (sheets r8): the spec-side loaders the Rust crates switch to, proven equal to the paths they use today.
//  - character: the spec's movement / move-extra / character / physics / curve records == mh-character's record dump
//    (core/tests/golden/character/records.json, godot/tools/export_golden_character.gd), compared as f32 (the exe's
//    binary32; the dump holds the reference's f64)
//  - mode: Spec::nested(ENT_MODE_*) == the mode records the reference reads (godot/data_gen/spec_src/modes.json data)
//  - the mod layer as an overlay (Spec::apply_overlay) on a tiny matrix
// Data cases are skipped with a message when the local Triternion data is absent.
use mh_spec::Spec;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    std::env::var("MORDHAU_REPO").map(PathBuf::from).unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

fn real() -> Option<Spec> {
    let d = Spec::default_dir();
    if !d.join("index.json").exists() {
        eprintln!("SKIP: no {}", d.display());
        return None;
    }
    Some(Spec::load(&d, false).expect("load data_gen/spec"))
}

fn read(rel: &str) -> Option<Value> {
    let p = repo().join(rel);
    match std::fs::read_to_string(&p) {
        Ok(t) => Some(serde_json::from_str(&t).expect(rel)),
        Err(_) => {
            eprintln!("SKIP: no {}", p.display());
            None
        }
    }
}

/// mh-character's dump encoding: floats as the 16 hex digits of their little-endian f64 bytes
fn hex_f64(s: &str) -> Option<f64> {
    if s.len() != 16 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut b = [0u8; 8];
    for (i, x) in b.iter_mut().enumerate() {
        *x = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(f64::from_le_bytes(b))
}

/// dump value vs spec record value: f32-equal numbers, equal bools / ints / strings, [3 hex] vectors
fn same(rec: &mh_spec::Record, name: &str, d: &Value) -> Result<(), String> {
    let s = rec.value(name).ok_or_else(|| format!("{}.{name}: not in the spec", rec.entity))?;
    let ok = match d {
        Value::String(h) if hex_f64(h).is_some() => s.as_f64().map(|x| x as f32) == hex_f64(h).map(|x| x as f32),
        Value::Array(a) if a.iter().all(|x| x.as_str().and_then(hex_f64).is_some()) => {
            s.as_array().is_some_and(|b| b.len() == a.len() && a.iter().zip(b).all(|(x, y)| {
                y.as_f64().map(|v| v as f32) == x.as_str().and_then(hex_f64).map(|v| v as f32)
            }))
        }
        Value::Number(n) => s.as_f64() == n.as_f64(),
        _ => s == d,
    };
    if ok { Ok(()) } else { Err(format!("{}.{name}: dump {d} vs spec {s}", rec.entity)) }
}

#[test]
fn character_records_from_spec_equal_the_dump() {
    let (Some(s), Some(d)) = (real(), read("core/tests/golden/character/records.json")) else { return };
    let mut n = 0;
    let mut fails = vec![];
    for (sec, ent) in [("movement", "ENT_MOV_MOVEMENT"), ("move_extra", "ENT_MOV_MOVE_EXTRA"), ("character", "ENT_CHR_BP_MordhauCharacter")] {
        let rec = s.record(ent).unwrap();
        for (k, v) in d[sec].as_object().unwrap() {
            if let Err(e) = same(&rec, k, v) { fails.push(e) }
            n += 1;
        }
    }
    let phys = s.record("ENT_PHYS_WORLD").unwrap();
    for k in ["gravity_z", "terminal_velocity"] {
        if let Err(e) = same(&phys, k, &d[k]) { fails.push(e) }
        n += 1;
    }
    for (path, c) in d["curves"].as_object().unwrap() {
        let sc = s.curve(path).unwrap();
        let keys: Vec<(f32, f32, String)> = c["keys"].as_array().unwrap().iter().map(|k| (
            hex_f64(k["time"].as_str().unwrap()).unwrap() as f32, hex_f64(k["value"].as_str().unwrap()).unwrap() as f32,
            k["interp"].as_str().unwrap().to_string())).collect();
        if sc.keys != keys { fails.push(format!("curve {path}: {:?} vs {:?}", sc.keys, keys)) }
        n += 1;
    }
    assert!(fails.is_empty(), "{} of {n} differ: {:?}", fails.len(), &fails[..fails.len().min(8)]);
    eprintln!("character: {n} record values / curves from the spec == mh-character's dump (f32)");
}

#[test]
fn mode_records_from_spec_equal_the_reference() {
    let (Some(s), Some(d)) = (real(), read("godot/data_gen/spec_src/modes.json")) else { return };
    let mut n = 0;
    let mut fails = vec![];
    for (id, m) in d.as_object().unwrap() {
        let v = s.nested(&format!("ENT_MODE_{id}"), &["scoring", "state", "meta", "rules", "table"]).unwrap();
        for g in ["scoring", "state", "meta"] {
            for (k, x) in m["data"][g].as_object().into_iter().flatten() {
                if k.starts_with("__") || x.is_object() || x.is_array() { continue }
                let y = &v[g][k];
                let ok = match (x.as_f64(), y.as_f64()) {
                    (Some(a), Some(b)) => a as f32 == b as f32,
                    _ => x == y,
                };
                if !ok { fails.push(format!("{id}.{g}.{k}: reference {x} vs spec {y}")) }
                n += 1;
            }
        }
    }
    assert!(fails.is_empty(), "{} of {n} differ: {:?}", fails.len(), &fails[..fails.len().min(8)]);
    assert!(n > 50, "only {n} values compared");
    eprintln!("mode: {n} nested record values from the spec == the reference's records (f32)");
}

#[test]
fn overlay_replaces_typed_values_atomically() {
    let index = serde_json::from_value(json!({"format": "mordhau-spec/1", "content_sha1": "x",
        "entity_types": {"weapon": {"file": "entities/weapon.json", "code": "WPN", "entities": 1, "fields": 2, "values": 2}},
        "counts": {}, "evidence_status": {}})).unwrap();
    let f = |n: &str, t: &str| json!({"entity_type": "weapon", "name": n, "type": t, "ref_type": "", "unit": "",
        "unit_source": "", "source": "", "description": ""});
    let fields = json!({"FLD_WPN_POINT_COST": f("point_cost", "int"), "FLD_WPN_SLIDE_RADIUS": f("slide_radius", "float")});
    let weapon = json!({"ENT_WPN_W": {"name": "", "parent": null, "source": "S", "record": "",
        "values": {"FLD_WPN_POINT_COST": 7, "FLD_WPN_SLIDE_RADIUS": 70.0}}});
    let mut s = Spec::from_parts(index, vec![("fields.json".into(), fields.to_string()), ("rules.json".into(), "{}".into()),
        ("sources.json".into(), "{}".into()), ("entities/weapon.json".into(), weapon.to_string())]).unwrap();
    // a bad value anywhere: nothing changes
    assert!(s.apply_overlay(&json!({"ENT_WPN_W": {"FLD_WPN_POINT_COST": 9, "FLD_WPN_SLIDE_RADIUS": "wide"}})).is_err());
    assert_eq!(s.i64("ENT_WPN_W", "FLD_WPN_POINT_COST").unwrap(), 7);
    assert_eq!(s.apply_overlay(&json!({"ENT_WPN_W": {"FLD_WPN_POINT_COST": 9, "FLD_WPN_SLIDE_RADIUS": 0.1}})).unwrap(), 2);
    assert_eq!(s.record("ENT_WPN_W").unwrap().i64("point_cost").unwrap(), 9);
    assert_eq!(s.f32("ENT_WPN_W", "FLD_WPN_SLIDE_RADIUS").unwrap(), 0.1f32);
    assert!(s.apply_overlay(&json!({"ENT_WPN_NOPE": {"FLD_WPN_POINT_COST": 1}})).is_err());
}

//! ModeData::from_spec_class (src/spec_horde.rs) for HRD / BR == the data_hrd_br golden header (hrd_mode + horde,
//! br_mode + br), every leaf the spec provides, numbers as f32. The spec's gaps (requested from the sheets builder)
//! come from the dump fallback and are listed; without a fallback they must be exactly the listed paths.
use mh_mode::data::{ModeData, ModeDataExt};
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn cmp(path: &str, golden: &Value, got: &Value, fails: &mut Vec<String>, n: &mut usize) {
    match golden {
        Value::Object(m) => {
            for (k, v) in m {
                cmp(&format!("{path}.{k}"), v, &got[k.as_str()], fails, n);
            }
        }
        Value::Array(a) => {
            let b = got.as_array().cloned().unwrap_or_default();
            if a.len() != b.len() {
                fails.push(format!("{path}: len {} vs {}", a.len(), b.len()));
                return;
            }
            for (i, (x, y)) in a.iter().zip(&b).enumerate() {
                cmp(&format!("{path}[{i}]"), x, y, fails, n);
            }
        }
        // the dump never exported the legacy Waves row count (its serde default 0; the spec has the real 21 rows,
        // read only when IsSquadSpawningEnabled is false)
        Value::Number(_) if path.ends_with("legacy_wave_count") => {}
        Value::Number(x) => {
            *n += 1;
            if got.as_f64().map(|y| y as f32) != x.as_f64().map(|x| x as f32) {
                fails.push(format!("{path}: golden {x} vs spec {got}"));
            }
        }
        _ => {
            *n += 1;
            if golden != got {
                fails.push(format!("{path}: golden {golden} vs spec {got}"));
            }
        }
    }
}

#[test]
fn hrd_br_from_spec_equal_the_dump() {
    let r = root();
    let spec_dir = r.join("data_gen/spec");
    let Ok(text) = std::fs::read_to_string(r.join("core/tests/golden/mode/data_hrd_br.jsonl")) else {
        return eprintln!("SKIP: no data_hrd_br");
    };
    if !spec_dir.join("index.json").exists() {
        return eprintln!("SKIP: no spec");
    }
    let spec = mh_spec::Spec::load(&spec_dir, false).expect("spec");
    let mut h: Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
    mordhau_core::data::decode_exact(&mut h);
    for (id, mode, ext) in [("HRD", "hrd_mode", "horde"), ("BR", "br_mode", "br")] {
        let mut m = h[mode].clone();
        let mut e = h[ext].clone();
        e["id"] = Value::String(id.into());
        m["ext"] = e;
        let dump: ModeData = serde_json::from_value(m.clone()).unwrap();
        let got = ModeData::from_spec_class(&spec, id).unwrap();
        let (mut fails, mut n) = (vec![], 0);
        cmp(id, &serde_json::to_value(&dump).unwrap(), &serde_json::to_value(&got).unwrap(), &mut fails, &mut n);
        assert!(fails.is_empty(), "{id}: {} of {n} leaves differ: {:?}", fails.len(), &fails[..fails.len().min(10)]);
        eprintln!("{id}: {n} leaves of the dump == the spec (f32)");
        if let ModeDataExt::Horde(hd) = &got.ext {
            assert_eq!((hd.squad_waves.len(), hd.squads.len(), hd.extras.skills.len()), (25, 25, 52));
        }
    }
    // E_HordeSkill: the spec's UEnum::Names (sorted by value) == horde_extras::E_HORDE_SKILL (+ E_MAX)
    let en = mh_mode::spec_horde::skill_enum(&spec).unwrap();
    for (i, n) in mh_mode::horde_extras::E_HORDE_SKILL.iter().enumerate() {
        assert_eq!((en[i].0.trim_start_matches("E_HordeSkill::"), en[i].1), (*n, i as i64));
    }
}

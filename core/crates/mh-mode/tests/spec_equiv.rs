//! ModeData::from_spec (src/spec_load.rs, the spec matrix) == the golden exporter's mode records (the header line 2 of
//! core/tests/golden/mode/<scenario>.jsonl, `data`), every leaf, numbers compared as f32. Skipped (eprintln, pass)
//! when data_gen/spec or the goldens are absent (local Triternion data).
use mh_mode::data::ModeData;
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    std::env::var("MORDHAU_REPO").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

/// every leaf of `golden` must equal `got` (numbers as f32); keys only in `got` are ignored (the spec record
/// carries extra variables, e.g. rules_*/table_*)
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
fn from_spec_equals_the_golden_headers() {
    let r = root();
    let spec_dir = std::env::var("MORDHAU_SPEC_DIR").map(PathBuf::from).unwrap_or_else(|_| r.join("data_gen/spec"));
    if !spec_dir.join("index.json").exists() {
        eprintln!("SKIP: no {}", spec_dir.display());
        return;
    }
    let spec = mh_spec::Spec::load(&spec_dir, false).expect("spec");
    let mut total = 0;
    for (scen, id) in [("ffa_kills", "FFA"), ("tdm_kills", "TDM"), ("skm_rounds", "SKM"), ("duel_rooms", "DU"), ("tf_rooms", "TF"), ("fl_camp", "FL")] {
        let p = r.join(format!("core/tests/golden/mode/{scen}.jsonl"));
        let Ok(text) = std::fs::read_to_string(&p) else {
            eprintln!("SKIP {id}: no {}", p.display());
            continue;
        };
        let mut header: Value = serde_json::from_str(text.lines().nth(1).expect("header line")).expect("header json");
        mordhau_core::data::decode_exact(&mut header);
        // both through ModeData: the dump the way the crate loads it today (serde from the header's data), and the spec
        let golden: ModeData = serde_json::from_value(header["data"].clone()).unwrap_or_else(|e| panic!("{id} dump: {e}"));
        let golden = serde_json::to_value(golden).unwrap();
        let got = serde_json::to_value(ModeData::from_spec(&spec, id).unwrap_or_else(|e| panic!("{id}: {e}"))).unwrap();
        let (mut fails, mut n) = (vec![], 0);
        cmp(id, &golden, &got, &mut fails, &mut n);
        // FL: the golden header's starts are captured after FlMode set up its control points, and
        // AControlPoint::UpdateSpawns rva=0x151b3f0's port (control_point.gd update_spawns) writes bIsSpawnDisabled on
        // the shared start records at match start: runtime state, not data. No FL_Camp package serializes
        // bIsSpawnDisabled (the placed starts keep the class default false), which is the spec's value.
        let runtime = fails.len();
        fails.retain(|f| !(id == "FL" && f.contains(".b_is_spawn_disabled: golden true vs spec false")));
        if runtime != fails.len() {
            eprintln!("{id}: {} start(s) spawn-disabled at match start in the golden header (UpdateSpawns runtime state) skipped", runtime - fails.len());
        }
        let kinds: std::collections::BTreeSet<String> = fails.iter()
            .map(|f| f.split(':').next().unwrap_or("").split('[').map(|s| s.split(']').last().unwrap_or("")).collect::<String>())
            .collect();
        assert!(fails.is_empty(), "{id}: {} of {n} leaves differ in {kinds:?}: {:?}", fails.len(), &fails[..fails.len().min(10)]);
        eprintln!("{id}: {n} leaves of from_spec == the golden header ({scen}, f32)");
        total += n;
    }
    eprintln!("from_spec: {total} leaves equal");
}

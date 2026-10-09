//! mh_mode::ai::bt_read (the Rust port of bot_data.gd tree_def) against the spec's trees (the Godot reader's output):
//! every ENT_BT_* tree read from the packages (extract/json) equals the spec record; the horde trees read. SKIP
//! without extract/json or the spec.
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn load(p: &str) -> Option<Vec<Value>> {
    let t = std::fs::read_to_string(root().join("extract/json").join(format!("{p}.json"))).ok()?;
    serde_json::from_str::<Value>(&t).ok()?.as_array().cloned()
}

#[test]
fn trees_from_packages_equal_the_spec() {
    let d = root().join("data_gen/spec");
    if !d.join("index.json").exists() || load("Mordhau/Content/Mordhau/AI/BehaviorTrees/BT_Deathmatch").is_none() {
        return eprintln!("SKIP: no spec / extract/json");
    }
    let spec = mh_spec::Spec::load(&d, false).unwrap();
    for name in ["BT_Deathmatch", "BT_CombatGeneric", "BT_Frontline", "BT_Push"] {
        let want = mh_mode::spec_bots::tree(&spec, name).unwrap();
        let got = mh_mode::ai::bt_read::tree_def(&load, &format!("Mordhau/Content/Mordhau/AI/BehaviorTrees/{name}")).unwrap();
        let (a, b) = (serde_json::to_value(&want).unwrap(), serde_json::to_value(&got).unwrap());
        let mut diffs = vec![];
        diff("", &a, &b, &mut diffs);
        assert!(diffs.is_empty(), "{name}: {} differences, first: {:#?}", diffs.len(), &diffs[..diffs.len().min(12)]);
        eprintln!("{name}: package read == spec");
    }
    for name in ["BT_Horde", "BT_HordeOgre", "BT_HordeKillObjective"] {
        let t = mh_mode::ai::bt_read::tree_def(&load, &format!("Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/AI/{name}")).unwrap();
        eprintln!("{name}: {} blackboard keys, root {}", t.blackboard.len(), t.root.type_);
        assert!(!t.root.children.is_empty());
    }
}

fn diff(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                diff(&format!("{path}.{k}"), x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null), out);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                diff(&format!("{path}[{i}]"), p, q, out);
            }
        }
        _ if a != b => out.push(format!("{path}: spec {a} vs read {b}")),
        _ => {}
    }
}

//! spec_bots (exe-mode data path) == the golden dumps: every bot profile / tree in the bots_* headers, and the Kismet
//! literals == godot/data_gen/mode/mode_kismet.json. Numbers as f32; SKIP without the spec / goldens.
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
fn bots_and_kismet_from_spec_equal_the_dumps() {
    let r = root();
    let dir = r.join("data_gen/spec");
    if !dir.join("index.json").exists() {
        return eprintln!("SKIP: no spec");
    }
    let spec = mh_spec::Spec::load(&dir, false).expect("spec");
    let (mut fails, mut n, mut checked) = (vec![], 0, 0);
    let scen: Value = serde_json::from_str(&std::fs::read_to_string(r.join("core/tests/scenarios/mode.json")).unwrap()).unwrap();
    for f in std::fs::read_dir(r.join("core/tests/golden/mode")).into_iter().flatten().flatten() {
        let name = f.file_name().to_string_lossy().to_string();
        if !name.starts_with("bots_") {
            continue;
        }
        let text = std::fs::read_to_string(f.path()).unwrap();
        let mut h: Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
        mordhau_core::data::decode_exact(&mut h);
        let sc = scen["scenarios"].as_array().unwrap().iter().find(|s| format!("{}.jsonl", s["name"].as_str().unwrap()) == name).cloned().unwrap_or_default();
        for (i, b) in h["bots"].as_array().unwrap().iter().enumerate() {
            // the scenario's fighter behavior (golden/mode.gd default BOTBEHAVIOR_Knight)
            let pname = sc["fighters"][i]["behavior"].as_str().unwrap_or("BOTBEHAVIOR_Knight").to_string();
            let tname = b["tree"]["asset"].as_str().unwrap_or("").rsplit('/').next().unwrap().to_string();
            let dp: mh_mode::ai::Profile = serde_json::from_value(b["profile"].clone()).unwrap();
            let dt: mh_mode::ai::TreeDef = serde_json::from_value(b["tree"].clone()).unwrap();
            let sp = mh_mode::spec_bots::profile(&spec, &pname).unwrap_or_else(|e| panic!("{name}: {e}"));
            let st = mh_mode::spec_bots::tree(&spec, &tname).unwrap_or_else(|e| panic!("{name}: {e}"));
            cmp(&format!("{name}.{pname}"), &serde_json::to_value(&dp).unwrap(), &serde_json::to_value(&sp).unwrap(), &mut fails, &mut n);
            cmp(&format!("{name}.{tname}"), &serde_json::to_value(&dt).unwrap(), &serde_json::to_value(&st).unwrap(), &mut fails, &mut n);
            checked += 1;
        }
    }
    let k = std::fs::read_to_string(r.join("godot/data_gen/mode/mode_kismet.json")).ok();
    if let Some(k) = k {
        let a: Value = serde_json::from_str(&k).unwrap();
        let b = mh_mode::spec_bots::kismet(&spec).unwrap();
        for (key, e) in a.as_object().unwrap() {
            cmp(&format!("kismet.{key}"), &e["value"], &b.value(key).cloned().unwrap_or(Value::Null), &mut fails, &mut n);
        }
    }
    assert!(fails.is_empty(), "{} of {n} leaves differ: {:?}", fails.len(), &fails[..fails.len().min(10)]);
    eprintln!("bots: {checked} profile+tree pairs and the kismet literals, {n} leaves == the spec (f32)");
}

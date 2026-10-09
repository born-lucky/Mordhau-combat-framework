//! mh_mode::rule_ids::RULES covers exactly the spec's RULE_HRD_* / RULE_BR_* rules (data_gen/spec/rules.json; SKIP
//! without it), with a known status, and every `path::fn` it names exists in this crate's source.
use std::collections::BTreeSet;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn rule_table_names_real_functions() {
    let mut seen = BTreeSet::new();
    for (id, status, imp, test, _) in mh_mode::rule_ids::RULES {
        assert!(seen.insert(*id), "{id} listed twice");
        assert!(["ported", "host", "not_ported"].contains(status), "{id}: status {status}");
        assert!(*status != "ported" || !imp.is_empty(), "{id}: ported without implemented_by");
        for part in imp.split(';').filter(|s| !s.is_empty()) {
            let (path, f) = part.split_once("::").unwrap_or((part, ""));
            let src = std::fs::read_to_string(root().join(path)).unwrap_or_else(|_| panic!("{id}: {path} missing"));
            assert!(f.is_empty() || src.contains(&format!("fn {f}(")), "{id}: fn {f} not in {path}");
        }
        assert!(test.is_empty() || root().join(test).exists(), "{id}: {test} missing");
    }
}

#[test]
fn rule_table_equals_the_spec_ids() {
    let p = root().join("../../../data_gen/spec/rules.json");
    let Ok(t) = std::fs::read_to_string(&p) else { return eprintln!("SKIP: no {}", p.display()) };
    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
    let rules = v.get("rules").unwrap_or(&v).as_object().unwrap();
    let spec: BTreeSet<&str> = rules.keys().map(|k| k.as_str()).filter(|k| k.starts_with("RULE_HRD_") || k.starts_with("RULE_BR_")).collect();
    let ours: BTreeSet<&str> = mh_mode::rule_ids::RULES.iter().map(|r| r.0).collect();
    assert_eq!(spec, ours, "rule_ids.rs vs the spec's RULE_HRD_* / RULE_BR_*");
    let ported = mh_mode::rule_ids::RULES.iter().filter(|r| r.1 == "ported").count();
    eprintln!("{} HRD / BR rules: {ported} ported, {} host, {} not ported", ours.len(),
        mh_mode::rule_ids::RULES.iter().filter(|r| r.1 == "host").count(),
        mh_mode::rule_ids::RULES.iter().filter(|r| r.1 == "not_ported").count());
}

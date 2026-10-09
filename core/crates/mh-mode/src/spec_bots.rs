//! The bot records and the Blueprint literals from the spec matrix (rust-mode-ai r4, exe-mode data path): `ENT_BOT_*`
//! (UBotBehaviorProfile class defaults, BotData.Profile), `ENT_BT_*` (behavior trees, BotData.TreeDef) and
//! `ENT_CONST_KISMET_*` (mode_kismet.json literals). The spec's field names are the reference's record names, so the
//! entities deserialize as they are (Spec::nested with no groups). tests/spec_equiv_bots.rs: == the golden dumps.

use crate::ai::{Profile, TreeDef};
use crate::kismet::Kismet;
use mh_spec::Spec;
use serde_json::{json, Map, Value};

fn err(e: impl std::fmt::Display) -> String {
    format!("spec: {e}")
}

/// the record of an entity with the spec's bookkeeping keys (`__class`, `__types`) dropped, recursively
fn clean(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, x)| (k, clean(x))).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(clean).collect()),
        x => x,
    }
}

/// a UBotBehaviorProfile by asset name ("BOTBEHAVIOR_Knight")
pub fn profile(spec: &Spec, name: &str) -> Result<Profile, String> {
    let v = clean(spec.nested(&format!("ENT_BOT_{name}"), &[]).map_err(err)?);
    serde_json::from_value(v).map_err(|e| format!("Profile {name} from spec: {e}"))
}

/// a behavior tree by asset name ("BT_Deathmatch")
pub fn tree(spec: &Spec, name: &str) -> Result<TreeDef, String> {
    let mut v = clean(spec.nested(&format!("ENT_BT_{name}"), &[]).map_err(err)?);
    if v.get("asset").is_none() {
        // the asset = the entity's source package (sources.json)
        let e = spec.entity(&format!("ENT_BT_{name}")).map_err(err)?;
        let path = spec.sources.get(&e.source).map(|s| s.path.clone()).unwrap_or_else(|| e.name.clone());
        v["asset"] = json!(path);
    }
    serde_json::from_value(v).map_err(|e| format!("TreeDef {name} from spec: {e}"))
}

/// the mode Blueprints' literals (mode_kismet.json) from the KISMET constants
pub fn kismet(spec: &Spec) -> Result<Kismet, String> {
    let mut out = Map::new();
    for id in spec.entities_of("constant") {
        let Some(key) = id.strip_prefix("ENT_CONST_KISMET_") else { continue };
        let r = spec.record(id).map_err(err)?;
        let value = r.json("literal").map_err(err)?.clone();
        out.insert(key.to_string(), json!({"value": value, "src": r.str("bp_src").unwrap_or("")}));
    }
    Kismet::from_json(&Value::Object(out).to_string())
}

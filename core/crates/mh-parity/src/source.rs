//! Where the parity runs get their records (docs: state/log.md rust-parity r2; scripts/parity/rust_records.py).
//!
//!   exe (default): the spec matrix (data_gen/spec, mh-spec; every UE float its binary32) through mh-sim's SpecBuilder
//!                  (+ the paks for curves and sockets), plus, on modded servers, the NoChamber mod as a layer.
//!   compat:        the GDScript record dump (data_gen/parity/records_<server>.json, export_golden.gd --spec-only):
//!                  the reference's f64 numbers. The matrix keeps each float as its f32 shortest decimal, which equals
//!                  the reference's f64 for package values (the f64 parse of CUE4Parse's shortest f32 decimal) but not
//!                  for native-constructor floats (the reference holds the f64 of the raw f32 bits, 0.33000001311302185
//!                  vs 0.33; docs/SPEC_SHEETS.md "Numbers"), and the matrix does not say which a value is. So compat,
//!                  whose job is to reproduce the reference bit for bit, reads the dump; exe reads the matrix.
//!
//! The mod layer (data_gen/parity/mod_layer_nochamber.json, rust_records.py): the records of the NoChamber server mod
//! loaded the way the parity runner loads it (tests/parity/replay.gd load_mod: the mod's package JSON, plus its CDOs
//! merged over the vanilla classes the port reads from fixed paths: the character and the Feinted / Blocked / Flinch
//! motions, compare.py OVERLAY) and the list of keys the mod adds or changes. Applied over the matrix build: `replace`
//! keys win; every other key keeps the matrix value; keys only the mod tree has (the mod's own weapons' motions,
//! profiles, curves) are added. Layer numbers are rounded to f32 (RecordsJsonExe), as the exe holds them.

use mordhau_core::data::{decode_exact, load_records, round_f32, Spec};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, String>;

fn read(p: &Path) -> Result<String> {
    std::fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()))
}

/// The modded records of a layer, as the exe holds them (f32)
fn layer_spec(layer: &Value) -> Result<Spec> {
    let mut t = layer["tree"].clone();
    decode_exact(&mut t);
    round_f32(&mut t);
    load_records(t)
}

fn replaced_keys(layer: Option<&Value>, sec: &str) -> HashSet<String> {
    layer
        .and_then(|l| l["replace"][sec].as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// The matrix build for `weapons` (left items included) + the optional mod layer
pub fn matrix_spec(weapons: &[String], layer_path: Option<&Path>) -> Result<Spec> {
    let matrix = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).map_err(|e| format!("spec matrix: {e:?}"))?;
    let vfs = Arc::new(mh_pak::Vfs::mount_default().map_err(|e| format!("paks: {e:?}"))?);
    let rd = mh_pak::Reader::new(vfs);
    let layer: Option<Value> = match layer_path {
        Some(p) => Some(serde_json::from_str(&read(p)?).map_err(|e| format!("parse {}: {e}", p.display()))?),
        None => None,
    };
    let replaced = |sec: &str| -> HashSet<String> { replaced_keys(layer.as_ref(), sec) };
    let lw = replaced("weapons");
    let from_matrix: Vec<&str> = weapons.iter().filter(|w| !lw.contains(*w)).map(|s| s.as_str()).collect();
    let mut m = mh_sim::spec::SpecBuilder::new(&matrix, &rd).build(&from_matrix)?;
    let Some(layer) = layer.as_ref() else { return Ok(m) };
    let l = layer_spec(layer)?;
    for k in replaced("weapons") {
        if let Some(v) = l.weapons.get(&k) {
            m.weapons.insert(k, v.clone());
        }
    }
    for k in replaced("motion_defs") {
        if let Some(v) = l.motion_defs.get(&k) {
            m.motion_defs.insert(k, v.clone());
        }
    }
    for k in replaced("profiles") {
        if let Some(v) = l.profiles.get(&k) {
            m.profiles.insert(k, v.clone());
        }
    }
    for k in replaced("curves") {
        if let Some(v) = l.curves.get(&k) {
            m.curves.insert(k, v.clone());
        }
    }
    for k in replaced("class_chains") {
        if let Some(v) = l.class_chains.get(&k) {
            m.class_chains.insert(k, v.clone());
        }
    }
    for k in replaced("stats") {
        if let Some(v) = l.stats.get(&k) {
            m.stats.insert(k, v.clone());
        }
    }
    for k in replaced("single") {
        match k.as_str() {
            "character" => m.character = l.character.clone(),
            "kick_weapon_path" => m.kick_weapon_path = l.kick_weapon_path.clone(),
            "constants" => m.constants = l.constants.clone(),
            "block_collider" => m.block_collider = l.block_collider.clone(),
            x => return Err(format!("mod layer: unknown single section {x}")),
        }
    }
    // keys only the mod tree has (or the matrix build did not need): added, never overriding a matrix value
    for (k, v) in &l.weapons {
        m.weapons.entry(k.clone()).or_insert_with(|| v.clone());
    }
    for (k, v) in &l.motion_defs {
        m.motion_defs.entry(k.clone()).or_insert_with(|| v.clone());
    }
    for (k, v) in &l.profiles {
        m.profiles.entry(k.clone()).or_insert_with(|| v.clone());
    }
    for (k, v) in &l.curves {
        m.curves.entry(k.clone()).or_insert_with(|| v.clone());
    }
    for (k, v) in &l.class_chains {
        m.class_chains.entry(k.clone()).or_insert_with(|| v.clone());
    }
    Ok(m)
}

/// The GDScript record dump (compat: f64 as the reference; exe: rounded to f32)
pub fn records_spec(path: &Path, exe: bool) -> Result<Spec> {
    let t = read(path)?;
    let mut v: Value = serde_json::from_str(&t).map_err(|e| format!("parse {}: {e}", path.display()))?;
    decode_exact(&mut v);
    if exe {
        round_f32(&mut v);
    }
    load_records(v)
}

/// Every weapon path a probe / replay input uses ("<weapon>|<left item>" split), plus the Longsword the probes use
pub fn weapons_of(cmd: &str, doc: &Value) -> Vec<String> {
    let mut ws: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        for p in s.split('|').filter(|x| !x.is_empty()) {
            if !ws.iter().any(|w| w == p) {
                ws.push(p.to_string());
            }
        }
    };
    if cmd == "probe" {
        for w in doc["weapons"].as_array().into_iter().flatten() {
            let w = w.as_str().unwrap_or("");
            add(w);
            for p in doc["parriers"][w].as_array().into_iter().flatten() {
                add(p.as_str().unwrap_or(""));
            }
        }
    } else {
        for sc in doc["scenarios"].as_array().into_iter().flatten() {
            for v in sc["fighters"].as_object().into_iter().flatten().map(|(_, v)| v) {
                add(v.as_str().unwrap_or(""));
            }
        }
    }
    add(crate::LS);
    ws
}

/// The mod layer's character-script switches (modhooks.rs; mod_overlay.py writes "mod_hooks" per server)
pub fn layer_hooks(layer_path: Option<&Path>) -> Result<crate::modhooks::ModHooks> {
    let Some(p) = layer_path else { return Ok(Default::default()) };
    let v: Value = serde_json::from_str(&read(p)?).map_err(|e| format!("parse {}: {e}", p.display()))?;
    Ok(crate::modhooks::ModHooks::from_layer(&v))
}

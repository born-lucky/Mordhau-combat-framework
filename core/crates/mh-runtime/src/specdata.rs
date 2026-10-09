//! specdata.rs - the spec matrix (mh-spec, data_gen/spec: the Spreadsheet Method workbook generated from the exe, the
//! Blueprints and the configs, docs/SPEC_SHEETS.md) as a Bevy resource, with small typed getters. Every value the
//! runtime reads through it is cited by its field id (FLD_*); a missing value falls back to the stated default and is
//! recorded in `missing` (evidence: dump_state "spec_missing").

use bevy::prelude::Resource;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Resource, Clone)]
pub struct SpecData {
    pub m: Arc<mh_spec::Spec>,
    pub missing: Arc<Mutex<BTreeSet<String>>>,
}

#[allow(dead_code)]
impl SpecData {
    pub fn load() -> Result<SpecData, String> {
        let m = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).map_err(|e| format!("{e:?}"))?;
        Ok(SpecData { m: Arc::new(m), missing: Default::default() })
    }

    pub fn value(&self, ent: &str, fld: &str) -> Option<&Value> {
        let v = self.m.entities.get(ent).and_then(|e| e.values.get(fld));
        if v.is_none() {
            self.missing.lock().unwrap().insert(format!("{ent}.{fld}"));
        }
        v
    }
    pub fn f(&self, ent: &str, fld: &str, d: f64) -> f64 {
        self.value(ent, fld).and_then(|v| v.as_f64()).unwrap_or(d)
    }
    pub fn b(&self, ent: &str, fld: &str, d: bool) -> bool {
        self.value(ent, fld).and_then(|v| v.as_bool()).unwrap_or(d)
    }
    pub fn s(&self, ent: &str, fld: &str) -> String {
        self.value(ent, fld).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }
    pub fn v2(&self, ent: &str, fld: &str, d: [f64; 2]) -> [f64; 2] {
        self.value(ent, fld)
            .and_then(|v| v.as_array())
            .map(|a| [a.first().and_then(|x| x.as_f64()).unwrap_or(d[0]), a.get(1).and_then(|x| x.as_f64()).unwrap_or(d[1])])
            .unwrap_or(d)
    }
    pub fn v3(&self, ent: &str, fld: &str, d: [f64; 3]) -> [f64; 3] {
        self.value(ent, fld)
            .and_then(|v| v.as_array())
            .map(|a| [0, 1, 2].map(|i| a.get(i).and_then(|x| x.as_f64()).unwrap_or(d[i])))
            .unwrap_or(d)
    }
    /// ENT_CONST_<name> FLD_CONST_VALUE (exe .rdata / ctor constants)
    pub fn constant(&self, name: &str, d: f64) -> f64 {
        self.f(&format!("ENT_CONST_{name}"), "FLD_CONST_VALUE", d)
    }
    /// Entity id of a Blueprint path in a table ("ENT_MOT_" + file name)
    pub fn ent_of(prefix: &str, path: &str) -> String {
        format!("{prefix}{}", path.rsplit('/').next().unwrap_or(path).split('.').next().unwrap_or(""))
    }
    pub fn missing_list(&self) -> Vec<String> {
        self.missing.lock().unwrap().iter().cloned().collect()
    }
}

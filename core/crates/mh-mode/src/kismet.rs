//! ModeKismet (godot/components/ue/records/mode_kismet.gd): the Blueprint bytecode literals the mode / HUD / bot task
//! ports use (timers, stage numbers, texts, colours), from data_gen/mode/mode_kismet.json, which scripts/mode_kismet.py
//! writes from the cooked packages (function bodies decoded with scripts/kismet). Every entry is {value, src}, src =
//! "<package>:<function>@<in-memory statement index>". A missing key is an error, never a default: the getters panic,
//! as the reference's push_error + null would break the rule using it.
//!
//! No I/O here: the host reads the file and hands its text to `Kismet::from_json`.

use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct Kismet {
    map: BTreeMap<String, (Value, String)>,
}

impl Kismet {
    /// parse mode_kismet.json text ({key: {value, src}})
    pub fn from_json(text: &str) -> Result<Kismet, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("mode_kismet.json: {e}"))?;
        let o = v.as_object().ok_or("mode_kismet.json: not an object")?;
        let mut map = BTreeMap::new();
        for (k, e) in o {
            let val = e.get("value").cloned().ok_or_else(|| format!("mode_kismet.json: {k} has no value"))?;
            let src = e.get("src").and_then(|s| s.as_str()).unwrap_or("?").to_string();
            map.insert(k.clone(), (val, src));
        }
        Ok(Kismet { map })
    }

    /// the raw literal of `key`
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.map.get(key).map(|v| &v.0)
    }

    pub fn ok(&self) -> bool {
        !self.map.is_empty()
    }

    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// the raw value (ModeKismet.kv); panics on a missing key
    pub fn kv(&self, key: &str) -> &Value {
        match self.map.get(key) {
            Some((v, _)) => v,
            None => panic!("ModeKismet: no bytecode constant {key} in mode_kismet.json"),
        }
    }

    /// ModeKismet.src: where the literal was read
    pub fn src(&self, key: &str) -> &str {
        self.map.get(key).map(|e| e.1.as_str()).unwrap_or("?")
    }

    /// GDScript `float(kv(key))`
    pub fn f(&self, key: &str) -> f64 {
        num(self.kv(key), key)
    }

    /// GDScript `int(kv(key))` (a JSON float truncates toward zero)
    pub fn i(&self, key: &str) -> i64 {
        let v = self.kv(key);
        if let Some(i) = v.as_i64() {
            return i;
        }
        num(v, key) as i64
    }

    /// GDScript `bool(kv(key))`
    pub fn b(&self, key: &str) -> bool {
        let v = self.kv(key);
        match v {
            Value::Bool(b) => *b,
            _ => num(v, key) != 0.0,
        }
    }

    /// GDScript `String(kv(key))`
    pub fn s(&self, key: &str) -> String {
        match self.kv(key) {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        }
    }

    /// an array literal (e.g. skm_alive_counts_args [only_living, only_valid_profiles])
    pub fn arr(&self, key: &str) -> &Vec<Value> {
        match self.kv(key) {
            Value::Array(a) => a,
            _ => panic!("ModeKismet: {key} is not an array"),
        }
    }
}

fn num(v: &Value, key: &str) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::Bool(b) => *b as i64 as f64,
        _ => panic!("ModeKismet: {key} is not a number"),
    }
}

/// GDScript `bool(v)` / `float(v)` / `int(v)` of an array element
pub fn vb(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        _ => false,
    }
}
pub fn vf(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::Bool(b) => *b as i64 as f64,
        _ => 0.0,
    }
}
pub fn vs(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

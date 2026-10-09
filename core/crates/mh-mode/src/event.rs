//! The event records the mode / control point / bot ports hand to their host (the reference's `{kind, t, ...}`
//! dictionaries: `_ev(kind, extra)` in mordhau_game_mode.gd and control_point.gd). Plain data: a kind and named
//! arguments, so a host matches on the same kind strings the GDScript adapters do (spawn_pawn, possess, kill_notify,
//! match_end_info, round_info, announce, ...). `to_json` gives the shape the golden traces compare.

use serde_json::{Map, Number, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    I(i64),
    F(f64),
    S(String),
    B(bool),
    /// a Godot Vector3 (f32 components)
    V([f32; 3]),
    /// a nested dictionary (round_info / stage payloads)
    M(Vec<(&'static str, Arg)>),
}

impl Arg {
    pub fn to_json(&self) -> Value {
        match self {
            Arg::I(i) => Value::Number((*i).into()),
            Arg::F(f) => Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null),
            Arg::S(s) => Value::String(s.clone()),
            Arg::B(b) => Value::Bool(*b),
            Arg::V(v) => Value::Array(v.iter().map(|x| Number::from_f64(*x as f64).map(Value::Number).unwrap_or(Value::Null)).collect()),
            Arg::M(m) => {
                let mut o = Map::new();
                for (k, a) in m {
                    o.insert((*k).to_string(), a.to_json());
                }
                Value::Object(o)
            }
        }
    }
    pub fn as_i(&self) -> i64 {
        match self {
            Arg::I(i) => *i,
            Arg::F(f) => *f as i64,
            Arg::B(b) => *b as i64,
            _ => 0,
        }
    }
    pub fn as_f(&self) -> f64 {
        match self {
            Arg::I(i) => *i as f64,
            Arg::F(f) => *f,
            Arg::B(b) => *b as i64 as f64,
            _ => 0.0,
        }
    }
    pub fn as_s(&self) -> &str {
        match self {
            Arg::S(s) => s,
            _ => "",
        }
    }
}

impl From<i64> for Arg {
    fn from(v: i64) -> Self {
        Arg::I(v)
    }
}
impl From<f64> for Arg {
    fn from(v: f64) -> Self {
        Arg::F(v)
    }
}
impl From<bool> for Arg {
    fn from(v: bool) -> Self {
        Arg::B(v)
    }
}
impl From<String> for Arg {
    fn from(v: String) -> Self {
        Arg::S(v)
    }
}
impl From<&str> for Arg {
    fn from(v: &str) -> Self {
        Arg::S(v.to_string())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ev {
    pub kind: &'static str,
    pub args: Vec<(&'static str, Arg)>,
}

impl Ev {
    pub fn new(kind: &'static str) -> Ev {
        Ev { kind, args: Vec::new() }
    }
    pub fn with(mut self, k: &'static str, v: impl Into<Arg>) -> Ev {
        self.args.push((k, v.into()));
        self
    }
    pub fn get(&self, k: &str) -> Option<&Arg> {
        self.args.iter().find(|(n, _)| *n == k).map(|(_, a)| a)
    }
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("kind".into(), Value::String(self.kind.to_string()));
        for (k, a) in &self.args {
            o.insert((*k).to_string(), a.to_json());
        }
        Value::Object(o)
    }
}

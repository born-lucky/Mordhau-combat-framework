//! Property access over an actor: its instance properties, then its class defaults (the Blueprint CDO values merged
//! over the chain, mh-level `GameplayClass::defaults`). A property in neither is the Blueprint variable's zero value
//! (a CDO serializes only values that differ from the property's zero; UE tagged-property delta serialization).

use serde_json::{Map, Value};
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct Props {
    pub inst: Map<String, Value>,
    pub defaults: Rc<Map<String, Value>>,
}

impl Props {
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.inst.get(k).or_else(|| self.defaults.get(k))
    }
    pub fn f(&self, k: &str) -> f64 {
        self.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }
    pub fn b(&self, k: &str) -> bool {
        self.get(k).and_then(|v| v.as_bool()).unwrap_or(false)
    }
    /// byte / int / enum ("EType::NewEnumeratorN" -> N, "EType::Name" -> the given table index)
    pub fn i(&self, k: &str) -> i64 {
        match self.get(k) {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
            Some(Value::Bool(b)) => *b as i64,
            Some(Value::String(s)) => s.rsplit("NewEnumerator").next().and_then(|t| t.parse().ok()).unwrap_or(0),
            _ => 0,
        }
    }
    pub fn arr(&self, k: &str) -> Vec<Value> {
        self.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default()
    }
    /// an object reference's "pkg.N" path
    pub fn obj(&self, k: &str) -> String {
        self.get(k).and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }
}

/// UKismetMathLibrary::FInterpTo_Constant: step towards the target at a constant speed, clamped
pub fn finterp_to_constant(current: f64, target: f64, dt: f64, speed: f64) -> f64 {
    let dist = target - current;
    if dist * dist < 1e-8 {
        return target;
    }
    let step = speed * dt;
    current + dist.clamp(-step, step)
}

/// UKismetMathLibrary::NearlyEqual_FloatFloat
pub fn nearly_equal(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

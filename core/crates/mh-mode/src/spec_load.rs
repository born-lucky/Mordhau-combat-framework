//! `ModeData::from_spec`: the mode records from the Spreadsheet Method spec matrix (data_gen/spec, mh-spec;
//! docs/SPEC_SHEETS.md "Consumers"), the exe-mode data path. Written by the sheets builder (r9) at rust-mode-ai's
//! request; the reference_compat goldens keep reading the exporter dumps.
//!
//! The spec's mode entity (ENT_MODE_<id>) holds every variable of the reference's mode data object (MordhauModeData +
//! the mode's inner data class, godot/game/mode/*_mode.gd) under the same names, nested records flattened as
//! `scoring_*` / `state_*` / `meta_*`. `Spec::nested` restores that shape, so the record maps onto `ModeData` by serde
//! with three adaptations, each what godot/tools/golden/mode.gd does for the dump:
//!  - `starts[].origin` = the placed start's transform origin (`xf.origin`, Godot metres, f32);
//!  - `ext` = the same object tagged with the mode id (the inner data class's variables sit at the top level);
//!  - FL control points: `capture_speed` / `neutralize_speed` = the UCurveFloat sampled at the integer leads 0..64
//!    (UCurveFloat::GetFloatValue rva=0x3026f70 -> FRichCurve::Eval rva=0x3024860, `mordhau_core::data::Spec::curve_eval`,
//!    the one port of it) from the spec's `curve` entities; no curve -> empty.

use crate::data::ModeData;
use mh_spec::Spec;
use serde_json::{json, Value};

/// the capture-curve samples per control point (godot/tools/golden/mode.gd CURVE_SAMPLES)
pub const CURVE_SAMPLES: usize = 65;

fn err(e: impl std::fmt::Display) -> String {
    format!("spec: {e}")
}

/// UCurveFloat::GetFloatValue at 0, 1, .., CURVE_SAMPLES - 1 for a curve package ("" -> no samples)
fn sample_curve(spec: &Spec, core: &mordhau_core::data::Spec, path: &str) -> Result<Vec<f64>, String> {
    if path.is_empty() {
        return Ok(vec![]);
    }
    let id = spec.entity_id("curve", path.rsplit('/').next().unwrap_or(path)).map_err(err)?;
    let r = spec.record(&id).map_err(err)?;
    let keys: Vec<mordhau_core::data::CurveKey> = serde_json::from_value(r.json("keys").map_err(err)?.clone()).map_err(err)?;
    let (pre, post) = (r.str("pre_extrap").map_err(err)?, r.str("post_extrap").map_err(err)?);
    Ok((0..CURVE_SAMPLES).map(|i| core.curve_eval(&keys, i as f64, pre, post)).collect())
}

impl ModeData {
    /// The mode records of mode `id` from the spec matrix.
    /// HRD (Horde: ENT_MODE_HRD + ENT_HRDD_HORDE) goes through `from_spec_class` (spec_horde.rs), so every mode class
    /// of the spec (FFA / TDM / SKM / DU / TF / FL / HRD / BR) loads by this one entry point.
    pub fn from_spec(spec: &Spec, id: &str) -> Result<ModeData, String> {
        if id == "HRD" {
            return crate::spec_horde::hrd_from_spec(spec);
        }
        let mut v = spec.nested(&format!("ENT_MODE_{id}"), &["scoring", "state", "meta", "rules", "table"]).map_err(err)?;
        let o = v.as_object_mut().ok_or("spec: mode record is not an object")?;
        // placed starts: origin = xf.origin (Godot metres) as f32
        if let Some(Value::Array(starts)) = o.get_mut("starts") {
            for s in starts.iter_mut() {
                let org = s["xf"]["origin"].clone();
                let f: Vec<Value> = org.as_array().into_iter().flatten()
                    .map(|x| json!(x.as_f64().unwrap_or(0.0) as f32 as f64)).collect();
                s["origin"] = Value::Array(f);
            }
        }
        // FL: control points with their sampled speed curves (curve_eval needs the two .rdata literals it reads)
        if let Some(Value::Array(cps)) = o.get_mut("control_points") {
            let mut core = mordhau_core::data::Spec::default();
            core.constants.small_number = spec.f32("ENT_CONST_COMBAT_small_number", "FLD_CONST_VALUE").map_err(err)? as f64;
            core.constants.curve_third = spec.f32("ENT_CONST_ENGINE_curve_third", "FLD_CONST_VALUE").map_err(err)? as f64;
            for cp in cps.iter_mut() {
                for (curve, out) in [("capture_speed_curve", "capture_speed"), ("neutralize_speed_curve", "neutralize_speed")] {
                    let p = cp[curve].as_str().unwrap_or("").to_string();
                    cp[out] = json!(sample_curve(spec, &core, &p)?);
                }
            }
        }
        let mut ext = Value::Object(o.clone());
        ext["id"] = json!(id);
        o.insert("ext".to_string(), ext);
        serde_json::from_value(v).map_err(|e| format!("ModeData::from_spec({id}): {e}"))
    }
}

//! `SpecCharacter`: the character records loaded from the Spreadsheet Method spec matrix (data_gen/spec, mh-spec;
//! docs/SPEC_SHEETS.md "Consumers"), the exe-mode data path. Written by the sheets builder (r8) at rust-character's
//! request; feature `spec` (mh-character does not pull mh-spec in by default).
//!
//! The spec holds every record field under the same snake_case name as the `record!` structs: Movement on
//! ENT_MOV_MOVEMENT, MoveExtra on ENT_MOV_MOVE_EXTRA, Character on ENT_CHR_BP_MordhauCharacter, the DefaultEngine.ini
//! physics on ENT_PHYS_WORLD and the UCurveFloat keys as `curve` entities (all from MordhauMovement.load_default via
//! godot/tools/gen_spec_src.gd). Rather than list the fields a second time, this source renders the spec into the
//! record dump's own format (floats as the hex of their little-endian f64 bytes) and hands it to `RecordsJson`, so the
//! field lists and their checks stay in records.rs alone. Every float is the spec's binary32 widened to f64 (the
//! exe's value; the reference dump holds the f64 of the package decimal, which differs for constructor values).

use crate::records::{CharacterRecords, CharacterSource, RecordsJson};
use mh_spec::{FieldType, Spec};
use serde_json::{json, Map, Value};

/// The spec matrix as a `CharacterSource`.
pub struct SpecCharacter<'a>(pub &'a Spec);

/// f64 -> the dump's 16 hex digits of its little-endian bytes
fn hex(x: f64) -> Value {
    Value::String(x.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect())
}

/// every value of one entity, in the dump encoding, keyed by the field's record name
fn section(spec: &Spec, entity: &str) -> Result<Value, String> {
    let e = spec.entity(entity).map_err(|e| format!("spec: {e}"))?;
    let mut out = Map::new();
    for (fid, v) in &e.values {
        let f = spec.field(fid).map_err(|e| format!("spec: {e}"))?;
        let enc = match f.ty {
            FieldType::Float => hex(spec.f32(entity, fid).map_err(|e| format!("spec: {e}"))? as f64),
            FieldType::Double => hex(spec.f64(entity, fid).map_err(|e| format!("spec: {e}"))?),
            FieldType::Vec2 | FieldType::Vec3 | FieldType::FloatList => Value::Array(
                v.as_array().into_iter().flatten().map(|x| hex(x.as_f64().unwrap_or(f64::NAN) as f32 as f64)).collect()),
            _ => v.clone(),
        };
        out.insert(f.name.clone(), enc);
    }
    Ok(Value::Object(out))
}

impl SpecCharacter<'_> {
    /// the records in the dump format (what `RecordsJson` reads)
    pub fn dump(&self) -> Result<Value, String> {
        let s = self.0;
        let phys = s.record("ENT_PHYS_WORLD").map_err(|e| format!("spec: {e}"))?;
        let move_extra = section(s, "ENT_MOV_MOVE_EXTRA")?;
        let mut curves = Map::new();
        for k in ["turn_sprint_prevention_decay_curve", "turn_sprint_prevention_slowdown_curve"] {
            let Some(path) = move_extra[k].as_str().filter(|p| !p.is_empty()) else { continue };
            let c = s.curve(path).map_err(|e| format!("spec curve {path}: {e}"))?;
            let keys: Vec<Value> = c.keys.iter()
                .map(|(t, v, m)| json!({"time": hex(*t as f64), "value": hex(*v as f64), "interp": m})).collect();
            curves.insert(path.to_string(), json!({"keys": keys, "extrap": [c.pre, c.post]}));
        }
        Ok(json!({
            "movement": section(s, "ENT_MOV_MOVEMENT")?,
            "move_extra": move_extra,
            "character": section(s, "ENT_CHR_BP_MordhauCharacter")?,
            "gravity_z": hex(phys.f32("gravity_z").map_err(|e| format!("spec: {e}"))? as f64),
            "terminal_velocity": hex(phys.f32("terminal_velocity").map_err(|e| format!("spec: {e}"))? as f64),
            "curves": curves,
        }))
    }
}

impl CharacterSource for SpecCharacter<'_> {
    fn load(&self) -> Result<CharacterRecords, String> {
        RecordsJson(&self.dump()?.to_string()).load()
    }
}

//! Explicit mod authoring around original FAttackInfo, motion defaults and UCurveFloat keys.
//! Native timing/evaluation algorithms remain in combat; edits are validated on a detached Spec.
//! See TIMING-AUTHORING-PROPOSAL.md and its independent review, 2026-10-08.
use crate::data::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::rc::Rc;

type Fields = BTreeMap<String, Value>;
fn version() -> u32 { 1 }

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WeaponTiming {
    pub values: Fields,
    pub attacks: BTreeMap<String, Fields>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MotionTiming {
    pub attack: Option<Fields>,
    pub parry: Option<Fields>,
    pub blocked: Option<Fields>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimingEdits {
    #[serde(default = "version")]
    pub schema_version: u32,
    #[serde(default)]
    pub weapons: BTreeMap<String, WeaponTiming>,
    #[serde(default)]
    pub motions: BTreeMap<String, MotionTiming>,
    #[serde(default)]
    pub curves: BTreeMap<String, Value>,
}
impl Default for TimingEdits {
    fn default() -> Self { Self { schema_version: 1, weapons: Default::default(), motions: Default::default(), curves: Default::default() } }
}

fn number(v: &Value, name: &str) -> Result<f64, String> {
    let n = v.as_f64().ok_or_else(|| format!("{name}: expected a number"))?;
    let f = n as f32;
    if !n.is_finite() || !f.is_finite() { return Err(format!("{name}: expected a finite binary32 value")); }
    Ok(f as f64)
}

macro_rules! fields {
    ($export:ident, $patch:ident, $ty:ty; $($field:ident),+ $(,)?) => {
        fn $export(d: &$ty) -> Fields {
            [$( (stringify!($field).to_owned(), serde_json::json!(d.$field)) ),+].into_iter().collect()
        }
        fn $patch(d: &mut $ty, edits: &Fields) -> Result<(), String> {
            for (name, value) in edits {
                match name.as_str() {
                    $(stringify!($field) => d.$field = number(value, name)?,)+
                    _ => return Err(format!("unsupported timing field {name}")),
                }
            }
            Ok(())
        }
    }
}
fields!(attack_values, patch_attack, AttackInfo;
    windup, release, miss_recovery, combo_windup_increase, miss_combo_extra_windup_increase,
    feint_lock_out, hit_effect_speed_up_exponent);
fields!(weapon_values, patch_weapon, WeaponData; stab_release_modifier);
fields!(motion_attack_values, patch_motion_attack_numbers, AttackDef;
    clash_on_parry_follow_up_windup, morph_windup_modifier, riposte_windup_modifier,
    min_windup_time_before_morphing, recovery_queue_window, riposte_windup_can_parry_window,
    hit_recovery, clashed_recovery, hit_stop_recovery, early_release, early_release_time_factor,
    riposte_early_release, riposte_early_release_time_factor,
    extra_early_release_for_look_up_non_undercuts, extra_early_release_for_look_up_overheads,
    early_release_is_clashable_after);
fields!(parry_values, patch_parry, ParryDef;
    parry_up_time, parry_recovery_time, minimum_held_parry_time, non_held_parry_extension_time,
    minimum_held_riposte_parry_time, shield_wall_raise_time, miss_parry_recovery_time,
    shield_wall_recovery_time, held_parry_recovery_time, held_parry_success_recovery_time,
    parry_success_recovery_time, parry_in_flinch_duration_max,
    non_held_parry_extension_and_riposte_window_extra, riposte_window_base,
    held_riposte_window_extra, easy_parry_duration, held_block_memory_duration,
    timed_block_memory_duration);
fields!(blocked_values, patch_blocked, BlockedDef;
    parried_recovery_time_offset, world_recovery_time, chambered_recovery_time_offset,
    queue_window, queue_window_hit);

fn attack_mut<'a>(w: &'a mut WeaponData, kind: &str) -> Result<&'a mut AttackInfo, String> {
    match kind {
        "strike" => Ok(&mut w.strike), "stab" => Ok(&mut w.stab),
        "second_strike" => Ok(&mut w.second_strike), "second_stab" => Ok(&mut w.second_stab),
        "couch" => Ok(&mut w.couch), "second_couch" => Ok(&mut w.second_couch),
        "kick" => Ok(&mut w.kick), "second_kick" => Ok(&mut w.second_kick), "bash" => Ok(&mut w.bash),
        _ => Err(format!("unknown attack record {kind}")),
    }
}

fn weapon_snapshot(w: &WeaponData) -> WeaponTiming {
    let attacks = [
        ("strike", &w.strike), ("stab", &w.stab),
        ("second_strike", &w.second_strike), ("second_stab", &w.second_stab),
        ("couch", &w.couch), ("second_couch", &w.second_couch),
        ("kick", &w.kick), ("second_kick", &w.second_kick), ("bash", &w.bash),
    ].into_iter().map(|(k, a)| (k.into(), attack_values(a))).collect();
    WeaponTiming { values: weapon_values(w), attacks }
}

fn motion_snapshot(m: &MotionDef) -> MotionTiming {
    let attack = m.attack.as_ref().map(|a| {
        let mut out = motion_attack_values(a);
        for (name, value) in [("windup_curve", &a.windup_curve), ("morph_windup_curve", &a.morph_windup_curve),
            ("release_curve", &a.release_curve), ("riposte_release_curve", &a.riposte_release_curve)] {
            out.insert(name.into(), Value::String(value.clone()));
        }
        // ComboWindUpCurve is perspective-aware and captured independently. Its existing FP/TP
        // keys are editable by asset path; core-only reference replacement is intentionally rejected.
        out
    });
    MotionTiming { attack, parry: m.parry.as_ref().map(parry_values), blocked: m.blocked.as_ref().map(blocked_values) }
}

/// Export original class timing values. Unsupported curves remain readable in the source assets,
/// but are omitted from the editable set so exporting defaults never creates an invalid patch.
pub fn export(spec: &Spec) -> TimingEdits {
    TimingEdits {
        schema_version: 1,
        weapons: spec.weapons.iter().map(|(p,w)| (p.clone(), weapon_snapshot(&w.weapon))).collect(),
        motions: spec.motion_defs.iter().map(|(p,m)| (p.clone(), motion_snapshot(m))).collect(),
        curves: spec.curves.iter().filter_map(|(p,c)| {
            let value = serde_json::to_value(c).ok()?;
            validate_curve(&value).ok().map(|_| (p.clone(), value))
        }).collect(),
    }
}

/// Exact original keys are retained until explicitly edited; new keys/numbers are stored as UE floats.
fn validate_curve(value: &Value) -> Result<Curve, String> {
    let object = value.as_object().ok_or("curve: expected object")?;
    if object.keys().any(|k| !matches!(k.as_str(), "keys" | "pre" | "post")) {
        return Err("curve: unknown field".into());
    }
    for key in object.get("keys").and_then(Value::as_array).ok_or("curve: expected keys array")? {
        let fields = key.as_object().ok_or("curve key: expected object")?;
        if fields.keys().any(|k| !matches!(k.as_str(), "Time" | "Value" | "InterpMode" | "TangentMode" |
            "TangentWeightMode" | "ArriveTangent" | "LeaveTangent" | "ArriveTangentWeight" | "LeaveTangentWeight")) {
            return Err("curve key: unknown field".into());
        }
    }
    let mut c: Curve = serde_json::from_value(value.clone()).map_err(|e| format!("curve: {e}"))?;
    for e in [&c.pre, &c.post] {
        if !matches!(e.as_str(), "RCCE_Constant" | "RCCE_Linear") {
            return Err(format!("curve extrapolation {e} is not reconstructed"));
        }
    }
    let mut previous = None;
    for k in &mut c.keys {
        if !matches!(k.interp_mode.as_deref(), None | Some("RCIM_Linear" | "RCIM_Constant" | "RCIM_Cubic")) {
            return Err("curve interpolation is not reconstructed".into());
        }
        if !matches!(k.tangent_weight_mode.as_deref(), None | Some("RCTWM_WeightedNone")) {
            return Err("weighted curve tangents are not reconstructed".into());
        }
        if !matches!(k.tangent_mode.as_deref(), None | Some("RCTM_Auto" | "RCTM_User" | "RCTM_Break" | "RCTM_None")) {
            return Err("unknown curve tangent mode".into());
        }
        // Cooked/evaluated curves consume explicit tangents. This authoring boundary does not
        // silently recalculate Unreal editor Auto tangents after changing a key's value.
        for (name, value) in [("Time", &mut k.time), ("Value", &mut k.value),
            ("ArriveTangent", &mut k.arrive_tangent), ("LeaveTangent", &mut k.leave_tangent),
            ("ArriveTangentWeight", &mut k.arrive_tangent_weight), ("LeaveTangentWeight", &mut k.leave_tangent_weight)] {
            *value = number(&serde_json::json!(*value), name)?;
        }
        if previous.is_some_and(|t| k.time <= t) {
            return Err("curve times must increase strictly after binary32 rounding".into());
        }
        previous = Some(k.time);
    }
    Ok(c)
}

pub fn parse(text: &str) -> Result<TimingEdits, String> {
    serde_json::from_str(text).map_err(|e| format!("combat timings: {e}"))
}

/// Build a complete candidate without mutating the baseline. Empty edits preserve the original Rc data.
pub fn apply(base: &Spec, edits: &TimingEdits) -> Result<Spec, String> {
    if edits.schema_version != 1 { return Err("unsupported combat timing schema".into()); }
    let mut out = base.clone();
    for (path, edit) in &edits.weapons {
        let w = out.weapons.get_mut(path).ok_or_else(|| format!("unknown weapon {path}"))?;
        let data = Rc::make_mut(&mut w.weapon);
        patch_weapon(data, &edit.values)?;
        for (kind, values) in &edit.attacks { patch_attack(attack_mut(data, kind)?, values)?; }
    }
    for (path, value) in &edits.curves {
        let curve = validate_curve(value)?;
        // A full exported stock catalog must not turn unchanged FP pak keys into an override.
        if base.curves.get(path).is_some_and(|c| serde_json::to_value(c).ok().as_ref() == Some(value)) { continue; }
        out.curve_overrides.insert(path.clone(), curve);
    }
    for (path, edit) in &edits.motions {
        let m = out.motion_defs.get_mut(path).ok_or_else(|| format!("unknown motion {path}"))?;
        let m = Rc::make_mut(m);
        if let Some(values) = &edit.attack {
            let a = m.attack.as_mut().ok_or_else(|| format!("{path} is not an attack motion"))?;
            let mut numbers = values.clone();
            for (name, target) in [("windup_curve", &mut a.windup_curve), ("morph_windup_curve", &mut a.morph_windup_curve),
                ("release_curve", &mut a.release_curve), ("riposte_release_curve", &mut a.riposte_release_curve)] {
                if let Some(value) = numbers.remove(name) {
                    let reference = value.as_str().ok_or_else(|| format!("{name}: expected curve path"))?;
                    // All explicit custom curves must be supplied in this same atomic document.
                    if !reference.is_empty() && !base.curves.contains_key(reference) && !edits.curves.contains_key(reference) {
                        return Err(format!("unknown curve {reference}"));
                    }
                    *target = reference.into();
                }
            }
            patch_motion_attack_numbers(a, &numbers)?;
        }
        if let Some(values) = &edit.parry { patch_parry(m.parry.as_mut().ok_or_else(|| format!("{path} is not a parry motion"))?, values)?; }
        if let Some(values) = &edit.blocked { patch_blocked(m.blocked.as_mut().ok_or_else(|| format!("{path} is not a blocked motion"))?, values)?; }
    }
    Ok(out)
}

/// Refresh just authored timing fields on an existing weapon actor. Instance flags, actor identity,
/// damage/blood/placement and all unrelated values survive both apply and reset.
pub(crate) fn copy_weapon_timings(target: &mut WeaponData, source: &WeaponData) {
    patch_weapon(target, &weapon_values(source)).expect("validated weapon timings");
    for (kind, values) in weapon_snapshot(source).attacks {
        patch_attack(attack_mut(target, &kind).unwrap(), &values).expect("validated attack timings");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Spec {
        let mut s = Spec::default();
        let mut weapon = WeaponData::default();
        weapon.strike.windup = 0.5; weapon.strike.release = 0.5; weapon.second_stab.release = 0.25;
        s.weapons.insert("greatsword".into(), WeaponSetup { weapon: Rc::new(weapon), equip: None, motions: Default::default(), tracer: Default::default() });
        s.motion_defs.insert("strike".into(), Rc::new(MotionDef { attack: Some(AttackDef { early_release: 0.25, early_release_time_factor: 1.5, ..Default::default() }), ..Default::default() }));
        s
    }
    #[test]
    fn absent_zero_alternate_and_invalid_atomic() {
        let base = fixture();
        let blank = apply(&base, &TimingEdits::default()).unwrap();
        assert!(Rc::ptr_eq(&base.weapons["greatsword"].weapon, &blank.weapons["greatsword"].weapon));
        let edit = parse(r#"{"weapons":{"greatsword":{"attacks":{"strike":{"release":0},"second_stab":{"release":0.4}}}}}"#).unwrap();
        let out = apply(&base, &edit).unwrap();
        assert_eq!(out.weapons["greatsword"].weapon.strike.release, 0.0);
        assert_eq!(out.weapons["greatsword"].weapon.strike.windup, 0.5);
        assert_eq!(out.weapons["greatsword"].weapon.second_stab.release, 0.4f32 as f64);
        assert_eq!(base.weapons["greatsword"].weapon.strike.release, 0.5);
        let bad = parse(r#"{"weapons":{"greatsword":{"attacks":{"strike":{"release":0.2,"invented":3}}}}}"#).unwrap();
        assert!(apply(&base, &bad).is_err());
        assert_eq!(base.weapons["greatsword"].weapon.strike.release, 0.5);
        let combo = parse(r#"{"motions":{"strike":{"attack":{"combo_windup_curve":"wrong"}}}}"#).unwrap();
        assert!(apply(&base, &combo).is_err());
    }
    #[test]
    fn curve_keys_modes_and_reference_validation() {
        let base = fixture();
        let edit = parse(r#"{"curves":{"custom":{"keys":[{"Time":0,"Value":0,"InterpMode":"RCIM_Linear"},{"Time":1,"Value":0.5}],"pre":"RCCE_Constant","post":"RCCE_Constant"}},"motions":{"strike":{"attack":{"release_curve":"custom","early_release":0}}}}"#).unwrap();
        let out = apply(&base, &edit).unwrap();
        assert_eq!(out.curve_value("custom",0.5),0.25);
        assert_eq!(out.motion_defs["strike"].attack().early_release,0.0);
        assert!(base.curve_overrides.is_empty());
        let mut bad = edit.clone(); bad.curves.get_mut("custom").unwrap()["keys"][0]["TangentWeightMode"] = "RCTWM_WeightedBoth".into();
        assert!(apply(&base,&bad).is_err());
        let mut bad = edit.clone(); bad.curves.get_mut("custom").unwrap()["post"] = "RCCE_Cycle".into();
        assert!(apply(&base,&bad).is_err());
        let mut bad = edit.clone(); bad.curves.get_mut("custom").unwrap()["keys"][1]["Time"] = 0.0.into();
        assert!(apply(&base,&bad).is_err());
        let mut bad = edit.clone(); bad.curves.get_mut("custom").unwrap()["keys"][0]["TangentMode"] = "imaginary".into();
        assert!(apply(&base,&bad).is_err());
        let mut stock = fixture();
        stock.curves.insert("stock".into(),serde_json::from_value(edit.curves["custom"].clone()).unwrap());
        let exported = export(&stock);
        assert!(apply(&stock,&exported).unwrap().curve_overrides.is_empty());
    }
    #[test]
    fn original_actor_reload_busy_snapshot_and_stock_reset() {
        use crate::combat::{World, enums::{at, mv}};
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/spec.json");
        let required = std::env::var("MORDHAU_GOLDEN_REQUIRED").is_ok_and(|v| v == "1");
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) => { assert!(!required,"required original timings {}: {e}",p.display()); return; }
        };
        let base = Rc::new(RecordsJsonExe(&text).load_spec().unwrap());
        const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
        let mut w = World::new(base.clone(),1.0/120.0);
        let fi = w.add_fighter("A",LS,"");
        w.set_weapon_no_drop(fi);
        let actor = w.fighters[fi].right_actor.unwrap();
        let mut edit = TimingEdits::default();
        edit.weapons.insert(LS.into(),WeaponTiming { attacks: [("strike".into(),[("release".into(),serde_json::json!(0.45))].into_iter().collect())].into_iter().collect(), ..Default::default() });
        let changed = Rc::new(apply(&base,&edit).unwrap());
        w.install_timing_spec(changed).unwrap();
        assert_eq!(w.fighters[fi].right_actor,Some(actor));
        assert_eq!(w.fighters[fi].weapon.as_ref().unwrap().strike.release,0.45f32 as f64);
        assert!(!w.equipment_actor(actor).unwrap().weapon.as_ref().unwrap().b_allow_drop);
        w.assign_net_attack_motion(fi,at::REGULAR,mv::RIGHT_STRIKE,0.0);
        let captured = w.cur(fi).unwrap();
        assert_eq!(w.m(fi,captured).attack().unwrap().ai.release,0.45f32 as f64);
        assert!(!w.timing_edit_ready());
        assert!(w.install_timing_spec(base.clone()).is_err());
        assert_eq!(w.m(fi,captured).attack().unwrap().ai.release,0.45f32 as f64);
        for _ in 0..300 { w.step(); }
        assert!(w.timing_edit_ready());
        let health = w.fighters[fi].health;
        w.install_timing_spec(base.clone()).unwrap();
        assert_eq!(w.fighters[fi].weapon.as_ref().unwrap().strike.release,base.weapons[LS].weapon.strike.release);
        assert_eq!(w.fighters[fi].right_actor,Some(actor));
        assert_eq!(w.fighters[fi].health,health);
        assert!(!w.equipment_actor(actor).unwrap().weapon.as_ref().unwrap().b_allow_drop);
        w.switch_mode_and_reattach(fi);
        assert!(w.fighters[fi].alternate_mode);
        edit.weapons.get_mut(LS).unwrap().attacks.insert("second_strike".into(),[("release".into(),serde_json::json!(0.6))].into_iter().collect());
        w.install_timing_spec(Rc::new(apply(&base,&edit).unwrap())).unwrap();
        assert_eq!(w.fighters[fi].weapon.as_ref().unwrap().strike.release,0.6f32 as f64);
        assert_eq!(w.fighters[fi].weapon.as_ref().unwrap().second_strike.release,0.45f32 as f64);
        assert_eq!(w.fighters[fi].right_actor,Some(actor));
        assert!(!w.equipment_actor(actor).unwrap().weapon.as_ref().unwrap().b_allow_drop);
        w.install_timing_spec(base.clone()).unwrap();
        assert_eq!(w.fighters[fi].weapon.as_ref().unwrap().strike.release,base.weapons[LS].weapon.second_strike.release);
        w.switch_mode_and_reattach(fi);
        assert!(!w.fighters[fi].alternate_mode);
        assert_eq!(w.fighters[fi].weapon.as_ref().unwrap().strike.release,base.weapons[LS].weapon.strike.release);
        assert_eq!(w.fighters[fi].right_actor,Some(actor));
        assert!(!w.equipment_actor(actor).unwrap().weapon.as_ref().unwrap().b_allow_drop);
    }
}

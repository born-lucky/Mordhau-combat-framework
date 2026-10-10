//! Read-only acceptance of generated weapon tables by the actual Rust consumers.
use std::{fs, path::Path};
use serde_json::{json, Value};
use mordhau_core::data::{AttackInfo, EquipmentDef, WeaponData};

fn required_fields(source: &str, name: &str, value: &Value) -> Result<(), String> {
    let marker = format!("pub struct {name} {{");
    let body = source.split_once(&marker).ok_or(format!("Missing consumer schema {name}"))?.1
        .split_once("\n}").ok_or("Unterminated consumer schema")?.0;
    let mut count = 0;
    for line in body.lines() {
        if let Some(field) = line.trim().strip_prefix("pub ").and_then(|s| s.split_once(':').map(|p| p.0)) {
            count += 1;
            if value.get(field).is_none() { return Err(format!("Missing required {name}.{field}; serde defaults cannot establish acceptance")); }
        }
    }
    if count == 0 { return Err(format!("Empty consumer schema {name}")); }
    Ok(())
}

fn same_value(expected: &Value, actual: &Value, what: &str) -> Result<(), String> {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) => {
            if (a.as_f64().ok_or(what)? as f32).to_bits() != (b.as_f64().ok_or(what)? as f32).to_bits() {
                return Err(format!("Numeric bits differ: {what}"));
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (x,y)) in a.iter().zip(b).enumerate() { same_value(x,y,&format!("{what}[{i}]"))?; }
        }
        _ if expected == actual => (),
        _ => return Err(format!("Original record and matrix differ: {what}")),
    }
    Ok(())
}

fn run() -> Result<Value, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 { return Err("Usage: mh-verify-weapon-import MATRIX RECORDS_JSON CONSUMER_DATA_RS".into()); }
    let source = fs::read_to_string(&args[2]).map_err(|e|e.to_string())?;
    let raw: Value = serde_json::from_slice(&fs::read(&args[1]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let records = raw.as_object().ok_or("Weapon records are not an object")?;
    let spec = mh_spec::Spec::load(Path::new(&args[0]), false).map_err(|e|e.to_string())?;
    let slots = ["strike", "second_strike", "stab", "second_stab", "couch", "second_couch", "kick", "second_kick", "bash"];
    // Independent authored GetBaseAttackInfo semantics. ALT_STAB means left
    // stab; alternate grip selects SecondStab rather than changing attack side.
    let moves = [("RIGHT_STRIKE","strike"),("LEFT_STRIKE","strike"),("STAB","stab"),
        ("ALT_STAB","stab"),("KICK","kick"),("BASH","bash"),("COUCH","couch"),("RANGED","strike")];
    let mut attacks = 0; let mut native_attacks = 0; let mut lookups = 0; let mut alternate = 0;
    for eid in spec.entities_of("weapon") {
        // Grouping every equip_ prefix also consumes WeaponData.equip_time_modifier.
        // Extract exactly the EquipmentDef keys, leaving weapon fields intact.
        let mut weapon = spec.nested(eid, &[]).map_err(|e|e.to_string())?;
        let wid = weapon["id"].as_str().ok_or("Weapon id absent")?.to_string();
        let original = records.get(&wid).ok_or(format!("Original source record absent: {wid}"))?;
        for slot in slots.into_iter().chain(["base_strike","base_second_strike","base_stab"]) {
            if original["weapon"].get(slot).is_none() { continue; }
            let aid = spec.reference(eid, &format!("FLD_WPN_ATTACK_{}",slot.to_uppercase())).map_err(|e|e.to_string())?
                .ok_or(format!("Native attack slot absent: {eid}.{slot}"))?;
            let attack = spec.nested(aid, &[]).map_err(|e|e.to_string())?;
            required_fields(&source,"AttackInfo",&attack)?;
            for (key,value) in original["weapon"][slot].as_object().ok_or("Source attack absent")? {
                if key.starts_with("__") || key=="unset" { continue; }
                same_value(value, attack.get(key).ok_or(format!("Attack field absent: {aid}.{key}"))?, &format!("{aid}.{key}"))?;
            }
            let _: AttackInfo = serde_json::from_value(attack.clone()).map_err(|e|e.to_string())?;
            weapon[slot] = attack;
            attacks += 1;
            if slots.contains(&slot) {native_attacks+=1;}
        }
        required_fields(&source,"WeaponData",&weapon)?;
        let w: WeaponData = serde_json::from_value(weapon.clone()).map_err(|e|e.to_string())?;
        let mut equip = json!({});
        for key in original["equip"].as_object().ok_or("Source equipment absent")?.keys() {
            let flat = format!("equip_{key}");
            if let Some(value) = weapon.get(&flat) { equip[key] = value.clone(); }
        }
        for key in ["b_has_alternate_mode", "b_is_two_handed", "b_second_is_two_handed"] { equip[key] = weapon[key].clone(); }
        equip["path"] = weapon["class_path"].clone(); equip["native"] = original["equip"]["native"].clone();
        required_fields(&source,"EquipmentDef",&equip)?;
        let e: EquipmentDef = serde_json::from_value(equip.clone()).map_err(|err|err.to_string())?;
        for (key,value) in original["equip"].as_object().ok_or("Source equipment absent")? {
            if key.starts_with("__") { continue; }
            same_value(value, equip.get(key).ok_or(format!("Equipment field absent: {wid}.{key}"))?, &format!("{wid}.equip.{key}"))?;
        }
        for (key,value) in original["weapon"].as_object().ok_or("Source weapon absent")? {
            if key.starts_with("__") || key=="unset" || slots.contains(&key.as_str()) || key.starts_with("base_") { continue; }
            same_value(value, weapon.get(key).ok_or(format!("Weapon field absent: {wid}.{key}"))?, &format!("{wid}.{key}"))?;
        }
        for (source_key, matrix_key) in [("profile","profile"),("parry_motion","parry_motion_path"),
            ("motions","motion_paths"),("alt_profile","alt_profile"),
            ("alt_parry_motion","alt_parry_motion_path"),("alt_motions","alt_motion_paths")] {
            if let Some(value) = original.get(source_key) {
                same_value(value,weapon.get(matrix_key).ok_or(format!("Profile field absent: {wid}.{matrix_key}"))?,
                    &format!("{wid}.{matrix_key}"))?;
            }
        }
        let switched = w.switched();
        if format!("{:?}",switched.switched()) != format!("{w:?}") { return Err(format!("Double weapon switch changed {wid}")); }
        if format!("{:?}",e.switched().switched()) != format!("{e:?}") { return Err(format!("Double equipment switch changed {wid}")); }
        for (first,second) in [(&w.strike,&switched.second_strike),(&w.second_strike,&switched.strike),
            (&w.stab,&switched.second_stab),(&w.second_stab,&switched.stab),
            (&w.couch,&switched.second_couch),(&w.second_couch,&switched.couch),
            (&w.kick,&switched.second_kick),(&w.second_kick,&switched.kick)] {
            if format!("{first:?}") != format!("{second:?}") { return Err(format!("Attack mode switch differs: {wid}")); }
        }
        if format!("{:?}",w.block_stamina_clamp)!=format!("{:?}",switched.second_block_stamina_clamp)
            || format!("{:?}",w.second_block_stamina_clamp)!=format!("{:?}",switched.block_stamina_clamp)
            || w.block_stamina_negation.to_bits()!=switched.second_block_stamina_negation.to_bits()
            || w.second_block_stamina_negation.to_bits()!=switched.block_stamina_negation.to_bits()
            || e.weapon_animation_profile!=e.switched().second_weapon_animation_profile
            || e.second_weapon_animation_profile!=e.switched().weapon_animation_profile
            || e.second_length.to_bits()!=e.switched().length.to_bits() {
            return Err(format!("Alternate mode field swap differs: {wid}"));
        }
        for (mv,normal_slot) in moves {
            for alt in [false,true] {
                let slot = if alt && matches!(normal_slot,"strike"|"stab"|"kick"|"couch") {format!("second_{normal_slot}")} else {normal_slot.to_string()};
                let expected = format!("ENT_ATK_{wid}_{}",slot.to_uppercase());
                if spec.attack_for_move(eid,mv,alt).map_err(|err|err.to_string())? != Some(expected.as_str()) {
                    return Err(format!("Wrong attack selection {wid}/{mv}/{alt}"));
                }
                lookups += 1;
            }
        }
        if w.stab.windup.to_bits()!=switched.second_stab.windup.to_bits() || e.length.to_bits()!=e.switched().second_length.to_bits() {
            return Err(format!("Native switch semantics differ {wid}"));
        }
        if weapon["b_has_alternate_mode"]==true {alternate+=1;}
    }
    let total_attacks=spec.entities_of("attack").count();
    if spec.entities_of("weapon").count()!=records.len() {return Err("Weapon matrix count differs from source records".into());}
    if attacks!=total_attacks {return Err("Attack matrix contains unchecked records".into());}
    Ok(json!({"weapon_records":records.len(),"native_attack_slots_checked":native_attacks,"attack_records_checked":attacks,"attack_entities":total_attacks,
        "alternate_mode_weapons":alternate,"attack_lookups_checked":lookups,"rust_consumers_accepted":true,"runtime_ready":false}))
}

fn main() {
    match run() { Ok(value)=>println!("{value}"), Err(error)=>{println!("{}",json!({"rust_consumers_accepted":false,"error":error}));std::process::exit(2);} }
}

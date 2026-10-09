// mh-spec tests. Unit cases run on a tiny inline matrix; the data cases run on the generated data_gen/spec/ and the
// record dump godot/data_gen/spec_src/ (both local Triternion data, git-ignored) and are skipped with a message when
// those are absent.
//
// EQUIVALENCE (the "migrate one subsystem" proof, docs/SPEC_SHEETS.md): every combat value the port reads from a
// record (godot/game/combat/attack_info.gd AttackInfo per weapon attack slot, weapon_data.gd WeaponData, the
// MotionDefs timing windows) equals the spec value looked up by ID, compared as f32 (UE float), and the generic
// attack_for_move rule picks the same AttackInfo the record path picks (CombatData.attack_info_for = GetBaseAttackInfo).
use mh_spec::{ids, FieldType, Spec, SpecError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn tiny() -> Spec {
    let index = serde_json::from_value(json!({"format": "mordhau-spec/1", "content_sha1": "x",
        "entity_types": {"weapon": {"file": "entities/weapon.json", "code": "WPN", "entities": 1, "fields": 3, "values": 3},
                         "attack": {"file": "entities/attack.json", "code": "ATK", "entities": 2, "fields": 2, "values": 3}},
        "counts": {}, "evidence_status": {}})).unwrap();
    let f = |et: &str, name: &str, ty: &str, rt: &str| json!({"entity_type": et, "name": name, "type": ty, "ref_type": rt,
        "unit": "", "unit_source": "", "source": "", "description": ""});
    let fields = json!({"FLD_WPN_ATTACK_STRIKE": f("weapon", "attack_strike", "ref", "attack"),
        "FLD_WPN_ATTACK_SECOND_STRIKE": f("weapon", "attack_second_strike", "ref", "attack"),
        "FLD_WPN_BLOCK_STAMINA_CLAMP": f("weapon", "block_stamina_clamp", "vec2", ""),
        "FLD_ATK_WINDUP": f("attack", "windup", "float", ""), "FLD_ATK_FEINT_COST": f("attack", "feint_cost", "int", "")});
    let rule = |p: Value| json!({"type": "lookup", "name": "", "description": "", "params": p, "fields": [], "source": "x",
        "implemented_by": [], "area": "", "tests": [], "status": "ported"});
    let rules = json!({"RULE_ATTACK_SLOT_BY_MOVE": rule(json!({"attack_slot_by_move": {"RIGHT_STRIKE": "strike"}})),
        "RULE_ALT_MODE_ATTACK_SWAP": rule(json!({"alt_mode_swaps": [["strike", "second_strike"]]}))});
    let e = |v: Value| json!({"name": "", "parent": null, "source": "SRC_X", "record": "", "values": v});
    let weapon = json!({"ENT_WPN_W": e(json!({"FLD_WPN_ATTACK_STRIKE": "ENT_ATK_W_STRIKE",
        "FLD_WPN_ATTACK_SECOND_STRIKE": "ENT_ATK_W_SECOND_STRIKE", "FLD_WPN_BLOCK_STAMINA_CLAMP": [6, 20]}))});
    let attack = json!({"ENT_ATK_W_STRIKE": e(json!({"FLD_ATK_WINDUP": 0.33, "FLD_ATK_FEINT_COST": 7})),
        "ENT_ATK_W_SECOND_STRIKE": e(json!({"FLD_ATK_WINDUP": 0.5}))});
    Spec::from_parts(index, vec![("fields.json".into(), fields.to_string()), ("rules.json".into(), rules.to_string()),
        ("sources.json".into(), "{}".into()), ("entities/weapon.json".into(), weapon.to_string()),
        ("entities/attack.json".into(), attack.to_string())]).unwrap()
}

#[test]
fn typed_lookup_checks_types() {
    let s = tiny();
    assert_eq!(s.f32("ENT_ATK_W_STRIKE", "FLD_ATK_WINDUP").unwrap(), 0.33f32);
    assert_eq!(s.i64("ENT_ATK_W_STRIKE", "FLD_ATK_FEINT_COST").unwrap(), 7);
    assert_eq!(s.vec2("ENT_WPN_W", "FLD_WPN_BLOCK_STAMINA_CLAMP").unwrap(), [6.0, 20.0]);
    assert!(matches!(s.i64("ENT_ATK_W_STRIKE", "FLD_ATK_WINDUP"), Err(SpecError::WrongType(_, FieldType::Float, "i64"))));
    assert!(matches!(s.f32("ENT_WPN_W", "FLD_ATK_WINDUP"), Err(SpecError::WrongEntityType(..))));
    assert!(matches!(s.i64("ENT_ATK_W_SECOND_STRIKE", "FLD_ATK_FEINT_COST"), Err(SpecError::Unset(..))));
    assert!(matches!(s.f32("ENT_ATK_NOPE", "FLD_ATK_WINDUP"), Err(SpecError::UnknownEntity(_))));
    assert!(matches!(s.f32("ENT_ATK_W_STRIKE", "FLD_ATK_NOPE"), Err(SpecError::UnknownField(_))));
}

#[test]
fn attack_rules_on_tiny_matrix() {
    let s = tiny();
    assert_eq!(s.attack_for_move("ENT_WPN_W", "RIGHT_STRIKE", false).unwrap(), Some("ENT_ATK_W_STRIKE"));
    assert_eq!(s.attack_for_move("ENT_WPN_W", "RIGHT_STRIKE", true).unwrap(), Some("ENT_ATK_W_SECOND_STRIKE"));
    assert!(s.attack_for_move("ENT_WPN_W", "BASH", false).is_err());
    assert_eq!(ids::field("ATK", "feint_lock_out"), "FLD_ATK_FEINT_LOCK_OUT");
    assert_eq!(ids::entity("WPN", "BP_Longsword"), "ENT_WPN_BP_Longsword");
}

fn repo() -> PathBuf {
    std::env::var("MORDHAU_REPO").map(PathBuf::from).unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

fn real() -> Option<Spec> {
    let d = Spec::default_dir();
    if !d.join("index.json").exists() {
        eprintln!("SKIP: no {} (python scripts/sheets_populate.py && python tools/sheets/build_matrix.py)", d.display());
        return None;
    }
    Some(Spec::load(&d, true).expect("load data_gen/spec"))
}

fn dump(name: &str) -> Option<Value> {
    let p = repo().join("godot/data_gen/spec_src").join(format!("{name}.json"));
    match std::fs::read_to_string(&p) {
        Ok(t) => Some(serde_json::from_str(&t).unwrap()),
        Err(_) => {
            eprintln!("SKIP: no {} (godot/tools/gen_spec_src.gd)", p.display());
            None
        }
    }
}

#[test]
fn loads_generated_matrix() {
    let Some(s) = real() else { return };
    assert_eq!(s.entities.len(), s.index.counts["entities"]);
    assert_eq!(s.fields.len(), s.index.counts["fields"]);
    assert_eq!(s.rules.len(), s.index.counts["rules"]);
    assert_eq!(s.evidence.len(), s.index.counts["evidence"]);
    for (et, info) in &s.index.entity_types {
        assert_eq!(s.entities_of(et).count(), info.entities, "{et}");
    }
    // every evidence target resolves (build_matrix checked it; the loader agrees)
    for (id, e) in &s.evidence {
        let t = e.target_id.split(':').next().unwrap();
        assert!(s.entities.contains_key(t) || s.fields.contains_key(t) || s.rules.contains_key(t), "{id} -> {t}");
    }
    // tiers: only a..d; tier d exactly when an observed (parity) evidence row backs it
    for (id, f) in &s.fields {
        assert!(f.tiers.iter().all(|t| ["a", "b", "c", "d"].contains(&t.as_str())), "{id}: {:?}", f.tiers);
    }
    assert!(s.fields.values().any(|f| f.has_tier("d")), "no demo-observed field");
    assert!(s.fields["FLD_ATK_WINDUP"].has_tier("a"), "attack windup is shipped data");
}

/// record value (spec_src dump) vs spec value of the same field, as the spec type says
fn same(s: &Spec, ent: &str, fid: &str, rec: &Value) -> Result<(), String> {
    let f = s.field(fid).unwrap();
    let bad = |x: String| Err(format!("{ent} {fid}: record {rec} vs spec {x}"));
    match f.ty {
        FieldType::Float => {
            let v = s.f32(ent, fid).map_err(|e| e.to_string())?;
            if rec.as_f64().map(|r| r as f32) != Some(v) { return bad(v.to_string()); }
        }
        FieldType::Int => {
            let v = s.i64(ent, fid).map_err(|e| e.to_string())?;
            if rec.as_f64() != Some(v as f64) { return bad(v.to_string()); }
        }
        FieldType::Bool => {
            let v = s.bool(ent, fid).map_err(|e| e.to_string())?;
            if rec.as_bool() != Some(v) { return bad(v.to_string()); }
        }
        FieldType::String => {
            let v = s.str(ent, fid).map_err(|e| e.to_string())?;
            if rec.as_str().unwrap_or("") != v { return bad(v.to_string()); }
        }
        FieldType::Vec2 | FieldType::Vec3 | FieldType::Color | FieldType::FloatList => {
            let v = s.value(ent, fid).unwrap().as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect::<Vec<_>>();
            let r = rec.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN) as f32).collect::<Vec<_>>());
            if r.as_ref() != Some(&v) { return bad(format!("{v:?}")); }
        }
        _ => {}
    }
    Ok(())
}

const SLOTS: [&str; 12] = ["strike", "second_strike", "stab", "second_stab", "couch", "second_couch", "kick", "second_kick",
    "bash", "base_strike", "base_second_strike", "base_stab"];

#[test]
fn equivalence_combat_weapons_and_attacks() {
    let (Some(s), Some(w)) = (real(), dump("weapons")) else { return };
    let mut n = 0usize;
    let mut fails = vec![];
    for (wid, e) in w.as_object().unwrap() {
        let rec = &e["weapon"];
        let went = ids::entity("WPN", wid);
        for (k, v) in rec.as_object().unwrap() {
            if k.starts_with("__") || SLOTS.contains(&k.as_str()) || k == "unset" { continue; }
            let fid = ids::field("WPN", k);
            if let Err(m) = same(&s, &went, &fid, v) { fails.push(m) }
            n += 1;
        }
        for slot in SLOTS {
            let a = &rec[slot];
            if a.is_null() { continue; }
            let aent = s.reference(&went, &ids::field("WPN", &format!("attack_{slot}"))).unwrap().unwrap().to_string();
            for (k, v) in a.as_object().unwrap() {
                if k.starts_with("__") || k == "unset" { continue; }
                if let Err(m) = same(&s, &aent, &ids::field("ATK", k), v) { fails.push(m) }
                n += 1;
            }
        }
    }
    assert!(fails.is_empty(), "{} of {n} values differ, first: {:?}", fails.len(), &fails[..fails.len().min(5)]);
    assert!(n > 100_000, "only {n} values compared");
    eprintln!("equivalence: {n} weapon + attack values equal (f32)");
}

#[test]
fn equivalence_motion_timings() {
    let (Some(s), Some(m)) = (real(), dump("motions")) else { return };
    let mut n = 0usize;
    let mut fails = vec![];
    for (key, rec) in m.as_object().unwrap() {
        let ent = match key.strip_prefix("native:") {
            Some(c) => ids::entity("MOT", &format!("NATIVE_{c}")),
            None => ids::entity("MOT", key.rsplit('/').next().unwrap()),
        };
        for (k, v) in rec.as_object().unwrap() {
            if k.starts_with("__") || v.is_object() { continue; }
            if let Err(e) = same(&s, &ent, &ids::field("MOT", k), v) { fails.push(e) }
            n += 1;
        }
    }
    assert!(fails.is_empty(), "{} of {n} differ: {:?}", fails.len(), &fails[..fails.len().min(5)]);
    eprintln!("equivalence: {n} motion values equal (f32)");
}

/// the generic rule picks the AttackInfo the record path picks (CombatData.attack_info_for, MotionSystem.switch_mode)
#[test]
fn equivalence_attack_for_move() {
    let (Some(s), Some(w), Some(r)) = (real(), dump("weapons"), dump("rules")) else { return };
    let by_move = r["attack_slot_by_move"].as_object().unwrap();
    let swaps: Vec<(String, String)> = r["alt_mode_swaps"].as_array().unwrap().iter()
        .map(|p| (p[0].as_str().unwrap().to_string(), p[1].as_str().unwrap().to_string())).collect();
    let mut n = 0;
    for wid in w.as_object().unwrap().keys() {
        let went = ids::entity("WPN", wid);
        for (mv, slot) in by_move {
            for alt in [false, true] {
                let mut want = slot.as_str().unwrap().to_string();
                if alt {
                    for (a, b) in &swaps {
                        if *a == want { want = b.clone(); break; }
                        if *b == want { want = a.clone(); break; }
                    }
                }
                let exp = w[wid]["weapon"][&want].is_object().then(|| ids::entity("ATK", &format!("{wid}_{}", want.to_uppercase())));
                assert_eq!(s.attack_for_move(&went, mv, alt).unwrap().map(str::to_string), exp, "{wid} {mv} alt={alt}");
                n += 1;
            }
        }
    }
    eprintln!("attack_for_move: {n} weapon x move x mode lookups equal the record path");
}

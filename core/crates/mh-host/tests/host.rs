//! mh-host loads what the hosts loaded from the golden dump, from the spec + paks:
//!  - combat_spec(Longsword) builds and has the weapon (record equality vs the dump: mh-sim tests/sim.rs)
//!  - kismet_json() == godot/data_gen/mode/mode_kismet.json (every key, value and src)
//!  - mode_data("DU") == the duel_rooms golden header's ModeData (what mh-net-node reads today)
//! Skipped (eprintln, pass) without the spec matrix / paks / local data.
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    std::env::var("MORDHAU_REPO").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

fn data() -> Option<mh_host::Data> {
    match mh_host::Data::load() {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!("SKIP: {e}");
            None
        }
    }
}

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

#[test]
fn combat_spec_builds_from_the_matrix() {
    let Some(d) = data() else { return };
    assert!(d.weapon_paths().iter().any(|w| w == LS), "the matrix lists the Longsword");
    let s = d.combat_spec(&[LS]).expect("combat spec");
    let w = s.weapon_setup(LS);
    assert!(w.weapon.strike.windup > 0.0, "Longsword strike windup {}", w.weapon.strike.windup);
}

#[test]
fn kismet_from_spec_equals_mode_kismet_json() {
    let Some(d) = data() else { return };
    let Ok(t) = std::fs::read_to_string(root().join("godot/data_gen/mode/mode_kismet.json")) else {
        return eprintln!("SKIP: no mode_kismet.json");
    };
    let want: Value = serde_json::from_str(&t).unwrap();
    let got: Value = serde_json::from_str(&d.kismet_json().unwrap()).unwrap();
    let (w, g) = (want.as_object().unwrap(), got.as_object().unwrap());
    assert_eq!(w.len(), g.len(), "key count");
    for (k, v) in w {
        let x = &g[k];
        assert_eq!(x["src"], v["src"], "{k} src");
        let same = match (v["value"].as_f64(), x["value"].as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => v["value"] == x["value"],
        };
        assert!(same, "{k}: {} vs {}", v["value"], x["value"]);
    }
    assert!(mh_mode::kismet::Kismet::from_json(&d.kismet_json().unwrap()).is_ok());
    eprintln!("kismet: {} literals from the spec == mode_kismet.json", w.len());
}

#[test]
fn mode_data_equals_the_net_node_golden() {
    let Some(d) = data() else { return };
    let Ok(g) = std::fs::read_to_string(root().join("core/tests/golden/mode/duel_rooms.jsonl")) else {
        return eprintln!("SKIP: no duel_rooms.jsonl");
    };
    let mut header: Value = serde_json::from_str(g.lines().nth(1).unwrap()).unwrap();
    mordhau_core::data::decode_exact(&mut header);
    let golden: mh_mode::data::ModeData = serde_json::from_value(header["data"].clone()).unwrap();
    let got = d.mode_data("DU").expect("ModeData::from_spec DU");
    let (a, b) = (serde_json::to_value(&golden).unwrap(), serde_json::to_value(&got).unwrap());
    // floats as f32: the spec holds binary32, the dump the reference's f64
    fn norm(v: &Value) -> Value {
        match v {
            Value::Number(n) if n.is_f64() => serde_json::json!(n.as_f64().unwrap() as f32 as f64),
            Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), norm(x))).collect()),
            x => x.clone(),
        }
    }
    assert_eq!(norm(&a), norm(&b), "DU ModeData from the spec == the duel_rooms header");
}

#[test]
fn character_records_equal_the_dump() {
    let Some(d) = data() else { return };
    let Ok(t) = std::fs::read_to_string(root().join("core/tests/golden/character/records.json")) else {
        return eprintln!("SKIP: no records.json");
    };
    use mh_character::CharacterSource;
    let a = d.character_records().expect("character records from the spec");
    let b = mh_character::RecordsJson(&t).load().unwrap();
    assert_eq!(a.movement.max_walk_speed as f32, b.movement.max_walk_speed as f32);
    assert_eq!(a.move_extra.chasing_sprint_time_start as f32, b.move_extra.chasing_sprint_time_start as f32);
    assert_eq!(a.character.jump_stamina_cost as f32, b.character.jump_stamina_cost as f32);
    assert_eq!(a.curves.len(), b.curves.len());
}

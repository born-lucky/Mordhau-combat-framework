//! source.rs on the user's data (skips when data_gen/spec, the paks or data_gen/parity are absent; all git-ignored):
//! the exe records from the spec matrix (+ the NoChamber mod layer) equal the GDScript record dump rounded to f32.

use mh_parity::source::{matrix_spec, records_spec};
use mh_parity::LS;
use std::path::PathBuf;

fn parity_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data_gen/parity")
}

const NC_LS: &str = "Mordhau/Content/Mordhau/Maps/WeaponRemovalMap_VersionA/NoChamber/Weapons/BP_Longsword_NC";

#[test]
fn matrix_equals_records_exe() {
    let rec = parity_dir().join("records_vanilla.json");
    if !rec.exists() {
        return eprintln!("SKIP: no {}", rec.display());
    }
    let m = match matrix_spec(&[LS.to_string()], None) {
        Ok(m) => m,
        Err(e) => return eprintln!("SKIP: {e}"),
    };
    let r = records_spec(&rec, true).unwrap();
    let (a, b) = (m.weapon_setup(LS), r.weapon_setup(LS));
    assert_eq!(format!("{:?}", a.weapon), format!("{:?}", b.weapon));
    assert_eq!(format!("{:?}", a.equip), format!("{:?}", b.equip));
    assert_eq!(format!("{:?}", m.character), format!("{:?}", r.character));
}

#[test]
fn mod_layer_adds_mod_weapons_and_overlays_the_character() {
    let (layer, rec) = (parity_dir().join("mod_layer_nochamber.json"), parity_dir().join("records_modded.json"));
    if !layer.exists() || !rec.exists() {
        return eprintln!("SKIP: no mod layer");
    }
    let ws = vec![LS.to_string(), NC_LS.to_string()];
    let m = match matrix_spec(&ws, Some(&layer)) {
        Ok(m) => m,
        Err(e) => return eprintln!("SKIP: {e}"),
    };
    let r = records_spec(&rec, true).unwrap();
    for w in [LS, NC_LS] {
        assert_eq!(format!("{:?}", m.weapon_setup(w).weapon), format!("{:?}", r.weapon_setup(w).weapon), "{w}");
    }
    assert_eq!(format!("{:?}", m.character), format!("{:?}", r.character), "overlaid character");
    assert_eq!(m.kick_weapon_path, r.kick_weapon_path);
    for k in m.weapon_setup(NC_LS).motions.values() {
        assert_eq!(format!("{:?}", m.motion_defs.get(k)), format!("{:?}", r.motion_defs.get(k)), "{k}");
    }
}

//! Material resolution from the paks (mh_assets::material) against the Godot builder (ue_material.gd, reading extract/)
//! for every material the Godot port's maps use. The references are data_gen/materials/<map>_godot.json, written by
//! godot/tools/export_materials.gd:
//!   sh scripts/godot_import.sh --run timeout 900 godot --headless --path godot --script res://tools/export_materials.gd
//! Per material: master, chain, blend / two-sided / foliage, shader mode, packed layout, flat colour, and every ue_tint
//! uniform the builder set (textures by content path; floats to 1e-5). White materials (no albedo texture, no flat
//! colour) are listed by name.

mod common;
use common::compare;
use mh_assets::material::Resolver;
use mh_assets::pak_source::PakSource;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

/// Every map the Godot port generates (godot/data_gen/maps: DU/FFA/TDM/SKM/TF on Arena, Contraband, Cortile, Highlands,
/// Truce), including the HLOD proxy materials ("<HLOD package>/<material>"): each material resolved from the paks equals
/// the Godot builder's. Materials Godot has no export for ("json": false) and white materials are listed per name.
#[test]
fn map_materials_equal_godot_builder() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data_gen/materials");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("{} missing: run godot/tools/export_materials.gd (see this file's header)", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with("_godot.json") && !p.to_string_lossy().ends_with("characters_godot.json"))
        .collect();
    files.sort();
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks"));
    let src = PakSource::new(vfs.clone());
    let mut diffs: Vec<String> = Vec::new();
    let (mut total, mut hlods) = (0, 0);
    let mut white = std::collections::BTreeSet::new();
    let mut unresolved = std::collections::BTreeSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut gaps: Vec<String> = Vec::new();
    for f in &files {
        let d: Value = serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap();
        let rd = mh_pak::Reader::new(vfs.clone());
        let r = Resolver::new(&rd, &src);
        let mut n = 0;
        for g in d["materials"].as_array().unwrap() {
            let pkg = g["pkg"].as_str().unwrap();
            if g["json"] != true {
                unresolved.insert(pkg.to_string());
                continue;
            }
            n += 1;
            if !seen.insert(pkg.to_string()) {
                continue;
            }
            hlods += (g["hlod"] == true) as usize;
            compare(&r, pkg, g, &mut diffs, &mut white, &mut gaps);
        }
        total += n;
        println!("  {}: {n} materials", f.file_name().unwrap().to_string_lossy());
        src.clear_cache();
    }
    println!("  {} maps, {total} material uses, {} distinct ({hlods} HLOD), {} differences", files.len(), seen.len(), diffs.len());
    println!("  white: {white:?}");
    println!("  no Godot export (Godot leaves the glb material): {unresolved:?}");
    for x in &gaps {
        println!("  EXPORT GAP (master not exported for Godot) {x}");
    }
    for x in &diffs {
        println!("  DIFF {x}");
    }
    assert!(files.len() >= 20, "only {} maps exported", files.len());
    assert!(diffs.is_empty(), "{} differences (listed above)", diffs.len());
}

/// shaders/ue_tint.wgsl parses and validates (naga, the WGSL front end of wgpu / Bevy)
#[test]
fn ue_tint_wgsl_validates() {
    let m = naga::front::wgsl::parse_str(mh_assets::shader::ue_tint_wgsl()).unwrap_or_else(|e| {
        panic!("{}", e.emit_to_string(mh_assets::shader::ue_tint_wgsl()))
    });
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&m)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(mh_assets::shader::ue_tint_wgsl())));
    assert!(m.functions.iter().any(|(_, f)| f.name.as_deref() == Some("ue_tint_surface")));
}

/// The main menu's dome (MainMenu map, Dome_3 -> INST_MainMenuSphere -> M_PromoSphere): the master is MSM_Unlit with
/// EmissiveColor wired straight to the "Dome Emissive" vector parameter (master default C2C2C2 = 0.54), which the instance
/// overrides to 292D30 (linear 0.0225, 0.026625, 0.03). Drawn lit-white it showed as a flat grey void behind the menu.
#[test]
fn main_menu_dome_is_unlit_instance_emissive() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks"));
    let src = PakSource::new(vfs.clone());
    let rd = mh_pak::Reader::new(vfs.clone());
    let r = Resolver::new(&rd, &src);
    let pkg = "Mordhau/Content/Mordhau/Maps/MainMenu/INST_MainMenuSphere";
    let p = r.params(pkg).expect("params");
    println!("master {} unlit {} emissive_param {:?} colors {:?}", p.master, p.unlit, p.emissive_param, p.colors);
    let d = r.build(pkg).expect("material");
    assert!(d.unlit, "M_PromoSphere is MSM_Unlit");
    match d.uniforms.get("base_color") {
        Some(mh_assets::material::Uniform::V3(c)) => {
            assert!((c[0] - 0.0225).abs() < 1e-4 && (c[1] - 0.026625).abs() < 1e-4 && (c[2] - 0.03).abs() < 1e-4, "{c:?}")
        }
        u => panic!("base_color {u:?}"),
    }
}

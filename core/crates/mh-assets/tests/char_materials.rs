//! Character and weapon materials from the paks against the Godot port's builders, dumped by
//! godot/tools/export_char_materials.gd into data_gen/materials/characters_godot.json:
//!   sh scripts/godot_import.sh --run timeout 3000 godot --headless --path godot --script res://tools/export_char_materials.gd
//! - parts: every slot material of every weapon look (default + every skin part, UeWeapon) and of the default profiles'
//!   character parts (Armor.construction), resolved as the import hook does (UeMaterial.json_for -> build) == Resolver::build;
//!   and the mesh's own FSkeletalMaterial package (skeletal_mesh::SkelMaterial::material_package) names the same material.
//! - wearables: CharacterBuilder._paint's M_WEARABLE material for every DefaultProfiles wearable ==
//!   Resolver::wearable_from_class with the same pattern index and resolved colours.

mod common;
use common::{compare, diff_desc};
use mh_assets::material::Resolver;
use mh_assets::pak_source::PakSource;
use mh_assets::skeletal_mesh;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

#[test]
fn character_and_weapon_materials_equal_godot_builders() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data_gen/materials/characters_godot.json");
    let Ok(raw) = std::fs::read(&p) else {
        panic!("{} missing: run godot/tools/export_char_materials.gd (see this file's header)", p.display())
    };
    let d: Value = serde_json::from_slice(&raw).unwrap();
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks"));
    let rd = mh_pak::Reader::new(vfs.clone());
    let src = PakSource::new(vfs);
    let r = Resolver::new(&rd, &src);
    let (mut diffs, mut gaps, mut notes) = (Vec::new(), Vec::new(), Vec::new());
    let mut white = BTreeSet::new();
    let parts = d["parts"].as_array().unwrap();
    let mut n = 0;
    let (mut paks_only, mut unresolved) = (0, Vec::<String>::new());
    for g in parts {
        let key = g["pkg"].as_str().unwrap();
        if g["json"] != true {
            // Godot has no exported material JSON (its builder leaves the mesh's own glb material): resolve from the
            // paks through the mesh's FSkeletalMaterial package
            let (mesh, name) = key.rsplit_once(':').unwrap();
            let pkgm = skeletal_mesh::lod0(&src, mesh).ok().and_then(|m| m.materials.iter().find(|s| s.material == name).map(|s| s.material_package.clone()));
            match pkgm.as_deref().and_then(|p| r.build(p)) {
                Some(m) => {
                    paks_only += 1;
                    if m.is_white() {
                        white.insert(m.package.clone());
                    }
                }
                None => unresolved.push(key.to_string()),
            }
            src.clear_cache();
            continue;
        }
        n += 1;
        compare(&r, key, g, &mut diffs, &mut white, &mut gaps);
        // the mesh's own slot material package (paks) vs the material json Godot's json_for found by name
        let mesh = g["mesh"].as_str().unwrap();
        if let Ok(m) = skeletal_mesh::lod0(&src, mesh) {
            let name = key.rsplit('/').next().unwrap();
            if let Some(s) = m.materials.iter().find(|s| s.material == name) {
                if s.material_package != key {
                    notes.push(format!("{mesh}: slot {name} is {} in the mesh, Godot's json_for found {key}", s.material_package));
                }
            }
        }
        src.clear_cache();
    }
    let wear = d["wearables"].as_array().unwrap();
    let (mut nw, mut unpainted) = (0, 0);
    let mut seen = BTreeSet::new();
    for g in wear {
        let class = g["wearable"].as_str().unwrap();
        let masked = g["masked"].as_bool().unwrap_or(false);
        let pattern = g["pattern"].as_u64().unwrap() as usize;
        let cols: Vec<[f32; 3]> = g["colors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| [0, 1, 2].map(|k| c[k].as_f64().unwrap() as f32))
            .collect();
        let key = format!("{class}#{pattern}#{masked}#{cols:?}");
        if !seen.insert(key) {
            continue;
        }
        let m = r.wearable_from_class(class, pattern, Some([cols[0], cols[1], cols[2]]), masked);
        if g["painted"] != true {
            unpainted += 1;
            if m.is_some() {
                gaps.push(format!("{class}: albedo not exported for Godot (keeps the mesh material), painted from the paks"));
            }
            continue;
        }
        nw += 1;
        match m {
            None => diffs.push(format!("{class}: no wearable material from the paks")),
            Some(m) => diff_desc(class, &m, g, &mut diffs),
        }
    }
    println!("  {n} part materials ({} weapons), {nw} distinct wearable materials ({unpainted} unpainted in Godot), {} differences",
        d["weapons"], diffs.len());
    println!("  {paks_only} slot materials Godot has no export for, resolved from the paks; unresolved: {unresolved:?}");
    println!("  white: {white:?}");
    for x in &notes {
        println!("  NOTE {x}");
    }
    for x in &gaps {
        println!("  EXPORT GAP {x}");
    }
    for x in &diffs {
        println!("  DIFF {x}");
    }
    // the Godot builders can only build materials whose JSON mdx exported: 10 part materials, 36 distinct wearables
    assert!(n >= 10 && nw >= 30, "{n} parts, {nw} wearables");
    assert!(unresolved.is_empty(), "{} slot materials not resolved from the paks", unresolved.len());
    assert!(diffs.is_empty(), "{} differences (listed above)", diffs.len());
}

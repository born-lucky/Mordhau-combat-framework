//! Colour tables and wearable colours from the paks (mh_assets::cosmetics) against the sheets builder's spec
//! (data_gen/spec/entities/color.json, `UeWearable.color`) and the Godot character builder's resolved colours for every
//! DefaultProfiles wearable (data_gen/materials/characters_godot.json, godot/tools/export_char_materials.gd), and
//! the fully data-driven wearable material (colours from the tables) = Godot's.

use mh_assets::cosmetics::{profiles, wearable_color_setup, wearable_colors, ColorTables};
use mh_assets::material::Resolver;
use mh_assets::pak_source::PakSource;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

fn data(p: &str) -> Value {
    let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").join(p);
    serde_json::from_slice(&std::fs::read(&f).unwrap_or_else(|_| panic!("{} missing", f.display()))).unwrap()
}

#[test]
fn color_tables_equal_spec() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount")));
    let ct = ColorTables::read(&rd);
    let spec = data("data_gen/spec/entities/color.json");
    let mut n = 0;
    for (_, e) in spec.as_object().unwrap() {
        let v = &e["values"];
        let (t, i) = (v["FLD_COLOR_TABLE"].as_i64().unwrap(), v["FLD_COLOR_ENTRY"].as_i64().unwrap());
        let want: Vec<f64> = v["FLD_COLOR_COLOR"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let got = ct.color(t, i);
        for k in 0..4 {
            assert!((got[k] - want[k]).abs() < 1e-6, "T{t}_{i}: {got:?} vs {want:?}");
        }
        n += 1;
    }
    let total: usize = ct.tables.iter().map(|t| t.len()).sum();
    println!("  {} tables, {total} entries, {n} = spec", ct.tables.len());
    assert_eq!(n, total);
}

#[test]
fn profile_wearable_colors_equal_godot() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("mount"));
    let rd = mh_pak::Reader::new(vfs.clone());
    let src = PakSource::new(vfs);
    let r = Resolver::new(&rd, &src);
    let ct = ColorTables::read(&rd);
    let profs = profiles(&rd, "DefaultProfiles");
    let d = data("data_gen/materials/characters_godot.json");
    let (mut n, mut mats) = (0, 0);
    let mut diffs = Vec::new();
    for g in d["wearables"].as_array().unwrap() {
        let pn = g["profile"].as_str().unwrap();
        let slot = g["slot"].as_u64().unwrap() as usize;
        let class = g["wearable"].as_str().unwrap();
        let Some(p) = profs.iter().find(|p| p.name == pn) else { panic!("profile {pn} not in the paks") };
        let (tables, use_slot) = wearable_color_setup(&rd, class);
        let cols = wearable_colors(&ct, &tables, use_slot, slot, &p.colors);
        let want: Vec<Vec<f64>> = g["colors"].as_array().unwrap().iter().map(|c| c.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()).collect();
        for k in 0..3 {
            for c in 0..3 {
                if (cols[k][c] as f64 - want[k][c]).abs() > 1e-6 {
                    diffs.push(format!("{pn} slot {slot} {class}: colour {k} {:?} vs {:?}", cols[k], want[k]));
                }
            }
        }
        assert_eq!(g["pattern"].as_i64().unwrap(), p.patterns[slot], "{pn} slot {slot} pattern");
        n += 1;
        // the whole material from data: pattern + colours from the loadout, textures from the class
        if g["painted"] == true {
            let m = r.wearable_from_class(class, p.patterns[slot] as usize, Some(cols), g["masked"].as_bool().unwrap_or(false)).unwrap();
            for (k, gv) in g["uniforms"].as_object().unwrap() {
                if let Some(mh_assets::material::Uniform::V3(v)) = m.uniforms.get(k) {
                    let w: Vec<f64> = gv.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
                    if (0..3).any(|i| (v[i] as f64 - w[i]).abs() > 1e-5) {
                        diffs.push(format!("{class}: {k} {v:?} vs {w:?}"));
                    }
                }
            }
            mats += 1;
        }
    }
    println!("  {n} profile wearables, {mats} painted materials, {} differences", diffs.len());
    for x in &diffs {
        println!("  DIFF {x}");
    }
    assert!(n > 50 && diffs.is_empty());
}

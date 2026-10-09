//! UMG trees and fonts from the paks (mh_assets::umg, fonts) against the Godot port's UMG reader (umg.gd), dumped by
//! godot/tools/export_umg.gd into data_gen/ui/umg_godot.json for the HUD and main-menu WidgetBlueprints the Godot views
//! draw: every widget umg.gd indexes, its CanvasPanelSlot rect, padding, text / colour (sRGB, as umg.gd draws) / font
//! px, brushes, image colour, progress and button styles, every MovieScene's last tick, and ui_scale. Fonts: the
//! .ufont bytes = extract/raw (the Godot json backend's copy), metrics from the font tables.

use mh_assets::fonts;
use mh_assets::umg::{self, srgb, Brush, UmgPackage};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn arr(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()
}

fn near(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol * (1.0 + y.abs()))
}

fn brush_eq(b: &Brush, g: &Value) -> bool {
    near(&b.image_size, &arr(&g["size"]), 1e-6)
        && near(&srgb(b.tint), &arr(&g["tint"]), 1e-5)
        && b.resource.as_ref().map_or("", |r| r.0.as_str()) == g["resource"].as_str().unwrap()
}

#[test]
fn umg_equals_godot_reader() {
    let p = root().join("data_gen/ui/umg_godot.json");
    let Ok(raw) = std::fs::read(&p) else { panic!("{} missing: run godot/tools/export_umg.gd", p.display()) };
    let d: Value = serde_json::from_slice(&raw).unwrap();
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount")));
    let parent: [f64; 4] = arr(&d["parent"]).try_into().unwrap();
    let desired: [f64; 2] = arr(&d["desired"]).try_into().unwrap();
    let mut diffs = Vec::new();
    let (mut nw, mut tree_n) = (0, 0);
    for (pkg, g) in d["packages"].as_object().unwrap() {
        let u = UmgPackage::read(&rd, pkg).unwrap_or_else(|| panic!("{pkg}: not read"));
        if let Some(r) = &u.root {
            let mut all = Vec::new();
            r.walk(&mut all);
            tree_n += all.len();
        }
        for (name, w) in g["widgets"].as_object().unwrap() {
            nw += 1;
            let mut bad = |what: &str, ok: bool| {
                if !ok {
                    diffs.push(format!("{pkg} {name}: {what}"));
                }
            };
            bad("class", u.class(name) == w["type"].as_str().unwrap());
            let slot = u.slot(name);
            bad("slot class", slot.as_ref().map_or("", |s| s.class.as_str()) == w["slot"].as_str().unwrap());
            bad("padding", near(&slot.as_ref().map_or([0.0; 4], |s| s.padding()), &arr(&w["padding"]), 1e-6));
            if let Some(c) = w.get("canvas") {
                let r = slot.as_ref().unwrap().canvas().rect(parent, desired);
                bad(&format!("canvas {r:?} vs {c}"), near(&r, &arr(c), 1e-5));
            }
            if w.get("text").is_some() {
                let t = u.text_style(name).unwrap();
                bad("text", t.text == w["text"].as_str().unwrap());
                bad("colour", near(&srgb(t.color), &arr(&w["color"]), 1e-5));
                bad("shadow", near(&srgb(t.shadow), &arr(&w["shadow"]), 1e-5));
                let f = t.font.as_ref().unwrap();
                bad("font px", fonts::font_px(f.size, 1.0) == w["font_px"].as_i64().unwrap());
                bad("typeface", f.typeface == w["typeface"].as_str().unwrap());
            }
            if let Some(b) = w.get("brush") {
                bad("brush", brush_eq(&u.brush(name), b));
                bad("image colour", near(&srgb(u.image_color(name)), &arr(&w["image_color"]), 1e-5));
            }
            if let Some(pg) = w.get("progress") {
                let (a, b) = u.progress_tints(name);
                bad("progress", near(&srgb(a), &arr(&pg[0]), 1e-5) && near(&srgb(b), &arr(&pg[1]), 1e-5));
            }
            if let Some(bt) = w.get("button") {
                let bs = u.button_brushes(name);
                bad("button", (0..3).all(|i| brush_eq(&bs[i], &bt[i])));
            }
        }
        for (ms, t) in g["movie_scenes"].as_object().unwrap() {
            let mine = u.movie_scene_last_tick(ms);
            if (mine - t.as_f64().unwrap()).abs() > 1e-9 {
                diffs.push(format!("{pkg} MovieScene {ms}: {mine} vs {t}"));
            }
        }
    }
    let keys = umg::ui_scale_keys(&rd);
    for (k, v) in d["ui_scale"].as_object().unwrap() {
        let (w, h) = k.split_once('x').unwrap();
        let s = umg::ui_scale(&keys, [w.parse().unwrap(), h.parse().unwrap()]);
        if (s - v.as_f64().unwrap()).abs() > 1e-6 {
            diffs.push(format!("ui_scale {k}: {s} vs {v}"));
        }
    }
    println!("  {} packages, {nw} widgets compared, {tree_n} widgets in the trees, {} differences", d["packages"].as_object().unwrap().len(), diffs.len());
    for x in &diffs {
        println!("  DIFF {x}");
    }
    assert!(nw > 200 && diffs.is_empty());
}

/// The HUD tree: BP_StatusBar's root holds the Health / Stamina overlays under bars_Canvas with their canvas slots
#[test]
fn status_bar_tree() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount")));
    let u = UmgPackage::read(&rd, "Mordhau/Content/Mordhau/UI/BP_StatusBar").unwrap();
    let r = u.root.as_ref().expect("root");
    let bars = r.find("bars_Canvas").expect("bars_Canvas");
    let names: Vec<&str> = bars.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Health", "Stamina"]);
    let st = bars.children[1].slot.as_ref().unwrap().canvas();
    assert_eq!(st.offsets, [-372.33334, -15.0, 321.75, 28.0]); // the shortest float32 decimal, as mh-pak and extract/json write it
    println!("  {} animations: {:?}", u.animations.len(), u.animations.iter().map(|a| (&a.name, a.last_tick, a.tracks.len())).collect::<Vec<_>>());
}

#[test]
fn mordhau_font_faces() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount")));
    let cf = fonts::composite(&rd, "Mordhau/Content/Mordhau/UI/UIAssets/Fonts/MordhauFont").expect("MordhauFont");
    assert!(cf.default.len() >= 7 && !cf.sub.is_empty());
    assert!(cf.sub_for(0xAC00).is_some(), "Korean range");
    for e in &cf.default {
        let f = fonts::face(&rd, &e.face).unwrap_or_else(|| panic!("{}: no face", e.face));
        let m = &f.metrics;
        assert!(m.units_per_em >= 16 && m.ascender > 0 && m.descender <= 0 && m.num_glyphs > 0, "{}: {m:?}", e.name);
        let raw = root().join(format!("extract/raw/{}.ufont", f.package));
        if raw.exists() {
            assert!(std::fs::read(&raw).unwrap() == f.data, "{}: .ufont bytes differ from extract/raw", f.package);
        }
        println!("  {} -> {} ({}, {} glyphs, {} upem, asc {} desc {})", e.name, f.package.rsplit('/').next().unwrap(), m.family, m.num_glyphs, m.units_per_em, m.ascender, m.descender);
    }
    assert_eq!(cf.face("Cinzel Bold").unwrap().face, "Mordhau/Content/Mordhau/UI/UIAssets/Fonts/Cinzel-Bold_new");
    assert_eq!(fonts::font_px(12.0, 1.0), 16);
}

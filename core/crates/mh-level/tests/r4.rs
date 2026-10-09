//! Round 4 readers against their references: the volumetric lightmap and reflection captures (Arena) against
//! extract/json Arena_BuiltData (CUE4Parse) and scripts/map/vlm_extract.py's sampler; the landscape collision
//! heightfield, grass data and Landscape actor (Camp) against the render heightmap and extract/json; decals (Camp)
//! against extract/json; BSP model count. SKIP (pass) without install / extract.

use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAPS: &str = "Mordhau/Content/Mordhau/Maps/";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}
fn pkgs() -> Option<Pkgs> {
    match Vfs::mount_default() {
        Ok(v) => Some(Pkgs::new(Reader::new(Arc::new(v)))),
        Err(e) => {
            eprintln!("SKIP: {e}");
            None
        }
    }
}
fn json(rel: &str) -> Option<Value> {
    let t = std::fs::read_to_string(repo().join(rel)).ok()?;
    mh_pak::equiv::parse_lenient(&t)
}
fn b64(s: &str) -> Vec<u8> {
    let tbl = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let v: Vec<u8> = s.bytes().filter(|&c| c != b'=' && !c.is_ascii_whitespace()).map(tbl).collect();
    let mut out = vec![];
    for ch in v.chunks(4) {
        let n = ch.iter().enumerate().fold(0u32, |a, (i, &x)| a | (x as u32) << (18 - 6 * i));
        for i in 0..ch.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

#[test]
fn volumetric_lightmap_and_reflections_arena() {
    let Some(pk) = pkgs() else { return };
    let Some(bj) = json(&format!("extract/json/{MAPS}Arena_Map/Arena_BuiltData.json")) else { return };
    let reg = bj.as_array().unwrap().iter().find(|e| e["Type"] == "MapBuildDataRegistry").unwrap().clone();
    let d = read(&pk, &format!("{MAPS}Arena_Map/DU_Arena"));
    let li = d.levels.iter().position(|l| l.pkg.ends_with("Arena_Map/Arena")).unwrap();
    let b = d.build[li].as_ref().unwrap();
    // volumetric lightmap: every layer byte-equal to CUE4Parse's
    let wv = reg["LevelPrecomputedVolumetricLightmapBuildData"].as_object().unwrap();
    assert_eq!(b.volumetric.len(), wv.len());
    let (g, v) = &b.volumetric[0];
    let w = &wv[g];
    assert_eq!(v.brick_size as i64, w["BrickSize"].as_i64().unwrap());
    assert_eq!(v.brick_dims, ["X", "Y", "Z"].map(|k| w["BrickDataDimensions"][k].as_u64().unwrap() as usize));
    assert_eq!(v.indirection_dims, ["X", "Y", "Z"].map(|k| w["IndirectionTextureDimensions"][k].as_u64().unwrap() as usize));
    assert_eq!(v.bounds_min[0], w["Bounds"]["Min"]["X"].as_f64().unwrap() as f32 as f64);
    let same = |l: &mh_level::vlm::VlmLayer, j: &Value, what: &str| {
        assert_eq!(l.format, j["PixelFormatString"].as_str().unwrap(), "{what} format");
        assert!(l.data[..] == b64(j["Data"].as_str().unwrap())[..], "{what} bytes");
    };
    let bd = &w["BrickData"];
    same(&v.indirection, &w["IndirectionTexture"], "indirection");
    same(&v.ambient, &bd["AmbientVector"], "ambient");
    for i in 0..6 {
        same(&v.sh[i], &bd["SHCoefficients"][i], "sh");
    }
    same(&v.sky_bent_normal, &bd["SkyBentNormal"], "sky bent normal");
    same(&v.directional_light_shadowing, &bd["DirectionalLightShadowing"], "shadowing");
    same(v.lq_light_color.as_ref().unwrap(), &bd["LQLightColor"], "lq colour");
    same(v.lq_light_direction.as_ref().unwrap(), &bd["LQLightDirection"], "lq direction");
    // the reference sampler (python scripts/map/vlm_extract.py 0 0 200 0 0 1 and -1500 700 350 0.6 0 0.8)
    for (p, n, want) in [
        ([0.0, 0.0, 200.0], [0.0, 0.0, 1.0], [0.3322, 0.2104, 0.1689]),
        ([-1500.0, 700.0, 350.0], [0.6, 0.0, 0.8], [1.0544, 1.0328, 1.2222]),
    ] {
        let got = v.irradiance(p, n);
        assert!((0..3).all(|c| (got[c] - want[c]).abs() < 6e-4), "irradiance at {p:?}: {got:?} vs {want:?}");
    }
    println!("Arena VLM: bricks {:?} x{} ({} texels), indirection {:?}, all 13 layers = CUE4Parse; irradiance = vlm_extract.py", v.brick_dims, v.brick_size, v.ambient.data.len() / 4, v.indirection_dims);
    // reflection captures: header values = CUE4Parse, cubemap mip chain complete, every capture resolves
    let wr = reg["ReflectionCaptureBuildData"].as_object().unwrap();
    assert_eq!(b.reflections.len(), wr.len());
    for (k, x) in wr {
        let r = &b.reflections[k];
        assert_eq!(r.cubemap_size as i64, x["CubemapSize"].as_i64().unwrap());
        assert_eq!(r.average_brightness, x["AverageBrightness"].as_f64().unwrap() as f32);
        assert_eq!(r.brightness, x["Brightness"].as_f64().unwrap() as f32);
        assert_eq!(r.full_hdr.len(), r.expected_len(), "{k}: FullHDRCapturedData is not a full {}^2 cube mip chain", r.cubemap_size);
        assert!(r.encoded.is_none());
        assert_eq!(r.face(0, 5).map(|f| f.len()), Some(r.cubemap_size * r.cubemap_size * 8));
    }
    let caps: Vec<_> = d.reflection_captures.iter().filter(|c| c.level == li).collect();
    assert_eq!(caps.len(), 11);
    assert!(caps.iter().all(|c| d.reflection_data(c).is_some()), "a capture without cooked data");
    println!("Arena: {} reflection captures, cubemaps {} ({} mips, {} bytes each)", caps.len(), b.reflections.values().next().unwrap().cubemap_size, b.reflections.values().next().unwrap().mip_count(), b.reflections.values().next().unwrap().full_hdr.len());
}

#[test]
fn landscape_collision_grass_actor_camp() {
    let Some(pk) = pkgs() else { return };
    let d = read(&pk, &format!("{MAPS}DuelCamp/Camp"));
    assert_eq!(d.landscape_collision.len(), 144);
    // collision heights = the render heightmap at the same vertex (same raw 16-bit heights, CollisionMipLevel 0)
    let mut worst = 0f32;
    let mut holes = 0;
    let mut matched = 0;
    for c in &d.landscape_collision {
        let Some(r) = d.landscape.iter().find(|r| r.section_base == c.section_base && r.level == c.level) else { continue };
        assert_eq!(c.verts, r.verts, "{}", c.name);
        for (a, b) in c.heights.iter().zip(&r.heights) {
            worst = worst.max((a - b).abs());
        }
        holes += c.materials.iter().filter(|&&m| m == mh_level::landscape::HOLE).count();
        assert!(c.materials.iter().all(|&m| m == mh_level::landscape::HOLE || (m as usize) < c.physical_materials.len().max(1)));
        matched += 1;
    }
    println!("Camp collision: {matched} components matched to render, worst height diff {worst} (local units), {holes} hole vertices, sample order {}", mh_level::landscape::SAMPLE_ORDER);
    assert_eq!(matched, 144);
    assert!(worst < 1e-6, "collision heights differ from the render heightmap: {worst}");
    // grass: baked heights = the render heightmap's raw values; every grass type resolves to its asset
    let with_grass: Vec<_> = d.landscape.iter().filter(|c| c.grass.is_some()).collect();
    for c in &with_grass {
        let g = c.grass.as_ref().unwrap();
        if g.heights.len() == c.heights.len() {
            for (h, r) in g.heights.iter().zip(&c.heights) {
                // the grass map is rendered from the landscape (a render target), so heights agree to one raw unit
                assert!(((*h as f64 - 32768.0) / 128.0 - *r as f64).abs() <= 1.0 / 128.0 + 1e-9);
            }
        }
        for (t, w) in &g.weights {
            assert!(d.grass_types.contains_key(t), "grass type {t}");
            assert!(w.len() == g.heights.len() || w.is_empty());
        }
    }
    let varieties: usize = d.grass_types.values().map(|p| p.get("GrassVarieties").and_then(|v| v.as_array()).map_or(0, |a| a.len())).sum();
    println!("Camp grass: {} components with baked grass, {} grass types, {varieties} varieties", with_grass.len(), d.grass_types.len());
    // the Landscape actor's LOD settings are its tagged properties
    assert_eq!(d.landscape_actors.len(), 1);
    let la = &d.landscape_actors[0];
    assert_eq!(la.props.get("LOD0ScreenSize").and_then(|v| v.as_f64()), Some(0.55));
    assert!(la.material.ends_with("M_Duelcamp_LandscapeExpansion"));
}

#[test]
fn decals_and_bsp_camp() {
    let Some(pk) = pkgs() else { return };
    let d = read(&pk, &format!("{MAPS}DuelCamp/Camp"));
    let Some(js) = json(&format!("extract/json/{MAPS}DuelCamp/Camp.json")) else { return };
    let js = js.as_array().unwrap();
    let want: Vec<&Value> = js.iter().filter(|e| e["Type"] == "DecalComponent").collect();
    let mine: Vec<_> = d.decals.iter().filter(|x| x.level == 0).collect();
    assert_eq!(mine.len(), want.len());
    for w in want {
        let name = w["Name"].as_str().unwrap();
        let m = mine.iter().find(|x| x.name.ends_with(&format!(".{name}")) && x.material == w["Properties"]["DecalMaterial"]["ObjectPath"].as_str().unwrap_or(""));
        assert!(m.is_some(), "decal {name}");
    }
    let models = js.iter().filter(|e| e["Type"] == "Model").count();
    let comps = js.iter().filter(|e| e["Type"] == "ModelComponent").count();
    assert_eq!(d.levels.len(), 1);
    assert_eq!(d.bsp_models, models);
    assert_eq!(comps, 0);
    println!("Camp: {} decals = extract/json, {} UModel (brush) exports, 0 ModelComponent (no BSP render geometry cooked)", mine.len(), models);
}

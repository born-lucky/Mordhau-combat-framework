//! Volumetric lightmap GPU data + CPU reference (mh_assets::vlm) over mh-level's registry decode (Vlm::from_level of
//! mh_level::read(map).volumetric_lightmap(), from the paks). Checked, for every map the Godot port lights with a VLM:
//! the layers = CUE4Parse's decode (extract/json/<level>_BuiltData.json, base64), the GPU data = the Godot port's VLM
//! files (extract/gltf/<..>_VLM/, scripts/map/vlm_extract.py), and irradiance = vlm_extract.py's reference sampler.

use mh_assets::vlm::{Layer, Vlm};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn extract() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extract")
}

fn b64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    };
    let b: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(b.len() / 4 * 3);
    for q in b.chunks(4) {
        let pad = q.iter().filter(|&&c| c == b'=').count();
        let n = q.iter().fold(0u32, |a, &c| (a << 6) | val(c)) << (6 * (4 - q.len()));
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad.min(2)]);
    }
    out
}


/// A `Vlm` from CUE4Parse's JSON of `<level>_BuiltData`
fn vlm_from_json(pkg: &str) -> Vlm {
    let d: Value = serde_json::from_slice(&std::fs::read(extract().join("json").join(format!("{pkg}.json"))).unwrap()).unwrap();
    let reg = d.as_array().unwrap().iter().find(|e| e["Type"] == "MapBuildDataRegistry").unwrap();
    let j = reg["LevelPrecomputedVolumetricLightmapBuildData"].as_object().unwrap().values().next().unwrap().clone();
    let l = |w: &Value| Layer { data: b64(w["Data"].as_str().unwrap()), format: w["PixelFormatString"].as_str().unwrap().into() };
    let iv = |w: &Value| ["X", "Y", "Z"].map(|k| w[k].as_i64().unwrap() as i32);
    let fv = |w: &Value| ["X", "Y", "Z"].map(|k| w[k].as_f64().unwrap() as f32);
    let bd = &j["BrickData"];
    Vlm {
        bounds_min: fv(&j["Bounds"]["Min"]),
        bounds_max: fv(&j["Bounds"]["Max"]),
        indirection_dims: iv(&j["IndirectionTextureDimensions"]),
        indirection: l(&j["IndirectionTexture"]),
        brick_size: j["BrickSize"].as_i64().unwrap() as i32,
        brick_dims: iv(&j["BrickDataDimensions"]),
        ambient: l(&bd["AmbientVector"]),
        sh: (0..6).map(|k| l(&bd["SHCoefficients"][k])).collect(),
        sky_bent_normal: l(&bd["SkyBentNormal"]),
        directional_light_shadowing: l(&bd["DirectionalLightShadowing"]),
        lq_light_color: l(&bd["LQLightColor"]),
        lq_light_direction: l(&bd["LQLightDirection"]),
        sub_level_brick_positions: vec![],
        indirection_original: vec![],
    }
}

/// (map, the BuiltData package of the level holding its VLM)
const MAPS: &[(&str, &str)] = &[
    ("Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena", "Mordhau/Content/Mordhau/Maps/Arena_Map/Arena_BuiltData"),
    ("Mordhau/Content/Mordhau/Maps/Contraband/DU_Contraband", "Mordhau/Content/Mordhau/Maps/Contraband/Contraband_BuiltData"),
    ("Mordhau/Content/Mordhau/Maps/Cortile/DU_Cortile", "Mordhau/Content/Mordhau/Maps/Cortile/Cortile_Map_BuiltData"),
    ("Mordhau/Content/Mordhau/Maps/Highlands/DU_Highlands", "Mordhau/Content/Mordhau/Maps/Highlands/HighlandsRanked_BuiltData"),
    ("Mordhau/Content/Mordhau/Maps/Truce/DU_Truce", "Mordhau/Content/Mordhau/Maps/Truce/Truce_BuiltData"),
];

/// From mh-level's decode of the level's MapBuildDataRegistry (mh_level::vlm::VolumetricLightmap, rust-pak r4;
/// `mh_level::read(..).volumetric_lightmap()`). Bounds narrowed to f32 as UE stores them (FBox of floats).
fn from_level(v: &mh_level::vlm::VolumetricLightmap) -> Vlm {
    let l = |x: &mh_level::vlm::VlmLayer| Layer { data: x.data.to_vec(), format: x.format.clone() };
    let empty = Layer { data: vec![], format: String::new() };
    Vlm {
        bounds_min: v.bounds_min.map(|x| x as f32),
        bounds_max: v.bounds_max.map(|x| x as f32),
        indirection_dims: v.indirection_dims.map(|x| x as i32),
        indirection: l(&v.indirection),
        brick_size: v.brick_size as i32,
        brick_dims: v.brick_dims.map(|x| x as i32),
        ambient: l(&v.ambient),
        sh: v.sh.iter().map(l).collect(),
        sky_bent_normal: l(&v.sky_bent_normal),
        directional_light_shadowing: l(&v.directional_light_shadowing),
        lq_light_color: v.lq_light_color.as_ref().map_or(empty.clone(), l),
        lq_light_direction: v.lq_light_direction.as_ref().map_or(empty, l),
        sub_level_brick_positions: v.sub_level_brick_positions.clone(),
        indirection_original: v.indirection_original_values.clone(),
    }
}

fn from_paks(map: &str) -> Vlm {
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("mount"))));
    let d = mh_level::read(&pk, map);
    from_level(d.volumetric_lightmap().unwrap_or_else(|| panic!("{map}: no VLM from mh-level")))
}

#[test]
fn vlm_gpu_data_matches_godot_files() {
    for (map, pkg) in MAPS {
        let v = from_paks(map);
        let j = vlm_from_json(pkg);
        assert!(v.indirection == j.indirection && v.ambient == j.ambient && v.sh == j.sh && v.sky_bent_normal == j.sky_bent_normal
            && (v.bounds_min, v.bounds_max, v.indirection_dims, v.brick_dims, v.brick_size)
                == (j.bounds_min, j.bounds_max, j.indirection_dims, j.brick_dims, j.brick_size), "{map}: mh-level's VLM differs from CUE4Parse's");
        v.validate().unwrap_or_else(|e| panic!("{pkg}: {e}"));
        // the Godot port's vlm.json (UeVlm.uniforms input)
        let g: Value = serde_json::from_slice(&std::fs::read(extract().join("gltf").join(format!("{pkg}_VLM/vlm.json"))).unwrap()).unwrap();
        let gi = |k: &str| g[k].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect::<Vec<_>>();
        assert_eq!(gi("dims"), v.brick_dims.map(|x| x as f32).to_vec(), "{pkg} dims");
        assert_eq!(gi("ind"), v.indirection_dims.map(|x| x as f32).to_vec(), "{pkg} ind");
        assert_eq!(gi("min"), v.bounds_min.to_vec());
        assert_eq!(gi("max"), v.bounds_max.to_vec());
        assert_eq!(g["brick_size"].as_i64().unwrap() as i32, v.brick_size);
        // the Godot port's ambient texture (vlm_extract.py: R11G11B10 -> float16 RGBA, A = 1) equals ambient_rgba16f
        let amb = std::fs::read(extract().join("gltf").join(format!("{pkg}_VLM/ambient_rgbah.bin"))).unwrap();
        let mine: Vec<u8> = v.ambient_rgba16f().iter().flat_map(|h| h.to_le_bytes()).collect();
        assert!(amb == mine, "{pkg}: ambient half floats differ from vlm_extract.py's");
        for (k, n) in ["sh0r", "sh1r", "sh0g", "sh1g", "sh0b", "sh1b"].iter().enumerate() {
            let f = std::fs::read(extract().join("gltf").join(format!("{pkg}_VLM/{n}_rgba8.bin"))).unwrap();
            assert!(f == v.sh_stack()[k * f.len()..(k + 1) * f.len()], "{pkg}: SH layer {n} differs");
        }
        println!("  {}: ind {:?}, bricks {:?}", pkg.rsplit('/').next().unwrap(), v.indirection_dims, v.brick_dims);
    }
}

/// Irradiance (before 1/pi) = scripts/map/vlm_extract.py's reference sampler (the game's shader 001 163-254), printed by
///   python scripts/map/vlm_extract.py 0 0 200 0 0 1            -> [0.3322 0.2104 0.1689]
///   python scripts/map/vlm_extract.py 1234.5 -800 350 0.6 0 0.8 -> [0.7161 0.3888 0.2517]
#[test]
fn vlm_irradiance_equals_reference_sampler() {
    let v = from_paks(MAPS[0].0);
    for (p, n, want) in [
        ([0.0, 0.0, 200.0], [0.0, 0.0, 1.0], [0.3322, 0.2104, 0.1689]),
        ([1234.5, -800.0, 350.0], [0.6, 0.0, 0.8], [0.7161, 0.3888, 0.2517]),
    ] {
        let got = v.irradiance(p, n);
        for k in 0..3 {
            assert!((got[k] - want[k]).abs() <= 1e-4, "{p:?} {n:?}: {got:?} vs {want:?}");
        }
    }
}

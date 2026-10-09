//! Round 3 readers against their references: landscape (Camp) against each component's serialized CachedLocalBox and
//! the extract/json component counts; spline meshes (Camp) against extract/json SplineParams; painted vertex colours
//! (Arena) against extract/json/<level>.vcolors.json (`mdx vcolors`, ue_vcolor.gd's input); lightmaps / shadowmaps
//! (Arena) against extract/json Arena_BuiltData MeshBuildData + LightBuildData (CUE4Parse); UDS values (Contraband)
//! against extract/gltf/<level>_UDS.json (scripts/shaders/uds_values.py). SKIP (pass) without install / extract.

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
fn digits(g: &str) -> String {
    g.replace('-', "")
}

#[test]
fn landscape_camp() {
    let Some(pk) = pkgs() else { return };
    let d = read(&pk, &format!("{MAPS}DuelCamp/Camp"));
    let n_json = json(&format!("extract/json/{MAPS}DuelCamp/Camp.json"))
        .and_then(|v| v.as_array().map(|a| a.iter().filter(|e| e["Type"] == "LandscapeComponent").count()));
    println!("Camp: {} landscape components (extract/json {:?})", d.landscape.len(), n_json);
    assert!(!d.landscape.is_empty());
    if let Some(n) = n_json {
        assert_eq!(d.landscape.len(), n);
    }
    let (mut worst, mut nlayers, mut lo, mut hi) = (0f64, 0usize, f64::INFINITY, f64::NEG_INFINITY);
    for c in &d.landscape {
        assert_eq!(c.heights.len(), c.verts * c.verts);
        let (mn, mx) = c.height_range();
        // CachedLocalBox Z = the component's height range in local units (ULandscapeComponent::UpdateCachedBounds)
        let (bmin, bmax) = c.cached_box.expect("CachedLocalBox");
        worst = worst.max((mn as f64 - bmin[2]).abs()).max((mx as f64 - bmax[2]).abs());
        assert_eq!((bmax[0], bmax[1]), (c.size_quads as f64, c.size_quads as f64));
        nlayers += c.layers.len();
        for l in &c.layers {
            assert_eq!(l.weights.len(), c.verts * c.verts, "{} {}", c.name, l.name);
        }
        for (x, y) in [(0, 0), (c.verts - 1, c.verts - 1)] {
            let p = c.vertex_world(x, y);
            lo = lo.min(p[2]);
            hi = hi.max(p[2]);
            let n = c.normal_local(x, y);
            assert!((n[0] * n[0] + n[1] * n[1] + n[2] * n[2] - 1.0).abs() < 0.02 + 1e-9 || n[2] == 0.0);
        }
    }
    println!("  heights vs CachedLocalBox worst {worst:.4} (1/128 = {:.4}); {nlayers} layer maps; world Z {lo:.0}..{hi:.0} cm", 1.0 / 128.0);
    assert!(worst <= 1.0 / 128.0 + 1e-6, "height range off CachedLocalBox by {worst}");
    // neighbouring components share their edge vertices (same heightmap texels): the seam is continuous
    let first = &d.landscape[0];
    if let Some(nb) = d.landscape.iter().find(|c| c.section_base == [first.section_base[0] + first.size_quads as i64, first.section_base[1]]) {
        let (n, v) = (first.verts, first.verts - 1);
        for y in 0..n {
            let (a, b) = (first.vertex_world(v, y), nb.vertex_world(0, y));
            assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-3), "seam at y {y}: {a:?} vs {b:?}");
        }
        println!("  seam {} | {} continuous", first.name, nb.name);
    }
}

#[test]
fn spline_meshes_camp() {
    let Some(pk) = pkgs() else { return };
    let d = read(&pk, &format!("{MAPS}DuelCamp/Camp"));
    let Some(js) = json(&format!("extract/json/{MAPS}DuelCamp/Camp.json")) else { return };
    let js = js.as_array().unwrap();
    let want: Vec<&Value> = js.iter().filter(|e| e["Type"] == "SplineMeshComponent").collect();
    println!("Camp: {} spline meshes (extract/json {} components)", d.splines.len(), want.len());
    assert_eq!(d.splines.len(), want.len());
    for e in want {
        let sp = &e["Properties"]["SplineParams"];
        let name = e["Name"].as_str().unwrap();
        let m = d.splines.iter().find(|s| s.name.ends_with(&format!(".{name}")) && s.end_pos[0] == sp["EndPos"]["X"].as_f64().unwrap_or(100.0)).unwrap_or_else(|| panic!("no spline {name}"));
        assert_eq!(m.start_pos[0], sp["StartPos"]["X"].as_f64().unwrap_or(0.0));
        assert_eq!(m.start_tangent[1], sp["StartTangent"]["Y"].as_f64().unwrap_or(0.0));
        assert_eq!(m.end_tangent[2], sp["EndTangent"]["Z"].as_f64().unwrap_or(0.0));
        assert_eq!(m.start_scale[0], sp["StartScale"]["X"].as_f64().unwrap_or(1.0));
        assert!(m.mesh.ends_with(e["Properties"]["StaticMesh"]["ObjectPath"].as_str().unwrap().rsplit('/').next().unwrap()));
    }
}

#[test]
fn vertex_colours_arena() {
    let Some(pk) = pkgs() else { return };
    let Some(vc) = json(&format!("extract/json/{MAPS}Arena_Map/Arena.vcolors.json")) else { return };
    let d = read(&pk, &format!("{MAPS}Arena_Map/DU_Arena"));
    let vc = vc.as_object().unwrap();
    let mut n = 0;
    for (k, r) in vc {
        let m = d.meshes.iter().find(|m| &m.name == k).unwrap_or_else(|| panic!("no placement {k}"));
        let c = m.lods.first().and_then(|l| l.override_colors.as_ref()).unwrap_or_else(|| panic!("{k}: no override colours"));
        let rgba = b64(r["rgba"].as_str().unwrap());
        assert_eq!(c.len(), r["n"].as_u64().unwrap() as usize, "{k} vertex count");
        let mine: Vec<u8> = c.iter().flatten().copied().collect();
        assert!(mine == rgba, "{k}: colours differ");
        n += 1;
    }
    let painted = d.meshes.iter().filter(|m| m.lods.first().is_some_and(|l| l.override_colors.is_some())).count();
    println!("Arena: {n} painted components == vcolors.json ({painted} with colours in DU_Arena's levels)");
    assert_eq!(painted, vc.len());
}

#[test]
fn lightmaps_arena() {
    let Some(pk) = pkgs() else { return };
    let Some(bj) = json(&format!("extract/json/{MAPS}Arena_Map/Arena_BuiltData.json")) else { return };
    let reg = bj.as_array().unwrap().iter().find(|e| e["Type"] == "MapBuildDataRegistry").unwrap().clone();
    let d = read(&pk, &format!("{MAPS}Arena_Map/DU_Arena"));
    let li = d.levels.iter().position(|l| l.pkg.ends_with("Arena_Map/Arena")).unwrap();
    let b = d.build[li].as_ref().expect("Arena build data decoded");
    let want = reg["MeshBuildData"].as_object().unwrap();
    println!("Arena_BuiltData: {} mesh build entries (json {}), vt indices {}, {} light channels (json {})", b.meshes.len(), want.len(), b.vt_indices, b.light_channels.len(), reg["LightBuildData"].as_object().map_or(0, |m| m.len()));
    assert_eq!(b.meshes.len(), want.len());
    let f = |v: &Value| v.as_f64().unwrap_or(f64::NAN) as f32;
    for (k, w) in want {
        let m = b.meshes.get(k).unwrap_or_else(|| panic!("no entry {k}"));
        match (&m.light_map, w.get("LightMap").filter(|v| !v.is_null())) {
            (Some(l), Some(wl)) => {
                assert_eq!(l.coordinate_scale, [f(&wl["CoordinateScale"]["X"]), f(&wl["CoordinateScale"]["Y"])], "{k}");
                assert_eq!(l.coordinate_bias, [f(&wl["CoordinateBias"]["X"]), f(&wl["CoordinateBias"]["Y"])], "{k}");
                for i in 0..4 {
                    for (j, c) in ["X", "Y", "Z", "W"].iter().enumerate() {
                        assert_eq!(l.scale_vectors[i][j], f(&wl["ScaleVectors"][i][c]), "{k} scale {i}");
                        assert_eq!(l.add_vectors[i][j], f(&wl["AddVectors"][i][c]), "{k} add {i}");
                    }
                }
                let tn = wl["Textures"][0]["ObjectName"].as_str().unwrap_or("");
                assert!(tn.ends_with(&format!(":{}'", l.textures[0].as_ref().unwrap().name)), "{k} {tn}");
                assert_eq!(l.light_guids.len(), wl["LightGuids"].as_array().map_or(0, |a| a.len()));
            }
            (None, None) => {}
            (a, b) => panic!("{k}: lightmap {:?} vs {:?}", a.is_some(), b.is_some()),
        }
        match (&m.shadow_map, w.get("ShadowMap").filter(|v| !v.is_null())) {
            (Some(s), Some(ws)) => {
                assert_eq!(s.coordinate_scale, [f(&ws["CoordinateScale"]["X"]), f(&ws["CoordinateScale"]["Y"])], "{k}");
                assert_eq!(s.light_guids.iter().map(|g| g.as_str()).collect::<Vec<_>>(), ws["LightGuids"].as_array().unwrap().iter().map(|g| digits(g.as_str().unwrap())).collect::<Vec<_>>().iter().map(|s| s.as_str()).collect::<Vec<_>>());
            }
            (None, None) => {}
            (a, b) => panic!("{k}: shadowmap {:?} vs {:?}", a.is_some(), b.is_some()),
        }
        assert_eq!(m.per_instance.len(), w.get("PerInstanceLightmapData").and_then(|v| v.as_array()).map_or(0, |a| a.len()), "{k}");
    }
    if let Some(lb) = reg["LightBuildData"].as_object() {
        assert_eq!(b.light_channels.len(), lb.len());
        for (g, v) in lb {
            assert_eq!(b.light_channels.get(g).copied(), v["ShadowMapChannel"].as_i64().map(|x| x as i32), "light {g}");
        }
    }
    // placements find their entry through LOD 0's MapBuildDataId
    let lit = d.meshes.iter().filter(|m| m.level == li && d.build_data(m.level, &m.lods).is_some_and(|b| b.light_map.is_some())).count();
    println!("  {lit} Arena placements resolve a lightmap");
    assert!(lit > 100);
}

#[test]
fn uds_contraband() {
    let Some(pk) = pkgs() else { return };
    let Some(want) = json(&format!("extract/gltf/{MAPS}Contraband/Contraband_UDS.json")) else { return };
    let d = read(&pk, &format!("{MAPS}Contraband/DU_Contraband"));
    let u = d.uds.as_ref().expect("UDS");
    assert_eq!(u.mic, want["mic"].as_str().unwrap());
    assert_eq!(u.mid, want["mid"].as_str().unwrap());
    for (k, v) in want["scalars"].as_object().unwrap() {
        assert_eq!(u.scalars.get(k).copied(), v.as_f64(), "scalar {k}");
    }
    for (k, v) in want["vectors"].as_object().unwrap() {
        let w: Vec<f64> = v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        assert_eq!(u.vectors.get(k).map(|a| a.to_vec()), Some(w), "vector {k}");
    }
    assert_eq!(u.scalars.len(), want["scalars"].as_object().unwrap().len());
    let al = &want["actor_location"];
    assert_eq!(u.actor_location, [al["X"].as_f64().unwrap(), al["Y"].as_f64().unwrap(), al["Z"].as_f64().unwrap()]);
    println!("Contraband UDS: {} scalars, {} vectors, {} actor props, mic {}", u.scalars.len(), u.vectors.len(), u.props.len(), u.mic);
}

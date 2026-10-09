//! equiv.rs - the `equiv_assets` oracle verb: with a level loaded from extract's glb/PNG (`--assets gltf`), decode the
//! same meshes and textures from the paks with mh-assets (paksrc.rs, the `--assets pak` path) and compare them
//! (evidence assets_equiv.json). The glb/PNG files are CUE4Parse exports of the same packages, so equal data means the
//! pak path renders what the extract path renders.

use crate::level::LevelState;
use crate::material::MatCache;
use crate::paksrc;
use crate::source::Source;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use serde_json::{json, Value};

fn f3(m: &Mesh, a: bevy::mesh::MeshVertexAttribute) -> Option<Vec<[f32; 3]>> {
    match m.attribute(a)? {
        VertexAttributeValues::Float32x3(v) => Some(v.clone()),
        _ => None,
    }
}
fn f4(m: &Mesh, a: bevy::mesh::MeshVertexAttribute) -> Option<Vec<[f32; 4]>> {
    match m.attribute(a)? {
        VertexAttributeValues::Float32x4(v) => Some(v.clone()),
        _ => None,
    }
}
fn f2(m: &Mesh, a: bevy::mesh::MeshVertexAttribute) -> Option<Vec<[f32; 2]>> {
    match m.attribute(a)? {
        VertexAttributeValues::Float32x2(v) => Some(v.clone()),
        _ => None,
    }
}
fn maxd<const N: usize>(a: &[[f32; N]], b: &[[f32; N]]) -> f32 {
    a.iter().zip(b).flat_map(|(x, y)| x.iter().zip(y).map(|(p, q)| (p - q).abs())).fold(0.0, f32::max)
}

pub fn equiv_assets(world: &mut World, limit: usize) -> Value {
    let Some(ps) = world.resource::<Source>().pak.clone() else { return json!({"error": "paks not mounted"}) };
    let lvl = world.resource::<LevelState>();
    let mut pkgs: Vec<(String, Vec<Handle<Mesh>>)> = lvl.gltf_meshes.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    pkgs.sort_by(|a, b| a.0.cmp(&b.0));
    if pkgs.is_empty() {
        return json!({"error": "no glb meshes loaded (run with --assets gltf)"});
    }
    let meshes = world.resource::<Assets<Mesh>>();
    let (mut n_pkg, mut n_prim, mut prim_count_ok, mut vert_ok, mut idx_ok) = (0, 0, 0, 0, 0);
    let (mut pos_max, mut nrm_max, mut uv_max) = (0f32, 0f32, 0f32);
    let (mut tw_agree, mut tw_total) = (0usize, 0usize);
    let mut used_eq = 0usize;
    let (mut tri_eq, mut tri_max) = (0usize, 0f32);
    let mut glb_w: std::collections::BTreeMap<String, usize> = Default::default();
    let mut bad: Vec<Value> = Vec::new();
    for (pkg, hs) in pkgs.iter().take(limit) {
        let dec = match paksrc::mesh(&ps, pkg, false) {
            Ok(d) => d,
            Err(e) => {
                bad.push(json!({"mesh": pkg, "error": e}));
                continue;
            }
        };
        n_pkg += 1;
        if dec.len() == hs.len() {
            prim_count_ok += 1;
        } else {
            bad.push(json!({"mesh": pkg, "glb_prims": hs.len(), "pak_sections": dec.len()}));
        }
        for (h, (pm, _)) in hs.iter().zip(dec.iter()) {
            let Some(gm) = meshes.get(h) else { continue };
            n_prim += 1;
            let (gv, pv) = (gm.count_vertices(), pm.count_vertices());
            let (gi, pi) = (gm.indices().map_or(0, |i| i.len()), pm.indices().map_or(0, |i| i.len()));
            idx_ok += (gi == pi) as usize;
            if gv != pv {
                // CUE4Parse's glTF writer keeps only the vertices the section's triangles use; count the decoded ones
                let used: std::collections::HashSet<usize> = pm.indices().map(|i| i.iter().collect()).unwrap_or_default();
                if used.len() == gv {
                    used_eq += 1;
                    continue;
                }
                // different vertex lists (the writer dedupes / reorders): compare the triangles' corner positions
                if let (Some(a), Some(b), Some(ia), Some(ib)) = (f3(gm, Mesh::ATTRIBUTE_POSITION), f3(pm, Mesh::ATTRIBUTE_POSITION), gm.indices(), pm.indices()) {
                    if ia.len() == ib.len() {
                        let d = ia.iter().zip(ib.iter()).map(|(x, y)| (0..3).map(|c| (a[x][c] - b[y][c]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
                        tri_max = tri_max.max(d);
                        if d < 1e-4 {
                            tri_eq += 1;
                            continue;
                        }
                    }
                }
                if bad.len() < 40 {
                    bad.push(json!({"mesh": pkg, "glb_vertices": gv, "pak_vertices": pv, "glb_indices": gi, "pak_indices": pi}));
                }
                continue;
            }
            vert_ok += 1;
            if let (Some(a), Some(b)) = (f3(gm, Mesh::ATTRIBUTE_POSITION), f3(pm, Mesh::ATTRIBUTE_POSITION)) {
                let d = maxd(&a, &b);
                if d > 1e-3 && bad.len() < 40 {
                    bad.push(json!({"mesh": pkg, "position_max_diff": d}));
                }
                pos_max = pos_max.max(d);
            }
            if let (Some(a), Some(b)) = (f3(gm, Mesh::ATTRIBUTE_NORMAL), f3(pm, Mesh::ATTRIBUTE_NORMAL)) {
                nrm_max = nrm_max.max(maxd(&a, &b));
            }
            if let (Some(a), Some(b)) = (f2(gm, Mesh::ATTRIBUTE_UV_0), f2(pm, Mesh::ATTRIBUTE_UV_0)) {
                uv_max = uv_max.max(maxd(&a, &b));
            }
            if let (Some(a), Some(b)) = (f4(gm, Mesh::ATTRIBUTE_TANGENT), f4(pm, Mesh::ATTRIBUTE_TANGENT)) {
                for (x, y) in a.iter().zip(b.iter()) {
                    *glb_w.entry(format!("{}", x[3])).or_default() += 1;
                    tw_total += 1;
                    tw_agree += (x[3].signum() == y[3].signum()) as usize;
                }
            }
        }
    }
    // textures: the PNGs the extract materials bound vs mip 0 decoded from the paks
    let cache = world.resource::<MatCache>();
    let stdm = world.resource::<Assets<StandardMaterial>>();
    let images = world.resource::<Assets<Image>>();
    let mut tex: Vec<(String, Handle<Image>, bool)> = Vec::new();
    for (h, info) in cache.by_json.values() {
        let Some(m) = stdm.get(h) else { continue };
        if let Some(t) = &m.base_color_texture {
            tex.push((info.pick.albedo.trim_end_matches(".png").to_string(), t.clone(), false));
        }
        if let Some(t) = &m.normal_map_texture {
            tex.push((info.pick.normal.trim_end_matches(".png").to_string(), t.clone(), true));
        }
    }
    tex.sort_by(|a, b| a.0.cmp(&b.0));
    tex.dedup_by(|a, b| a.0 == b.0);
    let (mut t_n, mut t_dim_ok, mut t_sum, mut t_cnt, mut t_max) = (0, 0, 0f64, 0usize, 0u8);
    let mut t_bad: Vec<Value> = Vec::new();
    for (r, h, normal) in tex.iter().take(limit) {
        let Some(img) = images.get(h) else { continue };
        let Some(png) = img.data.as_ref() else { continue };
        let t = match paksrc::tex_info(&ps, r) {
            Ok(t) => t,
            Err(e) => {
                t_bad.push(json!({"texture": r, "error": e}));
                continue;
            }
        };
        let px = match paksrc::decoded_rgba8(&ps, &t) {
            Ok(p) => p,
            Err(e) => {
                t_bad.push(json!({"texture": r, "error": e}));
                continue;
            }
        };
        t_n += 1;
        let m0 = &t.mips[0];
        if (img.width(), img.height()) != (m0.size_x as u32, m0.size_y as u32) || png.len() != px.len() {
            t_bad.push(json!({"texture": r, "png": [img.width(), img.height()], "pak_mip0": [m0.size_x, m0.size_y], "format": t.pixel_format_name}));
            continue;
        }
        t_dim_ok += 1;
        // normals: R, G only (the exported BC5 PNG's blue is CUE4Parse's reconstructed Z; texture::decode doc)
        let ch: &[usize] = if *normal { &[0, 1] } else { &[0, 1, 2, 3] };
        let mut worst = 0u8;
        for (a, b) in png.chunks_exact(4).zip(px.chunks_exact(4)) {
            for &c in ch {
                let d = a[c].abs_diff(b[c]);
                t_sum += d as f64;
                t_cnt += 1;
                worst = worst.max(d);
            }
        }
        t_max = t_max.max(worst);
        if worst > 2 && t_bad.len() < 40 {
            t_bad.push(json!({"texture": r, "max_abs_diff": worst, "format": t.pixel_format_name}));
        }
    }
    // fighter body: the glb's UMA_Master primitives vs the skeletal mesh decoded from the paks (pak_fighter.rs)
    let body = (|| {
        let fa = world.get_resource::<crate::fighter::FighterAssets>()?;
        let src = world.resource::<Source>();
        let rd = mh_pak::Reader::new(src.vfs.clone()?);
        let f = crate::pak_fighter::build(&ps, &rd, crate::pak_fighter::BODY_MESH, crate::pak_fighter::IDLE_ANIM).ok()?;
        let meshes = world.resource::<Assets<Mesh>>();
        let mut prims = Vec::new();
        for (h, (pm, mat)) in fa.body_prims.iter().zip(f.parts.iter()) {
            let Some(gm) = meshes.get(h) else { continue };
            let (a, b) = (f3(gm, Mesh::ATTRIBUTE_POSITION), f3(pm, Mesh::ATTRIBUTE_POSITION));
            let d = match (&a, &b) {
                (Some(a), Some(b)) if a.len() == b.len() => maxd(a, b),
                _ => -1.0,
            };
            prims.push(json!({"material": mat, "glb_vertices": gm.count_vertices(), "pak_vertices": pm.count_vertices(),
                "glb_indices": gm.indices().map_or(0, |i| i.len()), "pak_indices": pm.indices().map_or(0, |i| i.len()), "position_max_diff_m": d}));
        }
        Some(json!({"glb_prims": fa.body_prims.len(), "pak_parts": f.parts.len(), "bones": f.bones.len(),
            "clip_tracks": f.clip_tracks, "clip_tracks_unmatched": f.clip_tracks_unmatched, "prims": prims}))
    })();
    json!({
        "fighter_body": body,
        "meshes": {"packages_compared": n_pkg, "packages_total": pkgs.len(), "section_count_equal": prim_count_ok,
            "primitives": n_prim, "vertex_count_equal": vert_ok,
            "vertex_count_equal_after_dropping_unreferenced": used_eq,
            "other_vertex_lists_same_triangles": tri_eq, "triangle_corner_max_diff_m": tri_max, "glb_tangent_w_values": glb_w, "index_count_equal": idx_ok,
            "position_max_diff_m": pos_max, "normal_max_diff": nrm_max, "uv0_max_diff": uv_max,
            "tangent_w_agree": if tw_total > 0 { tw_agree as f64 / tw_total as f64 } else { -1.0 },
            "tangent_w_sign_used": paksrc::TANGENT_W_SIGN, "mismatches": bad},
        "textures": {"compared": t_n, "total": tex.len(), "size_equal": t_dim_ok,
            "mean_abs_diff": if t_cnt > 0 { t_sum / t_cnt as f64 } else { -1.0 }, "max_abs_diff": t_max, "mismatches": t_bad},
    })
}

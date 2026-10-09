//! plan.rs - one engine-side level plan from either reader, and the equivalence check between them.
//!
//! Sources (R2, docs/RUST_RUNTIME.md section 6):
//! - `pak`: mh-level (rust-pak) reads the map from the user's paks through mh-pak: placements, HLOD proxies, spawns and
//!   the lighting actors. Default when the install mounts.
//! - `extract`: this crate's R1 reader over extract/json (ue.rs), kept as the fallback (`--level-source extract`).
//! Both are ports of godot/components/ue/ue_level.gd `read`; `compare` asserts they place the same components with the
//! same transforms and material overrides (evidence: load_map.json "equiv").

use crate::ue::{self, StartRec};
use bevy::math::{Affine3A, Mat4, Vec3};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug)]
pub struct Placement {
    pub name: String,
    pub actor: String,
    /// StaticMesh object path ("pkg.N") or package
    pub mesh: String,
    /// glTF space (Y up, metres)
    pub xf: Affine3A,
    /// OverrideMaterials per slot: material package or ""
    pub mats: Vec<String>,
    pub noshadow: bool,
    /// instanced components: per-instance transforms relative to `xf` (glTF space)
    pub inst: Vec<Affine3A>,
    /// baked HQ lightmap of the component (mh-level MapBuildData; ue_lightmap.gd)
    pub lm: Option<LightmapRec>,
}

/// A component's FLightMap2D (ue_lightmap.gd: lm_coord = (CoordinateScale, CoordinateBias), lm_scale0/add0 =
/// ScaleVectors[0]/AddVectors[0], lm_scale1/add1 = ScaleVectors[1]/AddVectors[1]; Textures[0] = HQ, top half
/// coefficient 0, bottom 1; SkyOcclusionTexture)
#[derive(Clone, Debug)]
pub struct LightmapRec {
    /// "<package>/<export name>" (paksrc::tex_info spelling)
    pub tex: String,
    pub sky: Option<String>,
    pub coord: [f32; 4],
    pub scale0: [f32; 4],
    pub add0: [f32; 4],
    pub scale1: [f32; 4],
    pub add1: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct HlodRec {
    pub name: String,
    pub lod_level: i64,
    pub mesh_pkg: String,
    pub mesh_name: String,
    pub xf: Affine3A,
    /// LODDrawDistance / MinDrawDistance, cm
    pub min_draw_cm: f32,
    /// proxy bounds in glTF world space (ue_level.gd hlod_aabb)
    pub aabb: (Vec3, Vec3),
    pub subs: Vec<String>,
}

#[derive(Default)]
#[allow(dead_code)] // map: kept with the plan for debugging
pub struct Plan {
    pub source: &'static str,
    pub map: String,
    pub levels: Vec<String>,
    pub placements: Vec<Placement>,
    pub starts: Vec<StartRec>,
    pub hlods: Vec<HlodRec>,
    pub lighting: Option<mh_level::Lighting>,
    /// BP_Sky_Sphere MID colours (lighting.rs sky_mid)
    pub sky: Option<crate::lighting::SkyColors>,
    /// the map's precomputed volumetric lightmap (mh-level registry -> mh-assets Vlm, tests/vlm.rs from_level)
    pub vlm: Option<mh_assets::vlm::Vlm>,
    /// the reflection capture nearest the PlayerStarts' centre: (component name, Brightness, cooked cubemap)
    pub reflection: Option<(String, f32, mh_level::vlm::ReflectionCapture)>,
    /// every reflection capture with cooked data: (name, kind, UE component transform, props, cubemap)
    pub reflections: Vec<(String, String, mh_level::Xf, serde_json::Map<String, serde_json::Value>, mh_level::vlm::ReflectionCapture)>,
    pub seen: usize,
    pub skips: BTreeMap<String, usize>,
}

pub fn xf_gltf(x: &mh_level::Xf) -> Affine3A {
    Affine3A::from_mat4(Mat4::from_cols_array(&x.to_gltf().cols_array()))
}

/// R1 reader over extract/json.
pub fn from_extract(json_root: &std::path::Path, map: &str) -> Plan {
    let mut pk = ue::Pkgs::new(json_root);
    let i = ue::read_level(&mut pk, map);
    let conv = |r: &ue::MeshRec| Placement {
        name: r.name.clone(),
        actor: r.actor.clone(),
        mesh: r.mesh.clone(),
        xf: r.xf,
        mats: r.mats.clone(),
        noshadow: r.noshadow,
        inst: r.inst.clone(),
        lm: None,
    };
    Plan {
        source: "extract",
        map: map.into(),
        levels: i.levels.clone(),
        placements: i.meshes.iter().chain(i.isms.iter()).map(conv).collect(),
        starts: i.starts.clone(),
        hlods: Vec::new(),
        lighting: None,
        sky: None,
        vlm: None,
        reflection: None,
        reflections: Vec::new(),
        seen: i.seen,
        skips: i.skips.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
    }
}

/// mh-level over the paks.
pub fn from_pak(vfs: &std::sync::Arc<mh_pak::vfs::Vfs>, map: &str) -> Plan {
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
    let d = mh_level::read(&pk, map);
    // components the game does not draw (bHiddenInGame / bVisible false / editor-only, over the BP class chain's
    // component overrides: mh-level hidden_in_game, mh-world r3); they still collide (the sim's collision world)
    let hidden_n = d.meshes.iter().filter(|m| m.hidden_in_game).count();
    let placements = d
        .meshes
        .iter()
        .filter(|m| !m.hidden_in_game)
        .map(|m| Placement {
            name: m.name.clone(),
            actor: m.actor.clone(),
            mesh: m.mesh.clone(),
            xf: xf_gltf(&m.xf),
            mats: m.materials.clone(),
            noshadow: !m.cast_shadow,
            inst: m.instances.iter().map(xf_gltf).collect(),
            lm: d.build_data(m.level, &m.lods).and_then(|b| b.light_map.as_ref()).and_then(|l| {
                let t = l.textures[0].as_ref()?;
                Some(LightmapRec {
                    tex: format!("{}/{}", t.pkg, t.name),
                    sky: l.sky_occlusion.as_ref().map(|t| format!("{}/{}", t.pkg, t.name)),
                    coord: [l.coordinate_scale[0], l.coordinate_scale[1], l.coordinate_bias[0], l.coordinate_bias[1]],
                    scale0: l.scale_vectors[0],
                    add0: l.add_vectors[0],
                    scale1: l.scale_vectors[1],
                    add1: l.add_vectors[1],
                })
            }),
        })
        .collect();
    let starts = d
        .spawns
        .iter()
        .map(|s| StartRec { name: s.name.clone(), xf: xf_gltf(&s.xf), team: s.team.as_ref().and_then(|t| t.as_i64()) })
        .collect();
    let hlods = d
        .hlods
        .iter()
        .map(|h| {
            // ue_level.gd hlod_aabb: the mesh's UE bounds (Origin -+ BoxExtent) through the component transform
            let (o, e) = (h.box_origin, h.box_extent);
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for sx in [-1.0, 1.0] {
                for sy in [-1.0, 1.0] {
                    for sz in [-1.0, 1.0] {
                        let p = h.xf.apply([o[0] + sx * e[0], o[1] + sy * e[1], o[2] + sz * e[2]]);
                        let g = Vec3::new(p[0] as f32, p[2] as f32, p[1] as f32) * 0.01;
                        lo = lo.min(g);
                        hi = hi.max(g);
                    }
                }
            }
            HlodRec {
                name: h.name.clone(),
                lod_level: h.lod_level,
                mesh_pkg: mh_level::level::pkg_of(&h.mesh).to_string(),
                mesh_name: h.mesh_name.clone(),
                xf: xf_gltf(&h.xf),
                min_draw_cm: h.min_draw as f32,
                aabb: (lo, hi),
                subs: h.subs.clone(),
            }
        })
        .collect();
    Plan {
        source: "pak",
        map: map.into(),
        levels: d.levels.iter().map(|l| l.pkg.clone()).collect(),
        placements,
        starts,
        hlods,
        seen: d.seen,
        skips: {
            let mut k = d.skip_counts();
            if hidden_n > 0 {
                k.insert("hidden_in_game".into(), hidden_n);
            }
            k
        },
        sky: d.lights.sky.as_ref().and_then(|s| crate::lighting::sky_mid(&pk, s)),
        vlm: d.volumetric_lightmap().map(vlm_from_level),
        reflections: d
            .reflection_captures
            .iter()
            .filter_map(|r| d.reflection_data(r).map(|x| (r.name.clone(), r.kind.clone(), r.xf, r.props.clone(), x.clone())))
            .collect(),
        reflection: {
            let n = d.spawns.len().max(1) as f64;
            let c = d.spawns.iter().fold([0.0; 3], |a, s| {
                let t = s.xf.translation();
                [a[0] + t[0] / n, a[1] + t[1] / n, a[2] + t[2] / n]
            });
            d.reflection_captures
                .iter()
                .filter_map(|r| d.reflection_data(r).map(|x| (r, x)))
                .min_by(|a, b| {
                    let dist = |r: &mh_level::level::ReflectionCaptureRec| {
                        let t = r.xf.translation();
                        (t[0] - c[0]).powi(2) + (t[1] - c[1]).powi(2) + (t[2] - c[2]).powi(2)
                    };
                    dist(a.0).partial_cmp(&dist(b.0)).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(r, x)| (r.name.clone(), r.props.get("Brightness").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32, x.clone()))
        },
        lighting: Some(d.lights),
    }
}

/// mh_level::vlm::VolumetricLightmap -> mh_assets::vlm::Vlm (the copy core/crates/mh-assets/tests/vlm.rs from_level
/// makes; bounds narrowed to f32 as UE stores them)
pub fn vlm_from_level(v: &mh_level::vlm::VolumetricLightmap) -> mh_assets::vlm::Vlm {
    use mh_assets::vlm::{Layer, Vlm};
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
        lq_light_color: v.lq_light_color.as_ref().map(l).unwrap_or(empty.clone()),
        lq_light_direction: v.lq_light_direction.as_ref().map(l).unwrap_or(empty),
        sub_level_brick_positions: v.sub_level_brick_positions.clone(),
        indirection_original: v.indirection_original_values.clone(),
    }
}

/// Result of `compare` (written to load_map.json).
#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct Equiv {
    pub a: &'static str,
    pub b: &'static str,
    pub placements_a: usize,
    pub placements_b: usize,
    pub matched: usize,
    pub only_a: Vec<String>,
    pub only_b: Vec<String>,
    pub mesh_mismatch: Vec<String>,
    pub mats_mismatch: Vec<String>,
    pub inst_mismatch: Vec<String>,
    /// largest |element| difference of the glTF-space 3x4 matrices (translation in metres)
    pub max_xf_diff: f32,
    /// largest |a - b| / max(1, |a|): the readers differ only by f32 (ue.rs) vs f64 (mh-level) arithmetic, whose
    /// error grows with the magnitude (vista meshes sit kilometres out)
    pub max_xf_rel_diff: f32,
    pub worst: String,
    pub levels_equal: bool,
    pub skips_equal: bool,
    /// skip reason -> (extract count, pak count) where they differ
    pub skips_diff: Vec<(String, usize, usize)>,
    pub starts_equal: bool,
    pub identical: bool,
}

fn xf_diff(a: &Affine3A, b: &Affine3A) -> (f32, f32) {
    let (ma, mb) = (Mat4::from(*a), Mat4::from(*b));
    ma.to_cols_array().iter().zip(mb.to_cols_array().iter()).fold((0.0, 0.0), |(ab, rl), (x, y)| {
        let d = (x - y).abs();
        (f32::max(ab, d), f32::max(rl, d / x.abs().max(1.0)))
    })
}

/// Same placements, same transforms (to 1e-5 relative: f32 vs f64 readers), same overrides and instance counts.
pub fn compare(a: &Plan, b: &Plan) -> Equiv {
    let mut e = Equiv { a: a.source, b: b.source, placements_a: a.placements.len(), placements_b: b.placements.len(), ..Default::default() };
    let mut by: HashMap<&str, Vec<&Placement>> = HashMap::new();
    for p in &b.placements {
        by.entry(p.name.as_str()).or_default().push(p);
    }
    for p in &a.placements {
        let Some(q) = by.get_mut(p.name.as_str()).and_then(|v| if v.is_empty() { None } else { Some(v.remove(0)) }) else {
            e.only_a.push(p.name.clone());
            continue;
        };
        e.matched += 1;
        if ue::strip(&p.mesh) != ue::strip(&q.mesh) {
            e.mesh_mismatch.push(p.name.clone());
        }
        if p.mats != q.mats {
            e.mats_mismatch.push(p.name.clone());
        }
        if p.inst.len() != q.inst.len() {
            e.inst_mismatch.push(p.name.clone());
        }
        let (mut d, mut r) = xf_diff(&p.xf, &q.xf);
        for (i, j) in p.inst.iter().zip(q.inst.iter()) {
            let (d2, r2) = xf_diff(i, j);
            d = d.max(d2);
            r = r.max(r2);
        }
        e.max_xf_diff = e.max_xf_diff.max(d);
        if r > e.max_xf_rel_diff {
            e.max_xf_rel_diff = r;
            e.worst = p.name.clone();
        }
    }
    e.only_b = by.into_values().flatten().map(|p| p.name.clone()).collect();
    e.levels_equal = a.levels.iter().map(|s| s.to_lowercase()).eq(b.levels.iter().map(|s| s.to_lowercase()));
    e.skips_equal = a.skips == b.skips;
    let keys: std::collections::BTreeSet<&String> = a.skips.keys().chain(b.skips.keys()).collect();
    e.skips_diff = keys
        .into_iter()
        .filter_map(|k| {
            let (x, y) = (a.skips.get(k).copied().unwrap_or(0), b.skips.get(k).copied().unwrap_or(0));
            (x != y).then(|| (k.clone(), x, y))
        })
        .collect();
    let st = |p: &Plan| {
        let mut v: Vec<_> = p.starts.iter().map(|s| (s.name.clone(), s.team, (Vec3::from(s.xf.translation) * 1000.0).round().to_array().map(|x| x as i64))).collect();
        v.sort();
        v
    };
    e.starts_equal = st(a) == st(b);
    e.identical = e.only_a.is_empty()
        && e.only_b.is_empty()
        && e.mesh_mismatch.is_empty()
        && e.mats_mismatch.is_empty()
        && e.inst_mismatch.is_empty()
        && e.max_xf_rel_diff < 1e-5
        && e.levels_equal
        && e.skips_equal
        && e.starts_equal;
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The pak reader (mh-level) and the extract reader (ue.rs) place DU_Arena identically. Skipped without the
    /// install or extract/ (game data is never in the repo).
    #[test]
    fn du_arena_pak_equals_extract() {
        let json = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extract/json");
        let Ok(vfs) = mh_pak::vfs::Vfs::mount_default() else {
            eprintln!("skip: no Mordhau install");
            return;
        };
        if !json.is_dir() {
            eprintln!("skip: no extract/json");
            return;
        }
        let map = "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena";
        let a = from_extract(&json, map);
        let b = from_pak(&std::sync::Arc::new(vfs), map);
        let e = compare(&a, &b);
        assert!(e.identical, "{e:#?}");
        assert_eq!(e.matched, 830 - 64);
    }
}

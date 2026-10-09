//! Shared comparison of a resolved MaterialDesc with a Godot builder dump record (tools/export_*materials.gd)
#![allow(dead_code)]

use mh_assets::material::{MaterialDesc, Resolver, Uniform};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

pub fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-5 * (1.0 + b.abs())
}

pub fn same(u: &Uniform, g: &Value) -> bool {
    let nums = |v: &[f32]| g.as_array().is_some_and(|a| a.len() == v.len() && a.iter().zip(v).all(|(x, y)| close(*y as f64, x.as_f64().unwrap_or(f64::NAN))));
    match u {
        Uniform::F(x) => g.as_f64().is_some_and(|y| close(*x as f64, y)),
        Uniform::I(x) => g.as_i64() == Some(*x as i64) || g.as_f64() == Some(*x as f64),
        Uniform::B(x) => g.as_bool() == Some(*x),
        Uniform::V2(v) => nums(v),
        Uniform::V3(v) => nums(v),
        Uniform::V4(v) => nums(v),
        Uniform::Tex(t) => g.get("tex").and_then(Value::as_str) == Some(t.0.as_str()),
    }
}

/// Differences of `m` against Godot record `g` (master, chain, render state, mode, layout, flat, every uniform)
pub fn diff_desc(pkg: &str, m: &MaterialDesc, g: &Value, diffs: &mut Vec<String>) {
    let mut d1 = |what: &str, a: String, b: String| {
        if a != b {
            diffs.push(format!("{pkg}: {what} pak {a} godot {b}"));
        }
    };
    if g.get("master").is_some() {
        d1("master", m.master.clone(), g["master"].as_str().unwrap_or("").into());
        d1("chain", format!("{:?}", m.chain), format!("{:?}", g["chain"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect::<Vec<_>>()));
        d1("blend", m.blend.to_string(), g["blend"].to_string());
        d1("two_sided", m.two_sided.to_string(), g["variant"][1].to_string());
        d1("foliage", m.foliage.to_string(), g["variant"][2].to_string());
        d1("mode", m.mode.clone(), g["mode"].as_str().unwrap_or("").into());
        d1("layout", m.layout.clone(), g["layout"].as_str().unwrap_or("").into());
        d1("flat", m.flat.to_string(), g["flat"].to_string());
        d1("missing_albedo", m.missing_albedo.as_ref().map_or(String::new(), |t| t.0.clone()), g["missing_albedo"].as_str().unwrap_or("").into());
    }
    let gu = g["uniforms"].as_object().unwrap();
    for (k, v) in &m.uniforms {
        match gu.get(k) {
            None => diffs.push(format!("{pkg}: uniform {k} = {v:?} only from the paks")),
            Some(gv) if !same(v, gv) => diffs.push(format!("{pkg}: uniform {k} pak {v:?} godot {gv}")),
            _ => {}
        }
    }
    for (k, gv) in gu {
        if !m.uniforms.contains_key(k) {
            diffs.push(format!("{pkg}: uniform {k} = {gv} only in godot"));
        }
    }
}

/// Builds `pkg` and diffs it against `g`. The Godot builder reads a master's textures from its `mdx export` file when
/// the master was exported (CUE4Parse GetParams: ReferencedTextures + PM_* guesses) and from
/// CachedExpressionData.ReferencedTextures alone when it was not (ue_material.gd params, map r13); the paks have every
/// master, so mh-assets always takes the first form. Differences on a material whose master has no export go to `gaps`.
pub fn compare(r: &Resolver, pkg: &str, g: &Value, all: &mut Vec<String>, white: &mut BTreeSet<String>, gaps: &mut Vec<String>) {
    let Some(m) = r.build(pkg) else {
        all.push(format!("{pkg}: not resolved from the paks"));
        return;
    };
    if m.is_white() {
        white.insert(pkg.to_string());
    }
    let gap = !Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../extract/gltf/{}.json", m.master_package)).exists();
    let mut local = Vec::new();
    diff_desc(pkg, &m, g, &mut local);
    if gap { gaps.extend(local) } else { all.extend(local) }
}

//! material.rs - UE material instance -> Bevy StandardMaterial (R1). Port of the generic path of
//! godot/components/ue/ue_material.gd: json_for (mesh slot -> material JSON), params (instance -> parents -> master,
//! nearest first), pick (albedo / normal / packed by slot name, then by texture-name suffix). The per-master shader
//! math (ue_tint, docs/SHADERS.md) is phase R2; here the packed texture is re-swizzled on the CPU into Bevy's
//! occlusion(R) / roughness(G) / metallic(B) layout using ue_material.gd LAYOUTS.

use crate::ue::{strip, Pkgs};
use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageLoaderSettings};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ue_material.gd SLOT_ALBEDO / SLOT_NORMAL / SLOT_PACKED / SUFFIX / NOT_ALBEDO / BASE_COLORS / BLEND (same values)
const SLOT_ALBEDO: &[&str] = &["AlbedoMap", "Albedo", "Albedo Texture", "BaseColor", "BaseColor_01", "Diffuse", "DiffuseMap",
    "Torso Albedo", "Color", "AlbedoDetail", "Vegetation_Color_T_Main"];
const SLOT_NORMAL: &[&str] = &["NormalMap", "Normal", "Normal Texture", "Normal_01", "Vegetation_Normal_T_Main"];
const SLOT_PACKED: &[(&str, &str)] = &[("RoughnessMap", "RMA"), ("RHAO", "RHAO"), ("RHAO_01", "RHAO"), ("RoughnessAO", "RHAO"),
    ("SRMH", "SRMH"), ("Compact", "MRA"), ("Roughness Texture", "R")];
const SUFFIX: &[(&str, &str)] = &[("_rhao", "RHAO"), ("_rcrvao", "RHAO"), ("_srmh", "SRMH"), ("_srm", "SRMH"),
    ("_rmae", "RMA"), ("_rmao", "RMA"), ("_rma", "RMA"), ("_r", "RMA_R"), ("_c", "MRA"),
    ("_normal", "N"), ("_n", "N"),
    ("_basecolor", "A"), ("_albedo", "A"), ("_diffuse", "A"), ("_color", "A"), ("_d", "A"), ("_a", "A")];
const NOT_ALBEDO: &[&str] = &["grunge", "noise", "mask", "bloodmask", "_id", "translucency", "emblem", "pattern", "_s",
    "blendfunc", "default", "subsurface"];
const BASE_COLORS: &[&str] = &["Color", "ColorMultiply", "ColorOverlay", "Color_Wood", "diffuse color", "ColorA", "AlbedoColor",
    "Albedo", "Ceramic_Color", "Sub_Color", "WoodColorMultiply"];

fn blend_of(s: &str) -> i32 {
    match s {
        "BLEND_Masked" => 1,
        "BLEND_Translucent" => 2,
        "BLEND_Additive" => 3,
        "BLEND_Modulate" => 4,
        _ => 0,
    }
}

/// ue_material.gd params(): merged parameters, nearest first.
#[derive(Default, Debug, Clone)]
pub struct Params {
    pub tex: Vec<(String, String)>, // (slot, texture object path)
    pub colors: HashMap<String, [f32; 3]>,
    pub scalars: HashMap<String, f32>,
    pub blend: i32,
    pub clip: f32,
    pub two_sided: bool,
    pub master: String,
}

/// ue_material.gd pick(): asset paths (relative to the gltf root) of the chosen textures.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Pick {
    pub albedo: String,
    pub normal: String,
    pub packed: String,
    pub layout: String,
}

fn obj_pkg(op: &str) -> &str {
    strip(op)
}

/// Texture reference -> content path (ue_material.gd _tex_obj: sub-export textures are <package>/<object name>).
fn tex_obj(v: &Value) -> String {
    let op = v.get("ObjectPath").and_then(|s| s.as_str()).unwrap_or("");
    let on = v.get("ObjectName").and_then(|s| s.as_str()).unwrap_or("");
    let nm = if on.contains('\'') { on.split('\'').nth(1).unwrap_or("") } else { on };
    let pkg = obj_pkg(op);
    if op.is_empty() || nm.is_empty() || pkg.rsplit('/').next() == Some(nm) {
        return op.to_string();
    }
    format!("{pkg}/{nm}")
}

fn vec3(c: &Value) -> [f32; 3] {
    let g = |k: &str| c.get(k).and_then(|x| x.as_f64()).unwrap_or(1.0) as f32;
    [g("R"), g("G"), g("B")]
}

fn read_json(p: &Path) -> Value {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

pub struct MatCtx<'a> {
    pub pk: &'a mut Pkgs,
    /// extract/gltf (Bevy asset root; godot/data)
    pub data: &'a Path,
}

impl MatCtx<'_> {
    fn data_json(&self, pkg: &str) -> PathBuf {
        self.data.join(format!("{pkg}.json"))
    }

    pub fn tex_exists(&self, obj_path: &str) -> bool {
        !obj_path.is_empty() && self.data.join(format!("{}.png", obj_pkg(obj_path))).is_file()
    }

    /// ue_material.gd params(json_path), json_path = <data>/<pkg>.json.
    pub fn params(&mut self, own_pkg: &str) -> Option<Params> {
        let d = read_json(&self.data_json(own_pkg));
        if d.is_null() {
            return None;
        }
        let mut out = Params { blend: -1, clip: -1.0, ..Default::default() };
        let mut two_sided: Option<bool> = None;
        if let Some(t) = d.get("Textures").and_then(|t| t.as_object()) {
            for (k, v) in t {
                if v.is_object() {
                    out.tex.push((k.clone(), tex_obj(v)));
                }
            }
        }
        if let Some(t) = d.get("Colors").and_then(|t| t.as_object()) {
            for (k, v) in t {
                out.colors.insert(k.clone(), vec3(v));
            }
        }
        if let Some(t) = d.get("Scalars").and_then(|t| t.as_object()) {
            for (k, v) in t {
                out.scalars.insert(k.clone(), v.as_f64().unwrap_or(0.0) as f32);
            }
        }
        let own = own_pkg.to_string();
        let mut pkg = own.clone();
        let mut sub = String::new();
        if !self.pk.exists(&pkg) {
            // sub-export / map-embedded material: walk up to the package (ue_material.gd params)
            let mut up = Path::new(&pkg).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            while up.contains('/') && !self.pk.exists(&up) {
                up = Path::new(&up).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            }
            if self.pk.exists(&up) {
                sub = pkg.rsplit('/').next().unwrap_or("").to_string();
                pkg = up;
            } else {
                pkg.clear();
            }
        }
        for _ in 0..16 {
            if pkg.is_empty() {
                break;
            }
            let exps = self.pk.load_pkg(&pkg);
            let e = exps.iter().find(|x| {
                x.get("Type").and_then(|t| t.as_str()).unwrap_or("").starts_with("Material")
                    && (sub.is_empty() || x.get("Name").and_then(|n| n.as_str()) == Some(&sub))
            });
            sub.clear();
            let Some(e) = e.cloned() else { break };
            let p = e.get("Properties").cloned().unwrap_or(Value::Null);
            if e.get("Type").and_then(|t| t.as_str()) == Some("Material") {
                out.master = e.get("Name").and_then(|n| n.as_str()).unwrap_or("").into();
                self.master(&p, &mut out, &mut two_sided);
                if pkg != own && !self.data_json(&pkg).is_file() {
                    for t in p.pointer("/CachedExpressionData/ReferencedTextures").and_then(|a| a.as_array()).into_iter().flatten() {
                        let on = t.get("ObjectName").and_then(|s| s.as_str()).unwrap_or("");
                        if !on.is_empty() {
                            let nm = if on.contains('\'') { on.split('\'').nth(1).unwrap_or("") } else { on };
                            out.tex.push((nm.to_string(), tex_obj(t)));
                        }
                    }
                } else {
                    self.gltf_textures(&pkg, &own, &mut out);
                }
                break;
            }
            for t in p.get("TextureParameterValues").and_then(|a| a.as_array()).into_iter().flatten() {
                if let (Some(n), Some(v)) = (t.pointer("/ParameterInfo/Name").and_then(|s| s.as_str()), t.get("ParameterValue")) {
                    if v.is_object() {
                        out.tex.push((n.to_string(), tex_obj(v)));
                    }
                }
            }
            for v in p.get("VectorParameterValues").and_then(|a| a.as_array()).into_iter().flatten() {
                if let Some(n) = v.pointer("/ParameterInfo/Name").and_then(|s| s.as_str()) {
                    out.colors.entry(n.to_string()).or_insert_with(|| vec3(v.get("ParameterValue").unwrap_or(&Value::Null)));
                }
            }
            for v in p.get("ScalarParameterValues").and_then(|a| a.as_array()).into_iter().flatten() {
                if let Some(n) = v.pointer("/ParameterInfo/Name").and_then(|s| s.as_str()) {
                    out.scalars.entry(n.to_string()).or_insert(v.get("ParameterValue").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32);
                }
            }
            self.gltf_textures(&pkg, &own, &mut out);
            // bOverride_ flags (UE Material Instances docs, Material Property Overrides; ue_material.gd)
            if let Some(o) = p.get("BasePropertyOverrides") {
                let flag = |k: &str| o.get(k).and_then(|b| b.as_bool()).unwrap_or(false);
                if out.blend < 0 && flag("bOverride_BlendMode") {
                    out.blend = blend_of(o.get("BlendMode").and_then(|s| s.as_str()).unwrap_or("BLEND_Opaque"));
                }
                if out.clip < 0.0 && flag("bOverride_OpacityMaskClipValue") {
                    out.clip = o.get("OpacityMaskClipValue").and_then(|x| x.as_f64()).unwrap_or(0.3333) as f32;
                }
                if two_sided.is_none() && flag("bOverride_TwoSided") {
                    two_sided = o.get("TwoSided").and_then(|b| b.as_bool());
                }
            }
            pkg = obj_pkg(p.pointer("/Parent/ObjectPath").and_then(|s| s.as_str()).unwrap_or("")).to_string();
        }
        if out.blend < 0 {
            out.blend = d.get("BlendMode").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
        }
        if out.clip < 0.0 {
            // UE default OpacityMaskClipValue 0.3333 (ue_material.gd)
            out.clip = d.pointer("/Properties/BasePropertyOverrides/OpacityMaskClipValue").and_then(|x| x.as_f64()).unwrap_or(0.3333) as f32;
        }
        out.two_sided = two_sided.unwrap_or(false);
        Some(out)
    }

    fn gltf_textures(&self, pkg: &str, own: &str, out: &mut Params) {
        if pkg == own || !self.data_json(pkg).is_file() {
            return;
        }
        let g = read_json(&self.data_json(pkg));
        if let Some(t) = g.get("Textures").and_then(|t| t.as_object()) {
            for (k, v) in t {
                if v.is_object() {
                    out.tex.push((k.clone(), tex_obj(v)));
                }
            }
        }
    }

    /// ue_material.gd _master: CachedExpressionData.Parameters RuntimeEntries ([0] scalars, [1] vectors, [2] textures).
    fn master(&self, p: &Value, out: &mut Params, two_sided: &mut Option<bool>) {
        let cp = p.pointer("/CachedExpressionData/Parameters").cloned().unwrap_or(Value::Null);
        let names = |k: &str| -> Vec<String> {
            cp.get(k)
                .and_then(|e| e.get("ParameterInfos"))
                .and_then(|a| a.as_array())
                .map(|a| a.iter().map(|i| i.get("Name").and_then(|s| s.as_str()).unwrap_or("").to_string()).collect())
                .unwrap_or_default()
        };
        let arr = |k: &str| cp.get(k).and_then(|a| a.as_array()).cloned().unwrap_or_default();
        for (n, v) in names("RuntimeEntries[2]").iter().zip(arr("TextureValues")) {
            if v.is_object() {
                out.tex.push((n.clone(), tex_obj(&v)));
            }
        }
        for (n, v) in names("RuntimeEntries[1]").iter().zip(arr("VectorValues")) {
            out.colors.entry(n.clone()).or_insert_with(|| vec3(&v));
        }
        for (n, v) in names("RuntimeEntries").iter().zip(arr("ScalarValues")) {
            out.scalars.entry(n.clone()).or_insert(v.as_f64().unwrap_or(0.0) as f32);
        }
        if out.blend < 0 {
            out.blend = blend_of(p.get("BlendMode").and_then(|s| s.as_str()).unwrap_or("BLEND_Opaque"));
        }
        if out.clip < 0.0 {
            if let Some(c) = p.get("OpacityMaskClipValue").and_then(|x| x.as_f64()) {
                out.clip = c as f32;
            }
        }
        if two_sided.is_none() {
            *two_sided = p.get("TwoSided").and_then(|b| b.as_bool());
        }
    }

    /// ue_material.gd pick(tex), generic part (no clrmask / layers: R2).
    pub fn pick(&self, tex: &[(String, String)]) -> Pick {
        // have: (slot, png asset path, texture file name)
        let have: Vec<(String, String, String)> = tex
            .iter()
            .filter(|t| self.tex_exists(&t.1))
            .map(|t| {
                let p = obj_pkg(&t.1);
                (t.0.clone(), format!("{p}.png"), p.rsplit('/').next().unwrap_or("").to_string())
            })
            .collect();
        let first = |names: &[&str]| -> Option<&(String, String, String)> {
            names.iter().find_map(|n| have.iter().find(|h| h.0 == *n))
        };
        let by_suffix = |role: &str| -> Option<&(String, String, String)> {
            have.iter().find(|h| h.0 == h.2 && suffix_role(&h.0) == role && !h.0.to_lowercase().starts_with("default"))
        };
        let mut out = Pick::default();
        let mut a = first(SLOT_ALBEDO).or_else(|| by_suffix("A"));
        if a.is_none() {
            a = have.iter().find(|h| {
                let l = h.0.to_lowercase();
                h.0 == h.2 && suffix_role(&h.0).is_empty() && !NOT_ALBEDO.iter().any(|x| l.contains(x))
            });
        }
        if a.is_none() {
            a = first(&["PM_Diffuse"]).filter(|h| matches!(suffix_role(&h.2), "" | "A"));
        }
        out.albedo = a.map(|h| h.1.clone()).unwrap_or_default();
        let n = first(SLOT_NORMAL).or_else(|| by_suffix("N")).or_else(|| first(&["PM_Normals"]));
        out.normal = n.map(|h| h.1.clone()).unwrap_or_default();
        for (slot, lay) in SLOT_PACKED {
            if let Some(h) = first(&[slot]) {
                out.packed = h.1.clone();
                out.layout = lay.to_string();
                break;
            }
        }
        if out.packed.is_empty() {
            // longest shared prefix with the albedo's name wins, ties by role order (ue_material.gd pick)
            let roles = ["RHAO", "SRMH", "RMA", "MRA", "RMA_R"];
            let alb = a.map(|h| h.2.to_lowercase()).unwrap_or_default();
            let mut best: Option<(String, String)> = None;
            let mut best_score: i64 = -1;
            for h in &have {
                let role = suffix_role(&h.0);
                let Some(ri) = roles.iter().position(|r| *r == role) else { continue };
                if h.0 != h.2 || h.0.to_lowercase().starts_with("default") {
                    continue;
                }
                let hn = h.2.to_lowercase();
                let pre = alb.chars().zip(hn.chars()).take_while(|(x, y)| x == y).count() as i64;
                let score = pre * 10 + (4 - ri as i64);
                if score > best_score {
                    best_score = score;
                    best = Some((h.1.clone(), role.to_string()));
                }
            }
            if let Some((p, r)) = best {
                out.packed = p;
                out.layout = if r == "RMA_R" { "RMA".into() } else { r };
            }
        }
        if out.packed.is_empty() {
            if let Some(h) = first(&["PM_SpecularMasks"]) {
                let role = suffix_role(&h.2);
                if ["RMA", "RHAO", "SRMH", "MRA", "RMA_R"].contains(&role) {
                    out.packed = h.1.clone();
                    out.layout = if role == "RMA_R" { "RMA".into() } else { role.into() };
                }
            }
        }
        out
    }

    /// ue_material.gd json_for(mesh_pkg, mat_name, glb_dir) restricted to slot index: the slot's real package from
    /// the mesh JSON (StaticMaterials[k].MaterialInterface), else the nearest <name>.json around the glb (find_json).
    pub fn json_for_slot(&mut self, mesh_pkg: &str, slot: usize, mat_name: &str) -> Option<String> {
        let exps = self.pk.load_pkg(mesh_pkg);
        for e in exps.iter() {
            let Some(slots) = e.pointer("/Properties/StaticMaterials").and_then(|a| a.as_array()) else { continue };
            if let Some(op) = slots.get(slot).and_then(|s| s.pointer("/MaterialInterface/ObjectPath")).and_then(|s| s.as_str()) {
                let p = obj_pkg(op).to_string();
                if self.data_json(&p).is_file() {
                    return Some(p);
                }
            }
        }
        self.find_json(mesh_pkg, mat_name)
    }

    /// ue_material.gd find_json: the glb's folder, its subfolders, then parents (2 levels), never a whole category.
    fn find_json(&self, mesh_pkg: &str, mat_name: &str) -> Option<String> {
        if mat_name.is_empty() {
            return None;
        }
        let mut dir = Path::new(mesh_pkg).parent()?.to_path_buf();
        for i in 0..3 {
            let leaf = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if i > 0 && ["Weapons", "Assets", "Content", "data"].contains(&leaf.as_str()) {
                break;
            }
            let rel = |p: &Path| p.to_string_lossy().replace('\\', "/");
            let cand = dir.join(mat_name);
            if self.data_json(&rel(&cand)).is_file() {
                return Some(rel(&cand));
            }
            if let Ok(rd) = std::fs::read_dir(self.data.join(&dir)) {
                for s in rd.flatten() {
                    if s.path().is_dir() {
                        let c = dir.join(s.file_name()).join(mat_name);
                        if self.data_json(&rel(&c)).is_file() {
                            return Some(rel(&c));
                        }
                    }
                }
            }
            dir = dir.parent()?.to_path_buf();
        }
        None
    }
}

/// Role of a slot that is a texture name ("WoodWall_A" -> "A"), or "" (ue_material.gd suffix_role).
pub fn suffix_role(name: &str) -> &'static str {
    let n = name.to_lowercase();
    SUFFIX.iter().find(|s| n.ends_with(s.0)).map(|s| s.1).unwrap_or("")
}

/// glb section -> material slot (ue_level.gd section_slots: RenderData.LODs[0].Sections with triangles, in order;
/// CUE4Parse Gltf.cs:155-161 writes one primitive per such section).
pub fn section_slots(pk: &mut Pkgs, mesh_obj: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for e in pk.load_pkg(strip(mesh_obj)).iter() {
        if e.get("Type").and_then(|t| t.as_str()) != Some("StaticMesh") {
            continue;
        }
        if let Some(secs) = e.pointer("/RenderData/LODs/0/Sections").and_then(|a| a.as_array()) {
            for s in secs {
                if s.get("NumTriangles").and_then(|x| x.as_i64()).unwrap_or(0) > 0 {
                    out.push(s.get("MaterialIndex").and_then(|x| x.as_u64()).unwrap_or(0) as usize);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- Bevy side

/// What a material resolved to (kept for dump_state / evidence).
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // R2 picks the ue_tint mode from master
pub struct MatInfo {
    pub json: String,
    pub master: String,
    pub pick: Pick,
    pub blend: i32,
}

/// Packed textures waiting to be re-swizzled into Bevy's AO/rough/metal layout once loaded.
#[derive(Resource, Default)]
pub struct PackedJobs(pub Vec<(Handle<Image>, String, Handle<StandardMaterial>)>);

#[derive(Resource, Default)]
pub struct MatCache {
    pub by_json: HashMap<String, (Handle<StandardMaterial>, MatInfo)>,
    pub fallback: Option<Handle<StandardMaterial>>,
}

/// Channel selectors per layout, from ue_material.gd LAYOUTS: (rough, metal, ao) channel index or None.
/// None for metal = 0, None for ao = 1 (ao_add 1).
pub fn layout_sel(layout: &str) -> (Option<usize>, Option<usize>, Option<usize>) {
    match layout {
        "RMA" => (Some(0), Some(1), Some(2)),
        "RHAO" => (Some(0), None, Some(2)),
        "SRMH" => (Some(1), Some(2), None),
        "MRA" => (Some(1), Some(0), Some(2)),
        "R" => (Some(0), None, None),
        _ => (None, None, None),
    }
}

/// Build (or reuse) the StandardMaterial for material package `json_pkg`.
pub fn build(
    ctx: &mut MatCtx,
    json_pkg: &str,
    assets: &AssetServer,
    mats: &mut Assets<StandardMaterial>,
    cache: &mut MatCache,
    jobs: &mut PackedJobs,
) -> Option<(Handle<StandardMaterial>, MatInfo)> {
    if let Some(c) = cache.by_json.get(json_pkg) {
        return Some(c.clone());
    }
    let p = ctx.params(json_pkg)?;
    let k = ctx.pick(&p.tex);
    let mut m = StandardMaterial {
        // UE's default for an unconnected roughness input is 0.5 (ue_material.gd LAYOUTS "")
        perceptual_roughness: *p.scalars.get("Roughness").unwrap_or(&0.5),
        metallic: 0.0,
        // UE normal maps are DirectX style (ue_material.gd header): flip green
        flip_normal_map_y: true,
        ..default()
    };
    if !k.albedo.is_empty() {
        m.base_color_texture = Some(assets.load(k.albedo.clone()));
    } else if let Some(c) = BASE_COLORS.iter().find_map(|n| p.colors.get(*n)) {
        m.base_color = Color::linear_rgb(c[0], c[1], c[2]);
    }
    if !k.normal.is_empty() {
        m.normal_map_texture = Some(
            assets.load_builder().with_settings(|s: &mut ImageLoaderSettings| s.is_srgb = false).load(k.normal.clone()),
        );
    }
    match p.blend {
        1 => m.alpha_mode = AlphaMode::Mask(p.clip),
        2 => m.alpha_mode = AlphaMode::Blend,
        3 => m.alpha_mode = AlphaMode::Add,
        4 => m.alpha_mode = AlphaMode::Multiply,
        _ => {}
    }
    if p.two_sided {
        m.double_sided = true;
        m.cull_mode = None;
    }
    let h = mats.add(m);
    if !k.packed.is_empty() {
        let src: Handle<Image> =
            assets.load_builder().with_settings(|s: &mut ImageLoaderSettings| s.is_srgb = false).load(k.packed.clone());
        jobs.0.push((src, k.layout.clone(), h.clone()));
    }
    let info = MatInfo { json: json_pkg.to_string(), master: p.master.clone(), pick: k, blend: p.blend };
    cache.by_json.insert(json_pkg.to_string(), (h.clone(), info.clone()));
    Some((h, info))
}

/// RGBA8 packed texels -> R = AO, G = roughness, B = metallic, A = 1 (absent channels: AO 1, roughness 0.5, metal 0)
pub fn swizzle(px: &mut [u8], (r, mt, ao): (Option<usize>, Option<usize>, Option<usize>)) {
    for p in px.chunks_exact_mut(4) {
        let s = [p[0], p[1], p[2], p[3]];
        p[0] = ao.map(|i| s[i]).unwrap_or(255);
        p[1] = r.map(|i| s[i]).unwrap_or(128);
        p[2] = mt.map(|i| s[i]).unwrap_or(0);
        p[3] = 255;
    }
}

/// StandardMaterial from rust-assets' MaterialDesc (`--assets pak --materials standard`): the albedo / normal / packed
/// roles of ue_material.gd pick, base colour when there is no albedo, render state. `tex(ref, srgb)` gives an image;
/// `packed(ref, layout)` gives the re-swizzled AO/rough/metal image.
/// Texture reference a MaterialDesc binds to a ue_tint texture uniform ("albedo_texture", ...)
pub fn desc_tex(d: &mh_assets::material::MaterialDesc, k: &str) -> Option<String> {
    match d.uniforms.get(k) {
        Some(mh_assets::material::Uniform::Tex(r)) => Some(r.0.clone()),
        _ => None,
    }
}

pub fn std_from_desc(
    d: &mh_assets::material::MaterialDesc,
    albedo: Option<Handle<Image>>,
    normal: Option<Handle<Image>>,
    packed_img: Option<Handle<Image>>,
) -> (StandardMaterial, Pick) {
    use mh_assets::material::Uniform;
    let t = |k: &str| desc_tex(d, k);
    let mut m = StandardMaterial { perceptual_roughness: 0.5, metallic: 0.0, flip_normal_map_y: true, ..default() };
    if let Some(Uniform::F(r)) = d.uniforms.get("rough_add") {
        m.perceptual_roughness = r.clamp(0.0, 1.0);
    }
    let mut pick = Pick { layout: d.layout.clone(), ..Default::default() };
    if let Some(a) = t("albedo_texture") {
        m.base_color_texture = albedo;
        pick.albedo = a;
    } else if let Some(Uniform::V3(c)) = d.uniforms.get("base_color") {
        m.base_color = Color::linear_rgb(c[0], c[1], c[2]);
    }
    if let Some(n) = t("normal_texture") {
        m.normal_map_texture = normal;
        pick.normal = n;
    }
    if let Some(p) = t("rma_texture") {
        if let Some(h) = packed_img {
            let (_, _, ao) = layout_sel(&d.layout);
            m.metallic_roughness_texture = Some(h.clone());
            m.perceptual_roughness = 1.0;
            m.metallic = 1.0;
            if ao.is_some() {
                m.occlusion_texture = Some(h);
            }
        }
        pick.packed = p;
    }
    match d.blend {
        1 => m.alpha_mode = AlphaMode::Mask(d.clip),
        2 => m.alpha_mode = AlphaMode::Blend,
        3 => m.alpha_mode = AlphaMode::Add,
        4 => m.alpha_mode = AlphaMode::Multiply,
        _ => {}
    }
    if d.two_sided {
        m.double_sided = true;
        m.cull_mode = None;
    }
    (m, pick)
}

/// Once a packed texture is loaded, write a copy with R = AO, G = roughness, B = metallic (Bevy's
/// occlusion_texture R / metallic_roughness_texture G,B convention) and hook it into the material.
pub fn swizzle_packed(
    mut jobs: ResMut<PackedJobs>,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
) {
    let mut keep = Vec::new();
    for (src, layout, mat) in std::mem::take(&mut jobs.0) {
        if !assets.is_loaded_with_dependencies(&src) {
            if assets.load_state(&src).is_failed() {
                continue;
            }
            keep.push((src, layout, mat));
            continue;
        }
        let Some(img) = images.get(&src) else { continue };
        let fmt = img.texture_descriptor.format;
        if !matches!(fmt, TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb) {
            continue; // 16-bit / compressed: left unpacked (R2)
        }
        let Some(data) = img.data.as_ref() else { continue };
        let (_, _, ao) = layout_sel(&layout);
        let mut out = data.clone();
        swizzle(&mut out, layout_sel(&layout));
        let mut copy = Image::new(
            img.texture_descriptor.size,
            img.texture_descriptor.dimension,
            out,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        );
        copy.sampler = img.sampler.clone();
        let h = images.add(copy);
        if let Some(mut m) = mats.get_mut(&mat) {
            m.metallic_roughness_texture = Some(h.clone());
            m.perceptual_roughness = 1.0;
            m.metallic = 1.0;
            if ao.is_some() {
                m.occlusion_texture = Some(h);
            }
        }
    }
    jobs.0.extend(keep);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_roles() {
        assert_eq!(suffix_role("WdMtlRpe_Trim_RMAO"), "RMA");
        assert_eq!(suffix_role("prop_barrel_01_r"), "RMA_R");
        assert_eq!(suffix_role("Castle_Wall_N"), "N");
        assert_eq!(suffix_role("romankit_background_basecolor"), "A");
        assert_eq!(suffix_role("SomethingElse"), "");
    }

    #[test]
    fn layouts_match_ue_material_gd() {
        assert_eq!(layout_sel("RMA"), (Some(0), Some(1), Some(2)));
        assert_eq!(layout_sel("SRMH"), (Some(1), Some(2), None));
        assert_eq!(layout_sel("MRA"), (Some(1), Some(0), Some(2)));
    }
}

//! Materials: a UE material instance resolved through its Parent chain to its master, read from the paks (mh-pak's
//! export JSON, the shape extract/json has), and turned into an engine-neutral `MaterialDesc`: render state, master
//! shader mode, the textures by role and every `ue_tint` uniform (`shader.rs`, shaders/ue_tint.wgsl). Port of
//! godot/components/ue/ue_material.gd (`params`, `pick`, `build`, `_layers`, `_paint`, `alpha_is_mask`, `tex_srgb`);
//! the comments there (packed layouts measured per texture, blend/two-sided override rules, the master fallback through
//! CachedExpressionData.ReferencedTextures) apply here unchanged and are cited by function.
//!
//! Differences from the Godot builder, which reads extract/ (both forced by "read from the paks, never extract"):
//! - The builder's "own file" `res://data/<pkg>.json` is what `mdx export` wrote: CUE4Parse's
//!   `GetParams(CMaterialParams2, TopLayerOnly)` of that one material (MaterialExporter.cs:15, ExportOptions.cs:16).
//!   `cue_params` ports that function (UMaterialInstanceConstant.cs:246-272, UMaterialInstance.cs:110-123,
//!   UMaterialInterface.cs:79-138, UMaterial.cs:239-322, CMaterialParams2.cs:18-26/305-343) over the package's own
//!   properties, for every material of the chain (every material in the paks counts as "exported").
//! - "the texture was exported (.png present)" becomes "the texture is in the paks" (`texture_exists`).

use crate::{texture, PackageSource};
use mh_pak::Reader;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// A texture reference as the Godot builder spells it (ue_material.gd tex_path minus res://data/ and .png): the texture
/// package, or "<package>/<object>" for a texture that is a sub-export of another package (HLOD proxies, `_tex_obj`)
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TexRef(pub String);

impl TexRef {
    /// last path segment = the texture object's name (`t[1].get_file().get_basename()`)
    pub fn name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or("")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Uniform {
    F(f32),
    I(i32),
    B(bool),
    V2([f32; 2]),
    V3([f32; 3]),
    V4([f32; 4]),
    Tex(TexRef),
}

/// EBlendMode (UE 4.26 EngineTypes.h; ue_material.gd BLEND)
pub const BLEND_OPAQUE: i32 = 0;
pub const BLEND_MASKED: i32 = 1;
pub const BLEND_TRANSLUCENT: i32 = 2;

fn blend_id(s: &str) -> i32 {
    match s {
        "BLEND_Opaque" => 0,
        "BLEND_Masked" => 1,
        "BLEND_Translucent" => 2,
        "BLEND_Additive" => 3,
        "BLEND_Modulate" => 4,
        _ => 0,
    }
}

/// Master -> shader mode (ue_material.gd MODES; each cites its compiled-shader source there and in docs/SHADERS.md)
pub const MODES: &[(&str, &str)] = &[
    ("M_WeaponMaster", "M_WEAPON"),
    ("MetalWood_Master", "M_METALWOOD"),
    ("Castle_Mat_Optimized_ItBump_Arena_New2", "M_LAYERED"),
    ("Castle_Mat_Optimized_ItBump_Arena", "M_LAYERED"),
    ("Arena_Blend_M", "M_ARENABLEND"),
    ("Architecture_M_New", "M_ARCH"),
    ("RomanKit_Background", "M_ROMANKITBG"),
    ("M_CY_RoundBanner", "M_ROUNDBANNER"),
    ("RomanTrim_Background", "M_ROMANKITBG"),
    ("M_Bush", "M_BUSH"),
    ("M_MegascansFoliageMaster", "M_MEGAFOLIAGE"),
    ("M_MordhauHLOD", "M_HLOD"),
    ("Cloth_M", "M_CLOTH"),
    ("Tent_Cloth_Master", "M_CLOTH"),
    ("BannerAtlasTweak_M", "M_BANNER"),
    ("Grass_mat1_Castello", "M_GRASSCAST"),
    ("M_PLains_Fern01", "M_FERN"),
    ("Flora_twoSides_ALT", "M_FLORA"),
    ("Treasure_Decal_Ceramics", "M_DECALDIRT"),
    ("Ships_Wood_Master", "M_SHIPSWOOD"),
    ("SandBags_M", "M_SANDBAGS"),
    ("M_MainMenuCircle", "M_MAINMENUCIRCLE"),
];

/// ue_mode uniform values (ue_material.gd MODE_IDS)
pub const MODE_IDS: &[(&str, i32)] = &[
    ("", 0), ("M_WEAPON", 1), ("M_METALWOOD", 2), ("M_LAYERED", 3), ("M_ARENABLEND", 4), ("M_ARCH", 5),
    ("M_ROMANKITBG", 6), ("M_WEARABLE", 7), ("M_ROUNDBANNER", 8), ("M_CLOTH", 9), ("M_BANNER", 10),
    ("M_GRASSCAST", 11), ("M_FERN", 12), ("M_FLORA", 13), ("M_DECALDIRT", 14), ("M_HLOD", 15), ("M_MEGAFOLIAGE", 16),
    ("M_BUSH", 17), ("M_SHIPSWOOD", 18), ("M_SANDBAGS", 19), ("M_MAINMENUCIRCLE", 20),
];

pub fn mode_id(mode: &str) -> i32 {
    MODE_IDS.iter().find(|m| m.0 == mode).map_or(0, |m| m.1)
}

const SLOT_ALBEDO: &[&str] = &["AlbedoMap", "Albedo", "Albedo Texture", "BaseColor", "BaseColor_01", "Diffuse", "DiffuseMap",
    "Torso Albedo", "Color", "AlbedoDetail", "Vegetation_Color_T_Main"];
const SLOT_NORMAL: &[&str] = &["NormalMap", "Normal", "Normal Texture", "Normal_01", "Vegetation_Normal_T_Main"];
const SLOT_PACKED: &[(&str, &str)] = &[("RoughnessMap", "RMA"), ("RHAO", "RHAO"), ("RHAO_01", "RHAO"),
    ("RoughnessAO", "RHAO"), ("SRMH", "SRMH"), ("Compact", "MRA"), ("Roughness Texture", "R")];
const SUFFIX: &[(&str, &str)] = &[
    ("_rhao", "RHAO"), ("_rcrvao", "RHAO"), ("_srmh", "SRMH"), ("_srm", "SRMH"),
    ("_rmae", "RMA"), ("_rmao", "RMA"), ("_rma", "RMA"), ("_r", "RMA_R"), ("_c", "MRA"),
    ("_normal", "N"), ("_n", "N"),
    ("_basecolor", "A"), ("_albedo", "A"), ("_diffuse", "A"), ("_color", "A"), ("_d", "A"), ("_a", "A"),
];
const NOT_ALBEDO: &[&str] = &["grunge", "noise", "mask", "bloodmask", "_id", "translucency", "emblem", "pattern", "_s",
    "blendfunc", "default", "subsurface"];
const BASE_COLORS: &[&str] = &["Color", "ColorMultiply", "ColorOverlay", "Color_Wood", "diffuse color", "ColorA",
    "AlbedoColor", "Albedo", "Ceramic_Color", "Sub_Color", "WoodColorMultiply"];

/// [rough_sel, metal_sel, ao_sel, rough_add, metal_add, ao_add] per packed layout (ue_material.gd LAYOUTS)
pub fn layout(name: &str) -> ([f32; 4], [f32; 4], [f32; 4], f32, f32, f32) {
    match name {
        "RMA" => ([1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], 0., 0., 0.),
        "RHAO" => ([1., 0., 0., 0.], [0.; 4], [0., 0., 1., 0.], 0., 0., 0.),
        "SRMH" => ([0., 1., 0., 0.], [0., 0., 1., 0.], [0.; 4], 0., 0., 1.),
        "MRA" => ([0., 1., 0., 0.], [1., 0., 0., 0.], [0., 0., 1., 0.], 0., 0., 0.),
        "R" => ([1., 0., 0., 0.], [0.; 4], [0.; 4], 0., 0., 1.),
        _ => ([0.; 4], [0.; 4], [0.; 4], 0.5, 0., 1.),
    }
}

/// Role of a slot that is a texture name ("WoodWall_A" -> "A"), or "" (ue_material.gd suffix_role)
pub fn suffix_role(name: &str) -> &'static str {
    let n = name.to_lowercase();
    SUFFIX.iter().find(|s| n.ends_with(s.0)).map_or("", |s| s.1)
}

// ---- CUE4Parse GetParams (TopLayerOnly): what `mdx export` wrote as res://data/<material>.json ----------------------

/// CMaterialParams2 regexes (CMaterialParams2.cs:23-26), case-insensitive, matched by hand:
/// Diffuse `.*(?:Diff|_Tex|_?Albedo|_?Base_?Color).*|(?:_D|_DIF|_DM|_C|_CM|_DS|_DA)$`
fn is_diffuse(n: &str) -> bool {
    let l = n.to_lowercase();
    ["diff", "_tex", "albedo", "basecolor", "base_color"].iter().any(|s| l.contains(s))
        || ["_d", "_dif", "_dm", "_c", "_cm", "_ds", "_da"].iter().any(|s| l.ends_with(s))
}
/// Normals `^NO_|.*Norm.*|(?:_N|_NM|_NRM)$`
fn is_normals(n: &str) -> bool {
    let l = n.to_lowercase();
    l.starts_with("no_") || l.contains("norm") || ["_n", "_nm", "_nrm"].iter().any(|s| l.ends_with(s))
}
/// SpecularMasks `^SP_|.*(?:Specu|_S_|MR|(?<!no)RM).*|(?:_S|_LP|_PAK)$`
fn is_specmasks(n: &str) -> bool {
    let l = n.to_lowercase();
    let rm = l.match_indices("rm").any(|(i, _)| i < 2 || &l[i - 2..i] != "no");
    l.starts_with("sp_") || l.contains("specu") || l.contains("_s_") || l.contains("mr") || rm
        || ["_s", "_lp", "_pak"].iter().any(|s| l.ends_with(s))
}
/// Emissive `.*Emiss.*|(?:_E|_EM)$`
fn is_emissive(n: &str) -> bool {
    let l = n.to_lowercase();
    l.contains("emiss") || l.ends_with("_e") || l.ends_with("_em")
}

/// C# Dictionary semantics: assignment keeps an existing key's position
fn dset<T>(v: &mut Vec<(String, T)>, k: &str, x: T) {
    match v.iter_mut().find(|e| e.0 == k) {
        Some(e) => e.1 = x,
        None => v.push((k.to_string(), x)),
    }
}

/// What CUE4Parse's GetParams(CMaterialParams2, TopLayerOnly) collects for one material export. Textures keep
/// `_tex_obj` spelling ("<pkg>.N" or "<pkg>/<object>"), in dictionary order.
#[derive(Debug, Clone, Default)]
pub struct CueParams {
    pub textures: Vec<(String, String)>,
    pub colors: Vec<(String, [f32; 3])>,
    pub scalars: Vec<(String, f32)>,
    pub switches: Vec<(String, bool)>,
    pub blend: i32,
}

impl CueParams {
    /// CMaterialParams2.VerifyTexture (CMaterialParams2.cs:326-343), sampler type Color
    fn verify(&mut self, name: &str, tex: &str, append: bool) -> bool {
        let fb = if is_diffuse(name) {
            "PM_Diffuse"
        } else if is_normals(name) {
            "PM_Normals"
        } else if is_specmasks(name) {
            "PM_SpecularMasks"
        } else if is_emissive(name) {
            "PM_Emissive"
        } else {
            ""
        };
        if !fb.is_empty() {
            dset(&mut self.textures, fb, tex.to_string());
        }
        if append {
            dset(&mut self.textures, name, tex.to_string());
        }
        !fb.is_empty()
    }
    fn has(&self, k: &str) -> bool {
        self.textures.iter().any(|t| t.0 == k)
    }
}

/// The object name inside "Texture2D'Name'" (or the plain name)
fn quoted(on: &str) -> &str {
    if on.contains('\'') { on.split('\'').nth(1).unwrap_or("") } else { on }
}

pub fn strip_index(p: &str) -> &str {
    match p.rsplit_once('.') {
        Some((a, b)) if !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()) => a,
        _ => p,
    }
}

/// ue_material.gd `_tex_obj`: ObjectPath, or "<package>/<object name>" when the texture is a sub-export with another name
fn tex_obj(v: &Value) -> String {
    let op = v.get("ObjectPath").and_then(Value::as_str).unwrap_or("");
    let nm = quoted(v.get("ObjectName").and_then(Value::as_str).unwrap_or(""));
    let pkg = strip_index(op);
    if op.is_empty() || nm.is_empty() || pkg.rsplit('/').next() == Some(nm) {
        return op.to_string();
    }
    format!("{pkg}/{nm}")
}

fn tex_name(v: &Value) -> String {
    quoted(v.get("ObjectName").and_then(Value::as_str).unwrap_or("")).to_string()
}

fn vec3(c: &Value) -> [f32; 3] {
    let g = |k: &str| c.get(k).and_then(Value::as_f64).unwrap_or(1.0) as f32;
    if c.is_object() { [g("R"), g("G"), g("B")] } else { [1.0; 3] }
}

fn pname(v: &Value) -> String {
    v.get("ParameterInfo").and_then(|p| p.get("Name")).and_then(Value::as_str).unwrap_or("").to_string()
}

/// UMaterialInterface.GetParams (UMaterialInterface.cs:79-138) after the subclass parts
fn cue_interface(p: &Value, out: &mut CueParams) {
    if let Some(t) = p.get("FlattenedTexture").filter(|t| t.is_object()) {
        out.verify("Diffuse", &tex_obj(t), false);
    }
    if let Some(t) = p.get("MobileBaseTexture").filter(|t| t.is_object()) {
        out.verify("Diffuse", &tex_obj(t), false);
    }
    if let Some(t) = p.get("MobileNormalTexture").filter(|t| t.is_object()) {
        out.verify("Normal", &tex_obj(t), false);
    }
    for d in p.get("TextureStreamingData").and_then(Value::as_array).into_iter().flatten() {
        let n = d.get("TextureName").and_then(Value::as_str).unwrap_or("");
        if let Some(t) = out.textures.iter().find(|t| t.0 == n).map(|t| t.1.clone()) {
            out.verify(n, &t, false);
        }
    }
    // ParseCachedDataLegacy (4.26: CachedExpressionData.Parameters, RuntimeEntries[0..2] = scalar/vector/texture)
    let Some(cp) = p.get("CachedExpressionData").and_then(|c| c.get("Parameters")) else { return };
    let infos = |k: &str| -> Vec<String> {
        cp.get(k)
            .and_then(|e| e.get("ParameterInfos"))
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|i| i.get("Name").and_then(Value::as_str).unwrap_or("").to_string()).collect())
            .unwrap_or_default()
    };
    let arr = |k: &str| cp.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let (sn, sv) = (infos("RuntimeEntries"), arr("ScalarValues"));
    for (n, v) in sn.iter().zip(&sv) {
        dset(&mut out.scalars, n, v.as_f64().unwrap_or(0.0) as f32);
    }
    let (vn, vv) = (infos("RuntimeEntries[1]"), arr("VectorValues"));
    for (n, v) in vn.iter().zip(&vv) {
        dset(&mut out.colors, n, vec3(v));
    }
    let (tn, tv) = (infos("RuntimeEntries[2]"), arr("TextureValues"));
    for (n, v) in tn.iter().zip(&tv) {
        if v.is_object() {
            out.verify(n, &tex_obj(v), true);
        }
    }
}

/// GetParams(CMaterialParams2, TopLayerOnly) of one exported material (Type "Material" or a material instance)
pub fn cue_params(e: &Value) -> CueParams {
    let mut out = CueParams::default();
    let empty = Value::Object(Default::default());
    let p = e.get("Properties").unwrap_or(&empty);
    if e.get("Type").and_then(Value::as_str) == Some("Material") {
        // UMaterial.cs:239-322
        out.blend = blend_id(p.get("BlendMode").and_then(Value::as_str).unwrap_or("BLEND_Opaque"));
        let rt: Vec<&Value> = p
            .get("CachedExpressionData")
            .and_then(|c| c.get("ReferencedTextures"))
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        for t in rt.iter().filter(|t| t.is_object()) {
            dset(&mut out.textures, &tex_name(t), tex_obj(t));
        }
        cue_interface(p, &mut out);
        if rt.len() == 1 && rt[0].is_object() {
            dset(&mut out.textures, "PM_Diffuse", tex_obj(rt[0]));
            return out;
        }
        let mut i = rt.len();
        while !(out.has("PM_Diffuse") && out.has("PM_Normals") && out.has("PM_SpecularMasks") && out.has("PM_Emissive"))
            && i > 0
        {
            i -= 1;
            let t = rt[i];
            if !t.is_object() {
                continue;
            }
            let n = tex_name(t);
            if !out.has("PM_Diffuse") && is_diffuse(&n) {
                dset(&mut out.textures, "PM_Diffuse", tex_obj(t));
                continue;
            }
            if !out.has("PM_Normals") && is_normals(&n) {
                dset(&mut out.textures, "PM_Normals", tex_obj(t));
                continue;
            }
            if !out.has("PM_SpecularMasks") && is_specmasks(&n) {
                dset(&mut out.textures, "PM_SpecularMasks", tex_obj(t));
                continue;
            }
            if !out.has("PM_Emissive") && is_emissive(&n) {
                dset(&mut out.textures, "PM_Emissive", tex_obj(t));
            }
        }
        return out;
    }
    // UMaterialInstanceConstant.cs:246-272 (TopLayerOnly: the parent is not read), base UMaterialInstance.cs:110-123
    cue_interface(p, &mut out);
    for s in p.get("StaticParameters").and_then(|s| s.get("StaticSwitchParameters")).and_then(Value::as_array).into_iter().flatten() {
        dset(&mut out.switches, &pname(s), s.get("Value").and_then(Value::as_bool).unwrap_or(false));
    }
    if let Some(o) = p.get("BasePropertyOverrides").filter(|o| o.is_object()) {
        out.blend = blend_id(o.get("BlendMode").and_then(Value::as_str).unwrap_or("BLEND_Opaque"));
    }
    for t in p.get("TextureParameterValues").and_then(Value::as_array).into_iter().flatten() {
        let Some(v) = t.get("ParameterValue").filter(|v| v.is_object()) else { continue };
        let to = tex_obj(v);
        if !out.verify(&pname(t), &to, true) {
            out.verify(&tex_name(v), &to, true);
        }
    }
    for v in p.get("VectorParameterValues").and_then(Value::as_array).into_iter().flatten() {
        if let Some(c) = v.get("ParameterValue").filter(|c| c.is_object()) {
            dset(&mut out.colors, &pname(v), vec3(c));
        }
    }
    for v in p.get("ScalarParameterValues").and_then(Value::as_array).into_iter().flatten() {
        dset(&mut out.scalars, &pname(v), v.get("ParameterValue").and_then(Value::as_f64).unwrap_or(0.0) as f32);
    }
    out
}

// ---- params (ue_material.gd params / _master) -------------------------------------------------------------------

/// Merged parameters of a material, nearest first (ue_material.gd params)
#[derive(Debug, Clone, Default)]
pub struct Params {
    /// (slot, texture) in lookup order, duplicates kept
    pub tex: Vec<(String, TexRef)>,
    pub colors: HashMap<String, [f32; 3]>,
    pub scalars: HashMap<String, f32>,
    pub switches: HashMap<String, bool>,
    pub blend: i32,
    pub clip: f32,
    pub two_sided: bool,
    pub foliage: bool,
    /// the root Material's ShadingModel is MSM_Unlit: UE outputs EmissiveColor only, no lighting
    pub unlit: bool,
    /// the vector parameter the root Material's EmissiveColor input reads (MaterialExpressionVectorParameter wired
    /// straight to EmissiveColor); resolved through `colors` so instances override it
    pub emissive_param: Option<String>,
    pub chain: Vec<String>,
    pub master: String,
    /// package of the root Material
    pub master_package: String,
}

fn tref(s: &str) -> TexRef {
    TexRef(strip_index(s).to_string())
}

/// The material resolver: package JSON through mh-pak, texture pixels / existence through a `PackageSource`
pub struct Resolver<'a> {
    pub rd: &'a Reader,
    pub src: &'a dyn PackageSource,
    pkgs: RefCell<HashMap<String, Option<std::rc::Rc<Vec<Value>>>>>,
    exists: RefCell<HashMap<String, bool>>,
    alpha: RefCell<HashMap<String, bool>>,
}

impl<'a> Resolver<'a> {
    pub fn new(rd: &'a Reader, src: &'a dyn PackageSource) -> Self {
        Resolver { rd, src, pkgs: Default::default(), exists: Default::default(), alpha: Default::default() }
    }

    fn load(&self, pkg: &str) -> Option<std::rc::Rc<Vec<Value>>> {
        let k = pkg.to_lowercase();
        if let Some(v) = self.pkgs.borrow().get(&k) {
            return v.clone();
        }
        let v = self.rd.read(pkg).map(std::rc::Rc::new);
        self.pkgs.borrow_mut().insert(k, v.clone());
        v
    }

    fn pkg_exists(&self, pkg: &str) -> bool {
        self.rd.vfs.has(&format!("{pkg}.uasset")) || self.rd.vfs.has(&format!("{pkg}.umap"))
    }

    /// "the texture is in the paks" (the Godot builder: "the .png was exported", ResourceLoader.exists): its package,
    /// or for "<package>/<object>" an export of that name in the package
    pub fn texture_exists(&self, t: &TexRef) -> bool {
        if t.0.is_empty() {
            return false;
        }
        if let Some(&b) = self.exists.borrow().get(&t.0) {
            return b;
        }
        let ok = if self.pkg_exists(&t.0) {
            true
        } else if let Some((pkg, obj)) = t.0.rsplit_once('/') {
            self.rd.open(pkg).is_some_and(|a| a.exports.iter().any(|e| e.name == obj))
        } else {
            false
        };
        self.exists.borrow_mut().insert(t.0.clone(), ok);
        ok
    }

    /// (package, export name) of a texture reference
    fn tex_pkg(&self, t: &TexRef) -> (String, Option<String>) {
        if self.pkg_exists(&t.0) {
            (t.0.clone(), None)
        } else {
            let (p, o) = t.0.rsplit_once('/').unwrap_or((&t.0, ""));
            (p.to_string(), Some(o.to_string()))
        }
    }

    /// The texture's UTexture SRGB flag (default true; ue_material.gd tex_srgb)
    pub fn tex_srgb(&self, t: &TexRef) -> bool {
        let (p, o) = self.tex_pkg(t);
        for e in self.load(&p).iter().flat_map(|v| v.iter()) {
            let ty = e.get("Type").and_then(Value::as_str).unwrap_or("");
            if ty.starts_with("Texture") && o.as_deref().is_none_or(|o| e.get("Name").and_then(Value::as_str) == Some(o)) {
                return e.get("Properties").and_then(|p| p.get("SRGB")).and_then(Value::as_bool).unwrap_or(true);
            }
        }
        true
    }

    /// At least 1% of the texels pass `clip` in the texture's alpha, sampled every 4th texel in x and y; false when
    /// every alpha is 255 (Image.detect_alpha ALPHA_NONE). ue_material.gd alpha_is_mask, over mip 0 decoded from the
    /// paks instead of the exported PNG.
    pub fn alpha_is_mask(&self, t: &TexRef, clip: f32) -> bool {
        let key = format!("{}@{}", t.0, clip);
        if let Some(&b) = self.alpha.borrow().get(&key) {
            return b;
        }
        let (p, o) = self.tex_pkg(t);
        let ok = (|| -> Option<bool> {
            let tx = texture::info(self.src, &p, o.as_deref()).ok()?;
            let data = texture::mip_data(self.src, &tx, 0).ok()?;
            let (w, h) = (tx.mips[0].size_x as usize, tx.mips[0].size_y as usize);
            let px = match texture::decode(tx.format?, w, h, &data).ok()? {
                texture::Pixels::Rgba8(v) => v,
                texture::Pixels::RgbaF32(v) => v.iter().map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8).collect(),
            };
            if px.chunks_exact(4).all(|c| c[3] == 255) {
                return Some(false);
            }
            let (mut n, mut pass) = (0usize, 0usize);
            for y in (0..h).step_by(4) {
                for x in (0..w).step_by(4) {
                    n += 1;
                    pass += (px[(y * w + x) * 4 + 3] as f32 / 255.0 >= clip) as usize;
                }
            }
            Some(n > 0 && pass as f64 / n as f64 >= 0.01)
        })()
        .unwrap_or(false);
        self.alpha.borrow_mut().insert(key, ok);
        ok
    }

    /// ue_material.gd params(): own parameters (CUE4Parse's view, `cue_params`), then the package chain own ->
    /// parents -> master, nearest first. None when the package holds no material.
    pub fn params(&self, mat_pkg: &str) -> Option<Params> {
        let own = strip_index(mat_pkg).to_string();
        let mut out = Params { blend: -1, clip: -1.0, ..Default::default() };
        let (mut two_sided, mut foliage): (Option<bool>, Option<bool>) = (None, None);
        let first = |pkg: &str, sub: Option<&str>| -> Option<Value> {
            self.load(pkg)?
                .iter()
                .find(|x| {
                    x.get("Type").and_then(Value::as_str).unwrap_or("").starts_with("Material")
                        && sub.is_none_or(|s| x.get("Name").and_then(Value::as_str) == Some(s))
                })
                .cloned()
        };
        // A material that is a sub-export of another package (HLOD proxies: ".../HLOD/Arena_0_HLOD/M_..." in package
        // ".../HLOD/Arena_0_HLOD") is looked up by name inside the nearest existing package (ue_material.gd params)
        let (start, sub) = if self.pkg_exists(&own) {
            (own.clone(), None)
        } else {
            let mut up = own.rsplit_once('/').map_or("", |x| x.0).to_string();
            while up.contains('/') && !self.pkg_exists(&up) {
                up = up.rsplit_once('/').map_or("", |x| x.0).to_string();
            }
            if !self.pkg_exists(&up) {
                return None;
            }
            (up, own.rsplit('/').next().map(str::to_string))
        };
        let own_e = first(&start, sub.as_deref())?;
        let d = cue_params(&own_e);
        for (k, t) in &d.textures {
            out.tex.push((k.clone(), tref(t)));
        }
        for (k, v) in &d.colors {
            out.colors.insert(k.clone(), *v);
        }
        for (k, v) in &d.scalars {
            out.scalars.insert(k.clone(), *v);
        }
        for (k, v) in &d.switches {
            out.switches.insert(k.clone(), *v);
        }
        let mut pkg = start.clone();
        let mut sub = sub;
        for _ in 0..16 {
            let Some(e) = (if pkg.is_empty() { None } else { first(&pkg, sub.take().as_deref()) }) else { break };
            out.chain.push(e.get("Name").and_then(Value::as_str).unwrap_or("").to_string());
            let empty = Value::Object(Default::default());
            let p = e.get("Properties").unwrap_or(&empty);
            if e.get("Type").and_then(Value::as_str) == Some("Material") {
                out.master = e.get("Name").and_then(Value::as_str).unwrap_or("").to_string();
                out.master_package = pkg.clone();
                self.master(p, &mut out, &mut two_sided, &mut foliage);
                // the master's own textures (ReferencedTextures by name, its parameter textures, PM_* guesses): what
                // the builder gets from the master's export or, unexported, from CachedExpressionData.ReferencedTextures
                // (map r13 fix; CUE4Parse UMaterial.cs:46-47, :272-275)
                if pkg != start {
                    for (k, t) in cue_params(&e).textures {
                        out.tex.push((k, tref(&t)));
                    }
                }
                break;
            }
            for t in p.get("TextureParameterValues").and_then(Value::as_array).into_iter().flatten() {
                if let Some(v) = t.get("ParameterValue").filter(|v| v.is_object()) {
                    out.tex.push((pname(t), tref(&tex_obj(v))));
                }
            }
            for v in p.get("VectorParameterValues").and_then(Value::as_array).into_iter().flatten() {
                out.colors.entry(pname(v)).or_insert_with(|| vec3(v.get("ParameterValue").unwrap_or(&Value::Null)));
            }
            for v in p.get("ScalarParameterValues").and_then(Value::as_array).into_iter().flatten() {
                out.scalars.entry(pname(v)).or_insert(v.get("ParameterValue").and_then(Value::as_f64).unwrap_or(0.0) as f32);
            }
            for v in p.get("StaticParameters").and_then(|s| s.get("StaticSwitchParameters")).and_then(Value::as_array).into_iter().flatten() {
                out.switches.entry(pname(v)).or_insert(v.get("Value").and_then(Value::as_bool).unwrap_or(false));
            }
            if pkg != start {
                // _gltf_textures: a parent's own export
                for (k, t) in cue_params(&e).textures {
                    out.tex.push((k, tref(&t)));
                }
            }
            // an instance's override counts only with its bOverride_ flag (UE Material Instances docs)
            let o = p.get("BasePropertyOverrides").cloned().unwrap_or(Value::Null);
            let flag = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
            if out.blend < 0 && flag("bOverride_BlendMode") {
                out.blend = blend_id(o.get("BlendMode").and_then(Value::as_str).unwrap_or("BLEND_Opaque"));
            }
            if out.clip < 0.0 && flag("bOverride_OpacityMaskClipValue") {
                out.clip = o.get("OpacityMaskClipValue").and_then(Value::as_f64).unwrap_or(0.3333) as f32;
            }
            if two_sided.is_none() && flag("bOverride_TwoSided") {
                two_sided = Some(o.get("TwoSided").and_then(Value::as_bool).unwrap_or(false));
            }
            if foliage.is_none() && flag("bOverride_ShadingModel") {
                foliage = Some(o.get("ShadingModel").and_then(Value::as_str) == Some("MSM_TwoSidedFoliage"));
            }
            pkg = strip_index(p.get("Parent").and_then(|x| x.get("ObjectPath")).and_then(Value::as_str).unwrap_or("")).to_string();
        }
        if out.blend < 0 {
            out.blend = d.blend;
        }
        if out.clip < 0.0 {
            out.clip = own_e
                .get("Properties")
                .and_then(|p| p.get("BasePropertyOverrides"))
                .and_then(|o| o.get("OpacityMaskClipValue"))
                .and_then(Value::as_f64)
                .unwrap_or(0.3333) as f32;
        }
        out.two_sided = two_sided.unwrap_or(false);
        out.foliage = foliage.unwrap_or(false);
        Some(out)
    }

    /// ue_material.gd _master: the root Material's defaults in CachedExpressionData.Parameters (RuntimeEntries =
    /// scalars, [1] = vectors, [2] = textures, parallel to ScalarValues / VectorValues / TextureValues) and its render
    /// state
    fn master(&self, p: &Value, out: &mut Params, two_sided: &mut Option<bool>, foliage: &mut Option<bool>) {
        let empty = Value::Null;
        let cp = p.get("CachedExpressionData").and_then(|c| c.get("Parameters")).unwrap_or(&empty);
        let names = |k: &str| -> Vec<String> {
            cp.get(k)
                .and_then(|e| e.get("ParameterInfos"))
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|i| i.get("Name").and_then(Value::as_str).unwrap_or("").to_string()).collect())
                .unwrap_or_default()
        };
        let arr = |k: &str| cp.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        for (n, v) in names("RuntimeEntries[2]").iter().zip(arr("TextureValues")) {
            if v.is_object() {
                out.tex.push((n.clone(), tref(&tex_obj(&v))));
            }
        }
        for (n, v) in names("RuntimeEntries[1]").iter().zip(arr("VectorValues")) {
            out.colors.entry(n.clone()).or_insert_with(|| vec3(&v));
        }
        for (n, v) in names("RuntimeEntries").iter().zip(arr("ScalarValues")) {
            out.scalars.entry(n.clone()).or_insert(v.as_f64().unwrap_or(0.0) as f32);
        }
        if out.blend < 0 {
            out.blend = blend_id(p.get("BlendMode").and_then(Value::as_str).unwrap_or("BLEND_Opaque"));
        }
        if out.clip < 0.0 {
            if let Some(c) = p.get("OpacityMaskClipValue").and_then(Value::as_f64) {
                out.clip = c as f32;
            }
        }
        if two_sided.is_none() {
            if let Some(b) = p.get("TwoSided").and_then(Value::as_bool) {
                *two_sided = Some(b);
            }
        }
        if foliage.is_none() {
            if let Some(s) = p.get("ShadingModel").and_then(Value::as_str) {
                *foliage = Some(s == "MSM_TwoSidedFoliage");
            }
        }
        // MSM_Unlit (e.g. M_PromoSphere, the main menu's dome: EmissiveColor <- VectorParameter "Dome Emissive").
        // The export names only the expression object, not its ParameterName, so the parameter is identified when the
        // root has exactly one vector parameter. UNCONFIRMED for masters with several vector parameters (left unset).
        out.unlit = p.get("ShadingModel").and_then(Value::as_str) == Some("MSM_Unlit");
        let ec = p.get("EmissiveColor");
        let from_vector_param = ec
            .and_then(|e| e.get("ExpressionName"))
            .and_then(Value::as_str)
            .is_some_and(|n| n.starts_with("MaterialExpressionVectorParameter"));
        let use_constant = ec.and_then(|e| e.get("UseConstant")).and_then(Value::as_bool).unwrap_or(false);
        if from_vector_param && !use_constant {
            let v = names("RuntimeEntries[1]");
            if v.len() == 1 {
                out.emissive_param = Some(v[0].clone());
            }
        }
    }

    // ---- pick (ue_material.gd pick) ----

    pub fn pick(&self, tex: &[(String, TexRef)]) -> Pick {
        let have: Vec<(&str, &TexRef, &str)> =
            tex.iter().filter(|t| self.texture_exists(&t.1)).map(|t| (t.0.as_str(), &t.1, t.1.name())).collect();
        let first = |names: &[&str]| -> Option<(&str, &TexRef, &str)> {
            names.iter().find_map(|n| have.iter().find(|h| h.0 == *n).copied())
        };
        let by_suffix = |role: &str| -> Option<(&str, &TexRef, &str)> {
            have.iter()
                .find(|h| h.0 == h.2 && suffix_role(h.0) == role && !h.0.to_lowercase().starts_with("default"))
                .copied()
        };
        let mut out = Pick::default();
        out.clrmask = have.iter().find(|h| h.0 == h.2 && h.0.to_lowercase().ends_with("_clrmask")).map(|h| h.1.clone());
        for i in 2..5 {
            let Some(la) = first(&[&format!("BaseColor_0{i}")]) else { break };
            let ln = first(&[&format!("Normal_0{i}")]);
            let lp = first(&[&format!("RHAO_0{i}")]);
            out.layers.push((la.1.clone(), ln.map(|h| h.1.clone()), lp.map(|h| h.1.clone())));
        }
        'm: for nm in SLOT_ALBEDO {
            for t in tex {
                if t.0 == *nm && !t.1 .0.is_empty() && !self.texture_exists(&t.1) && out.albedo_missing.is_none() {
                    out.albedo_missing = Some(t.1.clone());
                    break 'm;
                }
            }
        }
        let mut a = first(SLOT_ALBEDO).or_else(|| by_suffix("A"));
        if a.is_none() {
            a = have
                .iter()
                .find(|h| h.0 == h.2 && suffix_role(h.0).is_empty() && !NOT_ALBEDO.iter().any(|x| h.0.to_lowercase().contains(x)))
                .copied();
        }
        if a.is_none() {
            a = first(&["PM_Diffuse"]).filter(|h| matches!(suffix_role(h.2), "" | "A"));
        }
        out.albedo = a.map(|h| h.1.clone());
        let n = first(SLOT_NORMAL).or_else(|| by_suffix("N")).or_else(|| first(&["PM_Normals"]));
        out.normal = n.map(|h| h.1.clone());
        for (slot, lay) in SLOT_PACKED {
            if let Some(h) = first(&[slot]) {
                out.packed = Some(h.1.clone());
                out.layout = lay.to_string();
                break;
            }
        }
        if out.packed.is_none() {
            // longest shared name prefix with the albedo, ties by role order (ue_material.gd pick)
            let roles = ["RHAO", "SRMH", "RMA", "MRA", "RMA_R"];
            let alb: Vec<char> = a.map_or("", |h| h.2).chars().collect();
            let mut best: Option<(&TexRef, &str)> = None;
            let mut best_score = -1i64;
            for h in &have {
                let role = suffix_role(h.0);
                if h.0 != h.2 || !roles.contains(&role) || h.0.to_lowercase().starts_with("default") {
                    continue;
                }
                let hc: Vec<char> = h.2.chars().collect();
                let mut pre = 0;
                while pre < alb.len().min(hc.len()) && alb[pre].to_lowercase().eq(hc[pre].to_lowercase()) {
                    pre += 1;
                }
                let score = pre as i64 * 10 + (4 - roles.iter().position(|r| *r == role).unwrap() as i64);
                if score > best_score {
                    best_score = score;
                    best = Some((h.1, role));
                }
            }
            if let Some((t, role)) = best {
                out.packed = Some(t.clone());
                out.layout = if role == "RMA_R" { "RMA".into() } else { role.into() };
            }
        }
        if out.packed.is_none() {
            if let Some(h) = first(&["PM_SpecularMasks"]) {
                let role = suffix_role(h.2);
                if ["RMA", "RHAO", "SRMH", "MRA", "RMA_R"].contains(&role) {
                    out.packed = Some(h.1.clone());
                    out.layout = if role == "RMA_R" { "RMA".into() } else { role.into() };
                }
            }
        }
        out
    }

    /// First texture in the paks for slot name `slot`, nearest in the chain (ue_material.gd slot_png)
    pub fn slot_tex(&self, p: &Params, slot: &str) -> Option<TexRef> {
        p.tex.iter().find(|t| t.0 == slot && self.texture_exists(&t.1)).map(|t| t.1.clone())
    }

    // ---- build (ue_material.gd build) ----

    /// The material of a package: render state, mode and the ue_tint uniforms the Godot builder sets
    pub fn build(&self, mat_pkg: &str) -> Option<MaterialDesc> {
        let p = self.params(mat_pkg)?;
        let k = self.pick(&p.tex);
        let mode = MODES.iter().find(|m| m.0 == p.master).map_or("", |m| m.1).to_string();
        let mut u: BTreeMap<String, Uniform> = BTreeMap::new();
        let mut set = |n: &str, v: Uniform| {
            u.insert(n.to_string(), v);
        };
        // set_variant (build: map materials get the VLM path, no lightmap yet)
        set("ue_mode", Uniform::I(mode_id(&mode)));
        set("use_foliage", Uniform::B(p.foliage));
        set("use_lightmap", Uniform::B(false));
        set("use_vlm", Uniform::B(true));
        if let Some(t) = &k.albedo {
            set("albedo_texture", Uniform::Tex(t.clone()));
        }
        if let Some(t) = &k.normal {
            set("normal_texture", Uniform::Tex(t.clone()));
        }
        set("use_normal", Uniform::B(k.normal.is_some()));
        if let Some(t) = &k.packed {
            set("rma_texture", Uniform::Tex(t.clone()));
        }
        let lay = layout(&k.layout);
        set("rough_sel", Uniform::V4(lay.0));
        set("metal_sel", Uniform::V4(lay.1));
        set("ao_sel", Uniform::V4(lay.2));
        set("rough_add", Uniform::F(if k.packed.is_none() { p.scalars.get("Roughness").copied().unwrap_or(lay.3) } else { lay.3 }));
        set("metal_add", Uniform::F(lay.4));
        set("ao_add", Uniform::F(lay.5));
        if p.blend == BLEND_MASKED || p.blend == BLEND_TRANSLUCENT {
            set("alpha_clip", Uniform::F(p.clip));
            let a = k.albedo.as_ref().is_some_and(|t| self.alpha_is_mask(t, if p.blend == BLEND_MASKED { p.clip } else { 0.01 }));
            set("alpha_from_albedo", Uniform::B(a));
            set("alpha_value", Uniform::F(if p.blend == BLEND_TRANSLUCENT && k.albedo.is_none() { 0.0 } else { 1.0 }));
        }
        drop(set);
        let mut b = Builder { r: self, p: &p, u };
        b.mode(&mode, &k);
        let mut u = b.u;
        let mut flat = false;
        if k.albedo.is_none() {
            let base = BASE_COLORS.iter().find_map(|c| p.colors.get(*c).copied());
            u.insert("base_color".into(), Uniform::V3(base.unwrap_or([1.0; 3])));
            flat = base.is_some() || mode == "M_DECALDIRT";
        }
        // unlit: the renderer outputs base_color as the emissive (no lighting); base_color carries the resolved
        // EmissiveColor parameter (instance value first, then the master default)
        let unlit_emissive = if p.unlit { p.emissive_param.as_ref().and_then(|n| p.colors.get(n).copied()) } else { None };
        if let Some(e) = unlit_emissive {
            u.remove("albedo_texture");
            u.insert("base_color".into(), Uniform::V3(e));
            flat = true;
        }
        Some(MaterialDesc {
            package: strip_index(mat_pkg).to_string(),
            master: p.master.clone(),
            master_package: p.master_package.clone(),
            chain: p.chain.clone(),
            mode,
            blend: p.blend,
            two_sided: p.two_sided,
            foliage: p.foliage,
            unlit: unlit_emissive.is_some(),
            clip: p.clip,
            layout: k.layout.clone(),
            flat,
            missing_albedo: if k.albedo.is_none() { k.albedo_missing.clone() } else { None },
            uniforms: u,
        })
    }
}

/// ue_material.gd pick() result
#[derive(Debug, Clone, Default)]
pub struct Pick {
    pub albedo: Option<TexRef>,
    pub normal: Option<TexRef>,
    pub packed: Option<TexRef>,
    pub layout: String,
    pub albedo_missing: Option<TexRef>,
    pub clrmask: Option<TexRef>,
    /// layers 2..4: (BaseColor_0n, Normal_0n, RHAO_0n)
    pub layers: Vec<(TexRef, Option<TexRef>, Option<TexRef>)>,
}

/// A resolved material: everything a renderer needs, engine-neutral
#[derive(Debug, Clone)]
pub struct MaterialDesc {
    pub package: String,
    pub master: String,
    pub master_package: String,
    pub chain: Vec<String>,
    /// shader mode ("" generic, "M_WEAPON", ...; `mode_id`)
    pub mode: String,
    /// EBlendMode: 0 opaque, 1 masked (alpha clip at `clip`), 2 translucent, 3 additive, 4 modulate
    pub blend: i32,
    pub two_sided: bool,
    /// MSM_TwoSidedFoliage shading
    pub foliage: bool,
    /// MSM_Unlit with a resolved emissive: draw `base_color` as emissive only (no lighting)
    pub unlit: bool,
    pub clip: f32,
    /// packed-texture layout ("RMA", "RHAO", "SRMH", "MRA", "R" or "" = none)
    pub layout: String,
    /// no albedo texture and a flat base colour from BASE_COLORS (or the decal)
    pub flat: bool,
    /// a named albedo slot whose texture is not in the paks
    pub missing_albedo: Option<TexRef>,
    /// ue_tint uniforms by name (godot/game/render/ue_tint.gdshaderinc; shaders/ue_tint.wgsl), textures as `Tex`
    pub uniforms: BTreeMap<String, Uniform>,
}

impl MaterialDesc {
    /// The material's textures by uniform name (role)
    pub fn textures(&self) -> impl Iterator<Item = (&str, &TexRef)> {
        self.uniforms.iter().filter_map(|(k, v)| if let Uniform::Tex(t) = v { Some((k.as_str(), t)) } else { None })
    }
    /// "white": no albedo texture and no flat colour
    pub fn is_white(&self) -> bool {
        !self.uniforms.contains_key("albedo_texture") && !self.flat
    }
}

/// A soft / hard object reference as a content package (ue_wearable.gd pkg_path): "/Game/X/Y.Y_C" ->
/// "Mordhau/Content/X/Y", {"AssetPathName": ..} / {"ObjectPath": "pkg.N"} dicts; "" for None
pub fn ue_pkg_path(v: &Value) -> String {
    let s = match v {
        Value::Object(o) => o.get("AssetPathName").or_else(|| o.get("ObjectPath")).and_then(Value::as_str).unwrap_or(""),
        Value::String(s) => s.as_str(),
        _ => "",
    };
    if s.is_empty() || s == "None" {
        return String::new();
    }
    if let Some(r) = s.strip_prefix("/Game/") {
        return format!("Mordhau/Content/{}", r.split('.').next().unwrap_or(""));
    }
    if let Some(r) = s.strip_prefix('/') {
        return r.split('.').next().unwrap_or("").to_string();
    }
    strip_index(s).to_string()
}

impl Resolver<'_> {
    /// The wearable material CharacterBuilder._paint builds for a wearable class (UMordhauWearable AlbedoMap +0x68,
    /// NormalMap +0x70, RoughnessMap +0x78, Patterns[pattern].Texture; class defaults over the Blueprint chain, mh-pak
    /// Reader::defaults): None when its albedo is not in the paks (the builder then keeps the mesh's own material).
    /// `colors` = the resolved ColorA/B/C (colour tables are the loadout's; None = master defaults); `masked` = the
    /// mesh slot's material is a *_Masked instance (alpha clip with the albedo alpha if it is a mask, alpha_is_mask).
    pub fn wearable_from_class(&self, class_pkg: &str, pattern: usize, colors: Option<[[f32; 3]; 3]>, masked: bool)
        -> Option<MaterialDesc> {
        let d = self.rd.defaults(class_pkg);
        let tex = |k: &str| -> Option<TexRef> {
            let t = TexRef(ue_pkg_path(d.get(k).unwrap_or(&Value::Null)));
            self.texture_exists(&t).then_some(t)
        };
        let albedo = tex("AlbedoMap")?;
        let mask = d
            .get("Patterns")
            .and_then(Value::as_array)
            .and_then(|a| a.get(pattern))
            .map(|p| TexRef(ue_pkg_path(p.get("Texture").unwrap_or(&Value::Null))))
            .filter(|t| self.texture_exists(t));
        let set = WearableSet { albedo: Some(albedo.clone()), normal: tex("NormalMap"), rma: tex("RoughnessMap"), mask, colors,
            has_emblem: false };
        let mut m = wearable(&set, None, None, masked);
        m.package = class_pkg.to_string();
        if masked {
            m.uniforms.insert("alpha_from_albedo".into(), Uniform::B(self.alpha_is_mask(&albedo, 0.3333)));
        }
        Some(m)
    }
}

/// One texture set of a wearable (ue_material.gd wearable(): primary / secondary)
#[derive(Debug, Clone, Default)]
pub struct WearableSet {
    pub albedo: Option<TexRef>,
    pub normal: Option<TexRef>,
    pub rma: Option<TexRef>,
    pub mask: Option<TexRef>,
    /// ColorA, ColorB, ColorC (linear); None = the master defaults FF0000 / 000000 / FFFFFF
    pub colors: Option<[[f32; 3]; 3]>,
    pub has_emblem: bool,
}

/// Wearable material (SHADERS.md 7, M_EquipmentMasterChestShoulder; ue_material.gd wearable): the runtime inputs UE
/// sets on the dynamic instance. Mask channels R -> ColorA, B -> ColorB, G -> ColorC (059 lines 94-103).
/// emblem = (texture, EmblemColorA, EmblemColorB).
pub fn wearable(primary: &WearableSet, secondary: Option<&WearableSet>, emblem: Option<(TexRef, [f32; 3], [f32; 3])>,
    masked: bool) -> MaterialDesc {
    let mut u: BTreeMap<String, Uniform> = BTreeMap::new();
    let mut set = |n: &str, v: Uniform| {
        u.insert(n.to_string(), v);
    };
    set("ue_mode", Uniform::I(mode_id("M_WEARABLE")));
    set("use_foliage", Uniform::B(false));
    set("use_lightmap", Uniform::B(false));
    set("use_vlm", Uniform::B(false));
    let def = [[1.0, 0.0, 0.0], [0.0; 3], [1.0; 3]];
    let put = |set: &mut dyn FnMut(&str, Uniform), s: &WearableSet, names: [&str; 4], cols: [&str; 3], em: &str| {
        for (t, n) in [&s.albedo, &s.normal, &s.rma, &s.mask].into_iter().zip(names) {
            if let Some(t) = t {
                set(n, Uniform::Tex(t.clone()));
            }
        }
        let c = s.colors.unwrap_or(def);
        for k in 0..3 {
            set(cols[k], Uniform::V3(c[k]));
        }
        set(em, Uniform::F(if s.has_emblem { 1.0 } else { 0.0 }));
    };
    put(&mut set, primary, ["albedo_texture", "normal_texture", "rma_texture", "color_mask"], ["color_a", "color_b", "color_c"],
        "primary_has_emblem");
    set("use_normal", Uniform::B(primary.normal.is_some()));
    if let Some(s) = secondary {
        set("has_secondary", Uniform::B(true));
        put(&mut set, s, ["secondary_albedo", "secondary_normal", "secondary_rma", "secondary_mask"],
            ["secondary_color_a", "secondary_color_b", "secondary_color_c"], "secondary_has_emblem");
    }
    if let Some((t, a, b)) = emblem {
        set("emblem_texture", Uniform::Tex(t));
        set("emblem_color_a", Uniform::V3(a));
        set("emblem_color_b", Uniform::V3(b));
    }
    MaterialDesc {
        package: String::new(),
        master: "M_EquipmentMasterChestShoulder".into(),
        master_package: String::new(),
        chain: vec![],
        mode: "M_WEARABLE".into(),
        blend: if masked { BLEND_MASKED } else { BLEND_OPAQUE },
        two_sided: false,
        foliage: false,
        unlit: false,
        clip: 0.3333,
        layout: String::new(),
        flat: false,
        missing_albedo: None,
        uniforms: u,
    }
}

struct Builder<'r, 'a> {
    r: &'r Resolver<'a>,
    p: &'r Params,
    u: BTreeMap<String, Uniform>,
}

impl Builder<'_, '_> {
    fn s(&mut self, name: &str, key: &str, def: f32) {
        let v = self.p.scalars.get(key).copied().unwrap_or(def);
        self.u.insert(name.into(), Uniform::F(v));
    }
    fn v(&mut self, name: &str, key: &str, def: [f32; 3]) {
        let v = self.p.colors.get(key).copied().unwrap_or(def);
        self.u.insert(name.into(), Uniform::V3(v));
    }
    fn g(&self, key: &str, def: f32) -> f32 {
        self.p.scalars.get(key).copied().unwrap_or(def)
    }
    fn sw(&self, key: &str, def: bool) -> bool {
        self.p.switches.get(key).copied().unwrap_or(def)
    }
    fn set(&mut self, name: &str, v: Uniform) {
        self.u.insert(name.into(), v);
    }
    fn tex(&mut self, name: &str, slot: &str) -> Option<TexRef> {
        let t = self.r.slot_tex(self.p, slot);
        if let Some(t) = &t {
            self.set(name, Uniform::Tex(t.clone()));
        }
        t
    }

    /// The per-master uniforms (ue_material.gd build, `match mode`)
    fn mode(&mut self, mode: &str, k: &Pick) {
        const ONE: [f32; 3] = [1.0; 3];
        const HALF: [f32; 3] = [0.5; 3];
        match mode {
            "M_WEAPON" => {
                self.tex("color_mask", "ColorMap");
                for c in ["A", "B", "C"] {
                    self.v(&format!("color_{}", c.to_lowercase()), &format!("Color{c}"), ONE);
                }
                self.s("metal_tweak", "Metal Tweak", 1.15);
                self.s("roughness_power", "RoughnessPower", 1.0);
                self.s("ao_weaken", "AOWeaken", 0.0);
                // Shipped M_WeaponMaster/r0/086: BloodMask RGBA chooses exact stage/channel;
                // constant blood textures use UV0*4 (VS112), all three are linear.
                let mask = self.tex("weapon_blood_mask", "BloodMask");
                // M_WeaponMaster's shipped default resource includes this branch (r0/086).
                // A MIC without its own static permutation inherits that resource:
                // UMaterialInstance::GetMaterialResource RVA0x31eab20. Cooked root
                // static defaults are stripped; absence is not a false switch.
                // Keep explicit child disables and require the resolved original mask.
                self.set("weapon_blood_on", Uniform::B(self.sw("UseBlood", true) && mask.is_some()));
                self.set("weapon_blood_normal", Uniform::Tex(TexRef("Mordhau/Content/Mordhau/Textures/gore30_n".into())));
                self.set("weapon_blood_grunge", Uniform::Tex(TexRef("Mordhau/Content/Mordhau/Assets/Environment/MaterialFunctions/T_GrungeScratchBlood".into())));
                self.s("blood_stage1", "BloodStage1", 0.0);
                self.s("blood_stage2", "BloodStage2", 0.0);
                self.s("blood_mask_scalar", "BloodMask", 1.0);
                self.s("blood_normal", "BloodNormal", 0.05);
                self.s("blood_metallic", "BloodMetallic", 0.0);
                self.s("blood_albedo_mix", "BloodAlbedoMix", 2.0);
                self.s("blood_roughness", "BloodRoughness", 0.15);
            }
            "M_METALWOOD" => {
                if let Some(c) = &k.clrmask {
                    self.set("color_mask", Uniform::Tex(c.clone()));
                }
                self.v("color_var", "ColorVar", HALF);
                self.v("color_overlay", "ColorOverlay", HALF);
                self.s("color_desat", "ColorDesaturaton", 0.0);
                self.s("ao_bias", "AOBias", 0.0);
                self.s("metal_contrast", "Metal_Contrast", 0.0);
                self.v("metal_color_lerp", "MetalColorLerp", HALF);
                self.s("metal_color_bias", "MetalColorBias", 0.0);
                self.s("rough_lerp_wood", "RoughnessLerp_Wood", 1.0);
                self.s("rough_lerp_wood_bias", "RoughnessLerpWood_BIas", 0.0);
                self.s("metal_roughness", "MetalRoughness", 0.0);
                // static permutations (Barricade_Spikes_01 r0/030): UseGrunge with UseGrungeR / G / B, UseDetailNM?
                let gt = self.tex("mw_grunge_texture", "GrungeTexture");
                let ug = self.sw("UseGrunge", false) && gt.is_some();
                let on = |b: bool| if ug && b { 1.0 } else { 0.0 };
                let gm = [on(self.sw("UseGrungeR", false)), on(self.sw("UseGrungeG", false)), on(self.sw("UseGrungeB", false)), 0.0];
                self.set("mw_grunge", Uniform::V4(gm));
                let sc = [self.g("Scale_R", 1.0), self.g("Scale_G", 1.0), self.g("Scale_B", 1.0)];
                self.set("mw_scale", Uniform::V3(sc));
                let ct = [self.g("Contrast_R", 0.0), self.g("Contrast_G", 0.0), self.g("Contrast_B", 0.0)];
                self.set("mw_contrast", Uniform::V3(ct));
                self.v("mw_color_r", "Color_R", ONE);
                self.v("mw_color_g", "Color_G", ONE);
                self.v("mw_color_b", "Color_B", ONE);
                self.s("mw_overall_bias", "OverallBias", 1.0);
                self.s("mw_limit_metal", "LimitGrungeToMetal", 0.0);
                let dn = self.tex("detail_normal", "Detail_NM");
                self.set("mw_detail", Uniform::B(self.sw("UseDetailNM?", false) && dn.is_some()));
                self.s("mw_detail_scale", "Detail_Scale", 1.0);
                self.s("mw_normal_flatten", "NormalFlatten", 0.0);
            }
            "M_LAYERED" | "M_ARENABLEND" => {
                self.layers(k);
                if mode == "M_LAYERED" {
                    self.s("ao_bias", "AO_Bias", 0.0);
                    let gr = self.r.slot_tex(self.p, "GrungeTexture");
                    let ug = self.sw("UseGrunge", false) && self.sw("UseGrungeB", false) && gr.is_some();
                    self.set("use_grunge", Uniform::B(ug));
                    if let Some(g) = gr {
                        let srgb = self.r.tex_srgb(&g);
                        self.set("grunge_texture", Uniform::Tex(g));
                        self.set("grunge_srgb", Uniform::B(srgb));
                    }
                    self.s("grunge_scale", "Scale", 1.0);
                    self.s("contrast_b", "Contrast_B", 1.0);
                    self.v("grunge_color_b", "Color_B", ONE);
                    self.s("overall_bias", "OverallBias", 0.0);
                    self.s("metallic_stain", "Metallic_Stain", 0.0);
                    self.s("roughness_stain", "Roughness_Stain", 0.9);
                    self.s("spec_04", "Spec_04", 0.5);
                } else {
                    self.paint();
                    self.s("paint_flatten_normal", "PaintFlattenNormal", 0.5);
                }
            }
            "M_ARCH" => {
                self.paint();
                self.tex("rhao_detail", "RHAODetail");
                let nd = self.tex("normal_detail", "NormalDetail");
                self.set("use_normal_detail", Uniform::B(nd.is_some() && k.normal.is_some()));
                let ns = self.p.colors.get("NonTriplanarNormalUVScale").copied().unwrap_or(ONE);
                self.set("normal_detail_scale", Uniform::V2([ns[0], ns[1]]));
                self.tex("albedo_detail", "AlbedoDetail");
                let rs = self.p.colors.get("NonTriplanarRHAOUVScale").copied().unwrap_or(ONE);
                let cs = self.p.colors.get("NonTriplanarColorUVScale").copied().unwrap_or(ONE);
                self.set("rhao_detail_scale", Uniform::V2([rs[0], rs[1]]));
                self.set("albedo_detail_scale", Uniform::V2([cs[0], cs[1]]));
                self.s("a_desaturation", "Desaturation", 0.0);
                self.v("a_color", "Color", ONE);
                self.s("detail_power", "Detail_Power", 1.0);
                self.s("desaturation_detail", "Desaturation_Detail", 0.0);
                self.v("albedo_color", "AlbedoColor", ONE);
                self.s("overlay_bias", "Bias", 0.0);
                self.v("curvature_color", "CurvatureColor", ONE);
                self.v("dirt_color", "DirtColor", HALF);
                self.s("dirt_power", "DirtPower", 3.0);
                self.s("dirt_contrast", "Dirt_Contrast_01", 0.4);
                self.s("damage_desaturation", "Damage_Desaturation", 0.0);
                self.v("color_damage", "ColorDamage", HALF);
                self.s("roughness_bias", "RoughnessBias", 0.5);
                self.s("ao_bias", "AO_Bias", 1.0);
                self.s("bias_normal", "Bias_Normal", 0.0);
            }
            "M_ROUNDBANNER" => {
                if let Some(t) = self.p.tex.iter().find(|t| {
                    t.0 == t.1.name() && t.0.to_lowercase().ends_with("_c") && self.r.texture_exists(&t.1)
                }) {
                    self.set("color_mask", Uniform::Tex(t.1.clone()));
                }
                self.v("color_a", "ColorA", ONE);
                self.v("color_b", "ColorB", ONE);
                self.s("bleed", "Bleed", 1.5);
                self.s("albedo_scale", "Albedo", 0.75);
                self.s("roughness_scale", "Roughness", 1.0);
                self.tex("emblem_texture", "EmblemTexture");
                self.v("emblem_color_a", "EmblemColorA", ONE);
                self.v("emblem_color_b", "EmblemColorB", [0.0, 0.0, 1.0]);
                self.v("sss", "SSS", ONE);
            }
            "M_CLOTH" => {
                let tent = self.p.master == "Tent_Cloth_Master";
                self.tex("albedo_texture", "Albedo");
                let tile = [self.g("Tile_Amount_02", 1.0), self.g(if tent { "TileAmount_Detail" } else { "TileAmount" }, 1.0)];
                self.set("cloth_tile", Uniform::V2(tile));
                self.s("cloth_desat", "Desaturation", 0.0);
                self.v("cloth_mul", "ColorMultiply", ONE);
                self.tex("detail_albedo", "linencloth_basecolor");
                self.tex("detail_normal", "linencloth_normal");
                if let Some(g) = self.r.slot_tex(self.p, if tent { "GrungeTexture" } else { "Grunge_Tex" }) {
                    let srgb = self.r.tex_srgb(&g);
                    self.set("cloth_grunge", Uniform::Tex(g));
                    self.set("cloth_grunge_srgb", Uniform::B(srgb));
                }
                self.set("cloth_grunge_uv1", Uniform::B(!tent));
                self.s("cloth_grunge_scale", if tent { "Scale_R" } else { "Grunge_Scale" }, 1.0);
                let c = if tent { self.g("Contrast_R", 0.0) } else { 0.0 };
                self.set("cloth_grunge_contrast", Uniform::F(c));
                self.v("cloth_grunge_color", if tent { "Color_R" } else { "Grunge_Color" }, ONE);
                let b = if tent { self.g("OverallBias", 0.0) } else { 1.0 };
                self.set("cloth_grunge_bias", Uniform::F(b));
                self.s("cloth_nm_bias", if tent { "NM_Strength_Detail" } else { "NM_Strength_Bias" }, 0.0);
                self.tex("rma_texture", if tent { "RMA" } else { "RHAO" });
            }
            "M_BANNER" => {
                self.tex("detail_albedo", "linencloth_basecolor");
                self.tex("rma_texture", "Banner_RMA");
                let bn = self.tex("normal_texture", "Banner_Normal");
                self.set("use_normal", Uniform::B(bn.is_some()));
                if let Some(b) = self.r.slot_tex(self.p, "BannerID") {
                    let srgb = self.r.tex_srgb(&b);
                    self.set("banner_mask", Uniform::Tex(b));
                    self.set("banner_mask_srgb", Uniform::B(srgb));
                }
                if let Some(g) = self.r.slot_tex(self.p, "GrungeTexture") {
                    let srgb = self.r.tex_srgb(&g);
                    self.set("cloth_grunge", Uniform::Tex(g));
                    self.set("cloth_grunge_srgb", Uniform::B(srgb));
                }
                let ds = [self.g("U_Scale_Detail", 5.0), self.g("V_Scale_Detail", 5.0)];
                self.set("banner_detail_scale", Uniform::V2(ds));
                self.v("color_a", "ColorA", [1.0, 0.0, 0.0]);
                self.v("color_b", "ColorB", [0.0, 0.0, 1.0]);
                self.s("bleed", "Bleed", 1.5);
                self.s("banner_scale_r", "Scale_R", 0.0);
                self.s("banner_contrast_r", "Contrast_R", 1.0);
                self.v("banner_color_r", "Color_R", ONE);
                self.s("banner_scale_b", "Scale_B", 0.0);
                self.s("banner_contrast_b", "Contrast_B", 1.0);
                self.v("banner_color_b", "Color_B", ONE);
                self.s("banner_bias", "OverallBias", 0.0);
                self.s("banner_sss", "SSS", 0.3);
            }
            "M_GRASSCAST" | "M_FERN" => {
                self.tex("albedo_texture", if mode == "M_GRASSCAST" { "Grass_01" } else { "T_Plains_Fern01_D" });
                if mode == "M_FERN" {
                    let n = self.tex("normal_texture", "T_Plains_Fern01_N");
                    self.set("use_normal", Uniform::B(n.is_some()));
                }
            }
            "M_FLORA" => {
                self.s("flora_tiling", "TextureTiling", 1.0);
                self.v("flora_color", "Color", ONE);
                self.s("flora_color_lerp", "ColorLerp", 0.0);
                self.s("flora_desat", "desaturation", 0.0);
                self.s("flora_hsl", "HSL", 0.0);
                self.v("flora_mul", "ColorMultiply", ONE);
                self.s("flora_emissive", "Emissive", 0.0);
                self.v("flora_sub", "SubColor", ONE);
                self.s("flora_ss_bias", "SS_Bias", 0.0);
                self.s("flora_metallic", "metallic", 0.0);
                self.s("flora_spec", "spec", 0.0);
                self.s("flora_rough", "Roughness", 0.0);
                self.s("flora_nm_flat", "NM_Flatness", 0.0);
            }
            // M_MainMenuCircle (the main menu / Armory floor; state/shaders/M_MainMenuCircle/r0/002 75-93): EmissiveColor
            // = Lerp(Dark, Light, RadialGradientExponential) with the vector parameters Light / Dark (preshaders: cb1[3] =
            // Light.rgb, cb1[4] = Dark.rgb; cb1[5] / cb1[6].x = SelectionColor.rgb / .a = 0, the editor selection lerp)
            "M_MAINMENUCIRCLE" => {
                self.v("color_a", "Light", [0.06375, 0.075437, 0.085]);
                self.v("color_b", "Dark", [0.0225, 0.026625, 0.03]);
            }
            "M_SANDBAGS" => {
                // SandBags_M/r0 (no static switches): the master's ReferencedTextures [4] linencloth overlay at UV x 6,
                // GrungeTexture (master default Grunge_Dirt), channel R only
                self.s("sb_normal_strength", "Normal_Strength", 1.0);
                self.v("sb_color", "Color", ONE);
                self.s("sb_color_lerp", "ColorLerp", 0.0);
                self.s("sb_ao_in_bias", "AO_InBias", 0.0);
                self.s("sb_roughness", "Roughness", 1.0);
                self.s("sb_ao_bias", "AO_Bias", 0.0);
                if self.tex("albedo_detail", "linencloth_basecolor").is_none() {
                    self.set("albedo_detail", Uniform::Tex(TexRef("Mordhau/Content/Mordhau/Maps/AndrewG_Testing_Map/AG_Art/Assets/Castle/Castle2017/Castle_Materials/Textures/linencloth_basecolor".into())));
                }
                let gt = self.tex("mw_grunge_texture", "GrungeTexture").or_else(|| self.tex("mw_grunge_texture", "Grunge_Dirt"));
                self.set("mw_grunge", Uniform::V4([if gt.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]));
                self.set("mw_scale", Uniform::V3([self.g("Scale_R", 1.0), 1.0, 1.0]));
                self.set("mw_contrast", Uniform::V3([self.g("Contrast_R", 0.0), 0.0, 0.0]));
                self.v("mw_color_r", "Color_R", ONE);
                self.s("mw_overall_bias", "OverallBias", 1.0);
            }
            "M_SHIPSWOOD" => {
                // Plks_Nails_1to1/r0/119: no opacity clip in its depth-only PS (126) although the instance is masked;
                // the albedo alpha is the HasMetal metallic mask, not an opacity
                self.set("alpha_from_albedo", Uniform::B(false));
                self.set("alpha_value", Uniform::F(1.0));
                self.set("sw_scale", Uniform::V2([self.g("ScaleTextures", 1.0), self.g("ScaleTextures_DetailNM", 1.0)]));
                self.set("sw_nm_flat", Uniform::V2([self.g("NM_Flatness", 0.0), self.g("NM_Flatness_Detail", 0.0)]));
                let dn = self.tex("detail_normal", "NormalDetailMap");
                self.set("sw_detail", Uniform::B(dn.is_some()));
                self.s("sw_desat", "Desaturation", 0.0);
                self.v("sw_color_mul", "ColorMultiply", ONE);
                self.v("sw_color_lerp", "Color_Lerp", ONE);
                self.s("sw_color_lerp_bias", "ColorLerpBias", 0.0);
                self.v("sw_vc_color", "VertexColor_A", ONE);
                self.s("sw_vc_bias", "VertexColor_Bias", 0.0);
                self.s("sw_rough_mod", "RoughnessModulate", 0.0);
                self.s("sw_vertex_rough", "VertexRoughness", 0.0);
                self.s("sw_vertex_rough_bias", "Vertex_RoughnessBias", 0.0);
                self.s("sw_ao_bias", "AO_Bias", 0.0);
                self.set("sw_has_metal", Uniform::B(self.sw("HasMetal", false)));
                let gt = self.tex("mw_grunge_texture", "GrungeTexture");
                let ug = self.sw("UseGrunge", false) && self.sw("UseGrungeR", false) && gt.is_some();
                self.set("mw_grunge", Uniform::V4([if ug { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]));
                self.set("mw_scale", Uniform::V3([self.g("Scale_R", 1.0), 1.0, 1.0]));
                self.set("mw_contrast", Uniform::V3([self.g("Contrast_R", 0.0), 0.0, 0.0]));
                self.v("mw_color_r", "Color_R", ONE);
                self.s("mw_overall_bias", "OverallBias", 1.0);
            }
            "M_DECALDIRT" => {
                self.tex("color_mask", "treasuretrims_CrvMaskDirt");
            }
            "M_HLOD" => {
                let dt = self.r.slot_tex(self.p, "DiffuseTexture");
                self.set("use_diffuse_tex", Uniform::B(self.sw("UseDiffuse", false) && dt.is_some()));
                if let Some(t) = dt {
                    self.set("albedo_texture", Uniform::Tex(t));
                }
                if let Some(t) = self.r.slot_tex(self.p, "NormalTexture") {
                    self.set("normal_texture", Uniform::Tex(t));
                    let un = self.sw("UseNormal", true);
                    self.set("use_normal", Uniform::B(un));
                }
                self.v("diffuse_const", "DiffuseConst", [0.0; 3]);
                self.s("metallic_const", "MetallicConst", 0.0);
                self.s("specular_const", "SpecularConst", 0.0);
                self.s("roughness_const", "RoughnessConst", 0.9);
                self.s("ao_const", "AmbientOcclusionConst", 1.0);
            }
            "M_MEGAFOLIAGE" => {
                self.v("albedo_tint", "Albedo", ONE);
                self.s("mf_desaturation", "Desaturation", 0.0);
                self.s("hue_shift", "Hue Shift", 0.0);
                self.s("mf_emissive", "Emissive", 0.0);
                self.s("top_power", "TopPower", 0.5);
                self.s("top_darken", "TopDarken", 0.5);
                self.s("mf_roughness", "Roughness", 1.0);
                self.v("mf_sss", "SSS", ONE);
                self.s("mf_specular", "SpecMax", 0.2);
                if let Some(t) = self.r.slot_tex(self.p, "Translucency Texture") {
                    let srgb = self.r.tex_srgb(&t);
                    self.set("translucency_texture", Uniform::Tex(t));
                    self.set("translucency_srgb", Uniform::B(srgb));
                }
            }
            "M_BUSH" => {
                if let Some(t) = self.r.slot_tex(self.p, "T_Foliage_Variation_Mask") {
                    let srgb = self.r.tex_srgb(&t);
                    self.set("variation_texture", Uniform::Tex(t));
                    self.set("variation_srgb", Uniform::B(srgb));
                }
                self.s("saturation", "Saturation", 0.0);
                self.v("bush_color", "Color", ONE);
                self.s("brightness", "Brightness", 1.0);
                self.s("variation_tiling", "Variation_Tiling", 5000.0);
                self.s("variation_intensity", "Variation_Intensity", 0.1);
                self.s("ao_power", "AO", 1.0);
                self.s("emissive", "Emissive", 0.0);
                self.s("roughness_power", "Roughness", 2.0);
                self.s("specular_value", "Specular", 0.15);
            }
            _ => {}
        }
    }

    /// ue_material.gd _layers (SHADERS.md 6a/6b)
    fn layers(&mut self, k: &Pick) {
        let t4 = [self.g("Tiling_01", 1.0), self.g("Tiling_02", 1.0), self.g("Tiling_03", 1.0), self.g("Tiling_04", 1.0)];
        self.set("tiling", Uniform::V4(t4));
        let hc = [self.g("HeightContrast01", 0.0), self.g("HeightContrast02", 0.0), self.g("HeightContrast03", 0.0), 0.0];
        self.set("height_contrast", Uniform::V4(hc));
        let ad = [0.0, self.g("Add_02", 0.0), self.g("Add_03", 0.0), self.g("Add_04", 0.0)];
        self.set("add", Uniform::V4(ad));
        let ct = [0.0, self.g("Contrast_02", 0.0), self.g("Contrast_03", 0.0), self.g("Contrast_04", 0.0)];
        self.set("contrast", Uniform::V4(ct));
        let rm = [self.g("01_RoughnessMultiply_01", 1.0), self.g("01_RoughnessMultiply_02", 1.0),
            self.g("01_RoughnessMultiply_03", 1.0), self.g("01_RoughnessMultiply_04", 1.0)];
        self.set("rough_mul", Uniform::V4(rm));
        self.v("mul_1", "01_Color_Multiply_01", [1.0; 3]);
        self.v("overlay", "ColorOverlay", [1.0; 3]);
        self.s("desaturation", "Desaturation", 0.0);
        let bp = [self.g("BumpValue", 0.0), self.g("BumpValue_02", 0.0), self.g("BumpValue_03", 0.0), self.g("BumpValue_04", 0.0)];
        self.set("bump", Uniform::V4(bp));
        self.s("bump_offset_power", "BumpOffsetPower", 1.0);
        let mut n = 1;
        for (i, l) in k.layers.iter().enumerate() {
            let li = i + 2;
            if !self.sw(&format!("UseLayer{li}"), false) {
                break;
            }
            self.set(&format!("albedo_{li}"), Uniform::Tex(l.0.clone()));
            if let Some(t) = &l.1 {
                self.set(&format!("normal_{li}"), Uniform::Tex(t.clone()));
            }
            if let Some(t) = &l.2 {
                self.set(&format!("packed_{li}"), Uniform::Tex(t.clone()));
            }
            self.v(&format!("mul_{li}"), &format!("01_Color_Multiply_0{li}"), [1.0; 3]);
            n = li;
        }
        self.set("layer_count", Uniform::I(n as i32));
    }

    /// ue_material.gd _paint (static switch "UsePaintLayer?", SHADERS.md 6b/6c)
    fn paint(&mut self) {
        if !self.sw("UsePaintLayer?", false) {
            return;
        }
        self.set("use_paint", Uniform::B(true));
        self.s("paint_power", "PaintPower", 0.0);
        self.s("paint_contrast1", "PaintContrast_Color1", 0.0);
        self.s("paint_contrast2", "PaintContrast_Color2", 0.0);
        self.s("curvature_cc", "CurvatureColor", 1.0);
        self.s("mask_output_level", "MaskOutputLevel", 0.5);
        self.s("paint_roughness", "PaintRoughness", 0.0);
        self.s("dhi", "DetailLayer_HeightInfluence", 0.5);
        self.v("color_inner", "Color_Inner", [1.0; 3]);
        self.v("color_outer", "Color_Outer", [1.0, 0.0, 0.0]);
        self.v("color_curvaturer", "Color_Curvaturer", [1.0; 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_blood_greatsword_resolves_original_linear_masks_and_defaults() {
        let vfs = std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("mount original paks"));
        let rd = Reader::new(vfs.clone());
        let src = crate::pak_source::PakSource::new(vfs);
        let r = Resolver::new(&rd, &src);
        let d = r.build("Mordhau/Content/Mordhau/Assets/Weapons/Greatsword/Instanced_materials/M_Greatswords_01").expect("original Greatsword MIC");
        assert_eq!(d.uniforms.get("weapon_blood_on"), Some(&Uniform::B(true)));
        let g = crate::shader::gpu(&d);
        for (name, expected) in [("blood_stage1", 0.0),("blood_stage2",0.0),("blood_mask_scalar",1.0),
            ("blood_normal",0.05),("blood_metallic",0.0),("blood_albedo_mix",2.0),("blood_roughness",0.15)] {
            assert!((g.get(name)[0]-expected).abs()<1e-6, "{name}: {:?}", g.get(name));
        }
        for slot in 4..=6 {
            let crate::shader::SlotBinding::Texture { tex, srgb, .. } = &g.slots[slot] else { panic!("blood slot{slot} unresolved") };
            assert!(!srgb);
            let info = crate::texture::info(&src, &tex.0, None).expect("original blood texture");
            assert!(!info.srgb, "{} must be linear", tex.0);
            let mip = &info.mips[0];
            let data = crate::texture::mip_data(&src, &info, 0).expect("original mip data");
            let pixels = crate::texture::decode(info.format.unwrap(), mip.size_x as usize, mip.size_y as usize, &data).expect("decode original blood texture");
            let crate::texture::Pixels::Rgba8(rgba) = pixels else { panic!("expected original LDR blood texture") };
            assert!(rgba.chunks_exact(4).any(|p| p != &rgba[..4]), "{} has actual coverage/detail", tex.0);
        }
    }

    #[test]
    fn weapon_blood_commander_inherits_original_parent_shader_without_static_override() {
        let vfs = std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("mount original paks"));
        let rd = Reader::new(vfs.clone());
        let src = crate::pak_source::PakSource::new(vfs);
        let r = Resolver::new(&rd, &src);
        let path = "Mordhau/Content/Mordhau/Assets/Weapons/Zweihander/M_commander_zweihander";
        let p = r.params(path).expect("original Commander MIC");
        assert_eq!(p.master, "M_WeaponMaster");
        assert!(!p.switches.contains_key("UseBlood"), "This case inherits the compiled parent resource");
        let d = r.build(path).expect("original Commander weapon material");
        let g = crate::shader::gpu(&d);
        assert_eq!(g.get("weapon_blood_on")[0], 1.0);
        let expected = ["Mordhau/Content/Mordhau/Assets/Weapons/Zweihander/T_commanderzweihander_bloodmask",
            "Mordhau/Content/Mordhau/Textures/gore30_n",
            "Mordhau/Content/Mordhau/Assets/Environment/MaterialFunctions/T_GrungeScratchBlood"];
        for (slot, path) in (4..=6).zip(expected) {
            let crate::shader::SlotBinding::Texture { tex, srgb, .. } = &g.slots[slot] else { panic!("blood slot{slot} unresolved") };
            assert_eq!(tex.0, path);
            assert!(!srgb);
            assert!(!crate::texture::info(&src, path, None).expect("original texture").srgb);
        }
    }

    #[test]
    fn cue_regexes() {
        assert!(is_diffuse("T_Wall_BaseColor") && is_diffuse("x_D") && is_diffuse("AlbedoMap") && is_diffuse("Foo_Tex"));
        assert!(!is_diffuse("T_Wall_N"));
        assert!(is_normals("NormalMap") && is_normals("NO_x") && is_normals("wall_nrm"));
        assert!(is_specmasks("T_RMA") && !is_specmasks("RoughnessMap") && !is_specmasks("NoRmal"));
        assert!(is_specmasks("x_s") && is_specmasks("SP_a") && is_specmasks("Metal_MR_x"));
        assert!(is_emissive("EmissiveMap") && is_emissive("t_em"));
    }

    #[test]
    fn suffix_roles_and_layouts() {
        assert_eq!(suffix_role("WoodWall_RHAO"), "RHAO");
        assert_eq!(suffix_role("prop_barrel_01_r"), "RMA_R");
        assert_eq!(suffix_role("x_basecolor"), "A");
        assert_eq!(layout("SRMH").5, 1.0);
        assert_eq!(strip_index("A/B.12"), "A/B");
        assert_eq!(strip_index("A/B.x"), "A/B.x");
    }

    #[test]
    fn verify_keeps_dictionary_order() {
        let mut c = CueParams::default();
        c.verify("AlbedoMap", "a", true);
        c.verify("Diffuse2", "b", true);
        let k: Vec<&str> = c.textures.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(k, ["PM_Diffuse", "AlbedoMap", "Diffuse2"]);
        assert_eq!(c.textures[0].1, "b");
    }
}

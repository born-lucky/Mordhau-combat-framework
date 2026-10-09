//! The ue_tint material shader, engine-neutral: the GPU data layout of shaders/ue_tint.wgsl (`UNIFORMS`, `TEX_SLOTS`,
//! `gpu`) and a CPU reference of its math (`surface`, `lightmap`), line for line with the WGSL and with
//! godot/game/render/ue_tint.gdshaderinc (which cites the compiled UE shaders, docs/SHADERS.md and state/shaders/).
//! The CPU reference is what the unit tests check against hand-computed values; the WGSL is validated with naga.

use crate::material::{MaterialDesc, TexRef, Uniform};

/// Locally imported shader source. The distribution never embeds or supplies this data.
pub fn ue_tint_wgsl() -> &'static str {
    static SOURCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SOURCE.get_or_init(|| {
        let root = std::env::var_os("MORDHAU_LOCAL_DATA").expect("local-import cache required");
        std::fs::read_to_string(std::path::PathBuf::from(root).join("shaders/ue_tint.wgsl"))
            .expect("locally reconstructed material shader missing")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Int,
    Bool,
    Float,
    Vec2,
    Vec3,
    Vec4,
}

include!("shader_uniforms.rs");

/// Index of a uniform in `UNIFORMS` (= its vec4 slot in the WGSL struct UeTint)
pub fn uniform_index(name: &str) -> Option<usize> {
    UNIFORMS.iter().position(|u| u.0 == name)
}

/// Default image for an unbound texture slot (Godot hints hint_default_white / hint_default_black / hint_normal)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexDefault {
    White,
    Black,
    /// (0.5, 0.5, 1, 1)
    Normal,
}

/// Every texture uniform of the GDScript shader: (name, WGSL slot t0..t14, sRGB view (Godot `source_color`), default).
/// Names sharing a slot are never used by the same master mode (`mode_textures`).
pub const TEX_SLOTS: &[(&str, usize, bool, TexDefault)] = &[
    ("albedo_texture", 0, true, TexDefault::White),
    ("normal_texture", 1, false, TexDefault::Normal),
    ("rma_texture", 2, false, TexDefault::White),
    ("color_mask", 3, false, TexDefault::Black),
    ("grunge_texture", 3, false, TexDefault::Black),
    ("weapon_blood_mask", 4, false, TexDefault::Black),
    ("weapon_blood_normal", 5, false, TexDefault::Normal),
    ("weapon_blood_grunge", 6, false, TexDefault::Black),
    ("albedo_2", 4, true, TexDefault::White),
    ("rhao_detail", 4, false, TexDefault::White),
    ("secondary_albedo", 4, true, TexDefault::White),
    ("detail_albedo", 4, true, TexDefault::White),
    ("translucency_texture", 4, false, TexDefault::Black),
    ("variation_texture", 4, false, TexDefault::Black),
    ("normal_2", 5, false, TexDefault::Normal),
    ("albedo_detail", 5, true, TexDefault::White),
    ("secondary_normal", 5, false, TexDefault::Normal),
    ("detail_normal", 5, false, TexDefault::Normal),
    ("mw_grunge_texture", 4, false, TexDefault::Black),
    ("packed_2", 6, false, TexDefault::White),
    ("normal_detail", 6, false, TexDefault::Normal),
    ("secondary_rma", 6, false, TexDefault::White),
    ("cloth_grunge", 6, false, TexDefault::Black),
    ("albedo_3", 7, true, TexDefault::White),
    ("secondary_mask", 7, false, TexDefault::Black),
    ("banner_mask", 7, false, TexDefault::Black),
    ("normal_3", 8, false, TexDefault::Normal),
    ("emblem_texture", 8, false, TexDefault::Black),
    ("packed_3", 9, false, TexDefault::White),
    ("albedo_4", 10, true, TexDefault::White),
    ("normal_4", 11, false, TexDefault::Normal),
    ("packed_4", 12, false, TexDefault::White),
    ("lightmap_texture", 13, false, TexDefault::Black),
    ("sky_occlusion_texture", 14, false, TexDefault::White),
];
pub const NUM_TEX_SLOTS: usize = 15;

/// The texture uniforms a mode's code samples (ue_tint.wgsl / gdshaderinc branches), lightmap slots included
pub fn mode_textures(mode: i32) -> &'static [&'static str] {
    const LM: [&str; 2] = ["lightmap_texture", "sky_occlusion_texture"];
    let _ = LM;
    match mode {
        1 => &["albedo_texture", "normal_texture", "rma_texture", "color_mask", "weapon_blood_mask", "weapon_blood_normal", "weapon_blood_grunge", "lightmap_texture", "sky_occlusion_texture"],
        2 => &["albedo_texture", "normal_texture", "rma_texture", "color_mask", "mw_grunge_texture", "detail_normal",
            "lightmap_texture", "sky_occlusion_texture"],
        3 => &["albedo_texture", "normal_texture", "rma_texture", "grunge_texture", "albedo_2", "normal_2", "packed_2", "albedo_3",
            "normal_3", "packed_3", "albedo_4", "normal_4", "packed_4", "lightmap_texture", "sky_occlusion_texture"],
        4 => &["albedo_texture", "normal_texture", "rma_texture", "albedo_2", "normal_2", "packed_2", "lightmap_texture",
            "sky_occlusion_texture"],
        5 => &["albedo_texture", "normal_texture", "rma_texture", "rhao_detail", "albedo_detail", "normal_detail",
            "lightmap_texture", "sky_occlusion_texture"],
        7 => &["albedo_texture", "normal_texture", "rma_texture", "color_mask", "secondary_albedo", "secondary_normal",
            "secondary_rma", "secondary_mask", "emblem_texture", "lightmap_texture", "sky_occlusion_texture"],
        8 => &["albedo_texture", "normal_texture", "rma_texture", "color_mask", "emblem_texture", "lightmap_texture",
            "sky_occlusion_texture"],
        9 => &["albedo_texture", "normal_texture", "rma_texture", "detail_albedo", "detail_normal", "cloth_grunge",
            "lightmap_texture", "sky_occlusion_texture"],
        10 => &["normal_texture", "rma_texture", "detail_albedo", "cloth_grunge", "banner_mask", "lightmap_texture",
            "sky_occlusion_texture"],
        14 => &["albedo_texture", "normal_texture", "color_mask", "lightmap_texture", "sky_occlusion_texture"],
        16 => &["albedo_texture", "normal_texture", "rma_texture", "translucency_texture", "lightmap_texture",
            "sky_occlusion_texture"],
        17 => &["albedo_texture", "normal_texture", "variation_texture", "lightmap_texture", "sky_occlusion_texture"],
        19 => &["albedo_texture", "normal_texture", "rma_texture", "mw_grunge_texture", "albedo_detail", "lightmap_texture",
            "sky_occlusion_texture"],
        18 => &["albedo_texture", "normal_texture", "rma_texture", "mw_grunge_texture", "detail_normal", "lightmap_texture",
            "sky_occlusion_texture"],
        _ => &["albedo_texture", "normal_texture", "rma_texture", "color_mask", "lightmap_texture", "sky_occlusion_texture"],
    }
}

/// One texture slot of the bind group
#[derive(Debug, Clone, PartialEq)]
pub enum SlotBinding {
    /// the texture (create an sRGB view when `srgb`)
    Texture { tex: TexRef, srgb: bool, uniform: &'static str },
    Default(TexDefault),
}

/// GPU data of a material: the UeTint uniform buffer (one vec4 per `UNIFORMS` entry, defaults where the material sets
/// nothing) and the 15 texture slots
#[derive(Debug, Clone)]
pub struct Gpu {
    pub uniforms: Vec<[f32; 4]>,
    pub slots: Vec<SlotBinding>,
}

impl Gpu {
    /// The uniform buffer as bytes (std140-compatible: 16-byte vec4 slots)
    pub fn uniform_bytes(&self) -> Vec<u8> {
        self.uniforms.iter().flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes())).collect()
    }
    pub fn get(&self, name: &str) -> [f32; 4] {
        uniform_index(name).map_or([0.0; 4], |i| self.uniforms[i])
    }
    pub fn set(&mut self, name: &str, v: [f32; 4]) {
        if let Some(i) = uniform_index(name) {
            self.uniforms[i] = v;
        }
    }
}

/// Uniform buffer + texture slots of a resolved material. Render state (blend / two-sided) stays on the MaterialDesc
/// (pipeline state, the Godot variants ue_tint_<masked|translucent>[_2s]).
pub fn gpu(m: &MaterialDesc) -> Gpu {
    let mut g = Gpu { uniforms: UNIFORMS.iter().map(|u| u.2).collect(), slots: Vec::new() };
    for (k, v) in &m.uniforms {
        let x = match v {
            Uniform::F(f) => [*f, 0.0, 0.0, 0.0],
            Uniform::I(i) => [*i as f32, 0.0, 0.0, 0.0],
            Uniform::B(b) => [if *b { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            Uniform::V2(v) => [v[0], v[1], 0.0, 0.0],
            Uniform::V3(v) => [v[0], v[1], v[2], 0.0],
            Uniform::V4(v) => *v,
            Uniform::Tex(_) => continue,
        };
        g.set(k, x);
    }
    let mode = g.get("ue_mode")[0].round() as i32;
    let used = mode_textures(mode);
    for s in 0..NUM_TEX_SLOTS {
        let names: Vec<&(&str, usize, bool, TexDefault)> =
            TEX_SLOTS.iter().filter(|t| t.1 == s && used.contains(&t.0)).collect();
        let bound = names.iter().find_map(|t| match m.uniforms.get(t.0) {
            Some(Uniform::Tex(x)) => Some(SlotBinding::Texture { tex: x.clone(), srgb: t.2, uniform: t.0 }),
            _ => None,
        });
        let def = names.first().map_or(TexDefault::White, |t| t.3);
        g.slots.push(bound.unwrap_or(SlotBinding::Default(def)));
    }
    g
}

// ---- CPU reference --------------------------------------------------------------------------------------------------

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
fn sc(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn sp(s: f32) -> V3 {
    [s; 3]
}
fn dot3(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn mix3(a: V3, b: V3, t: f32) -> V3 {
    add(a, sc(sub(b, a), t))
}
fn mixv(a: V3, b: V3, t: V3) -> V3 {
    [a[0] + (b[0] - a[0]) * t[0], a[1] + (b[1] - a[1]) * t[1], a[2] + (b[2] - a[2]) * t[2]]
}
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}
fn sat3(a: V3) -> V3 {
    [sat(a[0]), sat(a[1]), sat(a[2])]
}
fn xyz(v: [f32; 4]) -> V3 {
    [v[0], v[1], v[2]]
}
fn norm3(a: V3) -> V3 {
    let l = dot3(a, a).sqrt();
    sc(a, 1.0 / l)
}
fn cross3(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
/// sRGB -> linear (IEC 61966-2-1)
pub fn s2l(c: V3) -> V3 {
    c.map(|x| if x >= 0.04045 { ((x + 0.055) / 1.055).powf(2.4) } else { x / 12.92 })
}
/// UE CheapContrast
pub fn cc(x: f32, c: f32) -> f32 {
    sat(mix(-c, 1.0 + c, x))
}
pub fn luma(c: V3) -> f32 {
    dot3(c, [0.3, 0.59, 0.11])
}
fn step3(e: f32, x: V3) -> V3 {
    x.map(|v| if v >= e { 1.0 } else { 0.0 })
}
fn overlay(a: V3, b: V3) -> V3 {
    mixv(mul(sc(a, 2.0), b), sub(sp(1.0), mul(sc(sub(sp(1.0), a), 2.0), sub(sp(1.0), b))), step3(0.5, a))
}
fn tsn(xy: [f32; 2]) -> V3 {
    let v = [xy[0] * 2.0 - 1.0, xy[1] * 2.0 - 1.0];
    [v[0], v[1], (1.0 - v[0] * v[0] - v[1] * v[1]).max(0.0).sqrt()]
}
fn rnm(nn: V3, dn: V3) -> V3 {
    let t = add(nn, [0.0, 0.0, 1.0]);
    let w = mul(dn, [-1.0, -1.0, 1.0]);
    sub(sc(t, dot3(t, w)), sc(w, t[2]))
}
fn hue_rot(c: V3, th: f32) -> V3 {
    let k = sp(0.57735);
    let pk = dot3(k, c);
    let perp = sub(c, sc(k, pk));
    add(add(sc(perp, th.cos()), sc(cross3(k, perp), th.sin())), sc(k, pk))
}

/// Texture fetches for the CPU reference: slot t0..t14 at uv, as the shader sees them (sRGB slots already decoded)
pub trait Sampler {
    fn sample(&self, slot: usize, uv: [f32; 2]) -> [f32; 4];
}

/// Interpolated inputs (ue_tint.wgsl UeTintIn)
#[derive(Debug, Clone, Copy)]
pub struct SurfaceIn {
    pub uv: [f32; 2],
    pub uv2: [f32; 2],
    pub custom0: [f32; 2],
    pub color: [f32; 4],
    pub world_pos: V3,
    pub n: V3,
    pub t: V3,
    pub b: V3,
    pub v: V3,
}

impl Default for SurfaceIn {
    fn default() -> Self {
        SurfaceIn {
            uv: [0.5; 2],
            uv2: [0.5; 2],
            custom0: [0.0; 2],
            color: [1.0; 4],
            world_pos: [0.0; 3],
            n: [0.0, 1.0, 0.0],
            t: [1.0, 0.0, 0.0],
            b: [0.0, 0.0, 1.0],
            v: [0.0, 1.0, 0.0],
        }
    }
}

/// ue_tint.wgsl UeTintOut
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SurfaceOut {
    pub base: V3,
    pub alpha: f32,
    pub roughness: f32,
    pub metallic: f32,
    pub specular: f32,
    pub ao: f32,
    pub normal_ts: V3,
    pub backlight: V3,
    pub emissive: V3,
    pub alpha_clip: f32,
}

/// CPU reference of `ue_tint_surface` (ue_tint.wgsl), same order of operations
pub fn surface(g: &Gpu, s: &dyn Sampler, i: &SurfaceIn) -> SurfaceOut {
    let u = |n: &str| g.get(n);
    let f = |n: &str| g.get(n)[0];
    let b1 = |n: &str| g.get(n)[0] > 0.5;
    let v3 = |n: &str| xyz(g.get(n));
    let tx = |slot: usize, uv: [f32; 2]| s.sample(slot, uv);
    let uvs = |uv: [f32; 2], k: f32| [uv[0] * k, uv[1] * k];
    let uvm = |uv: [f32; 2], k: [f32; 4]| [uv[0] * k[0], uv[1] * k[1]];
    let pom = |slot: usize, uv0: [f32; 2], vts: [f32; 2], k: f32| {
        let mut uv = uv0;
        for _ in 0..4 {
            let h = tx(slot, uv)[1];
            uv = [uv[0] + vts[0] * (k * h - 0.5 * k), uv[1] + vts[1] * (k * h - 0.5 * k)];
        }
        uv
    };
    let paint_p = |m: f32| cc(if m > 0.0 { m.powf(f("paint_power")).min(1.0) } else { 0.0 }, f("paint_contrast1"));
    let paint_colors = |under: V3, p: f32| {
        let p2 = cc(p, f("paint_contrast2"));
        let c = mix3(under, v3("color_inner"), p);
        let c = mix3(c, v3("color_outer"), p2);
        mix3(c, v3("color_curvaturer"), cc(p2 - p, f("curvature_cc")))
    };
    let mode = f("ue_mode").round() as i32;
    let uv = i.uv;
    let col = i.color;
    let mut alb = tx(0, uv);
    let mut base = mul(xyz(alb), v3("base_color"));
    let mut n = xyz(tx(1, uv));
    let mut rough: f32;
    let (mut metal, mut ao, mut spec) = (0.0f32, 1.0f32, 0.5f32);
    let mut ss = sp(0.0);
    let mut emis = sp(0.0);
    let mut alpha_override = -1.0f32;
    match mode {
        1 => {
            let a = base;
            let m = xyz(tx(3, uv));
            let rma = tx(2, uv);
            let wa = 1.0 - (1.0 - (m[0] + m[1])) * (1.0 - (m[0] + m[1]));
            base = mix3(a, mul(a, v3("color_a")), wa);
            base = mix3(base, mul(a, v3("color_b")), m[1]);
            base = mix3(base, mul(a, v3("color_c")), m[2]);
            metal = rma[1];
            rough = if rma[0] > 0.0 { rma[0].powf(f("roughness_power")) } else { 0.0 };
            if b1("weapon_blood_on") {
                let b = tx(4, uv);
                let one_hot = |stage: f32| [1.0, 2.0, 3.0].map(|k| if stage == k { 1.0 } else { 0.0 });
                let mask = sat(mix(dot3(xyz(b), one_hot(f("blood_stage2"))), dot3(xyz(b), one_hot(f("blood_stage1"))), b[3]) * f("blood_mask_scalar"));
                let grain = tx(6, uvs(uv, 4.0))[0];
                base = mix3(base, mul(sc(a, f("blood_albedo_mix")), [0.25 * grain, 0.0, 0.0]), mask);
                metal = mix(metal, f("blood_metallic"), mask);
                rough = mix(rough, f("blood_roughness"), mask);
                let bn = tx(5, uvs(uv, 4.0));
                let nn = norm3(mix3(tsn([n[0], n[1]]), tsn([bn[0], bn[1]]), mask * f("blood_normal")));
                n = add(sc(nn, 0.5), sp(0.5));
            }
            base = mix3(base, sat3(sc(base, f("metal_tweak"))), rma[1]);
            ao = sat(rma[2] + f("ao_weaken"));
        }
        2 => {
            let rma = tx(2, uv);
            let c = mix3(base, v3("color_var"), tx(3, uv)[0]);
            let o = overlay(c, v3("color_overlay"));
            let mut c2 = mix3(c, o, 0.5);
            c2 = mix3(c2, sp(luma(c2)), f("color_desat"));
            let c3 = mix3(c2, sc(c2, rma[2]), f("ao_bias"));
            metal = cc(rma[1], f("metal_contrast"));
            let c4 = mix3(c, c3, 1.0 - metal);
            base = sat3(mix3(mix3(c4, v3("metal_color_lerp"), metal), c4, f("metal_color_bias")));
            let gm = u("mw_grunge");
            if gm[0] + gm[1] + gm[2] > 0.0 {
                let (scl, con) = (u("mw_scale"), u("mw_contrast"));
                let mut gb = base;
                for (k, col) in ["mw_color_r", "mw_color_g", "mw_color_b"].iter().enumerate() {
                    let g = cc(tx(4, uvs(uv, scl[k]))[k], con[k]) * gm[k];
                    gb = mix3(gb, mul(gb, v3(col)), g);
                }
                let lim = sat(1.0 + f("mw_limit_metal") * (metal - 1.0));
                base = sat3(mix3(base, gb, lim * f("mw_overall_bias")));
            }
            let r1 = rma[0] + (1.0 - metal) * (f("rough_lerp_wood") - rma[0]) * f("rough_lerp_wood_bias");
            rough = sat(mix(r1, mix(1.0, r1, 1.0 - metal), f("metal_roughness")));
            ao = sat(rma[2]);
        }
        3 => {
            let vts = [dot3(i.t, i.v), -dot3(i.b, i.v)];
            let lc = f("layer_count").round() as i32;
            let (til, bump, addv, con, hc, rm) = (u("tiling"), u("bump"), u("add"), u("contrast"), u("height_contrast"), u("rough_mul"));
            let u1 = pom(2, uvs(uv, til[0]), vts, bump[0] * 2.0);
            alb = tx(0, u1);
            let p1 = tx(2, u1);
            base = mul(xyz(alb), v3("mul_1"));
            n = xyz(tx(1, u1));
            ao = p1[2];
            rough = p1[0] * rm[0];
            let mut hb = p1[1];
            let ua = pom(6, uvs(uv, til[1]), vts, bump[1] * 2.0);
            let pa = tx(6, ua);
            let wa = if lc >= 2 { cc(sat(2.0 * col[0] + addv[1] - cc(hb, hc[0])), con[1]) } else { 0.0 };
            hb = mix(hb, cc(pa[1], hc[1]), wa);
            base = mix3(base, mul(xyz(tx(4, ua)), v3("mul_2")), wa);
            n = mix3(n, xyz(tx(5, ua)), wa);
            ao = mix(ao, pa[2], wa);
            rough = mix(rough, pa[0] * rm[1], wa);
            let ub = pom(9, uvs(uv, til[2]), vts, bump[2] * 2.0);
            let pb = tx(9, ub);
            let wb = if lc >= 3 { cc(sat(2.0 * col[1] + addv[2] - hb), con[2]) } else { 0.0 };
            hb = mix(hb, cc(pb[1], hc[2]), wb);
            base = mix3(base, mul(xyz(tx(7, ub)), v3("mul_3")), wb);
            n = mix3(n, xyz(tx(8, ub)), wb);
            ao = mix(ao, pb[2], wb);
            rough = mix(rough, pb[0] * rm[2], wb);
            let uc = pom(12, uvs(uv, til[3]), vts, bump[3] * 2.0);
            let pc = tx(12, uc);
            let w4 = if lc >= 4 { cc(sat(2.0 * col[2] + addv[3] - hb), con[3]) } else { 0.0 };
            base = mix3(base, mul(xyz(tx(10, uc)), v3("mul_4")), w4);
            n = mix3(n, xyz(tx(11, uc)), w4);
            ao = mix(ao, pc[2], w4);
            rough = mix(rough, pc[0] * rm[3], w4);
            base = mul(base, v3("overlay"));
            base = mix3(base, sp(luma(base)), f("desaturation"));
            base = mix3(base, sc(base, ao), f("ao_bias"));
            let gt = xyz(tx(3, uvs(uv, f("grunge_scale"))));
            if b1("use_grunge") {
                let gg = cc(if b1("grunge_srgb") { s2l(gt) } else { gt }[2], f("contrast_b"));
                base = sat3(mix3(base, mul(base, v3("grunge_color_b")), gg * f("overall_bias")));
            }
            metal = sat(mix(f("metallic_stain"), 0.0, col[3]));
            rough = sat(mix(f("roughness_stain"), rough, col[3]));
            spec = sat(mix(0.5, f("spec_04"), w4));
            ao = sat(ao);
        }
        4 => {
            let vts = [dot3(i.t, i.v), -dot3(i.b, i.v)];
            let ang = 1.0 - (1.0 - dot3(i.n, i.v).max(0.0)).abs().max(0.0001).powf(f("bump_offset_power"));
            let (til, bump) = (u("tiling"), u("bump"));
            let u1 = pom(2, uvs(uv, til[0]), vts, bump[0] * 2.0 * ang);
            let u2 = pom(6, uvs(uv, til[1]), vts, bump[1] * 2.0 * ang);
            alb = tx(0, u1);
            let p1 = tx(2, u1);
            let p2 = tx(6, u2);
            let w2 = if f("layer_count").round() as i32 >= 2 {
                cc(sat(2.0 * col[0] + u("add")[1] - cc(p1[1], u("height_contrast")[0])), u("contrast")[1])
            } else {
                0.0
            };
            let mut l2 = mul(xyz(tx(4, u2)), v3("mul_2"));
            let mut pm = 0.0;
            let mut n2 = xyz(tx(5, u2));
            if b1("use_paint") {
                let p = paint_p(p2[0] * p2[2] * mix(1.0, p2[1], f("dhi")) * col[1]);
                pm = p * f("mask_output_level");
                l2 = paint_colors(l2, p);
                n2 = mix3(n2, [0.5, 0.5, 1.0], sat(pm * f("paint_flatten_normal")));
            }
            base = mix3(mul(xyz(alb), v3("mul_1")), l2, w2);
            let bo = mul(base, v3("overlay"));
            base = sat3(mix3(bo, sp(luma(bo)), f("desaturation")));
            rough = sat(mix(p1[0] * u("rough_mul")[0], mix(p2[0], f("paint_roughness"), pm), w2));
            n = mix3(xyz(tx(1, u1)), n2, w2);
        }
        5 => {
            let rh = tx(2, uv);
            let d = tx(4, uvm(uv, u("rhao_detail_scale")));
            let a = mul(mix3(xyz(alb), sp(luma(xyz(alb))), f("a_desaturation")), v3("a_color"));
            let dp = f("detail_power");
            let mut dd = xyz(tx(5, uvm(uv, u("albedo_detail_scale")))).map(|x| x.powf(dp));
            dd = mul(mix3(dd, sp(luma(dd)), f("desaturation_detail")), v3("albedo_color"));
            let ov = overlay(a, dd);
            let c0 = mix3(a, ov, f("overlay_bias"));
            let mut c1 = mix3(c0, v3("curvature_color"), rh[1]);
            let mut pm = 0.0;
            if b1("use_paint") {
                let p = paint_p(d[1] * d[1] * mix(1.0, rh[0], f("dhi")) * col[2]);
                pm = p * f("mask_output_level");
                c1 = paint_colors(c1, p);
            }
            let mut c = mix3(c1, c0, rh[1]);
            c = mix3(c, v3("dirt_color"), sat((rh[0] - pm) * cc(1.0 - rh[2].powf(f("dirt_power")), f("dirt_contrast"))));
            base = sat3(mix3(c, mul(mix3(c, sp(luma(c)), f("damage_desaturation")), v3("color_damage")), col[1]));
            rough = rh[0] + f("roughness_bias") * d[0];
            ao = d[2] * mix(mix(1.0, rh[2], f("ao_bias")), 1.0, f("bias_normal"));
            rough = mix(rough, rough * f("paint_roughness"), pm);
            ao = mix(ao, 1.0, pm);
            ao = sat(mix(ao, d[2], col[1]));
            rough = sat(rough);
            let ndt = tx(6, uvm(uv, u("normal_detail_scale")));
            if b1("use_normal_detail") {
                let nn = tsn([n[0], n[1]]);
                let dn = mix3(tsn([ndt[0], ndt[1]]), [0.0, 0.0, 1.0], f("bias_normal"));
                let mut r = rnm(nn, dn);
                let sq = rh[1] - rh[2] + 1.0;
                r = mix3(r, nn, if sq > 0.0 { sq.sqrt() } else { 0.0 });
                r = mix3(r, nn, pm);
                r = mix3(r, sub(sc(dn, 2.0), [0.0, 0.0, 1.0]), col[1]);
                n = add(sc(norm3(r), 0.5), sp(0.5));
            }
        }
        6 => {
            base = sat3(xyz(alb));
            rough = if base[0] > 0.0 { (1.0 - base[0].powf(2.5)).max(0.0) } else { 1.0 };
        }
        7 => {
            let sel = if b1("has_secondary") { (col[0] - 0.1).ceil() } else { 0.0 };
            let s2 = sel > 0.5;
            let (m, rma) = if s2 { (xyz(tx(7, uv)), tx(6, uv)) } else { (xyz(tx(3, uv)), tx(2, uv)) };
            if s2 {
                alb = tx(4, uv);
                n = xyz(tx(5, uv));
            }
            let pre = if s2 { "secondary_" } else { "" };
            let (ca, cb, ccol) = (v3(&format!("{pre}color_a")), v3(&format!("{pre}color_b")), v3(&format!("{pre}color_c")));
            let a = mul(xyz(alb), v3("base_color"));
            let bl = f("bleed");
            let w: V3 = [0, 1, 2].map(|k| 1.0 - (1.0 - m[k]) * (1.0 - bl * m[k]));
            let mut c = mix3(a, mul(a, ca), w[0]);
            c = mix3(c, mul(a, cb), w[2]);
            c = mix3(c, mul(a, ccol), w[1]);
            let emb = tx(8, i.uv2);
            let e = [sat(1.0 - (1.0 - bl * emb[0]) * (1.0 - emb[0])), sat(1.0 - (1.0 - bl * emb[2]) * (1.0 - emb[2]))];
            let asat = sat3(a);
            let mut ea = mix3(a, mul(asat, v3("emblem_color_a")), e[0]);
            ea = mix3(ea, mul(asat, v3("emblem_color_b")), e[1]);
            let k = (e[0] + e[1]).min(1.0).powf(10.0);
            base = mix3(c, ea, mix(f("primary_has_emblem"), f("secondary_has_emblem"), sel) * k);
            metal = rma[1];
            rough = mix(rma[0], mix(rma[0], rma[0] * f("metal_roughness_mul") + f("metal_roughness_add"), f("metal_roughness_scale")), rma[1]);
            ao = rma[2];
        }
        8 => {
            let a = sc(xyz(alb), f("albedo_scale"));
            let cm = xyz(tx(3, uv));
            let bl = f("bleed");
            let w: V3 = [0, 1, 2].map(|k| 1.0 - (1.0 - cm[k]) * (1.0 - bl * cm[k]));
            let mut c = mix3(a, mul(a, v3("color_a")), w[0]);
            c = mix3(c, mul(a, v3("color_b")), w[2]);
            c = sc(c, 1.0 - w[1]);
            let emb = tx(8, i.uv2);
            let e = [sat(1.0 - (1.0 - bl * emb[0]) * (1.0 - emb[0])), sat(1.0 - (1.0 - bl * emb[2]) * (1.0 - emb[2]))];
            let asat = sat3(a);
            let mut ea = mix3(a, mul(asat, v3("emblem_color_a")), e[0]);
            ea = mix3(ea, mul(asat, v3("emblem_color_b")), e[1]);
            base = mix3(c, ea, (e[0] + e[1]).min(1.0).powf(10.0));
            let dv = sat(dot3(i.v, i.n));
            base = sc(base, 1.0 - 0.6 * dv + 0.8 * (1.0 - dv).powf(6.0));
            let rh = tx(2, uv);
            rough = sat(rh[0] * f("roughness_scale"));
            ao = sat(rh[2]);
            ss = sat3(mul(base, v3("sss")));
        }
        9 => {
            let ct = u("cloth_tile");
            let u0 = uvs(uv, ct[0]);
            let ud = uvs(uv, ct[1]);
            let mut a = xyz(tx(0, u0));
            a = mul(mix3(a, sp(luma(a)), f("cloth_desat")), v3("cloth_mul"));
            let da = tx(4, ud);
            let mut c = mul(a, xyz(da));
            let gt = xyz(tx(6, uvs(if b1("cloth_grunge_uv1") { i.uv2 } else { uv }, f("cloth_grunge_scale"))));
            let gg = cc(if b1("cloth_grunge_srgb") { s2l(gt) } else { gt }[0], f("cloth_grunge_contrast"));
            c = mix3(c, mul(c, v3("cloth_grunge_color")), gg * f("cloth_grunge_bias"));
            let rh = tx(2, u0);
            base = sat3(c);
            rough = sat(rh[0]);
            ao = sat(da[3] * rh[2]);
            let n0 = tx(1, u0);
            let nd = tx(5, ud);
            let nn = tsn([n0[0], n0[1]]);
            let dn = mix3(tsn([nd[0], nd[1]]), [0.0, 0.0, 1.0], f("cloth_nm_bias"));
            n = add(sc(norm3(rnm(nn, dn)), 0.5), sp(0.5));
        }
        10 => {
            let a = xyz(tx(4, uvm(uv, u("banner_detail_scale"))));
            let mut bm = xyz(tx(7, uv));
            if b1("banner_mask_srgb") {
                bm = s2l(bm);
            }
            let bl = f("bleed");
            let w: V3 = [0, 1, 2].map(|k| 1.0 - (1.0 - bm[k]) * (1.0 - bl * bm[k]));
            let mut c = mix3(a, mul(a, v3("color_a")), w[0]);
            c = mix3(c, mul(a, v3("color_b")), w[2]);
            c = sc(c, 1.0 - w[1]);
            let mut gr = xyz(tx(6, uvs(uv, f("banner_scale_r"))));
            let mut gb = xyz(tx(6, uvs(uv, f("banner_scale_b"))));
            if b1("cloth_grunge_srgb") {
                gr = s2l(gr);
                gb = s2l(gb);
            }
            let mut c1 = mix3(c, mul(c, v3("banner_color_r")), cc(gr[0], f("banner_contrast_r")));
            c1 = mix3(c1, mul(c1, v3("banner_color_b")), cc(gb[2], f("banner_contrast_b")));
            base = sat3(mix3(c, c1, f("banner_bias")));
            ss = sat3(sc(c, f("banner_sss")));
            let rm = tx(2, uv);
            rough = sat(rm[0]);
            ao = sat(rm[2]);
        }
        11 => {
            base = sat3(mul(xyz(alb), [0.212429, 0.244792, 0.060914]));
            emis = mul(xyz(alb), [0.031864, 0.036719, 0.009137]);
            rough = 0.7;
            spec = 0.01;
        }
        12 => {
            base = sat3(xyz(alb));
            rough = sat(alb[0] * 0.6 + 0.4);
            ss = base;
        }
        13 => {
            let fu = uvs(uv, f("flora_tiling"));
            let a = xyz(tx(0, fu));
            let mut c = mix3(a, v3("flora_color"), f("flora_color_lerp"));
            c = mix3(c, sp(luma(c)), f("flora_desat"));
            c = hue_rot(c, f("flora_hsl") * 6.28319);
            c = mul(c, v3("flora_mul"));
            emis = sc(c, f("flora_emissive"));
            ss = sat3(sc(mul(c, v3("flora_sub")), f("flora_ss_bias")));
            base = sat3(c);
            metal = sat(f("flora_metallic"));
            spec = sat(f("flora_spec"));
            rough = sat(f("flora_rough"));
            n = mix3(xyz(tx(1, fu)), [0.5, 0.5, 1.0], f("flora_nm_flat"));
        }
        14 => {
            base = sp(0.0);
            rough = 0.5;
            alpha_override = sat(tx(3, uv)[1]);
        }
        15 => {
            base = sat3(if b1("use_diffuse_tex") { xyz(alb) } else { v3("diffuse_const") });
            metal = sat(f("metallic_const"));
            spec = sat(f("specular_const"));
            rough = sat(f("roughness_const"));
            ao = sat(f("ao_const"));
        }
        16 => {
            let a = xyz(alb);
            emis = sc(a, f("mf_emissive"));
            let mut c = mul(a, v3("albedo_tint"));
            c = mix3(c, sp(luma(c)), f("mf_desaturation"));
            c = hue_rot(c, f("hue_shift") * 6.28319);
            let ray = sc(i.v, -1.0);
            let tz = 1.0 + ray[1];
            let top = (if tz > 0.0 { tz.powf(f("top_power")) } else { 0.0 }).max(f("top_darken")).min(1.0);
            base = sat3(sc(c, top));
            rough = sat(tx(2, uv)[0] * f("mf_roughness"));
            let tl = xyz(tx(4, uv));
            ss = sat3(mul(if b1("translucency_srgb") { s2l(tl) } else { tl }, v3("mf_sss")));
            spec = f("mf_specular");
        }
        17 => {
            let a = xyz(alb);
            let mut c = mul(mix3(a, sp(luma(a)), f("saturation")), v3("bush_color"));
            let b = sc(c, f("brightness"));
            let wue = [i.world_pos[0] * 100.0, i.world_pos[2] * 100.0];
            let vt_ = f("variation_tiling");
            let vt = xyz(tx(4, [wue[0] / vt_, wue[1] / vt_]));
            let vi = f("variation_intensity");
            let v = (if b1("variation_srgb") { s2l(vt) } else { vt }).map(|x| (1.0 - x).max(vi).min(1.0));
            c = overlay(b, v);
            let t = 1.0 - uv[1];
            c = sc(c, if t > 0.0 { t.powf(f("ao_power")) } else { 0.0 });
            emis = sc(c, f("emissive")).map(|x| x.max(0.0));
            base = sat3(c);
            ss = base;
            let tr = 1.0 - a[0];
            rough = if tr > 0.0 { tr.powf(f("roughness_power")).min(1.0) } else { 0.0 };
            spec = sat(f("specular_value"));
        }
        18 => {
            let u0 = uvs(uv, u("sw_scale")[0]);
            alb = tx(0, u0);
            let a = xyz(alb);
            let c0 = mix3(a, sp(luma(a)), f("sw_desat"));
            let cm = mul(c0, v3("sw_color_mul"));
            let mut c = mix3(cm, v3("sw_color_lerp"), f("sw_color_lerp_bias"));
            let g = cc(tx(4, uvs(uv, u("mw_scale")[0]))[0], u("mw_contrast")[0]) * u("mw_grunge")[0];
            c = add(c, sc(sub(mul(c, v3("mw_color_r")), c), f("mw_overall_bias") * g));
            c = sat3(add(c, sc(sub(v3("sw_vc_color"), c), f("sw_vc_bias") * col[3])));
            base = c;
            metal = if b1("sw_has_metal") { sat(alb[3]) } else { 0.0 };
            let ra = tx(2, u0);
            let r = ra[0] - f("sw_rough_mod") * ra[0];
            rough = sat(r + f("sw_vertex_rough_bias") * (f("sw_vertex_rough") - r) * col[3]);
            ao = sat(ra[2] + f("sw_ao_bias") * (1.0 - ra[2]));
            let nt = tx(1, u0);
            let nb = mix3(tsn([nt[0], nt[1]]), [0.0, 0.0, 1.0], u("sw_nm_flat")[0]);
            let dt = tx(5, uvs(uv, u("sw_scale")[1]));
            let nr = if b1("sw_detail") { rnm(nb, mix3(tsn([dt[0], dt[1]]), [0.0, 0.0, 1.0], u("sw_nm_flat")[1])) } else { nb };
            n = add(sc(norm3(nr), 0.5), sp(0.5));
        }
        19 => {
            let a = xyz(alb);
            let mut c = mix3(a, v3("sb_color"), f("sb_color_lerp"));
            let d = xyz(tx(5, uvs(uv, 6.0)));
            let o = [0, 1, 2].map(|k| if c[k] >= 0.5 { 1.0 - 2.0 * (1.0 - c[k]) * (1.0 - d[k]) } else { 2.0 * c[k] * d[k] });
            c = o;
            let ra = tx(2, uv);
            c = mix3(c, sc(c, ra[2]), f("sb_ao_in_bias"));
            let g = cc(tx(4, uvs(uv, u("mw_scale")[0]))[0], u("mw_contrast")[0]) * u("mw_grunge")[0];
            base = sat3(add(c, sc(sub(mul(c, v3("mw_color_r")), c), f("mw_overall_bias") * g)));
            rough = sat(ra[0] * f("sb_roughness"));
            ao = sat(ra[2] + f("sb_ao_bias") * (1.0 - ra[2]));
            let nt = tsn([n[0], n[1]]);
            n = add(sc(norm3(mix3(nt, [0.0, 0.0, 1.0], 1.0 - f("sb_normal_strength"))), 0.5), sp(0.5));
        }
        _ => {
            let packed = tx(2, uv);
            let m = xyz(tx(3, uv));
            base = mix3(base, mul(base, v3("color_a")), m[0]);
            base = mix3(base, mul(base, v3("color_b")), m[1]);
            base = mix3(base, mul(base, v3("color_c")), m[2]);
            let d4 = |a: [f32; 4], b: [f32; 4]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
            rough = sat(d4(packed, u("rough_sel")) + f("rough_add"));
            metal = sat(d4(packed, u("metal_sel")) + f("metal_add"));
            ao = sat(d4(packed, u("ao_sel")) + f("ao_add"));
        }
    }
    let mut o = SurfaceOut { base, roughness: rough, metallic: metal, specular: spec, ao, emissive: emis, ..Default::default() };
    o.normal_ts = [0.0, 0.0, 1.0];
    if b1("use_normal") {
        let nm = if b1("normal_flip_y") { [n[0], 1.0 - n[1]] } else { [n[0], n[1]] };
        o.normal_ts = tsn(nm);
    }
    o.backlight = if b1("use_foliage") { ss } else { sp(0.0) };
    o.alpha = if b1("alpha_from_albedo") { alb[3] } else { f("alpha_value") };
    if alpha_override >= 0.0 {
        o.alpha = alpha_override;
    }
    o.alpha_clip = f("alpha_clip");
    o
}

/// CPU reference of `ue_lightmap` (ue_tint.wgsl; SHADERS.md 3, 4): (emissive to add, AO with sky occlusion)
pub fn lightmap(g: &Gpu, smp: &dyn Sampler, i: &SurfaceIn, s: &SurfaceOut, nw: V3) -> (V3, f32) {
    let u = |n: &str| g.get(n);
    let lm_uv = u("lm_uv")[0];
    let src = if lm_uv > 1.5 { i.custom0 } else if lm_uv > 0.5 { i.uv2 } else { i.uv };
    let c = u("lm_coord");
    let auv = [src[0] * c[0] + c[2], src[1] * c[1] + c[3]];
    let luv = [auv[0], auv[1] * 0.5];
    let l0 = smp.sample(13, luv);
    let l1 = smp.sample(13, [luv[0], luv[1] + 0.5]);
    let (s0, a0, s1, a1) = (u("lm_scale0"), u("lm_add0"), u("lm_scale1"), u("lm_add1"));
    let log_l = (l0[3] + l1[3] * (1.0 / 255.0) - (0.5 / 255.0)) * s0[3] + a0[3];
    let uvw = [0, 1, 2].map(|k| l0[k] * l0[k] * s0[k] + a0[k]);
    let lum = log_l.exp2() - 0.01858136;
    let sh: [f32; 4] = [0, 1, 2, 3].map(|k| l1[k] * s1[k] + a1[k]);
    let nue = [nw[0], nw[2], nw[1]];
    let d4 = |n: V3| sh[0] * n[1] + sh[1] * n[2] + sh[2] * n[0] + sh[3];
    let dir = d4(nue).max(0.0);
    let base = s.base;
    let diffuse = sc(base, 1.0 - s.metallic);
    let a = s.ao;
    let ao_mb: V3 = [0, 1, 2].map(|k| {
        let b = base[k];
        a.max(((a * (2.0404 * b - 0.3324) + (-4.7951 * b + 0.6417)) * a + (2.7552 * b + 0.6903)) * a)
    });
    let mut lit = sc(mul(uvw, diffuse), lum * dir);
    if g.get("use_foliage")[0] > 0.5 {
        lit = add(lit, sc(mul(uvw, s.backlight), lum * d4(sc(nue, -1.0)).max(0.0)));
    }
    let mut ao = a;
    let sk = smp.sample(14, auv);
    if g.get("use_sky_occlusion")[0] > 0.5 {
        let vis = sk[3] * sk[3];
        let bent = norm3([sk[0] * 2.0 - 1.0, sk[1] * 2.0 - 1.0, sk[2] * 2.0 - 1.0]);
        let wv = 1.0 - (1.0 - vis) * (1.0 - vis);
        ao = a * vis * mix(sat(dot3(bent, nue)), 1.0, wv);
    }
    (mul(lit, ao_mb), ao)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::MaterialDesc;
    use std::collections::BTreeMap;

    /// constant colour per slot
    struct Const([[f32; 4]; NUM_TEX_SLOTS]);
    impl Sampler for Const {
        fn sample(&self, slot: usize, _uv: [f32; 2]) -> [f32; 4] {
            self.0[slot]
        }
    }

    fn desc(mode: &str, u: &[(&str, Uniform)]) -> MaterialDesc {
        let mut m: BTreeMap<String, Uniform> = u.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        m.insert("ue_mode".into(), Uniform::I(crate::material::mode_id(mode)));
        MaterialDesc {
            package: String::new(),
            master: String::new(),
            master_package: String::new(),
            chain: vec![],
            mode: mode.into(),
            blend: 0,
            two_sided: false,
            foliage: false,
            unlit: false,
            clip: 0.3333,
            layout: String::new(),
            flat: false,
            missing_albedo: None,
            uniforms: m,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn wgsl_struct_matches_uniform_table() {
        let s = ue_tint_wgsl();
        let a = s.find("struct UeTint {").unwrap();
        let body = &s[a..a + s[a..].find('}').unwrap()];
        let fields: Vec<&str> = body.lines().skip(1).filter_map(|l| l.trim().strip_suffix(": vec4<f32>,")).collect();
        let names: Vec<&str> = UNIFORMS.iter().map(|u| u.0).collect();
        assert_eq!(fields, names);
    }

    #[test]
    fn slots_never_collide_within_a_mode() {
        for mode in 0..18 {
            let used = mode_textures(mode);
            let mut seen = std::collections::HashMap::new();
            for n in used {
                let t = TEX_SLOTS.iter().find(|t| t.0 == *n).unwrap_or_else(|| panic!("{n} has no slot"));
                if let Some(o) = seen.insert(t.1, *n) {
                    panic!("mode {mode}: {o} and {n} share slot {}", t.1);
                }
            }
        }
    }

    #[test]
    fn generic_rhao_layout_and_defaults() {
        let (rs, ms, as_, ra, ma, aa) = crate::material::layout("RHAO");
        let m = desc("", &[("rough_sel", Uniform::V4(rs)), ("metal_sel", Uniform::V4(ms)), ("ao_sel", Uniform::V4(as_)),
            ("rough_add", Uniform::F(ra)), ("metal_add", Uniform::F(ma)), ("ao_add", Uniform::F(aa)),
            ("base_color", Uniform::V3([0.5, 1.0, 1.0]))]);
        let g = gpu(&m);
        let mut t = [[0.0; 4]; NUM_TEX_SLOTS];
        t[0] = [0.8, 0.6, 0.4, 1.0];
        t[1] = [0.5, 0.5, 1.0, 1.0];
        t[2] = [0.7, 0.3, 0.9, 1.0];
        let o = surface(&g, &Const(t), &SurfaceIn::default());
        assert!(close(o.base[0], 0.4) && close(o.base[1], 0.6));
        assert!(close(o.roughness, 0.7) && close(o.metallic, 0.0) && close(o.ao, 0.9));
        // flat normal map -> (0, 0, 1) after the DirectX Y flip
        assert!(close(o.normal_ts[2], 1.0));
        assert_eq!(o.specular, 0.5);
    }

    #[test]
    fn weapon_blood_dxbc_stage_routing_and_original_metal_albedo_weight() {
        // PS086: V15/16 = 1-abs(sign(stage-(1,2,3))); mask alpha routes
        // channel2->channel1. MetalTweak uses the unmodified RMA.g.
        let mut g = gpu(&desc("M_WEAPON", &[("weapon_blood_on", Uniform::B(true)),
            ("blood_stage1", Uniform::F(1.0)), ("blood_stage2", Uniform::F(3.0))]));
        let mut t = [[0.0;4];NUM_TEX_SLOTS];
        t[0]=[0.8,0.6,0.4,1.0]; t[1]=[0.5,0.5,1.0,1.0];
        t[2]=[0.8,1.0,0.7,1.0]; t[4]=[1.0,0.0,0.0,1.0];
        t[5]=[0.5,0.5,1.0,1.0]; t[6]=[1.0;4];
        let out = surface(&g,&Const(t),&SurfaceIn::default());
        assert!(close(out.base[0],0.46)); // .8 * 2 * .25 * MetalTweak1.15
        assert_eq!(&out.base[1..], &[0.0,0.0]);
        assert!(close(out.metallic,0.0) && close(out.roughness,0.15));
        assert!(close(out.ao,0.7));
        // Stage2's blue is0, so alpha0 selects clean; half-stage has no one-hot.
        t[4][3]=0.0;
        let clean=surface(&g,&Const(t),&SurfaceIn::default());
        assert!(close(clean.metallic,1.0) && close(clean.roughness,0.8));
        t[4][3]=1.0;g.set("blood_stage1",[1.5,0.0,0.0,0.0]);
        assert_eq!(surface(&g,&Const(t),&SurfaceIn::default()).base,clean.base);
        g.set("blood_stage1",[1.0,0.0,0.0,0.0]); g.set("weapon_blood_on",[0.0;4]);
        assert_eq!(surface(&g,&Const(t),&SurfaceIn::default()).base,clean.base);
    }

    #[test]
    fn weapon_blood_dxbc_uv0_times_four_not_mesh_uv1() {
        struct Probe(std::cell::RefCell<Vec<(usize,[f32;2])>>);
        impl Sampler for Probe {
            fn sample(&self, slot:usize, uv:[f32;2])->[f32;4] {
                self.0.borrow_mut().push((slot,uv)); [0.5,0.5,1.0,1.0]
            }
        }
        let g=gpu(&desc("M_WEAPON",&[("weapon_blood_on",Uniform::B(true))]));
        let p=Probe(Default::default());
        surface(&g,&p,&SurfaceIn{uv:[0.25,0.75],uv2:[0.1,0.2],..Default::default()});
        let samples=p.0.borrow();
        assert!(samples.contains(&(4,[0.25,0.75])));
        assert!(samples.contains(&(5,[1.0,3.0])) && samples.contains(&(6,[1.0,3.0])));
        assert!(!samples.contains(&(5,[0.1,0.2])));
    }

    #[test]
    fn weapon_tints_by_color_map() {
        // SHADERS.md 5: mask r = 1 -> wa = 1 -> A * ColorA; RoughnessPower 2
        let m = desc("M_WEAPON", &[("color_a", Uniform::V3([1.0, 0.0, 0.0])), ("roughness_power", Uniform::F(2.0))]);
        let g = gpu(&m);
        let mut t = [[0.0; 4]; NUM_TEX_SLOTS];
        t[0] = [0.5, 0.5, 0.5, 1.0];
        t[2] = [0.5, 0.0, 1.0, 1.0];
        t[3] = [1.0, 0.0, 0.0, 1.0];
        let o = surface(&g, &Const(t), &SurfaceIn::default());
        assert!(close(o.base[0], 0.5) && close(o.base[1], 0.0));
        assert!(close(o.roughness, 0.25) && close(o.metallic, 0.0) && close(o.ao, 1.0));
    }

    #[test]
    fn cheap_contrast_and_srgb() {
        assert_eq!(cc(0.5, 0.0), 0.5);
        assert_eq!(cc(0.75, 1.0), 1.0);
        assert!(close(s2l([0.5; 3])[0], 0.214_041_14));
        assert!(close(luma([1.0, 0.0, 0.0]), 0.3));
    }

    #[test]
    fn layered_vertex_red_blends_layer_two() {
        // SHADERS.md 6a: COLOR.r = 1 with Add 0, contrast 0, flat height 0 -> w = cc(clamp(2 - 0), 0) = 1
        let m = desc("M_LAYERED", &[("layer_count", Uniform::I(2)), ("mul_2", Uniform::V3([1.0; 3]))]);
        let g = gpu(&m);
        let mut t = [[1.0; 4]; NUM_TEX_SLOTS];
        t[0] = [0.2, 0.2, 0.2, 1.0];
        t[2] = [0.5, 0.0, 1.0, 1.0];
        t[4] = [0.9, 0.1, 0.1, 1.0];
        t[6] = [0.3, 0.0, 0.5, 1.0];
        let i = SurfaceIn { color: [1.0, 0.0, 0.0, 1.0], ..Default::default() };
        let o = surface(&g, &Const(t), &i);
        // ao_bias 0 -> base = layer-2 albedo
        assert!(close(o.base[0], 0.9) && close(o.base[1], 0.1));
        assert!(close(o.roughness, 0.3) && close(o.ao, 0.5));
    }

    #[test]
    fn lightmap_decode_constant_texels() {
        // both halves read (1, 1, 1, 0.5): log L = 0.5 + 0.5/255 - 0.5/255; sh = l1 + lm_add1 = (1, 1, 1, 1.5);
        // N = +Y (UE +Z) -> nue.yzx = (0, 1, 0), dir = 1 + 1.5
        let m = desc("", &[]);
        let mut g = gpu(&m);
        g.set("lm_add1", [0.0, 0.0, 0.0, 1.0]);
        let mut t = [[0.0; 4]; NUM_TEX_SLOTS];
        t[13] = [1.0, 1.0, 1.0, 0.5];
        let s = SurfaceOut { base: [1.0; 3], ao: 1.0, ..Default::default() };
        let (e, ao) = lightmap(&g, &Const(t), &SurfaceIn::default(), &s, [0.0, 1.0, 0.0]);
        let lum = (0.5f32 + 0.5 / 255.0 - 0.5 / 255.0).exp2() - 0.01858136;
        // ao_mb at a = 1, base 1: max(1, (2.0404 - 0.3324 - 4.7951 + 0.6417 + 2.7552 + 0.6903)) = 1.0001
        let mb = 1.0f32.max(2.0404 - 0.3324 - 4.7951 + 0.6417 + 2.7552 + 0.6903);
        assert!(close(e[0], lum * 2.5 * mb), "{e:?} vs {}", lum * 2.5 * mb);
        assert_eq!(ao, 1.0);
    }

    #[test]
    fn wearable_slots_and_secondary_select() {
        use crate::material::{wearable, WearableSet};
        let t = |n: &str| Some(TexRef(n.into()));
        let p = WearableSet { albedo: t("A"), mask: t("M"), ..Default::default() };
        let s2 = WearableSet { albedo: t("A2"), colors: Some([[0.0, 1.0, 0.0], [0.0; 3], [1.0; 3]]), ..Default::default() };
        let m = wearable(&p, Some(&s2), None, true);
        let g = gpu(&m);
        assert_eq!(g.slots[0], SlotBinding::Texture { tex: TexRef("A".into()), srgb: true, uniform: "albedo_texture" });
        assert_eq!(g.slots[4], SlotBinding::Texture { tex: TexRef("A2".into()), srgb: true, uniform: "secondary_albedo" });
        assert_eq!(g.slots[1], SlotBinding::Default(TexDefault::Normal));
        assert_eq!(m.blend, crate::material::BLEND_MASKED);
        // VC.r = 1 -> secondary set (059 14-16): mask r = 1 -> A2 * secondary ColorA (green)
        let mut tx = [[0.0; 4]; NUM_TEX_SLOTS];
        tx[4] = [0.5, 0.5, 0.5, 1.0];
        tx[7] = [1.0, 0.0, 0.0, 1.0];
        let o = surface(&g, &Const(tx), &SurfaceIn { color: [1.0, 0.0, 0.0, 1.0], ..Default::default() });
        assert!(close(o.base[0], 0.0) && close(o.base[1], 0.5), "{:?}", o.base);
    }
}

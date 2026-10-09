//! uesky.rs - the map's BP_Sky_Sphere sky and the stationary SkyLight's diffuse light (rust-render r1).
//!
//! Sky radiance: M_Sky_Panning_Clouds2 as ported in godot/game/render/ue_sky.gdshader (the reference implementation,
//! line-cited to the game's cooked SM5 pixel shader state/shaders/M_Sky_Panning_Clouds2/r0/000_TBasePassPS...asm), with
//! the level's MaterialInstanceDynamic values (BP_Sky_Sphere "Sky material", Arena.308 on DU_Arena). It is evaluated on
//! the CPU into a cube map: the camera's Skybox (UE draws SM_SkySphere, a mesh around the map; at its radius the
//! direction alone decides the colour) and the SkyLight capture.
//!
//! SkyLight: USkyLightComponent ctor (rva 0x2fea8e0): SourceType +0x228 = 0 (SLS_CapturedScene), CubemapResolution
//! +0x23c = 128, SkyDistanceThreshold +0x240 = 150000, bLowerHemisphereIsBlack +0x245 = 1, LowerHemisphereColor +0x248 =
//! 0 (.rdata 0x4496d80). The map's SkyLightComponent0 serializes only Intensity 3, LightColor (188, 227, 255),
//! IndirectLightingIntensity 2.5 and its location, so it is Stationary (ULightComponentBase default) and captures the
//! scene beyond 150000 cm = the sky sphere only. A stationary sky light's lighting is not in the lightmaps (Lightmass
//! bakes only its occlusion: the lightmaps' SkyOcclusion texture, ue_tint.wgsl ue_lightmap sky_occlusion); the renderer
//! adds SH irradiance of the captured cube x LightColor x Intensity at runtime (FSkyLightSceneProxy ctor .text
//! 0x2fe9f70 stores linear LightColor x Intensity, ue_light.gd skylight_tint). Here that irradiance is a small diffuse
//! cube on Bevy's EnvironmentMapLight, scaled per pixel by ue_tint's diffuse_occlusion (sky visibility x AO).
//!
//! Cube convention: Bevy samples cubes with the direction's z negated (bevy_pbr environment_map.wgsl "Cube maps are
//! left-handed", bevy_core_pipeline skybox.wgsl); a texel at wgpu face direction d holds Bevy direction (d.x, d.y, -d.z)
//! = UE direction (d.x, -d.z, d.y) (UE X fwd, Y right, Z up = Bevy x, -z... via SwapYZ, ue.rs).

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use std::collections::HashMap;

/// Material parameter defaults where the MID sets none (ue_sky.gdshader uniform defaults = the cooked shader map's
/// parameter list, state/shaders/M_Sky_Panning_Clouds2/map.json)
fn default_param(k: &str) -> [f32; 4] {
    match k {
        "zenith color" => [0.085177, 0.153746, 0.35, 1.0],
        "horizon color" => [0.940601, 1.0, 1.0, 1.0],
        "cloud color" => [0.71685, 0.782221, 0.885, 1.0],
        "overall color" => [1.0, 1.0, 1.0, 1.0],
        "sun color" => [1.0, 0.8, 0.4, 1.0],
        "light direction" => [-2.0, 0.0, -1.0, 1.0],
        "selectioncolor" => [0.0, 0.0, 0.0, 0.0],
        "horizon falloff" => [3.0, 0.0, 0.0, 0.0],
        "sun brightness" => [50.0, 0.0, 0.0, 0.0],
        "sun radius" => [0.0003, 0.0, 0.0, 0.0],
        "sun height" => [1.0, 0.0, 0.0, 0.0],
        "stars brightness" => [0.1, 0.0, 0.0, 0.0],
        "cloud speed" => [0.1, 0.0, 0.0, 0.0],
        "cloud opacity" => [1.0, 0.0, 0.0, 0.0],
        "noisepower1" => [1.0, 0.0, 0.0, 0.0],
        "noisepower2" => [4.0, 0.0, 0.0, 0.0],
        _ => [0.0; 4],
    }
}

/// One channel of an EngineSky texture (all read as .r by the material), decoded from the paks. The textures do not
/// serialize SRGB, so UTexture's default SRGB = true holds and the GPU returns linear values: decoded here with the
/// sRGB curve. (The Godot reference samples them raw: a discrepancy with the data, reported.)
pub struct Tex {
    w: usize,
    h: usize,
    r: Vec<f32>,
}

impl Tex {
    fn bilinear(&self, u: f32, v: f32) -> f32 {
        // repeat wrap (the material's samplers wrap)
        let x = u * self.w as f32 - 0.5;
        let y = v * self.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |xi: i64, yi: i64| -> f32 {
            let xi = xi.rem_euclid(self.w as i64) as usize;
            let yi = yi.rem_euclid(self.h as i64) as usize;
            self.r[yi * self.w + xi]
        };
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * fx;
        let b = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * fx;
        a + (b - a) * fy
    }
}

pub fn load_tex(src: &mh_assets::pak_source::PakSource, pkg: &str) -> Option<Tex> {
    let t = crate::paksrc::tex_info(src, pkg).ok()?;
    let m = t.mips.first()?;
    let (w, h) = (m.size_x.max(1) as usize, m.size_y.max(1) as usize);
    let px = crate::paksrc::decoded_rgba8(src, &t).ok()?;
    let srgb = t.srgb;
    let r = px.chunks_exact(4).map(|p| {
        let x = p[0] as f32 / 255.0;
        if srgb { crate::uepost::srgb_to_lin(x) } else { x }
    }).collect();
    Some(Tex { w, h, r })
}

pub struct Sky {
    pub params: HashMap<String, [f32; 4]>,
    pub clouds: Option<Tex>,
    pub blue: Option<Tex>,
    pub stars: Option<Tex>,
}

impl Sky {
    fn p(&self, k: &str) -> [f32; 4] {
        self.params.get(k).copied().unwrap_or_else(|| default_param(k))
    }
    fn s(&self, k: &str) -> f32 {
        self.p(k)[0]
    }
    fn c(&self, k: &str) -> Vec3 {
        let v = self.p(k);
        Vec3::new(v[0], v[1], v[2])
    }

    /// Radiance of M_Sky_Panning_Clouds2 for a UE world direction (ue_sky.gdshader sky(), asm lines there; GameTime 0)
    pub fn radiance(&self, d: Vec3) -> Vec3 {
        let tau = std::f32::consts::TAU;
        let h = d.z.clamp(0.0, 1.0);
        let el = d.z.clamp(-1.0, 1.0).asin();
        let uv0 = Vec2::new((0.75 - d.y.atan2(d.x) / tau).rem_euclid(1.0), 1.0 - el.abs() / (0.5 * std::f32::consts::PI));
        let tx = |t: &Option<Tex>, uv: Vec2| t.as_ref().map(|t| t.bilinear(uv.x, uv.y)).unwrap_or(0.0);
        let mut c = Vec3::splat(tx(&self.stars, uv0 * 12.0) * self.s("stars brightness") * self.s("sun height")) + self.c("zenith color");
        let hf = 1.0 - h;
        let f = if hf <= 0.0 { 0.0 } else { hf.powf(self.s("horizon falloff")).min(1.0) };
        c = c.lerp(self.c("horizon color"), f);
        let l = self.c("light direction").normalize_or_zero();
        let k = d.dot(-l) - 1.0;
        let sun = self.c("sun color");
        c += (1.0 - k.abs() / self.s("sun radius").max(1e-5)).clamp(0.0, 1.0) * sun * self.s("sun brightness");
        let t = 0.0 * self.s("cloud speed"); // GameTime at capture / first frame: UNCONFIRMED (clouds pan with it)
        let a = {
            let b = tx(&self.blue, uv0 + Vec2::new(0.0002 * t, 0.0));
            let cl = tx(&self.clouds, uv0 + Vec2::new(0.001 * t, 0.0));
            b + (cl - b) * h
        };
        let mut m = a * self.s("cloud opacity") * (1.0 - (el.sin() / -0.1).clamp(0.0, 1.0));
        let np = {
            let n1 = self.s("noisepower1");
            n1 + (self.s("noisepower2") - n1) * tx(&self.clouds, uv0 * 0.5)
        };
        m = if m <= 0.0 { 0.0 } else { m.powf(np) };
        let gb = (1.0 - k.abs() * 0.769231).max(0.0);
        let glow = if gb <= 0.0 { Vec3::ZERO } else { gb.powf(10.0) * sun };
        c = c.lerp(self.c("cloud color") * m + glow * 0.4 * m * m, (m * m).min(1.0)) * self.c("overall color");
        let sel = self.p("selectioncolor");
        (c * 1.5 + sel[3] * (Vec3::new(sel[0], sel[1], sel[2]) - 1.5 * c)).max(Vec3::ZERO)
    }
}

/// wgpu cube face + (u, v) -> direction (reflection.rs face_dir)
fn face_dir(f: usize, u: f32, v: f32) -> Vec3 {
    let (s, t) = (2.0 * u - 1.0, 2.0 * v - 1.0);
    match f {
        0 => Vec3::new(1.0, -t, -s),
        1 => Vec3::new(-1.0, -t, s),
        2 => Vec3::new(s, 1.0, t),
        3 => Vec3::new(s, -1.0, -t),
        4 => Vec3::new(s, -t, 1.0),
        _ => Vec3::new(-s, -t, -1.0),
    }
}

/// The UE direction a Bevy cube texel at wgpu face direction d stands for (module header)
pub fn texel_ue_dir(d: Vec3) -> Vec3 {
    Vec3::new(d.x, -d.z, d.y).normalize()
}

fn cube(size: u32, f: impl Fn(Vec3) -> Vec3) -> Image {
    let mut data = Vec::with_capacity((size * size * 6 * 8) as usize);
    for face in 0..6 {
        for y in 0..size {
            for x in 0..size {
                let d = face_dir(face, (x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
                let c = f(texel_ue_dir(d));
                for v in [c.x, c.y, c.z, 1.0] {
                    data.extend_from_slice(&crate::uepost_render::f32_to_f16(v.min(65000.0)).to_le_bytes());
                }
            }
        }
    }
    let mut img = Image::new(Extent3d { width: size, height: size, depth_or_array_layers: 6 }, TextureDimension::D2, data, TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD);
    img.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..Default::default() });
    img
}

/// The sky as the camera sees it (Skybox cube)
pub fn sky_cube(sky: &Sky, size: u32) -> Image {
    cube(size, |d| sky.radiance(d))
}

/// SH (order 3, 9 coefficients per channel) of the SkyLight capture: radiance x tint, lower hemisphere black
/// (bLowerHemisphereIsBlack), sampled on a CubemapResolution x CubemapResolution cube with texel solid angles.
pub fn capture_sh(sky: &Sky, tint: Vec3, res: u32) -> [Vec3; 9] {
    let mut sh = [Vec3::ZERO; 9];
    for face in 0..6 {
        for y in 0..res {
            for x in 0..res {
                let (u, v) = ((x as f32 + 0.5) / res as f32, (y as f32 + 0.5) / res as f32);
                let d = face_dir(face, u, v);
                // texel solid angle: (2/res)^2 / |d|^3 for the unnormalised face vector
                let dw = (2.0 / res as f32).powi(2) / d.length().powi(3);
                let ue = texel_ue_dir(d);
                if ue.z < 0.0 {
                    continue; // LowerHemisphereColor 0
                }
                let l = sky.radiance(ue) * tint;
                for (i, b) in sh_basis(ue).iter().enumerate() {
                    sh[i] += l * *b * dw;
                }
            }
        }
    }
    sh
}

fn sh_basis(d: Vec3) -> [f32; 9] {
    let (x, y, z) = (d.x, d.y, d.z);
    [
        0.282095,
        0.488603 * y,
        0.488603 * z,
        0.488603 * x,
        1.092548 * x * y,
        1.092548 * y * z,
        0.315392 * (3.0 * z * z - 1.0),
        1.092548 * x * z,
        0.546274 * (x * x - y * y),
    ]
}

/// Lambert diffuse radiance for a surface normal: E(n) / pi from the SH (cosine-lobe convolution pi, 2pi/3, pi/4;
/// Ramamoorthi & Hanrahan 2001). UE's own SH convolution constants (FSHVectorRGB3 / CalcDiffuseTransferSH3) are not read
/// from the exe yet: UNCONFIRMED normalisation (this one returns L for a uniform full-sphere radiance L).
pub fn sh_diffuse(sh: &[Vec3; 9], n: Vec3) -> Vec3 {
    let a = [std::f32::consts::PI, 2.0 * std::f32::consts::PI / 3.0, std::f32::consts::PI / 4.0];
    let b = sh_basis(n);
    let band = [0, 1, 1, 1, 2, 2, 2, 2, 2];
    let mut e = Vec3::ZERO;
    for i in 0..9 {
        e += sh[i] * a[band[i]] * b[i];
    }
    (e / std::f32::consts::PI).max(Vec3::ZERO)
}

/// The SkyLight's diffuse cube for Bevy's EnvironmentMapLight (sampled at the normal)
pub fn diffuse_cube(sh: &[Vec3; 9], size: u32) -> Image {
    cube(size, |n| sh_diffuse(sh, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convention_maps_up_and_forward() {
        // Bevy +Y (face 2 centre) is UE +Z; Bevy samples +Z with z negated -> face 5 centre (-z) holds Bevy +Z = UE +Y
        assert!((texel_ue_dir(Vec3::new(0.0, 1.0, 0.0)) - Vec3::Z).length() < 1e-6);
        assert!((texel_ue_dir(Vec3::new(0.0, 0.0, -1.0)) - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn uniform_sky_gives_its_radiance_on_an_up_surface() {
        let sky = Sky { params: [("zenith color".to_string(), [1.0, 1.0, 1.0, 1.0]), ("horizon color".to_string(), [1.0, 1.0, 1.0, 1.0]),
            ("sun brightness".to_string(), [0.0; 4]), ("cloud opacity".to_string(), [0.0; 4])].into_iter().collect(),
            clouds: None, blue: None, stars: None };
        // radiance: 1 x 1.5 everywhere above the horizon
        let sh = capture_sh(&sky, Vec3::ONE, 48);
        let up = sh_diffuse(&sh, Vec3::Z);
        assert!((up.x - 1.5).abs() < 0.08, "{up}");
        // a wall sees half the upper hemisphere's cosine-weighted light
        let side = sh_diffuse(&sh, Vec3::X);
        assert!((side.x - 0.75).abs() < 0.12, "{side}");
    }
}

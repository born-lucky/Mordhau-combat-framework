//! lighting.rs - the map's own light, sky, fog and post values (mh-level `Lighting`, read from the paks) as Bevy
//! lights and camera settings. Port of godot/components/ue/ue_light.gd (sun_node, local_node, color, energy,
//! skylight_tint, ue_exposure, height_fog_node); its comments carry the sources and are cited per function here.
//! Owner: rust-render (r1 rewrote the camera look: UE's own post chain, sky and SkyLight replace Bevy's).
//!
//! Units: Bevy works in physical units like UE (lux for the sun, lumens for local lights, cd/m^2 for ambient), so the
//! Godot port's 1/PI non-physical scale (ue_light.gd GODOT_PI_SCALE) does not apply: UE values go in unscaled. Bevy's
//! camera exposure is pinned to 1 (ev100 = -log2(1.2)): the scene colour reaches the post chain in UE scene units and
//! uepost_render.rs applies UE's eye adaptation, bloom and tonemapper.
//!
//! - Post: the unbound PostProcessVolume over UE's FPostProcessSettings defaults (uepost.rs PostSettings), the
//!   CombineLUT volume (film curve + ColorGradingLUT) built here, eye adaptation / bloom / tonemap on the GPU.
//! - Sky: BP_Sky_Sphere's MID evaluated into a Skybox cube; the stationary SkyLight's diffuse is the SH irradiance of
//!   that sky x LightColor x Intensity (uesky.rs), on the camera's and every reflection probe's EnvironmentMapLight.
//! - ExponentialHeightFog: StartDistance 5000 cm on DU_Arena; Bevy's DistanceFog (no height term, no start distance)
//!   is no longer used. UE's height fog pass is not drawn yet (UNCONFIRMED gap, see RUST_RUNTIME.md rust-render r1).
//! - Static (baked) local lights exist only in the lightmaps (ue_light.gd read "baked").

use crate::level::ue_look_basis;
use crate::plan::xf_gltf;
use bevy::prelude::*;
use serde_json::{Map, Value};

/// ue_light.gd DEF (CUE4Parse ULightComponent.cs class defaults, file:line there)
fn def(k: &str) -> Value {
    match k {
        "Intensity" => Value::from(std::f64::consts::PI), // ULightComponent.cs:21
        "Temperature" => Value::from(6500.0),             // :49
        "bUseTemperature" => Value::from(false),          // :52
        "AttenuationRadius" => Value::from(1000.0),       // :97
        "SourceRadius" => Value::from(0.0),               // :193
        "bUseInverseSquaredFalloff" => Value::from(true), // :196
        "SourceWidth" => Value::from(64.0),               // :246
        "SourceHeight" => Value::from(64.0),              // :247
        "CastShadows" => Value::from(true),               // engine default true (ue_light.gd DEF note)
        _ => Value::Null,
    }
}
fn pf(p: &Map<String, Value>, k: &str) -> f32 {
    p.get(k).cloned().unwrap_or_else(|| def(k)).as_f64().unwrap_or(0.0) as f32
}
fn pb(p: &Map<String, Value>, k: &str) -> bool {
    p.get(k).cloned().unwrap_or_else(|| def(k)).as_bool().unwrap_or(false)
}

/// FLinearColor.MakeFromColorTemperature (CUE4Parse FLinearColor.cs:120-142; ue_light.gd temp_color)
pub fn temp_color(t: f32) -> [f32; 3] {
    let t = t.clamp(1000.0, 15000.0);
    let u = (0.860117757 + 1.54118254e-4 * t + 1.28641212e-7 * t * t) / (1.0 + 8.42420235e-4 * t + 7.08145163e-7 * t * t);
    let v = (0.317398726 + 4.22806245e-5 * t + 4.20481691e-8 * t * t) / (1.0 - 2.89741816e-5 * t + 1.61456053e-7 * t * t);
    let x = 3.0 * u / (2.0 * u - 8.0 * v + 4.0);
    let y = 2.0 * v / (2.0 * u - 8.0 * v + 4.0);
    let z = 1.0 - x - y;
    let (xx, zz) = (x / y, z / y);
    [
        (3.2404542 * xx - 1.5371385 - 0.4985314 * zz).max(0.0),
        (-0.9692660 * xx + 1.8760108 + 0.0415560 * zz).max(0.0),
        (0.0556434 * xx - 0.2040259 + 1.0572252 * zz).max(0.0),
    ]
}

/// LightColor (8-bit FColor, decoded as sRGB by UE: FLightSceneProxy ctor .text 0x2f96f64 via FLinearColor(FColor)
/// 0x189e990) x MakeFromColorTemperature when bUseTemperature (ue_light.gd color)
pub fn color(p: &Map<String, Value>) -> Color {
    let c = p.get("LightColor");
    let g = |k: &str| c.and_then(|c| c.get(k)).and_then(|x| x.as_f64()).unwrap_or(255.0) as u8;
    let lin = Color::srgb_u8(g("R"), g("G"), g("B")).to_linear();
    if pb(p, "bUseTemperature") {
        let k = temp_color(pf(p, "Temperature"));
        Color::linear_rgb(lin.red * k[0], lin.green * k[1], lin.blue * k[2])
    } else {
        lin.into()
    }
}

/// Luminous intensity in candela of a point/spot light, or None for the legacy (non inverse-squared) falloff, which
/// Bevy cannot express (ue_light.gd energy: Lumens / 4 pi, Candelas, Unitless / 625 = CUE4Parse LightUtils.cs:25)
fn point_candela(p: &Map<String, Value>) -> Option<f32> {
    if !pb(p, "bUseInverseSquaredFalloff") {
        return None;
    }
    let i = pf(p, "Intensity");
    Some(match p.get("IntensityUnits").and_then(|u| u.as_str()).unwrap_or("ELightUnits::Unitless") {
        "ELightUnits::Lumens" => i / (4.0 * std::f32::consts::PI),
        "ELightUnits::Candelas" => i,
        _ => i / 625.0,
    })
}

/// Rect light luminous power in lumens: Lumens as is; Candelas and Unitless (/ 625 -> cd) x PI (a Lambertian
/// emitter's I = Phi / PI; CUE4Parse LightUtils.ConvertToIntensityToNits uses angle PI for rect lights)
fn rect_lumens(p: &Map<String, Value>) -> f32 {
    let i = pf(p, "Intensity");
    match p.get("IntensityUnits").and_then(|u| u.as_str()).unwrap_or("ELightUnits::Unitless") {
        "ELightUnits::Lumens" => i,
        "ELightUnits::Candelas" => i * std::f32::consts::PI,
        _ => i / 625.0 * std::f32::consts::PI,
    }
}

/// FPostProcessSettings defaults read from the exe (ue_light.gd PP_DEFAULTS: FPostProcessSettings ctor .text 0x33bf...)
const PP_BLOOM: f32 = 0.675;
const PP_MIN_B: f32 = 0.03;
const PP_MAX_B: f32 = 8.0;

/// AEM_Basic steady-state exposure at the clamp's top (ue_light.gd ue_exposure; GetEyeAdaptationParameters .text
/// 0x21749a0 / GetEyeAdaptationFixedExposure 0x2174950): 0.18 / (0.18 max(Min, Max)) * 2^Bias. Only bOverride_ values count.
pub fn ue_exposure(pp: &Map<String, Value>) -> f32 {
    let ov = |k: &str, d: f32| {
        if pp.get(&format!("bOverride_{k}")).and_then(|b| b.as_bool()).unwrap_or(false) {
            pp.get(k).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(d)
        } else {
            d
        }
    };
    let (mn, mx) = (ov("AutoExposureMinBrightness", PP_MIN_B), ov("AutoExposureMaxBrightness", PP_MAX_B));
    // AutoExposureBias only with its bOverride_ flag (FSceneView::OverridePostProcessSettings 0x33d4b60); the default is
    // r.DefaultFeature.AutoExposure.Bias = 0 (FPostProcessSettings ctor 0x1433c0178, DefaultEngine.ini:249). DU_Arena
    // serializes 0.1699 without the flag: not applied (the r2..r7 runs applied it: 0.75 instead of 0.667).
    let bias = ov("AutoExposureBias", 0.0);
    0.18 / (0.18 * mn.max(mx)).max(1e-4) * 2f32.powf(bias)
}

/// Bevy exposure: pixel = L * 2^-ev100 / 1.2 (bevy_camera Exposure::exposure) -> ev100 for a UE exposure multiplier
pub fn ev100_for(exposure: f32) -> f32 {
    -(1.2 * exposure).log2()
}

/// ExponentialHeightFog scales from the exe (ue_light.gd FOG_DEFAULTS / height_fog_node: FExponentialHeightFogSceneInfo
/// .text 0x226d310 stores FogDensity * 0.001 and FogHeightFalloff * 0.001; ctor 0x142f97930 defaults 0.02 / 0.2)
#[derive(Clone, Debug, serde::Serialize)]
pub struct HeightFog {
    pub density_cm: f32,
    pub falloff_cm: f32,
    pub height_cm: f32,
    pub start_cm: f32,
    pub inscatter: [f32; 3],
}

impl HeightFog {
    /// UE density per metre at height y (metres, glTF Y)
    pub fn density_at(&self, y_m: f32) -> f32 {
        self.density_cm * 100.0 * (-self.falloff_cm * (y_m * 100.0 - self.height_cm)).exp()
    }
}

/// What the camera takes from the map (camera.rs applies it).
#[derive(Resource, Clone, Debug, Default, serde::Serialize)]
pub struct Look {
    pub exposure: f32,
    pub ev100: f32,
    pub bloom: Option<f32>,
    pub fog: Option<HeightFog>,
    pub clear: [f32; 3],
    pub version: u32,
    /// specular environment from the map's reflection capture (reflection.rs): (specular cube, black diffuse cube,
    /// intensity); None = no environment map
    #[serde(skip)]
    pub env: Option<(Handle<Image>, Handle<Image>, f32)>,
    pub env_info: Option<serde_json::Value>,
    /// UE post settings of the level (uepost.rs), the CombineLUT volume, the sky cube and the SkyLight diffuse cube
    pub post: Option<crate::uepost::PostSettings>,
    #[serde(skip)]
    pub lut: Option<Handle<Image>>,
    #[serde(skip)]
    pub sky_cube: Option<Handle<Image>>,
    #[serde(skip)]
    pub sky_diffuse: Option<Handle<Image>>,
    pub post_info: Option<serde_json::Value>,
}

#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct LightStats {
    pub sun: Option<serde_json::Value>,
    pub sky_light_intensity: Option<f32>,
    pub ambient: Option<[f32; 4]>,
    pub sky: Option<serde_json::Value>,
    pub locals_dynamic: usize,
    pub locals_baked_skipped: usize,
    pub locals_legacy_falloff: usize,
    pub post: Option<serde_json::Value>,
    pub fog: Option<HeightFog>,
}

/// Sky MID colours (ue_light.gd sky_mid: the level's saved MaterialInstanceDynamic of BP_Sky_Sphere)
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SkyColors {
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    /// every MID parameter, lower-case name -> value (scalars in [0]); uesky.rs evaluates the sky material with them
    pub params: std::collections::HashMap<String, [f32; 4]>,
}

pub fn sky_mid(pk: &mh_level::Pkgs, sky: &mh_level::LightRec) -> Option<SkyColors> {
    let mid = pk.obj(sky.props.get("Sky material"))?;
    let mut out = SkyColors { zenith: [1.0; 3], horizon: [1.0; 3], params: Default::default() };
    let mut any = false;
    for e in mid.get("Properties").and_then(|p| p.get("ScalarParameterValues")).and_then(|a| a.as_array()).into_iter().flatten() {
        let n = e.pointer("/ParameterInfo/Name").and_then(|s| s.as_str()).unwrap_or("").to_lowercase();
        let v = e.get("ParameterValue").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
        out.params.insert(n, [v, 0.0, 0.0, 0.0]);
    }
    for e in mid.get("Properties").and_then(|p| p.get("VectorParameterValues")).and_then(|a| a.as_array()).into_iter().flatten() {
        let n = e.pointer("/ParameterInfo/Name").and_then(|s| s.as_str()).unwrap_or("").to_lowercase();
        let v = e.get("ParameterValue");
        let g = |k: &str| v.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
        let c = [g("R"), g("G"), g("B")];
        out.params.insert(n.clone(), [c[0], c[1], c[2], g("A")]);
        match n.as_str() {
            "zenith color" => { out.zenith = c; any = true; }
            "horizon color" => { out.horizon = c; any = true; }
            _ => {}
        }
    }
    any.then_some(out)
}

/// Spawn the map's lights under `root`; returns the stats and the camera look.
pub fn spawn(commands: &mut Commands, root: Entity, l: &mh_level::Lighting, sky: Option<&SkyColors>, ambient: &mut GlobalAmbientLight) -> (LightStats, Look) {
    let mut st = LightStats::default();
    let pp = l.post.as_ref().map(|p| p.props.clone()).unwrap_or_default();
    let exposure = ue_exposure(&pp);
    let mut look = Look { exposure, ev100: ev100_for(exposure), ..Default::default() };
    if pp.get("bOverride_BloomIntensity").and_then(|b| b.as_bool()).unwrap_or(false) {
        let b = pp.get("BloomIntensity").and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(PP_BLOOM);
        look.bloom = Some(b / PP_BLOOM * bevy::post_process::bloom::Bloom::NATURAL.intensity);
    }
    st.post = Some(serde_json::json!({"exposure": exposure, "ev100": look.ev100, "bloom_bevy": look.bloom,
        "bOverride_keys": pp.keys().filter(|k| k.starts_with("bOverride_")).collect::<Vec<_>>()}));
    if let Some(s) = &l.sun {
        let p = &s.props;
        let lux = pf(p, "Intensity");
        let c = color(p);
        commands.spawn((
            DirectionalLight { illuminance: lux, color: c, shadow_maps_enabled: pb(p, "CastShadows"), ..default() },
            bevy::camera::visibility::RenderLayers::from_layers(&[0, crate::fighter::SHADOW_LAYER]),
            Transform::from_rotation(ue_look_basis(&xf_gltf(&s.xf))),
            Name::new(format!("Sun {}", s.name)),
            ChildOf(root),
        ));
        st.sun = Some(serde_json::json!({"name": s.name, "lux": lux, "color_linear": c.to_linear().to_f32_array(),
            "shadows": pb(p, "CastShadows"), "dir": (ue_look_basis(&xf_gltf(&s.xf)) * Vec3::NEG_Z).to_array()}));
    }
    // SkyLight: linear LightColor x Intensity (ue_light.gd skylight_tint, FSkyLightSceneProxy .text 0x2fe9f70)
    let tint = l.sky_light.as_ref().map(|s| {
        st.sky_light_intensity = Some(pf(&s.props, "Intensity"));
        let c = color(&s.props).to_linear();
        let i = pf(&s.props, "Intensity");
        [c.red * i, c.green * i, c.blue * i]
    });
    let skyc = sky.cloned().unwrap_or(SkyColors { zenith: [0.3, 0.45, 0.8], horizon: [0.6, 0.7, 0.85], params: Default::default() }); // no MID: UNCONFIRMED stand-in
    st.sky = Some(serde_json::json!({"from_mid": sky.is_some(), "zenith": skyc.zenith, "horizon": skyc.horizon}));
    if let Some(t) = tint {
        let m = [0.5 * (skyc.zenith[0] + skyc.horizon[0]), 0.5 * (skyc.zenith[1] + skyc.horizon[1]), 0.5 * (skyc.zenith[2] + skyc.horizon[2])];
        let a = [m[0] * t[0], m[1] * t[1], m[2] * t[2]];
        let mx = a[0].max(a[1]).max(a[2]).max(1e-6);
        ambient.color = Color::linear_rgb(a[0] / mx, a[1] / mx, a[2] / mx);
        ambient.brightness = mx; // cd/m^2: the sky's radiance (UE sky emissive = luminance)
        st.ambient = Some([a[0], a[1], a[2], mx]);
    }
    // (rust-armory) where no geometry is drawn UE's scene colour stays black (the SkyLight lights surfaces, it draws no
    // background); only an AtmosphericFog draws a sky there (the horizon colour stands in for it). MainMenu has
    // neither: black below the dome's rim, as state/reference/real/00_start.png
    look.clear = if l.atmos.is_some() { [skyc.horizon[0] * exposure, skyc.horizon[1] * exposure, skyc.horizon[2] * exposure] } else { [0.0; 3] };
    for r in &l.locals {
        if r.baked {
            st.locals_baked_skipped += 1;
            continue;
        }
        let p = &r.props;
        let xf = xf_gltf(&r.xf);
        let tr = Transform::from_translation(xf.translation.into()).with_rotation(ue_look_basis(&xf));
        let range = pf(p, "AttenuationRadius") * 0.01;
        match r.kind.as_str() {
            // (rust-armory) a further directional light (mh-level keeps all but the first with the locals): Intensity
            // in lux as the sun's, dynamic shadows only when it casts them (UDirectionalLightComponent
            // CastDynamicShadows / CastShadows; MainMenu's FillLight / RimLight set CastDynamicShadows false)
            "DirectionalLightComponent" => {
                let shadows = pb(p, "CastShadows") && p.get("CastDynamicShadows").and_then(|v| v.as_bool()).unwrap_or(true);
                commands.spawn((
                    DirectionalLight { illuminance: pf(p, "Intensity"), color: color(p), shadow_maps_enabled: shadows, ..default() },
                    bevy::camera::visibility::RenderLayers::from_layers(&[0, crate::fighter::SHADOW_LAYER]),
                    Transform::from_rotation(ue_look_basis(&xf)),
                    Name::new(r.name.clone()),
                    ChildOf(root),
                ));
            }
            "RectLightComponent" => {
                commands.spawn((
                    RectLight { color: color(p), intensity: rect_lumens(p), range, width: pf(p, "SourceWidth") * 0.01, height: pf(p, "SourceHeight") * 0.01 },
                    tr, Name::new(r.name.clone()), ChildOf(root),
                ));
            }
            _ => {
                let Some(cd) = point_candela(p) else {
                    st.locals_legacy_falloff += 1;
                    continue;
                };
                let lm = cd * 4.0 * std::f32::consts::PI;
                if r.kind == "SpotLightComponent" {
                    let outer = p.get("OuterConeAngle").and_then(|x| x.as_f64()).unwrap_or(44.0) as f32; // USpotLightComponent default 44 [recalled, UNCONFIRMED]
                    let inner = p.get("InnerConeAngle").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                    commands.spawn((
                        SpotLight { color: color(p), intensity: lm, range, radius: pf(p, "SourceRadius") * 0.01,
                            shadow_maps_enabled: pb(p, "CastShadows"), outer_angle: outer.to_radians(), inner_angle: inner.to_radians(), ..default() },
                        tr, Name::new(r.name.clone()), ChildOf(root),
                    ));
                } else {
                    commands.spawn((
                        PointLight { color: color(p), intensity: lm, range, radius: pf(p, "SourceRadius") * 0.01,
                            shadow_maps_enabled: pb(p, "CastShadows"), ..default() },
                        tr, Name::new(r.name.clone()), ChildOf(root),
                    ));
                }
            }
        }
        st.locals_dynamic += 1;
    }
    if let Some(f) = &l.fog {
        let p = &f.props;
        let g = |k: &str, d: f32| p.get(k).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(d);
        let ic = p.get("FogInscatteringColor");
        let c = |k: &str| ic.and_then(|c| c.get(k)).and_then(|x| x.as_f64()).unwrap_or(1.0) as f32;
        let hf = HeightFog {
            density_cm: g("FogDensity", 0.02) * 0.001,
            falloff_cm: g("FogHeightFalloff", 0.2) * 0.001,
            height_cm: f.xf.translation()[2] as f32,
            start_cm: g("StartDistance", 0.0),
            inscatter: [c("R"), c("G"), c("B")],
        };
        st.fog = Some(hf.clone());
        look.fog = Some(hf);
    }
    look.version += 1;
    (st, look)
}

/// SkyCube size of the visible sky (Skybox); the SkyLight capture itself uses CubemapResolution 128 (uesky.rs)
pub const SKY_CUBE: u32 = 256;
/// Bevy diffuse environment cube size (an SH3 irradiance is smooth)
pub const SKY_DIFFUSE_CUBE: u32 = 16;

/// The level's UE post settings, CombineLUT, sky and SkyLight into the Look (rust-render r1). `pak`: the user's paks
/// (the ColorGradingLUT and the EngineSky textures are read from them).
pub fn build_post(images: &mut Assets<Image>, pak: Option<&mh_assets::pak_source::PakSource>, l: &mh_level::Lighting, sky: Option<&SkyColors>,
    ambient: &mut GlobalAmbientLight, look: &mut Look) {
    let t0 = std::time::Instant::now();
    let pp = l.post.as_ref().map(|p| p.props.clone()).unwrap_or_default();
    let ps = crate::uepost::PostSettings::from_volume(&pp);
    let user = match (pak, &ps.color_grading_lut) {
        (Some(src), Some(path)) => crate::paksrc::tex_info(src, path).ok().and_then(|t| {
            let w = t.mips.first()?.size_x.max(1) as usize;
            crate::paksrc::decoded_rgba8(src, &t).ok().map(|rgba8| crate::uepost::UserLut { w, rgba8 })
        }),
        _ => None,
    };
    let lut = crate::uepost::combined_lut(&ps, user.as_ref());
    look.lut = Some(images.add(crate::uepost_render::lut_image(&lut)));
    let eye = crate::uepost::eye_params(&ps);
    // the sky and the SkyLight
    let mut sky_info = serde_json::Value::Null;
    if let Some(sc) = sky {
        let ld = |p: &str| pak.and_then(|s| crate::uesky::load_tex(s, p));
        let skym = crate::uesky::Sky {
            params: sc.params.clone(),
            clouds: ld("Engine/Content/EngineSky/T_Sky_Clouds_M"),
            blue: ld("Engine/Content/EngineSky/T_Sky_Blue"),
            stars: ld("Engine/Content/EngineSky/T_Sky_Stars"),
        };
        look.sky_cube = Some(images.add(crate::uesky::sky_cube(&skym, SKY_CUBE)));
        if let Some(sl) = &l.sky_light {
            let c = color(&sl.props).to_linear();
            let i = pf(&sl.props, "Intensity");
            let tint = Vec3::new(c.red * i, c.green * i, c.blue * i);
            let sh = crate::uesky::capture_sh(&skym, tint, 128);
            let up = crate::uesky::sh_diffuse(&sh, Vec3::Z);
            let side = crate::uesky::sh_diffuse(&sh, Vec3::X);
            look.sky_diffuse = Some(images.add(crate::uesky::diffuse_cube(&sh, SKY_DIFFUSE_CUBE)));
            // UE has no ambient term: the SkyLight is the only sky light
            ambient.brightness = 0.0;
            sky_info = serde_json::json!({"skylight_tint": tint.to_array(), "diffuse_up": up.to_array(), "diffuse_side": side.to_array(),
                "textures": [skym.clouds.is_some(), skym.blue.is_some(), skym.stars.is_some()],
                "zenith_radiance": skym.radiance(Vec3::Z).to_array(), "horizon_radiance": skym.radiance(Vec3::X).to_array()});
        }
    }
    look.post_info = Some(serde_json::json!({"settings": &ps, "eye": eye, "user_lut": user.is_some(),
        "steady_exposure_bright_scene": crate::uepost::steady_exposure(&eye, 10.0),
        "sky": sky_info, "height_fog_not_drawn": look.fog.clone(), "build_secs": t0.elapsed().as_secs_f32()}));
    look.post = Some(ps);
    // Bevy's camera exposure 1: UE scene units into the post chain
    look.exposure = 1.0;
    look.ev100 = ev100_for(1.0);
    look.bloom = None;
    look.fog = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exposure_matches_ue_light_gd_arena_comment() {
        // ue_light.gd camera_attributes: Arena Min/Max 0.8 / 1.5 (overridden), Bias 0 -> 2^0 / 1.5 = 0.667;
        // the comment's 0.75 includes Bias; check the formula itself
        let pp = json!({"bOverride_AutoExposureMinBrightness": true, "AutoExposureMinBrightness": 0.8,
                        "bOverride_AutoExposureMaxBrightness": true, "AutoExposureMaxBrightness": 1.5});
        let e = ue_exposure(pp.as_object().unwrap());
        assert!((e - 1.0 / 1.5).abs() < 1e-6);
        // no overrides: Max default 8
        assert!((ue_exposure(&Map::new()) - 0.125).abs() < 1e-6);
        // Bevy round trip
        let ev = ev100_for(e);
        assert!((2f32.powf(-ev) / 1.2 - e).abs() < 1e-6);
    }

    #[test]
    fn d65_is_near_white() {
        let c = temp_color(6500.0);
        assert!(c.iter().all(|x| (x - 1.0).abs() < 0.06), "{c:?}");
    }
}

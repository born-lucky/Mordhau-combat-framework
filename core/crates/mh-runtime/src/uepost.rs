//! uepost.rs - UE 4.26's post-process settings and the CPU half of its image pipeline as Mordhau's shipped exe runs it
//! (rust-render r1). The GPU half (passes, WGSL) is uepost_render.rs. Every value is read from the exe
//! (Mordhau-Win64-Shipping.exe build 702625635, disassembled by RVA with the PDB names in extract/native/pdb_procs.tsv),
//! from the game's compiled global shaders (Engine/GlobalShaderCache-PCD3D_SM5.bin -> state/shaders/_global/out/...)
//! or from the map's data. Nothing here is Bevy's: Bevy's tonemapper, bloom, exposure and fog are switched off on the
//! camera (camera.rs) and replaced by these passes.
//!
//! Frame pipeline (AddPostProcessingPasses rva 0x216d6d0, the calls at 0x216e3fa..0x216ec63):
//!   scene colour -> AddDownsamplePass (half res, r.Downsample.Quality = 1 = High: CVarDownsampleQuality init 0x6c47d0)
//!   -> FSceneDownsampleChain::Init (rva 0x217d170, bLogLumaInAlpha = 1 for AEM_Basic): 5 more halvings (1/4..1/64),
//!      stage 1 (1/4) followed by AddBasicEyeAdaptationSetupPass (0x215fab0) -> log luminance in alpha
//!   -> AddBasicEyeAdaptationPass (0x215f370) on the 1/64 texture -> the exposure texture (1x1, persistent)
//!   -> bloom: BloomThreshold -1 (not > -1) reuses that chain (0x216e648-0x216e66f); AddBloomPass (0x210c530):
//!      BloomQuality 5 -> 6 stages, Bloom6..Bloom1 over chain[5]..chain[0], each AddGaussianBlurPass (0x21acf20)
//!   -> AddCombineLUTPass (0x215fdf0, the film curve + grading LUT, a 32^3 volume) -> AddTonemapPass (0x21b2ed0).
//! Scalability: a fresh install picks Epic (3) in every group: FQualityLevels::SetDefaults (0x33dc0c0) stores
//! sg.<Group>.NumLevels - 2 = 5 - 2 (the NumLevels cvars' initializers pass 5), and no game code calls
//! RunHardwareBenchmark (extract/native/decomp: UMordhauGameUserSettings::SetToDefaults only clears the benchmark
//! results). DefaultScalability.ini @3 (from the paks): BloomQuality 5, EyeAdaptationQuality 2, SceneColorFringeQuality 1,
//! Tonemapper.Quality 5, Tonemapper.GrainQuantization 1, Filter.SizeScale 1, FastBlurThreshold 100,
//! PostProcessAAQuality 4 (TAA), MotionBlurQuality 3, AmbientOcclusionLevels -1.

use serde_json::{Map, Value};

/// FPostProcessSettings, the fields the image pipeline reads. `Default` = FPostProcessSettings::FPostProcessSettings
/// (rva 0x33bfb30: memset 0 over 0x550 bytes, then the stores 0x1433bfcc6..0x1433c04ae; field offsets from the PDB
/// layout extract/native/types/FPostProcessSettings.h) followed by FSceneView::StartFinalPostprocessSettings
/// (rva 0x33df570: ColorGradingIntensity +0x474 and AmbientCubemapIntensity +0x2f4 zeroed, project DefaultFeature
/// cvars applied: r.DefaultFeature.AutoExposure.Method = 1 from DefaultEngine.ini:247 -> AutoExposureMethod +0x22,
/// r.DefaultFeature.LensFlare default 0 -> LensFlareIntensity 0; Bloom / AmbientOcclusion / AutoExposure /
/// MotionBlur / AmbientOcclusionStaticFraction default 1 = kept).
#[derive(Clone, Debug, serde::Serialize)]
pub struct PostSettings {
    pub white_temp: f32,              // +0x24 = 6500 (qword 0x45cb2000 at 0x1433bfcc6)
    pub white_tint: f32,              // +0x28 = 0 (same qword)
    /// ColorSaturation / Contrast / Gamma / Gain / Offset for the Global, Shadows, Midtones and Highlights sets
    /// (+0x30..+0x16f, FVector4 each, PDB layout FPostProcessSettings.h:196-215): (1,1,1,1) except the Offsets
    /// (0,0,0,0) (ctor stores 0x1433bfcd6-0x1433bfd4f: xmm6 = (1,1,1,1) @0x143fe0df0, xmm0 = 0 for +0x70/+0xc0/+0x110/+0x160)
    pub color_grade: [[[f32; 4]; 5]; 4],
    pub cc_shadows_max: f32,          // +0x174 = 0.09 (0x1433bfd56)
    pub cc_highlights_min: f32,       // +0x170 = 0.5 (0x1433bfd60)
    pub blue_correction: f32,         // +0x178 = 0.6 (0x1433bfd6a)
    pub expand_gamut: f32,            // +0x17c = 1
    pub tone_curve_amount: f32,       // +0x180 = 1
    pub film_slope: f32,              // +0x184 = 0.88 (0x1433bfe00)
    pub film_toe: f32,                // +0x188 = 0.55
    pub film_shoulder: f32,           // +0x18c = 0.26 (qword 0x3e851eb8 at 0x1433bfe14)
    pub film_black_clip: f32,         // +0x190 = 0 (high dword of that qword)
    pub film_white_clip: f32,         // +0x194 = 0.04
    pub scene_color_tint: [f32; 4],   // +0x204 = (1, 1, 1, 1) (xmm6 at 0x1433bfe29)
    pub scene_fringe_intensity: f32,  // +0x214 = 0 (0x1433bfe30)
    pub ca_start_offset: f32,         // +0x218 = 0 (memset)
    pub bloom_intensity: f32,         // +0x21c = 0.675 (0x1433bfe3a)
    pub bloom_threshold: f32,         // +0x220 = -1
    pub bloom_size_scale: f32,        // +0x224 = 4
    pub bloom_sizes: [f32; 6],        // +0x228.. = 0.3, 1, 2, 10, 30, 64 (0x1433bfe67..0x1433bfedc)
    pub bloom_tints: [[f32; 4]; 6],   // +0x240.. = .3465, .138, .1176, .066, .066, .061 grey, a 1 (.rdata 0x144a6d920..0x144a6d8e0)
    pub bloom_dirt_intensity: f32,    // +0x2d0 = 0
    pub auto_exposure_method: u8,     // +0x22: 0 Histogram, 1 Basic, 2 Manual
    pub auto_exposure_bias: f32,      // +0x314 = r.DefaultFeature.AutoExposure.Bias (0x1433c0178) = 0 (DefaultEngine.ini:249)
    pub auto_exposure_low_percent: f32,  // +0x338 = 10
    pub auto_exposure_high_percent: f32, // +0x33c = 90
    pub auto_exposure_min_brightness: f32, // +0x340 = 0.03 (ExtendDefaultLuminanceRange 0: 0x1433c008d..0x1433c00b5)
    pub auto_exposure_max_brightness: f32, // +0x344 = 8
    pub auto_exposure_speed_up: f32,  // +0x348 = 3 (0x1433c0180)
    pub auto_exposure_speed_down: f32, // +0x34c = 1
    pub histogram_log_min: f32,       // +0x350 = -8
    pub histogram_log_max: f32,       // +0x354 = 4
    pub vignette_intensity: f32,      // +0x400 = 0.4 (qword 0x3ecccccd at 0x1433c01ff)
    pub grain_jitter: f32,            // +0x404 = 0
    pub grain_intensity: f32,         // +0x408 = 0
    pub ambient_occlusion_intensity: f32, // +0x40c = 0.5
    pub ambient_occlusion_radius: f32,    // +0x414 = 200
    pub indirect_lighting_intensity: f32, // +0x464 = 1
    pub color_grading_intensity: f32, // +0x474 = 1, zeroed by StartFinalPostprocessSettings (0x1433df5db)
    /// ColorGradingLUT texture package (+0x478), None = neutral
    pub color_grading_lut: Option<String>,
    pub motion_blur_amount: f32,      // +0x4b4 = 0.5
    pub motion_blur_max: f32,         // +0x4b8 = 5
    pub lens_flare_intensity: f32,    // +0x35c = 1, zeroed by r.DefaultFeature.LensFlare 0 (0x1433df6ea)
    pub screen_percentage: f32,       // +0x538 = 100
    /// bOverride_ keys of the volume this port does not apply (reported in the run's evidence)
    pub not_ported: Vec<String>,
}

impl Default for PostSettings {
    fn default() -> Self {
        PostSettings {
            white_temp: 6500.0,
            white_tint: 0.0,
            color_grade: [[[1.0; 4], [1.0; 4], [1.0; 4], [1.0; 4], [0.0; 4]]; 4],
            cc_shadows_max: 0.09,
            cc_highlights_min: 0.5,
            blue_correction: 0.6,
            expand_gamut: 1.0,
            tone_curve_amount: 1.0,
            film_slope: 0.88,
            film_toe: 0.55,
            film_shoulder: 0.26,
            film_black_clip: 0.0,
            film_white_clip: 0.04,
            scene_color_tint: [1.0; 4],
            scene_fringe_intensity: 0.0,
            ca_start_offset: 0.0,
            bloom_intensity: 0.675,
            bloom_threshold: -1.0,
            bloom_size_scale: 4.0,
            bloom_sizes: [0.3, 1.0, 2.0, 10.0, 30.0, 64.0],
            bloom_tints: [[0.3465, 0.3465, 0.3465, 1.0], [0.138, 0.138, 0.138, 1.0], [0.1176, 0.1176, 0.1176, 1.0],
                [0.066, 0.066, 0.066, 1.0], [0.066, 0.066, 0.066, 1.0], [0.061, 0.061, 0.061, 1.0]],
            bloom_dirt_intensity: 0.0,
            auto_exposure_method: 1, // StartFinalPostprocessSettings 0x1433df6c5 with the project's Method = 1
            auto_exposure_bias: 0.0,
            auto_exposure_low_percent: 10.0,
            auto_exposure_high_percent: 90.0,
            auto_exposure_min_brightness: 0.03,
            auto_exposure_max_brightness: 8.0,
            auto_exposure_speed_up: 3.0,
            auto_exposure_speed_down: 1.0,
            histogram_log_min: -8.0,
            histogram_log_max: 4.0,
            vignette_intensity: 0.4,
            grain_jitter: 0.0,
            grain_intensity: 0.0,
            ambient_occlusion_intensity: 0.5,
            ambient_occlusion_radius: 200.0,
            indirect_lighting_intensity: 1.0,
            color_grading_intensity: 0.0,
            color_grading_lut: None,
            motion_blur_amount: 0.5,
            motion_blur_max: 5.0,
            lens_flare_intensity: 0.0,
            screen_percentage: 100.0,
            not_ported: Vec::new(),
        }
    }
}

/// (set, kind) of a colour-grading member name: set 0 Global, 1 Shadows, 2 Midtones, 3 Highlights; kind 0 Saturation,
/// 1 Contrast, 2 Gamma, 3 Gain, 4 Offset (FPostProcessSettings.h:196-215)
fn grade_slot(name: &str) -> Option<(usize, usize)> {
    let rest = name.strip_prefix("Color")?;
    let kinds = ["Saturation", "Contrast", "Gamma", "Gain", "Offset"];
    let (kind, tail) = kinds.iter().enumerate().find_map(|(i, k)| rest.strip_prefix(k).map(|t| (i, t)))?;
    let set = match tail {
        "" => 0,
        "Shadows" => 1,
        "Midtones" => 2,
        "Highlights" => 3,
        _ => return None,
    };
    Some((set, kind))
}

fn color4(v: &Value) -> Option<[f32; 4]> {
    let g = |k: &str| v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
    Some([g("R")?, g("G")?, g("B")?, g("A").unwrap_or(1.0)])
}

impl PostSettings {
    /// The level's unbound PostProcessVolume over the defaults: FSceneView::OverridePostProcessSettings (rva 0x33d4b60)
    /// copies a member only when its bOverride_ flag is set. A flag with no serialized value means the class default
    /// (tagged properties are written only when they differ from the CDO). The colour grading LUT is a blend:
    /// LerpTo(LUT, Weight x ColorGradingIntensity) over the neutral LUT (weight 1 after StartFinalPostprocessSettings),
    /// so with one volume of BlendWeight 1 the user LUT weighs ColorGradingIntensity (FLUTBlenderPS LUTWeights, see
    /// godot/components/ue/ue_post.gd 470-481).
    pub fn from_volume(pp: &Map<String, Value>) -> PostSettings {
        let mut s = PostSettings::default();
        let on = |k: &str| pp.get(&format!("bOverride_{k}")).and_then(|b| b.as_bool()).unwrap_or(false);
        let cdo = PostSettings::default();
        for (k, _) in pp.iter().filter(|(k, v)| k.starts_with("bOverride_") && v.as_bool() == Some(true)) {
            let name = &k["bOverride_".len()..];
            let f = pp.get(name).and_then(|x| x.as_f64()).map(|x| x as f32);
            match name {
                "WhiteTemp" => s.white_temp = f.unwrap_or(cdo.white_temp),
                "WhiteTint" => s.white_tint = f.unwrap_or(cdo.white_tint),
                "BlueCorrection" => s.blue_correction = f.unwrap_or(cdo.blue_correction),
                "ExpandGamut" => s.expand_gamut = f.unwrap_or(cdo.expand_gamut),
                "ToneCurveAmount" => s.tone_curve_amount = f.unwrap_or(cdo.tone_curve_amount),
                "FilmSlope" => s.film_slope = f.unwrap_or(cdo.film_slope),
                "FilmToe" => s.film_toe = f.unwrap_or(cdo.film_toe),
                "FilmShoulder" => s.film_shoulder = f.unwrap_or(cdo.film_shoulder),
                "FilmBlackClip" => s.film_black_clip = f.unwrap_or(cdo.film_black_clip),
                "FilmWhiteClip" => s.film_white_clip = f.unwrap_or(cdo.film_white_clip),
                "SceneColorTint" => s.scene_color_tint = pp.get(name).and_then(color4).unwrap_or(cdo.scene_color_tint),
                "SceneFringeIntensity" => s.scene_fringe_intensity = f.unwrap_or(cdo.scene_fringe_intensity),
                "ChromaticAberrationStartOffset" => s.ca_start_offset = f.unwrap_or(cdo.ca_start_offset),
                "BloomIntensity" => s.bloom_intensity = f.unwrap_or(cdo.bloom_intensity),
                "BloomThreshold" => s.bloom_threshold = f.unwrap_or(cdo.bloom_threshold),
                "BloomSizeScale" => s.bloom_size_scale = f.unwrap_or(cdo.bloom_size_scale),
                "AutoExposureMethod" => {
                    s.auto_exposure_method = match pp.get(name).and_then(|x| x.as_str()).unwrap_or("") {
                        "AEM_Histogram" => 0,
                        "AEM_Basic" => 1,
                        "AEM_Manual" => 2,
                        _ => 0, // the enum's first value: a missing serialized value is the CDO's (memset 0 = AEM_Histogram)
                    }
                }
                "AutoExposureBias" => s.auto_exposure_bias = f.unwrap_or(cdo.auto_exposure_bias),
                "AutoExposureMinBrightness" => s.auto_exposure_min_brightness = f.unwrap_or(cdo.auto_exposure_min_brightness),
                "AutoExposureMaxBrightness" => s.auto_exposure_max_brightness = f.unwrap_or(cdo.auto_exposure_max_brightness),
                "AutoExposureSpeedUp" => s.auto_exposure_speed_up = f.unwrap_or(cdo.auto_exposure_speed_up),
                "AutoExposureSpeedDown" => s.auto_exposure_speed_down = f.unwrap_or(cdo.auto_exposure_speed_down),
                "HistogramLogMin" => s.histogram_log_min = f.unwrap_or(cdo.histogram_log_min),
                "HistogramLogMax" => s.histogram_log_max = f.unwrap_or(cdo.histogram_log_max),
                "VignetteIntensity" => s.vignette_intensity = f.unwrap_or(cdo.vignette_intensity),
                "GrainJitter" => s.grain_jitter = f.unwrap_or(cdo.grain_jitter),
                "GrainIntensity" => s.grain_intensity = f.unwrap_or(cdo.grain_intensity),
                "AmbientOcclusionIntensity" => s.ambient_occlusion_intensity = f.unwrap_or(cdo.ambient_occlusion_intensity),
                "AmbientOcclusionRadius" => s.ambient_occlusion_radius = f.unwrap_or(cdo.ambient_occlusion_radius),
                "IndirectLightingIntensity" => s.indirect_lighting_intensity = f.unwrap_or(cdo.indirect_lighting_intensity),
                "ColorGradingIntensity" => s.color_grading_intensity = f.unwrap_or(1.0), // the CDO's 1 (ctor 0x1433c02cf)
                "ColorGradingLUT" => {
                    s.color_grading_lut = pp.get(name).and_then(|v| v.get("ObjectPath")).and_then(|p| p.as_str()).map(|p| crate::ue::strip(p).to_string())
                }
                "MotionBlurAmount" => s.motion_blur_amount = f.unwrap_or(cdo.motion_blur_amount),
                "MotionBlurMax" => s.motion_blur_max = f.unwrap_or(cdo.motion_blur_max),
                "LensFlareIntensity" => s.lens_flare_intensity = f.unwrap_or(1.0), // CDO 1 (0x1433c01da)
                "ColorCorrectionShadowsMax" => s.cc_shadows_max = f.unwrap_or(cdo.cc_shadows_max),
                "ColorCorrectionHighlightsMin" => s.cc_highlights_min = f.unwrap_or(cdo.cc_highlights_min),
                _ => {
                    // Color{Saturation,Contrast,Gamma,Gain,Offset}{,Shadows,Midtones,Highlights}: FVector4 {X,Y,Z,W}
                    if let Some((set, kind)) = grade_slot(name) {
                        let v = pp.get(name);
                        let g = |k: &str| v.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).map(|x| x as f32);
                        s.color_grade[set][kind] = match (g("X"), g("Y"), g("Z"), g("W")) {
                            (Some(x), Some(y), Some(z), Some(w)) => [x, y, z, w],
                            _ => cdo.color_grade[set][kind],
                        };
                    }
                }
            }
            // overrides the image pipeline does not draw yet (they still parse above when listed)
            if matches!(name, "MotionBlurAmount" | "MotionBlurMax" | "LensFlareIntensity" | "AmbientOcclusionIntensity"
                | "AmbientOcclusionRadius" | "IndirectLightingColor" | "IndirectLightingIntensity"
                | "ScreenSpaceReflectionIntensity" | "ScreenSpaceReflectionMaxRoughness" | "GrainJitter" | "GrainIntensity"
                | "WhiteTint")
                || (grade_slot(name).is_some() || name.starts_with("ColorCorrection")) && !color_correct_enabled()
            {
                s.not_ported.push(name.to_string());
            }
        }
        // the LUT only counts with its intensity (LerpTo weight); intensity alone keeps the neutral LUT
        if !on("ColorGradingLUT") {
            s.color_grading_intensity = 0.0;
        }
        s
    }

    /// The scene colour tint as FTonemapPS ColorScale0 (AddTonemapPass 0x21b3731: SceneColorTint as is)
    pub fn tint(&self) -> [f32; 4] {
        self.scene_color_tint
    }

    /// ChromaticAberrationParams (AddTonemapPass 0x21b31e0-0x21b3243): StartOffset = ChromaticAberrationStartOffset;
    /// Multiplier = 1 / (1 - StartOffset) unless StartOffset >= 0.9999 (then 1 and the scales 0);
    /// x = SceneFringeIntensity x 0.01 x 1.029 x Multiplier, y = SceneFringeIntensity x 0.01 x 0.5936 x Multiplier.
    pub fn ca_params(&self) -> [f32; 4] {
        let o = self.ca_start_offset;
        if o >= 0.9999 {
            return [0.0, 0.0, 0.0, 0.0];
        }
        let m = 1.0 / (1.0 - o);
        let i = self.scene_fringe_intensity * 0.01;
        [i * 1.029 * m, i * 0.5936 * m, o, 0.0]
    }

    /// FTonemapPS permutation dims (AddTonemapPass 0x21b389f-0x21b3938): vignette when VignetteIntensity > 0, fringe when
    /// SceneFringeIntensity > 0.01 and r.SceneColorFringeQuality > 0 (Epic 1), sharpen when r.Tonemapper.Sharpen > 0.
    pub fn fringe_on(&self) -> bool {
        self.scene_fringe_intensity > 0.01
    }
}

/// r.Tonemapper.Sharpen: DefaultEngine.ini:225 sets 1 (cvar default 0, CVarTonemapperSharpen init 0x6c8b90). The shader
/// gets clamp(v, 0, 10) / 6 (AddTonemapPass 0x21b31a9-0x21b31c4).
pub const TONEMAPPER_SHARPEN: f32 = 1.0;
pub fn sharpen_param() -> f32 {
    TONEMAPPER_SHARPEN.clamp(0.0, 10.0) * (1.0 / 6.0)
}

// ---------------------------------------------------------------- UMG BackgroundBlur (Slate)

/// Slate.BackgroundBlurDownsample = 1 (int at .data 0x1454dce3c, read by SBackgroundBlur::OnPaint 0x1dae75c),
/// Slate.BackgroundBlurMaxKernelSize = 255 (.data 0x1454dce38, OnPaint 0x1dae796), Slate.AllowBackgroundBlurWidgets = 1
/// (.data 0x1454dce34, OnPaint 0x1dae5cb)
pub const SLATE_BLUR_DOWNSAMPLE: bool = true;
pub const SLATE_BLUR_MAX_KERNEL: i32 = 255;

/// FMath::RoundToInt as the exe compiles it: cvtss2si(2x + 0.5) >> 1 (OnPaint 0x1dae6f9-0x1dae748)
fn round_to_int(x: f32) -> i32 {
    ((2.0 * x + 0.5).round_ties_even() as i32) >> 1
}

/// SBackgroundBlur::OnPaint (rva 0x1dae580) -> FSlateDrawElement::MakePostProcessPass params for a strength (already x
/// the tint alpha), the optional BlurRadius and the element's render-bounding size in pixels. Returns (KernelSize,
/// Strength = the Gaussian sigma in render-target texels, DownsampleAmount, render-target width, height), or None when
/// nothing is drawn (strength <= 0 or an empty target: 0x1dae63d, 0x1dae7e6-0x1dae7ec).
pub fn slate_blur_params(strength: f32, radius: Option<i32>, w_px: f32, h_px: f32) -> Option<(i32, f32, i32, i32, i32)> {
    if !(strength > 0.0) {
        return None;
    }
    let (mut rw, mut rh) = (round_to_int(w_px), round_to_int(h_px));
    // KernelSize = RoundToInt(Strength x 3) unless the radius is set (0x1dae6f9 x 3 @0x143fe4e10, 0x1dae750-0x1dae75a)
    let mut k = radius.unwrap_or(round_to_int(strength * 3.0));
    let mut down = 0;
    if SLATE_BLUR_DOWNSAMPLE && k > 9 {
        down = if k >= 64 { 4 } else { 2 }; // 0x1dae764-0x1dae77e
        k /= down;
    }
    // odd kernel, clamped to [3, MaxKernelSize] (0x1dae780-0x1dae79e)
    let k = if k & 1 != 0 { k } else { k + 1 };
    let k = if k < 3 { 3 } else { k.min(SLATE_BLUR_MAX_KERNEL) };
    // Strength = max(Strength, 0.5) / Downsample; target = DivideAndRoundUp(size, Downsample) (0x1dae7a1-0x1dae7d8)
    let mut s = strength.max(0.5);
    if down > 0 {
        rw = (rw - 1 + down) / down;
        rh = (rh - 1 + down) / down;
        s /= down as f32;
    }
    if rw <= 0 || rh <= 0 {
        return None;
    }
    Some((k, s, down, rw, rh))
}

/// GetWeightAndOffset (rva 0x142b86cc0): two neighbouring Gaussian taps folded into one bilinear tap.
/// w(x) = exp(-0.5 x^2 / sigma^2) / sqrt(2 pi sigma^2) (-0.5 @0x144022404, 2 pi @0x14402cb88); W = w(d) + w(d + 1),
/// offset = (w(d) d + w(d + 1)(d + 1)) / W, or d when W <= 0.
fn slate_weight_offset(d: f32, sigma: f32) -> (f32, f32) {
    let s2 = sigma * sigma;
    let k = 1.0 / (s2 * std::f32::consts::TAU).sqrt();
    let w0 = (d * d * (-0.5 / s2)).exp() * k;
    let w1 = ((d + 1.0) * (d + 1.0) * (-0.5 / s2)).exp() * k;
    let w = w1 + w0;
    let o = if w > 0.0 { (w1 * (d + 1.0) + w0 * d) / w } else { d };
    (w, o)
}

/// FSlatePostProcessor::BlurRect (rva 0x2b73230) weight table for FSlatePostProcessBlurPS: entry 0 = (centre weight
/// 1 / sqrt(2 pi sigma^2), -, pair at 1); then pairs at (3, 5), (7, 9), ... while < KernelSize (0x142b73308-0x142b733fd).
/// Returns the vec4 table and SampleCount = (KernelSize + 1) / 2 (the shader's cb0[63].x, [rbp+8] at 0x142b73d84).
pub fn slate_blur_weights(kernel: i32, sigma: f32) -> (Vec<[f32; 4]>, i32) {
    let centre = 1.0 / (sigma * sigma * std::f32::consts::TAU).sqrt();
    let (w1, o1) = slate_weight_offset(1.0, sigma);
    let mut out = vec![[centre, 0.0, w1, o1]];
    let mut b = 3;
    while b < kernel {
        let (wa, oa) = slate_weight_offset(b as f32, sigma);
        let (wb, ob) = slate_weight_offset((b + 2) as f32, sigma);
        out.push([wa, oa, wb, ob]);
        b += 4;
    }
    (out, (kernel + 1) / 2)
}

// ---------------------------------------------------------------- eye adaptation

/// FEyeAdaptationParameters as GetEyeAdaptationParameters (rva 0x21759a0) fills it for AEM_Basic / AEM_Histogram on
/// the non-extended luminance range (IsExtendDefaultLuminanceRangeEnabled 0x217e630 false: the cvar defaults to 0 and
/// no ini sets it). Output offsets: +0 LowPercent, +4 HighPercent, +8 MinAverageLuminance, +0xc MaxAverageLuminance,
/// +0x10 ExposureCompensationSettings, +0x14 ExposureCompensationCurve, +0x18 DeltaWorldTime, +0x1c SpeedUp, +0x20
/// SpeedDown, +0x24 HistogramScale, +0x28 HistogramBias, +0x2c LuminanceMin, +0x30 BlackHistogramBucketInfluence,
/// +0x34 GreyMult, +0x38 ExponentialUpM, +0x3c ExponentialDownM, +0x40 StartDistance, +0x44 LuminanceMax, +0x48
/// ForceTarget (stores 0x14217615f-0x142176208).
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct EyeParams {
    pub min_avg_lum: f32,
    pub max_avg_lum: f32,
    pub comp_settings: f32,
    pub comp_curve: f32,
    pub speed_up: f32,
    pub speed_down: f32,
    pub hist_scale: f32,
    pub hist_bias: f32,
    pub lum_min: f32,
    pub grey_mult: f32,
    pub exp_up_m: f32,
    pub exp_down_m: f32,
    pub start_distance: f32,
    pub lum_max: f32,
}

/// r.EyeAdaptation.ExponentialTransitionDistance default 1.5 (CVarEyeAdaptationExponentialTransitionDistance init 0x6c49b0)
pub const EA_TRANSITION_DISTANCE: f32 = 1.5;
/// r.EyeAdaptation.LensAttenuation default 0.78 (init 0x6c4a30); LuminanceMax = 0.78 / max(it, 0.01) only on the
/// extended range (LuminanceMaxFromLensAttenuation 0x217eab0), else 1
pub const EA_LUMINANCE_MAX: f32 = 1.0;

pub fn eye_params(s: &PostSettings) -> EyeParams {
    // Low/High percent clamped [1, 99] x 0.01 (0x142175a7a-0x142175ac0): only the histogram method uses them
    let method = s.auto_exposure_method;
    // MinAverageLuminance = 0.18 x min(MinB, MaxB), MaxAverageLuminance = 0.18 x MaxB (0x142176000 -> 0x142175e76,
    // x 0.18 at 0x14217611d / 0x14217612b)
    let (mut mn, mut mx) = (s.auto_exposure_min_brightness, s.auto_exposure_max_brightness);
    if method == 2 {
        // AEM_Manual (0x142175f08-0x142175fa9): not this game's path; the physical-camera exposure is not ported
        mn = 1.0;
        mx = 1.0;
    }
    let mn = mn.min(mx);
    // HistogramLogMin' = min(HistogramLogMax - 1, HistogramLogMin) (0x142175bb8-0x142175bc1); scale = 1 / (max - min'),
    // bias = -min' x scale (0x142175e84-0x142175ea4)
    let lmin = (s.histogram_log_max - 1.0).min(s.histogram_log_min);
    let scale = 1.0 / (s.histogram_log_max - lmin);
    // LuminanceMin = 0.0001 for AEM_Basic, 2^HistogramLogMin' otherwise (0x142175eac-0x142175fe5)
    let lum_min = if method == 1 { 0.0001 } else { lmin.exp2() };
    // ExponentialUp/DownM = (1/60) / ((1 - exp2(-Speed / 60)) x StartDistance / max(Speed, 0.001)) (0x142176066-0x142176119,
    // exp2 = the CRT import at 0x3fa89b8, doubles 1.0 @0x400b6f8 and 1/60 @0x448e368, -1/60 @0x45abe5c)
    let em = |sp: f32| -> f32 {
        let k = (EA_TRANSITION_DISTANCE / sp.max(0.001)) as f64;
        (1.0 / 60.0 / ((1.0 - (sp as f64 * (-1.0 / 60.0)).exp2()) * k)) as f32
    };
    EyeParams {
        min_avg_lum: 0.18 * mn,
        max_avg_lum: 0.18 * mx,
        // 2^AutoExposureBias (0x142175c2e; the -1 at 0x142175c1b is for mobile platforms without MobileHDR)
        comp_settings: s.auto_exposure_bias.exp2(),
        // AutoExposureBiasCurve unset -> exp2(0) (0x142175c8a-0x142175d9c)
        comp_curve: 1.0,
        speed_up: s.auto_exposure_speed_up,
        speed_down: s.auto_exposure_speed_down,
        hist_scale: scale,
        hist_bias: -lmin * scale,
        lum_min,
        // GreyMult 0.18 unless AEM_Manual (0x142175db7-0x142175dcd)
        grey_mult: if method == 2 { 1.0 } else { 0.18 },
        exp_up_m: em(s.auto_exposure_speed_up),
        exp_down_m: em(s.auto_exposure_speed_down),
        start_distance: EA_TRANSITION_DISTANCE,
        lum_max: EA_LUMINANCE_MAX,
    }
}

/// The steady state FBasicEyeAdaptationCS converges to for a measured average luminance (g117/01834 tail: exposure =
/// CompSettings x CompCurve x GreyMult / clamp(avg, Min, Max)). Used by tests and the evidence dump.
pub fn steady_exposure(e: &EyeParams, avg: f32) -> f32 {
    e.comp_settings * e.comp_curve * e.grey_mult / avg.clamp(e.min_avg_lum, e.max_avg_lum).max(0.0001)
}

// ---------------------------------------------------------------- bloom kernels

/// r.Filter.SizeScale @ Epic = 1 (DefaultScalability.ini PostProcessQuality@3); clamped [0.1, 10] in the kernel
pub const FILTER_SIZE_SCALE: f32 = 1.0;
/// AddGaussianBlurPass (0x21ad02c-0x21ad06b): 32 samples on SM5 (r.Filter.LoopMode cvar 0 -> not 128; feature level >= 3)
pub const MAX_FILTER_SAMPLES: u32 = 32;

/// `anonymous namespace'::Compute1DGaussianFilterKernel (rva 0x21baed0): (offset in texels, weight) pairs, weights
/// normalised to sum 1. Gaussian = expf(-16.7 x (|x| / ClampedRadius)^2) (const -16.7 @0x45b63c0, expf import
/// 0x3fa89e0), lerped to max(0, 1 - |x|) by CrossCenterWeight <= 1 or powf(max(0, 1 - |x|), Cross) above 1; samples
/// taken in pairs (i, i + 1) folded into one bilinear tap.
pub fn gauss_kernel(radius: f32, max_samples: u32, cross: f32) -> Vec<(f32, f32)> {
    let size_scale = if FILTER_SIZE_SCALE >= 0.1 { FILTER_SIZE_SCALE.min(10.0) } else { 0.1 };
    let maxr = (max_samples - 1) as f32;
    let clamped = if radius >= 1e-5 { maxr.min(radius) } else { 1e-5 };
    let scaled = size_scale * radius;
    let x = if scaled >= 1e-5 { maxr.min(scaled) } else { 1e-5 };
    // cvtss2si (round to nearest even) of (-0.5 - 2x), sar 1, neg (0x21baf65-0x21baf8c)
    let r = -(((-0.5 - 2.0 * x).round_ties_even() as i32) >> 1);
    let int_r = r.min(max_samples as i32 - 1);
    let w = |s: i32| -> f32 {
        let dx = (s as f32).abs();
        let lin = (1.0 - dx).max(0.0);
        if cross > 1.0 {
            lin.powf(cross)
        } else {
            let g = (-16.7f32 * (dx / clamped) * (dx / clamped)).exp();
            (lin - g) * cross + g
        }
    };
    let mut out = Vec::new();
    let mut sum = 0.0;
    let mut i = -int_r;
    while i <= int_r {
        let w0 = w(i);
        let w1 = if i != int_r { w(i + 1) } else { 0.0 };
        let ws = w0 + w1;
        out.push((i as f32 + w1 / ws, ws));
        sum += ws;
        i += 2;
    }
    for o in out.iter_mut() {
        o.1 /= sum;
    }
    out
}

// ---------------------------------------------------------------- CombineLUT (film curve + grading)
// Port of godot/components/ue/ue_post.gd (the reference implementation, which reads the game's FLUTBlenderPS
// state/shaders/_global/out/g131/01929_FLUTBlenderPS.asm; line numbers there). This version builds UE's own LUT: the
// 32^3 volume indexed by the log encoding the tonemapper uses (FLUTBlenderPS 01929 asm 1-24: cell uvw -> lin =
// 0.18 x 2^((uvw x 32/31 - 0.5/31 - 0.434018) x 14) - 0.002668; output x 0.952381 = 1/1.05, asm tail).

type V3 = [f32; 3];
type M3 = [V3; 3];

const SRGB_2_AP1: M3 = [[0.613191, 0.339512, 0.047366], [0.070207, 0.916336, 0.013450], [0.020619, 0.109567, 0.869607]];
const WIDE_2_AP1: M3 = [[1.370413, -0.329291, -0.063683], [-0.083434, 1.097091, -0.010862], [-0.025793, -0.098626, 1.203694]];
const BLUE_CORRECT: M3 = [[0.938639, 0.0, 0.061361], [0.0, 0.830794, 0.169206], [0.0, 0.0, 1.0]];
const BLUE_CORRECT_INV: M3 = [[1.065375, 0.000001, -0.065371], [0.0, 1.203663, -0.203668], [0.0, 0.0, 1.0]];
const AP1_2_AP0: M3 = [[0.695452, 0.140679, 0.163869], [0.044795, 0.859671, 0.095534], [-0.005526, 0.004025, 1.001501]];
const AP0_2_AP1: M3 = [[1.451439, -0.236511, -0.214929], [-0.076554, 1.176230, -0.099676], [0.008316, -0.006032, 0.997716]];
const AP1_2_SRGB: M3 = [[1.705052, -0.621791, -0.083258], [-0.130257, 1.140803, -0.010549], [-0.024003, -0.128969, 1.152972]];
const AP1_Y: V3 = [0.272229, 0.674082, 0.053690];
const BRADFORD: M3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
const SRGB_2_XYZ: M3 = [[0.412456, 0.357576, 0.180438], [0.212673, 0.715152, 0.072175], [0.019334, 0.119192, 0.950304]];
const XYZ_2_SRGB: M3 = [[3.240970, -1.537383, -0.498611], [-0.969244, 1.875968, 0.041555], [0.055630, -0.203977, 1.056972]];

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn mul(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
fn mm(a: &M3, b: &M3) -> M3 {
    let mut o = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            o[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    o
}
fn inv(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let k = 1.0 / det;
    [
        [(e * i - f * h) * k, (c * h - b * i) * k, (b * f - c * e) * k],
        [(f * g - d * i) * k, (a * i - c * g) * k, (c * d - a * f) * k],
        [(d * h - e * g) * k, (b * g - a * h) * k, (a * e - b * d) * k],
    ]
}
fn lerp3(a: V3, b: V3, t: f32) -> V3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
fn max0(v: V3) -> V3 {
    [v[0].max(0.0), v[1].max(0.0), v[2].max(0.0)]
}

/// WhiteBalance matrix (FLUTBlenderPS asm 113-201; ue_post.gd white_balance), tint 0 only (the isotherm offset is 0;
/// a WhiteTint override is listed in not_ported)
pub fn white_balance(temp: f32) -> M3 {
    let t = temp * 1.000556;
    let mut x = if temp <= 6996.10791 {
        0.244063 + (99.11 + (2967800.0 - 4607000064.0 / t) / t) / t
    } else {
        0.237040 + (247.48 + (1901800.0 - 2006400000.0 / t) / t) / t
    };
    let mut y = -3.0 * x * x + 2.87 * x - 0.275;
    if temp < 4000.0 {
        let u = (0.860117757 + 1.54118254e-4 * temp + 1.28641212e-7 * temp * temp) / (1.0 + 8.42420235e-4 * temp + 7.08145163e-7 * temp * temp);
        let v = (0.317398726 + 4.22806245e-5 * temp + 4.20481691e-8 * temp * temp) / (1.0 - 2.89741816e-5 * temp + 1.61456053e-7 * temp * temp);
        x = 3.0 * u / (2.0 * u - 8.0 * v + 4.0);
        y = 2.0 * v / (2.0 * u - 8.0 * v + 4.0);
    }
    let src = mul(&BRADFORD, [x / y, 1.0, (1.0 - x - y) / y]);
    let dst = [0.941379, 1.040436, 1.089767]; // Bradford x D65, asm 177-178
    let s = [dst[0] / src[0], dst[1] / src[1], dst[2] / src[2]];
    let scaled = [
        [BRADFORD[0][0] * s[0], BRADFORD[0][1] * s[0], BRADFORD[0][2] * s[0]],
        [BRADFORD[1][0] * s[1], BRADFORD[1][1] * s[1], BRADFORD[1][2] * s[1]],
        [BRADFORD[2][0] * s[2], BRADFORD[2][1] * s[2], BRADFORD[2][2] * s[2]],
    ];
    mm(&XYZ_2_SRGB, &mm(&mm(&inv(&BRADFORD), &scaled), &SRGB_2_XYZ))
}

/// ACES RRT parts + UE film curve, AP1 in -> AP1 out (asm 300-461; ue_post.gd film)
fn film(c1: V3, s: &PostSettings) -> V3 {
    let mut a0 = mul(&AP1_2_AP0, c1);
    let mx = a0[0].max(a0[1]).max(a0[2]);
    let mn = a0[0].min(a0[1]).min(a0[2]);
    let sat = (mx.max(0.0) - mn.max(0.0)) / mx.max(0.01);
    let chroma = (a0[2] * (a0[2] - a0[1]) + a0[1] * (a0[1] - a0[0]) + a0[0] * (a0[0] - a0[2])).max(0.0).sqrt();
    let yc = (a0[0] + a0[1] + a0[2] + 1.75 * chroma) / 3.0;
    let sx = (sat - 0.4) * 2.5;
    let st = (1.0 - sx.abs()).max(0.0);
    let sig = (1.0 + sx.signum() * if sx == 0.0 { 0.0 } else { 1.0 - st * st }) * 0.025;
    let glow = if yc * 3.0 <= 0.16 { sig } else if yc * 3.0 >= 0.48 { 0.0 } else { sig * (0.08 / yc - 0.5) };
    for v in a0.iter_mut() {
        *v *= 1.0 + glow;
    }
    let mut hue = 0.0f32;
    if !(a0[0] == a0[1] && a0[1] == a0[2]) {
        hue = (3f32.sqrt() * (a0[1] - a0[2])).atan2(2.0 * a0[0] - a0[1] - a0[2]).to_degrees();
        if hue < 0.0 {
            hue += 360.0;
        }
    }
    hue = hue.clamp(0.0, 360.0);
    if hue > 180.0 {
        hue -= 360.0;
    }
    let mut hw = (1.0 - (hue * 2.0 / 135.0).abs()).max(0.0);
    hw = hw * hw * (3.0 - 2.0 * hw);
    hw *= hw;
    a0[0] += hw * sat * (0.03 - a0[0]) * 0.18;
    let mut w = max0(mul(&AP0_2_AP1, a0));
    w = lerp3([dot(w, AP1_Y); 3], w, 0.96);
    let (slope, toe, shoulder, black, white) = (s.film_slope, s.film_toe, s.film_shoulder, s.film_black_clip, s.film_white_clip);
    let toe_scale = 1.0 + black - toe;
    let shoulder_scale = 1.0 + white - shoulder;
    let log_in = 0.18f32.log10();
    let toe_match = if toe > 0.8 {
        (1.0 - toe - 0.18) / slope + log_in
    } else {
        let bt = (0.18 + black) / toe_scale - 1.0;
        log_in - 0.5 * ((1.0 + bt) / (1.0 - bt)).ln() * (toe_scale / slope)
    };
    let straight_match = (1.0 - toe) / slope - toe_match;
    let shoulder_match = shoulder / slope - straight_match;
    let mut o = [0.0; 3];
    for i in 0..3 {
        let lc = w[i].max(1e-10).log10();
        let straight = slope * (lc + straight_match);
        let mut tc = -black + (2.0 * toe_scale) / (1.0 + ((-2.0 * slope / toe_scale) * (lc - toe_match)).exp());
        let mut sc = (1.0 + white) - (2.0 * shoulder_scale) / (1.0 + ((2.0 * slope / shoulder_scale) * (lc - shoulder_match)).exp());
        if lc >= toe_match {
            tc = straight;
        }
        if lc <= shoulder_match {
            sc = straight;
        }
        let mut t = ((lc - toe_match) / (shoulder_match - toe_match)).clamp(0.0, 1.0);
        if shoulder_match < toe_match {
            t = 1.0 - t;
        }
        t = (3.0 - 2.0 * t) * t * t;
        o[i] = tc + (sc - tc) * t;
    }
    o = lerp3([dot(o, AP1_Y); 3], o, 0.93);
    max0(o)
}

/// MH_UE_COLOR_CORRECT=1: apply the volume's colour-grading vectors (color_correct_all) in the CombineLUT
pub fn color_correct_enabled() -> bool {
    std::env::var("MH_UE_COLOR_CORRECT").map(|v| v == "1").unwrap_or(false)
}

/// One ColorCorrect set (01929_FLUTBlenderPS.asm 186-208 for Shadows; Midtones 246-268, Highlights 214-235 are the same
/// sequence): sat/contrast/gamma/gain/offset are Global x Set (cb0[45..48] x cb0[50..53] etc., offset = Global + Set,
/// cb0[49] + cb0[54]), each vector's xyz scaled by its w (offset: xyz + w).
fn color_correct(c: V3, luma: f32, s: &PostSettings, set: usize) -> V3 {
    let g = &s.color_grade[0];
    let k = &s.color_grade[set];
    let m = |i: usize| -> V3 {
        let v = [g[i][0] * k[i][0], g[i][1] * k[i][1], g[i][2] * k[i][2], g[i][3] * k[i][3]];
        [v[0] * v[3], v[1] * v[3], v[2] * v[3]]
    };
    let (sat, con, gam, gain) = (m(0), m(1), m(2), m(3));
    let off = [g[4][0] + k[4][0], g[4][1] + k[4][1], g[4][2] + k[4][2], g[4][3] + k[4][3]];
    let mut o = [0.0; 3];
    for i in 0..3 {
        // max(0, lerp(luma, c, sat)) x 5.555555, pow contrast, x 0.18, pow 1/gamma, x gain + (offset.xyz + offset.w)
        let w = (luma + sat[i] * (c[i] - luma)).max(0.0) * 5.555555;
        let w = w.powf(con[i]) * 0.18;
        let w = w.powf(1.0 / gam[i]);
        o[i] = w * gain[i] + (off[i] + off[3]);
    }
    o
}

/// ColorCorrectAll (01929 asm 185-269): the Shadows / Midtones / Highlights results weighted by luma (AP1 Y):
/// shadows = 1 - smoothstep(0, ShadowsMax, luma) (cb0[65].x, asm 209-213), highlights = smoothstep(HighlightsMin, 1,
/// luma) (cb0[65].y, asm 236-242), midtones = 1 - shadows - highlights (asm 265-266). At the defaults it is max(0, x).
pub fn color_correct_all(c: V3, s: &PostSettings) -> V3 {
    let luma = dot(c, AP1_Y);
    let t = (luma * (1.0 / s.cc_shadows_max)).clamp(0.0, 1.0);
    let ws = 1.0 - (3.0 - 2.0 * t) * t * t;
    let t = ((luma - s.cc_highlights_min) * (1.0 / (1.0 - s.cc_highlights_min))).clamp(0.0, 1.0);
    let wh = (3.0 - 2.0 * t) * t * t;
    let wm = 1.0 - ws - wh;
    let (cs, cm, ch) = (color_correct(c, luma, s, 1), color_correct(c, luma, s, 2), color_correct(c, luma, s, 3));
    [cs[0] * ws + cm[0] * wm + ch[0] * wh, cs[1] * ws + cm[1] * wm + ch[1] * wh, cs[2] * ws + cm[2] * wm + ch[2] * wh]
}

pub fn lin_to_srgb(x: f32) -> f32 {
    if x < 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
}
pub fn srgb_to_lin(x: f32) -> f32 {
    if x < 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}

/// CombineLUTs up to the user LUT: linear sRGB scene colour -> sRGB-encoded film colour (asm 113-469; ue_post.gd combine)
fn combine(lin: V3, s: &PostSettings, wb: &M3) -> V3 {
    let mut c = mul(&SRGB_2_AP1, mul(wb, lin));
    let y = dot(c, AP1_Y);
    if y > 0.0 {
        let ch = [c[0] / y - 1.0, c[1] / y - 1.0, c[2] / y - 1.0];
        let amt = (1.0 - (-4.0 * dot(ch, ch)).exp2()) * (1.0 - (-4.0 * s.expand_gamut * y * y).exp2());
        c = lerp3(c, mul(&WIDE_2_AP1, c), amt);
    }
    // ColorCorrectAll: ported (below) but opt-in until it is checked on screen (rust-render r1 paused by the
    // orchestrator's STAGES plan: no change to the default look). MH_UE_COLOR_CORRECT=1 enables it; off = the
    // defaults' result max(0, x), which ignores a volume's ColorSaturation / ColorContrast ... overrides.
    c = if color_correct_enabled() { color_correct_all(c, s) } else { max0(c) };
    c = lerp3(c, mul(&BLUE_CORRECT, c), s.blue_correction);
    c = lerp3(c, film(c, s), s.tone_curve_amount);
    c = lerp3(c, mul(&BLUE_CORRECT_INV, c), s.blue_correction);
    let f = mul(&AP1_2_SRGB, c);
    [lin_to_srgb(f[0].clamp(0.0, 1.0)), lin_to_srgb(f[1].clamp(0.0, 1.0)), lin_to_srgb(f[2].clamp(0.0, 1.0))]
}

/// A 256x16 RGBTable16x1 strip (raw texels: the LUT textures are SRGB false, e.g. RGBTable16x1_CastelloInterior
/// "SRGB": false, PF_B8G8R8A8), sampled like asm 470-479 (bilinear in r/g, lerp of two blue slices)
pub struct UserLut {
    pub w: usize,
    pub rgba8: Vec<u8>,
}

impl UserLut {
    fn px(&self, slice: usize, x: usize, y: usize) -> V3 {
        let o = (y * self.w + slice * 16 + x) * 4;
        [self.rgba8[o] as f32 / 255.0, self.rgba8[o + 1] as f32 / 255.0, self.rgba8[o + 2] as f32 / 255.0]
    }
    fn bilerp(&self, slice: usize, r: f32, g: f32) -> V3 {
        let r0 = (r.floor() as i32).clamp(0, 15) as usize;
        let r1 = (r0 + 1).min(15);
        let fr = r - r0 as f32;
        let g0 = (g.floor() as i32).clamp(0, 15) as usize;
        let g1 = (g0 + 1).min(15);
        let fg = g - g0 as f32;
        lerp3(lerp3(self.px(slice, r0, g0), self.px(slice, r1, g0), fr), lerp3(self.px(slice, r0, g1), self.px(slice, r1, g1), fr), fg)
    }
    pub fn sample(&self, c: V3) -> V3 {
        let b = c[2] * 15.0;
        let b0 = (b.floor() as i32).clamp(0, 15) as usize;
        let b1 = (b0 + 1).min(15);
        let fb = b - b0 as f32;
        lerp3(self.bilerp(b0, c[0] * 15.0, c[1] * 15.0), self.bilerp(b1, c[0] * 15.0, c[1] * 15.0), fb)
    }
}

/// UE's log LUT encoding of a linear colour (FTonemapPS 01206 asm: log2(x + 0.002668) x 0.071429 + 0.610727, saturate)
pub fn lut_encode(x: f32) -> f32 {
    ((x + 0.002668).log2() * 0.071429 + 0.610727).clamp(0.0, 1.0)
}
/// Its inverse for a LUT cell (FLUTBlenderPS 01929 asm 18-23)
pub fn lut_decode(e: f32) -> f32 {
    0.18 * ((e - 0.434018) * 14.0).exp2() - 0.002668
}

pub const LUT_SIZE: usize = 32;

/// The combined LUT (r, g, b fastest-first: index = (b x 32 + g) x 32 + r) as FLUTBlenderPS writes it: the display
/// value (sRGB-encoded film colour blended with the user LUT by ColorGradingIntensity) x 1/1.05.
pub fn combined_lut(s: &PostSettings, user: Option<&UserLut>) -> Vec<[f32; 3]> {
    let wb = white_balance(s.white_temp);
    let weight = if user.is_some() { s.color_grading_intensity } else { 0.0 };
    let n = LUT_SIZE;
    let xs: Vec<f32> = (0..n).map(|i| lut_decode(i as f32 / (n - 1) as f32)).collect();
    let mut out = Vec::with_capacity(n * n * n);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let f = combine([xs[r], xs[g], xs[b]], s, &wb);
                let d = match user {
                    Some(u) if weight > 0.0 => lerp3(f, u.sample(f), weight),
                    _ => f,
                };
                out.push([d[0] * 0.952381, d[1] * 0.952381, d[2] * 0.952381]);
            }
        }
    }
    out
}

/// The whole UE display transform for one linear scene colour after exposure (CPU reference of the GPU path; tests and
/// the per-stage evidence curves)
pub fn display(lin: V3, s: &PostSettings, user: Option<&UserLut>) -> V3 {
    let wb = white_balance(s.white_temp);
    let t = s.scene_color_tint;
    let f = combine([lin[0] * t[0], lin[1] * t[1], lin[2] * t[2]], s, &wb);
    match user {
        Some(u) if s.color_grading_intensity > 0.0 => lerp3(f, u.sample(f), s.color_grading_intensity),
        _ => f,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn arena() -> PostSettings {
        // DU_Arena's Arena sub-level PostProcessVolume_1.Settings (extract/json/.../Arena_Map/Arena.json)
        let pp = json!({"bOverride_WhiteTemp": true, "bOverride_SceneColorTint": true, "bOverride_SceneFringeIntensity": true,
            "bOverride_BloomIntensity": true, "bOverride_AutoExposureMethod": true, "bOverride_AutoExposureMinBrightness": true,
            "bOverride_AutoExposureMaxBrightness": true, "bOverride_ColorGradingIntensity": true, "bOverride_ColorGradingLUT": true,
            "AutoExposureMethod": "AEM_Basic", "SceneColorTint": {"R": 0.786458, "G": 0.74059, "B": 0.688151, "A": 1.0},
            "SceneFringeIntensity": 0.25, "ChromaticAberrationStartOffset": 0.15, "BloomIntensity": 1.25,
            "AutoExposureBias": 0.16992499, "AutoExposureMinBrightness": 0.8, "AutoExposureMaxBrightness": 1.5,
            "ColorGradingIntensity": 0.25,
            "ColorGradingLUT": {"ObjectPath": "Mordhau/Content/Mordhau/Maps/Castello/LUT/RGBTable16x1_CastelloInterior.0"}});
        PostSettings::from_volume(pp.as_object().unwrap())
    }

    #[test]
    fn volume_overrides_only_flagged_members() {
        let s = arena();
        assert_eq!(s.auto_exposure_method, 1);
        // AutoExposureBias is serialized but not overridden: the default (cvar r.DefaultFeature.AutoExposure.Bias = 0) holds
        assert_eq!(s.auto_exposure_bias, 0.0);
        // ChromaticAberrationStartOffset likewise (no bOverride_): default 0
        assert_eq!(s.ca_start_offset, 0.0);
        assert_eq!(s.white_temp, 6500.0);
        assert!((s.bloom_intensity - 1.25).abs() < 1e-6);
        assert!((s.color_grading_intensity - 0.25).abs() < 1e-6);
        assert_eq!(s.color_grading_lut.as_deref(), Some("Mordhau/Content/Mordhau/Maps/Castello/LUT/RGBTable16x1_CastelloInterior"));
        assert!((s.tint()[0] - 0.786458).abs() < 1e-6);
    }

    #[test]
    fn eye_adaptation_matches_exe_math() {
        let e = eye_params(&arena());
        assert!((e.min_avg_lum - 0.144).abs() < 1e-6 && (e.max_avg_lum - 0.27).abs() < 1e-6);
        assert!((e.hist_scale - 1.0 / 12.0).abs() < 1e-7 && (e.hist_bias - 8.0 / 12.0).abs() < 1e-6);
        assert_eq!(e.lum_min, 0.0001);
        // bright sunlit scene -> held at MaxAverageLuminance: 0.18 / 0.27
        assert!((steady_exposure(&e, 1.4) - 0.18 / 0.27).abs() < 1e-6);
        assert!((steady_exposure(&e, 0.05) - 0.18 / 0.144).abs() < 1e-6);
        // defaults: 0.03 .. 8
        let d = eye_params(&PostSettings::default());
        assert!((d.max_avg_lum - 1.44).abs() < 1e-6);
        assert!(e.exp_up_m > 0.0 && e.exp_down_m > 0.0);
    }

    #[test]
    fn gaussian_kernel_is_normalised_and_symmetric() {
        let k = gauss_kernel(25.6, MAX_FILTER_SAMPLES, 0.0);
        let s: f32 = k.iter().map(|x| x.1).sum();
        assert!((s - 1.0).abs() < 1e-5);
        let c: f32 = k.iter().map(|x| x.0 * x.1).sum();
        assert!(c.abs() < 0.05, "centroid {c}");
        // radius clamps to MaxSamples - 1
        let big = gauss_kernel(500.0, MAX_FILTER_SAMPLES, 0.0);
        assert!(big.len() <= 32);
    }

    #[test]
    fn lut_round_trip_and_film_grey() {
        for e in [0.1f32, 0.43, 0.61, 0.9] {
            assert!((lut_encode(lut_decode(e)) - e).abs() < 2e-4, "{e}");
        }
        // UE film: 18% grey stays near 18% of display range before sRGB encoding (InMatch = OutMatch = 0.18)
        let s = PostSettings::default();
        let g = display([0.18; 3], &s, None);
        let lin = srgb_to_lin(g[1]);
        assert!(lin > 0.1 && lin < 0.25, "grey -> {lin}");
        // black stays black, white compresses below 1
        assert!(display([0.0; 3], &s, None)[0] < 0.01);
        assert!(display([100.0; 3], &s, None)[0] <= 1.0);
    }

    #[test]
    fn color_correct_defaults_are_max0_and_contraband_grades() {
        let d = PostSettings::default();
        for c in [[0.0f32, 0.0, 0.0], [0.02, 0.05, 0.01], [0.18, 0.18, 0.18], [0.9, 0.4, 0.1], [4.0, 2.0, 1.0]] {
            let o = color_correct_all(c, &d);
            for i in 0..3 {
                assert!((o[i] - c[i].max(0.0)).abs() < 1e-4 * (1.0 + c[i]), "{c:?} -> {o:?}");
            }
        }
        // Contraband PPOutside: bOverride_ColorSaturation 0.95 x3, ColorContrast (1, 0.98475, 0.90796, 1.2)
        let pp = json!({"bOverride_ColorSaturation": true, "ColorSaturation": {"X": 0.95, "Y": 0.95, "Z": 0.95, "W": 1.0},
            "bOverride_ColorContrast": true, "ColorContrast": {"X": 1.0, "Y": 0.98475, "Z": 0.90796, "W": 1.2},
            "ColorGain": {"X": 0.7, "Y": 0.7, "Z": 0.7, "W": 1.0}});
        let s = PostSettings::from_volume(pp.as_object().unwrap());
        assert_eq!(s.color_grade[0][0], [0.95, 0.95, 0.95, 1.0]);
        assert_eq!(s.color_grade[0][1], [1.0, 0.98475, 0.90796, 1.2]);
        // ColorGain has no bOverride_: stays the default
        assert_eq!(s.color_grade[0][3], [1.0; 4]);
        // 18% grey is the contrast pivot: unchanged in luma
        let g = color_correct_all([0.18; 3], &s);
        assert!((g[0] - 0.18).abs() < 1e-3, "{g:?}");
        assert_eq!(grade_slot("ColorGainMidtones"), Some((2, 3)));
        assert_eq!(grade_slot("ColorGradingIntensity"), None);
    }

    #[test]
    fn slate_blur_matches_onpaint_and_blurrect() {
        // BlurStrength 10 (42 of the game's BackgroundBlur widgets): kernel round(30) = 30 > 9 -> downsample 2, 15 (odd),
        // sigma 10 / 2 = 5, a 1280 x 720 rect -> 640 x 360
        assert_eq!(slate_blur_params(10.0, None, 1280.0, 720.0), Some((15, 5.0, 2, 640, 360)));
        // BlurStrength 3: kernel 9, no downsample
        assert_eq!(slate_blur_params(3.0, None, 101.0, 50.0), Some((9, 3.0, 0, 101, 50)));
        // BlurStrength 20: kernel 60 -> /2 = 30 -> 31, sigma 10; 64+ -> downsample 4
        assert_eq!(slate_blur_params(20.0, None, 9.0, 9.0), Some((31, 10.0, 2, 5, 5)));
        assert_eq!(slate_blur_params(30.0, None, 9.0, 9.0).map(|p| (p.0, p.2)), Some((23, 4)));
        assert_eq!(slate_blur_params(0.0, None, 100.0, 100.0), None);
        // weight table: centre + pairs at 1, (3, 5), (7, 9), (11, 13): 4 vec4 for kernel 15, SampleCount 8
        let (w, n) = slate_blur_weights(15, 5.0);
        assert_eq!((w.len(), n), (4, 8));
        // the shader reads entries 1..n/2 (i = 2, 4, 6 -> i / 2): the table covers them
        assert!(w.len() >= (n as usize).div_ceil(2));
        // taps sum (centre + 2 x every pair) close to 1 for a kernel reaching ~3 sigma
        let s: f32 = w[0][0] + 2.0 * w[0][2] + w[1..].iter().map(|e| 2.0 * (e[0] + e[2])).sum::<f32>();
        assert!((s - 1.0).abs() < 0.02, "{s}");
        // pair offsets sit between their two taps
        assert!(w[0][3] > 1.0 && w[0][3] < 2.0 && w[1][1] > 3.0 && w[1][1] < 4.0);
    }

    #[test]
    fn ca_params_from_exe_constants() {
        let mut s = PostSettings::default();
        s.scene_fringe_intensity = 0.25;
        let p = s.ca_params();
        assert!((p[0] - 0.25 * 0.01 * 1.029).abs() < 1e-7 && (p[1] - 0.25 * 0.01 * 0.5936).abs() < 1e-7);
        assert!(s.fringe_on());
    }
}

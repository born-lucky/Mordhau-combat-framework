// uepost.wgsl - UE 4.26's post-process passes as Mordhau's shipped global shaders run them (rust-render r1). Each entry
// point is a line-by-line port of the game's own compiled shader (Engine/GlobalShaderCache-PCD3D_SM5.bin, fxc listings
// in state/shaders/_global/out/); the listing and its line roles are cited per function. CPU-side constants: uepost.rs.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct P { v: array<vec4<f32>, 72> }

@group(0) @binding(0) var ta: texture_2d<f32>;
@group(0) @binding(1) var tb: texture_2d<f32>;
@group(0) @binding(2) var tc: texture_2d<f32>;
@group(0) @binding(3) var lut: texture_3d<f32>;
@group(0) @binding(4) var smp: sampler;
@group(0) @binding(5) var<uniform> p: P;

// FDownsamplePS, High quality (g118/01842_FDownsamplePS.asm): four bilinear taps one input texel off the output pixel's
// centre (a 4x4 box), averaged; rgb max(0), alpha averaged. v[0].xy = input texel size.
@fragment
fn fs_downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let t = p.v[0].xy;
    let a = textureSampleLevel(ta, smp, in.uv - t, 0.0);
    let b = textureSampleLevel(ta, smp, in.uv + vec2<f32>(t.x, -t.y), 0.0);
    let c = textureSampleLevel(ta, smp, in.uv + vec2<f32>(-t.x, t.y), 0.0);
    let d = textureSampleLevel(ta, smp, in.uv + t, 0.0);
    let s = (a + b + c + d) * 0.25;
    return vec4<f32>(max(s.rgb, vec3<f32>(0.0)), s.a);
}

// FBasicEyeAdaptationSetupPS (g117/01836): L = dot(rgb, 1/3); alpha = HistogramScale x clamp(log2(max(L, LuminanceMin)),
// -10, 20) + HistogramBias. v[0] = (HistogramScale, HistogramBias, LuminanceMin, -).
@fragment
fn fs_ea_setup(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(ta, smp, in.uv, 0.0).rgb;
    let l = dot(c, vec3<f32>(0.333333));
    let e = clamp(log2(max(l, p.v[0].z)), -10.0, 20.0);
    return vec4<f32>(c, p.v[0].x * e + p.v[0].y);
}

// FBasicEyeAdaptationCS (g117/01834) as a one-pixel pass: the weighted mean of the 1/64 texture's alpha (weight =
// max(meter mask, 0.05); no AutoExposureMeterMask -> uniform), back to luminance, then the temporal step from the
// previous frame's exposure (tb, 1x1) and the clamp. Out = (exposure, exposure at target, average luminance,
// CompSettings x CompCurve).
// v[0] = (MinAverageLuminance, MaxAverageLuminance, CompSettings x CompCurve, GreyMult)
// v[1] = (DeltaWorldTime, SpeedUp, SpeedDown, ForceTarget)   v[2] = (HistScale, HistBias, ExponentialUpM, ExponentialDownM)
// v[3] = (StartDistance, OneOverPreExposure, -, -)
@fragment
fn fs_ea(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dim = textureDimensions(ta);
    var sw = 0.0;
    var sv = 0.0;
    for (var y = 0u; y < dim.y; y++) {
        for (var x = 0u; x < dim.x; x++) {
            let w = 1.0;  // max(white meter mask, 0.05)
            sw += w;
            sv += w * textureLoad(ta, vec2<u32>(x, y), 0).a;
        }
    }
    let avg = exp2((sv / max(sw, 1e-6) - p.v[2].y) / p.v[2].x);
    let avg_pre = avg * p.v[3].y;
    let k = p.v[0].z * p.v[0].w;                                  // r1.x = CompSettings x CompCurve x GreyMult
    let target_lum = clamp(avg_pre, p.v[0].x, p.v[0].y);          // r1.y
    var prev = textureLoad(tb, vec2<u32>(0u, 0u), 0).x;
    prev = select(1.0, prev, prev != 0.0);
    let prev_lum = k / prev;                                       // r1.z
    let lt = log2(target_lum);
    let lp = log2(prev_lum);
    let diff = lt - lp;                                            // r2.x
    let up = diff > 0.0;
    let speed = select(p.v[1].z, p.v[1].y, up);                    // r2.z
    let m = select(p.v[2].w, p.v[2].z, up);                        // r2.y
    let dt = p.v[1].x;
    let expo = (1.0 - exp2(-dt * speed)) * diff * m + lp;
    var lin = select(max(lp - dt * speed, lt), min(lp + dt * speed, lt), lp < lt);
    let r = select(expo, lin, p.v[3].x < abs(diff));
    var sm = exp2(r);
    sm = sm + p.v[1].w * (target_lum - sm);
    sm = clamp(sm, p.v[0].x, p.v[0].y);
    sm = max(sm, 0.0001);
    let tl = max(target_lum, 0.0001);
    return vec4<f32>(k / sm, k / tl, avg_pre, k / p.v[0].w);
}

// FFilterPS (g097/00893 additive / 00859-00863 plain): sum of N bilinear taps at uv + dir x offset_i, each x weight_i x
// tint, plus the additive input (the previous, smaller bloom stage, bilinear) at uv.
// v[0] = (dir.x, dir.y, N, has_additive)  v[1] = tint  v[2 + i] = (offset_i in texels, weight_i, -, -)
@fragment
fn fs_blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let n = u32(p.v[0].z);
    var s = vec4<f32>(0.0);
    for (var i = 0u; i < n; i++) {
        let k = p.v[2u + i];
        s += textureSampleLevel(ta, smp, in.uv + p.v[0].xy * k.x, 0.0) * (k.y * p.v[1]);
    }
    if (p.v[0].w > 0.5) {
        s += textureSampleLevel(tb, smp, in.uv, 0.0);
    }
    return s;
}

fn srgb_to_lin(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c < vec3<f32>(0.04045));
}

// FTonemapPS, the permutation Mordhau's settings select at Epic: bloom + vignette + sharpen + colour fringe, no grain
// intensity / jitter, SDR sRGB output (g102/01206_FTonemapPS.asm, VS g102/01197_FTonemapVS.asm). ta = scene colour,
// tb = bloom (half res), tc = eye adaptation (1x1, .x = exposure), lut = the CombineLUT volume (32^3).
// v[0] = ColorScale0 (SceneColorTint)  v[1] = (ColorScale1 = BloomIntensity, VignetteIntensity, Sharpen, OneOverPreExposure)
// v[2] = ChromaticAberrationParams (x, y scales, z StartOffset, w fringe on)  v[3] = (texel x, texel y, aspect H/W, frame noise)
// v[4] = (extent x, extent y, film on, -)
@fragment
fn fs_tonemap(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let pre = p.v[1].w;
    // grain noise for the quantization dither (01206 1-4: frac(sin(grainuv.y x 543.31 + grainuv.x) x 493013))
    let guv = uv + vec2<f32>(p.v[3].w, fract(p.v[3].w * 7.31));
    let noise = fract(sin(guv.y * 543.31 + guv.x) * 493013.0);
    // colour fringe (5-17): lens position in [-1, 1]; r and g read at LensUV - sign x saturate(|LensUV| - Start) x scale
    var c: vec3<f32>;
    let lens = uv * 2.0 - 1.0;
    if (p.v[2].w > 0.5) {
        let s = sign(lens);
        let o = clamp(abs(lens) - p.v[2].z, vec2<f32>(0.0), vec2<f32>(1.0));
        let ur = (lens - s * o * p.v[2].x) * 0.5 + 0.5;
        let ug = (lens - s * o * p.v[2].y) * 0.5 + 0.5;
        c = vec3<f32>(textureSampleLevel(ta, smp, ur, 0.0).r, textureSampleLevel(ta, smp, ug, 0.0).g, textureSampleLevel(ta, smp, uv, 0.0).b);
    } else {
        c = textureSampleLevel(ta, smp, uv, 0.0).rgb;
    }
    c *= pre;
    let exposure = textureLoad(tc, vec2<u32>(0u, 0u), 0).x;   // VS 01197: o1.x = EyeAdaptation[0].x
    // sharpen (18-53): 4 neighbours - two sampled across the 2x2 quad edge, two from the quad partner via derivatives
    let lw = vec3<f32>(0.3, 0.59, 0.11);
    let luma = dot(c, lw);
    let par = vec2<f32>(vec2<u32>(floor(uv * p.v[4].xy)) & vec2<u32>(1u)) * 2.0 - 1.0;
    let n1 = textureSampleLevel(ta, smp, uv + vec2<f32>(par.x * p.v[3].x, 0.0), 0.0).rgb * pre;
    let n2 = textureSampleLevel(ta, smp, uv + vec2<f32>(0.0, par.y * p.v[3].y), 0.0).rgb * pre;
    let n3 = c - dpdxFine(c) * par.x;
    let n4 = c - dpdyFine(c) * par.y;
    let l3 = luma - dpdxFine(luma) * par.x;
    let l4 = luma - dpdyFine(luma) * par.y;
    let d = vec4<f32>(luma) - vec4<f32>(dot(n1, lw), dot(n2, lw), l3, l4);
    let md = max(max(abs(d.x), abs(d.y)), max(abs(d.z), abs(d.w)));
    let k = -clamp(1.0 - exposure * md, 0.0, 1.0) * p.v[1].z;
    c = c + (n1 + n2 + n3 + n4 - 4.0 * c) * k;
    // bloom (54-63): bloom x (dirt mask x DirtTint + ColorScale1); no dirt mask -> x BloomIntensity
    let bloom = textureSampleLevel(tb, smp, uv, 0.0).rgb * pre;
    var lin = c * p.v[0].rgb + bloom * p.v[1].x;
    lin *= exposure;
    // vignette (64-69; VS: VignetteSpace = ScreenPos x (1, H/W) x sqrt(2) / sqrt(1 + (H/W)^2)): cos^4 law
    let asp = p.v[3].z;
    let vs = lens * vec2<f32>(1.0, asp) * (1.414214 / sqrt(1.0 + asp * asp)) * p.v[1].y;
    let cos4 = 1.0 / (dot(vs, vs) + 1.0);
    lin = lin * (cos4 * cos4);
    // LUT (70-73): log encode, 32^3 volume, x 1.05 (74), grain quantization dither (76-77)
    let e = clamp(log2(lin + vec3<f32>(0.002668)) * 0.071429 + vec3<f32>(0.610727), vec3<f32>(0.0), vec3<f32>(1.0));
    var g = textureSampleLevel(lut, smp, e * 0.96875 + vec3<f32>(0.015625), 0.0).rgb * 1.05;
    if (p.v[4].z < 0.5) {
        // debug (`post -film`): no CombineLUT, the exposed linear colour clamped and sRGB-encoded
        let l = clamp(lin, vec3<f32>(0.0), vec3<f32>(1.0));
        g = select(1.055 * pow(l, vec3<f32>(1.0 / 2.4)) - 0.055, l * 12.92, l < vec3<f32>(0.0031308));
    }
    let out = g + vec3<f32>(noise * 0.003906 - 0.001953);
    // UE writes the display value to a UNORM back buffer; Bevy's target encodes linear -> sRGB, so hand it linear
    return vec4<f32>(srgb_to_lin(clamp(out, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}

// ---------------------------------------------------------------- UMG BackgroundBlur (Slate post-process blur)
// Slate blurs the back buffer, i.e. the display-encoded image; here the view's main texture holds linear values that
// Bevy encodes on output, so the passes encode on read (fs_slate_down) and decode on write (fs_slate_comp).

fn lin_to_srgb3(c: vec3<f32>) -> vec3<f32> {
    let l = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(l, vec3<f32>(1.0 / 2.4)) - 0.055, l * 12.92, l < vec3<f32>(0.0031308));
}

// FSlatePostProcessDownsamplePS (g045/00337): four bilinear taps at uv -+ (x, y), uv + (x, -y), uv + (-x, y), each clamped
// to UVBounds, averaged. v[0].xy = tap offset (source texel size), v[1] = source uv rect (x0, y0, x1, y1) the output covers,
// v[2] = UVBounds (cb0[65]), v[3].x = 1: the 4-tap downsample, 0: one tap (no downsample: the region copied as is).
@fragment
fn fs_slate_down(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = mix(p.v[1].xy, p.v[1].zw, in.uv);
    let o = p.v[0].xy;
    let lo = p.v[2].xy;
    let hi = p.v[2].zw;
    if (p.v[3].x < 0.5) {
        return vec4<f32>(lin_to_srgb3(textureSampleLevel(ta, smp, clamp(uv, lo, hi), 0.0).rgb), 1.0);
    }
    let a = lin_to_srgb3(textureSampleLevel(ta, smp, clamp(uv - o, lo, hi), 0.0).rgb);
    let b = lin_to_srgb3(textureSampleLevel(ta, smp, clamp(uv + o * vec2<f32>(1.0, -1.0), lo, hi), 0.0).rgb);
    let c = lin_to_srgb3(textureSampleLevel(ta, smp, clamp(uv + o * vec2<f32>(-1.0, 1.0), lo, hi), 0.0).rgb);
    let d = lin_to_srgb3(textureSampleLevel(ta, smp, clamp(uv + o, lo, hi), 0.0).rgb);
    return vec4<f32>((a + b + c + d) * 0.25, 1.0);
}

// FSlatePostProcessBlurPS (g045/00336), line for line: v[0..62] = WeightAndOffsets (v[0].x the centre weight, v[0].zw the
// first (weight, offset) pair, then two pairs per vec4), v[63].x = SampleCount, v[64] = (direction.xy, inverse buffer
// size.zw), v[65] = UVBounds. Every tap is clamped to UVBounds.
@fragment
fn fs_slate_blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let lo = p.v[65].xy;
    let hi = p.v[65].zw;
    let dir = p.v[64].xy;
    let inv = p.v[64].zw;
    var s = textureSampleLevel(ta, smp, clamp(in.uv, lo, hi), 0.0).rgb * p.v[0].x;
    let o1 = dir * p.v[0].w * inv;
    s += textureSampleLevel(ta, smp, clamp(in.uv + o1, lo, hi), 0.0).rgb * p.v[0].z;
    s += textureSampleLevel(ta, smp, clamp(in.uv - o1, lo, hi), 0.0).rgb * p.v[0].z;
    let n = i32(p.v[63].x);
    for (var i = 2; i < n; i += 2) {
        let k = p.v[i / 2];
        let oa = dir * k.y * inv;
        let ob = dir * k.w * inv;
        s += (textureSampleLevel(ta, smp, clamp(in.uv + oa, lo, hi), 0.0).rgb + textureSampleLevel(ta, smp, clamp(in.uv - oa, lo, hi), 0.0).rgb) * k.x;
        s += (textureSampleLevel(ta, smp, clamp(in.uv + ob, lo, hi), 0.0).rgb + textureSampleLevel(ta, smp, clamp(in.uv - ob, lo, hi), 0.0).rgb) * k.z;
    }
    return vec4<f32>(s, 1.0);
}

// The blurred (display-encoded) region back into the view, drawn with the viewport on the widget's rect (Slate draws the
// post-process result as a quad over the element's geometry, bilinear-upsampled).
@fragment
fn fs_slate_comp(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = clamp(textureSampleLevel(ta, smp, in.uv, 0.0).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(srgb_to_lin(c), 1.0);
}

// Debug passes for the per-stage evidence (script verb `post`): plain copy of the scene x exposure, no tonemap.
@fragment
fn fs_copy(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(ta, smp, in.uv, 0.0).rgb * p.v[0].x, 1.0);
}

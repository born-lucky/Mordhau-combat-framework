// ue_tint.wgsl - WGSL port of the Godot ue_tint material shader (godot/game/render/ue_tint.gdshaderinc), the stand-in
// for Mordhau's cooked UE 4.26 materials. The math is read from Mordhau's own compiled D3D11 shaders (DXBC in the
// material packages, state/shaders/, written up in docs/SHADERS.md); every block cites its section / dump line range
// exactly as the GDScript shader does. mh_assets::shader is the CPU reference of the same code (unit-tested), and
// mh_assets::material resolves a material's uniforms (MaterialDesc) from the paks.
//
// Engine-neutral: this file declares the material bindings and `ue_tint_surface`, which returns the surface (base
// colour, roughness, metallic, specular, AO, tangent-space normal, subsurface, emissive, alpha); the host's PBR
// lighting consumes it (mh-runtime maps it to Bevy's PbrInput). `ue_lightmap` decodes UE's baked HQ lightmap
// (SHADERS.md 3, 4). Not ported here: the volumetric lightmap (ue_vlm.gdshaderinc) - UNCONFIRMED gap for R2.
//
// Bindings (group UE_TINT_GROUP = 2 below; a Bevy host substitutes its material bind group index):
//   0 UeTint uniform (every scalar/vector uniform of the GDScript shader, each a vec4<f32>; mh_assets::shader::UNIFORMS)
//   1 sampler: linear + mipmap linear + anisotropic, repeat (Godot filter_linear_mipmap_anisotropic, repeat_enable)
//   2 sampler: same, clamp-to-edge (repeat_disable: lightmap, sky occlusion)
//   3..17 t0..t14: texture slots, mapped per master mode from the GDScript uniform names (mh_assets::shader::TEX_SLOTS).
// Texture colour spaces follow the Godot hints: slots bound to a `source_color` uniform (albedo_*, *_albedo,
// detail_albedo, albedo_detail) are created with an sRGB view, all others linear (mh_assets::shader::gpu_textures).
// Vectors: n, t, b, v in WORLD space (Y up, metres; v = unit fragment -> camera, Godot VIEW); b = cross(n, t) * sign
// (Godot BINORMAL). Vertex colour = the mesh's FColor / 255 as stored (no sRGB decode, as UE's vertex factory).

// Every scalar / vector uniform of ue_tint.gdshaderinc, in declaration order, one vec4<f32> each (x = float / int /
// bool, xy / xyz / xyzw = vec2 / vec3 / vec4); mh_assets::shader::UNIFORMS is the same list with the defaults.
struct UeTint {
    ue_mode: vec4<f32>,
    use_lightmap: vec4<f32>,
    use_vlm: vec4<f32>,
    use_foliage: vec4<f32>,
    base_color: vec4<f32>,
    alpha_from_albedo: vec4<f32>,
    alpha_clip: vec4<f32>,
    alpha_value: vec4<f32>,
    normal_flip_y: vec4<f32>,
    use_normal: vec4<f32>,
    rough_sel: vec4<f32>,
    metal_sel: vec4<f32>,
    ao_sel: vec4<f32>,
    rough_add: vec4<f32>,
    metal_add: vec4<f32>,
    ao_add: vec4<f32>,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    color_c: vec4<f32>,
    metal_tweak: vec4<f32>,
    roughness_power: vec4<f32>,
    ao_weaken: vec4<f32>,
    weapon_blood_on: vec4<f32>,
    blood_stage1: vec4<f32>,
    blood_stage2: vec4<f32>,
    blood_mask_scalar: vec4<f32>,
    blood_normal: vec4<f32>,
    blood_metallic: vec4<f32>,
    blood_albedo_mix: vec4<f32>,
    blood_roughness: vec4<f32>,

    color_var: vec4<f32>,
    color_overlay: vec4<f32>,
    color_desat: vec4<f32>,
    ao_bias: vec4<f32>,
    metal_contrast: vec4<f32>,
    metal_color_lerp: vec4<f32>,
    metal_color_bias: vec4<f32>,
    rough_lerp_wood: vec4<f32>,
    rough_lerp_wood_bias: vec4<f32>,
    metal_roughness: vec4<f32>,
    layer_count: vec4<f32>,
    tiling: vec4<f32>,
    mul_1: vec4<f32>,
    mul_2: vec4<f32>,
    mul_3: vec4<f32>,
    mul_4: vec4<f32>,
    height_contrast: vec4<f32>,
    add: vec4<f32>,
    contrast: vec4<f32>,
    rough_mul: vec4<f32>,
    overlay: vec4<f32>,
    desaturation: vec4<f32>,
    bump: vec4<f32>,
    bump_offset_power: vec4<f32>,
    use_grunge: vec4<f32>,
    grunge_srgb: vec4<f32>,
    grunge_scale: vec4<f32>,
    contrast_b: vec4<f32>,
    grunge_color_b: vec4<f32>,
    overall_bias: vec4<f32>,
    metallic_stain: vec4<f32>,
    roughness_stain: vec4<f32>,
    spec_04: vec4<f32>,
    use_paint: vec4<f32>,
    paint_power: vec4<f32>,
    paint_contrast1: vec4<f32>,
    paint_contrast2: vec4<f32>,
    curvature_cc: vec4<f32>,
    mask_output_level: vec4<f32>,
    paint_roughness: vec4<f32>,
    dhi: vec4<f32>,
    color_inner: vec4<f32>,
    color_outer: vec4<f32>,
    color_curvaturer: vec4<f32>,
    paint_flatten_normal: vec4<f32>,
    rhao_detail_scale: vec4<f32>,
    albedo_detail_scale: vec4<f32>,
    a_desaturation: vec4<f32>,
    a_color: vec4<f32>,
    detail_power: vec4<f32>,
    desaturation_detail: vec4<f32>,
    albedo_color: vec4<f32>,
    overlay_bias: vec4<f32>,
    curvature_color: vec4<f32>,
    dirt_color: vec4<f32>,
    dirt_power: vec4<f32>,
    dirt_contrast: vec4<f32>,
    damage_desaturation: vec4<f32>,
    color_damage: vec4<f32>,
    roughness_bias: vec4<f32>,
    bias_normal: vec4<f32>,
    use_normal_detail: vec4<f32>,
    normal_detail_scale: vec4<f32>,
    bleed: vec4<f32>,
    emblem_color_a: vec4<f32>,
    emblem_color_b: vec4<f32>,
    has_secondary: vec4<f32>,
    secondary_color_a: vec4<f32>,
    secondary_color_b: vec4<f32>,
    secondary_color_c: vec4<f32>,
    primary_has_emblem: vec4<f32>,
    secondary_has_emblem: vec4<f32>,
    metal_roughness_mul: vec4<f32>,
    metal_roughness_add: vec4<f32>,
    metal_roughness_scale: vec4<f32>,
    albedo_scale: vec4<f32>,
    roughness_scale: vec4<f32>,
    sss: vec4<f32>,
    albedo_tint: vec4<f32>,
    mf_desaturation: vec4<f32>,
    hue_shift: vec4<f32>,
    mf_emissive: vec4<f32>,
    top_power: vec4<f32>,
    top_darken: vec4<f32>,
    mf_roughness: vec4<f32>,
    translucency_srgb: vec4<f32>,
    mf_sss: vec4<f32>,
    mf_specular: vec4<f32>,
    cloth_grunge_srgb: vec4<f32>,
    cloth_tile: vec4<f32>,
    cloth_desat: vec4<f32>,
    cloth_mul: vec4<f32>,
    cloth_grunge_uv1: vec4<f32>,
    cloth_grunge_scale: vec4<f32>,
    cloth_grunge_contrast: vec4<f32>,
    cloth_grunge_color: vec4<f32>,
    cloth_grunge_bias: vec4<f32>,
    cloth_nm_bias: vec4<f32>,
    banner_detail_scale: vec4<f32>,
    banner_mask_srgb: vec4<f32>,
    banner_scale_r: vec4<f32>,
    banner_contrast_r: vec4<f32>,
    banner_color_r: vec4<f32>,
    banner_scale_b: vec4<f32>,
    banner_contrast_b: vec4<f32>,
    banner_color_b: vec4<f32>,
    banner_bias: vec4<f32>,
    banner_sss: vec4<f32>,
    flora_tiling: vec4<f32>,
    flora_color: vec4<f32>,
    flora_color_lerp: vec4<f32>,
    flora_desat: vec4<f32>,
    flora_hsl: vec4<f32>,
    flora_mul: vec4<f32>,
    flora_emissive: vec4<f32>,
    flora_sub: vec4<f32>,
    flora_ss_bias: vec4<f32>,
    flora_metallic: vec4<f32>,
    flora_spec: vec4<f32>,
    flora_rough: vec4<f32>,
    flora_nm_flat: vec4<f32>,
    use_diffuse_tex: vec4<f32>,
    diffuse_const: vec4<f32>,
    metallic_const: vec4<f32>,
    specular_const: vec4<f32>,
    roughness_const: vec4<f32>,
    ao_const: vec4<f32>,
    saturation: vec4<f32>,
    bush_color: vec4<f32>,
    brightness: vec4<f32>,
    variation_srgb: vec4<f32>,
    variation_tiling: vec4<f32>,
    variation_intensity: vec4<f32>,
    ao_power: vec4<f32>,
    emissive: vec4<f32>,
    specular_value: vec4<f32>,
    lm_coord: vec4<f32>,
    lm_scale0: vec4<f32>,
    lm_add0: vec4<f32>,
    lm_scale1: vec4<f32>,
    lm_add1: vec4<f32>,
    lm_uv: vec4<f32>,
    use_sky_occlusion: vec4<f32>,
    mw_grunge: vec4<f32>,
    mw_scale: vec4<f32>,
    mw_contrast: vec4<f32>,
    mw_color_r: vec4<f32>,
    mw_color_g: vec4<f32>,
    mw_color_b: vec4<f32>,
    mw_overall_bias: vec4<f32>,
    mw_limit_metal: vec4<f32>,
    mw_detail: vec4<f32>,
    mw_detail_scale: vec4<f32>,
    mw_normal_flatten: vec4<f32>,
    sw_scale: vec4<f32>,
    sw_nm_flat: vec4<f32>,
    sw_detail: vec4<f32>,
    sw_desat: vec4<f32>,
    sw_color_mul: vec4<f32>,
    sw_color_lerp: vec4<f32>,
    sw_color_lerp_bias: vec4<f32>,
    sw_vc_color: vec4<f32>,
    sw_vc_bias: vec4<f32>,
    sw_rough_mod: vec4<f32>,
    sw_vertex_rough: vec4<f32>,
    sw_vertex_rough_bias: vec4<f32>,
    sw_ao_bias: vec4<f32>,
    sw_has_metal: vec4<f32>,
    trail_mesh_up: vec4<f32>,
    trail_mesh_right: vec4<f32>,
    trail_min: vec4<f32>,
    trail_motion: vec4<f32>,
    sb_normal_strength: vec4<f32>,
    sb_color: vec4<f32>,
    sb_color_lerp: vec4<f32>,
    sb_ao_in_bias: vec4<f32>,
    sb_roughness: vec4<f32>,
    sb_ao_bias: vec4<f32>,
}

const UE_TINT_GROUP: u32 = 2u;

@group(2) @binding(0) var<uniform> u: UeTint;
@group(2) @binding(1) var s_repeat: sampler;
@group(2) @binding(2) var s_clamp: sampler;
@group(2) @binding(3) var t0: texture_2d<f32>;
@group(2) @binding(4) var t1: texture_2d<f32>;
@group(2) @binding(5) var t2: texture_2d<f32>;
@group(2) @binding(6) var t3: texture_2d<f32>;
@group(2) @binding(7) var t4: texture_2d<f32>;
@group(2) @binding(8) var t5: texture_2d<f32>;
@group(2) @binding(9) var t6: texture_2d<f32>;
@group(2) @binding(10) var t7: texture_2d<f32>;
@group(2) @binding(11) var t8: texture_2d<f32>;
@group(2) @binding(12) var t9: texture_2d<f32>;
@group(2) @binding(13) var t10: texture_2d<f32>;
@group(2) @binding(14) var t11: texture_2d<f32>;
@group(2) @binding(15) var t12: texture_2d<f32>;
@group(2) @binding(16) var t13: texture_2d<f32>;
@group(2) @binding(17) var t14: texture_2d<f32>;

struct UeTintIn {
    uv: vec2<f32>,          // UE UV0 (Godot UV)
    uv2: vec2<f32>,         // UE UV1 (Godot UV2)
    custom0: vec2<f32>,     // UE UV2 (lightmap UV for LightMapCoordinateIndex 2)
    color: vec4<f32>,       // vertex colour
    world_pos: vec3<f32>,   // Y-up metres
    n: vec3<f32>,
    t: vec3<f32>,
    b: vec3<f32>,
    v: vec3<f32>,
}

struct UeTintOut {
    base: vec3<f32>,
    alpha: f32,
    roughness: f32,
    metallic: f32,
    specular: f32,
    ao: f32,
    // tangent-space normal, OpenGL Y+ (Godot NORMAL_MAP decode: xy * 2 - 1, z rebuilt); (0, 0, 1) when unused
    normal_ts: vec3<f32>,
    // transmitted light colour (MSM_TwoSidedFoliage; Godot BACKLIGHT), 0 when not foliage
    backlight: vec3<f32>,
    emissive: vec3<f32>,
    // alpha-test threshold for the masked render state (0 = no clip)
    alpha_clip: f32,
}

fn b1(x: vec4<f32>) -> bool { return x.x > 0.5; }
fn i1(x: vec4<f32>) -> i32 { return i32(round(x.x)); }

// sRGB -> linear, the decode the GPU applies to an sRGB texture fetch (IEC 61966-2-1 piecewise curve).
fn s2l(c: vec3<f32>) -> vec3<f32> {
    return select(c / 12.92, pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c >= vec3<f32>(0.04045));
}
// UE CheapContrast: saturate(lerp(-c, 1 + c, x))
fn cc(x: f32, c: f32) -> f32 { return clamp(mix(-c, 1.0 + c, x), 0.0, 1.0); }
fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.3, 0.59, 0.11)); }
fn step3(e: f32, x: vec3<f32>) -> vec3<f32> { return select(vec3<f32>(0.0), vec3<f32>(1.0), x >= vec3<f32>(e)); }
// Godot's normal_flip_y: UE maps are DirectX (Y-)
fn nmap(n: vec3<f32>) -> vec3<f32> {
    if (b1(u.normal_flip_y)) { return vec3<f32>(n.x, 1.0 - n.y, n.z); }
    return n;
}
// Tangent-space (UE, DirectX Y) normal from a BC5-style texture: xy*2-1, z = sqrt(max(1 - dot(xy, xy), 0)).
fn tsn(xy: vec2<f32>) -> vec3<f32> {
    let v = xy * 2.0 - 1.0;
    return vec3<f32>(v, sqrt(max(1.0 - dot(v, v), 0.0)));
}
// 4-step parallax (Ranked_Arena_Ground_InnerNew/r2/042 17-32, Arena_Blend_M_Walls_Red/r1/006 12-35):
// uv += Vts * (k * RHAO.g(uv) - 0.5 k), four times, k = BumpValue*2. One function per packed slot (WGSL textures are
// not first-class values in every backend's uniformity analysis).
fn pom2(uv0: vec2<f32>, vts: vec2<f32>, k: f32) -> vec2<f32> {
    var uv = uv0;
    for (var i = 0; i < 4; i++) { uv += vts * (k * textureSample(t2, s_repeat, uv).g - 0.5 * k); }
    return uv;
}
fn pom6(uv0: vec2<f32>, vts: vec2<f32>, k: f32) -> vec2<f32> {
    var uv = uv0;
    for (var i = 0; i < 4; i++) { uv += vts * (k * textureSample(t6, s_repeat, uv).g - 0.5 * k); }
    return uv;
}
fn pom9(uv0: vec2<f32>, vts: vec2<f32>, k: f32) -> vec2<f32> {
    var uv = uv0;
    for (var i = 0; i < 4; i++) { uv += vts * (k * textureSample(t9, s_repeat, uv).g - 0.5 * k); }
    return uv;
}
fn pom12(uv0: vec2<f32>, vts: vec2<f32>, k: f32) -> vec2<f32> {
    var uv = uv0;
    for (var i = 0; i < 4; i++) { uv += vts * (k * textureSample(t12, s_repeat, uv).g - 0.5 * k); }
    return uv;
}
// p from the paint mask m (SHADERS.md 6b lines 71-79, 6c 43-51).
fn paint_p(m: f32) -> f32 {
    return cc(select(0.0, min(pow(m, u.paint_power.x), 1.0), m > 0.0), u.paint_contrast1.x);
}
fn paint_colors(under: vec3<f32>, p: f32) -> vec3<f32> {
    let p2 = cc(p, u.paint_contrast2.x);
    var c = mix(under, u.color_inner.xyz, p);
    c = mix(c, u.color_outer.xyz, p2);
    return mix(c, u.color_curvaturer.xyz, cc(p2 - p, u.curvature_cc.x));
}
// Rodrigues rotation of c about the grey axis by th (M_FLORA 44-51, M_MEGAFOLIAGE 59-66)
fn hue_rot(c: vec3<f32>, th: f32) -> vec3<f32> {
    let K = vec3<f32>(0.57735);
    let pk = dot(K, c);
    let perp = c - pk * K;
    return perp * cos(th) + cross(K, perp) * sin(th) + pk * K;
}
// RNM blend (014 9-58, Cloth_M 10-32): t = N + (0,0,1); u = D * (-1,-1,1); r = t * dot(t, u) - t.z * u
fn rnm(nn: vec3<f32>, dn: vec3<f32>) -> vec3<f32> {
    let t = nn + vec3<f32>(0.0, 0.0, 1.0);
    let w = dn * vec3<f32>(-1.0, -1.0, 1.0);
    return t * dot(t, w) - t.z * w;
}

// M_WeaponMaster's World Position Offset (M_WeaponMaster/r0/112; mh_assets::weapon_trail): the offset, UE world
// cm, of a mesh-space (UE cm) position; zero unless the host fills the trail_* uniforms during a swing
fn ue_tint_trail_offset(p: vec3<f32>) -> vec3<f32> {
    let r = max(dot(p, u.trail_mesh_right.xyz) - u.trail_min.y, 0.0);
    let up = max(dot(p, u.trail_mesh_up.xyz) - u.trail_min.x, 0.0);
    let w = r * up;
    return select(vec3<f32>(0.0), u.trail_motion.xyz * w * u.trail_motion.w, w * u.trail_motion.w >= 1e-4);
}

fn ue_tint_surface(i: UeTintIn) -> UeTintOut {
    let mode = i1(u.ue_mode);
    let UV = i.uv;
    let COLOR = i.color;
    var alb = textureSample(t0, s_repeat, UV);
    var base = alb.rgb * u.base_color.xyz;
    var n = textureSample(t1, s_repeat, UV).rgb;
    var rough = 0.5;
    var metal = 0.0;
    var ao = 1.0;
    var spec = 0.5;
    var ss = vec3<f32>(0.0);
    var emis = vec3<f32>(0.0);
    var alpha_override = -1.0;

    if (mode == 1) {   // M_WEAPON: SHADERS.md 5 (M_WeaponMaster/r0/086 137-171, 092 187)
        let A = base;
        let m = textureSample(t3, s_repeat, UV).rgb;
        let rma = textureSample(t2, s_repeat, UV);
        let wa = 1.0 - (1.0 - (m.r + m.g)) * (1.0 - (m.r + m.g));
        base = mix(A, A * u.color_a.xyz, wa);
        base = mix(base, A * u.color_b.xyz, m.g);
        base = mix(base, A * u.color_c.xyz, m.b);
        metal = rma.g;
        rough = select(0.0, pow(rma.r, u.roughness_power.x), rma.r > 0.0);
        if (b1(u.weapon_blood_on)) {
            // PS086 104-115, 146-163, 191-200: preshaders V15/16 are one-hot,
            // not cumulative coverage. Mask alpha routes stage2 -> stage1.
            let b = textureSample(t4, s_repeat, UV);
            let stage = vec3<f32>(1.0, 2.0, 3.0);
            let w2 = vec3<f32>(1.0) - abs(sign(vec3<f32>(u.blood_stage2.x) - stage));
            let w1 = vec3<f32>(1.0) - abs(sign(vec3<f32>(u.blood_stage1.x) - stage));
            let mask = clamp(mix(dot(b.rgb, w2), dot(b.rgb, w1), b.a) * u.blood_mask_scalar.x, 0.0, 1.0);
            let grain = textureSample(t6, s_repeat, UV * 4.0).r;
            base = mix(base, A * u.blood_albedo_mix.x * vec3<f32>(0.25 * grain, 0.0, 0.0), mask);
            metal = mix(metal, u.blood_metallic.x, mask);
            rough = mix(rough, u.blood_roughness.x, mask);
            let bn = textureSample(t5, s_repeat, UV * 4.0).rg;
            n = normalize(mix(tsn(n.rg), tsn(bn), mask * u.blood_normal.x)) * 0.5 + 0.5;
        }
        // PS086 220-222 uses original RMA.g here, not blood-updated metallic.
        base = mix(base, clamp(base * u.metal_tweak.x, vec3<f32>(0.0), vec3<f32>(1.0)), rma.g);
        ao = clamp(rma.b + u.ao_weaken.x, 0.0, 1.0);
    } else if (mode == 2) {   // M_METALWOOD: SHADERS.md 5 (MetalWood_Master/r0/030 27-61)
        let rma = textureSample(t2, s_repeat, UV);
        let c = mix(base, u.color_var.xyz, textureSample(t3, s_repeat, UV).r);
        let o = mix(2.0 * c * u.color_overlay.xyz, 1.0 - 2.0 * (1.0 - c) * (1.0 - u.color_overlay.xyz), step3(0.5, c));
        var c2 = mix(c, o, 0.5);
        c2 = mix(c2, vec3<f32>(luma(c2)), u.color_desat.x);
        let c3 = mix(c2, c2 * rma.b, u.ao_bias.x);
        metal = cc(rma.g, u.metal_contrast.x);
        let c4 = mix(c, c3, 1.0 - metal);
        base = clamp(mix(mix(c4, u.metal_color_lerp.xyz, metal), c4, u.metal_color_bias.x), vec3<f32>(0.0), vec3<f32>(1.0));
        // UseGrunge (Barricade_Spikes_01/r0/030 95-124): per enabled channel k of GrungeTexture at UV x Scale_k,
        // g = CheapContrast(.k, Contrast_k), b = lerp(b, b x Color_k, g); then base = saturate(base + (b - base) x
        // OverallBias x saturate(1 + LimitGrungeToMetal x (metal - 1))). Channel B by symmetry (UNCONFIRMED: no
        // permutation with UseGrungeB read)
        if (u.mw_grunge.x + u.mw_grunge.y + u.mw_grunge.z > 0.0) {
            var gb = base;
            let gr = textureSample(t4, s_repeat, UV * u.mw_scale.x).r;
            gb = mix(gb, gb * u.mw_color_r.xyz, cc(gr, u.mw_contrast.x) * u.mw_grunge.x);
            let gg = textureSample(t4, s_repeat, UV * u.mw_scale.y).g;
            gb = mix(gb, gb * u.mw_color_g.xyz, cc(gg, u.mw_contrast.y) * u.mw_grunge.y);
            let gbl = textureSample(t4, s_repeat, UV * u.mw_scale.z).b;
            gb = mix(gb, gb * u.mw_color_b.xyz, cc(gbl, u.mw_contrast.z) * u.mw_grunge.z);
            let lim = clamp(1.0 + u.mw_limit_metal.x * (metal - 1.0), 0.0, 1.0);
            base = clamp(base + lim * (gb - base) * u.mw_overall_bias.x, vec3<f32>(0.0), vec3<f32>(1.0));
        }
        let r1 = rma.r + (1.0 - metal) * (u.rough_lerp_wood.x - rma.r) * u.rough_lerp_wood_bias.x;
        rough = clamp(mix(r1, mix(1.0, r1, 1.0 - metal), u.metal_roughness.x), 0.0, 1.0);
        ao = clamp(rma.b, 0.0, 1.0);
        // UseDetailNM? (030 13-37): RNM of Detail_NM at UV x Detail_Scale (lerped to flat by NormalFlatten) onto the
        // normal, back to the base normal by the metal mask
        let dnt = textureSample(t5, s_repeat, UV * u.mw_detail_scale.x).rg;
        if (b1(u.mw_detail)) {
            let nn = tsn(n.rg);
            let dn = mix(tsn(dnt), vec3<f32>(0.0, 0.0, 1.0), u.mw_normal_flatten.x);
            let r = mix(rnm(nn, dn), nn, metal);
            n = normalize(r) * 0.5 + 0.5;
        }
    } else if (mode == 3) {   // M_LAYERED: SHADERS.md 6a (Ranked_Arena_Ground_InnerNew/r2/042)
        let vts = vec2<f32>(dot(i.t, i.v), -dot(i.b, i.v));
        let lc = i1(u.layer_count);
        let u1 = pom2(UV * u.tiling.x, vts, u.bump.x * 2.0);
        alb = textureSample(t0, s_repeat, u1);
        let p1 = textureSample(t2, s_repeat, u1);
        base = alb.rgb * u.mul_1.xyz;
        n = textureSample(t1, s_repeat, u1).rgb;
        ao = p1.b;
        rough = p1.r * u.rough_mul.x;
        var hb = p1.g;
        var w4 = 0.0;
        // all layers sampled (uniform control flow), weights zero above layer_count
        let ua = pom6(UV * u.tiling.y, vts, u.bump.y * 2.0);
        let pa = textureSample(t6, s_repeat, ua);
        let wa = select(0.0, cc(clamp(2.0 * COLOR.r + u.add.y - cc(hb, u.height_contrast.x), 0.0, 1.0), u.contrast.y), lc >= 2);
        hb = mix(hb, cc(pa.g, u.height_contrast.y), wa);
        base = mix(base, textureSample(t4, s_repeat, ua).rgb * u.mul_2.xyz, wa);
        n = mix(n, textureSample(t5, s_repeat, ua).rgb, wa);
        ao = mix(ao, pa.b, wa);
        rough = mix(rough, pa.r * u.rough_mul.y, wa);
        let ub = pom9(UV * u.tiling.z, vts, u.bump.z * 2.0);
        let pb = textureSample(t9, s_repeat, ub);
        let wb = select(0.0, cc(clamp(2.0 * COLOR.g + u.add.z - hb, 0.0, 1.0), u.contrast.z), lc >= 3);
        hb = mix(hb, cc(pb.g, u.height_contrast.z), wb);
        base = mix(base, textureSample(t7, s_repeat, ub).rgb * u.mul_3.xyz, wb);
        n = mix(n, textureSample(t8, s_repeat, ub).rgb, wb);
        ao = mix(ao, pb.b, wb);
        rough = mix(rough, pb.r * u.rough_mul.z, wb);
        let uc = pom12(UV * u.tiling.w, vts, u.bump.w * 2.0);
        let pc = textureSample(t12, s_repeat, uc);
        w4 = select(0.0, cc(clamp(2.0 * COLOR.b + u.add.w - hb, 0.0, 1.0), u.contrast.w), lc >= 4);
        base = mix(base, textureSample(t10, s_repeat, uc).rgb * u.mul_4.xyz, w4);
        n = mix(n, textureSample(t11, s_repeat, uc).rgb, w4);
        ao = mix(ao, pc.b, w4);
        rough = mix(rough, pc.r * u.rough_mul.w, w4);
        base *= u.overlay.xyz;
        base = mix(base, vec3<f32>(luma(base)), u.desaturation.x);
        base = mix(base, base * ao, u.ao_bias.x);
        let gt = textureSample(t3, s_repeat, UV * u.grunge_scale.x).rgb;
        if (b1(u.use_grunge)) {
            let g = cc(select(gt, s2l(gt), b1(u.grunge_srgb)).b, u.contrast_b.x);
            base = clamp(mix(base, base * u.grunge_color_b.xyz, g * u.overall_bias.x), vec3<f32>(0.0), vec3<f32>(1.0));
        }
        metal = clamp(mix(u.metallic_stain.x, 0.0, COLOR.a), 0.0, 1.0);
        rough = clamp(mix(u.roughness_stain.x, rough, COLOR.a), 0.0, 1.0);
        spec = clamp(mix(0.5, u.spec_04.x, w4), 0.0, 1.0);
        ao = clamp(ao, 0.0, 1.0);
    } else if (mode == 4) {   // M_ARENABLEND: SHADERS.md 6b (Arena_Blend_M_Walls_Red/r1/006 13-127)
        let vts = vec2<f32>(dot(i.t, i.v), -dot(i.b, i.v));
        // 006 13-21: k = BumpValue*2 * (1 - pow(max(|1 - max(dot(N_vertex, V), 0)|, 1e-4), BumpOffsetPower)).
        let ang = 1.0 - pow(max(abs(1.0 - max(dot(i.n, i.v), 0.0)), 0.0001), u.bump_offset_power.x);
        let u1 = pom2(UV * u.tiling.x, vts, u.bump.x * 2.0 * ang);
        let u2 = pom6(UV * u.tiling.y, vts, u.bump.y * 2.0 * ang);
        alb = textureSample(t0, s_repeat, u1);
        let p1 = textureSample(t2, s_repeat, u1);
        let p2 = textureSample(t6, s_repeat, u2);
        let w2 = select(0.0, cc(clamp(2.0 * COLOR.r + u.add.y - cc(p1.g, u.height_contrast.x), 0.0, 1.0), u.contrast.y), i1(u.layer_count) >= 2);
        var l2 = textureSample(t4, s_repeat, u2).rgb * u.mul_2.xyz;
        var pm = 0.0;
        var n2 = textureSample(t5, s_repeat, u2).rgb;
        if (b1(u.use_paint)) {
            let p = paint_p(p2.r * p2.b * mix(1.0, p2.g, u.dhi.x) * COLOR.g);
            pm = p * u.mask_output_level.x;
            l2 = paint_colors(l2, p);
            n2 = mix(n2, vec3<f32>(0.5, 0.5, 1.0), clamp(pm * u.paint_flatten_normal.x, 0.0, 1.0));
        }
        base = mix(alb.rgb * u.mul_1.xyz, l2, w2);
        base = clamp(mix(base * u.overlay.xyz, vec3<f32>(luma(base * u.overlay.xyz)), u.desaturation.x), vec3<f32>(0.0), vec3<f32>(1.0));
        rough = clamp(mix(p1.r * u.rough_mul.x, mix(p2.r, u.paint_roughness.x, pm), w2), 0.0, 1.0);
        n = mix(textureSample(t1, s_repeat, u1).rgb, n2, w2);
    } else if (mode == 5) {   // M_ARCH: SHADERS.md 6c (Architecture_RomanTrimKit_RedPaint/r2/014 10-140)
        let rh = textureSample(t2, s_repeat, UV);
        let d = textureSample(t4, s_repeat, UV * u.rhao_detail_scale.xy);
        let a = mix(alb.rgb, vec3<f32>(luma(alb.rgb)), u.a_desaturation.x) * u.a_color.xyz;
        var dd = pow(textureSample(t5, s_repeat, UV * u.albedo_detail_scale.xy).rgb, vec3<f32>(u.detail_power.x));
        dd = mix(dd, vec3<f32>(luma(dd)), u.desaturation_detail.x) * u.albedo_color.xyz;
        let ov = mix(2.0 * a * dd, 1.0 - 2.0 * (1.0 - a) * (1.0 - dd), step3(0.5, a));
        let c0 = mix(a, ov, u.overlay_bias.x);
        var c1 = mix(c0, u.curvature_color.xyz, rh.g);
        var pm = 0.0;
        if (b1(u.use_paint)) {
            let p = paint_p(d.g * d.g * mix(1.0, rh.r, u.dhi.x) * COLOR.b);
            pm = p * u.mask_output_level.x;
            c1 = paint_colors(c1, p);
        }
        var c = mix(c1, c0, rh.g);
        c = mix(c, u.dirt_color.xyz, clamp((rh.r - pm) * cc(1.0 - pow(rh.b, u.dirt_power.x), u.dirt_contrast.x), 0.0, 1.0));
        base = clamp(mix(c, mix(c, vec3<f32>(luma(c)), u.damage_desaturation.x) * u.color_damage.xyz, COLOR.g), vec3<f32>(0.0), vec3<f32>(1.0));
        rough = rh.r + u.roughness_bias.x * d.r;
        ao = d.b * mix(mix(1.0, rh.b, u.ao_bias.x), 1.0, u.bias_normal.x);
        rough = mix(rough, rough * u.paint_roughness.x, pm);
        ao = mix(ao, 1.0, pm);
        ao = clamp(mix(ao, d.b, COLOR.g), 0.0, 1.0);
        rough = clamp(rough, 0.0, 1.0);
        let ndt = textureSample(t6, s_repeat, UV * u.normal_detail_scale.xy).rg;
        if (b1(u.use_normal_detail)) {
            // 014 9-58: RNM of NormalDetail (lerped to flat by Bias_Normal) onto Normal; back to N by sqrt(RHAO.g -
            // RHAO.b + 1), by the paint, and to 2D - (0,0,1) by VC.g; normalized after.
            let nn = tsn(n.rg);
            let dn = mix(tsn(ndt), vec3<f32>(0.0, 0.0, 1.0), u.bias_normal.x);
            var r = rnm(nn, dn);
            let sq = rh.g - rh.b + 1.0;
            r = mix(r, nn, select(0.0, sqrt(max(sq, 0.0)), sq > 0.0));
            r = mix(r, nn, pm);
            r = mix(r, 2.0 * dn - vec3<f32>(0.0, 0.0, 1.0), COLOR.g);
            n = normalize(r) * 0.5 + 0.5;
        }
    } else if (mode == 6) {   // M_ROMANKITBG: SHADERS.md 6d (RomanKit_Background/r0/006 5-31)
        base = clamp(alb.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        rough = select(1.0, max(1.0 - pow(base.r, 2.5), 0.0), base.r > 0.0);
    } else if (mode == 7) {   // M_WEARABLE: SHADERS.md 7 (M_EquipmentMasterChestShoulder_Masked_Inst/r2/059 14-139)
        let sel = select(0.0, ceil(COLOR.r - 0.1), b1(u.has_secondary));   // 14-16
        let s2 = sel > 0.5;
        // both sets sampled, then selected (uniform control flow for the implicit-derivative samples)
        let alb2 = textureSample(t4, s_repeat, UV);
        let n2 = textureSample(t5, s_repeat, UV).rgb;
        let rma2 = textureSample(t6, s_repeat, UV);
        let m2 = textureSample(t7, s_repeat, UV).rgb;
        let m = select(textureSample(t3, s_repeat, UV).rgb, m2, s2);
        let rma = select(textureSample(t2, s_repeat, UV), rma2, s2);
        alb = select(alb, alb2, s2);
        n = select(n, n2, s2);
        let ca = select(u.color_a.xyz, u.secondary_color_a.xyz, s2);
        let cb = select(u.color_b.xyz, u.secondary_color_b.xyz, s2);
        let ccol = select(u.color_c.xyz, u.secondary_color_c.xyz, s2);
        let A = alb.rgb * u.base_color.xyz;
        let w = 1.0 - (1.0 - m) * (1.0 - u.bleed.x * m);             // 91-93
        var c = mix(A, A * ca, w.r);                                  // 94-95
        c = mix(c, A * cb, w.b);                                      // 96-99
        c = mix(c, A * ccol, w.g);                                    // 100-103
        let emb = textureSample(t8, s_repeat, i.uv2).rb;              // 106 (UE UV1)
        let e = clamp(1.0 - (1.0 - u.bleed.x * emb) * (1.0 - emb), vec2<f32>(0.0), vec2<f32>(1.0));
        let As = clamp(A, vec3<f32>(0.0), vec3<f32>(1.0));
        var ea = mix(A, As * u.emblem_color_a.xyz, e.x);              // 110-113
        ea = mix(ea, As * u.emblem_color_b.xyz, e.y);
        let k = pow(min(e.x + e.y, 1.0), 10.0);                       // 114-120
        base = mix(c, ea, mix(u.primary_has_emblem.x, u.secondary_has_emblem.x, sel) * k);   // 121-125
        metal = rma.g;                                                // 135
        rough = mix(rma.r, mix(rma.r, rma.r * u.metal_roughness_mul.x + u.metal_roughness_add.x, u.metal_roughness_scale.x), rma.g);
        ao = rma.b;
    } else if (mode == 8) {   // M_ROUNDBANNER: SHADERS.md 8 (M_CY_RoundBanner 17-84)
        let A = alb.rgb * u.albedo_scale.x;
        let cm = textureSample(t3, s_repeat, UV).rgb;
        let w = 1.0 - (1.0 - cm) * (1.0 - u.bleed.x * cm);
        var c = mix(A, A * u.color_a.xyz, w.r);
        c = mix(c, A * u.color_b.xyz, w.b);
        c *= 1.0 - w.g;
        let emb = textureSample(t8, s_repeat, i.uv2).rb;
        let e = clamp(1.0 - (1.0 - u.bleed.x * emb) * (1.0 - emb), vec2<f32>(0.0), vec2<f32>(1.0));
        let As = clamp(A, vec3<f32>(0.0), vec3<f32>(1.0));
        var ea = mix(A, As * u.emblem_color_a.xyz, e.x);
        ea = mix(ea, As * u.emblem_color_b.xyz, e.y);
        base = mix(c, ea, pow(min(e.x + e.y, 1.0), 10.0));
        let dv = clamp(dot(i.v, i.n), 0.0, 1.0);
        base *= 1.0 - 0.6 * dv + 0.8 * pow(1.0 - dv, 6.0);
        let rh = textureSample(t2, s_repeat, UV);
        rough = clamp(rh.r * u.roughness_scale.x, 0.0, 1.0);
        ao = clamp(rh.b, 0.0, 1.0);
        ss = clamp(base * u.sss.xyz, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if (mode == 9) {   // M_CLOTH: Cloth_M/r0/083, Tent_Cloth_Castello_Red_01/r0 (gdshaderinc cloth block)
        let u0 = UV * u.cloth_tile.x;
        let ud = UV * u.cloth_tile.y;
        var A = textureSample(t0, s_repeat, u0).rgb;
        A = mix(A, vec3<f32>(luma(A)), u.cloth_desat.x) * u.cloth_mul.xyz;
        let da = textureSample(t4, s_repeat, ud);
        var c = A * da.rgb;
        let gt = textureSample(t6, s_repeat, select(UV, i.uv2, b1(u.cloth_grunge_uv1)) * u.cloth_grunge_scale.x).rgb;
        let g = cc(select(gt, s2l(gt), b1(u.cloth_grunge_srgb)).r, u.cloth_grunge_contrast.x);
        c = mix(c, c * u.cloth_grunge_color.xyz, g * u.cloth_grunge_bias.x);
        let rh = textureSample(t2, s_repeat, u0);
        base = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
        rough = clamp(rh.r, 0.0, 1.0);
        ao = clamp(da.a * rh.b, 0.0, 1.0);
        let nn = tsn(textureSample(t1, s_repeat, u0).rg);
        let dn = mix(tsn(textureSample(t5, s_repeat, ud).rg), vec3<f32>(0.0, 0.0, 1.0), u.cloth_nm_bias.x);
        n = normalize(rnm(nn, dn)) * 0.5 + 0.5;
    } else if (mode == 10) {   // M_BANNER: BannerAtlasTweak_M (INST_BannerRed_WPO/r0 14-61)
        let A = textureSample(t4, s_repeat, UV * u.banner_detail_scale.xy).rgb;
        var bm = textureSample(t7, s_repeat, UV).rgb;
        bm = select(bm, s2l(bm), b1(u.banner_mask_srgb));
        let w = 1.0 - (1.0 - bm) * (1.0 - u.bleed.x * bm);
        var c = mix(A, A * u.color_a.xyz, w.r);
        c = mix(c, A * u.color_b.xyz, w.b);
        c *= 1.0 - w.g;
        var gr = textureSample(t6, s_repeat, UV * u.banner_scale_r.x).rgb;
        var gb = textureSample(t6, s_repeat, UV * u.banner_scale_b.x).rgb;
        gr = select(gr, s2l(gr), b1(u.cloth_grunge_srgb));
        gb = select(gb, s2l(gb), b1(u.cloth_grunge_srgb));
        var c1 = mix(c, c * u.banner_color_r.xyz, cc(gr.r, u.banner_contrast_r.x));
        c1 = mix(c1, c1 * u.banner_color_b.xyz, cc(gb.b, u.banner_contrast_b.x));
        base = clamp(mix(c, c1, u.banner_bias.x), vec3<f32>(0.0), vec3<f32>(1.0));
        ss = clamp(c * u.banner_sss.x, vec3<f32>(0.0), vec3<f32>(1.0));
        let rm = textureSample(t2, s_repeat, UV);
        rough = clamp(rm.r, 0.0, 1.0);
        ao = clamp(rm.b, 0.0, 1.0);
    } else if (mode == 11) {   // M_GRASSCAST: Grass_mat1_Castello/r0/009 30-32, 53, 72
        base = clamp(alb.rgb * vec3<f32>(0.212429, 0.244792, 0.060914), vec3<f32>(0.0), vec3<f32>(1.0));
        emis = alb.rgb * vec3<f32>(0.031864, 0.036719, 0.009137);
        rough = 0.7;
        spec = 0.01;
    } else if (mode == 12) {   // M_FERN: M_PLains_Fern01/r0/009 31-35, 55, 78
        base = clamp(alb.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        rough = clamp(alb.r * 0.6 + 0.4, 0.0, 1.0);
        ss = base;
    } else if (mode == 13) {   // M_FLORA: Flora_twoSides_ALT/r1/009 38-81
        let fu = UV * u.flora_tiling.x;
        let A = textureSample(t0, s_repeat, fu).rgb;
        var c = mix(A, u.flora_color.xyz, u.flora_color_lerp.x);
        c = mix(c, vec3<f32>(luma(c)), u.flora_desat.x);
        c = hue_rot(c, u.flora_hsl.x * 6.28319);
        c *= u.flora_mul.xyz;
        emis = c * u.flora_emissive.x;
        ss = clamp(c * u.flora_sub.xyz * u.flora_ss_bias.x, vec3<f32>(0.0), vec3<f32>(1.0));
        base = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
        metal = clamp(u.flora_metallic.x, 0.0, 1.0);
        spec = clamp(u.flora_spec.x, 0.0, 1.0);
        rough = clamp(u.flora_rough.x, 0.0, 1.0);
        n = mix(textureSample(t1, s_repeat, fu).rgb, vec3<f32>(0.5, 0.5, 1.0), u.flora_nm_flat.x);
    } else if (mode == 14) {   // M_DECALDIRT: Treasure_Decal_Ceramics/r0/009 8-14, 71-73
        base = vec3<f32>(0.0);
        rough = 0.5;
        alpha_override = clamp(textureSample(t3, s_repeat, UV).g, 0.0, 1.0);
    } else if (mode == 15) {   // M_HLOD: M_MordhauHLOD/r0/053 4-25
        base = clamp(select(u.diffuse_const.xyz, alb.rgb, b1(u.use_diffuse_tex)), vec3<f32>(0.0), vec3<f32>(1.0));
        metal = clamp(u.metallic_const.x, 0.0, 1.0);
        spec = clamp(u.specular_const.x, 0.0, 1.0);
        rough = clamp(u.roughness_const.x, 0.0, 1.0);
        ao = clamp(u.ao_const.x, 0.0, 1.0);
    } else if (mode == 16) {   // M_MEGAFOLIAGE: INST_SparseGrass_Arena/r2/009 41-79
        let A = alb.rgb;
        emis = A * u.mf_emissive.x;
        var c = A * u.albedo_tint.xyz;
        c = mix(c, vec3<f32>(luma(c)), u.mf_desaturation.x);
        c = hue_rot(c, u.hue_shift.x * 6.28319);
        let ray = -i.v;   // unit camera -> pixel, world (UE z = Y up)
        let tz = 1.0 + ray.y;
        let top = min(max(select(0.0, pow(tz, u.top_power.x), tz > 0.0), u.top_darken.x), 1.0);
        base = clamp(c * top, vec3<f32>(0.0), vec3<f32>(1.0));
        rough = clamp(textureSample(t2, s_repeat, UV).r * u.mf_roughness.x, 0.0, 1.0);
        let tl = textureSample(t4, s_repeat, UV).rgb;
        ss = clamp(select(tl, s2l(tl), b1(u.translucency_srgb)) * u.mf_sss.xyz, vec3<f32>(0.0), vec3<f32>(1.0));
        spec = u.mf_specular.x;
    } else if (mode == 17) {   // M_BUSH: M_Bush/r0/009 35-73
        let A = alb.rgb;
        var c = mix(A, vec3<f32>(luma(A)), u.saturation.x) * u.bush_color.xyz;
        let b = c * u.brightness.x;
        let wue = vec2<f32>(i.world_pos.x, i.world_pos.z) * 100.0;   // UE world X, Y in cm
        let vt = textureSample(t4, s_repeat, wue / u.variation_tiling.x).rgb;
        let v = min(max(1.0 - select(vt, s2l(vt), b1(u.variation_srgb)), vec3<f32>(u.variation_intensity.x)), vec3<f32>(1.0));
        c = mix(2.0 * b * v, 1.0 - 2.0 * (1.0 - b) * (1.0 - v), step3(0.5, b));
        let t = 1.0 - UV.y;
        c *= select(0.0, pow(t, u.ao_power.x), t > 0.0);
        emis = max(c * u.emissive.x, vec3<f32>(0.0));
        base = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
        ss = base;
        let tr = 1.0 - A.r;
        rough = select(0.0, min(pow(tr, u.roughness_power.x), 1.0), tr > 0.0);
        spec = clamp(u.specular_value.x, 0.0, 1.0);
    } else if (mode == 18) {   // M_SHIPSWOOD: Ships_Wood_Master (Plks_Nails_1to1/r0/119 9-90)
        let u0 = UV * u.sw_scale.x;
        alb = textureSample(t0, s_repeat, u0);
        let A = alb.rgb;
        var c = mix(A, vec3<f32>(luma(A)), u.sw_desat.x);
        let cm = c * u.sw_color_mul.xyz;
        c = mix(cm, u.sw_color_lerp.xyz, u.sw_color_lerp_bias.x);
        // UseGrunge R (the only channel in this permutation), at the raw UV x Scale_R
        let g = cc(textureSample(t4, s_repeat, UV * u.mw_scale.x).r, u.mw_contrast.x) * u.mw_grunge.x;
        c = c + u.mw_overall_bias.x * g * (c * u.mw_color_r.xyz - c);
        c = clamp(c + u.sw_vc_bias.x * (u.sw_vc_color.xyz - c) * COLOR.a, vec3<f32>(0.0), vec3<f32>(1.0));
        base = c;
        // HasMetal: metallic = the albedo alpha (saturated)
        metal = select(0.0, clamp(alb.a, 0.0, 1.0), b1(u.sw_has_metal));
        let ra = textureSample(t2, s_repeat, u0);
        var r = ra.r - u.sw_rough_mod.x * ra.r;
        rough = clamp(r + u.sw_vertex_rough_bias.x * (u.sw_vertex_rough.x - r) * COLOR.a, 0.0, 1.0);
        ao = clamp(ra.b + u.sw_ao_bias.x * (1.0 - ra.b), 0.0, 1.0);
        let nb = mix(tsn(textureSample(t1, s_repeat, u0).rg), vec3<f32>(0.0, 0.0, 1.0), u.sw_nm_flat.x);
        let dt = textureSample(t5, s_repeat, UV * u.sw_scale.y).rg;
        var nr = nb;
        if (b1(u.sw_detail)) {
            nr = rnm(nb, mix(tsn(dt), vec3<f32>(0.0, 0.0, 1.0), u.sw_nm_flat.y));
        }
        n = normalize(nr) * 0.5 + 0.5;
    } else if (mode == 20) {   // M_MAINMENUCIRCLE: M_MainMenuCircle/r0/002 75-93 (TBasePassPSFNoLightMapPolicy)
        // RadialGradientExponential: t = 1 - |UV - 0.5| * 6.666667 (radius 0.15), g = 1 - exp(-(t * 2.333)^2); the
        // gradient term is kept only where t >= 0 and |t| > 1e-5 (the `and` masks at 89-90); EmissiveColor =
        // Dark + g * (Light - Dark) (color_a = Light, color_b = Dark); BaseColor unconnected (0), opaque
        let t = 1.0 - length(UV - vec2<f32>(0.5)) * 6.666667;
        let e = exp((t * 2.333) * (t * 2.333));
        let g = select(0.0, 1.0 - 1.0 / e, t >= 0.0 && abs(t) > 0.00001);
        base = vec3<f32>(0.0);
        emis = u.color_b.xyz + g * (u.color_a.xyz - u.color_b.xyz);
    } else if (mode == 19) {   // M_SANDBAGS: SandBags_M/r0 (TBasePassPSTLightMapPolicyHQ) 9-58
        var c = mix(alb.rgb, u.sb_color.xyz, u.sb_color_lerp.x);
        let d = textureSample(t5, s_repeat, UV * 6.0).rgb;
        c = mix(2.0 * c * d, 1.0 - 2.0 * (1.0 - c) * (1.0 - d), step3(0.5, c));
        let ra = textureSample(t2, s_repeat, UV);
        c = mix(c, c * ra.b, u.sb_ao_in_bias.x);
        let g = cc(textureSample(t4, s_repeat, UV * u.mw_scale.x).r, u.mw_contrast.x) * u.mw_grunge.x;
        base = clamp(c + u.mw_overall_bias.x * g * (c * u.mw_color_r.xyz - c), vec3<f32>(0.0), vec3<f32>(1.0));
        rough = clamp(ra.r * u.sb_roughness.x, 0.0, 1.0);
        ao = clamp(ra.b + u.sb_ao_bias.x * (1.0 - ra.b), 0.0, 1.0);
        let nb = mix(tsn(n.rg), vec3<f32>(0.0, 0.0, 1.0), 1.0 - u.sb_normal_strength.x);
        n = normalize(nb) * 0.5 + 0.5;
    } else {   // generic: packed channels by selector (UeMaterial.LAYOUTS, measured per texture)
        let packed = textureSample(t2, s_repeat, UV);
        let m = textureSample(t3, s_repeat, UV).rgb;
        base = mix(base, base * u.color_a.xyz, m.r);
        base = mix(base, base * u.color_b.xyz, m.g);
        base = mix(base, base * u.color_c.xyz, m.b);
        rough = clamp(dot(packed, u.rough_sel) + u.rough_add.x, 0.0, 1.0);
        metal = clamp(dot(packed, u.metal_sel) + u.metal_add.x, 0.0, 1.0);
        ao = clamp(dot(packed, u.ao_sel) + u.ao_add.x, 0.0, 1.0);
    }

    var o: UeTintOut;
    o.base = base;
    o.roughness = rough;
    o.metallic = metal;
    o.specular = spec;
    o.ao = ao;
    o.normal_ts = vec3<f32>(0.0, 0.0, 1.0);
    if (b1(u.use_normal)) {
        // Godot NORMAL_MAP: "The blue channel is ignored, as it's reconstructed" (spatial_shader.html)
        o.normal_ts = tsn(nmap(n).xy);
    }
    o.backlight = select(vec3<f32>(0.0), ss, b1(u.use_foliage));
    o.emissive = emis;
    o.alpha = select(u.alpha_value.x, alb.a, b1(u.alpha_from_albedo));
    if (alpha_override >= 0.0) { o.alpha = alpha_override; }
    o.alpha_clip = u.alpha_clip.x;
    return o;
}

struct UeLightmapOut {
    // diffuse light to add (UE adds baked lighting to scene colour), already x DiffuseColor x multi-bounce AO
    emissive: vec3<f32>,
    // AO with sky occlusion applied (SHADERS.md 4), = s.ao when there is none
    ao: f32,
}

// UE baked HQ lightmap + sky occlusion (SHADERS.md 3, 4; gdshaderinc use_lightmap block). `s` = ue_tint_surface's
// output, `nw` = world normal (Y up). Per-component coordinate scale/bias and scale/add from MapBuildData are the
// lm_* fields of UeTint (Godot instance uniforms).
fn ue_lightmap(i: UeTintIn, s: UeTintOut, nw: vec3<f32>) -> UeLightmapOut {
    let src = select(select(i.uv, i.uv2, u.lm_uv.x > 0.5), i.custom0, u.lm_uv.x > 1.5);
    let auv = src * u.lm_coord.xy + u.lm_coord.zw;
    let luv = auv * vec2<f32>(1.0, 0.5);
    let l0 = textureSample(t13, s_clamp, luv);
    let l1 = textureSample(t13, s_clamp, luv + vec2<f32>(0.0, 0.5));
    var log_l = l0.w + l1.w * (1.0 / 255.0) - (0.5 / 255.0);
    log_l = log_l * u.lm_scale0.w + u.lm_add0.w;
    let uvw = l0.rgb * l0.rgb * u.lm_scale0.rgb + u.lm_add0.rgb;
    let lum = exp2(log_l) - 0.01858136;
    let sh = l1 * u.lm_scale1 + u.lm_add1;
    let nue = vec3<f32>(nw.x, nw.z, nw.y);   // Y-up -> UE axes (SwapYZ)
    let dir = max(0.0, dot(sh, vec4<f32>(nue.yzx, 1.0)));
    let base = s.base;
    let diffuse = base * (1.0 - s.metallic);
    let a = s.ao;
    // UE AOMultiBounce
    let ao_mb = max(vec3<f32>(a), ((a * (2.0404 * base - 0.3324) + (-4.7951 * base + 0.6417)) * a + (2.7552 * base + 0.6903)) * a);
    var lit = uvw * lum * dir * diffuse;
    if (b1(u.use_foliage)) {
        lit += uvw * lum * max(0.0, dot(sh, vec4<f32>(-nue.yzx, 1.0))) * s.backlight;
    }
    var o: UeLightmapOut;
    o.emissive = lit * ao_mb;
    o.ao = a;
    let sk = textureSample(t14, s_clamp, auv);
    if (b1(u.use_sky_occlusion)) {
        let vis = sk.a * sk.a;
        let bent = normalize(sk.rgb * 2.0 - 1.0);
        let wv = 1.0 - (1.0 - vis) * (1.0 - vis);
        o.ao = a * vis * mix(clamp(dot(bent, nue), 0.0, 1.0), 1.0, wv);
    }
    return o;
}

// ---- volumetric lightmap (ue_vlm.gdshaderinc; mh_assets::vlm is the CPU reference and builds the data) ----
// Game shader: state/shaders/INST_SparseGrass_Arena/r0/001_TBasePassPSFPrecomputedVolumetricLightmapLightingPolicy-
// Skylight: 163-177 brick lookup, 201-254 SH3 irradiance, 326-330 x 1/pi x DiffuseColor [+ back x Subsurface]. View
// constants: SetupPrecomputedVolumetricLightmapUniformBufferParameters (.text 0x2288ae0, 0x2288e16 / 0x2288e4c /
// 0x2288ea3 / 0x2288eab-0x2288ebb). Textures (Vlm::uniforms / ambient_rgba16f / sh_stack): indirection RGBA8 unorm
// (read with textureLoad, x 255), ambient RGBA16F, the six SH layers (0R 1R 0G 1G 0B 1B) stacked along Z in one RGBA8
// unorm texture of depth 6 D (brick lookups stay inside one layer: uvw.z never leaves [0.5, D - 0.5] texels).
struct UeVlm {
    scale: vec4<f32>,     // 1 / (Max - Min)
    add: vec4<f32>,       // -Min * scale
    ind_size: vec4<f32>,  // IndirectionTextureDimensions
    brick: vec4<f32>,     // x = BrickSize, y = brick atlas depth D (one SH layer)
    texel: vec4<f32>,     // 1 / brick atlas size
}
@group(2) @binding(18) var vlm_ind: texture_3d<f32>;
@group(2) @binding(19) var vlm_amb: texture_3d<f32>;
@group(2) @binding(20) var vlm_sh: texture_3d<f32>;
@group(2) @binding(21) var<uniform> vlm: UeVlm;

fn ue_vlm_uv(p: vec3<f32>) -> vec3<f32> {
    let c = clamp(p * vlm.scale.xyz + vlm.add.xyz, vec3<f32>(0.0), vec3<f32>(0.99)) * vlm.ind_size.xyz;
    let e = floor(textureLoad(vlm_ind, vec3<i32>(c), 0) * 255.0 + 0.5);
    return (e.xyz * (vlm.brick.x + 1.0) + fract(c / e.w) * vlm.brick.x + 0.5) * vlm.texel.xyz;
}
fn vlm_sh_layer(uv: vec3<f32>, k: f32) -> vec4<f32> {
    return textureSampleLevel(vlm_sh, s_clamp, vec3<f32>(uv.xy, (uv.z + k) / 6.0), 0.0) * 2.0 - 1.0;
}
fn ue_vlm_channel(c0: vec4<f32>, c1: vec4<f32>, av: f32, n: vec3<f32>) -> f32 {
    let b1 = vec3<f32>(-1.023328 * n.y, 1.023328 * n.z, -1.023328 * n.x);
    let q = vec4<f32>(0.858085 * n.x * n.y, -0.858085 * n.y * n.z, 0.247708 * (3.0 * n.z * n.z - 1.0), -0.858085 * n.x * n.z);
    let q4 = vec4<f32>(c0.w * 3.872979, c1.x * 3.872979, c1.y * 4.472139, c1.z * 3.872979);
    return max(0.0, 0.886228 * av + dot(c0.xyz * 1.732051, b1) + dot(q4, q) + c1.w * 3.872979 * 0.429043 * (n.x * n.x - n.y * n.y));
}

// Indirect diffuse from the VLM (gdshaderinc use_vlm block): front irradiance x 1/pi x DiffuseColor x multi-bounce AO
// [+ back (-N) x subsurface for foliage]. For geometry without a baked lightmap; 0 when use_vlm is off or no volume.
fn ue_vlm(i: UeTintIn, s: UeTintOut, nw: vec3<f32>) -> vec3<f32> {
    if (!b1(u.use_vlm) || b1(u.use_lightmap) || vlm.scale.x == 0.0) { return vec3<f32>(0.0); }
    let w = normalize(nw);
    let vn = vec3<f32>(w.x, w.z, w.y);
    let vp = vec3<f32>(i.world_pos.x, i.world_pos.z, i.world_pos.y) * 100.0;
    let base = s.base;
    let vdiff = base * (1.0 - s.metallic);
    let a = s.ao;
    let vao = max(vec3<f32>(a), ((a * (2.0404 * base - 0.3324) + (-4.7951 * base + 0.6417)) * a + (2.7552 * base + 0.6903)) * a);
    let uv = ue_vlm_uv(vp);
    let av = textureSampleLevel(vlm_amb, s_clamp, uv, 0.0).rgb;
    let r0 = vlm_sh_layer(uv, 0.0) * av.r;
    let r1 = vlm_sh_layer(uv, 1.0) * av.r;
    let g0 = vlm_sh_layer(uv, 2.0) * av.g;
    let g1 = vlm_sh_layer(uv, 3.0) * av.g;
    let c0 = vlm_sh_layer(uv, 4.0) * av.b;
    let c1 = vlm_sh_layer(uv, 5.0) * av.b;
    let front = vec3<f32>(ue_vlm_channel(r0, r1, av.r, vn), ue_vlm_channel(g0, g1, av.g, vn), ue_vlm_channel(c0, c1, av.b, vn));
    var e = front * 0.31831 * vdiff * vao;
    if (b1(u.use_foliage)) {
        let back = vec3<f32>(ue_vlm_channel(r0, r1, av.r, -vn), ue_vlm_channel(g0, g1, av.g, -vn), ue_vlm_channel(c0, c1, av.b, -vn));
        e += back * 0.31831 * s.backlight;
    }
    return e;
}

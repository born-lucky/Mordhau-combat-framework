//! The particle materials' pixel math, read from their cooked D3D11 shaders (scripts/shaders/extract.sh dumps of the
//! inline shader maps into state/shaders/<material>/r0, annotate.py; the sprite permutations
//! `*__FParticleSpriteVertexFactory`): the masked ones' clip from `FDepthOnlyPS` (042), colour / opacity from the base
//! pass PS. The material graphs themselves are not cooked (editor-only), so these are the only source.
//!
//! Sprite inputs as UE's particle sprite vertex factory feeds them: uv / uv_b = the two SubUV cells, the SubUV lerp
//! (encoded in the vertex normal's x / z, see lib.rs), the particle colour.
//!
//! | mode | material | source | opacity | colour |
//! |---|---|---|---|---|
//! | 1 | M_impact_splash_subUV (masked, clip 0.5) | 042 FDepthOnlyPS, 026 base pass | clip lerp(T_blood_various.r(uv), .r(uv_b), lerp) * a - 0.5 | saturate(that mask * rgb) (lit: base colour) |
//! | 2 | M_BloodDrops (masked) | 042, 026 | m = (1 - exp(-(10 x)^2)) * a, x = 1 - abs(uv - 0.5) / Radius (0.3), 0 for x < 0 or abs(x) <= 1e-5; clip m + dither / 6 - 0.8333 | saturate((1 - exp(-(10 x)^2)) * rgb) |
//! | 3 | M_radial_ramp (translucent, unlit) | 010 base pass | g = max(1 - 2 abs(uv - 0.5), 0): saturate(g^4 * a), 0 where g <= 0 | particle rgb (emissive) |
//! | 4 | M_Dust_Particle (translucent) | 034 base pass | saturate(lerp(T_Dust_Particle_D.r) * a * pixel-size fade) | saturate(rgb) (lit) |
//! | 5 | M_FootstepDust (additive) | 028 base pass | saturate(tex.a * a * depth fade) | tex.rgb * rgb (lit) |
//!
//! Host gaps (UNCONFIRMED stand-ins): the lit materials (1, 2, 4, 5) are drawn unlit with their base colour (no
//! scene lighting in this plugin); M_Dust_Particle's pixel-size fade and M_FootstepDust's soft depth fade (0.02 x scene
//! depth difference) are 1; the BloodDrops dither uses frame 0 of the temporal pattern. M_ImpactDistortion is not
//! drawn: its base pass PS writes only the editor SelectionColor term with alpha 0 (o0.w = 0), the rest is the
//! FDistortionMeshPS refraction pass. Any other material: mode 0, the first non-normal texture x the particle colour
//! (UNCONFIRMED).

use bevy::asset::uuid_handle;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::{Shader, ShaderRef};
use bevy::mesh::MeshVertexBufferLayoutRef;

pub const SHADER: Handle<Shader> = uuid_handle!("3b8f6e2a-51c4-4d0e-9a7b-6c2d1e4f8a93");

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct FxParams {
    /// mode, Radius (BloodDrops), clip value, additive (1)
    pub p: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct ParticleMaterial {
    #[uniform(0)]
    pub params: FxParams,
    #[texture(1)]
    #[sampler(2)]
    pub tex: Handle<Image>,
    /// EBlendMode: 1 masked (discard, Opaque pipeline), 2 translucent, 3 additive
    pub blend: i32,
}

impl Material for ParticleMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER)
    }
    fn alpha_mode(&self) -> AlphaMode {
        match self.blend {
            1 => AlphaMode::Opaque,
            3 => AlphaMode::Add,
            _ => AlphaMode::Blend,
        }
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(_p: &MaterialPipeline, d: &mut RenderPipelineDescriptor, _l: &MeshVertexBufferLayoutRef, _k: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        d.primitive.cull_mode = None;
        Ok(())
    }
}

/// which shader-derived mode a material package uses, its texture slot (index into the material's
/// CachedExpressionData.ReferencedTextures, = the `Texture2D_k` parameter), and the material scalar Radius
pub fn mode_of(pkg: &str) -> Option<(u32, Option<usize>, f32)> {
    let name = pkg.rsplit('/').next().unwrap_or(pkg);
    Some(match name {
        // Texture2D_1 = T_blood_various (tex#1)
        "M_impact_splash_subUV" => (1, Some(1), 0.0),
        // S1 = Radius, default 0.3 (map.json scalar_parameters)
        "M_BloodDrops" => (2, None, 0.3),
        "M_radial_ramp" => (3, None, 0.0),
        "M_Dust_Particle" => (4, Some(0), 0.0),
        "M_FootstepDust" => (5, Some(0), 0.0),
        "M_ImpactDistortion" => return None,
        _ => (0, None, 0.0),
    })
}

pub const WGSL: &str = r#"
#import bevy_pbr::forward_io::{VertexOutput, FragmentOutput}
#import bevy_pbr::mesh_view_bindings::view
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping::tone_mapping
#endif

struct FxParams { p: vec4<f32>, }
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> fx: FxParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var samp: sampler;

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var uv = vec2<f32>(0.5);
    var uv2 = vec2<f32>(0.5);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
#ifdef VERTEX_UVS_B
    uv2 = in.uv_b;
#endif
    var c = vec4<f32>(1.0);
#ifdef VERTEX_COLORS
    c = in.color;
#endif
    // SubUV lerp: normal = normalize(lerp, 0, 1)
    let n = in.world_normal;
    let lerp_k = select(0.0, n.x / max(n.z, 1e-6), n.z > 0.0);
    let mode = u32(fx.p.x + 0.5);
    var rgb = c.rgb;
    var a = c.a;
    if (mode == 1u) {
        // M_impact_splash_subUV 042 / 026
        let m = mix(textureSample(tex, samp, uv).r, textureSample(tex, samp, uv2).r, lerp_k);
        if (m * c.a - fx.p.z < 0.0) { discard; }
        rgb = clamp(m * c.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        a = 1.0;
    } else if (mode == 2u) {
        // M_BloodDrops 042 / 026
        let x = 1.0 - length(uv - vec2<f32>(0.5)) / fx.p.y;
        let e = 1.0 - exp2(-(x * 10.0) * (x * 10.0) * 1.442695);
        let ok = x >= 0.0 && abs(x) > 1e-5;
        let m = select(0.0, e * c.a, ok);
        let pos = in.position.xy;
        let d = fract((pos.y * 2.0 + pos.x - 1.5) * 0.2) * 5.0 + fract(dot(vec2<f32>(2.408451, 3.253521), pos));
        if (m + d * 0.166667 - 0.8333 < 0.0) { discard; }
        rgb = select(vec3<f32>(0.0), clamp(e * c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), ok);
        a = 1.0;
    } else if (mode == 3u) {
        // M_radial_ramp 010
        let g = max(1.0 - 2.0 * length(uv - vec2<f32>(0.5)), 0.0);
        a = select(clamp(g * g * g * g * c.a, 0.0, 1.0), 0.0, g <= 0.0);
        rgb = c.rgb;
    } else if (mode == 4u) {
        // M_Dust_Particle 034
        let m = mix(textureSample(tex, samp, uv).r, textureSample(tex, samp, uv2).r, lerp_k);
        a = clamp(m * c.a, 0.0, 1.0);
        rgb = clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if (mode == 5u) {
        // M_FootstepDust 028
        let t = textureSample(tex, samp, uv) * c;
        a = clamp(t.a, 0.0, 1.0);
        rgb = clamp(t.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    } else {
        let t = mix(textureSample(tex, samp, uv), textureSample(tex, samp, uv2), lerp_k) * c;
        rgb = t.rgb;
        a = clamp(t.a, 0.0, 1.0);
    }
    var out: FragmentOutput;
    if (fx.p.w > 0.5) {
        // additive: premultiplied, alpha 0 (Bevy AlphaMode::Add pipeline)
        out.color = vec4<f32>(rgb * a, 0.0);
    } else {
        out.color = vec4<f32>(rgb, a);
    }
#ifdef TONEMAP_IN_SHADER
    out.color = tone_mapping(out.color, view.color_grading);
#endif
    return out;
}
"#;

pub fn install(shaders: &mut Assets<Shader>) {
    let _ = shaders.insert(SHADER.id(), Shader::from_wgsl(WGSL, "mh-fx/particle.wgsl"));
}

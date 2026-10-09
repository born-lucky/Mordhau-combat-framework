//! The game's own particle pixel shaders, run in Bevy. `scripts/shaders/particle_wgsl.py` translates each particle
//! material's cooked base-pass pixel shader (TBasePassPSFNoLightMapPolicy__FParticleSpriteVertexFactory, from the
//! inline shader maps dumped by scripts/shaders/extract.sh) instruction by instruction into a WGSL function `ue_ps`
//! (data_gen/shaders/particles/<master>.wgsl + .json, derived from the user's install, never committed). This module:
//! - evaluates the material's uniform expressions (preshaders) with the instance's parameter values into the Material
//!   constant buffer (a port of scripts/shaders/preshader.py; opcode set and layout in docs/SHADERS.md section 2);
//! - feeds UE's View constant buffer slots the shaders read, computed from the Bevy camera (`view_slots`);
//! - wraps `ue_ps` in a vertex / fragment pair with the sprite vertex factory's interpolants (`UE_PRELUDE`).
//!
//! View slots (FViewUniformShaderParameters, identified by how the shaders use them): 44..47 SVPositionToTranslatedWorld,
//! 65 InvDeviceZToWorldZTransform, 66 ScreenPositionScaleBias, 70 PreViewTranslation, 129 ViewRectMin, 130
//! ViewSizeAndInvSize, 135.y PreExposure (1), 136 / 137 / 138 / 139 Diffuse / Specular / Normal / Roughness override
//! (identity: (0,0,0,1), (0,0,0,1), (0,0,0,1), (0,1,0,0)), 140.x OutOfBoundsMask (0), 142 time (all components the
//! app time: which one is GameTime is UNCONFIRMED), 143.y MaterialTextureMipBias (0), 145.y UnlitViewmodeMask (0), 157
//! IndirectLightingColorScale (1). Translucency lighting volume (TranslucentBasePass 114.w = 0: the shaders take
//! their no-volume path; the ambient inner / outer volumes are 1x1x1 textures of `UeAmbient`) and scene depth (a 1x1
//! texture at device depth 0 = infinitely far: depth fades evaluate to 1) are stand-ins (UNCONFIRMED). Vertex fog = (0,
//! 0, 0, 1). GBuffer-writing (masked lit) shaders: colour = scene colour (o0) + base colour (o3) x ambient (UNCONFIRMED:
//! the deferred lighting is not ported).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef, VertexFormat};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::{Shader, ShaderRef};
use serde_json::Value;
use std::collections::HashMap;

pub const ATTR_SUBUV: MeshVertexAttribute = MeshVertexAttribute::new("UeSubUV", 0x5e01_0001, VertexFormat::Float32x4);
pub const ATTR_TW0: MeshVertexAttribute = MeshVertexAttribute::new("UeTangent0", 0x5e01_0002, VertexFormat::Float32x4);
pub const ATTR_TW2: MeshVertexAttribute = MeshVertexAttribute::new("UeTangent2", 0x5e01_0003, VertexFormat::Float32x4);
pub const ATTR_DYNP: MeshVertexAttribute = MeshVertexAttribute::new("UeDynamicParameter", 0x5e01_0004, VertexFormat::Float32x4);
pub const ATTR_PPOS: MeshVertexAttribute = MeshVertexAttribute::new("UeParticlePosition", 0x5e01_0005, VertexFormat::Float32x4);
pub const ATTR_PVEL: MeshVertexAttribute = MeshVertexAttribute::new("UeParticleVelocity", 0x5e01_0006, VertexFormat::Float32x4);

pub const VIEW_SLOTS: usize = 216;
pub const MAT_SLOTS: usize = 32;
pub const PASS_SLOTS: usize = 120;
pub const PRIM_SLOTS: usize = 24;

#[derive(ShaderType, Clone, Debug)]
pub struct UeUniforms {
    pub view: [Vec4; VIEW_SLOTS],
    pub mat: [Vec4; MAT_SLOTS],
    pub pass: [Vec4; PASS_SLOTS],
    pub prim: [Vec4; PRIM_SLOTS],
    /// x: output kind (0 scene colour o0, 1 GBuffer: o0 + o3 x ambient), y: modulate, yzw unused; ambient rgb in amb
    pub mode: Vec4,
    pub amb: Vec4,
}

impl Default for UeUniforms {
    fn default() -> Self {
        UeUniforms { view: [Vec4::ZERO; VIEW_SLOTS], mat: [Vec4::ZERO; MAT_SLOTS], pass: [Vec4::ZERO; PASS_SLOTS], prim: [Vec4::ZERO; PRIM_SLOTS], mode: Vec4::ZERO, amb: Vec4::ONE }
    }
}

/// EBlendMode of the material
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum UeBlend {
    #[default]
    Opaque,
    Masked,
    Translucent,
    Additive,
    Modulate,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UeKey {
    pub shader: Handle<Shader>,
    pub blend: UeBlend,
    pub two_sided: bool,
}

impl From<&UeParticleMaterial> for UeKey {
    fn from(m: &UeParticleMaterial) -> Self {
        UeKey { shader: m.shader.clone(), blend: m.blend, two_sided: true }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
#[bind_group_data(UeKey)]
pub struct UeParticleMaterial {
    #[uniform(0)]
    pub u: UeUniforms,
    #[texture(1)]
    #[sampler(2)]
    pub mtex_0: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub mtex_1: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub mtex_2: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    pub mtex_3: Handle<Image>,
    #[texture(9)]
    #[sampler(10)]
    pub mtex_4: Handle<Image>,
    #[texture(11)]
    #[sampler(12)]
    pub mtex_5: Handle<Image>,
    #[texture(13)]
    #[sampler(14)]
    pub scene_depth: Handle<Image>,
    #[texture(15)]
    #[sampler(16)]
    pub dummy2d: Handle<Image>,
    #[texture(17, dimension = "3d")]
    #[sampler(18)]
    pub vol3d_0: Handle<Image>,
    #[texture(19, dimension = "3d")]
    #[sampler(20)]
    pub vol3d_1: Handle<Image>,
    #[texture(21, dimension = "3d")]
    #[sampler(22)]
    pub vol3d_2: Handle<Image>,
    #[texture(23, dimension = "3d")]
    #[sampler(24)]
    pub dummy3d: Handle<Image>,
    pub shader: Handle<Shader>,
    pub blend: UeBlend,
}

impl Material for UeParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(VERTEX)
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(VERTEX)
    }
    fn alpha_mode(&self) -> AlphaMode {
        match self.blend {
            UeBlend::Opaque | UeBlend::Masked => AlphaMode::Opaque,
            UeBlend::Additive => AlphaMode::Add,
            UeBlend::Modulate => AlphaMode::Multiply,
            UeBlend::Translucent => AlphaMode::Blend,
        }
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(_p: &MaterialPipeline, d: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        let vl = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(2),
            ATTR_SUBUV.at_shader_location(3),
            ATTR_TW0.at_shader_location(4),
            ATTR_TW2.at_shader_location(5),
            ATTR_DYNP.at_shader_location(6),
            ATTR_PPOS.at_shader_location(7),
            ATTR_PVEL.at_shader_location(8),
        ])?;
        d.vertex.buffers = vec![vl];
        d.primitive.cull_mode = None;
        if let Some(f) = d.fragment.as_mut() {
            f.shader = key.bind_group_data.shader.clone();
            // UE's translucency blend states: translucent = (SrcAlpha, InvSrcAlpha) on colour (the shaders output
            // straight colour + opacity), additive (One, One),
            // modulate (DestColor, Zero) (UE 4.26 GetBlendStateForMaterial; UNCONFIRMED: not read from the exe)
            let bs = match key.bind_group_data.blend {
                UeBlend::Translucent => Some(BlendState::ALPHA_BLENDING),
                UeBlend::Additive => Some(BlendState {
                    color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                }),
                UeBlend::Modulate => Some(BlendState {
                    color: BlendComponent { src_factor: BlendFactor::Dst, dst_factor: BlendFactor::Zero, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                }),
                _ => None,
            };
            for t in f.targets.iter_mut().flatten() {
                t.blend = bs;
            }
        }
        Ok(())
    }
}

pub const VERTEX: Handle<Shader> = bevy::asset::uuid_handle!("4c1e9a52-7d3b-4e61-8f0a-2b9c5d7e1f36");

/// the interpolants + entry points around a generated `ue_ps`
pub const UE_PRELUDE: &str = r#"
#import bevy_pbr::mesh_view_bindings::view
//UE_BEVY_BEGIN
#import bevy_pbr::{mesh_view_bindings as vb, mesh_view_types, pbr_types, pbr_functions, mesh_types, view_transformations, shadows}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils
#endif

// the fragment's world position (Bevy, m) and frag coord, set before ue_ps runs
var<private> ue_wp: vec3<f32>;
var<private> ue_frag: vec4<f32>;

fn ue_setup(clip: vec4<f32>) {
    ue_frag = clip;
    ue_wp = view_transformations::position_ndc_to_world(view_transformations::frag_coord_to_ndc(clip));
}

// SceneDepthTexture: the device depth of the opaque scene (Bevy's depth prepass, reversed Z like UE's); no prepass =
// 0 (infinitely far). `uv` = buffer UV (the shaders apply ScreenPositionScaleBias first)
fn ue_scene_depth(uv: vec2<f32>) -> vec4<f32> {
#ifdef DEPTH_PREPASS
    let px = clamp(uv * view.viewport.zw, vec2<f32>(0.0), view.viewport.zw - vec2<f32>(1.0));
    return vec4<f32>(prepass_utils::prepass_depth(vec4<f32>(px, 0.0, 0.0), 0u));
#else
    return vec4<f32>(0.0);
#endif
}

// TranslucencyLightingVolumeAmbientInner / Outer: what FDeferredShadingSceneRenderer::InjectTranslucentVolumeLighting
// writes per light (the SH L0 band: light colour x 0.282095, unexposed: the shaders apply PreExposure), evaluated at
// the pixel for the scene's directional lights with their shadows
fn ue_vol_ambient_at(uvw: vec3<f32>) -> vec4<f32> {
    var c = vec3<f32>(0.0);
    let n = vb::lights.n_directional_lights;
    let view_z = view_transformations::position_world_to_view(ue_wp).z;
    for (var i: u32 = 0u; i < n; i = i + 1u) {
        let l = vb::lights.directional_lights[i];
        var sh = 1.0;
        if ((l.flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
            sh = shadows::fetch_directional_shadow(i, vec4<f32>(ue_wp, 1.0), l.direction_to_light, view_z, ue_frag.xy);
        }
        c += l.color.rgb * sh * 0.282095;
    }
    return vec4<f32>(c, 0.0) + vec4<f32>(uvw.x * 0.0);
}

// the translucent vertex fog (inscatter rgb, transmittance a): the scene's distance fog (Bevy DistanceFog, which
// bevy-runtime builds from the map's ExponentialHeightFog), inscatter unexposed
fn ue_fog() -> vec4<f32> {
#ifdef DISTANCE_FOG
    if (vb::fog.mode != mesh_view_types::FOG_MODE_OFF) {
        let b = pbr_functions::apply_fog(vb::fog, vec4<f32>(0.0, 0.0, 0.0, 1.0), ue_wp, view.world_position, ue_frag.xy);
        let w = pbr_functions::apply_fog(vb::fog, vec4<f32>(1.0, 1.0, 1.0, 1.0), ue_wp, view.world_position, ue_frag.xy);
        let t = clamp(dot(w.rgb - b.rgb, vec3<f32>(1.0 / 3.0)), 0.0, 1.0);
        return vec4<f32>(b.rgb / max(view.exposure, 1e-8), t);
    }
#endif
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

// deferred shading of a GBuffer-writing (masked lit) particle: GBufferA normal (o1), GBufferB metallic / specular /
// roughness / shading model (o2), GBufferC base colour (o3); scene colour o0 (emissive) added. Lit with the scene's
// lights by Bevy's PBR (UNCONFIRMED: Bevy's BRDF / light units for UE's deferred lighting pass)
fn ue_deferred(o: array<vec4<f32>, 8>) -> vec3<f32> {
    let sm = u32(round(o[2].w * 255.0)) & 15u;
    if (sm == 0u) {
        return o[0].rgb;
    }
    var p = pbr_types::pbr_input_new();
    p.material.base_color = vec4<f32>(o[3].rgb, 1.0);
    p.material.metallic = o[2].x;
    // UE F0 = 0.08 x Specular; Bevy F0 = 0.16 x reflectance^2
    p.material.reflectance = vec3<f32>(sqrt(max(o[2].y, 0.0) * 0.5));
    p.material.perceptual_roughness = clamp(o[2].z, 0.089, 1.0);
    p.material.emissive = vec4<f32>(0.0);
    let ne = o[1].xyz * 2.0 - vec3<f32>(1.0);
    // UE (x, y, z) -> Bevy (x, z, y)
    let n = normalize(vec3<f32>(ne.x, ne.z, ne.y));
    p.world_normal = n;
    p.N = n;
    p.world_position = vec4<f32>(ue_wp, 1.0);
    p.frag_coord = ue_frag;
    p.V = normalize(view.world_position - ue_wp);
    p.is_orthographic = view.clip_from_view[3].w == 1.0;
    p.flags = mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT;
    return pbr_functions::apply_pbr_lighting(p).rgb + o[0].rgb;
}
//UE_BEVY_END
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping::tone_mapping
#endif

struct UeU {
    viewcb: array<vec4<f32>, 216>,
    mat: array<vec4<f32>, 32>,
    passb: array<vec4<f32>, 120>,
    prim: array<vec4<f32>, 24>,
    mode: vec4<f32>,
    amb: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> u: UeU;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var mtex_0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var samp_mtex_0: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var mtex_1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var samp_mtex_1: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var mtex_2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var samp_mtex_2: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var mtex_3: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var samp_mtex_3: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var mtex_4: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var samp_mtex_4: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var mtex_5: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(12) var samp_mtex_5: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(13) var scene_depth: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(14) var samp_scene_depth: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(15) var dummy2d: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(16) var samp_dummy2d: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(17) var vol3d_0: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(18) var samp_vol3d_0: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(19) var vol3d_1: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(20) var samp_vol3d_1: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(21) var vol3d_2: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(22) var samp_vol3d_2: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(23) var dummy3d: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(24) var samp_dummy3d: sampler;

struct UeVsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) subuv: vec4<f32>,
    @location(4) tw0: vec4<f32>,
    @location(5) tw2: vec4<f32>,
    @location(6) dynp: vec4<f32>,
    @location(7) ppos: vec4<f32>,
    @location(8) pvel: vec4<f32>,
}

struct UeVsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) subuv: vec4<f32>,
    @location(3) tw0: vec4<f32>,
    @location(4) tw2: vec4<f32>,
    @location(5) @interpolate(flat) dynp: vec4<f32>,
    @location(6) ppos: vec4<f32>,
    @location(7) pvel: vec4<f32>,
    @location(8) clipw: f32,
}

struct UeInputs {
    tw0: vec4<f32>,
    tw2: vec4<f32>,
    dynp: vec4<f32>,
    color: vec4<f32>,
    uv: vec4<f32>,
    subuv: vec4<f32>,
    ppos: vec4<f32>,
    pvel: vec4<f32>,
    light_off: vec4<f32>,
    fog: vec4<f32>,
    svpos: vec4<f32>,
    front: vec4<f32>,
}

@vertex
fn vertex(i: UeVsIn) -> UeVsOut {
    var o: UeVsOut;
    o.clip = view.clip_from_world * vec4<f32>(i.pos, 1.0);
    o.uv = vec4<f32>(i.uv, 0.0, 0.0);
    o.color = i.color;
    o.subuv = i.subuv;
    o.tw0 = i.tw0;
    o.tw2 = i.tw2;
    o.dynp = i.dynp;
    o.ppos = i.ppos;
    o.pvel = i.pvel;
    o.clipw = o.clip.w;
    return o;
}

@fragment
fn fragment(i: UeVsOut, @builtin(front_facing) ff: bool) -> @location(0) vec4<f32> {
    var vin: UeInputs;
    vin.tw0 = i.tw0;
    vin.tw2 = i.tw2;
    vin.dynp = i.dynp;
    vin.color = i.color;
    vin.uv = i.uv;
    vin.subuv = i.subuv;
    vin.ppos = i.ppos;
    vin.pvel = i.pvel;
    vin.light_off = vec4<f32>(0.0);
    ue_setup(i.clip);
    vin.fog = ue_fog();
    // SvPosition: pixel xy, device z, w = the clip w in UE units (cm)
    vin.svpos = vec4<f32>(i.clip.xy, i.clip.z, i.clipw * 100.0);
    vin.front = vec4<f32>(select(0.0, 1.0, ff));
    // masked (GBuffer) materials: the opacity-mask clip of the depth-only pass (FDepthOnlyPS) first
    if (u.mode.x > 0.5) {
        _ = ue_depth_ps(vin);
    }
    let o = ue_ps(vin);
    var c = o[0];
    if (u.mode.x > 0.5) {
        c = vec4<f32>(ue_deferred(o), 1.0);
    }
#ifdef TONEMAP_IN_SHADER
    c = tone_mapping(c, view.color_grading);
#endif
    return c;
}
"#;

/// the shared vertex stage (the fragment entry of each material is in its own module built from the same prelude)
pub fn install(shaders: &mut Assets<Shader>) {
    let _ = shaders.insert(VERTEX.id(), Shader::from_wgsl(format!("{UE_PRELUDE}\nfn ue_ps(vin: UeInputs) -> array<vec4<f32>, 8> {{ var o: array<vec4<f32>, 8>; o[0] = vin.color; return o; }}\nfn ue_depth_ps(vin: UeInputs) -> array<vec4<f32>, 8> {{ var o: array<vec4<f32>, 8>; return o; }}\n"), "mh-fx/ue_vertex.wgsl"));
}

/// one translated material: its shader module and binding metadata (data_gen/shaders/particles/<master>.json)
#[derive(Clone, Debug)]
pub struct Translated {
    pub master: String,
    pub wgsl: String,
    pub meta: Value,
}

/// the generated shaders' directory: $MH_SHADER_DIR, else <repo>/data_gen/shaders/particles
pub fn shader_dir() -> std::path::PathBuf {
    std::env::var_os("MH_SHADER_DIR").map(Into::into).unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data_gen/shaders/particles"))
}

pub fn load_translated(master_pkg: &str) -> Option<Translated> {
    let name = master_pkg.rsplit('/').next()?;
    let d = shader_dir();
    let wgsl = std::fs::read_to_string(d.join(format!("{name}.wgsl"))).ok()?;
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(d.join(format!("{name}.json"))).ok()?).ok()?;
    Some(Translated { master: master_pkg.to_string(), wgsl: format!("{UE_PRELUDE}\n{}", patch(&wgsl)), meta })
}

/// the engine inputs the generated code samples as textures, routed to the prelude's functions: SceneDepthTexture
/// (res 36) -> ue_scene_depth, the translucency ambient volumes (res 56 / 58) -> ue_vol_ambient_at. The volumetric
/// fog volume (res 28) keeps its texture: TranslucentBasePass 114.w (ApplyVolumetricFog) is 0, so it is never sampled
/// (UNCONFIRMED: most maps enable volumetric fog; its integration is not ported)
pub fn patch(wgsl: &str) -> String {
    let mut s = wgsl.to_string();
    for (from, to) in [
        ("textureSampleLevel(scene_depth, samp_scene_depth, ", "ue_scene_depth("),
        ("textureSampleLevel(vol3d_1, samp_vol3d_1, ", "ue_vol_ambient_at("),
        ("textureSampleLevel(vol3d_2, samp_vol3d_2, ", "ue_vol_ambient_at("),
    ] {
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(k) = rest.find(from) {
            out.push_str(&rest[..k]);
            let tail = &rest[k + from.len()..];
            // the coordinate expression "vecN<f32>(...)", then ", <lod>)"
            let Some(e) = tail.find("), ") else { break };
            let after = &tail[e + 1..];
            let Some(close) = after.find(')') else { break };
            out.push_str(to);
            out.push_str(&tail[..e + 1]);
            out.push(')');
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        s = out;
    }
    s
}

/// stand-ins for the prelude's Bevy-dependent block (`//UE_BEVY_BEGIN` .. `//UE_BEVY_END`), to validate the generated
/// code with naga alone (tests/ue_shaders.rs)
pub const UE_STUBS: &str = r#"
var<private> ue_wp: vec3<f32>;
var<private> ue_frag: vec4<f32>;
fn ue_setup(clip: vec4<f32>) { ue_frag = clip; }
fn ue_scene_depth(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(uv.x * 0.0); }
fn ue_vol_ambient_at(uvw: vec3<f32>) -> vec4<f32> { return vec4<f32>(uvw.x * 0.0); }
fn ue_fog() -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
fn ue_deferred(o: array<vec4<f32>, 8>) -> vec3<f32> { return o[0].rgb; }
"#;

/// preshader evaluation (scripts/shaders/preshader.py port): VectorExpressions then ScalarExpressions packed 4 per
/// float4; scalars / vectors = the instance values (lower-case names), missing names use the master's defaults
pub fn material_cbuffer(meta: &Value, scalars: &HashMap<String, f32>, vectors: &HashMap<String, [f32; 4]>) -> Result<Vec<[f32; 4]>, String> {
    let hex = meta.get("preshader_data_hex").and_then(Value::as_str).unwrap_or("");
    let data: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0)).collect();
    let sp = meta.get("scalar_parameters").and_then(Value::as_array).cloned().unwrap_or_default();
    let vp = meta.get("vector_parameters").and_then(Value::as_array).cloned().unwrap_or_default();
    let run = |p: &Value| -> Result<[f32; 4], String> {
        let off = p.get("OpcodeOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let size = p.get("OpcodeSize").and_then(Value::as_u64).unwrap_or(0) as usize;
        preshader(&data, off, size, &sp, &vp, scalars, vectors)
    };
    let mut out = vec![];
    for p in meta.get("vector_preshaders").and_then(Value::as_array).into_iter().flatten() {
        out.push(run(p)?);
    }
    let mut sc = vec![];
    for p in meta.get("scalar_preshaders").and_then(Value::as_array).into_iter().flatten() {
        sc.push(run(p)?[0]);
    }
    while sc.len() % 4 != 0 {
        sc.push(0.0);
    }
    for c in sc.chunks(4) {
        out.push([c[0], c[1], c[2], c[3]]);
    }
    Ok(out)
}

fn preshader(data: &[u8], off: usize, size: usize, sp: &[Value], vp: &[Value], sv: &HashMap<String, f32>, vv: &HashMap<String, [f32; 4]>) -> Result<[f32; 4], String> {
    let mut st: Vec<([f32; 4], usize)> = vec![];
    let (mut i, end) = (off, off + size);
    let rd16 = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]) as usize;
    let bin = |st: &mut Vec<([f32; 4], usize)>, f: &dyn Fn(f32, f32) -> f32| -> Result<(), String> {
        let b = st.pop().ok_or("stack")?;
        let a = st.pop().ok_or("stack")?;
        let n = a.1.max(b.1);
        let av = if a.1 > 1 { a.0 } else { [a.0[0]; 4] };
        let bv = if b.1 > 1 { b.0 } else { [b.0[0]; 4] };
        st.push(([f(av[0], bv[0]), f(av[1], bv[1]), f(av[2], bv[2]), f(av[3], bv[3])], n));
        Ok(())
    };
    while i < end {
        let op = data[i];
        i += 1;
        match op {
            0x01 => st.push(([0.0; 4], 1)),
            0x02 => {
                let c = [0, 1, 2, 3].map(|k| f32::from_le_bytes(data[i + 4 * k..i + 4 * k + 4].try_into().unwrap()));
                i += 16;
                st.push((c, if c[0] == c[1] && c[1] == c[2] && c[2] == c[3] { 1 } else { 4 }));
            }
            0x03 => {
                let k = rd16(i);
                i += 2;
                let p = sp.get(k).ok_or("scalar param")?;
                let nm = p.get("name").and_then(Value::as_str).unwrap_or("").to_lowercase();
                let v = sv.get(&nm).copied().unwrap_or(p.get("default").and_then(Value::as_f64).unwrap_or(0.0) as f32);
                st.push(([v; 4], 1));
            }
            0x04 => {
                let k = rd16(i);
                i += 2;
                let p = vp.get(k).ok_or("vector param")?;
                let nm = p.get("name").and_then(Value::as_str).unwrap_or("").to_lowercase();
                let d = p.get("default").cloned().unwrap_or(Value::Null);
                let g = |c: &str| d.get(c).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                st.push((vv.get(&nm).copied().unwrap_or([g("R"), g("G"), g("B"), g("A")]), 4));
            }
            0x05 => bin(&mut st, &|a, b| a + b)?,
            0x06 => bin(&mut st, &|a, b| a - b)?,
            0x07 => bin(&mut st, &|a, b| a * b)?,
            0x08 => bin(&mut st, &|a, b| if b != 0.0 { a / b } else { 0.0 })?,
            0x0a => bin(&mut st, &|a, b| a.min(b))?,
            0x0b => bin(&mut st, &|a, b| a.max(b))?,
            0x0c => {
                let hi = st.pop().ok_or("stack")?;
                let lo = st.pop().ok_or("stack")?;
                let x = st.pop().ok_or("stack")?;
                let c = |v: &([f32; 4], usize), k: usize| if v.1 > 1 { v.0[k] } else { v.0[0] };
                st.push(([0, 1, 2, 3].map(|k| x.0[k].max(c(&lo, k)).min(c(&hi, k))), x.1));
            }
            0x0d | 0x0e => {
                let a = st.pop().ok_or("stack")?;
                st.push((a.0.map(|v| if op == 0x0d { v.sin() } else { v.cos() }), a.1));
            }
            0x14 => {
                i += 1;
                let b = st.pop().ok_or("stack")?;
                let a = st.pop().ok_or("stack")?;
                let n = a.1.min(b.1);
                let d: f32 = (0..n).map(|k| a.0[k] * b.0[k]).sum();
                st.push(([d; 4], 1));
            }
            0x15 => {
                i += 1;
                let b = st.pop().ok_or("stack")?;
                let a = st.pop().ok_or("stack")?;
                let (x, y) = (a.0, b.0);
                st.push(([x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0], 0.0], 3));
            }
            0x16 => {
                let a = st.pop().ok_or("stack")?;
                st.push((a.0.map(|v| v.max(0.0).sqrt()), a.1));
            }
            0x23 => {
                let n = data[i] as usize;
                let idx = [data[i + 1], data[i + 2], data[i + 3], data[i + 4]];
                i += 5;
                let a = st.pop().ok_or("stack")?;
                let mut v = [0.0; 4];
                for k in 0..n.min(4) {
                    v[k] = a.0[(idx[k] as usize).min(3)];
                }
                st.push((v, n));
            }
            0x24 => {
                i += 1;
                let b = st.pop().ok_or("stack")?;
                let a = st.pop().ok_or("stack")?;
                let mut v: Vec<f32> = a.0[..a.1.min(4)].to_vec();
                v.extend_from_slice(&b.0[..b.1.min(4)]);
                let n = v.len().min(4);
                v.resize(4, 0.0);
                st.push(([v[0], v[1], v[2], v[3]], n));
            }
            _ => return Err(format!("preshader opcode {op:02X} at {}", i - 1)),
        }
    }
    let (v, n) = st.pop().ok_or("empty")?;
    Ok(if n > 1 { v } else { [v[0]; 4] })
}

/// the View constant buffer slots the particle shaders read, from a Bevy camera
pub fn view_slots(cam: &GlobalTransform, clip_from_view: Mat4, size: Vec2, near_m: f32, time: f32, pre_exposure: f32) -> [Vec4; VIEW_SLOTS] {
    let mut v = [Vec4::ZERO; VIEW_SLOTS];
    let world_from_view = cam.to_matrix();
    let world_from_clip = world_from_view * clip_from_view.inverse();
    let cam_b = cam.translation();
    // Bevy world (m, Y up) -> UE translated world (cm, Z up): (x, z, y) x 100, minus the camera
    let a = Mat4::from_cols(Vec4::new(100.0, 0.0, 0.0, 0.0), Vec4::new(0.0, 0.0, 100.0, 0.0), Vec4::new(0.0, 100.0, 0.0, 0.0), Vec4::new(0.0, 0.0, 0.0, 1.0));
    let t = Mat4::from_translation(-cam_b);
    let ndc_from_px = Mat4::from_cols(
        Vec4::new(2.0 / size.x, 0.0, 0.0, 0.0),
        Vec4::new(0.0, -2.0 / size.y, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 0.0),
        Vec4::new(-1.0, 1.0, 0.0, 1.0),
    );
    let m = a * t * world_from_clip * ndc_from_px;
    for c in 0..4 {
        v[44 + c] = m.col(c);
    }
    // TranslatedWorldToClip (rows 4..7, used only on the lighting-volume path)
    let clip_from_tw = (a * t * world_from_clip).inverse();
    for c in 0..4 {
        v[4 + c] = clip_from_tw.col(c);
    }
    let cam_ue = Vec3::new(cam_b.x, cam_b.z, cam_b.y) * 100.0;
    v[70] = (-cam_ue).extend(0.0);
    // 60 / 61 / 62 ViewForward / ViewUp / ViewRight (UE world; the sphere-normal particles build their normal from
    // them), 67 / 69 WorldCameraOrigin / WorldViewOrigin, 68 TranslatedWorldCameraOrigin (0)
    let ue = |d: Vec3| Vec3::new(d.x, d.z, d.y);
    v[60] = ue(cam.forward().as_vec3()).extend(0.0);
    v[61] = ue(cam.up().as_vec3()).extend(0.0);
    v[62] = ue(cam.right().as_vec3()).extend(0.0);
    v[67] = cam_ue.extend(0.0);
    v[69] = cam_ue.extend(0.0);
    v[65] = Vec4::new(0.0, 0.0, 1.0 / (near_m * 100.0), 0.0);
    v[66] = Vec4::new(0.5, -0.5, 0.5, 0.5);
    v[28] = Vec4::new(clip_from_view.col(0).x, 0.0, 0.0, 0.0);
    v[129] = Vec4::ZERO;
    v[130] = Vec4::new(size.x, size.y, 1.0 / size.x, 1.0 / size.y);
    // 135.y PreExposure: the shaders multiply their scene colour by it (o0 x 135.y); UE's eye-adaptation exposure,
    // here the Bevy camera's (bevy-runtime lighting.rs maps the map's UE exposure to the camera's EV100)
    v[135] = Vec4::new(pre_exposure, pre_exposure, 1.0, 1.0);
    v[136] = Vec4::new(0.0, 0.0, 0.0, 1.0);
    v[137] = Vec4::new(0.0, 0.0, 0.0, 1.0);
    v[138] = Vec4::new(0.0, 0.0, 0.0, 1.0);
    v[139] = Vec4::new(0.0, 1.0, 0.0, 0.0);
    v[142] = Vec4::splat(time);
    v[157] = Vec4::ONE;
    v
}

pub fn white_3d() -> Image {
    Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D3, vec![255; 4], TextureFormat::Rgba8Unorm, RenderAssetUsages::default())
}

/// scene depth stand-in: device depth 0 (reversed Z: infinitely far)
pub fn far_depth() -> Image {
    Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, 0f32.to_le_bytes().to_vec(), TextureFormat::R32Float, RenderAssetUsages::default())
}

/// the ambient lighting volume stand-in (rgb = the ambient irradiance the volume would hold)
pub fn ambient_3d(c: [f32; 3]) -> Image {
    let b = c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
    Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D3, vec![b[0], b[1], b[2], 255], TextureFormat::Rgba8Unorm, RenderAssetUsages::default())
}

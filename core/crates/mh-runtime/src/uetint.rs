//! uetint.rs - the ue_tint material (rust-assets' WGSL port of godot/game/render/ue_tint.gdshaderinc, which cites
//! Mordhau's compiled shaders, docs/SHADERS.md) as a Bevy `Material` (`--materials uetint`).
//!
//! Binding contract (mh_assets::shader / shaders/ue_tint.wgsl header): binding 0 = the UeTint uniform (one vec4 per
//! `shader::UNIFORMS` entry), 1 = repeat sampler, 2 = clamp sampler, 3..17 = texture slots t0..t14. The WGSL hard-codes
//! @group(2); here it is rewritten to Bevy's `#{MATERIAL_BIND_GROUP}`. The samplers are taken from the images bound to
//! t0 (repeat: every image this runtime creates for a material has the repeat sampler) and t13 (the lightmap slot,
//! clamp). The fragment shader below fills Bevy's PbrInput from `ue_tint_surface` and runs Bevy's PBR lighting, so
//! lights / shadows / fog / exposure are Bevy's (the host side the WGSL header leaves to the engine).
//!
//! Render state (the Godot variants ue_tint_<masked|translucent>[_2s]): two-sided -> no culling (pipeline key);
//! translucent -> AlphaMode::Blend; masked -> the fragment discards below the clip value but the material reports
//! Opaque, because a custom material in Mask mode would run Bevy's default alpha-discard prepass, which reads
//! StandardMaterial bindings. Cost: masked cards cast full-quad shadows (UNCONFIRMED visual gap, R3: own prepass).
//! Baked lightmaps (lm_*) are not bound yet (rust-pak r3 is decoding MapBuildData): use_lightmap stays 0.

use bevy::asset::uuid_handle;
use bevy::image::Image;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat};
use bevy::shader::{Shader, ShaderRef};
use mh_assets::shader::{self as sh, SlotBinding, TexDefault};

pub const N_UNIFORMS: usize = sh::UNIFORMS.len();
pub const SHADER: Handle<Shader> = uuid_handle!("6d2f3a51-8c1e-4b7a-9f0d-2e5b7c3a9d41");

#[derive(ShaderType, Clone, Debug)]
pub struct UeTintUniform {
    pub v: [Vec4; N_UNIFORMS],
}

#[derive(ShaderType, Clone, Debug, Default)]
pub struct UeVlmUniform {
    pub v: [Vec4; 5],
}

/// The map's VLM on the GPU (all materials of a map share it)
#[derive(Clone, Debug)]
pub struct VlmBind {
    pub ind: Handle<Image>,
    pub amb: Handle<Image>,
    pub sh: Handle<Image>,
    pub u: UeVlmUniform,
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
#[bind_group_data(UeTintKey)]
pub struct UeTintMaterial {
    #[uniform(0)]
    pub u: UeTintUniform,
    #[texture(3)]
    #[sampler(1)]
    pub t0: Handle<Image>,
    #[texture(4)]
    pub t1: Handle<Image>,
    #[texture(5)]
    pub t2: Handle<Image>,
    #[texture(6)]
    pub t3: Handle<Image>,
    #[texture(7)]
    pub t4: Handle<Image>,
    #[texture(8)]
    pub t5: Handle<Image>,
    #[texture(9)]
    pub t6: Handle<Image>,
    #[texture(10)]
    pub t7: Handle<Image>,
    #[texture(11)]
    pub t8: Handle<Image>,
    #[texture(12)]
    pub t9: Handle<Image>,
    #[texture(13)]
    pub t10: Handle<Image>,
    #[texture(14)]
    pub t11: Handle<Image>,
    #[texture(15)]
    pub t12: Handle<Image>,
    #[texture(16)]
    #[sampler(2)]
    pub t13: Handle<Image>,
    #[texture(17)]
    pub t14: Handle<Image>,
    // volumetric lightmap (ue_tint.wgsl 18-21: indirection RGBA8 unorm read with textureLoad, AmbientVector RGBA16F,
    // the six SH layers stacked along Z RGBA8 unorm; UeVlm = mh_assets::vlm::Vlm::uniforms)
    #[texture(18, dimension = "3d")]
    pub vlm_ind: Handle<Image>,
    #[texture(19, dimension = "3d")]
    pub vlm_amb: Handle<Image>,
    #[texture(20, dimension = "3d")]
    pub vlm_sh: Handle<Image>,
    #[uniform(21)]
    pub vlm: UeVlmUniform,
    pub blend: i32,
    pub two_sided: bool,
    pub debug_albedo: bool,
    /// MSM_Unlit (MaterialDesc::unlit): base_color is the emissive, no lighting
    pub unlit: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UeTintKey {
    pub two_sided: bool,
    /// BLEND_Masked: the only render state that alpha-tests (the Godot ue_tint_masked variant = UE_MASKED define;
    /// opaque materials never discard, whatever the albedo alpha holds)
    pub masked: bool,
    /// debug: output the surface's base colour unlit (`--debug-albedo` / script `debug albedo`)
    pub debug_albedo: bool,
    pub unlit: bool,
}

impl From<&UeTintMaterial> for UeTintKey {
    fn from(m: &UeTintMaterial) -> Self {
        UeTintKey { two_sided: m.two_sided, masked: m.blend == 1, debug_albedo: m.debug_albedo, unlit: m.unlit }
    }
}

impl Material for UeTintMaterial {
    /// the host vertex stage (VERTEX below: Bevy's mesh vertex + the weapon smear's World Position Offset)
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER)
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER)
    }
    fn alpha_mode(&self) -> AlphaMode {
        match self.blend {
            2 => AlphaMode::Blend,
            3 => AlphaMode::Add,
            4 => AlphaMode::Multiply,
            _ => AlphaMode::Opaque, // masked: discard in the fragment (module header)
        }
    }
    fn specialize(
        _p: &MaterialPipeline,
        d: &mut RenderPipelineDescriptor,
        _l: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.debug_albedo {
            if let Some(f) = d.fragment.as_mut() {
                f.shader_defs.push("UE_DEBUG_ALBEDO".into());
            }
        }
        if key.bind_group_data.unlit {
            if let Some(f) = d.fragment.as_mut() {
                f.shader_defs.push("UE_UNLIT".into());
            }
        }
        if key.bind_group_data.masked {
            if let Some(f) = d.fragment.as_mut() {
                f.shader_defs.push("UE_MASKED".into());
            }
        }
        if key.bind_group_data.two_sided {
            d.primitive.cull_mode = None;
        } else {
            d.primitive.cull_mode = Some(Face::Back);
        }
        Ok(())
    }
}

const HEADER: &str = r#"
#import bevy_pbr::{pbr_types, pbr_functions, forward_io::{Vertex, VertexOutput, FragmentOutput}, mesh_view_bindings::view}
#import bevy_pbr::{mesh_functions, mesh_bindings, skinning, morph, view_transformations}
"#;

const VERTEX: &str = r#"
// mh-runtime host vertex stage: Bevy's mesh.wgsl vertex (bevy_pbr 0.19.1 src/render/mesh.wgsl, skinning / morph /
// normals / tangents unchanged) + M_WeaponMaster's World Position Offset (mh_assets::weapon_trail; ue_tint.wgsl
// ue_tint_trail_offset: zero unless the host fills the trail_* uniforms of a held weapon's own material during a swing).
// The mesh-space position is converted to UE cm (glTF SwapYZ x 100) and the UE world cm offset back to Bevy m.
#ifdef MORPH_TARGETS
fn ue_morph_vertex(vertex_in: Vertex, instance_index: u32) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh_bindings::mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;
    let weight_count = morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = morph::weight_at(i, instance_index);
        if weight == 0.0 { continue; }
        vertex.position += weight * morph::morph_position(vertex_index, i, instance_index);
#ifdef VERTEX_NORMALS
        vertex.normal += weight * morph::morph_normal(vertex_index, i, instance_index);
#endif
#ifdef VERTEX_TANGENTS
        vertex.tangent += vec4(weight * morph::morph_tangent(vertex_index, i, instance_index), 0.0);
#endif
    }
    return vertex;
}
#endif

@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
    var out: VertexOutput;
#ifdef MORPH_TARGETS
    var vertex = ue_morph_vertex(vertex_no_morph, vertex_no_morph.instance_index);
#else
    var vertex = vertex_no_morph;
#endif
    let mesh_world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);
#ifdef SKINNED
    var world_from_local = skinning::skin_model(vertex.joint_indices, vertex.joint_weights, vertex_no_morph.instance_index);
#else
    var world_from_local = mesh_world_from_local;
#endif
#ifdef VERTEX_NORMALS
#ifdef SKINNED
    out.world_normal = skinning::skin_normals(world_from_local, vertex.normal);
#else
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_no_morph.instance_index);
#endif
#endif
#ifdef VERTEX_POSITIONS
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    let p_ue = vec3<f32>(vertex.position.x, vertex.position.z, vertex.position.y) * 100.0;
    let wpo = ue_tint_trail_offset(p_ue);
    out.world_position = vec4<f32>(out.world_position.xyz + vec3<f32>(wpo.x, wpo.z, wpo.y) * 0.01, out.world_position.w);
    out.position = view_transformations::position_world_to_clip(out.world_position.xyz);
#endif
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex_no_morph.instance_index);
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex_no_morph.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex_no_morph.instance_index, mesh_world_from_local[3]);
#endif
    return out;
}
"#;

const FRAGMENT: &str = r#"
// mh-runtime host: Bevy VertexOutput -> UeTintIn (world space, Y up), ue_tint_surface -> PbrInput -> Bevy lighting.
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var ti: UeTintIn;
#ifdef VERTEX_UVS_A
    ti.uv = in.uv;
#endif
#ifdef VERTEX_UVS_B
    ti.uv2 = in.uv_b;
#endif
    ti.custom0 = vec2<f32>(0.0);
#ifdef VERTEX_COLORS
    ti.color = in.color;
#else
    ti.color = vec4<f32>(1.0);
#endif
    ti.world_pos = in.world_position.xyz;
    var n = normalize(in.world_normal);
    if (!is_front) { n = -n; }
#ifdef VERTEX_TANGENTS
    let t = normalize(in.world_tangent.xyz);
    let b = cross(n, t) * in.world_tangent.w;
#else
    let t = normalize(cross(n, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9)));
    let b = cross(n, t);
#endif
    ti.n = n;
    ti.t = t;
    ti.b = b;
    ti.v = normalize(view.world_position.xyz - in.world_position.xyz);
    let s = ue_tint_surface(ti);
#ifdef UE_MASKED
    if (s.alpha_clip > 0.0 && s.alpha < s.alpha_clip) { discard; }
#endif

    var pbr = pbr_types::pbr_input_new();
    pbr.material.base_color = vec4<f32>(s.base, s.alpha);
    pbr.material.perceptual_roughness = clamp(s.roughness, 0.045, 1.0);
    pbr.material.metallic = s.metallic;
    // UE Specular 0.5 = F0 0.04 = Bevy reflectance 0.5 (both 0.08 * x)
    pbr.material.reflectance = vec3<f32>(s.specular);
    // baked lighting (UE adds it to scene colour): HQ lightmap with sky occlusion, else the VLM (gdshaderinc use_lightmap /
    // use_vlm blocks); added as emissive so Bevy's exposure applies to it like to the dynamic lights
    var baked = vec3<f32>(0.0);
    var ao = s.ao;
    if (b1(u.use_lightmap)) {
        let lm = ue_lightmap(ti, s, n);
        baked = lm.emissive;
        ao = lm.ao;
    } else {
        baked = ue_vlm(ti, s, n);
    }
    pbr.material.emissive = vec4<f32>(s.emissive + baked, 1.0);
    pbr.diffuse_occlusion = vec3<f32>(ao);
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;
    pbr.world_normal = n;
    pbr.N = normalize(t * s.normal_ts.x + b * s.normal_ts.y + n * s.normal_ts.z);
    pbr.V = ti.v;
    pbr.is_orthographic = view.clip_from_view[3].w == 1.0;
    var out: FragmentOutput;
#ifdef UE_UNLIT
    // MSM_Unlit: UE's base pass writes EmissiveColor only (no lighting, no fog here); the view exposure and the
    // tonemapper still apply, as for UE's scene colour
    out.color = vec4<f32>(s.base * view.exposure, 1.0);
    return out;
#endif
#ifdef UE_DEBUG_ALBEDO
    // unlit base colour (the camera's tonemapper still applies)
    out.color = vec4<f32>(s.base, 1.0);
    return out;
#endif
    out.color = pbr_functions::apply_pbr_lighting(pbr);
    out.color = pbr_functions::main_pass_post_lighting_processing(pbr, out.color);
    return out;
}
"#;

/// The composed shader source (header + rust-assets' WGSL with the bind group substituted + the host fragment).
pub fn shader_source() -> String {
    let body = sh::ue_tint_wgsl().replace("@group(2)", "@group(#{MATERIAL_BIND_GROUP})");
    format!("{HEADER}\n{body}\n{VERTEX}\n{FRAGMENT}")
}

/// 1x1 default images for unbound slots (TexDefault: Godot hint_default_white / black / normal).
#[derive(Resource, Clone)]
pub struct Defaults {
    pub white: Handle<Image>,
    pub black: Handle<Image>,
    pub normal: Handle<Image>,
    /// the lightmap slot t13 carries sampler 2 (clamp)
    pub black_clamp: Handle<Image>,
    /// no VLM: 1x1x1 zero volumes and a zero uniform (ue_vlm returns 0 when vlm.scale.x == 0)
    pub no_vlm: VlmBind,
}

fn vol(images: &mut Assets<Image>, size: [u32; 3], data: Vec<u8>, fmt: TextureFormat) -> Handle<Image> {
    let mut i = Image::new(Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] }, TextureDimension::D3, data, fmt,
        bevy::asset::RenderAssetUsages::RENDER_WORLD);
    i.sampler = crate::paksrc::clamp_sampler();
    images.add(i)
}

/// Upload a map's VLM (mh_assets::vlm::Vlm: GPU layouts of ue_tint.wgsl bindings 18-20 and the UeVlm uniform)
pub fn upload_vlm(images: &mut Assets<Image>, v: &mh_assets::vlm::Vlm) -> Result<VlmBind, String> {
    v.validate().map_err(|e| e.0)?;
    let d = v.indirection_dims.map(|x| x as u32);
    let b = v.brick_dims.map(|x| x as u32);
    let amb: Vec<u8> = v.ambient_rgba16f().iter().flat_map(|h| h.to_le_bytes()).collect();
    let u = v.uniforms();
    Ok(VlmBind {
        ind: vol(images, d, v.indirection.data.clone(), TextureFormat::Rgba8Unorm),
        amb: vol(images, b, amb, TextureFormat::Rgba16Float),
        sh: vol(images, [b[0], b[1], b[2] * 6], v.sh_stack(), TextureFormat::Rgba8Unorm),
        u: UeVlmUniform { v: u.map(Vec4::from_array) },
    })
}

fn px(images: &mut Assets<Image>, c: [u8; 4], clamp: bool) -> Handle<Image> {
    let mut i = Image::new(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, c.to_vec(),
        TextureFormat::Rgba8Unorm, bevy::asset::RenderAssetUsages::default());
    i.sampler = if clamp { crate::paksrc::clamp_sampler() } else { crate::paksrc::repeat_sampler() };
    images.add(i)
}

pub struct UeTintPlugin;

impl Plugin for UeTintPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<UeTintMaterial>::default())
            .init_resource::<DebugAlbedo>()
            .add_systems(PreStartup, setup)
            .add_systems(Update, apply_debug);
    }
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(SHADER.id(), Shader::from_wgsl(shader_source(), "mh-runtime/ue_tint_host.wgsl"));
    commands.insert_resource(Defaults {
        white: px(&mut images, [255, 255, 255, 255], false),
        black: px(&mut images, [0, 0, 0, 255], false),
        normal: px(&mut images, [128, 128, 255, 255], false),
        black_clamp: px(&mut images, [0, 0, 0, 255], true),
        no_vlm: VlmBind {
            ind: vol(&mut images, [1, 1, 1], vec![0; 4], TextureFormat::Rgba8Unorm),
            amb: vol(&mut images, [1, 1, 1], vec![0; 8], TextureFormat::Rgba16Float),
            sh: vol(&mut images, [1, 1, 1], vec![0; 4], TextureFormat::Rgba8Unorm),
            u: UeVlmUniform::default(),
        },
    });
}

/// A material for a resolved MaterialDesc. `tex(texref, srgb)` returns the image for a slot (None = default).
pub fn build(
    desc: &mh_assets::material::MaterialDesc,
    defaults: &Defaults,
    vlm: Option<&VlmBind>,
    mut tex: impl FnMut(&str, bool) -> Option<Handle<Image>>,
) -> (UeTintMaterial, usize) {
    let g = sh::gpu(desc);
    let mut v = [Vec4::ZERO; N_UNIFORMS];
    for (i, u) in g.uniforms.iter().enumerate().take(N_UNIFORMS) {
        v[i] = Vec4::from_array(*u);
    }
    let mut bound = 0;
    let mut slots: Vec<Handle<Image>> = Vec::with_capacity(sh::NUM_TEX_SLOTS);
    for (i, s) in g.slots.iter().enumerate() {
        let d = |k: &TexDefault| match k {
            TexDefault::White => defaults.white.clone(),
            TexDefault::Black => if i == 13 { defaults.black_clamp.clone() } else { defaults.black.clone() },
            TexDefault::Normal => defaults.normal.clone(),
        };
        slots.push(match s {
            SlotBinding::Texture { tex: t, srgb, .. } => match tex(&t.0, *srgb) {
                Some(h) => {
                    bound += 1;
                    h
                }
                None => d(&TexDefault::White),
            },
            SlotBinding::Default(k) => d(k),
        });
    }
    while slots.len() < 15 {
        slots.push(defaults.white.clone());
    }
    let s = |i: usize| slots[i].clone();
    let m = UeTintMaterial {
        u: UeTintUniform { v },
        t0: s(0), t1: s(1), t2: s(2), t3: s(3), t4: s(4), t5: s(5), t6: s(6), t7: s(7),
        t8: s(8), t9: s(9), t10: s(10), t11: s(11), t12: s(12), t13: s(13), t14: s(14),
        vlm_ind: vlm.unwrap_or(&defaults.no_vlm).ind.clone(),
        vlm_amb: vlm.unwrap_or(&defaults.no_vlm).amb.clone(),
        vlm_sh: vlm.unwrap_or(&defaults.no_vlm).sh.clone(),
        vlm: vlm.unwrap_or(&defaults.no_vlm).u.clone(),
        blend: desc.blend,
        two_sided: desc.two_sided,
        debug_albedo: false,
        unlit: desc.unlit,
    };
    (m, bound)
}

#[cfg(test)]
mod tests {
    #[test]
    fn composed_shader_has_host_entry_and_no_fixed_group() {
        let s = super::shader_source();
        assert!(s.contains("fn fragment("));
        assert!(!s.contains("@group(2)"));
        assert!(s.contains("fn ue_tint_surface("));
    }
}

/// `--debug-albedo` / script `debug albedo|lit`: switch every ue_tint material's unlit base-colour view.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct DebugAlbedo(pub bool);

pub fn apply_debug(dbg: Res<DebugAlbedo>, mut mats: ResMut<Assets<UeTintMaterial>>, mut last: Local<Option<bool>>, mut n: Local<usize>) {
    let count = mats.len();
    if *last == Some(dbg.0) && *n == count {
        return;
    }
    *last = Some(dbg.0);
    *n = count;
    let ids: Vec<_> = mats.iter().filter(|(_, m)| m.debug_albedo != dbg.0).map(|(id, _)| id).collect();
    for id in ids {
        if let Some(mut m) = mats.get_mut(id) {
            m.debug_albedo = dbg.0;
        }
    }
}

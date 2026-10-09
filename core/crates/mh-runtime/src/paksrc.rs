//! paksrc.rs - Bevy meshes and images decoded from the user's paks with mh-assets (rust-assets), the `--assets pak`
//! path (the extract/gltf glb + PNG path stays the default). Mirrors what the Godot `pak` backend builds
//! (godot/components/ue/pak/ue_static_mesh.gd array_mesh, ue_texture.gd), and is checked against the glb/PNG exports by
//! the `equiv_assets` script verb (equiv.rs).

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use mh_assets::coords;
use mh_assets::pak_source::PakSource;
use mh_assets::static_mesh::{self, StaticMesh};
use mh_assets::texture::{self, PixelFormat, Pixels};

/// Binormal sign of the decoded tangents relative to Bevy's (glTF) convention. The UE -> Y-up swap mirrors handedness,
/// so B = cross(N, T) * w flips: the stored w is negated. Checked against the CUE4Parse glb TANGENT.w by equiv_assets
/// (state/runtime_evidence/.../assets_equiv.json "tangent_w_agree").
pub const TANGENT_W_SIGN: f32 = -1.0;

/// UE UV channel 2 (the lightmap channel of meshes with LightMapCoordinateIndex 2); not read by Bevy's vertex shader,
/// kept on the mesh so a lightmapped copy can move it into UV_1 (level.rs)
pub const ATTRIBUTE_UV_2: bevy::mesh::MeshVertexAttribute =
    bevy::mesh::MeshVertexAttribute::new("Vertex_Uv_2_Ue", 0x6d68_7532, bevy::render::render_resource::VertexFormat::Float32x2);

/// One primitive per LOD0 section with triangles, in section order (CUE4Parse Gltf.cs:155-161, the glb layout),
/// vertices restricted to the section's [MinVertexIndex, MaxVertexIndex]. Returns (mesh, material slot).
pub fn sections(sm: &StaticMesh, colors: bool) -> Vec<(Mesh, usize)> {
    let v = &sm.vertices;
    let mut out = Vec::new();
    for s in &sm.sections {
        if s.num_triangles <= 0 {
            continue;
        }
        let (lo, hi) = (s.min_vertex_index.max(0) as usize, s.max_vertex_index.max(0) as usize);
        if hi >= v.positions.len() || lo > hi {
            continue;
        }
        let r = lo..hi + 1;
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, v.positions[r.clone()].iter().map(|p| coords::pos(*p)).collect::<Vec<_>>());
        if v.normals.len() == v.positions.len() {
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, v.normals[r.clone()].iter().map(|n| coords::dir(*n)).collect::<Vec<_>>());
        }
        if v.tangents.len() == v.positions.len() {
            m.insert_attribute(
                Mesh::ATTRIBUTE_TANGENT,
                v.tangents[r.clone()]
                    .iter()
                    .map(|t| {
                        let d = coords::dir([t[0], t[1], t[2]]);
                        [d[0], d[1], d[2], t[3] * TANGENT_W_SIGN]
                    })
                    .collect::<Vec<_>>(),
            );
        }
        if let Some(uv) = v.uvs.first().filter(|u| u.len() == v.positions.len()) {
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv[r.clone()].to_vec());
        }
        if let Some(uv) = v.uvs.get(1).filter(|u| u.len() == v.positions.len()) {
            m.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv[r.clone()].to_vec());
        }
        if let Some(uv) = v.uvs.get(2).filter(|u| u.len() == v.positions.len()) {
            m.insert_attribute(ATTRIBUTE_UV_2, uv[r.clone()].to_vec());
        }
        if colors && sm.colors.len() == v.positions.len() {
            // FColor / 255 as stored (ue_tint.wgsl header: no sRGB decode, as UE's vertex factory)
            m.insert_attribute(
                Mesh::ATTRIBUTE_COLOR,
                sm.colors[r.clone()].iter().map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0]).collect::<Vec<_>>(),
            );
        }
        let a = s.first_index.max(0) as usize;
        let b = (a + 3 * s.num_triangles as usize).min(sm.indices.len());
        // UE's corner order is already counter-clockwise-front after the Y/Z swap (mh_assets::coords header)
        let idx: Vec<u32> = sm.indices[a..b].iter().map(|i| i.saturating_sub(lo as u32)).collect();
        m.insert_indices(Indices::U32(idx));
        out.push((m, s.material_index.max(0) as usize));
    }
    out
}

pub fn mesh(src: &PakSource, pkg: &str, colors: bool) -> Result<Vec<(Mesh, usize)>, String> {
    static_mesh::lod0(src, pkg).map(|sm| sections(&sm, colors)).map_err(|e| e.0)
}

/// Repeat + trilinear + anisotropic: the ue_tint `s_repeat` sampler (Godot filter_linear_mipmap_anisotropic,
/// repeat_enable; ue_tint.wgsl header).
pub fn repeat_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..Default::default()
    })
}

pub fn clamp_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..Default::default()
    })
}

/// Texture reference (mh_assets::material::TexRef spelling): the package, or "<package>/<object>" for a sub-export.
pub fn tex_info(src: &PakSource, r: &str) -> Result<texture::Texture, String> {
    match texture::info(src, r, None) {
        Ok(t) => Ok(t),
        Err(e) => match r.rsplit_once('/') {
            Some((pkg, obj)) => texture::info(src, pkg, Some(obj)).map_err(|e2| format!("{}; {}", e.0, e2.0)),
            None => Err(e.0),
        },
    }
}

/// GPU format for a texture's own encoding (no decode): the BC formats upload compressed.
fn gpu_format(f: PixelFormat, srgb: bool) -> Option<TextureFormat> {
    Some(match (f, srgb) {
        (PixelFormat::Dxt1, true) => TextureFormat::Bc1RgbaUnormSrgb,
        (PixelFormat::Dxt1, false) => TextureFormat::Bc1RgbaUnorm,
        (PixelFormat::Dxt3, true) => TextureFormat::Bc2RgbaUnormSrgb,
        (PixelFormat::Dxt3, false) => TextureFormat::Bc2RgbaUnorm,
        (PixelFormat::Dxt5, true) => TextureFormat::Bc3RgbaUnormSrgb,
        (PixelFormat::Dxt5, false) => TextureFormat::Bc3RgbaUnorm,
        (PixelFormat::Bc4, _) => TextureFormat::Bc4RUnorm,
        (PixelFormat::Bc5, _) => TextureFormat::Bc5RgUnorm,
        (PixelFormat::Bc6h, _) => TextureFormat::Bc6hRgbUfloat,
        (PixelFormat::Bc7, true) => TextureFormat::Bc7RgbaUnormSrgb,
        (PixelFormat::Bc7, false) => TextureFormat::Bc7RgbaUnorm,
        (PixelFormat::B8G8R8A8, true) => TextureFormat::Bgra8UnormSrgb,
        (PixelFormat::B8G8R8A8, false) => TextureFormat::Bgra8Unorm,
        (PixelFormat::FloatRgba, _) => TextureFormat::Rgba16Float,
        // G8 is decoded to (l, l, l, 255) like the exported PNG (texture::decode), see `image`
        (PixelFormat::G8, _) => return None,
    })
}

#[derive(Default, Debug, Clone, Copy, serde::Serialize)]
pub struct TexStats {
    pub compressed: usize,
    pub decoded: usize,
    pub failed: usize,
    pub bytes: usize,
}

/// The texture as a Bevy image with its whole stored mip chain. Block formats go to the GPU compressed when mip 0 is a
/// whole number of 4x4 blocks (wgpu's rule for BC textures); otherwise (and for G8) mip 0 is decoded to RGBA8.
pub fn image(src: &PakSource, r: &str, srgb: bool, st: &mut TexStats) -> Result<Image, String> {
    let t = tex_info(src, r)?;
    let f = t.format.ok_or(format!("{r}: format {} not supported", t.pixel_format_name))?;
    let m0 = t.mips.first().ok_or(format!("{r}: no mips"))?;
    let (w, h) = (m0.size_x.max(1) as u32, m0.size_y.max(1) as u32);
    let size = Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let whole = f.block_bytes().is_none() || (w % 4 == 0 && h % 4 == 0);
    let mut img = match gpu_format(f, srgb).filter(|_| whole) {
        Some(fmt) => {
            let mut data = Vec::new();
            let mut n = 0u32;
            for lvl in 0..t.mips.len() {
                match texture::mip_data(src, &t, lvl) {
                    Ok(b) => {
                        let m = &t.mips[lvl];
                        let need = f.mip_size(m.size_x.max(1) as usize, m.size_y.max(1) as usize);
                        // the chain must halve each level (wgpu); stop at the first level that does not
                        if m.size_x.max(1) as u32 != (w >> lvl).max(1) || m.size_y.max(1) as u32 != (h >> lvl).max(1) {
                            break;
                        }
                        data.extend_from_slice(&b[..need]);
                        n += 1;
                    }
                    Err(_) => break,
                }
            }
            if n == 0 {
                return Err(format!("{r}: mip 0 unreadable"));
            }
            st.compressed += 1;
            st.bytes += data.len();
            // GPU copy only: the main-world bytes are dropped after upload (hundreds of MB for one map)
            let mut img = Image::new_uninit(size, TextureDimension::D2, fmt, RenderAssetUsages::RENDER_WORLD);
            img.data = Some(data);
            img.texture_descriptor.mip_level_count = n;
            img
        }
        None => {
            let px = decoded_rgba8(src, &t)?;
            st.decoded += 1;
            st.bytes += px.len();
            Image::new(size, TextureDimension::D2, px, if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm }, RenderAssetUsages::default())
        }
    };
    img.sampler = repeat_sampler();
    Ok(img)
}

/// Mip 0 decoded to RGBA8 (HDR clamped to [0, 1]).
pub fn decoded_rgba8(src: &PakSource, t: &texture::Texture) -> Result<Vec<u8>, String> {
    let f = t.format.ok_or("format")?;
    let m0 = t.mips.first().ok_or("no mips")?;
    let b = texture::mip_data(src, t, 0).map_err(|e| e.0)?;
    match texture::decode(f, m0.size_x.max(1) as usize, m0.size_y.max(1) as usize, &b).map_err(|e| e.0)? {
        Pixels::Rgba8(p) => Ok(p),
        Pixels::RgbaF32(p) => Ok(p.iter().map(|x| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()),
    }
}

/// Packed texture -> Bevy's AO (R) / roughness (G) / metallic (B) layout (material.rs layout_sel, ue_material.gd
/// LAYOUTS), from the decoded mip 0.
pub fn packed_image(src: &PakSource, r: &str, sel: (Option<usize>, Option<usize>, Option<usize>)) -> Result<Image, String> {
    let t = tex_info(src, r)?;
    let m0 = t.mips.first().ok_or(format!("{r}: no mips"))?;
    let mut px = decoded_rgba8(src, &t)?;
    crate::material::swizzle(&mut px, sel);
    let size = Extent3d { width: m0.size_x.max(1) as u32, height: m0.size_y.max(1) as u32, depth_or_array_layers: 1 };
    let mut img = Image::new(size, TextureDimension::D2, px, TextureFormat::Rgba8Unorm, RenderAssetUsages::default());
    img.sampler = repeat_sampler();
    Ok(img)
}

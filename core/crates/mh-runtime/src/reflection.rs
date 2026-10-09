//! reflection.rs - the map's cooked reflection capture as Bevy's specular environment (r4 item 1).
//!
//! Why: the Knight rendered uniformly blue. The unlit albedo view (`debug albedo`, 075644-armor/knight_albedo.png) shows
//! correct steel / leather, so the material was right and the lighting was not: plate is metallic, metals have no
//! diffuse, and with no environment map Bevy's only specular source is GlobalAmbientLight - which lighting.rs had set to
//! the SkyLight-tinted sky MID colour (blue). UE lights specular from the level's reflection captures (cubemaps of the
//! arena, mostly sand and stone) and the indirect diffuse from the lightmaps / VLM. So:
//!   - specular: the capture's prefiltered cubemap (mh-level FReflectionCaptureData FullHDRCapturedData, RGBA16F,
//!     mip-major then face) on the camera's EnvironmentMapLight, intensity = the capture component's Brightness
//!     (UReflectionCaptureComponent default 1, recalled: UNCONFIRMED when absent);
//!   - diffuse: black (UE's baked lighting already supplies it through ue_tint's lightmap / VLM terms), and the ambient
//!     light is zeroed on that path.
//! One capture for the whole map (the one nearest the PlayerStarts' centre); UE blends captures per pixel by their
//! influence radii (not ported: UNCONFIRMED approximation).
//!
//! Orientation: UE samples its cubes with UE-space directions (X fwd, Y right, Z up); Bevy with Y-up directions. The
//! faces are re-sampled: for each Bevy texel at wgpu face direction d, the Bevy direction it answers is (d.x, d.y, -d.z)
//! (Bevy negates z when sampling) = UE direction (d.x, -d.z, d.y) -> the UE face and texel (both in the D3D / wgpu cube
//! convention), nearest texel.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};

/// D3D / wgpu cube face + (u, v) in [0, 1] -> direction
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

/// direction -> D3D / wgpu cube face + (u, v)
fn dir_face(d: Vec3) -> (usize, f32, f32) {
    let a = d.abs();
    let (f, sc, tc, ma) = if a.x >= a.y && a.x >= a.z {
        if d.x > 0.0 { (0, -d.z, -d.y, a.x) } else { (1, d.z, -d.y, a.x) }
    } else if a.y >= a.z {
        if d.y > 0.0 { (2, d.x, d.z, a.y) } else { (3, d.x, -d.z, a.y) }
    } else if d.z > 0.0 {
        (4, d.x, -d.y, a.z)
    } else {
        (5, -d.x, -d.y, a.z)
    };
    (f, (sc / ma + 1.0) * 0.5, (tc / ma + 1.0) * 0.5)
}

fn cube_image(size: u32, mips: u32, data: Vec<u8>) -> Image {
    let mut img = Image::new_uninit(Extent3d { width: size, height: size, depth_or_array_layers: 6 }, TextureDimension::D2, TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD);
    img.data = Some(data);
    img.texture_descriptor.mip_level_count = mips;
    img.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..Default::default() });
    img
}

/// The capture's cubemap re-sampled into Bevy's axes, every mip (LayerMajor: face 0 mips, face 1 mips, ...).
pub fn specular_image(c: &mh_level::vlm::ReflectionCapture) -> Option<Image> {
    let n = c.cubemap_size;
    let mips = c.mip_count();
    if n == 0 || c.full_hdr.len() < c.expected_len() {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(c.expected_len());
    for f in 0..6 {
        for m in 0..mips {
            let s = (n >> m).max(1);
            for y in 0..s {
                for x in 0..s {
                    let d = face_dir(f, (x as f32 + 0.5) / s as f32, (y as f32 + 0.5) / s as f32);
                    // Bevy samples its cubes with z negated (bevy_pbr environment_map.wgsl "Cube maps are left-handed"):
                    // the texel at face direction d answers Bevy direction (d.x, d.y, -d.z) = UE (d.x, -d.z, d.y).
                    // rust-render r1: was (d.x, d.z, d.y), which mirrored every capture across UE's XZ plane.
                    let (uf, u, v) = dir_face(Vec3::new(d.x, -d.z, d.y));
                    let src = c.face(m, uf)?;
                    let (sx, sy) = (((u * s as f32) as usize).min(s - 1), ((v * s as f32) as usize).min(s - 1));
                    let o = (sy * s + sx) * 8;
                    out.extend_from_slice(&src[o..o + 8]);
                }
            }
        }
    }
    Some(cube_image(n as u32, mips as u32, out))
}

pub fn black_cube() -> Image {
    cube_image(1, 1, vec![0u8; 6 * 8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_convention_round_trips() {
        for f in 0..6 {
            for (u, v) in [(0.2, 0.7), (0.5, 0.5), (0.9, 0.1)] {
                let (g, uu, vv) = dir_face(face_dir(f, u, v));
                assert_eq!(g, f);
                assert!((uu - u).abs() < 1e-5 && (vv - v).abs() < 1e-5, "{f} {u} {v} -> {uu} {vv}");
            }
        }
    }
}

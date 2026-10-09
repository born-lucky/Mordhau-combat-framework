//! Precomputed volumetric lightmap (FPrecomputedVolumetricLightmapData, UE 4.26) and reflection-capture build data,
//! decoded from the MapBuildDataRegistry (builddata.rs). Layout: CUE4Parse UMapBuildDataRegistry.cs:298-389
//! (volumetric lightmap) and :133-188 (FReflectionCaptureData). The reference sampler `irradiance` is a port of
//! scripts/map/vlm_extract.py, which follows the game's shader (INST_SparseGrass_Arena r0/001 ...VolumetricLightmap...
//! lines 163-177 brick UVs, 201-269 SH3 decode; godot/game/render/ue_vlm.gdshaderinc).
//!
//! Volumes are UE space (cm, Z up). Brick textures are 3D, x fastest: texel (x, y, z) at ((z * H + y) * W + x) * bpp.

use mh_pak::Bytes;

/// One FVolumetricLightmapDataLayer: TArray<uint8> Data + FString PixelFormatString (:386-389), as a view into the paks
#[derive(Clone, Debug)]
pub struct VlmLayer {
    pub data: Bytes,
    /// "PF_R8G8B8A8", "PF_FloatR11G11B10", "PF_R8G8B8A8_UINT", "PF_G8", ...
    pub format: String,
}

#[derive(Clone, Debug)]
pub struct VolumetricLightmap {
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    /// IndirectionTextureDimensions (16^3 Arena, 48x48x16 Cortile ...)
    pub indirection_dims: [usize; 3],
    /// RGBA8 UINT per cell: brick xyz (in bricks) + brick size multiplier (w)
    pub indirection: VlmLayer,
    pub brick_size: usize,
    /// BrickDataDimensions in texels (bricks of brick_size + 1 padding texel)
    pub brick_dims: [usize; 3],
    /// AmbientVector (PF_FloatR11G11B10 in Mordhau's data; the shader's SH band 0 per colour)
    pub ambient: VlmLayer,
    /// SHCoefficients0R, 1R, 0G, 1G, 0B, 1B (PF_R8G8B8A8 unorm, x 2 - 1, scaled by the ambient channel)
    pub sh: [VlmLayer; 6],
    pub sky_bent_normal: VlmLayer,
    pub directional_light_shadowing: VlmLayer,
    /// mobile low-quality layers (FMobileObjectVersion LQVolumetricLightmapLayers)
    pub lq_light_color: Option<VlmLayer>,
    pub lq_light_direction: Option<VlmLayer>,
    pub sub_level_brick_positions: Vec<[i32; 3]>,
    pub indirection_original_values: Vec<[u8; 4]>,
}

/// R11G11B10 float (unsigned, 5-bit exponent bias 15) -> f32, as vlm_extract.py r11g11b10
fn uf(bits: u32, mbits: u32) -> f32 {
    let e = (bits >> mbits) & 0x1f;
    let m = (bits & ((1 << mbits) - 1)) as f32 / (1u32 << mbits) as f32;
    if e == 0 {
        m * 2f32.powi(-14)
    } else {
        (1.0 + m) * 2f32.powi(e as i32 - 15)
    }
}

impl VolumetricLightmap {
    fn texels(&self) -> usize {
        self.brick_dims[0] * self.brick_dims[1] * self.brick_dims[2]
    }
    /// AmbientVector texel i as linear RGB
    pub fn ambient_rgb(&self, i: usize) -> [f32; 3] {
        let b = &self.ambient.data[i * 4..i * 4 + 4];
        let u = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        [uf(u & 0x7ff, 6), uf((u >> 11) & 0x7ff, 6), uf((u >> 22) & 0x3ff, 5)]
    }
    /// Layer texel i, channel c, as unorm (u8 / 255)
    fn unorm(l: &VlmLayer, i: usize, c: usize) -> f32 {
        l.data[i * 4 + c] as f32 / 255.0
    }
    /// The whole AmbientVector layer as RGBA half floats' f32 source (alpha 1), for a 3D texture upload
    pub fn ambient_rgba_f32(&self) -> Vec<[f32; 4]> {
        (0..self.texels()).map(|i| {
            let [r, g, b] = self.ambient_rgb(i);
            [r, g, b, 1.0]
        }).collect()
    }
    /// Shader constants: uvw = P * scale + add (vlm_scale / vlm_add of ue_vlm.gdshaderinc)
    pub fn scale_add(&self) -> ([f64; 3], [f64; 3]) {
        let s = [0, 1, 2].map(|k| 1.0 / (self.bounds_max[k] - self.bounds_min[k]));
        (s, [0, 1, 2].map(|k| -self.bounds_min[k] * s[k]))
    }

    fn tri<F: Fn(usize) -> [f32; 4]>(&self, uvw: [f64; 3], f: F) -> [f32; 4] {
        let [w, h, d] = self.brick_dims;
        let p = [uvw[0] * w as f64 - 0.5, uvw[1] * h as f64 - 0.5, uvw[2] * d as f64 - 0.5];
        let i0 = p.map(|x| x.floor() as i64);
        let fr = [p[0] - i0[0] as f64, p[1] - i0[1] as f64, p[2] - i0[2] as f64];
        let mut out = [0f32; 4];
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    let ix = (i0[0] + dx).clamp(0, w as i64 - 1) as usize;
                    let iy = (i0[1] + dy).clamp(0, h as i64 - 1) as usize;
                    let iz = (i0[2] + dz).clamp(0, d as i64 - 1) as usize;
                    let wt = (if dx == 1 { fr[0] } else { 1.0 - fr[0] }) * (if dy == 1 { fr[1] } else { 1.0 - fr[1] }) * (if dz == 1 { fr[2] } else { 1.0 - fr[2] });
                    let v = f((iz * h + iy) * w + ix);
                    for c in 0..4 {
                        out[c] += wt as f32 * v[c];
                    }
                }
            }
        }
        out
    }

    /// Irradiance (before the 1/pi of the diffuse BRDF) at world point `p` for normal `n`, both UE space: the reference
    /// sampler of vlm_extract.py `irradiance` (indirection lookup, brick UV, trilinear brick fetch, SH3 decode)
    pub fn irradiance(&self, p: [f64; 3], n: [f64; 3]) -> [f64; 3] {
        let (sc, ad) = self.scale_add();
        let [ix, iy, iz] = self.indirection_dims;
        let u = [0, 1, 2].map(|k| (p[k] * sc[k] + ad[k]).clamp(0.0, 0.99) * [ix, iy, iz][k] as f64);
        let cell = ((u[2] as usize * iy + u[1] as usize) * ix + u[0] as usize) * 4;
        let e = &self.indirection.data[cell..cell + 4];
        let bs = self.brick_size as f64;
        let uv = [0, 1, 2].map(|k| {
            let q = u[k] / e[3] as f64;
            (e[k] as f64 * (bs + 1.0) + (q - q.trunc()) * bs + 0.5) / self.brick_dims[k] as f64
        });
        let a4 = self.tri(uv, |i| {
            let [r, g, b] = self.ambient_rgb(i);
            [r, g, b, 0.0]
        });
        let [nx, ny, nz] = n;
        let basis1 = [-1.023328 * ny, 1.023328 * nz, -1.023328 * nx];
        let q = [0.858085 * nx * ny, -0.858085 * ny * nz, 0.247708 * (3.0 * nz * nz - 1.0), -0.858085 * nx * nz];
        let mut res = [0f64; 3];
        for c in 0..3 {
            let s0 = &self.sh[2 * c];
            let s1 = &self.sh[2 * c + 1];
            let t0 = self.tri(uv, |i| [0, 1, 2, 3].map(|k| Self::unorm(s0, i, k)));
            let t1 = self.tri(uv, |i| [0, 1, 2, 3].map(|k| Self::unorm(s1, i, k)));
            let a = a4[c] as f64;
            let c0 = t0.map(|x| (x as f64 * 2.0 - 1.0) * a);
            let c1 = t1.map(|x| (x as f64 * 2.0 - 1.0) * a);
            let band1 = [c0[0] * 1.732051, c0[1] * 1.732051, c0[2] * 1.732051];
            let q4 = [c0[3] * 3.872979, c1[0] * 3.872979, c1[1] * 4.472139, c1[2] * 3.872979];
            let v = 0.886228 * a
                + band1.iter().zip(basis1).map(|(x, y)| x * y).sum::<f64>()
                + q4.iter().zip(q).map(|(x, y)| x * y).sum::<f64>()
                + c1[3] * 3.872979 * 0.429043 * (nx * nx - ny * ny);
            res[c] = v.max(0.0);
        }
        res
    }
}

/// FReflectionCaptureData (UMapBuildDataRegistry.cs:133-188), UE 4.26 cooked
#[derive(Clone, Debug)]
pub struct ReflectionCapture {
    pub cubemap_size: usize,
    pub average_brightness: f32,
    pub brightness: f32,
    /// FullHDRCapturedData: the prefiltered cubemap, FFloat16Color (RGBA half) texels, mip-major then face
    /// (+X, -X, +Y, -Y, +Z, -Z), each face (size >> mip)^2 texels
    pub full_hdr: Bytes,
    /// EncodedCaptureData (mobile; null in Mordhau's PC cook)
    pub encoded: Option<crate::builddata::TexRef>,
}

impl ReflectionCapture {
    pub fn mip_count(&self) -> usize {
        if self.cubemap_size == 0 {
            0
        } else {
            self.cubemap_size.trailing_zeros() as usize + 1
        }
    }
    /// Bytes of one face of one mip (8 bytes per texel), None when out of range
    pub fn face(&self, mip: usize, face: usize) -> Option<&[u8]> {
        let mut off = 0usize;
        for m in 0..self.mip_count() {
            let s = (self.cubemap_size >> m).max(1);
            let face_bytes = s * s * 8;
            if m == mip {
                let a = off + face * face_bytes;
                return self.full_hdr.get(a..a + face_bytes);
            }
            off += 6 * face_bytes;
        }
        None
    }
    /// Size the full mip chain must have
    pub fn expected_len(&self) -> usize {
        (0..self.mip_count()).map(|m| {
            let s = (self.cubemap_size >> m).max(1);
            6 * s * s * 8
        }).sum()
    }
}

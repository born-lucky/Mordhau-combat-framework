//! UE 4.26 precomputed volumetric lightmap (VLM): the level's FPrecomputedVolumetricLightmapData read from its
//! `<Level>_BuiltData` package (MapBuildDataRegistry), the GPU textures/uniforms of shaders/ue_tint.wgsl `ue_vlm`, and a
//! CPU reference of the game's sampling + SH3 irradiance. Port of godot/components/ue/ue_vlm.gd + ue_vlm.gdshaderinc +
//! scripts/map/vlm_extract.py (which read the same data from CUE4Parse's JSON); the math is the game's own shader
//! (state/shaders/INST_SparseGrass_Arena/r0/001_TBasePassPSFPrecomputedVolumetricLightmapLightingPolicySkylight...:
//! 163-177 brick lookup, 201-254 SH3 irradiance, 326-330 x 1/pi x DiffuseColor [+ back x Subsurface]); View constants
//! from SetupPrecomputedVolumetricLightmapUniformBufferParameters (.text 0x2288ae0): WorldToUVScale = 1 / (Max - Min)
//! (0x2288e16), Add = -Min * Scale (0x2288e4c), IndirectionTextureSize, BrickSize (0x2288ea3), BrickTexelSize =
//! 1 / brick atlas size (0x2288eab-0x2288ebb).
//!
//! Layout (CUE4Parse UMapBuildDataRegistry.cs:23-70, 298-390, UE 4.26 versions): UMapBuildDataRegistry = tagged
//! properties, FStripDataFlags, TMap<FGuid, FMeshMapBuildData> MeshBuildData, TMap<FGuid, FPrecomputedLightVolumeData>,
//! TMap<FGuid, FPrecomputedVolumetricLightmapData> (this), TMap<FGuid, FLightComponentMapBuildData>, ...
//! FPrecomputedVolumetricLightmapData: bool bValid, FBox Bounds (6 floats + uint8), FIntVector
//! IndirectionTextureDimensions, layer IndirectionTexture, int32 BrickSize, FIntVector BrickDataDimensions, layers
//! AmbientVector, SHCoefficients[6] (0R 1R 0G 1G 0B 1B), SkyBentNormal, DirectionalLightShadowing, LQLightColor,
//! LQLightDirection (FMobileObjectVersion LQVolumetricLightmapLayers), TArray<FIntVector> SubLevelBrickPositions,
//! TArray<FColor> IndirectionTextureOriginalValues (VolumetricLightmapStreaming). A layer
//! (FVolumetricLightmapDataLayer) = TArray<uint8> Data + FString PixelFormat.
//!
//! Decoding the registry is mh-level's (rust-pak r4, mh_level::vlm::VolumetricLightmap via
//! `mh_level::read(..).volumetric_lightmap()`): copy its fields into a `Vlm` (same names; tests/vlm.rs `from_level` is
//! the conversion; mh-assets does not depend on mh-level, which depends on mh_assets::collision). This module owns the
//! GPU data and the CPU reference of the shader path.

use crate::{err, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub data: Vec<u8>,
    pub format: String,
}

#[derive(Debug, Clone)]
pub struct Vlm {
    /// UE world cm
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub indirection_dims: [i32; 3],
    /// PF_R8G8B8A8_UINT: brick (x, y, z, scale) per indirection cell
    pub indirection: Layer,
    pub brick_size: i32,
    /// brick atlas size in voxels (BrickSize + 1 padding per brick)
    pub brick_dims: [i32; 3],
    /// PF_FloatR11G11B10
    pub ambient: Layer,
    /// PF_R8G8B8A8, order 0R 1R 0G 1G 0B 1B
    pub sh: Vec<Layer>,
    pub sky_bent_normal: Layer,
    pub directional_light_shadowing: Layer,
    pub lq_light_color: Layer,
    pub lq_light_direction: Layer,
    pub sub_level_brick_positions: Vec<[i32; 3]>,
    pub indirection_original: Vec<[u8; 4]>,
}

impl Vlm {
    /// Layer sizes and formats agree with the dimensions (what the shader and `irradiance` index)
    pub fn validate(&self) -> Result<()> {
        let [ix, iy, iz] = self.indirection_dims;
        let [w, h, d] = self.brick_dims;
        let nb = (w as i64 * h as i64 * d as i64) as usize;
        let ok = ix > 0 && iy > 0 && iz > 0 && w > 0 && h > 0 && d > 0
            && self.indirection.data.len() == (ix * iy * iz * 4) as usize
            && self.indirection.format == "PF_R8G8B8A8_UINT"
            && self.ambient.data.len() == nb * 4
            && self.ambient.format == "PF_FloatR11G11B10"
            && self.sh.iter().all(|l| l.data.len() == nb * 4 && l.format == "PF_R8G8B8A8");
        if !ok {
            return err("VLM: layer sizes / formats do not match the dimensions");
        }
        Ok(())
    }

    /// AmbientVector decoded (R11G11B10 floats -> RGBA f32, A = 1), brick voxel order x fastest
    pub fn ambient_f32(&self) -> Vec<[f32; 4]> {
        self.ambient
            .data
            .chunks_exact(4)
            .map(|c| {
                let [r, g, b] = r11g11b10(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                [r, g, b, 1.0]
            })
            .collect()
    }

    /// AmbientVector as RGBA16F texels (exact: R11G11B10 values are representable in half floats)
    pub fn ambient_rgba16f(&self) -> Vec<u16> {
        self.ambient_f32().iter().flat_map(|p| p.map(f32_to_half)).collect()
    }

    /// The six SH layers stacked along Z (layer k at z in [k * D, (k + 1) * D)): one RGBA8 3D texture of
    /// W x H x 6D for ue_tint.wgsl binding 20
    pub fn sh_stack(&self) -> Vec<u8> {
        self.sh.iter().flat_map(|l| l.data.iter().copied()).collect()
    }

    /// UeVlm uniform values (ue_vlm.gd uniforms): [scale, add, ind_size, (brick_size, sh depth D, 0, 0), texel]
    pub fn uniforms(&self) -> [[f32; 4]; 5] {
        let sc = [0, 1, 2].map(|k| 1.0 / (self.bounds_max[k] - self.bounds_min[k]));
        let [ix, iy, iz] = self.indirection_dims.map(|x| x as f32);
        let [w, h, d] = self.brick_dims.map(|x| x as f32);
        [
            [sc[0], sc[1], sc[2], 0.0],
            [-self.bounds_min[0] * sc[0], -self.bounds_min[1] * sc[1], -self.bounds_min[2] * sc[2], 0.0],
            [ix, iy, iz, 0.0],
            [self.brick_size as f32, d, 0.0, 0.0],
            [1.0 / w, 1.0 / h, 1.0 / d, 0.0],
        ]
    }

    /// brick-atlas uvw of UE world position p (cm): ue_vlm_uv (001 163-177)
    pub fn uvw(&self, p: [f32; 3]) -> [f32; 3] {
        let u = self.uniforms();
        let ind = self.indirection_dims;
        let c = [0, 1, 2].map(|k| (p[k] * u[0][k] + u[1][k]).clamp(0.0, 0.99) * ind[k] as f32);
        let ci = c.map(|x| x as usize);
        let o = ((ci[2] * ind[1] as usize + ci[1]) * ind[0] as usize + ci[0]) * 4;
        let e = &self.indirection.data[o..o + 4];
        let bs = self.brick_size as f32;
        [0, 1, 2].map(|k| {
            let q = c[k] / e[3] as f32;
            (e[k] as f32 * (bs + 1.0) + (q - q.floor()) * bs + 0.5) * u[4][k]
        })
    }

    /// Trilinear fetch (texel centres at (i + 0.5) / N, clamp to edge) of a W x H x D RGBA layer
    fn tri(&self, texel: &dyn Fn(usize) -> [f32; 4], uvw: [f32; 3]) -> [f32; 4] {
        let dims = self.brick_dims.map(|x| x as usize);
        let p = [0, 1, 2].map(|k| uvw[k] * dims[k] as f32 - 0.5);
        let i0 = p.map(|x| x.floor());
        let f = [0, 1, 2].map(|k| p[k] - i0[k]);
        let mut out = [0f32; 4];
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    let d = [dx, dy, dz];
                    let ix = [0, 1, 2].map(|k| ((i0[k] as i64 + d[k]).clamp(0, dims[k] as i64 - 1)) as usize);
                    let w = (0..3).map(|k| if d[k] == 1 { f[k] } else { 1.0 - f[k] }).product::<f32>();
                    let t = texel((ix[2] * dims[1] + ix[1]) * dims[0] + ix[0]);
                    for c in 0..4 {
                        out[c] += w * t[c];
                    }
                }
            }
        }
        out
    }

    /// SH3 irradiance (before the 1/pi) at UE position p (cm) for UE-axes unit normal n: ue_vlm_irradiance
    /// (001 201-254), per channel `channel`
    pub fn irradiance(&self, p: [f32; 3], n: [f32; 3]) -> [f32; 3] {
        let uv = self.uvw(p);
        let amb = self.ambient_f32();
        let av = self.tri(&|i| amb[i], uv);
        let unorm = |l: &Layer| -> Vec<[f32; 4]> {
            l.data.chunks_exact(4).map(|t| [0, 1, 2, 3].map(|c| t[c] as f32 / 255.0)).collect()
        };
        [0, 1, 2].map(|c| {
            let (a, b) = (unorm(&self.sh[2 * c]), unorm(&self.sh[2 * c + 1]));
            let s0 = self.tri(&|i| a[i], uv).map(|x| (x * 2.0 - 1.0) * av[c]);
            let s1 = self.tri(&|i| b[i], uv).map(|x| (x * 2.0 - 1.0) * av[c]);
            channel(s0, s1, av[c], n)
        })
    }
}

/// CPU reference of ue_tint.wgsl `ue_vlm`: indirect diffuse to add for a surface `s` at Y-up world position (m) with
/// world normal `nw` (gdshaderinc use_vlm block; 0.31831 = 1/pi; UE AOMultiBounce)
pub fn indirect(v: &Vlm, s: &crate::shader::SurfaceOut, world_pos: [f32; 3], nw: [f32; 3], foliage: bool) -> [f32; 3] {
    let l = (nw[0] * nw[0] + nw[1] * nw[1] + nw[2] * nw[2]).sqrt();
    let vn = [nw[0] / l, nw[2] / l, nw[1] / l];
    let vp = [world_pos[0] * 100.0, world_pos[2] * 100.0, world_pos[1] * 100.0];
    let front = v.irradiance(vp, vn);
    let a = s.ao;
    let mut e = [0f32; 3];
    for k in 0..3 {
        let b = s.base[k];
        let ao = a.max(((a * (2.0404 * b - 0.3324) + (-4.7951 * b + 0.6417)) * a + (2.7552 * b + 0.6903)) * a);
        e[k] = front[k] * 0.31831 * b * (1.0 - s.metallic) * ao;
    }
    if foliage {
        let back = v.irradiance(vp, vn.map(|x| -x));
        for k in 0..3 {
            e[k] += back[k] * 0.31831 * s.backlight[k];
        }
    }
    e
}

/// ue_vlm_channel: one colour channel's SH3 irradiance (001 226-254)
pub fn channel(c0: [f32; 4], c1: [f32; 4], av: f32, n: [f32; 3]) -> f32 {
    let [nx, ny, nz] = n;
    let b1 = [-1.023328 * ny, 1.023328 * nz, -1.023328 * nx];
    let q = [0.858085 * nx * ny, -0.858085 * ny * nz, 0.247708 * (3.0 * nz * nz - 1.0), -0.858085 * nx * nz];
    let q4 = [c0[3] * 3.872979, c1[0] * 3.872979, c1[1] * 4.472139, c1[2] * 3.872979];
    let band1 = (0..3).map(|k| c0[k] * 1.732051 * b1[k]).sum::<f32>();
    let band2 = (0..4).map(|k| q4[k] * q[k]).sum::<f32>();
    (0.886228 * av + band1 + band2 + c1[3] * 3.872979 * 0.429043 * (nx * nx - ny * ny)).max(0.0)
}

/// PF_FloatR11G11B10 (vlm_extract.py r11g11b10): 6/6/5-bit mantissas, 5-bit exponents, bias 15, no sign
pub fn r11g11b10(u: u32) -> [f32; 3] {
    let f = |bits: u32, mbits: u32| -> f32 {
        let e = (bits >> mbits) & 0x1f;
        let m = bits & ((1 << mbits) - 1);
        if e == 0 {
            m as f32 / (1 << mbits) as f32 * 2f32.powi(-14)
        } else {
            (1.0 + m as f32 / (1 << mbits) as f32) * 2f32.powi(e as i32 - 15)
        }
    };
    [f(u & 0x7ff, 6), f((u >> 11) & 0x7ff, 6), f((u >> 22) & 0x3ff, 5)]
}

/// f32 -> IEEE half bits, round to nearest even (normal range; values above 65504 -> inf)
pub fn f32_to_half(x: f32) -> u16 {
    let b = x.to_bits();
    let s = ((b >> 16) & 0x8000) as u16;
    let e = ((b >> 23) & 0xff) as i32;
    let m = b & 0x7f_ffff;
    if e == 255 {
        return s | 0x7c00 | if m != 0 { 0x200 } else { 0 };
    }
    let he = e - 127 + 15;
    if he >= 31 {
        return s | 0x7c00;
    }
    if he <= 0 {
        if he < -10 {
            return s;
        }
        let m = m | 0x80_0000;
        let shift = (14 - he) as u32;
        let mut h = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && h & 1 == 1) {
            h += 1;
        }
        return s | h as u16;
    }
    let mut h = ((he as u32) << 10) | (m >> 13);
    let rem = m & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && h & 1 == 1) {
        h += 1;
    }
    s | h as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::half_to_f32;

    #[test]
    fn r11g11b10_and_half() {
        // 1.0 = exponent 15, mantissa 0 in each channel
        let one = (15u32 << 6) | ((15u32 << 6) << 11) | ((15u32 << 5) << 22);
        assert_eq!(r11g11b10(one), [1.0, 1.0, 1.0]);
        assert_eq!(f32_to_half(1.0), 0x3c00);
        assert_eq!(f32_to_half(-2.0), 0xc000);
        assert_eq!(half_to_f32(f32_to_half(0.333_251_95)), 0.333_251_95);
        assert_eq!(half_to_f32(f32_to_half(5.960_464_5e-8)), 5.960_464_5e-8);
    }

    #[test]
    fn ambient_only_sh_gives_0_886_av() {
        // SH coefficients at 0.5 (= 0 after x2-1) leave the band-0 term: 0.886228 * AV
        assert!((channel([0.0; 4], [0.0; 4], 2.0, [0.0, 0.0, 1.0]) - 1.772456).abs() < 1e-6);
        // band 1 along +Z: c0.y * 1.732051 * 1.023328
        let v = channel([0.0, 0.5, 0.0, 0.0], [0.0; 4], 0.0, [0.0, 0.0, 1.0]);
        assert!((v - 0.5 * 1.732051 * 1.023328).abs() < 1e-6);
    }
}

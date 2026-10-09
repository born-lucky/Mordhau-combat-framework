//! A cooked UE 4.26 Texture2D read from the paks, mip by mip. Port of godot/components/ue/pak/ue_texture.gd. Only the
//! requested mip's bytes are read (`PackageSource::bulk_range` on the .ubulk, or the package data), so a 4K texture
//! never sits in memory whole. `mip_data` returns the mip as stored (block-compressed formats stay compressed, for a
//! GPU upload); `decode` turns a mip into RGBA8 (or RGBA f32 for the HDR formats) for tests and CPU use.
//!
//! Layout after the Texture2D export's tagged properties (CUE4Parse at the pinned commit, tools/CUE4Parse-src/
//! CUE4Parse/UE4/Assets/Exports/Texture/...): UObject Guid (bool + FGuid, UObject.cs:226-236); UTexture
//! FStripDataFlags (2 bytes, UTexture.cs:82); UTexture2D FStripDataFlags (2 bytes) + bool bCooked (UTexture2D.cs:33-34);
//! then per cooked platform format FName PixelFormat + int64 SkipOffset (absolute, header + export data) +
//! FTexturePlatformData, until FName None (UTexture.cs:105-150). FTexturePlatformData (FTexturePlatformData.cs): int32
//! SizeX, SizeY, uint32 PackedData (bit 30 = has FOptTexturePlatformData, 8 bytes), FString PixelFormat, int32
//! FirstMipToSerialize, TArray<FTexture2DMipMap>, bool bIsVirtual. FTexture2DMipMap (FTexture2DMipMap.cs:31-50): bool
//! bCooked, FByteBulkData, int32 SizeX, SizeY, SizeZ, each mip's payload located by `bulk`.

use crate::buf::{half_to_f32, Buf};
use crate::bulk::{self, BulkHeader};
use crate::props::{self, Props};
use crate::{err, skip_object_guid, PackageSource, Result};

/// FTexturePlatformData BitMask_HasOptData (FTexturePlatformData.cs)
pub const HAS_OPT_DATA: u32 = 1 << 30;

/// The EPixelFormat values Mordhau's textures use (ue_texture.gd FORMATS); others are reported, not guessed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Dxt1,
    Dxt3,
    Dxt5,
    Bc4,
    Bc5,
    /// unsigned-float BC6H (Godot FORMAT_BPTC_RGBFU)
    Bc6h,
    Bc7,
    /// 4 bytes per pixel, B G R A in memory
    B8G8R8A8,
    /// 1 byte per pixel, grayscale
    G8,
    /// 4 x f16 per pixel, R G B A
    FloatRgba,
}

impl PixelFormat {
    pub fn from_ue(name: &str) -> Option<Self> {
        Some(match name {
            "PF_DXT1" => Self::Dxt1,
            "PF_DXT3" => Self::Dxt3,
            "PF_DXT5" => Self::Dxt5,
            "PF_BC4" => Self::Bc4,
            "PF_BC5" => Self::Bc5,
            "PF_BC6H" => Self::Bc6h,
            "PF_BC7" => Self::Bc7,
            "PF_B8G8R8A8" => Self::B8G8R8A8,
            "PF_G8" => Self::G8,
            "PF_FloatRGBA" => Self::FloatRgba,
            _ => return None,
        })
    }

    /// bytes per 4x4 block for the block formats, None for the per-pixel ones
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            Self::Dxt1 | Self::Bc4 => Some(8),
            Self::Dxt3 | Self::Dxt5 | Self::Bc5 | Self::Bc6h | Self::Bc7 => Some(16),
            _ => None,
        }
    }

    pub fn is_hdr(self) -> bool {
        matches!(self, Self::Bc6h | Self::FloatRgba)
    }

    /// Bytes a w x h mip holds in this format
    pub fn mip_size(self, w: usize, h: usize) -> usize {
        match self.block_bytes() {
            Some(b) => w.div_ceil(4) * h.div_ceil(4) * b,
            None => match self {
                Self::B8G8R8A8 => w * h * 4,
                Self::G8 => w * h,
                _ => w * h * 8,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mip {
    pub size_x: i32,
    pub size_y: i32,
    pub size_z: i32,
    pub bulk: BulkHeader,
}

#[derive(Debug, Clone)]
pub struct Texture {
    /// package the texture came from (for `mip_data`)
    pub package: String,
    pub export_name: String,
    pub size_x: i32,
    pub size_y: i32,
    pub packed_data: u32,
    /// the EPixelFormat name as stored ("PF_DXT1")
    pub pixel_format_name: String,
    /// None for a format not listed in `PixelFormat` (reported by `mip_data`)
    pub format: Option<PixelFormat>,
    pub first_mip_to_serialize: i32,
    pub mips: Vec<Mip>,
    pub is_virtual: bool,
    /// UTexture SRGB (default true); not applied by the decoders, the material builder reads it
    pub srgb: bool,
    pub properties: Props,
}

/// Texture info of a package's Texture2D export (the first, or the one named `export_name`)
pub fn info(src: &dyn PackageSource, pkg_path: &str, export_name: Option<&str>) -> Result<Texture> {
    let pkg = src.package(pkg_path)?;
    // ULightMapTexture2D / UShadowMapTexture2D (MapBuildData lightmaps) serialize as a UTexture2D first; their own
    // fields (LightmapFlags / ShadowmapFlags) follow the platform data. Added by bevy-runtime r3.
    let e = match pkg
        .find_export("Texture2D", export_name)
        .or_else(|| pkg.find_export("LightMapTexture2D", export_name))
        .or_else(|| pkg.find_export("ShadowMapTexture2D", export_name))
    {
        Some(e) => e,
        None => return err(format!("{}: no Texture2D export", pkg_path)),
    };
    let (start, end) = pkg.export_range(e)?;
    let mut r = Buf::at(&pkg.data, start);
    let props = props::read_tagged(&pkg, &mut r, end);
    skip_object_guid(&mut r, e);
    r.skip(2); // UTexture FStripDataFlags (cooked: editor data stripped, no source art)
    r.skip(2); // UTexture2D FStripDataFlags
    if r.s32() == 0 {
        return err(format!("{}: texture not cooked", pkg_path)); // bCooked
    }
    let mut out: Option<Texture> = None;
    let mut pf = pkg.fname(&mut r);
    while pf != "None" && !r.bad {
        let skip_to = r.s64();
        if out.is_none() {
            let mut t = platform_data(&pkg, &mut r)?;
            if r.p as i64 != skip_to {
                return err(format!("{} {}: platform data ends at {}, SkipOffset {}", pkg.name, e.object_name, r.p, skip_to));
            }
            t.package = pkg.name.clone();
            t.export_name = e.object_name.clone();
            t.srgb = props::get_bool(&props, "SRGB", true);
            t.properties = props.clone();
            out = Some(t);
        }
        if skip_to < 0 {
            return err(format!("{}: SkipOffset {}", pkg.name, skip_to));
        }
        r.p = skip_to as usize;
        pf = pkg.fname(&mut r);
    }
    match out {
        Some(t) if !r.bad => Ok(t),
        _ => err(format!("{}: no cooked platform data", pkg_path)),
    }
}

fn platform_data(pkg: &crate::RawPackage, r: &mut Buf) -> Result<Texture> {
    let size_x = r.s32();
    let size_y = r.s32();
    let packed_data = r.u32();
    let pixel_format_name = r.fstring();
    if packed_data & HAS_OPT_DATA != 0 {
        r.skip(8); // FOptTexturePlatformData {uint32 ExtData, NumMipsInTail}
    }
    let first_mip_to_serialize = r.s32();
    let n = r.s32();
    if !(0..=32).contains(&n) {
        return err(format!("{}: {} mips", pkg.name, n));
    }
    let mut mips = Vec::with_capacity(n as usize);
    for _ in 0..n {
        r.s32(); // bCooked
        let bulk = bulk::header(pkg, r);
        let size_x = r.s32();
        let size_y = r.s32();
        let size_z = r.s32();
        mips.push(Mip { size_x, size_y, size_z, bulk });
    }
    // bIsVirtual: FVirtualTextureBuiltData follows (none in Mordhau's textures)
    let is_virtual = r.s32() != 0;
    if r.bad {
        return err(format!("{}: platform data read past the export", pkg.name));
    }
    Ok(Texture {
        package: String::new(),
        export_name: String::new(),
        size_x,
        size_y,
        packed_data,
        format: PixelFormat::from_ue(&pixel_format_name),
        pixel_format_name,
        first_mip_to_serialize,
        mips,
        is_virtual,
        srgb: true,
        properties: Props::new(),
    })
}

/// Bytes of mip `level` (0 = the largest the paks hold) in the texture's own format, as stored (B8G8R8A8 stays BGRA)
pub fn mip_data(src: &dyn PackageSource, t: &Texture, level: usize) -> Result<Vec<u8>> {
    let fmt = match t.format {
        Some(f) => f,
        None => return err(format!("{}: pixel format {} not supported", t.package, t.pixel_format_name)),
    };
    let m = match t.mips.get(level) {
        Some(m) => m,
        None => return err(format!("{}: mip {} of {}", t.package, level, t.mips.len())),
    };
    let pkg = src.package(&t.package)?;
    let b = bulk::bytes(src, &pkg, &m.bulk)?;
    let want = fmt.mip_size(m.size_x.max(0) as usize, m.size_y.max(0) as usize) * m.size_z.max(1) as usize;
    if b.len() < want {
        return err(format!("{}: mip {} has {} bytes, {} x {} {} needs {}", t.package, level, b.len(), m.size_x,
            m.size_y, t.pixel_format_name, want));
    }
    Ok(b)
}

/// A decoded mip: 8-bit RGBA, or RGBA f32 for the HDR formats (BC6H, FloatRGBA)
#[derive(Debug, Clone, PartialEq)]
pub enum Pixels {
    Rgba8(Vec<u8>),
    RgbaF32(Vec<f32>),
}

/// Decode one mip (`mip_data` bytes) to RGBA, w x h pixels, rows top to bottom. Channel views: B8G8R8A8 -> RGBA;
/// G8 -> (l, l, l, 255) (grayscale, as the exported PNG and Godot's L8 read it); BC4 -> (r, 0, 0, 255) and BC5 ->
/// (r, g, 0, 255) (D3D's view; the exported BC5 PNG's blue is CUE4Parse's reconstructed Z, not stored data);
/// BC6H -> (r, g, b, 1.0) f32; FloatRGBA -> f32.
pub fn decode(fmt: PixelFormat, w: usize, h: usize, data: &[u8]) -> Result<Pixels> {
    if data.len() < fmt.mip_size(w, h) {
        return err(format!("{:?} {}x{}: {} bytes, needs {}", fmt, w, h, data.len(), fmt.mip_size(w, h)));
    }
    match fmt {
        PixelFormat::B8G8R8A8 => {
            let mut o = data[..w * h * 4].to_vec();
            for p in o.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            Ok(Pixels::Rgba8(o))
        }
        PixelFormat::G8 => Ok(Pixels::Rgba8(data[..w * h].iter().flat_map(|&l| [l, l, l, 255]).collect())),
        PixelFormat::FloatRgba => Ok(Pixels::RgbaF32(
            data[..w * h * 8].chunks_exact(2).map(|c| half_to_f32(u16::from_le_bytes([c[0], c[1]]))).collect(),
        )),
        PixelFormat::Bc6h => {
            let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
            let mut out = vec![0f32; w * h * 4];
            let mut blk = [0f32; 16 * 3];
            for by in 0..bh {
                for bx in 0..bw {
                    let o = (by * bw + bx) * 16;
                    bcdec_rs::bc6h_float(&data[o..o + 16], &mut blk, 4 * 3, false);
                    for y in 0..4 {
                        for x in 0..4 {
                            let (px, py) = (bx * 4 + x, by * 4 + y);
                            if px < w && py < h {
                                let d = (py * w + px) * 4;
                                let s = (y * 4 + x) * 3;
                                out[d..d + 3].copy_from_slice(&blk[s..s + 3]);
                                out[d + 3] = 1.0;
                            }
                        }
                    }
                }
            }
            Ok(Pixels::RgbaF32(out))
        }
        _ => {
            let bb = fmt.block_bytes().unwrap();
            let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
            let mut out = vec![0u8; w * h * 4];
            let mut blk = [0u8; 64];
            for by in 0..bh {
                for bx in 0..bw {
                    let o = (by * bw + bx) * bb;
                    let c = &data[o..o + bb];
                    match fmt {
                        PixelFormat::Dxt1 => bcdec_rs::bc1(c, &mut blk, 16),
                        PixelFormat::Dxt3 => bcdec_rs::bc2(c, &mut blk, 16),
                        PixelFormat::Dxt5 => bcdec_rs::bc3(c, &mut blk, 16),
                        PixelFormat::Bc7 => bcdec_rs::bc7(c, &mut blk, 16),
                        PixelFormat::Bc4 => {
                            let mut one = [0u8; 16];
                            bcdec_rs::bc4(c, &mut one, 4, false);
                            for i in 0..16 {
                                blk[i * 4..i * 4 + 4].copy_from_slice(&[one[i], 0, 0, 255]);
                            }
                        }
                        PixelFormat::Bc5 => {
                            let mut two = [0u8; 32];
                            bcdec_rs::bc5(c, &mut two, 8, false);
                            for i in 0..16 {
                                blk[i * 4..i * 4 + 4].copy_from_slice(&[two[i * 2], two[i * 2 + 1], 0, 255]);
                            }
                        }
                        _ => unreachable!(),
                    }
                    for y in 0..4 {
                        let py = by * 4 + y;
                        if py >= h {
                            break;
                        }
                        for x in 0..4 {
                            let px = bx * 4 + x;
                            if px < w {
                                let d = (py * w + px) * 4;
                                let s = (y * 4 + x) * 4;
                                out[d..d + 4].copy_from_slice(&blk[s..s + 4]);
                            }
                        }
                    }
                }
            }
            Ok(Pixels::Rgba8(out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_sizes() {
        assert_eq!(PixelFormat::Dxt1.mip_size(128, 128), 32 * 32 * 8);
        assert_eq!(PixelFormat::Bc7.mip_size(2, 1), 16);
        assert_eq!(PixelFormat::B8G8R8A8.mip_size(171, 3), 171 * 3 * 4);
        assert_eq!(PixelFormat::from_ue("PF_BC5"), Some(PixelFormat::Bc5));
        assert_eq!(PixelFormat::from_ue("PF_ASTC_4x4"), None);
    }

    #[test]
    fn dxt1_solid_block() {
        // color0 = color1 = pure red (RGB565 0xF800), all indices 0
        let blk = [0x00, 0xf8, 0x00, 0xf8, 0, 0, 0, 0];
        match decode(PixelFormat::Dxt1, 4, 4, &blk).unwrap() {
            Pixels::Rgba8(p) => assert!(p.chunks(4).all(|c| c == [255, 0, 0, 255])),
            _ => panic!(),
        }
    }

    #[test]
    fn bgra_swaps_and_g8_replicates() {
        assert_eq!(decode(PixelFormat::B8G8R8A8, 1, 1, &[1, 2, 3, 4]).unwrap(), Pixels::Rgba8(vec![3, 2, 1, 4]));
        assert_eq!(decode(PixelFormat::G8, 1, 1, &[7]).unwrap(), Pixels::Rgba8(vec![7, 7, 7, 255]));
    }
}

//! Minimal Texture2D reader for the level-owned textures this crate decodes itself (landscape heightmaps and
//! weightmaps, PF_B8G8R8A8): the cooked platform data header and mip 0's bytes, zero-copy from the paks. Same layout as
//! `ue_texture.gd` (port of CUE4Parse UTexture2D): UObject Guid, UTexture FStripDataFlags (2 bytes, UTexture.cs:82),
//! UTexture2D FStripDataFlags (2) + bool bCooked (UTexture2D.cs:33-34), then per cooked format FName PixelFormat + int64
//! SkipOffset + FTexturePlatformData {int32 SizeX, SizeY, uint32 PackedData (bit 30 = FOptTexturePlatformData, 8
//! bytes), FString PixelFormat, int32 FirstMipToSerialize, TArray<FTexture2DMipMap>} (UTexture.cs:105-150,
//! FTexturePlatformData.cs); FTexture2DMipMap = bool bCooked, FByteBulkData, int32 SizeX, SizeY, SizeZ
//! (FTexture2DMipMap.cs:31-50). Block-compressed formats (lightmaps) are mh-assets' job (`texture::info/decode`).

use crate::level::Pkgs;
use crate::native::after_props;
use mh_pak::{bulk, Bytes};

#[derive(Clone, Debug)]
pub struct Tex {
    pub pixel_format: String,
    pub width: usize,
    pub height: usize,
    /// mip 0 bytes as stored
    pub data: Bytes,
}

/// Mip 0 of Texture2D export `export` of package `pkg`
pub fn mip0(pk: &Pkgs, pkg: &str, export: usize) -> Option<Tex> {
    let a = pk.rd.open(pkg)?;
    let (mut r, end) = after_props(pk, &a, export)?;
    r.skip(4); // UTexture + UTexture2D FStripDataFlags
    if r.s32() == 0 {
        return None; // not cooked
    }
    let pf = a.fname(&mut r);
    if pf == "None" {
        return None;
    }
    r.s64(); // SkipOffset
    let (sx, sy, packed) = (r.s32(), r.s32(), r.u32());
    let fmt = r.fstring();
    if packed & (1 << 30) != 0 {
        r.skip(8);
    }
    r.s32(); // FirstMipToSerialize
    let n = r.s32();
    if !(1..=32).contains(&n) {
        return None;
    }
    r.s32(); // bCooked
    let h = bulk::header(&a, &mut r);
    let (w, hh) = (r.s32(), r.s32());
    r.s32();
    if r.bad || r.p > end || w <= 0 || hh <= 0 {
        return None;
    }
    let _ = (sx, sy);
    let data = bulk::bytes(&pk.rd.vfs, &a, &h)?;
    Some(Tex { pixel_format: fmt, width: w as usize, height: hh as usize, data })
}

/// "pkg.N" -> (pkg, N)
pub fn split_ref(op: &str) -> Option<(&str, usize)> {
    let (p, i) = op.rsplit_once('.')?;
    Some((p, i.parse().ok()?))
}

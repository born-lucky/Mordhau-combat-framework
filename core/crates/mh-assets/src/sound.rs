//! A cooked UE 4.26 SoundWave read from the paks: its compressed audio (Mordhau cooks OGG Vorbis) as bytes. Port of
//! godot/components/ue/pak/ue_sound_wave.gd; the bytes are a complete .ogg file, byte-identical to what `mdx export`
//! wrote to extract/gltf (tests/assets.rs), for any Vorbis decoder in the host.
//!
//! Layout after the SoundWave export's tagged properties (CUE4Parse USoundWave.cs:21-80 at the pinned commit): UObject
//! Guid (bool + FGuid, UObject.cs:226-236); uint32 ESoundWaveFlag (bit 0 = cooked, USoundWave.cs:129-135); then, for a
//! non-streamed wave, FFormatContainer (FFormatContainer.cs: int32 count, per format FName + FByteBulkData) and FGuid
//! CompressedDataGuid. Streamed waves (bStreaming; FStreamedAudioPlatformData chunks) are not read: none of the sounds
//! the port loads is streamed.

use crate::buf::Buf;
use crate::bulk::{self, BulkHeader};
use crate::props::{self, Props};
use crate::{err, skip_object_guid, PackageSource, Result};

/// ESoundWaveFlag::CookedFlag
pub const COOKED_FLAG: u32 = 1 << 0;

#[derive(Debug, Clone)]
pub struct SoundWave {
    pub package: String,
    pub properties: Props,
    /// (format name, payload) per cooked format, in stored order ("OGG10000-1-1-1-1-1" style names)
    pub formats: Vec<(String, BulkHeader)>,
}

pub fn info(src: &dyn PackageSource, pkg_path: &str) -> Result<SoundWave> {
    let pkg = src.package(pkg_path)?;
    let e = match pkg.find_export("SoundWave", None) {
        Some(e) => e,
        None => return err(format!("{}: no SoundWave export", pkg_path)),
    };
    let (start, end) = pkg.export_range(e)?;
    let mut r = Buf::at(&pkg.data, start);
    let props = props::read_tagged(&pkg, &mut r, end);
    if props::get_bool(&props, "bStreaming", false) {
        return err(format!("{}: streamed wave (FStreamedAudioPlatformData not implemented)", pkg_path));
    }
    skip_object_guid(&mut r, e);
    if r.u32() & COOKED_FLAG == 0 {
        return err(format!("{}: not cooked", pkg_path));
    }
    let n = r.s32();
    if !(0..=16).contains(&n) {
        return err(format!("{}: {} formats", pkg_path, n));
    }
    let mut formats = Vec::new();
    for _ in 0..n {
        let f = pkg.fname(&mut r);
        formats.push((f, bulk::header(&pkg, &mut r)));
    }
    if r.bad {
        return err(format!("{}: format container read past the export", pkg_path));
    }
    Ok(SoundWave { package: pkg.name.clone(), properties: props, formats })
}

/// The compressed bytes of the first format whose name starts with `prefix` ("OGG" = Vorbis, as cooked for Windows)
pub fn compressed(src: &dyn PackageSource, pkg_path: &str, prefix: &str) -> Result<Vec<u8>> {
    let w = info(src, pkg_path)?;
    for (f, h) in &w.formats {
        if f.starts_with(prefix) {
            let pkg = src.package(&w.package)?;
            return bulk::bytes(src, &pkg, h);
        }
    }
    err(format!("{}: no {} format", pkg_path, prefix))
}

/// The OGG Vorbis file of a wave
pub fn ogg(src: &dyn PackageSource, pkg_path: &str) -> Result<Vec<u8>> {
    compressed(src, pkg_path, "OGG")
}

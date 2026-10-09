//! FByteBulkData, UE 4.26: the out-of-line payloads (texture mips, sound formats, mesh buffers, anim byte streams) of a
//! package's exports. Port of godot/components/ue/pak/ue_bulk.gd.
//!
//! Header (CUE4Parse FByteBulkDataHeader.cs:72-112): uint32 BulkDataFlags, ElementCount and SizeOnDisk (int32/uint32,
//! int64 with BULKDATA_Size64Bit), int64 OffsetInFile (+ the summary's BulkDataStartOffset unless
//! BULKDATA_NoOffsetFixUp). Where the payload is (TBulkData.cs:105-190): inline after the header
//! (BULKDATA_ForceInlinePayload), in <package>.ubulk (BULKDATA_PayloadInSeperateFile; .uptnl with
//! BULKDATA_OptionalPayload), or at OffsetInFile in the package itself (BULKDATA_PayloadAtEndOfFile). Payloads in a
//! .ubulk are read with `PackageSource::bulk_range`: never a whole .ubulk.

use crate::buf::Buf;
use crate::{err, PackageSource, RawPackage, Result};

// EBulkDataFlags (EBulkDataFlags.cs:7-31)
pub const BULKDATA_PAYLOAD_AT_END_OF_FILE: u32 = 1 << 0;
pub const BULKDATA_SERIALIZE_COMPRESSED_ZLIB: u32 = 1 << 1;
pub const BULKDATA_UNUSED: u32 = 1 << 5;
pub const BULKDATA_FORCE_INLINE_PAYLOAD: u32 = 1 << 6;
pub const BULKDATA_PAYLOAD_IN_SEPERATE_FILE: u32 = 1 << 8;
pub const BULKDATA_OPTIONAL_PAYLOAD: u32 = 1 << 11;
pub const BULKDATA_SIZE_64BIT: u32 = 1 << 13;
pub const BULKDATA_DUPLICATE_NON_OPTIONAL_PAYLOAD: u32 = 1 << 14;
pub const BULKDATA_BAD_DATA_VERSION: u32 = 1 << 15;
pub const BULKDATA_NO_OFFSET_FIX_UP: u32 = 1 << 16;

/// A located payload (the bytes are not read)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BulkHeader {
    pub flags: u32,
    pub count: i64,
    pub size: i64,
    /// OffsetInFile, fixed up by BulkDataStartOffset unless BULKDATA_NoOffsetFixUp (offset in the package data)
    pub offset: i64,
    /// offset of an inline payload in the package data
    pub inline: Option<usize>,
    /// companion file ("<package>.ubulk" / ".uptnl") and the payload's offset in it
    pub file: Option<(String, i64)>,
}

/// FByteBulkData header at the cursor; the cursor moves past it (and past an inline payload)
pub fn header(pkg: &RawPackage, r: &mut Buf) -> BulkHeader {
    let flags = r.u32();
    let b64 = flags & BULKDATA_SIZE_64BIT != 0;
    let count = if b64 { r.s64() } else { r.s32() as i64 };
    let size = if b64 { r.s64() } else { r.u32() as i64 };
    let raw = r.s64();
    let offset = if flags & BULKDATA_NO_OFFSET_FIX_UP != 0 { raw } else { raw + pkg.bulk_data_start };
    if flags & BULKDATA_BAD_DATA_VERSION != 0 {
        r.skip(2);
    }
    if flags & BULKDATA_DUPLICATE_NON_OPTIONAL_PAYLOAD != 0 {
        r.skip(4 + if b64 { 8 } else { 4 } + 8);
    }
    let mut h = BulkHeader { flags, count, size, offset, inline: None, file: None };
    if flags & BULKDATA_FORCE_INLINE_PAYLOAD != 0 {
        h.inline = Some(r.p);
        r.skip(size.max(0) as usize);
    }
    if flags & BULKDATA_PAYLOAD_IN_SEPERATE_FILE != 0 {
        // the .ubulk holds what would follow BulkDataStartOffset, so a fixed-up offset is relative to it there
        let ext = if flags & BULKDATA_OPTIONAL_PAYLOAD != 0 { ".uptnl" } else { ".ubulk" };
        let fo = if flags & BULKDATA_NO_OFFSET_FIX_UP != 0 { offset } else { offset - pkg.bulk_data_start };
        h.file = Some((format!("{}{}", pkg.name, ext), fo));
    }
    h
}

/// The payload of a header (only those bytes); empty when there is none
pub fn bytes(src: &dyn PackageSource, pkg: &RawPackage, h: &BulkHeader) -> Result<Vec<u8>> {
    if h.size == 0 || h.flags & BULKDATA_UNUSED != 0 {
        return Ok(Vec::new());
    }
    if h.size < 0 {
        return err(format!("{}: bulk data size {}", pkg.name, h.size));
    }
    if h.flags & BULKDATA_SERIALIZE_COMPRESSED_ZLIB != 0 {
        return err(format!("{}: zlib bulk data not implemented (none in Mordhau)", pkg.name));
    }
    let n = h.size as usize;
    let slice = |o: usize| -> Result<Vec<u8>> {
        match pkg.data.get(o..o.saturating_add(n)) {
            Some(s) => Ok(s.to_vec()),
            None => err(format!("{}: bulk payload [{}, +{}) beyond the package data", pkg.name, o, n)),
        }
    };
    if let Some(o) = h.inline {
        return slice(o);
    }
    if let Some((f, fo)) = &h.file {
        if *fo < 0 {
            return err(format!("{}: negative offset {} in {}", pkg.name, fo, f));
        }
        let b = src.bulk_range(f, *fo as u64, h.size as u64)?;
        if b.len() != n {
            return err(format!("{}: read {} of {} bytes from {}", pkg.name, b.len(), n, f));
        }
        return Ok(b);
    }
    if h.offset < 0 {
        return err(format!("{}: negative bulk offset {}", pkg.name, h.offset));
    }
    slice(h.offset as usize)
}

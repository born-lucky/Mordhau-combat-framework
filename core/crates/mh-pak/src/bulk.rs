//! FByteBulkData, UE 4.26: the out-of-line payloads (texture mips, sound formats, mesh LODs) of a package's exports.
//! Port of `ue_bulk.gd` `UeBulk`.
//! Header (CUE4Parse FByteBulkDataHeader.cs:72-112): uint32 BulkDataFlags, ElementCount and SizeOnDisk (int32/uint32,
//! int64 with BULKDATA_Size64Bit), int64 OffsetInFile (+ the summary's BulkDataStartOffset unless
//! BULKDATA_NoOffsetFixUp). Where the payload is (TBulkData.cs:105-190): inline after the header
//! (BULKDATA_ForceInlinePayload), in <package>.ubulk (BULKDATA_PayloadInSeperateFile; .uptnl with
//! BULKDATA_OptionalPayload), or at OffsetInFile in the package itself (BULKDATA_PayloadAtEndOfFile). Payloads are read
//! with `Vfs::read_range`: a view of only those bytes, never a whole .ubulk.

use crate::asset::Package;
use crate::buf::Cursor;
use crate::pak::Bytes;
use crate::vfs::Vfs;
use std::sync::Arc;

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

/// A located (not read) bulk payload
#[derive(Clone, Debug, PartialEq)]
pub struct BulkHeader {
    pub flags: u32,
    pub count: i64,
    pub size: i64,
    /// OffsetInFile, fixed up by BulkDataStartOffset unless NoOffsetFixUp
    pub offset: i64,
    /// position of an inline payload in the package's data, else None
    pub inline: Option<i64>,
    /// separate file (.ubulk/.uptnl) and the offset inside it
    pub file: Option<(String, i64)>,
}

/// FByteBulkData header at the cursor; the cursor ends after the header (and after an inline payload)
pub fn header(a: &Package, r: &mut Cursor) -> BulkHeader {
    let flags = r.u32();
    let b64 = flags & BULKDATA_SIZE_64BIT != 0;
    let count = if b64 { r.s64() } else { r.s32() as i64 };
    let size = if b64 { r.s64() } else { r.u32() as i64 };
    let raw = r.s64();
    let offset = if flags & BULKDATA_NO_OFFSET_FIX_UP != 0 { raw } else { raw + a.bulk_data_start };
    if flags & BULKDATA_BAD_DATA_VERSION != 0 {
        r.skip(2);
    }
    if flags & BULKDATA_DUPLICATE_NON_OPTIONAL_PAYLOAD != 0 {
        r.skip(4 + if b64 { 8 } else { 4 } + 8);
    }
    let mut inline = None;
    if flags & BULKDATA_FORCE_INLINE_PAYLOAD != 0 {
        inline = Some(r.p);
        r.skip(size);
    }
    let mut file = None;
    if flags & BULKDATA_PAYLOAD_IN_SEPERATE_FILE != 0 {
        // the .ubulk holds what would follow BulkDataStartOffset, so a fixed-up offset is relative to it there
        let ext = if flags & BULKDATA_OPTIONAL_PAYLOAD != 0 { ".uptnl" } else { ".ubulk" };
        let fo = if flags & BULKDATA_NO_OFFSET_FIX_UP == 0 { offset - a.bulk_data_start } else { offset };
        file = Some((format!("{}{ext}", a.name), fo));
    }
    BulkHeader { flags, count, size, offset, inline, file }
}

/// The payload of a header, read from the paks (only those bytes); None when there is none or it is out of range.
/// Zero-copy unless the range straddles the package's .uasset/.uexp seam.
pub fn bytes(vfs: &Vfs, a: &Package, m: &BulkHeader) -> Option<Bytes> {
    if m.size <= 0 || m.flags & BULKDATA_UNUSED != 0 {
        return None;
    }
    if m.flags & BULKDATA_SERIALIZE_COMPRESSED_ZLIB != 0 {
        return None; // zlib bulk data not implemented (none in Mordhau)
    }
    let in_pkg = |at: i64| -> Option<Bytes> {
        let (h, n) = (a.uasset.len() as i64, m.size);
        if at < 0 || at + n > h + a.uexp.len() as i64 {
            return None;
        }
        if at >= h {
            return sub(&a.uexp, (at - h) as usize, n as usize);
        }
        if at + n <= h {
            return sub(&a.uasset, at as usize, n as usize);
        }
        Some(Bytes::Owned(Arc::from(a.slice(at as usize, (at + n) as usize)?.into_owned())))
    };
    if let Some(at) = m.inline {
        return in_pkg(at);
    }
    if let Some((f, fo)) = &m.file {
        if *fo < 0 {
            return None;
        }
        return vfs.read_range(f, *fo as u64, m.size as u64);
    }
    in_pkg(m.offset)
}

fn sub(b: &Bytes, at: usize, n: usize) -> Option<Bytes> {
    if at + n > b.len() {
        return None;
    }
    Some(match b {
        Bytes::Mapped(m, r) => Bytes::Mapped(m.clone(), r.start + at..r.start + at + n),
        Bytes::Owned(v) => Bytes::Owned(Arc::from(&v[at..at + n])),
    })
}

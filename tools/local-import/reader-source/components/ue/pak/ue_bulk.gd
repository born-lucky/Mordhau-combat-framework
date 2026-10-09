# ue_bulk.gd - FByteBulkData, UE 4.26: the out-of-line payloads (texture mips, sound formats) of a package's exports.
# Header (CUE4Parse FByteBulkDataHeader.cs:72-112): uint32 BulkDataFlags, ElementCount and SizeOnDisk (int32/uint32,
# int64 with BULKDATA_Size64Bit), int64 OffsetInFile (+ the summary's BulkDataStartOffset unless BULKDATA_NoOffsetFixUp).
# Where the payload is (TBulkData.cs:105-190): inline after the header (BULKDATA_ForceInlinePayload), in
# <package>.ubulk (BULKDATA_PayloadInSeperateFile; .uptnl with BULKDATA_OptionalPayload), or at OffsetInFile in the
# package itself (BULKDATA_PayloadAtEndOfFile). Payloads are read with UePakVfs.read_range: never a whole .ubulk.
class_name UeBulk

# EBulkDataFlags (EBulkDataFlags.cs:7-31)
const BULKDATA_PAYLOAD_AT_END_OF_FILE := 1 << 0
const BULKDATA_SERIALIZE_COMPRESSED_ZLIB := 1 << 1
const BULKDATA_UNUSED := 1 << 5
const BULKDATA_FORCE_INLINE_PAYLOAD := 1 << 6
const BULKDATA_PAYLOAD_IN_SEPERATE_FILE := 1 << 8
const BULKDATA_OPTIONAL_PAYLOAD := 1 << 11
const BULKDATA_SIZE_64BIT := 1 << 13
const BULKDATA_DUPLICATE_NON_OPTIONAL_PAYLOAD := 1 << 14
const BULKDATA_BAD_DATA_VERSION := 1 << 15
const BULKDATA_NO_OFFSET_FIX_UP := 1 << 16
# FByteBulkData header at r: {flags, count, size, offset, inline, [file, file_offset]}; the payload is located, not read
static func header(a: UeAsset, r: UePakBuf) -> Dictionary:
	var flags := r.u32()
	var b64 := (flags & BULKDATA_SIZE_64BIT) != 0
	var m := {"flags": flags}
	m.count = r.s64() if b64 else r.s32()
	m.size = r.s64() if b64 else r.u32()
	var raw := r.s64()
	m.offset = raw if flags & BULKDATA_NO_OFFSET_FIX_UP else raw + a.bulk_data_start
	if flags & BULKDATA_BAD_DATA_VERSION:
		r.skip(2)
	if flags & BULKDATA_DUPLICATE_NON_OPTIONAL_PAYLOAD:
		r.skip(4 + (8 if b64 else 4) + 8)
	m.inline = -1
	if flags & BULKDATA_FORCE_INLINE_PAYLOAD:
		m.inline = r.p
		r.skip(m.size)
	if flags & BULKDATA_PAYLOAD_IN_SEPERATE_FILE:
		# the .ubulk holds what would follow BulkDataStartOffset, so a fixed-up offset is relative to it there
		m.file = a.name + (".uptnl" if flags & BULKDATA_OPTIONAL_PAYLOAD else ".ubulk")
		m.file_offset = m.offset - a.bulk_data_start if not (flags & BULKDATA_NO_OFFSET_FIX_UP) else m.offset
	return m

# The payload of a header() result, read from the paks (only those bytes); empty when there is none
static func bytes(pkg_path: String, m: Dictionary) -> PackedByteArray:
	var flags: int = m.flags
	if m.size == 0 or flags & BULKDATA_UNUSED:
		return PackedByteArray()
	var b := PackedByteArray()
	if m.inline >= 0:
		b = UeAsset.open(pkg_path).data().slice(m.inline, m.inline + m.size)
	elif m.has("file"):
		b = UePakVfs.read_range(m.file + "", m.file_offset, m.size)
	else:
		b = UeAsset.open(pkg_path).data().slice(m.offset, m.offset + m.size)
	if flags & BULKDATA_SERIALIZE_COMPRESSED_ZLIB:
		push_error("UeBulk: zlib bulk data in %s not implemented (none in Mordhau)" % pkg_path)
		return PackedByteArray()
	return b

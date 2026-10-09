# ue_pak.gd - read the game's own .pak files (UE 4.26 FPakFile), streaming: only the footer, the index and the
# requested entries are read (FileAccess seek + get_buffer); a pak is never loaded whole. Format notes, with the
# sources each field comes from, are in docs/PAK_FORMAT.md.
#
# UE 4.26 names: FPakInfo (Runtime/PakFile/Public/IPlatformFilePak.h), FPakFile::LoadIndex / FPakFile::DecodePakEntry
# (Runtime/PakFile/Private/IPlatformFilePak.cpp). Byte layout cross-checked against CUE4Parse
# (tools/CUE4Parse-src/CUE4Parse/UE4/Pak/Objects/FPakInfo.cs, Pak/PakFileReader.cs, Pak/Objects/FPakEntry.cs).
# Mordhau build 702625635: 44 paks, all version 11 (PakFile_Version_Fnv64BugFix), no compression method named in any
# footer, index not encrypted, 81,973 entries all stored uncompressed (test_pak).
class_name UePak

const MAGIC := 0x5A6F12E1					# FPakInfo::PakFile_Magic (FPakInfo.cs:40)
const VERSION_PATH_HASH_INDEX := 10		# EPakFileVersion (FPakInfo.cs:26-27): 10 = PathHashIndex, 11 = Fnv64BugFix
const COMPRESSION_NAME_LEN := 32			# FPakInfo::CompressionMethodNameLen (FPakInfo.cs:59)
const MAX_COMPRESSION_METHODS := 5		# 4.26 footer holds 5 names (FPakInfo.cs:492-507, default for 4.26 sizes)
# Footer = Guid(16) + bEncryptedIndex(1) + Magic(4) + Version(4) + IndexOffset(8) + IndexSize(8) + IndexHash(20)
#          + 5 * 32 compression method names (FPakInfo.cs:366-434, 509-530) = 221 bytes for versions 8..11
const FOOTER_SIZE := 16 + 1 + 4 + 4 + 8 + 8 + 20 + MAX_COMPRESSION_METHODS * COMPRESSION_NAME_LEN
# Each stored file is preceded by a copy of its FPakEntry: Offset, Size, UncompressedSize (int64 x3),
# CompressionMethodIndex (uint32), Hash (SHA1, 20), [blocks], Flags (uint8), CompressionBlockSize (uint32)
# (FPakEntry.cs:265-269 StructSize; FPakEntry.cs:41-153 serialized form).
const ENTRY_HEADER_SIZE := 8 * 3 + 4 + 20 + 1 + 4

var path := ""
var version := 0
var mount_point := ""			# e.g. "../../../Mordhau/Content/"
var encrypted_index := false
var compression := PackedStringArray(["None"])	# index 0 = none, then the footer's names (FPakInfo.cs:515-530)
var entry_count := 0
var encoded := PackedByteArray()	# FPakFile::EncodedPakEntries: bit-packed entries, decoded on read
var nonencoded: Array = []		# entries that did not fit the encoding (decoded FPakEntry dicts)
var error := ""

# Open a pak and parse its index. `files` receives "<mounted path>" -> Vector2i(slot, entry) where entry >= 0 is a byte
# offset into `encoded` and entry < 0 is -(index+1) into `nonencoded` (the same convention as the directory index,
# PakFileReader.cs:449-466). Returns false and sets `error` on anything this reader does not support.
func open(p: String, files: Dictionary, slot: int) -> bool:
	path = p
	var f := FileAccess.open(p, FileAccess.READ)
	if f == null:
		error = "cannot open " + p
		return false
	var flen := f.get_length()
	f.seek(flen - FOOTER_SIZE)
	var foot := f.get_buffer(FOOTER_SIZE)
	var r := UePakBuf.new(foot)
	r.skip(16)							# EncryptionKeyGuid
	encrypted_index = r.u8() != 0
	if r.u32() != MAGIC:
		error = "bad magic"
		return false
	version = r.s32()
	var index_offset := r.s64()
	var index_size := r.s64()
	r.skip(20)							# IndexHash
	for i in MAX_COMPRESSION_METHODS:
		var nm := foot.slice(r.p, r.p + COMPRESSION_NAME_LEN)
		r.skip(COMPRESSION_NAME_LEN)
		var z := nm.find(0)
		var s := (nm if z < 0 else nm.slice(0, z)).get_string_from_ascii()
		if s != "":
			compression.append(s)
	if encrypted_index:
		error = "encrypted index (needs the game's AES key; Mordhau's paks are not encrypted)"
		return false
	if version < VERSION_PATH_HASH_INDEX:
		error = "pak version %d: only the 4.25+ path-hash/directory index (v10+) is implemented" % version
		return false
	# Primary index (PakFileReader.cs:330-397 ReadIndexUpdated; UE FPakFile::LoadIndexInternal)
	f.seek(index_offset)
	r = UePakBuf.new(f.get_buffer(index_size))
	mount_point = r.fstring()
	entry_count = r.s32()
	r.skip(8)							# PathHashSeed
	if r.s32() != 0:					# bReaderHasPathHashIndex: offset, size, hash (unused here)
		r.skip(8 + 8 + 20)
	if r.s32() == 0:					# bReaderHasFullDirectoryIndex
		error = "no full directory index"
		return false
	var dir_offset := r.s64()
	var dir_size := r.s64()
	r.skip(20)
	var enc_size := r.s32()
	encoded = r.bytes(enc_size)
	var n_nonenc := r.s32()
	for i in n_nonenc:
		nonencoded.append(_read_entry(r))
	# FDirectoryIndex: TMap<FString dir, TMap<FString file, int32 entry>> (PakFileReader.cs:425-470)
	f.seek(dir_offset)
	var d := UePakBuf.new(f.get_buffer(dir_size))
	var trim := mount_point.ends_with("/")
	var nd := d.s32()
	for i in nd:
		var dir := d.fstring()
		if trim and dir.begins_with("/"):
			dir = dir.substr(1)
		var nf := d.s32()
		for j in nf:
			var fname := d.fstring()
			var loc := d.s32()
			if loc == -2147483648:			# int.MinValue = deleted record (PakFileReader.cs:450)
				continue
			files[mount_point + dir + fname] = Vector2i(slot, loc)
	return true

# Non-encoded FPakEntry as serialized in the index (FPakEntry.cs:41-153, version >= 8)
func _read_entry(r: UePakBuf) -> Dictionary:
	var e := {}
	e.offset = r.s64()
	e.csize = r.s64()
	e.usize = r.s64()
	e.method = r.u32()
	r.skip(20)
	e.blocks = PackedInt64Array()
	if e.method != 0:
		var nb := r.s32()
		for i in nb:
			e.blocks.append(r.s64() + e.offset)		# relative since PakFile_Version_RelativeChunkOffsets (FPakEntry.cs:137-145)
			e.blocks.append(r.s64() + e.offset)
	e.encrypted = (r.u8() & 1) != 0
	e.block_size = r.u32()
	return e

# FPakFile::DecodePakEntry (FPakEntry.cs:159-302). Bitfield: [31] offset fits 32 bits, [30] uncompressed size fits,
# [29] size fits, [28..23] compression method index, [22] encrypted, [21..6] block count, [5..0] block size / 2048
# (0x3f = an explicit uint32 follows).
func entry(loc: int) -> Dictionary:
	if loc < 0:
		return nonencoded[-loc - 1]
	var r := UePakBuf.new(encoded)
	r.p = loc
	var bf := r.u32()
	var block_size := r.u32() if (bf & 0x3f) == 0x3f else (bf & 0x3f) << 11
	var e := {}
	e.method = (bf >> 23) & 0x3f
	e.offset = r.u32() if (bf & (1 << 31)) != 0 else r.s64()
	e.usize = r.u32() if (bf & (1 << 30)) != 0 else r.s64()
	e.csize = e.usize
	if e.method != 0:
		e.csize = r.u32() if (bf & (1 << 29)) != 0 else r.s64()
	e.encrypted = ((bf >> 22) & 1) != 0
	var nb: int = (bf >> 6) & 0xffff
	e.block_size = e.usize if nb == 1 else block_size
	var hdr: int = ENTRY_HEADER_SIZE + (4 + nb * 16 if e.method != 0 else 0)
	var start: int = e.offset + hdr
	e.blocks = PackedInt64Array()
	if nb == 1 and not e.encrypted:
		e.blocks.append(start); e.blocks.append(start + e.csize)
	elif nb > 0:
		for i in nb:
			var ln := r.u32()
			e.blocks.append(start); e.blocks.append(start + ln)
			start += ln			# no AES alignment: encrypted entries are refused in read()
	e.header = hdr
	return e

# Bytes of one entry. Uncompressed and zlib entries only: Oodle is a proprietary codec with no redistributable
# decoder, and no Mordhau entry uses any compression (docs/PAK_FORMAT.md). `verify` checks the SHA1 the pak stores
# in the entry's header copy (FPakEntry Hash) against the bytes read.
func read(f: FileAccess, loc: int, verify := false) -> PackedByteArray:
	var e := entry(loc)
	if e.encrypted:
		push_error("UePak: encrypted entry in " + path)
		return PackedByteArray()
	var method: String = compression[e.method] if e.method < compression.size() else "?"
	f.seek(e.offset)
	var head := f.get_buffer(ENTRY_HEADER_SIZE)
	var hr := UePakBuf.new(head)
	hr.skip(8)
	if hr.s64() != e.csize or hr.s64() != e.usize:		# the header copy must agree with the index
		push_error("UePak: entry header mismatch at %d in %s" % [e.offset, path])
		return PackedByteArray()
	var sha := head.slice(28, 48)
	var out := PackedByteArray()
	if e.method == 0:
		f.seek(e.offset + e.header)
		out = f.get_buffer(e.usize)
	elif method.to_lower() == "zlib":
		# FCompression zlib: each block is a zlib stream of up to block_size bytes (UNTESTED: no zlib entry in Mordhau)
		var left: int = e.usize
		for i in range(0, e.blocks.size(), 2):
			f.seek(e.blocks[i])
			var blk := f.get_buffer(e.blocks[i + 1] - e.blocks[i])
			var n: int = mini(left, e.block_size)
			out.append_array(blk.decompress(n, FileAccess.COMPRESSION_DEFLATE))
			left -= n
	else:
		push_error("UePak: compression '%s' not supported (%s)" % [method, path])
		return PackedByteArray()
	if verify and e.method == 0:
		var h := HashingContext.new()
		h.start(HashingContext.HASH_SHA1)
		h.update(out)
		if h.finish() != sha:
			push_error("UePak: SHA1 mismatch at %d in %s" % [e.offset, path])
			return PackedByteArray()
	return out

# `size` bytes at `start` inside an uncompressed entry, without reading the rest (texture mips and sound chunks live in
# .ubulk files of up to tens of MB; only the requested slice is read). Empty when out of range or compressed.
func read_range(f: FileAccess, loc: int, start: int, size: int) -> PackedByteArray:
	var e := entry(loc)
	if e.encrypted or e.method != 0 or start < 0 or size < 0 or start + size > e.usize:
		push_error("UePak: range %d+%d not readable in entry at %d of %s" % [start, size, e.offset, path])
		return PackedByteArray()
	f.seek(e.offset + e.header + start)
	return f.get_buffer(size)

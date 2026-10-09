//! One `.pak` (UE 4.26 FPakFile), port of `ue_pak.gd` `UePak`. The file is memory-mapped read-only: only the
//! footer, the index and the entries asked for are ever touched, and entry bytes are views into the mapping
//! (zero-copy). Format notes with the source of each field: docs/PAK_FORMAT.md.
//!
//! UE 4.26 names: FPakInfo (Runtime/PakFile/Public/IPlatformFilePak.h), FPakFile::LoadIndex / FPakFile::DecodePakEntry
//! (Runtime/PakFile/Private/IPlatformFilePak.cpp). Byte layout cross-checked against CUE4Parse
//! (tools/CUE4Parse-src/CUE4Parse/UE4/Pak/Objects/FPakInfo.cs, Pak/PakFileReader.cs, Pak/Objects/FPakEntry.cs).
//! Mordhau build 702625635: 44 paks, all version 11 (PakFile_Version_Fnv64BugFix), no compression method named in any
//! footer, index not encrypted, 81,973 entries all stored uncompressed (tests/pak.rs).

use crate::buf::Cursor;
use memmap2::Mmap;
use std::fs::File;
use std::ops::{Deref, Range};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MAGIC: u32 = 0x5A6F12E1; // FPakInfo::PakFile_Magic (FPakInfo.cs:40)
pub const VERSION_PATH_HASH_INDEX: i32 = 10; // EPakFileVersion (FPakInfo.cs:26-27): 10 = PathHashIndex, 11 = Fnv64BugFix
pub const COMPRESSION_NAME_LEN: usize = 32; // FPakInfo::CompressionMethodNameLen (FPakInfo.cs:59)
pub const MAX_COMPRESSION_METHODS: usize = 5; // 4.26 footer holds 5 names (FPakInfo.cs:492-507)
/// Guid(16) + bEncryptedIndex(1) + Magic(4) + Version(4) + IndexOffset(8) + IndexSize(8) + IndexHash(20)
/// + 5 * 32 compression method names (FPakInfo.cs:366-434, 509-530) = 221 bytes for versions 8..11
pub const FOOTER_SIZE: usize = 16 + 1 + 4 + 4 + 8 + 8 + 20 + MAX_COMPRESSION_METHODS * COMPRESSION_NAME_LEN;
/// Each stored file is preceded by a copy of its FPakEntry: Offset, Size, UncompressedSize (int64 x3),
/// CompressionMethodIndex (uint32), Hash (SHA1, 20), [blocks], Flags (uint8), CompressionBlockSize (uint32)
/// (FPakEntry.cs:265-269 StructSize; FPakEntry.cs:41-153 serialized form).
pub const ENTRY_HEADER_SIZE: u64 = 8 * 3 + 4 + 20 + 1 + 4;
/// FDirectoryIndex location of a deleted record: int.MinValue (PakFileReader.cs:450)
const DELETED: i32 = i32::MIN;

/// Bytes of a pak entry: a view into the pak's mapping, or owned bytes for a decompressed entry
#[derive(Clone)]
pub enum Bytes {
    Mapped(Arc<Mmap>, Range<usize>),
    Owned(Arc<[u8]>),
}
impl Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Mapped(m, r) => &m[r.clone()],
            Bytes::Owned(v) => v,
        }
    }
}
impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bytes({} bytes)", self.len())
    }
}
impl Bytes {
    pub fn empty() -> Self {
        Bytes::Owned(Arc::from(Vec::new()))
    }
}

/// A decoded FPakEntry
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub offset: u64,
    pub csize: u64,
    pub usize: u64,
    pub method: u32,
    pub encrypted: bool,
    pub block_size: u64,
    /// absolute [start, end) of each compression block
    pub blocks: Vec<(u64, u64)>,
    /// bytes of the header copy before the data
    pub header: u64,
}

pub struct Pak {
    pub path: PathBuf,
    pub version: i32,
    pub mount_point: String, // e.g. "../../../Mordhau/Content/"
    pub encrypted_index: bool,
    /// index 0 = none, then the footer's names (FPakInfo.cs:515-530)
    pub compression: Vec<String>,
    pub entry_count: i32,
    map: Arc<Mmap>,
    /// FPakFile::EncodedPakEntries: bit-packed entries, decoded on read (a range of the mapping)
    encoded: Range<usize>,
    /// entries that did not fit the encoding
    nonencoded: Vec<Entry>,
}

#[derive(Debug)]
pub struct PakError(pub String);
impl std::fmt::Display for PakError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PakError {}
fn err<T>(s: impl Into<String>) -> Result<T, PakError> {
    Err(PakError(s.into()))
}

impl Pak {
    /// Open a pak and parse its index. `files` receives (mounted path, entry location) where location >= 0 is a byte
    /// offset into the encoded entries and < 0 is -(index+1) into the non-encoded list (the directory index's own
    /// convention, PakFileReader.cs:449-466).
    pub fn open(path: &Path, files: &mut Vec<(String, i32)>) -> Result<Pak, PakError> {
        let f = File::open(path).map_err(|e| PakError(format!("cannot open {}: {e}", path.display())))?;
        // SAFETY: read-only mapping of a file nothing in this process writes; the game folder is never written
        // (if Steam patched the pak while mapped, reads would see the new bytes - restart after a game update)
        let map = Arc::new(unsafe { Mmap::map(&f) }.map_err(|e| PakError(format!("cannot map {}: {e}", path.display())))?);
        let m: &[u8] = &map;
        if m.len() < FOOTER_SIZE {
            return err("file shorter than the pak footer");
        }
        let foot = &m[m.len() - FOOTER_SIZE..];
        let mut r = Cursor::new(foot);
        r.skip(16); // EncryptionKeyGuid
        let encrypted_index = r.u8() != 0;
        if r.u32() != MAGIC {
            return err("bad magic");
        }
        let version = r.s32();
        let index_offset = r.s64();
        let index_size = r.s64();
        r.skip(20); // IndexHash
        let mut compression = vec!["None".to_string()];
        for _ in 0..MAX_COMPRESSION_METHODS {
            let nm = &foot[r.p as usize..r.p as usize + COMPRESSION_NAME_LEN];
            r.skip(COMPRESSION_NAME_LEN as i64);
            let z = nm.iter().position(|&c| c == 0).unwrap_or(nm.len());
            let s = String::from_utf8_lossy(&nm[..z]).into_owned();
            if !s.is_empty() {
                compression.push(s);
            }
        }
        if encrypted_index {
            return err("encrypted index (needs the game's AES key; Mordhau's paks are not encrypted)");
        }
        if version < VERSION_PATH_HASH_INDEX {
            return err(format!("pak version {version}: only the 4.25+ path-hash/directory index (v10+) is implemented"));
        }
        let span = |o: i64, n: i64| -> Result<Range<usize>, PakError> {
            if o < 0 || n < 0 || (o + n) as usize > m.len() {
                return err(format!("index range {o}+{n} outside the file"));
            }
            Ok(o as usize..(o + n) as usize)
        };
        // Primary index (PakFileReader.cs:330-397 ReadIndexUpdated; UE FPakFile::LoadIndexInternal)
        let ir = span(index_offset, index_size)?;
        let mut r = Cursor::new(&m[ir.clone()]);
        let mount_point = r.fstring();
        let entry_count = r.s32();
        r.skip(8); // PathHashSeed
        if r.s32() != 0 {
            r.skip(8 + 8 + 20); // bReaderHasPathHashIndex: offset, size, hash (unused here)
        }
        if r.s32() == 0 {
            return err("no full directory index"); // bReaderHasFullDirectoryIndex
        }
        let dir_offset = r.s64();
        let dir_size = r.s64();
        r.skip(20);
        let enc_size = r.s32();
        let enc_start = ir.start + r.p as usize;
        let encoded = enc_start..enc_start + enc_size.max(0) as usize;
        r.skip(enc_size as i64);
        let n_nonenc = r.s32();
        let mut nonencoded = Vec::new();
        for _ in 0..n_nonenc.max(0) {
            nonencoded.push(read_entry(&mut r));
        }
        if r.bad || encoded.end > ir.end {
            return err("primary index truncated");
        }
        // FDirectoryIndex: TMap<FString dir, TMap<FString file, int32 entry>> (PakFileReader.cs:425-470)
        let mut d = Cursor::new(&m[span(dir_offset, dir_size)?]);
        let trim = mount_point.ends_with('/');
        let nd = d.s32();
        for _ in 0..nd.max(0) {
            let mut dir = d.fstring();
            if trim && dir.starts_with('/') {
                dir.remove(0);
            }
            let nf = d.s32();
            for _ in 0..nf.max(0) {
                let fname = d.fstring();
                let loc = d.s32();
                if loc == DELETED {
                    continue;
                }
                files.push((format!("{mount_point}{dir}{fname}"), loc));
            }
            if d.bad {
                return err("directory index truncated");
            }
        }
        Ok(Pak { path: path.to_path_buf(), version, mount_point, encrypted_index, compression, entry_count, map, encoded, nonencoded })
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// FPakFile::DecodePakEntry (FPakEntry.cs:159-302). Bitfield: [31] offset fits 32 bits, [30] uncompressed size
    /// fits, [29] size fits, [28..23] compression method index, [22] encrypted, [21..6] block count, [5..0] block
    /// size / 2048 (0x3f = an explicit uint32 follows).
    pub fn entry(&self, loc: i32) -> Entry {
        if loc < 0 {
            return self.nonencoded.get((-loc - 1) as usize).cloned().unwrap_or_else(|| empty_entry());
        }
        let mut r = Cursor::new(&self.map[self.encoded.clone()]);
        r.p = loc as i64;
        let bf = r.u32();
        let block_size = if bf & 0x3f == 0x3f { r.u32() as u64 } else { ((bf & 0x3f) as u64) << 11 };
        let method = (bf >> 23) & 0x3f;
        let offset = if bf & (1 << 31) != 0 { r.u32() as u64 } else { r.s64() as u64 };
        let usize_ = if bf & (1 << 30) != 0 { r.u32() as u64 } else { r.s64() as u64 };
        let mut csize = usize_;
        if method != 0 {
            csize = if bf & (1 << 29) != 0 { r.u32() as u64 } else { r.s64() as u64 };
        }
        let encrypted = (bf >> 22) & 1 != 0;
        let nb = ((bf >> 6) & 0xffff) as u64;
        let header = ENTRY_HEADER_SIZE + if method != 0 { 4 + nb * 16 } else { 0 };
        let mut start = offset + header;
        let mut blocks = Vec::new();
        if nb == 1 && !encrypted {
            blocks.push((start, start + csize));
        } else if nb > 0 {
            for _ in 0..nb {
                let ln = r.u32() as u64;
                blocks.push((start, start + ln));
                start += ln; // no AES alignment: encrypted entries are refused in read()
            }
        }
        Entry { offset, csize, usize: usize_, method, encrypted, block_size: if nb == 1 { usize_ } else { block_size }, blocks, header }
    }

    fn method_name(&self, e: &Entry) -> &str {
        self.compression.get(e.method as usize).map_or("?", |s| s.as_str())
    }

    /// Bytes of one entry. Uncompressed entries are views into the mapping; zlib entries are inflated (UNTESTED: no
    /// zlib entry in Mordhau). Oodle is a proprietary codec with no redistributable decoder and no Mordhau entry uses
    /// any compression (docs/PAK_FORMAT.md). `verify` checks the SHA1 the pak stores in the entry's header copy
    /// (FPakEntry Hash) against the bytes.
    pub fn read(&self, loc: i32, verify: bool) -> Result<Bytes, PakError> {
        let e = self.entry(loc);
        if e.encrypted {
            return err(format!("encrypted entry in {}", self.path.display()));
        }
        let m: &[u8] = &self.map;
        let hs = e.offset as usize;
        if hs + ENTRY_HEADER_SIZE as usize > m.len() {
            return err(format!("entry at {} outside {}", e.offset, self.path.display()));
        }
        let mut hr = Cursor::new(&m[hs..hs + ENTRY_HEADER_SIZE as usize]);
        hr.skip(8);
        if hr.s64() as u64 != e.csize || hr.s64() as u64 != e.usize {
            // the header copy must agree with the index
            return err(format!("entry header mismatch at {} in {}", e.offset, self.path.display()));
        }
        let sha = &m[hs + 28..hs + 48];
        let out = if e.method == 0 {
            let s = (e.offset + e.header) as usize;
            let end = s + e.usize as usize;
            if end > m.len() {
                return err(format!("entry at {} runs past {}", e.offset, self.path.display()));
            }
            Bytes::Mapped(self.map.clone(), s..end)
        } else if self.method_name(&e).eq_ignore_ascii_case("zlib") {
            // FCompression zlib: each block is a zlib stream of up to block_size bytes
            let mut v = Vec::with_capacity(e.usize as usize);
            for &(a, b) in &e.blocks {
                let blk = m.get(a as usize..b as usize).ok_or_else(|| PakError("zlib block outside the pak".into()))?;
                let d = miniz_oxide::inflate::decompress_to_vec_zlib(blk).map_err(|x| PakError(format!("zlib: {x:?}")))?;
                v.extend_from_slice(&d);
            }
            v.truncate(e.usize as usize);
            Bytes::Owned(Arc::from(v))
        } else {
            return err(format!("compression '{}' not supported ({})", self.method_name(&e), self.path.display()));
        };
        if verify && e.method == 0 && sha1_smol::Sha1::from(&out[..]).digest().bytes() != sha {
            return err(format!("SHA1 mismatch at {} in {}", e.offset, self.path.display()));
        }
        Ok(out)
    }

    /// `size` bytes at `start` inside an uncompressed entry, without touching the rest (texture mips and sound chunks
    /// live in .ubulk files of up to tens of MB)
    pub fn read_range(&self, loc: i32, start: u64, size: u64) -> Result<Bytes, PakError> {
        let e = self.entry(loc);
        if e.encrypted || e.method != 0 || start + size > e.usize {
            return err(format!("range {start}+{size} not readable in entry at {} of {}", e.offset, self.path.display()));
        }
        let s = (e.offset + e.header + start) as usize;
        if s + size as usize > self.map.len() {
            return err("range outside the pak");
        }
        Ok(Bytes::Mapped(self.map.clone(), s..s + size as usize))
    }
}

fn empty_entry() -> Entry {
    Entry { offset: 0, csize: 0, usize: 0, method: 0, encrypted: false, block_size: 0, blocks: vec![], header: ENTRY_HEADER_SIZE }
}

/// Non-encoded FPakEntry as serialized in the index (FPakEntry.cs:41-153, version >= 8)
fn read_entry(r: &mut Cursor) -> Entry {
    let offset = r.s64() as u64;
    let csize = r.s64() as u64;
    let usize_ = r.s64() as u64;
    let method = r.u32();
    r.skip(20);
    let mut blocks = Vec::new();
    if method != 0 {
        let nb = r.s32();
        for _ in 0..nb.max(0) {
            // relative since PakFile_Version_RelativeChunkOffsets (FPakEntry.cs:137-145)
            let a = r.s64() as u64 + offset;
            let b = r.s64() as u64 + offset;
            blocks.push((a, b));
        }
    }
    let encrypted = r.u8() & 1 != 0;
    let block_size = r.u32() as u64;
    let header = ENTRY_HEADER_SIZE + if method != 0 { 4 + blocks.len() as u64 * 16 } else { 0 };
    Entry { offset, csize, usize: usize_, method, encrypted, block_size, blocks, header }
}

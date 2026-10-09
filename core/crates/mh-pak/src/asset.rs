//! One cooked UE 4.26 package: summary, name/import/export maps (port of the header half of `ue_asset.gd` `UeAsset`).
//! Export data is read in `reader.rs`. Format sources per step are cited inline and collected in docs/PAK_FORMAT.md.
//!
//! UE 4.26 names: FPackageFileSummary operator<< (CoreUObject/Private/UObject/PackageFileSummary.cpp), FObjectImport /
//! FObjectExport operator<< (ObjectResource.cpp). Cross-checked byte for byte against CUE4Parse at the pinned commit
//! (tools/CUE4Parse-src/CUE4Parse/...).

use crate::buf::Cursor;
use crate::pak::Bytes;
use crate::vfs::Vfs;
use std::cell::RefCell;

pub const PACKAGE_FILE_TAG: u32 = 0x9E2A83C1; // PACKAGE_FILE_TAG (FPackageFileSummary.cs:123-163)
pub const LEGACY_FILE_VERSION_426: i32 = -7; // UE 4.26 writes -7 (FPackageFileSummary.cs:145-188)
// EUnrealEngineObjectUE4Version values for versioned headers (CUE4Parse UE4/Versions/ObjectVersion.cs: the enum counts
// up from OLDEST_LOADABLE_PACKAGE = 214 at line 298, one per enumerator; these are lines 797, 887, 911. Check: the
// count gives CORRECT_LICENSEE_FLAG = 522 = UE 4.26's VER_UE4_AUTOMATIC_VERSION)
pub const VER_SERIALIZE_TEXT_IN_PACKAGES: i32 = 459;
pub const VER_NAME_HASHES_SERIALIZED: i32 = 504;
pub const VER_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID: i32 = 516;
/// EPackageFlags::PKG_FilterEditorOnly: no LocalizationId (FPackageFileSummary.cs:277-282)
pub const PKG_FILTER_EDITOR_ONLY: u32 = 0x80000000;
/// EObjectFlags::RF_ClassDefaultObject: no object Guid after the properties (UObject.cs:226)
pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x10;

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub header_size: i32, // TotalHeaderSize
    pub package_flags: u32,
    pub name_count: i32,
    pub name_offset: i32,
    pub name_hashes: bool,
    pub export_count: i32,
    pub export_offset: i32,
    pub import_count: i32,
    pub import_offset: i32,
    /// BulkDataStartOffset (unversioned cooks only): .ubulk offsets count from it
    pub bulk_data_start: i64,
}

/// FObjectImport (ObjectResource.cs:377-382)
#[derive(Clone, Debug)]
pub struct Import {
    pub class_package: String,
    pub class_name: String,
    pub outer: i32,
    pub name: String,
}

/// FObjectExport, 4.26 (ObjectResource.cs:214-304)
#[derive(Clone, Debug)]
pub struct Export {
    pub cls: i32,
    pub sup: i32,
    pub tmpl: i32,
    pub outer: i32,
    pub name: String,
    pub flags: u32,
    pub size: i64,
    pub off: i64,
}

pub struct Package {
    /// "Mordhau/Content/.../BP_X" (stored spelling, no extension)
    pub name: String,
    pub ext: &'static str,
    pub header_size: i32,
    pub package_flags: u32,
    pub bulk_data_start: i64,
    pub names: Vec<String>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    /// the .uasset (header) and .uexp (export data) entries: views into the pak mapping
    pub uasset: Bytes,
    pub uexp: Bytes,
    /// property values this reader could not decode ("<type> <key>: <why>"), skipped by tag size
    pub unsupported: RefCell<Vec<String>>,
}

#[derive(Debug)]
pub struct AssetError(pub String);
impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AssetError {}

/// FPackageFileSummary, 4.26 cooked (FPackageFileSummary.cs:123-532), up to BulkDataStartOffset; Err for anything but an
/// unversioned 4.26 cook or when TotalHeaderSize is not the .uasset's size. `versioned_ok` also accepts a versioned
/// header (one package in Mordhau, Slate/Pointer, FileVersionUE4 514, an editor asset that shipped): its optional
/// fields follow the version (FPackageFileSummary.cs:276-288).
pub fn summary(r: &mut Cursor, p: &str, versioned_ok: bool) -> Result<Summary, AssetError> {
    let e = |s: String| Err(AssetError(s));
    if r.u32() != PACKAGE_FILE_TAG {
        return e(format!("not a package {p}"));
    }
    let legacy = r.s32();
    if legacy != LEGACY_FILE_VERSION_426 {
        return e(format!("LegacyFileVersion {legacy} in {p} (only 4.26's -7 is implemented)"));
    }
    r.skip(4); // LegacyUE3Version
    let ue4 = r.s32(); // FileVersionUE4: 0 = unversioned cook (all of Mordhau): read as the engine's latest, 4.26
    r.skip(4); // FileVersionLicenseeUE4
    if ue4 != 0 && !versioned_ok {
        return e(format!("versioned package (FileVersionUE4 {ue4}) in {p}: only unversioned 4.26 cooks are implemented"));
    }
    let ncv = r.s32();
    r.skip(20 * ncv as i64); // CustomVersions: FCustomVersion {FGuid Key; int32 Version}
    let mut sm = Summary { header_size: r.s32(), ..Default::default() };
    r.fstring(); // FolderName
    sm.package_flags = r.u32();
    sm.name_count = r.s32();
    sm.name_offset = r.s32();
    sm.name_hashes = ue4 == 0 || ue4 >= VER_NAME_HASHES_SERIALIZED;
    if sm.package_flags & PKG_FILTER_EDITOR_ONLY == 0 && (ue4 == 0 || ue4 >= VER_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID) {
        r.fstring(); // LocalizationId
    }
    if ue4 == 0 || ue4 >= VER_SERIALIZE_TEXT_IN_PACKAGES {
        r.skip(8); // GatherableTextDataCount, GatherableTextDataOffset
    }
    sm.export_count = r.s32();
    sm.export_offset = r.s32();
    sm.import_count = r.s32();
    sm.import_offset = r.s32();
    if ue4 == 0 {
        // the rest up to BulkDataStartOffset, 4.26 cooked (FPackageFileSummary.cs:320-507)
        r.skip(4 + 8 + 4 + 4); // DependsOffset, SoftPackageReferences count + offset, SearchableNamesOffset, ThumbnailTableOffset
        r.skip(16); // Guid (no PersistentGuid: PKG_FilterEditorOnly)
        let ng = r.s32();
        r.skip(8 * ng as i64); // Generations: FGenerationInfo {int32 ExportCount, NameCount}
        for _ in 0..2 {
            // SavedByEngineVersion, CompatibleWithEngineVersion: FEngineVersion = uint16 x3, uint32 Changelist,
            // FString Branch (FEngineVersion.cs)
            r.skip(10);
            r.fstring();
        }
        r.skip(4); // CompressionFlags
        let nc = r.s32();
        r.skip(16 * nc as i64); // CompressedChunks: FCompressedChunk = 4 x int32
        r.skip(4); // PackageSource
        for _ in 0..r.s32().max(0) {
            r.fstring(); // AdditionalPackagesToCook: TArray<FString>
        }
        r.skip(4); // AssetRegistryDataOffset
        sm.bulk_data_start = r.s64(); // BulkDataStartOffset: bulk data offsets are relative to it unless NoOffsetFixUp
    }
    // a split cook's .uasset is exactly the header; an uncooked (versioned) package holds its exports after it
    let len = r.len();
    let hs = sm.header_size as i64;
    if (hs != len && !(ue4 != 0 && hs < len)) || r.bad {
        return e(format!("TotalHeaderSize {} != .uasset size {} in {p}", sm.header_size, len));
    }
    Ok(sm)
}

impl Package {
    /// Open a package by path ("Mordhau/Content/.../BP_X", any case, no extension): .uasset, else .umap
    pub fn open(vfs: &Vfs, p: &str) -> Result<Package, AssetError> {
        let ext = if vfs.has(&format!("{p}.uasset")) {
            ".uasset"
        } else if vfs.has(&format!("{p}.umap")) {
            ".umap"
        } else {
            return Err(AssetError(format!("not in the paks: {p}")));
        };
        let full = format!("{p}{ext}");
        let sp = vfs.spelling(&full);
        let name = sp[..sp.len() - ext.len()].to_string();
        let uasset = vfs.try_read(&full, false).map_err(|e| AssetError(e.0))?;
        let uexp = vfs.read(&format!("{name}.uexp"), false).unwrap_or_else(Bytes::empty);
        let mut r = Cursor::new(&uasset);
        let sm = summary(&mut r, p, false)?;
        let mut pk = Package {
            name,
            ext,
            header_size: sm.header_size,
            package_flags: sm.package_flags,
            bulk_data_start: sm.bulk_data_start,
            names: Vec::with_capacity(sm.name_count.max(0) as usize),
            imports: Vec::with_capacity(sm.import_count.max(0) as usize),
            exports: Vec::with_capacity(sm.export_count.max(0) as usize),
            uasset: uasset.clone(),
            uexp,
            unsupported: RefCell::new(Vec::new()),
        };
        // Name map: FNameEntrySerialized = FString + uint16 NonCasePreservingHash + uint16 CasePreservingHash
        // (FNameEntrySerialized.cs:27; CUE4Parse also Trim()s the string there, UE does not: names are kept as stored)
        r.p = sm.name_offset as i64;
        for _ in 0..sm.name_count.max(0) {
            pk.names.push(r.fstring());
            r.skip(4);
        }
        // Import map: ClassPackage, ClassName (FName), OuterIndex (FPackageIndex), ObjectName (ObjectResource.cs:377-382)
        r.p = sm.import_offset as i64;
        for _ in 0..sm.import_count.max(0) {
            let class_package = pk.fname(&mut r);
            let class_name = pk.fname(&mut r);
            let outer = r.s32();
            let name = pk.fname(&mut r);
            pk.imports.push(Import { class_package, class_name, outer, name });
        }
        // Export map, 4.26 (ObjectResource.cs:214-304): ClassIndex, SuperIndex, TemplateIndex, OuterIndex, ObjectName,
        // ObjectFlags u32, SerialSize i64, SerialOffset i64, bForcedExport, bNotForClient, bNotForServer (i32 x3),
        // PackageGuid (16), PackageFlags u32, bNotAlwaysLoadedForEditorGame, bIsAsset (i32 x2), FirstExportDependency and
        // the 4 dependency counts (i32 x5) = 104 bytes
        r.p = sm.export_offset as i64;
        for _ in 0..sm.export_count.max(0) {
            let cls = r.s32();
            let sup = r.s32();
            let tmpl = r.s32();
            let outer = r.s32();
            let name = pk.fname(&mut r);
            let flags = r.u32();
            let size = r.s64();
            let off = r.s64();
            r.skip(12 + 16 + 4 + 8 + 20);
            pk.exports.push(Export { cls, sup, tmpl, outer, name, flags, size, off });
        }
        if r.bad {
            return Err(AssetError(format!("header maps run past the .uasset in {p}")));
        }
        Ok(pk)
    }

    /// FName in a package = int32 name-map index + int32 Number; Number > 0 prints as "<name>_<Number-1>" (FName.cs Text)
    pub fn fname(&self, r: &mut Cursor) -> String {
        let i = r.s32();
        let n = r.s32();
        let s = if i >= 0 && (i as usize) < self.names.len() { self.names[i as usize].as_str() } else { "None" };
        if n == 0 {
            s.to_string()
        } else {
            format!("{s}_{}", n - 1)
        }
    }

    /// A cursor over the whole package: .uasset then .uexp (export SerialOffset counts from the .uasset start)
    pub fn cursor(&self) -> Cursor<'_> {
        Cursor::split(&self.uasset, &self.uexp)
    }

    /// Total bytes of .uasset + .uexp
    pub fn data_len(&self) -> usize {
        self.uasset.len() + self.uexp.len()
    }

    /// `n` bytes at `at` of the package's concatenated data as a Bytes view into the pak mapping (zero-copy; copied
    /// only when the range straddles the .uasset/.uexp seam). For large native arrays (lighting bricks, cubemaps)
    pub fn bytes_at(&self, at: i64, n: usize) -> Option<Bytes> {
        let h = self.uasset.len() as i64;
        if at < 0 || at + n as i64 > h + self.uexp.len() as i64 {
            return None;
        }
        let sub = |b: &Bytes, o: usize| -> Bytes {
            match b {
                Bytes::Mapped(m, r) => Bytes::Mapped(m.clone(), r.start + o..r.start + o + n),
                Bytes::Owned(v) => Bytes::Owned(std::sync::Arc::from(&v[o..o + n])),
            }
        };
        if at >= h {
            Some(sub(&self.uexp, (at - h) as usize))
        } else if at + n as i64 <= h {
            Some(sub(&self.uasset, at as usize))
        } else {
            Some(Bytes::Owned(std::sync::Arc::from(self.slice(at as usize, at as usize + n)?.into_owned())))
        }
    }

    /// Bytes [a, b) of the package's concatenated data (copied only when the range straddles the .uasset/.uexp seam)
    pub fn slice(&self, a: usize, b: usize) -> Option<std::borrow::Cow<'_, [u8]>> {
        if b < a {
            return None;
        }
        self.cursor().at(a as i64, b - a)
    }
}

/// Class name of a package's first export, the column `mdx list` writes to extract/manifest.tsv, read from the .uasset
/// header alone: export 0's ClassIndex (an import, FPackageIndex < 0) -> that import's ObjectName -> the name map entry,
/// walking the name map only up to that entry. "" for a missing package or a class that is not an import.
pub fn first_export_class(vfs: &Vfs, pkg_path: &str) -> String {
    let Some(ua) = vfs.read(&format!("{pkg_path}.uasset"), false) else { return String::new() };
    let mut r = Cursor::new(&ua);
    let Ok(sm) = summary(&mut r, pkg_path, true) else { return String::new() };
    if sm.export_count == 0 {
        return String::new();
    }
    let rd = |at: i64| Cursor::new(&ua).at(at, 4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]));
    let Some(cls) = rd(sm.export_offset as i64) else { return String::new() };
    if cls >= 0 || -cls > sm.import_count {
        return String::new();
    }
    // FObjectImport = ClassPackage FName (8) + ClassName FName (8) + OuterIndex (4) + ObjectName FName (8) = 28 bytes
    let im = sm.import_offset as i64 + (-cls - 1) as i64 * 28;
    let (Some(ni), Some(num)) = (rd(im + 20), rd(im + 24)) else { return String::new() };
    if ni < 0 || ni >= sm.name_count {
        return String::new();
    }
    r.p = sm.name_offset as i64;
    for _ in 0..ni {
        // FNameEntrySerialized: FString (int32 length; < 0 = UTF-16 units) + 2 uint16 hashes
        let n = r.s32() as i64;
        r.skip((if n >= 0 { n } else { -n * 2 }) + if sm.name_hashes { 4 } else { 0 });
    }
    let s = r.fstring();
    if num == 0 {
        s
    } else {
        format!("{s}_{}", num - 1)
    }
}

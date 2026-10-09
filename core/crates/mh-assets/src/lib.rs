//! mh-assets: engine-neutral decoders for the bulk data of Mordhau's cooked UE 4.26 packages, read from the game's own
//! paks: textures (`texture`), sound waves (`sound`), static meshes (`static_mesh`), skeletal meshes
//! (`skeletal_mesh`) and animation sequences (`anim`), over `FByteBulkData` payloads (`bulk`).
//!
//! A port of the verified GDScript loaders in godot/components/ue/pak/ (ue_bulk.gd, ue_texture.gd, ue_sound_wave.gd,
//! ue_static_mesh.gd, ue_skeletal_mesh.gd, ue_anim_sequence.gd; docs/PAK_FORMAT.md "Assets"), with the same layouts and
//! CUE4Parse citations. Output is plain data in UE units (cm, Z up, left-handed) - no engine types; `coords` converts
//! to Y-up metres the way CUE4Parse's glTF writer (and so every extract/gltf export) does.
//!
//! Package access goes through the `PackageSource` trait (implemented for mh-pak's reader in `pak_source`): the
//! decoders need the package header tables, the export bytes and ranged reads of `.ubulk` companions, never a whole
//! `.ubulk`.

pub mod equipment_paint;
pub mod weapon_trail;
pub mod anim;
pub mod buf;
pub mod bulk;
pub mod coords;
pub mod cosmetics;
pub mod fonts;
/// FKAggregateGeom decoder (mh_pak::aggeom, shared with mh-level)
pub use mh_pak::aggeom as collision;
pub mod physics;
pub mod material;
pub mod particles;
pub mod pak_source;
pub mod props;
pub mod shader;
pub mod skeletal_mesh;
pub mod sound;
pub mod sound_class;
pub mod sound_cue;
pub mod static_mesh;
pub mod texture;
pub mod umg;
pub mod vlm;

use std::fmt;
use std::sync::Arc;

/// EObjectFlags::RF_ClassDefaultObject: a CDO stores no object Guid after its properties (UObject.cs:226)
pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x10;

#[derive(Debug, Clone, PartialEq)]
pub struct AssetError(pub String);

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AssetError {}

pub type Result<T> = std::result::Result<T, AssetError>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(AssetError(msg.into()))
}

/// One import of a package's import map (ObjectResource.cs:377-382), names already resolved
#[derive(Debug, Clone, Default)]
pub struct RawImport {
    pub class_name: String,
    pub outer: i32,
    pub object_name: String,
}

/// One export of a package's export map (ObjectResource.cs:214-304, 4.26)
#[derive(Debug, Clone, Default)]
pub struct RawExport {
    /// FPackageIndex of the class (< 0 import, > 0 export)
    pub class_index: i32,
    pub object_name: String,
    pub flags: u32,
    /// offset of the export's serialized bytes in `RawPackage::data` (the .uasset header, then the .uexp)
    pub serial_offset: u64,
    pub serial_size: u64,
}

/// What the decoders need of an opened package: its tables and its bytes. The reader (mh-pak) fills it.
#[derive(Debug, Clone, Default)]
pub struct RawPackage {
    /// mounted path as stored in the pak, without extension ("Mordhau/Content/.../X")
    pub name: String,
    /// .uasset + .uexp, so export serial offsets index it directly
    pub data: Vec<u8>,
    /// name map, as stored (no trim)
    pub names: Vec<String>,
    pub imports: Vec<RawImport>,
    pub exports: Vec<RawExport>,
    /// FPackageFileSummary::BulkDataStartOffset: bulk data offsets count from it unless BULKDATA_NoOffsetFixUp
    pub bulk_data_start: i64,
}

impl RawPackage {
    /// Object name of an FPackageIndex (0 = null -> "")
    pub fn object_name(&self, idx: i32) -> &str {
        if idx > 0 {
            self.exports.get(idx as usize - 1).map_or("", |e| e.object_name.as_str())
        } else if idx < 0 {
            self.imports.get((-idx) as usize - 1).map_or("", |i| i.object_name.as_str())
        } else {
            ""
        }
    }

    /// Class name of an export: the object name its ClassIndex resolves to (UeAsset.node_name(node(e.cls)))
    pub fn export_class(&self, e: &RawExport) -> &str {
        self.object_name(e.class_index)
    }

    /// FName in a package = int32 name-map index + int32 Number; Number > 0 prints as "<name>_<Number-1>" (FName.cs)
    pub fn fname(&self, r: &mut buf::Buf) -> String {
        let i = r.s32();
        let n = r.s32();
        let s = if i >= 0 && (i as usize) < self.names.len() { self.names[i as usize].as_str() } else { "None" };
        if n == 0 { s.to_string() } else { format!("{}_{}", s, n - 1) }
    }

    /// The first export of class `class` (optionally the one named `name`)
    pub fn find_export(&self, class: &str, name: Option<&str>) -> Option<&RawExport> {
        self.exports
            .iter()
            .find(|e| self.export_class(e) == class && name.map_or(true, |n| e.object_name == n))
    }

    /// The export's serialized bytes as (start, end) in `data`
    pub fn export_range(&self, e: &RawExport) -> Result<(usize, usize)> {
        let s = e.serial_offset as usize;
        let t = s.saturating_add(e.serial_size as usize);
        if t > self.data.len() {
            return err(format!("{}: export {} [{}, {}) beyond the package's {} bytes", self.name, e.object_name, s, t,
                self.data.len()));
        }
        Ok((s, t))
    }
}

/// Where packages and their companion files come from. Implemented for mh-pak's mounted pak file system
/// (`pak_source`); tests or other hosts may implement it over anything that yields the same bytes.
pub trait PackageSource {
    /// The package at a mounted path without extension ("Mordhau/Content/.../X"; any case, .uasset or .umap)
    fn package(&self, path: &str) -> Result<Arc<RawPackage>>;
    /// `len` bytes at `offset` of a mounted file (a `.ubulk` / `.uptnl`), read in place: never the whole file
    fn bulk_range(&self, file: &str, offset: u64, len: u64) -> Result<Vec<u8>>;
}

/// Skip the UObject Guid that a non-CDO object stores after its tagged properties (bool bHasGuid + FGuid,
/// UObject.cs:226-236)
pub(crate) fn skip_object_guid(r: &mut buf::Buf, e: &RawExport) {
    if e.flags & RF_CLASS_DEFAULT_OBJECT == 0 && r.s32() != 0 {
        r.skip(16);
    }
}

pub use anim::{AnimSequence, Track, VecTrack, QuatTrack};
pub use skeletal_mesh::{RefSkeleton, BoneInfo, Transform, SkeletalMesh};
pub use sound::SoundWave;
pub use static_mesh::StaticMesh;
pub use texture::{Texture, Mip, PixelFormat};

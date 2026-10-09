//! `PackageSource` over mh-pak: packages opened from the user's mounted paks (mh_pak::vfs::Vfs +
//! mh_pak::asset::Package), companion `.ubulk` ranges read in place (Vfs::read_range, never the whole file).
//! Opened packages are cached by lower-case path up to `CACHE_MAX_BYTES` of package data, then the cache is dropped
//! (as UeAsset's CACHE_MAX_BYTES does).

use crate::{AssetError, PackageSource, RawExport, RawImport, RawPackage, Result};
use mh_pak::asset::Package;
use mh_pak::vfs::Vfs;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const CACHE_MAX_BYTES: usize = 128 << 20;

pub struct PakSource {
    pub vfs: Arc<Vfs>,
    cache: Mutex<(HashMap<String, Arc<RawPackage>>, usize)>,
}

impl PakSource {
    pub fn new(vfs: Arc<Vfs>) -> Self {
        PakSource { vfs, cache: Mutex::new((HashMap::new(), 0)) }
    }

    /// Mount the install at mh_pak's default game folder ($MORDHAU_DIR, else Steam's)
    pub fn mount_default() -> Result<Self> {
        Vfs::mount_default().map(|v| PakSource::new(Arc::new(v))).map_err(|e| AssetError(e.0))
    }

    pub fn clear_cache(&self) {
        let mut c = self.cache.lock().unwrap();
        c.0.clear();
        c.1 = 0;
    }
}

/// The header tables and bytes the decoders need, copied out of an mh-pak Package
pub fn raw_package(pk: &Package) -> RawPackage {
    let mut data = Vec::with_capacity(pk.data_len());
    data.extend_from_slice(&pk.uasset);
    data.extend_from_slice(&pk.uexp);
    RawPackage {
        name: pk.name.clone(),
        data,
        names: pk.names.clone(),
        imports: pk
            .imports
            .iter()
            .map(|i| RawImport { class_name: i.class_name.clone(), outer: i.outer, object_name: i.name.clone() })
            .collect(),
        exports: pk
            .exports
            .iter()
            .map(|e| RawExport {
                class_index: e.cls,
                object_name: e.name.clone(),
                flags: e.flags,
                serial_offset: e.off.max(0) as u64,
                serial_size: e.size.max(0) as u64,
            })
            .collect(),
        bulk_data_start: pk.bulk_data_start,
    }
}

impl PackageSource for PakSource {
    fn package(&self, path: &str) -> Result<Arc<RawPackage>> {
        let key = path.to_lowercase();
        if let Some(p) = self.cache.lock().unwrap().0.get(&key) {
            return Ok(p.clone());
        }
        let pk = Package::open(&self.vfs, path).map_err(|e| AssetError(e.0))?;
        let raw = Arc::new(raw_package(&pk));
        let mut c = self.cache.lock().unwrap();
        if c.1 + raw.data.len() > CACHE_MAX_BYTES {
            c.0.clear();
            c.1 = 0;
        }
        c.1 += raw.data.len();
        c.0.insert(key, raw.clone());
        Ok(raw)
    }

    fn bulk_range(&self, file: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        match self.vfs.read_range(file, offset, len) {
            Some(b) => Ok(b.to_vec()),
            None => Err(AssetError(format!("cannot read {} bytes at {} of {}", len, offset, file))),
        }
    }
}

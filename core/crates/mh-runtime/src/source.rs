//! source.rs - where each kind of data comes from (R2 flags, docs/RUST_RUNTIME.md section 6).
//!   --level-source pak|extract   map placements: mh-level over the paks (default when the install mounts) or ue.rs
//!   --assets gltf|pak            meshes + textures: extract/gltf glb/PNG (default) or mh-assets decoding of the paks
//!   --materials standard|uetint  Bevy StandardMaterial approximation (default) or the ported ue_tint WGSL
//!   --sim stub|core              StubSim (default) or mordhau-core's combat World (sim_core.rs)

use bevy::prelude::Resource;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum LevelSrc {
    Pak,
    Extract,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum AssetSrc {
    Gltf,
    Pak,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum MatSrc {
    Standard,
    UeTint,
}

#[derive(Resource, Clone)]
pub struct Source {
    pub level: LevelSrc,
    pub assets: AssetSrc,
    pub materials: MatSrc,
    /// the user's install, mounted once (mh-pak); None when it is not found
    pub vfs: Option<Arc<mh_pak::Vfs>>,
    /// mh-assets' PackageSource over the same mount (decoders: meshes, textures, materials)
    pub pak: Option<Arc<mh_assets::pak_source::PakSource>>,
    /// mount error, for the evidence
    pub mount_error: Option<String>,
}

impl Source {
    pub fn mount() -> (Option<Arc<mh_pak::Vfs>>, Option<Arc<mh_assets::pak_source::PakSource>>, Option<String>) {
        match mh_pak::Vfs::mount_default() {
            Ok(v) => {
                let v = Arc::new(v);
                (Some(v.clone()), Some(Arc::new(mh_assets::pak_source::PakSource::new(v))), None)
            }
            Err(e) => (None, None, Some(format!("{e:?}"))),
        }
    }

    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"level": self.level, "assets": self.assets, "materials": self.materials,
            "paks_mounted": self.vfs.is_some(), "mount_error": self.mount_error})
    }
}

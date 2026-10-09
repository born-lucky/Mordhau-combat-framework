//! mh-pak: Mordhau's shipped `.pak` files and the cooked UE 4.26 packages inside them, read in place from the user's
//! install (never written, never extracted). Rust port of `godot/components/ue/pak` (UePakBuf, UePak, UePakVfs,
//! UeAsset, UeBulk, UePkgPak); format notes and sources in docs/PAK_FORMAT.md.
//!
//! ```no_run
//! use std::sync::Arc;
//! let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());       // 44 paks, 81,973 entries, mmapped
//! let rd = mh_pak::Reader::new(vfs.clone());
//! let p = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
//! let exports = rd.read(p).unwrap();                                // Vec<serde_json::Value>, extract/json's shape
//! let cdo = mh_pak::pkg::cdo(&exports);                             // class defaults
//! let merged = rd.defaults(p);                                      // CDO merged over the Blueprint parent chain
//! let ini = vfs.read("Mordhau/Config/DefaultInput.ini", true);      // raw file, SHA1-checked, zero-copy
//! ```
//!
//! Layers (each usable alone):
//! - [`Vfs`] / [`pak::Pak`]: path -> entry bytes ([`Bytes`], a view into the pak mapping), ranged reads, SHA1;
//! - [`asset::Package`]: summary, name/import/export maps, a [`Cursor`] over .uasset + .uexp;
//! - [`Reader`]: object references, tagged properties, exports as JSON (+ a package cache);
//! - [`bulk`]: FByteBulkData headers and payloads (texture mips, sound, mesh data) for the asset decoders;
//! - [`equiv`]: the extract/json equivalence check.

pub mod aggeom;
pub mod asset;
pub mod buf;
pub mod bulk;
pub mod cooked_body;
pub mod equiv;
pub mod pak;
pub mod pkg;
pub mod reader;
pub mod vfs;

pub use asset::{first_export_class, Export, Import, Package};
pub use buf::{short32, Cursor};
pub use pak::{Bytes, Pak, PakError};
pub use reader::{Node, Reader};
pub use vfs::{game_dir, paks_dir, Vfs};

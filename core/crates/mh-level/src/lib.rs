//! mh-level: Mordhau's maps read from the user's paks (through mh-pak) into engine-neutral placement data for the
//! runtime: levels (persistent + streamed, with their transforms), actors, static and instanced mesh placements with
//! per-slot material overrides, HLOD proxies, PlayerStarts, lights / sky / fog / post, brush volumes (nav bounds),
//! spectators. Port of godot/components/ue/ue_level.gd + ue_light.gd `read` + records/mode_data.gd `nav_bounds`;
//! API documented in docs/RUST_RUNTIME.md ("Level data").
//!
//! ```no_run
//! let pk = mh_level::Pkgs::new(mh_pak::Reader::new(std::sync::Arc::new(mh_pak::Vfs::mount_default().unwrap())));
//! let d = mh_level::read(&pk, "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena");
//! for m in &d.meshes {
//!     let world = m.xf.to_gltf();            // metres, Y up: the exported meshes' space
//!     let _ = (m.mesh_pkg(), &m.materials, &m.instances, world);
//! }
//! ```

pub mod actors;
pub mod builddata;
pub mod collision;
pub mod complex;
pub mod config;
pub mod gjk;
pub mod landscape;
pub mod level;
pub mod native;
pub mod texture;
pub mod vlm;
#[cfg(feature = "world")]
pub mod world;
pub mod xf;

pub use level::{levels, read, Actor, Crowd, Hlod, LevelData, LevelRef, LightRec, Lighting, MeshPlacement, Decal, Pkgs, ReflectionCaptureRec, Spawn, SplineMeshPlacement, Uds, Volume};
pub use xf::Xf;

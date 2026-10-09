//! Engine-neutral entry point for hosts and mods. This crate has no Bevy dependency.
//!
//! The combat rules, movement, pose evaluation and collision queries remain in their
//! original modules; this facade gives applications one place to import their APIs.
//! Original records are not compiled into the library. Applications load locally
//! generated records and read the user's installed game through the data layer.
//!
//! Setup verifies the original import inputs; launch requires prepared local data and runtime files.
//! A library interface is not DRM or proof of game ownership.
pub use mh_host as host;
pub use mh_install as install;
pub use mh_sim as simulation;
pub use mh_spec as matrix;
pub use mordhau_core as core;
pub use mh_host::Data as InstalledData;
pub use mh_sim::{FighterDesc, PoseSource, Sim, SimInput};
pub mod presentation;

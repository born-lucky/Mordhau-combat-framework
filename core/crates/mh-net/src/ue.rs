//! The UE math the net rules use: mordhau-core::ue (owner rust-combat), re-exported. Float model as there: scalars are
//! f64 (GDScript `float`), `f32r` marks every place the reference (godot/game/net) rounds to binary32 (`f32()`,
//! PackedFloat32Array stores, Vector3 components).
pub use mordhau_core::ue::{clampf, cvtss2si, f32r, maxf, minf};

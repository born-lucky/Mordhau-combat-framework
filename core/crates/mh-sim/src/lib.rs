//! mh-sim: the unified, engine-neutral Mordhau simulation facade (owner rust-combat; docs/RUST_CORE.md "mh-sim").
//!
//! One deterministic fixed-step `Sim` = exe-exact combat (mordhau-core) + character movement (mh-character
//! ExeMovement, collision through its `World` trait) + weapon traces against the character physics asset on the posed
//! skeleton (UE space) + hooks for game modes and bots (mh-mode) and replication (mordhau-core NetHooks, mh-net).
//! Hosts (Bevy mh-runtime, the C ABI, later SKSE) feed `SimInput`s and poses, call `step`, read `snapshot` / `drain`.
//!
//!   spec.rs     data::Spec from the spec matrix (mh-spec, f32) + paks (curves, sockets) + PDB / .rdata dumps
//!   physics.rs  PhysicsAsset body shapes, sockets, BlockCollider (UE cm) + segment-vs-shape tests
//!   pose.rs     reference skeleton, component-space poses (reference / AnimSequence sample), the gripped weapon
//!   trace.rs    AMordhauWeapon tracers + SampleTracers in UE space (a mordhau-core TraceHost)
//!   sim.rs      the facade and its frame order
//!   load.rs     loading it all from the user's install

pub mod additive;
pub mod air;
pub mod anim;
pub mod animgraph;
pub mod blendspace;
pub mod climb;
pub mod events;
pub mod flinch;
pub mod grip;
pub mod grounding;
pub mod load;
pub mod lower;
pub mod mode;
pub mod noise;
pub mod pawns;
pub mod physics;
pub mod pose;
pub mod procedural;
pub mod ranged;
pub mod ragdoll;
pub mod horse;
pub mod sim;
pub mod spec;
pub mod trace;

pub use sim::{FighterDesc, PoseSource, Sim, SimInput};

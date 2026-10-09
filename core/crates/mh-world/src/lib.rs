//! mh-world: the behaviour of Mordhau's placed gameplay actors (doors, destructibles, Frontline objectives, spawners,
//! ladders, pushables, the capture points' objective layer) as engine-neutral state machines, ported from their Blueprint bytecode (`python scripts/world_kismet.py`
//! -> state/world_kismet/<BP>.txt; cited as `BP_Door:ExecuteUbergraph@N`, N = the in-memory statement index jumps
//! use) and the native classes they call. The data (classes, properties, transforms) is mh-level's
//! `LevelData::gameplay`; class defaults are the Blueprint CDO values merged over the chain, instance properties win.
//!
//! The host drives it: `World::from_level`, `begin_play`, then per frame `interact` / `held_interact` /
//! `apply_damage` / `tick` with a `Queries` implementation (the overlap tests the Blueprints make), and applies the
//! returned `WorldEvent`s (character knockback / ragdoll / damage, component transforms, collision on/off, mesh
//! swaps, spawns, scores, objective changes). Server (authority) behaviour; cosmetic sound / particle / widget nodes
//! are not ported.

pub mod capture;
pub mod destructible;
pub mod door;
#[cfg(feature = "mode")]
pub mod fl_bridge;
pub mod frontline;
pub mod hazards;
pub mod interaction;
pub mod ladder;
pub mod pickups;
pub mod progress;
pub mod props;
pub mod pushable;
pub mod replication;
pub mod spawner;
pub mod spline;
pub mod world;

pub use world::{ActorId, CharId, CharView, DamageInfo, Kind, Queries, SpawnedStatus, World, WorldActor, WorldEvent};

pub type V = [f64; 3];

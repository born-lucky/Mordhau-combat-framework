//! mh-character: Mordhau character movement, engine-neutral (state/rust_brief.md; owner rust-character).
//!
//! Default (exe-exact) mode, `exe` + `exe_cmc`: the movement as the shipped exe runs it, in binary32, in UE axes, read
//! off the disassembly of UMordhauMovementComponent / UAdvancedCharacterMovement / AMordhauCharacter and the statically
//! linked UE 4.26 UCharacterMovementComponent (walking with floor finding, step-up, sliding, perching, falling with
//! landing, crouch with encroachment tests), moving a capsule against a host-implemented `world::World`.
//!
//! `reference_compat` feature, `compat`: the GDScript reference (godot/game/character/**) reproduced bit for bit for the
//! golden traces (tests/golden.rs). The `exe` module docs list every difference between the two.
//!
//! Plain data + deterministic fixed-step tick functions: no engine types, no I/O (record sources parse strings handed to
//! them), no globals. Every rule cites the exe function it comes from (rva + PDB name).
//!
//! Sim-facade API (exe mode): `records::CharacterSource` -> `CharacterRecords` -> `ExeMovement::new(&rec, location)`;
//! per frame `ExeMovement::frame(&world, dt, &ExeInput)`; events in `fall_damage` / `jumps` / `trips` /
//! `ragdoll_changes` / `landings` (drain them, as Fighter.apply_movement_events does); one-off calls: `knockback`,
//! `trip`; fields `motion_restriction`, `dead`, `authority`, `toggle_sprint`, `toggle_crouch`, `armor_speed`,
//! `armor_accel`, `dwarf_speed_modifier`.
//!
//! Projectiles (`projectile`): `ProjectileCfg::from_json_chain` (class chain JSONs) -> `Projectile::fire` (rotator)
//! / `fire_quat` (socket quaternion) -> per frame `tick(&world, &targets, dt, gravity_z)` -> `ProjectileHit`s in order,
//! resolved by the combat side with `stick` / `terminate` / nothing (pass through); `will_sticky` / `will_pass_through`.
//! World hits use `World::sweep_box` with the BoxComp's channel list (`ProjectileCfg::box_responses`).
//! Siege engines (`siege`): `SiegeEngine::from_bp_json` (BP_Ballista / BP_Catapult) -> per frame `frame(dt,
//! &SiegeInput)` -> `ShooterEvent`s (`Spawn { socket, fire, arm }`: fire the class at the posed socket with
//! `Projectile::fire_quat`, the catapult's speed through `catapult_initial_speed`); `VehicleAim` (turn / look caps and
//! limits), `Shooter` (stage machine), `VehicleNet` (AVehicleBase::SyncPhysics).

pub mod exe;
pub mod exe_climb;
pub mod exe_cmc;
pub mod exe_horse;
pub mod exe_ladder;
pub mod exe_lod;
pub mod exe_pseudo;
pub mod equipment;
pub mod projectile;
pub mod records;
pub mod siege;
#[cfg(feature = "spec")]
pub mod spec_source;
pub mod ue;
pub mod uemath;
pub mod uequat;
pub mod world;

#[cfg(feature = "reference_compat")]
pub mod compat;

/// EMovementRestriction (LF_ENUM in the PDB; godot/game/combat/combat_enums.gd MovementRestriction)
pub mod restriction {
    pub const NONE: i64 = 0;
    pub const PARTIAL_SPRINT: i64 = 1;
    pub const WALK: i64 = 2;
    pub const NO_MOVEMENT: i64 = 3;
}

pub use exe_lod::OtherPawn;
pub use exe::{ExeInput, ExeMovement, FloorResult, Mode, Sprint};
pub use records::{CharacterRecords, CharacterSource, ExtractJson, RecordsJson};
pub use world::{BoxWorld, HitResult, World};

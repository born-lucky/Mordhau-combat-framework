//! reference_compat mode: the GDScript reference (godot/game/character/mordhau_movement.gd, MordhauCharacter.step_model,
//! Fighter.apply_movement_events) reproduced bit for bit, for the golden traces (tests/golden.rs). Built only with the
//! `reference_compat` feature. The default, exe-exact port is crate::exe; its module docs list every
//! place the two differ.

pub mod character;
pub mod movement;

pub use character::{apply_movement_events, CharacterInput, CombatSide, StepOut};
pub use movement::{Mode, MordhauMovement, Sprint};

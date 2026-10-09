//! mh-mode: the engine-neutral Mordhau game modes and bots, ported from the cited GDScript reference
//! (godot/game/mode/**, godot/game/ai/**), which itself ports the shipped exe through the decomp (extract/native/decomp,
//! Ghidra + PDB) and the Blueprint bytecode (scripts/kismet). Every rule keeps the reference's citation (`rva=0x...` +
//! PDB name, Blueprint function + statement index, `.rdata` address, package path); unsourced rules say UNCONFIRMED.
//!
//! Plain data + deterministic fixed-step tick functions; no engine types, no I/O (records come in as serde JSON text
//! from a host), no globals. Floats follow mordhau-core::ue: GDScript `float` = f64, Godot Vector3 = FVector (f32).
//!
//!   game_mode     MordhauGameMode (AMordhauGameMode + BP_MordhauGameMode + the game state's server fields)
//!   modes         the mode Blueprints' overrides: FFA, TDM, SKM (rounds), DU / TF (rooms), FL (tickets)
//!   control_point AControlPoint + BP_SkirmishCapturePoint
//!   spawn_queue   AMordhauGameMode's spawn queue
//!   mode_table    the ported modes, one row each (ModeTable)
//!   data / kismet the mode records (ModeData) and Blueprint bytecode literals (mode_kismet.json)
//!   consts        the .rdata literals (ModeConstants, BotConstants)
//!   ai            AMordhauAIController, UBotBehaviorProfile, the behavior tree and its tasks
//!   horde / horde_extras / demon_horde / br   the Blueprint-only modes (bytecode ports)
//!   ue_math       the exe's own float math (FMath::SinCos, VectorSinCos, Atan2, Fmod, InvSqrt, the UCRT acosf)

pub mod ai;
pub mod br;
pub mod horde;
pub mod kismet_rand;
pub mod consts;
pub mod control_point;
pub mod data;
pub mod spec_load;
pub mod spec_horde;
pub mod spec_bots;
pub mod rule_ids;
pub mod event;
pub mod game_mode;
pub mod kismet;
pub mod mode_table;
pub mod modes;
pub mod sim;
pub mod spawn_queue;
pub mod ue_math;
pub mod views;
pub mod demon_horde;
pub mod horde_extras;

pub use control_point::ControlPoint;
pub use data::ModeData;
pub use event::{Arg, Ev};
pub use game_mode::{Ctrl, CtrlId, GameMode, MatchState};
pub use kismet::Kismet;

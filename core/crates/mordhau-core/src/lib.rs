//! mordhau-core: the engine-neutral Mordhau simulation core (docs/RUST_CORE.md).
//!
//! Plain data + deterministic fixed-step tick functions; no engine types, no I/O except the spec loader in `data`
//! (which reads a JSON string handed to it), no globals. Ported from the cited GDScript reference in godot/game/**,
//! which itself ports the shipped exe through the decomp (extract/native/decomp, Ghidra + PDB). Every rule keeps the
//! reference's citation (`rva=0x...` + PDB name); unsourced rules say UNCONFIRMED.
//!
//! Modules mirror godot/game/: `combat` (ported, golden-trace parity), `character`, `mode`, `ai`, `net` (other
//! crates: mh-character, mh-mode, mh-net; here only the hooks combat needs). `ue` is the shared UE math.

pub mod ue;
pub mod data;
pub mod timing;
pub mod combat;
pub mod snapshot;

pub mod character {
    //! Character / movement rules live in the mh-character crate (rust-character). The combat core reads the
    //! character's class defaults through `data::Character` and exposes the per-fighter flags movement feeds
    //! (airborne, airborne_time, look_up_value) on `combat::Fighter`.
}
pub mod mode {
    //! Game modes + AI live in the mh-mode crate (rust-mode-ai). Combat takes the running mode's damage rules as
    //! `data::ModeRules` (CombatState.set_game_mode in the reference).
}
pub mod ai {
    //! Bots live in mh-mode. They drive fighters through the same input API as players (`combat::World::input_*`)
    //! and draw from `ue::CrtRand`.
}
pub mod net {
    //! Networking lives in mh-net. The combat core is the authority-only path of the reference (CombatState with
    //! net_rep null, role 3); `combat::Fighter::role` / `remote_controlled` are kept for that crate's hooks.
}

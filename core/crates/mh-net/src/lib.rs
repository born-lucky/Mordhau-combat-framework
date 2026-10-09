//! mh-net: Mordhau's networking rules, ported from the reference GDScript port (godot/game/net/**), which ports them
//! from the shipped exe (extract/native/decomp, every rule cited by rva + PDB name). Engine-neutral: plain data and
//! deterministic functions, no Bevy, no globals; the only I/O is the optional UDP transport (std::net).
//!
//! Map (GDScript -> Rust):
//!   net_enums.gd -> enums, net_constants.gd -> consts, net_stat.gd -> stat, net_player_state.gd -> player_state,
//!   MotionSystem.NetMotion + NetMotionRep statics -> netmotion, net_motion_rep.gd -> rep (+ pawn: the seam to the
//!   combat crate, attack ping compensation / lag), net_rep_layout.gd -> layout, net_loopback.gd +
//!   net_enet_transport.gd -> transport (Loopback, UdpEndpoint, UdpLocal), net_server.gd -> server, net_client.gd ->
//!   client, net_session.gd -> session, net_game_session.gd -> game_session, net_beacon.gd -> beacon,
//!   net_duel_match.gd -> duel_match. CombatState is reached through `world::NetWorld` / `pawn::Pawn`.

pub mod beacon;
pub mod client;
pub mod consts;
pub mod core_world;
pub mod duel_match;
pub mod enums;
pub mod game_session;
pub mod layout;
#[cfg(feature = "mode")]
pub mod mode_adapter;
pub mod movement;
pub mod node;
pub mod msg;
pub mod netmotion;
pub mod pawn;
pub mod player_state;
pub mod quant;
pub mod relevancy;
pub mod rep;
pub mod server;
pub mod session;
#[cfg(feature = "sim")]
pub mod sim_server;
pub mod stat;
pub mod transport;
pub mod ue;
pub mod wire;
pub mod world;
pub mod world_rep;

pub use client::NetClient;
pub use msg::Msg;
pub use netmotion::FNetMotion;
pub use pawn::{LinkCtx, NetSlot, Pawn};
pub use rep::NetMotionRep;
pub use server::NetServer;
pub use session::NetSession;
pub use transport::{Loopback, Transport, UdpEndpoint, UdpLocal};
pub use world::NetWorld;

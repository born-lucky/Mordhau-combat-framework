//! A server-authoritative match over a NetSession: the game mode (mh-mode's DuelMode, BP_DuelGameMode) runs on the
//! server only, driven by the session's server tick; the clients see it only through replication, as in the shipped
//! game:
//!   - pawns: the mode's "spawn_pawn" / "destroy_pawn" become NetServer::spawn_pawn / destroy_pawn (an actor channel
//!     opening / closing on every client), owned by the controller's connection
//!   - teams: AMordhauPlayerState::SetTeam on the server (NetPlayerState::set_team), ReplicatedTeam to every client
//!   - round state: BP_DuelPlayerController ReplicatedRoomGame (the mode copies the room game into it every TickRoom)
//!     to the owning client only (NetServer::replicate_controller_prop)
//!   - deaths: the server's combat "died" events -> the mode's bIsAlive (DuelMode.set_alive, as duel_match.gd does)
//! Not a game rule: the server-side glue. Port of godot/game/net/net_duel_match.gd. The mode is reached through
//! `ServerMode` (mh-mode is being written in parallel; its DuelMode implements this trait once it lands).

use std::collections::BTreeMap;

use crate::server::NetServer;
use crate::session::NetSession;
use crate::world::NetWorld;

pub const ROOM_GAME: &str = "ReplicatedRoomGame";

/// what the server-side glue needs from a game mode (MordhauGameMode in the GDScript port)
#[derive(Clone, Debug, PartialEq)]
pub enum ModeEvent {
    SpawnPawn { who: String },
    DestroyPawn { who: String },
    KillPawn { who: String },
    Team { who: String, team: i64 },
    /// any other mode event (round_won, match_finished, ...), kept for the caller
    Other { kind: String, data: serde_json::Value },
}

pub trait ServerMode {
    /// AGameModeBase::PostLogin -> the mode's K2_PostLogin, with the session's PlayerId
    fn post_login(&mut self, ctrl: &str, player_id: i64);
    fn logout(&mut self, ctrl: &str);
    fn tick(&mut self, dt: f64);
    fn drain(&mut self) -> Vec<ModeEvent>;
    fn set_alive(&mut self, ctrl: &str, alive: bool);
    /// the controller's team (MordhauGameMode.Ctrl.team)
    fn team_of(&self, ctrl: &str) -> i64;
    /// the controller's ReplicatedRoomGame (None until the mode first wrote it: Ctrl.has_replicated)
    fn replicated_room_game(&self, ctrl: &str) -> Option<serde_json::Value>;
    /// a bot's controller logs in (AMordhauGameMode's bot login: the mode's own AI controller, no connection);
    /// false: this mode has no bots
    fn login_bot(&mut self, ctrl: &str) -> bool {
        let _ = ctrl;
        false
    }
}

pub struct NetDuelMatch<M: ServerMode> {
    pub mode: M,
    pub weapon: String,
    pub left: String,
    /// client endpoint -> controller name
    pub ctrl_of: BTreeMap<u32, String>,
    /// controller name -> client endpoint
    pub endpoint_of: BTreeMap<String, u32>,
    /// every mode event, in order
    pub mode_events: Vec<ModeEvent>,
    /// the bots' controllers (server-controlled pawns: NetServer::spawn_server_pawn)
    pub bots: std::collections::BTreeSet<String>,
    deaths_seen: usize,
}

impl<M: ServerMode> NetDuelMatch<M> {
    pub fn new(mode: M, weapon: &str, left: &str) -> Self {
        NetDuelMatch { mode, weapon: weapon.into(), left: left.into(), ctrl_of: BTreeMap::new(), endpoint_of: BTreeMap::new(), mode_events: vec![], bots: Default::default(), deaths_seen: 0 }
    }

    /// a player connects with URL options and logs in: the server's login gate (NetServer::pre_login: "Server full."
    /// ...) may refuse it (None, session.last_login_error); else AGameModeBase::PostLogin -> the mode's K2_PostLogin
    pub fn join<W: NetWorld>(&mut self, s: &mut NetSession<W>, nm: &str, options: &str) -> Option<u32> {
        let id = s.connect_client(options)?;
        self.ctrl_of.insert(id, nm.into());
        self.endpoint_of.insert(nm.into(), id);
        let pid = s.client_by_id(id).map(|c| c.player_state().player_id as i64).unwrap_or(-1);
        self.mode.post_login(nm, pid); // the session's PlayerId (NetGameSession::register_player)
        Some(id)
    }

    /// a player leaves: the mode's Logout, then the connection closes (NetSession::disconnect_client: its pawn goes)
    pub fn leave<W: NetWorld>(&mut self, s: &mut NetSession<W>, nm: &str) {
        let Some(ep) = self.endpoint_of.remove(nm) else { return };
        self.mode.logout(nm);
        s.disconnect_client(ep);
        self.ctrl_of.remove(&ep);
    }

    /// what a client knows of its room (its own ReplicatedRoomGame), None before the first one arrives
    pub fn room_game<W: NetWorld>(c: &crate::client::NetClient<W>) -> Option<&serde_json::Value> {
        c.pc_props.get(ROOM_GAME)
    }

    /// one session frame with the mode on the server tick
    pub fn step<W: NetWorld>(&mut self, s: &mut NetSession<W>) {
        s.step_with(&mut |srv, t, dt| self.server_tick(srv, t, dt));
    }

    /// a remote client (another process) logged in on `srv` as endpoint `ep` (approved by NetServer::pre_login):
    /// AGameModeBase::PostLogin -> the mode's K2_PostLogin with the session's PlayerId
    pub fn join_remote<W: NetWorld>(&mut self, srv: &mut NetServer<W>, ep: u32, nm: &str, options: &str) -> i32 {
        let pid = srv.login(ep, options).player_id;
        self.ctrl_of.insert(ep, nm.into());
        self.endpoint_of.insert(nm.into(), ep);
        self.mode.post_login(nm, pid as i64);
        pid
    }

    /// a server bot joins the match (the mode logs its controller in; its pawn spawns when the mode says so)
    pub fn add_bot(&mut self, nm: &str) -> bool {
        if self.mode.login_bot(nm) {
            self.bots.insert(nm.to_string());
            return true;
        }
        false
    }

    /// the mode on the server tick (after the world step, before replicate)
    pub fn server_tick<W: NetWorld>(&mut self, srv: &mut NetServer<W>, t: &mut dyn crate::transport::Transport, dt: f64) {
        {
            // deaths of this tick (the combat "died" event) -> the mode's bIsAlive
            let deaths = srv.world.deaths();
            for d in deaths.iter().skip(self.deaths_seen) {
                if self.endpoint_of.contains_key(d) || self.bots.contains(d) {
                    self.mode.set_alive(d, false);
                }
            }
            self.deaths_seen = deaths.len();
            self.mode.tick(dt);
            for ev in self.mode.drain() {
                self.mode_events.push(ev.clone());
                match &ev {
                    ModeEvent::SpawnPawn { who } => {
                        if let Some(&ep) = self.endpoint_of.get(who) {
                            let team = self.mode.team_of(who);
                            if let Some(ps) = srv.player_states.get_mut(&ep) {
                                ps.set_team(team as i32, true);
                            }
                            srv.spawn_pawn(t, ep, who, &self.weapon, &self.left);
                        } else if self.bots.contains(who) {
                            let team = self.mode.team_of(who);
                            srv.spawn_server_pawn(t, who, &self.weapon, &self.left, team);
                        }
                    }
                    ModeEvent::DestroyPawn { who } | ModeEvent::KillPawn { who } => srv.destroy_pawn(t, who),
                    ModeEvent::Team { who, team } => {
                        if let Some(&ep) = self.endpoint_of.get(who) {
                            if let Some(ps) = srv.player_states.get_mut(&ep) {
                                ps.set_team(*team as i32, true);
                            }
                        }
                    }
                    ModeEvent::Other { .. } => {}
                }
            }
            for (ep, c) in &self.ctrl_of {
                if let Some(v) = self.mode.replicated_room_game(c) {
                    srv.replicate_controller_prop(t, *ep, ROOM_GAME, v);
                }
            }
        }
    }
}

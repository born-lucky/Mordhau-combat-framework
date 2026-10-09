//! A server and its clients in one process, stepped in lockstep frames over a transport (default: Loopback). Each
//! machine runs its own world with the same fixed step; the order inside one frame n:
//!   1. every client steps (tick n): its own inputs run on its autonomous pawn (prediction) and queue server RPCs
//!   2. the server takes the RPCs that are due and schedules them at the start of its tick n
//!   3. the server steps (tick n): RPCs, then the authoritative combat (hits, parries, damage), then the server_tick
//!      callback (a game mode adapter: NetDuelMatch)
//!   4. the server replicates: client RPCs it sent during the tick, then property updates
//!   5. every client applies what is due (clock at tick n), then takes a ping sample
//! With latency L frames a message sent in frame f is handled in frame f + L at the same point.
//! Not a game rule: the stand-in for UE's net driver tick. The frame order is read from the exe's UWorld::Tick
//! rva=0x31b0df0 (engine, statically linked): a TMulticastDelegate<float> and a TMulticastDelegate<void> broadcast at
//! 0x1431b0ff9 / 0x1431b1005, before the tick groups (the FTickTaskManagerInterface calls from 0x1431b15da), and the
//! same pair at 0x1431b18e1 / 0x1431b18ed after them: UE 4.26's TickDispatchEvent / PostTickDispatchEvent (UNetDriver::
//! TickDispatch rva=0x3248830 receives, RPCs and OnReps run there) and TickFlushEvent / PostTickFlushEvent
//! (UNetDriver::TickFlush rva=0x3248a70 sends). The delegate names are from the 4.26 source (UNCONFIRMED in the exe:
//! only the broadcast positions are). So a machine applies what it received at the start of its frame, before its own
//! inputs and ticks, and sends at the end: here a client applies the server's frame n after its own tick n and before
//! tick n + 1 (the same point), and the server runs the clients' RPCs in its tick's call phase, before the inputs.
//! Consequence (exe behaviour, not a port gap): within one server tick, a client sees its own predicted motion before
//! a remote pawn's replicated one, whatever the server's actor order - the same-tick order of events can differ
//! between machines (tests/core.rs SAME_TICK_ORDER). The server-side send rules are not in the client exe (layout.rs):
//! engine-UNCONFIRMED.
//! Ping: every client frame the client's player state takes one sample of the link's round trip (2 L frames), the
//! way the engine feeds AMordhauPlayerState::UpdatePing rva=0x1604560 with measured round trips. UNCONFIRMED: the
//! engine's sampling cadence and RTT measurement (engine code, not traced); the median rule itself is the exe's.
//! Port of godot/game/net/net_session.gd.

use crate::client::NetClient;
use crate::server::NetServer;
use crate::transport::{Loopback, Transport};
use crate::world::NetWorld;

pub struct NetSession<W: NetWorld> {
    pub dt: f64,
    pub latency_frames: u64,
    /// the round trip each ping sample reports; < 0: 2 x latency_frames x dt
    pub rtt_s: f64,
    pub transport: Box<dyn Transport>,
    pub server: NetServer<W>,
    /// endpoint = order of connection (1..; never reused after a disconnect)
    pub clients: Vec<NetClient<W>>,
    make_world: Box<dyn Fn() -> W>,
    /// (name, weapon, left, owner endpoint) of add_player pawns
    players: Vec<(String, String, String, u32)>,
    endpoints: u32,
    /// what the last refused connect_client was told (NetServer::pre_login)
    pub last_login_error: String,
}

impl<W: NetWorld> NetSession<W> {
    /// `make_world`: a fresh world with the session's fixed step (every machine gets its own)
    pub fn new(dt: f64, latency_frames: u64, make_world: Box<dyn Fn() -> W>) -> Self {
        Self::with_transport(dt, latency_frames, make_world, Box::new(Loopback::new(latency_frames)))
    }

    pub fn with_transport(dt: f64, latency_frames: u64, make_world: Box<dyn Fn() -> W>, transport: Box<dyn Transport>) -> Self {
        let server = NetServer::new(make_world());
        NetSession { dt, latency_frames, rtt_s: -1.0, transport, server, clients: vec![], make_world, players: vec![], endpoints: 0, last_login_error: String::new() }
    }

    /// A client connects with its URL options ("?Name=A?SpectatorOnly=1"): the server's login gate first; refused ->
    /// None and last_login_error says why. Approved -> add_client. Returns the client's endpoint.
    pub fn connect_client(&mut self, options: &str) -> Option<u32> {
        self.last_login_error = self.server.pre_login(options);
        if !self.last_login_error.is_empty() {
            return None;
        }
        Some(self.add_client(options))
    }

    /// A new client connection with no pawn yet (a game mode spawns pawns: NetServer::spawn_pawn). Not gated: use
    /// connect_client for a login that can be refused.
    pub fn add_client(&mut self, options: &str) -> u32 {
        let id = self.next_endpoint();
        let mut c = NetClient::new(id, (self.make_world)());
        c.world.set_clock(self.server.world.tick_n(), self.server.world.now());
        let ps = self.server.login(id, options).clone();
        // APlayerState PlayerId is an engine replicated property (its condition is not traced: UNCONFIRMED); the
        // client's copy takes it at login here
        c.player_state_mut().player_id = ps.player_id;
        c.player_state_mut().spectator = ps.spectator;
        self.clients.push(c);
        id
    }

    /// The client leaves: the server logs it out (its pawns are destroyed on every other client too).
    pub fn disconnect_client(&mut self, id: u32) {
        self.server.logout(self.transport.as_mut(), id);
        self.clients.retain(|c| c.id != id);
    }

    fn next_endpoint(&mut self) -> u32 {
        self.endpoints += 1;
        self.endpoints
    }

    /// A new client that owns pawn `nm`, present from the start on every machine (no spawn message: the r1 duel bout).
    /// Every machine adds every pawn in the same order. Returns the client's endpoint.
    pub fn add_player(&mut self, nm: &str, weapon: &str, left: &str) -> u32 {
        let id = self.next_endpoint();
        let mut c = NetClient::new(id, (self.make_world)());
        for (pn, pw, pl, po) in &self.players {
            c.add_fighter(pn, pw, pl, false, Some(*po));
        }
        c.add_fighter(nm, weapon, left, true, None);
        for other in self.clients.iter_mut() {
            other.add_fighter(nm, weapon, left, false, Some(id));
        }
        self.server.add_player(id, nm, weapon, left);
        self.players.push((nm.into(), weapon.into(), left.into(), id));
        self.clients.push(c);
        id
    }

    pub fn step(&mut self) {
        self.step_with(&mut |_, _, _| {});
    }

    /// one lockstep frame; `server_tick` runs after the server's world step, before replicate (a game mode adapter)
    pub fn step_with(&mut self, server_tick: &mut dyn FnMut(&mut NetServer<W>, &mut dyn Transport, f64)) {
        let t = self.transport.as_mut();
        for c in self.clients.iter_mut() {
            c.step(t);
        }
        self.server.receive(t);
        self.server.step(t);
        server_tick(&mut self.server, t, self.dt);
        self.server.replicate(t);
        let rtt = if self.rtt_s >= 0.0 { self.rtt_s } else { 2.0 * self.latency_frames as f64 * self.dt };
        for c in self.clients.iter_mut() {
            c.receive(t);
            c.player_state_mut().update_ping(rtt);
        }
        t.advance();
    }

    pub fn run_until(&mut self, t_end: f64) {
        while self.server.world.now() < t_end - self.dt * 0.5 {
            self.step();
        }
    }

    /// the client that owns pawn `nm`
    pub fn client(&mut self, nm: &str) -> Option<&mut NetClient<W>> {
        self.clients.iter_mut().find(|c| c.own == nm)
    }

    pub fn client_by_id(&mut self, id: u32) -> Option<&mut NetClient<W>> {
        self.clients.iter_mut().find(|c| c.id == id)
    }

    pub fn client_index(&self, nm: &str) -> Option<usize> {
        self.clients.iter().position(|c| c.own == nm)
    }
}

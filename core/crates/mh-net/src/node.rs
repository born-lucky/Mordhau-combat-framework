//! Host / join API for a real program (the Bevy runtime's R6 network phase, the mh-net-node binary): one machine per
//! process over UDP, driven by non-blocking `poll(now)` calls from the host's frame loop. Not game rules: the
//! process-level stand-in for UE's net driver (docs/RUST_NET.md). The model is UE's:
//!   - the server ticks at its own fixed rate (NetServerMaxTickRate=60, DefaultEngine.ini
//!     [/Script/OnlineSubsystemUtils.IpNetDriver], over BaseEngine.ini's 30) and is authoritative; it never waits
//!     for a client
//!   - each frame, as UWorld::Tick rva=0x31b0df0 orders it: what arrived is applied first (TickDispatch: the clients'
//!     RPCs are scheduled into the tick's call phase, their ServerMoves run at once), then the world, the mode, the
//!     pawns' movement ticks, then replication and the move answers go out (TickFlush)
//!   - a client ticks at its own rate (its host's frame rate; NetClientMaxTickRate=144, DefaultEngine.ini, caps the
//!     engine's), predicts its own pawn (combat: the motion prediction of rep.rs; movement: movement.rs saved moves)
//!     and simulates the other pawns from what the server replicates (ProxyMove: SimulateMovement on the client's own
//!     collision + smoothing)
//!   - character replication: APawn's constructor rva=0x32ddce0 sets NetUpdateFrequency 100 (+0x108, 0x1432dde13) and
//!     NetPriority 3 (+0x110), and Mordhau's character classes do not change them (no write in the AAdvancedCharacter /
//!     AMordhauCharacter / AHorse constructors, none in BP_MordhauCharacter's or BP_Horse's package): above the server
//!     rate, so a pawn is considered every server frame and its ReplicatedMovement goes out whenever it changed.
//!     Relevancy (relevancy.rs): AAdvancedCharacter::IsNetRelevantFor 0x1487a70 (byte-matched in
//!     src/Mordhau/Private/AdvancedCharacter.cpp) = "the viewer's AMordhauPlayerController SharesInstanceWith the pawn
//!     (duel rooms; the host's `relevant`), then APawn::IsNetRelevantFor rva=0x32ef010" with its distance culling
//!     (NetCullDistanceSquared 15000^2, AActor::InitializeDefaults rva=0x2e31440; the viewer = the pawn the client
//!     drives); an actor channel stays open while the pawn was relevant within RelevantTimeout 5 s and closes after
//!     (the proxy is destroyed: MovementDestroy); the pawns due on a connection go out in AAdvancedCharacter::
//!     GetNetPriority rva=0x1484b60 order within the frame's MaxInternetClientRate budget (the send loop's structure is
//!     UE 4.26's: UNCONFIRMED). Culling applies to the movement channel; the combat properties and RPCs (server.rs)
//!     still go to every client (UNCONFIRMED: the engine closes the whole actor channel).
//!   - horses (exe_horse.rs): a movement-only pawn (no combat fighter) the host spawns (`spawn_horse`); the client that
//!     drives it (`set_horse_driver`, UMordhauVehicleComponent::StartDriving possesses it) predicts it with
//!     FSavedMove_Horse moves (the gear in the compressed flags), the rest see a proxy
//!   - climbing: the owning client's AttemptClimb sends ServerSetClimbLocation (Vector_NetQuantize) then RequestClimb
//!     (AssignNetMotion of FNetMotion::Climbing through the combat world's net rules); on the authority and the owning
//!     client the current UClimbingMotion moves the capsule (movement.rs climb_drive), proxies follow ReplicatedMovement
//! Combat RPCs and replicated properties use the reliable channel (properties are re-sent until acknowledged in the
//! engine too); ServerMove and the move answers are unreliable, as in the engine.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use mh_character::exe_horse::{HorseCfg, HorseInput};
use mh_character::{CharacterRecords, ExeInput, ExeMovement, World as Level};
use mordhau_core::ue::FVector;

use crate::client::NetClient;
use crate::duel_match::{NetDuelMatch, ServerMode};
use crate::movement::*;
use crate::msg::Msg;
use crate::relevancy::*;
use crate::server::NetServer;
use crate::transport::{NetSim, Transport, UdpEndpoint};
use crate::world::NetWorld;

/// NetServerMaxTickRate=60 (DefaultEngine.ini [/Script/OnlineSubsystemUtils.IpNetDriver])
pub const NET_SERVER_MAX_TICK_RATE: f64 = 60.0;
/// NetClientMaxTickRate=144 (DefaultEngine.ini [/Script/OnlineSubsystemUtils.IpNetDriver])
pub const NET_CLIENT_MAX_TICK_RATE: f64 = 144.0;
/// AActor NetUpdateFrequency of a pawn: 100 (APawn::APawn rva=0x32ddce0, `mov dword [rbx+0x108], 0x42c80000`)
pub const PAWN_NET_UPDATE_FREQUENCY: f64 = 100.0;

/// pawn classes a MovementSpawn names
pub const KIND_CHARACTER: u8 = 0;
pub const KIND_HORSE: u8 = 1;

/// What a horse pawn is made of: its character records (exe_horse::horse_character_records), BP_Horse's exports
/// (JSON) and its HorseCfg
#[derive(Clone)]
pub struct HorseTemplate {
    pub records: CharacterRecords,
    pub bp_horse: String,
    pub cfg: HorseCfg,
}

/// The movement side of a node: the level's collision, the character records, where a pawn spawns
pub struct MovementHost {
    pub level: Box<dyn Level>,
    pub records: CharacterRecords,
    /// spawn location (capsule centre, UE cm) and yaw of a pawn (the game mode's player start)
    pub spawn: Box<dyn Fn(&str) -> (FVector, f32)>,
    pub cfg: NetMoveCfg,
    pub smooth: SmoothCfg,
    /// horses (None: this host has none)
    pub horse: Option<HorseTemplate>,
}

impl MovementHost {
    /// a movement component of the class `kind` at `loc`
    pub fn make(&self, kind: u8, loc: FVector) -> ExeMovement {
        match (kind, &self.horse) {
            (KIND_HORSE, Some(h)) => ExeMovement::new_horse(&h.records, &h.bp_horse, h.cfg.clone(), loc).expect("BP_Horse"),
            _ => ExeMovement::new(&self.records, loc),
        }
    }
}

/// what a server poll did
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ServerPoll {
    /// not yet time for the next frame (or still waiting for the first logins)
    Waiting,
    /// frame n was simulated
    Stepped(u64),
    Ended,
}

/// one actor channel (pawn x connection) of the movement replication
#[derive(Clone, Debug, Default)]
struct Chan {
    open: bool,
    sent: Option<RepMovement>,
    /// UActorChannel LastUpdateTime (server time of the last ReplicatedMovement sent)
    last_sent: f64,
    /// UActorChannel RelevantTime (server time the pawn was last relevant)
    last_relevant: f64,
    /// the AttachmentReplication last sent
    attach: Option<(String, [f32; 3], u16)>,
}

struct ServerPawn {
    gen: u32,
    kind: u8,
    m: ExeMovement,
    sp: ServerPrediction,
    /// the client that drives it (None: a horse nobody drives - the server simulates it)
    owner: Option<u32>,
    chans: BTreeMap<u32, Chan>,
    climb: NetClimb,
    /// the horse this pawn rides (attached to its mesh; its movement does not replicate)
    attached: Option<String>,
}

/// The component a rider is attached to: the horse's mesh (UMordhauVehicleComponent MeshComponent, AttachSocketName
/// None: the mesh's own transform = the capsule + CharacterMesh0's RelativeLocation rotated by the actor yaw, the
/// mesh's relative rotation taken as identity as in exe_horse.rs: UNCONFIRMED), as (location, yaw)
pub fn attach_parent_xf(m: &ExeMovement) -> (FVector, f32) {
    match &m.horse {
        Some(h) => {
            let r = mh_character::uequat::quat_rotate(m.actor_quat, h.cfg.mesh_relative);
            (FVector { x: m.location.x + r.x, y: m.location.y + r.y, z: m.location.z + r.z }, m.yaw)
        }
        None => (m.location, m.yaw),
    }
}

/// a world location relative to a parent component (location, yaw): its relative location
pub fn to_relative(parent: (FVector, f32), world: FVector) -> FVector {
    let (_, f, r) = mh_character::uequat::actor_axes(parent.1);
    let d = FVector { x: world.x - parent.0.x, y: world.y - parent.0.y, z: world.z - parent.0.z };
    FVector { x: d.x * f.x + d.y * f.y, y: d.x * r.x + d.y * r.y, z: d.z }
}

/// the inverse of `to_relative`
pub fn from_relative(parent: (FVector, f32), rel: FVector) -> FVector {
    let (_, f, r) = mh_character::uequat::actor_axes(parent.1);
    FVector { x: parent.0.x + f.x * rel.x + r.x * rel.y, y: parent.0.y + f.y * rel.x + r.y * rel.y, z: parent.0.z + rel.z }
}

/// relevancy / priority settings of the server's movement replication (relevancy.rs)
#[derive(Clone, Copy, Debug)]
pub struct RelevancyCfg {
    /// AGameNetworkManager bUseDistanceBasedRelevancy (BaseGame.ini true)
    pub distance_based: bool,
    /// a pawn's NetCullDistanceSquared (AActor::InitializeDefaults rva=0x2e31440: 15000^2; Mordhau keeps it)
    pub net_cull_distance_squared: f32,
    /// RelevantTimeout (BaseEngine.ini 5.0 s)
    pub relevant_timeout: f64,
    /// a connection's bytes per second (MaxInternetClientRate=100000)
    pub client_rate: f32,
}

impl Default for RelevancyCfg {
    fn default() -> Self {
        RelevancyCfg { distance_based: USE_DISTANCE_BASED_RELEVANCY, net_cull_distance_squared: ACTOR_NET_CULL_DISTANCE_SQUARED, relevant_timeout: RELEVANT_TIMEOUT, client_rate: MAX_INTERNET_CLIENT_RATE }
    }
}

pub struct ServerNode<W: NetWorld, M: ServerMode> {
    pub srv: NetServer<W>,
    pub m: NetDuelMatch<M>,
    pub t: UdpEndpoint,
    pub dt: f64,
    /// number of logins before the first frame (the match starts when they are in)
    pub expected_clients: usize,
    pub frame: u64,
    next_tick: f64,
    pub quit: bool,
    pub movement: Option<MovementHost>,
    pawns: BTreeMap<String, ServerPawn>,
    /// IsNetRelevantFor's instance test: (viewer endpoint, pawn) -> relevant (default: every pawn)
    pub relevant: Box<dyn Fn(u32, &str) -> bool>,
    pub relevancy: RelevancyCfg,
    pub moves_received: u64,
    /// ServerMoves for an older spawn of a pawn (dropped)
    pub stale_gen_moves: u64,
    /// movement channels closed for irrelevancy, ReplicatedMovement updates held back by the send budget
    pub channels_closed: u64,
    pub budget_deferred: u64,
    /// the placed actors' replicated state per connection (world_rep.rs)
    pub world_rep: crate::world_rep::WorldRepServer,
    events_seen: usize,
    next_gen: u32,
}

impl<W: NetWorld, M: ServerMode> ServerNode<W, M> {
    /// bind e.g. "0.0.0.0:7777" ("127.0.0.1:0" = any free local port); the world is the server's (authority);
    /// dt = 1 / NetServerMaxTickRate by default
    pub fn bind(addr: &str, world: W, mode: M, weapon: &str, left: &str, dt: f64, expected_clients: usize) -> std::io::Result<Self> {
        Ok(ServerNode {
            srv: NetServer::new(world),
            m: NetDuelMatch::new(mode, weapon, left),
            t: UdpEndpoint::bind(0, addr)?,
            dt,
            expected_clients,
            frame: 0,
            next_tick: f64::NAN,
            quit: false,
            movement: None,
            pawns: BTreeMap::new(),
            relevant: Box::new(|_, _| true),
            relevancy: RelevancyCfg::default(),
            moves_received: 0,
            stale_gen_moves: 0,
            channels_closed: 0,
            budget_deferred: 0,
            world_rep: Default::default(),
            events_seen: 0,
            next_gen: 0,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.t.local_addr().expect("bound socket")
    }

    pub fn set_sim(&mut self, sim: NetSim) {
        self.t.set_sim(sim);
    }

    /// the server's movement state of a pawn (location, velocity, ...)
    pub fn pawn_movement(&self, nm: &str) -> Option<&ExeMovement> {
        self.pawns.get(nm).map(|p| &p.m)
    }
    pub fn pawn_movement_mut(&mut self, nm: &str) -> Option<&mut ExeMovement> {
        self.pawns.get_mut(nm).map(|p| &mut p.m)
    }
    /// the server's move bookkeeping of a pawn (corrections sent, moves, ...)
    pub fn pawn_prediction(&self, nm: &str) -> Option<&ServerPrediction> {
        self.pawns.get(nm).map(|p| &p.sp)
    }
    /// whether `nm`'s movement channel to `client` is open
    pub fn channel_open(&self, nm: &str, client: u32) -> bool {
        self.pawns.get(nm).and_then(|p| p.chans.get(&client)).is_some_and(|c| c.open)
    }

    fn handle_logins(&mut self) {
        let mut keep = vec![];
        for (from, msg) in std::mem::take(&mut self.srv.control) {
            if let Msg::Join { name, options } = msg {
                // the login gate: AGameModeBase::PreLogin -> AMordhauGameSession::ApproveLogin (NetGameSession)
                let err = self.srv.pre_login(&options);
                let ok = err.is_empty();
                let pid = if ok { self.m.join_remote(&mut self.srv, from, &name, &options) } else { -1 };
                self.t.send(from, &Msg::JoinResult { error: err, player_id: pid, tick_n: self.srv.world.tick_n(), now: self.srv.world.now() }, 0);
                if ok {
                    // the pawns that already exist (bots, earlier players) open their channels for the new connection
                    self.srv.send_existing_pawns(&mut self.t, from);
                }
            } else {
                keep.push((from, msg));
            }
        }
        self.srv.control = keep;
    }

    /// TickDispatch: every received ServerMove runs now, in arrival order; ServerSetClimbLocation / ServerRequestRearing
    /// (reliable) apply in their order
    fn dispatch_moves(&mut self) {
        let Some(mh) = &self.movement else {
            self.srv.moves.clear();
            self.srv.dispatch(&mut self.t);
            return;
        };
        let msgs = std::mem::take(&mut self.srv.moves);
        // ServerSetClimbLocation first: BP_MordhauCharacter AttemptClimb calls it (statement 611 / 1009) before
        // RequestClimb (1041), both reliable on the pawn's channel
        for (_, msg) in &msgs {
            if let Msg::ServerSetClimbLocation { who, gen, target } = msg {
                if let Some(p) = self.pawns.get_mut(who).filter(|p| p.gen == *gen) {
                    p.m.climb_target_location = FVector { x: target[0], y: target[1], z: target[2] };
                }
            }
        }
        // the combat RPCs run now, at the previous frame's TimeSeconds (NetServer::dispatch); a climb they start
        // begins its movement side at once (OnBegin's MOVE_Custom runs inside ChangeMotion)
        self.srv.dispatch(&mut self.t);
        for (n, p) in self.pawns.iter_mut() {
            if p.kind == KIND_CHARACTER && p.attached.is_none() {
                let cur = self.srv.world.net_climb_motion(n);
                climb_drive(&mut p.m, mh.level.as_ref(), cur, &mut p.climb);
            }
        }
        for (_, msg) in msgs {
            let (who, gen) = match &msg {
                Msg::ServerMove { who, gen, .. } | Msg::ServerSetClimbLocation { who, gen, .. } | Msg::ServerRequestRearing { who, gen } => (who.clone(), *gen),
                _ => continue,
            };
            let Some(p) = self.pawns.get_mut(&who) else { continue };
            if p.gen != gen {
                self.stale_gen_moves += 1;
                continue;
            }
            match msg {
                Msg::ServerMove { data, .. } => {
                    if p.attached.is_some() {
                        continue; // an attached rider is not possessed: no moves are expected for it
                    }
                    let wt = p.m.world_time;
                    server_move_packet(&mut p.m, &mut p.sp, &mh.cfg, mh.level.as_ref(), &data, wt, wt);
                    self.moves_received += 1;
                }
                Msg::ServerSetClimbLocation { .. } => {}
                Msg::ServerRequestRearing { .. } => {
                    // AHorse::ServerRequestRearing_Implementation rva=0x15155b0 (+ OnRep_ReplicatedRearing on the server)
                    if p.m.horse.is_some() {
                        p.m.horse_request_rearing();
                    }
                }
                _ => {}
            }
        }
        self.follow_riders();
    }

    /// attached riders move with their horse's mesh the moment it moves (the attachment keeps their relative
    /// transform): after every horse move (the moves in TickDispatch, the frame's movement pass)
    fn follow_riders(&mut self) {
        let riders: Vec<(String, String)> = self.pawns.iter().filter_map(|(n, p)| p.attached.clone().map(|h| (n.clone(), h))).collect();
        for (rn, hn) in riders {
            let Some((seat, v)) = self.pawns.get(&hn).filter(|h| h.m.horse.is_some()).map(|h| (h.m.horse_rider_seat(), h.m.velocity)) else { continue };
            let r = self.pawns.get_mut(&rn).unwrap();
            r.m.location = seat.0;
            r.m.set_yaw(seat.1);
            r.m.velocity = v;
        }
    }

    /// pawns that appeared / left in the combat world get / lose their movement (their channels open in the flush)
    fn sync_pawns(&mut self) {
        if self.movement.is_none() {
            return;
        }
        // a (re)spawn of the mode starts a fresh pawn at its start
        for ev in self.m.mode_events.iter().skip(self.events_seen) {
            if let crate::duel_match::ModeEvent::SpawnPawn { who } = ev {
                if self.pawns.get(who).is_some_and(|p| p.kind == KIND_CHARACTER) {
                    self.pawns.remove(who);
                }
            }
        }
        self.events_seen = self.m.mode_events.len();
        let names: Vec<String> = self.srv.reps.clone();
        let gone: Vec<String> = self.pawns.iter().filter(|(n, p)| p.kind == KIND_CHARACTER && !names.contains(n)).map(|(n, _)| n.clone()).collect();
        for n in gone {
            self.remove_pawn(&n);
        }
        let mut new = vec![];
        if let Some(mh) = &self.movement {
            for n in names {
                if self.pawns.contains_key(&n) {
                    continue;
                }
                let owner = self.srv.owner_of.get(&n).copied();
                // a world that moves its own pawns (a Sim) knows where a server pawn (a bot) is; else the mode's start
                let m = match (owner, self.srv.world.net_movement(&n)) {
                    (None, Some(m)) => m,
                    _ => {
                        let (loc, yaw) = (mh.spawn)(&n);
                        let mut m = mh.make(KIND_CHARACTER, loc);
                        m.set_yaw(yaw);
                        m
                    }
                };
                new.push((n, m, owner));
            }
        }
        let dt = self.dt as f32;
        for (n, m, owner) in new {
            self.insert_pawn(n.clone(), KIND_CHARACTER, m, owner);
            // created after this frame's world step (the clock already reads T): the frame's movement pass advances
            // every pawn's clock by dt, so start it one step back to land on T with the world (TimeSeconds)
            if let Some(p) = self.pawns.get_mut(&n) {
                p.m.world_time -= dt;
                p.m.creation_time = p.m.world_time + dt;
                p.sp.last_possession_time = p.m.creation_time;
                p.sp.world_creation_time = p.m.creation_time;
                if p.owner.is_some() {
                    self.srv.world.net_set_movement(&n, &p.m);
                }
            }
        }
    }

    fn insert_pawn(&mut self, n: String, kind: u8, mut m: ExeMovement, owner: Option<u32>) {
        m.world_time = self.srv.world.now() as f32;
        m.creation_time = m.world_time;
        let mut sp = ServerPrediction::default();
        sp.last_possession_time = m.world_time;
        sp.world_creation_time = m.world_time;
        self.next_gen += 1;
        let gen = self.next_gen;
        self.pawns.insert(n, ServerPawn { gen, kind, m, sp, owner, chans: BTreeMap::new(), climb: NetClimb::default(), attached: None });
    }

    /// a pawn leaves: its open channels close (the clients destroy their copies)
    fn remove_pawn(&mut self, n: &str) {
        if let Some(p) = self.pawns.remove(n) {
            for (c, ch) in p.chans {
                if ch.open && Some(c) != p.owner {
                    self.t.send(c, &Msg::MovementDestroy { who: n.to_string() }, 0);
                }
            }
        }
        self.srv.movement_owner_of.remove(n);
    }

    /// A horse (BP_Horse; MovementHost::horse) at `loc` facing `yaw`, driven by client `driver` (None: nobody; the
    /// server simulates it). Its actor channels open in the next flush.
    pub fn spawn_horse(&mut self, name: &str, loc: FVector, yaw: f32, driver: Option<u32>) {
        let Some(mh) = &self.movement else { return };
        let mut m = mh.make(KIND_HORSE, loc);
        m.set_yaw(yaw);
        m.authority = true;
        if let Some(h) = m.horse.as_mut() {
            h.control_yaw = yaw;
        }
        self.insert_pawn(name.to_string(), KIND_HORSE, m, None);
        self.set_horse_driver(name, driver);
    }

    pub fn destroy_horse(&mut self, name: &str) {
        self.remove_pawn(name);
    }

    /// The driver of a horse changes (UMordhauVehicleComponent::StartDriving rva=0x14d79a0 possesses it / StopDriving
    /// rva=0x14d8270 unpossesses): the old driver stops predicting it, the new one starts (MovementPossess); the
    /// server's move bookkeeping restarts (LastPossessionTime: the 5 s time-discrepancy grace of
    /// UAdvancedCharacterMovement::ProcessClientTimeStampForTimeDiscrepancy 0x149b1f0)
    pub fn set_horse_driver(&mut self, name: &str, driver: Option<u32>) {
        let now = self.srv.world.now() as f32;
        let Some(p) = self.pawns.get_mut(name) else { return };
        if p.kind != KIND_HORSE {
            return;
        }
        let old = p.owner;
        p.owner = driver;
        if let Some(h) = p.m.horse.as_mut() {
            h.controlled = driver.is_some();
        }
        p.m.player_controlled = driver.is_some();
        p.sp = ServerPrediction { last_possession_time: now, world_creation_time: p.sp.world_creation_time, ..Default::default() };
        match driver {
            Some(c) => {
                self.srv.movement_owner_of.insert(name.to_string(), c);
            }
            None => {
                self.srv.movement_owner_of.remove(name);
            }
        }
        if let Some(h) = p.m.horse.as_mut() {
            h.driver = driver.map(|c| c as usize);
        }
        let gen = p.gen;
        let seat = p.m.horse.as_ref().map(|_| p.m.horse_rider_seat());
        // the riders: StartDriving puts the driver at the seat and attaches it to the horse's mesh (SetReplicateMovement
        // false, AttachToComponent KeepWorldTransform: 0x1414d79a0 body); StopDriving detaches it at the exit spot
        // (horse_stop_driving: the rider's own location through FindTeleportSpot)
        if old != driver {
            if let Some(o) = old {
                let rider = self.pawns.iter().find(|(_, r)| r.kind == KIND_CHARACTER && r.owner == Some(o) && r.attached.as_deref() == Some(name)).map(|(n, _)| n.clone());
                if let Some(rn) = rider {
                    let (loc, yaw, rad, hh) = {
                        let r = &self.pawns[&rn];
                        (r.m.location, r.m.yaw, r.m.capsule_radius(), r.m.half_height())
                    };
                    let exit = match (self.movement.as_ref(), self.pawns.get_mut(name)) {
                        (Some(mh), Some(hp)) => hp.m.horse_stop_driving(mh.level.as_ref(), loc, yaw, None, rad, hh),
                        _ => (loc, yaw),
                    };
                    let r = self.pawns.get_mut(&rn).unwrap();
                    r.attached = None;
                    r.m.location = exit.0;
                    r.m.set_yaw(exit.1);
                    r.m.velocity = FVector::ZERO;
                    r.m.force_next_floor_check = true;
                    r.sp = ServerPrediction { last_possession_time: now, world_creation_time: r.sp.world_creation_time, ..Default::default() };
                }
            }
            if let (Some(c), Some((sl, sy))) = (driver, seat) {
                let rider = self.pawns.iter().find(|(_, r)| r.kind == KIND_CHARACTER && r.owner == Some(c)).map(|(n, _)| n.clone());
                if let Some(rn) = rider {
                    let r = self.pawns.get_mut(&rn).unwrap();
                    r.attached = Some(name.to_string());
                    r.m.location = sl; // SetActorLocationAndRotation(seat) before the attach
                    r.m.set_yaw(sy);
                }
            }
        }
        let p = self.pawns.get_mut(name).unwrap();
        if let Some(o) = old {
            if Some(o) != driver && p.chans.get(&o).is_some_and(|c| c.open) {
                self.t.send(o, &Msg::MovementPossess { who: name.to_string(), gen, owned: false }, 0);
                p.chans.entry(o).or_default().sent = None; // it is a proxy there again: the next flush replicates
            }
        }
        if let Some(c) = driver {
            if Some(c) != old && p.chans.get(&c).is_some_and(|ch| ch.open) {
                self.t.send(c, &Msg::MovementPossess { who: name.to_string(), gen, owned: true }, 0);
            }
        }
    }

    /// the pawn a client views from (its view target): the horse it drives, else its character
    fn viewer_of(&self, c: u32) -> Option<(FVector, f32)> {
        if let Some(p) = self.pawns.values().find(|p| p.kind == KIND_HORSE && p.owner == Some(c)) {
            return Some((p.m.location, p.m.yaw));
        }
        self.pawns.values().find(|p| p.kind == KIND_CHARACTER && p.owner == Some(c)).map(|p| (p.m.location, p.m.yaw))
    }

    /// TickFlush, the movement part: per connection, the actor channels open / stay / close by relevancy, the due
    /// ReplicatedMovement updates go out by priority within the budget (module docs, relevancy.rs)
    fn replicate_movement(&mut self) {
        let now = self.srv.world.now();
        let clients: Vec<u32> = self.m.ctrl_of.keys().copied().collect();
        let rc = self.relevancy;
        let budget = rc.client_rate * self.dt as f32;
        for &c in &clients {
            let viewer = self.viewer_of(c);
            let mut due: Vec<(String, RepMovement, f32, u32)> = vec![];
            let names: Vec<String> = self.pawns.keys().cloned().collect();
            for n in names {
                let instance = (self.relevant)(c, &n);
                let attach = self.pawns[&n].attached.as_ref().and_then(|h| {
                    let parent = attach_parent_xf(&self.pawns.get(h)?.m);
                    let p = &self.pawns[&n].m;
                    let rel = crate::quant::net_quantize100(to_relative(parent, p.location));
                    let yaw = crate::quant::compress_axis_to_short(p.yaw - parent.1);
                    Some((h.clone(), [rel.x, rel.y, rel.z], yaw))
                });
                let p = self.pawns.get_mut(&n).unwrap();
                let r = rep_movement_of(&p.m);
                let owner = p.owner == Some(c);
                let relevant = if owner {
                    true // the pawn its controller possesses (APawn::IsNetRelevantFor: the real viewer is its Controller)
                } else {
                    let q = PawnRelevancyQuery {
                        always_relevant: false,
                        viewer_is_controller: false,
                        tied_to_viewer: false,
                        hidden_without_collision: false,
                        base_relevant: None,
                        location: p.m.location,
                        net_cull_distance_squared: if rc.distance_based { rc.net_cull_distance_squared } else { f32::INFINITY },
                    };
                    match viewer {
                        Some((vl, _)) => advanced_character_is_net_relevant_for(instance, &q, vl),
                        None => instance,
                    }
                };
                let ch = p.chans.entry(c).or_default();
                if relevant {
                    ch.last_relevant = now;
                }
                let recently = relevant || (ch.open && now - ch.last_relevant < rc.relevant_timeout);
                if !recently {
                    if ch.open {
                        ch.open = false;
                        ch.sent = None;
                        ch.attach = None;
                        self.t.send(c, &Msg::MovementDestroy { who: n.clone() }, 0);
                        self.channels_closed += 1;
                    }
                    continue;
                }
                if !ch.open {
                    // the actor channel opens: the initial replication carries the spawn state (and who drives it)
                    ch.open = true;
                    ch.sent = Some(r.clone());
                    ch.last_sent = now;
                    self.t.send(c, &Msg::MovementSpawn { who: n.clone(), gen: p.gen, r: r.clone(), kind: p.kind }, 0);
                    if owner && p.kind == KIND_HORSE {
                        self.t.send(c, &Msg::MovementPossess { who: n.clone(), gen: p.gen, owned: true }, 0);
                    }
                }
                // AttachmentReplication (AActor::GatherCurrentMovement rva=0x2e2a1b0 fills it while the root has an attach
                // parent, 0x142e2a1c0..0x142e2a1e4, also with bReplicateMovement off), to every connection
                if ch.attach != attach {
                    ch.attach = attach.clone();
                    let (parent, offset, yaw) = match &attach {
                        Some((h, o, y)) => (Some(h.clone()), *o, *y),
                        None => (None, [0.0; 3], 0),
                    };
                    self.t.send(c, &Msg::RepAttachment { who: n.clone(), parent, offset, yaw }, 0);
                }
                if p.attached.is_some() {
                    continue; // ReplicatedMovement is off while attached (StartDriving: SetReplicateMovement(false))
                }
                if ch.sent.as_ref() == Some(&r) && ch.last_sent == now {
                    continue; // just opened this frame
                }
                // ReplicatedMovement: COND_SimulatedOrPhysics - not to the autonomous proxy
                if owner || ch.sent.as_ref() == Some(&r) {
                    continue;
                }
                let time = (now - ch.last_sent) as f32;
                let prio = match viewer {
                    Some((vl, vy)) => {
                        let (_, fwd, _) = mh_character::uequat::actor_axes(vy);
                        advanced_character_get_net_priority(vl, fwd, Some(p.m.location), false, time, PAWN_NET_PRIORITY)
                    }
                    None => time * PAWN_NET_PRIORITY,
                };
                let bytes = r.wire_bits().div_ceil(8);
                due.push((n, r, prio, bytes));
            }
            let prios: Vec<f32> = due.iter().map(|d| d.2).collect();
            let costs: Vec<u32> = due.iter().map(|d| d.3).collect();
            let order = send_order(&prios, &costs, budget);
            self.budget_deferred += (due.len() - order.len()) as u64;
            for i in order {
                let (n, r, _, _) = &due[i];
                self.t.send(c, &Msg::RepMovement { who: n.clone(), r: r.clone() }, 0);
                let ch = self.pawns.get_mut(n).unwrap().chans.get_mut(&c).unwrap();
                ch.sent = Some(r.clone());
                ch.last_sent = now;
            }
        }
    }

    /// Non-blocking: pump the socket, take logins and moves, and simulate a frame when it is due (`now` = the host's
    /// clock in seconds). `before_step(srv, m)` runs right before the world step (a host's server-side hit
    /// detection).
    pub fn poll(&mut self, now: f64, before_step: &mut dyn FnMut(&mut NetServer<W>, &mut NetDuelMatch<M>)) -> ServerPoll {
        if self.quit {
            return ServerPoll::Ended;
        }
        self.srv.receive(&mut self.t);
        self.handle_logins();
        self.dispatch_moves();
        if self.m.ctrl_of.len() < self.expected_clients && self.frame == 0 {
            return ServerPoll::Waiting;
        }
        if self.next_tick.is_nan() {
            self.next_tick = now;
        }
        if now < self.next_tick {
            return ServerPoll::Waiting;
        }
        // a host that fell behind more than a few frames does not try to catch up (the engine waits for the next
        // tick slot; it never runs extra frames)
        self.next_tick = if now - self.next_tick > 4.0 * self.dt { now + self.dt } else { self.next_tick + self.dt };
        self.frame += 1;
        before_step(&mut self.srv, &mut self.m);
        // a world that moves its own pawns (a Sim) gets the remote players' movement from their ServerMoves
        for (n, p) in &self.pawns {
            if p.kind == KIND_CHARACTER && p.owner.is_some() {
                self.srv.world.net_set_movement(n, &p.m);
            }
        }
        self.srv.step(&mut self.t);
        self.m.server_tick(&mut self.srv, &mut self.t, self.dt);
        self.sync_pawns();
        let dt = self.dt as f32;
        if let Some(mh) = &self.movement {
            for (n, p) in self.pawns.iter_mut() {
                p.m.world_time += dt;
                if p.attached.is_some() {
                    continue; // follows its horse below
                }
                if p.kind == KIND_CHARACTER && p.owner.is_none() {
                    // a server bot: the world moved it (its AI's inputs); replicate that
                    if let Some(w) = self.srv.world.net_movement(n) {
                        p.m.location = w.location;
                        p.m.velocity = w.velocity;
                        p.m.set_yaw(w.yaw);
                        p.m.mode = w.mode;
                    }
                    continue;
                }
                if p.kind == KIND_CHARACTER {
                    // the climbing motion's OnTick ran in the world step (authority): its movement side
                    let cur = self.srv.world.net_climb_motion(n);
                    climb_drive(&mut p.m, mh.level.as_ref(), cur, &mut p.climb);
                }
                if p.kind == KIND_HORSE && p.owner.is_none() {
                    server_uncontrolled_horse_tick(&mut p.m, mh.level.as_ref(), dt);
                } else {
                    server_remote_pawn_tick(&mut p.m, dt);
                }
            }
        }
        self.follow_riders();
        self.srv.replicate(&mut self.t);
        // TickFlush: ReplicatedMovement to the relevant non-owners, the move answers to the owners
        self.replicate_movement();
        let default_cfg = crate::movement::NetMoveCfg::default();
        let cfg = self.movement.as_ref().map(|mh| &mh.cfg).unwrap_or(&default_cfg);
        for (n, p) in self.pawns.iter_mut() {
            let now = p.m.world_time;
            if let (Some(o), Some(resp)) = (p.owner, send_client_adjustment(&mut p.sp, now, cfg)) {
                self.t.send_unreliable(o, &Msg::MoveResponse { who: n.clone(), r: resp });
            }
        }
        self.t.pump();
        ServerPoll::Stepped(self.frame)
    }

    /// Replicate the placed actors' state (world_rep.rs): the host calls this after its world's tick (the server's
    /// mh-world World). Changed values go to every logged-in client on the reliable channel.
    pub fn replicate_world(&mut self, w: &dyn crate::world_rep::RepWorld) {
        let vars = w.rep_vars();
        let now = self.srv.world.now();
        for c in self.m.ctrl_of.keys().copied().collect::<Vec<_>>() {
            for msg in self.world_rep.diff(c, now, &vars) {
                self.t.send(c, &msg, 0);
            }
        }
    }

    /// end the session: every client is told (reliable)
    pub fn end(&mut self) {
        for c in self.m.ctrl_of.keys().copied().collect::<Vec<_>>() {
            self.t.send(c, &Msg::FrameEnd { n: self.frame, quit: true }, 0);
        }
        self.quit = true;
    }

    /// pump the socket until every datagram sent is acknowledged (or `timeout`), before the process exits
    pub fn flush(&mut self, timeout: std::time::Duration) {
        flush(&mut self.t, timeout);
    }
}

fn flush(t: &mut UdpEndpoint, timeout: std::time::Duration) {
    let t0 = std::time::Instant::now();
    while t.unacked() > 0 && t0.elapsed() < timeout {
        t.pump();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// what a client poll did
#[derive(Clone, Debug, PartialEq)]
pub enum ClientPoll {
    /// not yet time for the next frame, or waiting for the login answer
    Waiting,
    /// the login was refused (the server's error: "Server full." ...)
    Refused(String),
    /// logged in
    Joined { player_id: i32 },
    /// this client simulated its frame n
    Stepped(u64),
    /// the server ended the session
    Ended,
}

/// a movement pawn this client drives that is not its character (a horse it mounted)
pub struct OwnVehicle {
    pub m: ExeMovement,
    pub cp: ClientPrediction,
    pub gen: u32,
    /// ServerRequestRearing calls already sent
    pub rear_sent: u32,
}

pub struct ClientNode<W: NetWorld> {
    pub c: NetClient<W>,
    pub t: UdpEndpoint,
    pub dt: f64,
    pub frame: u64,
    next_tick: f64,
    pub joined: bool,
    pub ended: bool,
    pub movement: Option<MovementHost>,
    /// the own pawn's movement and prediction (once its spawn state arrived)
    pub own_move: Option<(ExeMovement, ClientPrediction)>,
    /// the own pawn's spawn number
    pub own_gen: u32,
    /// the own pawn's running climb (movement side)
    pub own_climb: NetClimb,
    /// the move statistics of the own pawn's earlier spawns
    pub past_moves: Vec<ClientPrediction>,
    /// the other pawns, simulated from ReplicatedMovement
    pub proxies: BTreeMap<String, ProxyMove>,
    /// the pawn classes of the pawns this client has a channel for
    pub kinds: BTreeMap<String, u8>,
    /// horses this client drives
    pub vehicles: BTreeMap<String, OwnVehicle>,
    /// this frame's movement input for the own pawn (the host sets it before poll)
    pub move_input: ExeInput,
    /// this frame's input for a driven horse
    pub horse_input: HorseInput,
    /// climbs this client requested (ServerSetClimbLocation + RequestClimb)
    pub climbs_requested: u32,
    /// AttachmentReplication received: pawn -> (parent, relative location, relative yaw)
    pub attachments: BTreeMap<String, (String, FVector, f32)>,
    /// placed actors' properties received and not yet applied (`apply_world`)
    pub world_props: Vec<(u32, String, i64)>,
}

impl<W: NetWorld> ClientNode<W> {
    /// open a socket (any local port), send the login (URL options "?Name=A ..."); `id` = this client's endpoint
    /// number (1..; unique per server); dt = the client's frame time
    pub fn join(server: SocketAddr, id: u32, name: &str, options: &str, world: W, dt: f64) -> std::io::Result<Self> {
        let mut t = UdpEndpoint::bind(id, if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
        t.add_peer(0, server);
        let c = NetClient::new(id, world);
        let opts = if options.is_empty() { format!("?Name={name}") } else { options.to_string() };
        t.send(0, &Msg::Join { name: name.into(), options: opts }, id);
        Ok(ClientNode {
            c,
            t,
            dt,
            frame: 0,
            next_tick: f64::NAN,
            joined: false,
            ended: false,
            movement: None,
            own_move: None,
            own_gen: 0,
            own_climb: NetClimb::default(),
            past_moves: vec![],
            proxies: BTreeMap::new(),
            kinds: BTreeMap::new(),
            vehicles: BTreeMap::new(),
            move_input: ExeInput::default(),
            horse_input: HorseInput::default(),
            climbs_requested: 0,
            attachments: BTreeMap::new(),
            world_props: vec![],
        })
    }

    pub fn set_sim(&mut self, sim: NetSim) {
        self.t.set_sim(sim);
    }

    /// the drawn location of another pawn (proxy), of the own pawn or of a driven horse
    pub fn pawn_location(&self, nm: &str) -> Option<FVector> {
        if nm == self.c.own {
            return self.own_move.as_ref().map(|(m, _)| m.location);
        }
        if let Some(v) = self.vehicles.get(nm) {
            return Some(v.m.location);
        }
        self.proxies.get(nm).map(|p| p.visual())
    }

    fn apply_movement_msgs(&mut self) {
        // the placed actors' properties do not need a movement host
        let (props, rest): (Vec<Msg>, Vec<Msg>) = std::mem::take(&mut self.c.moves).into_iter().partition(|m| matches!(m, Msg::WorldProp { .. }));
        for m in props {
            if let Msg::WorldProp { actor, var, value } = m {
                self.world_props.push((actor, var, value));
            }
        }
        self.c.moves = rest;
        let Some(mh) = &self.movement else {
            self.c.moves.clear();
            return;
        };
        for msg in std::mem::take(&mut self.c.moves) {
            match msg {
                Msg::MovementSpawn { who, gen, r, kind } => {
                    self.kinds.insert(who.clone(), kind);
                    if who == self.c.own {
                        self.own_gen = gen;
                        if let Some((_, mut cp)) = self.own_move.take() {
                            cp.saved_moves.clear();
                            self.past_moves.push(cp);
                        }
                        self.own_climb = NetClimb::default();
                        // the own pawn's spawn state (initial actor replication)
                        let mut m = mh.make(kind, FVector { x: r.location[0], y: r.location[1], z: r.location[2] });
                        m.set_yaw(r.yaw());
                        m.world_time = self.c.world.now() as f32;
                        m.creation_time = m.world_time;
                        self.own_move = Some((m, ClientPrediction::default()));
                    } else {
                        self.vehicles.remove(&who);
                        let mut m = mh.make(kind, FVector::ZERO);
                        m.world_time = self.c.world.now() as f32;
                        self.proxies.insert(who, ProxyMove::new(&r, m));
                    }
                }
                Msg::MovementPossess { who, gen, owned } => {
                    if owned {
                        // a pawn this client now drives: autonomous proxy (locally controlled)
                        let mut m = match self.proxies.remove(&who) {
                            Some(p) => p.m,
                            None => continue,
                        };
                        m.authority = true; // the "authority or locally controlled" gates (horse gear block)
                        m.player_controlled = true;
                        if let Some(h) = m.horse.as_mut() {
                            h.controlled = true;
                            h.control_yaw = m.yaw;
                        }
                        self.vehicles.insert(who, OwnVehicle { m, cp: ClientPrediction::default(), gen, rear_sent: 0 });
                    } else if let Some(v) = self.vehicles.remove(&who) {
                        let r = rep_movement_of(&v.m);
                        self.proxies.insert(who, ProxyMove::new(&r, v.m));
                    }
                }
                Msg::RepAttachment { who, parent, offset, yaw } => match parent {
                    Some(p) => {
                        let rel = FVector { x: offset[0], y: offset[1], z: offset[2] };
                        self.attachments.insert(who, (p, rel, crate::quant::decompress_axis_from_short(yaw)));
                    }
                    None => {
                        // detached: the pawn stays where it is; the server's next answer (ClientAdjustPosition for
                        // the owner, ReplicatedMovement for proxies) moves it to the exit spot
                        self.attachments.remove(&who);
                    }
                },
                Msg::MovementDestroy { who } => {
                    self.proxies.remove(&who);
                    self.vehicles.remove(&who);
                    self.kinds.remove(&who);
                }
                Msg::RepMovement { who, r } => {
                    if let Some(p) = self.proxies.get_mut(&who) {
                        p.on_rep(&r, &mh.smooth);
                    }
                }
                Msg::MoveResponse { who, r } => {
                    if who == self.c.own {
                        if let Some((m, cp)) = self.own_move.as_mut() {
                            client_handle_response(m, cp, mh.level.as_ref(), &r);
                        }
                    } else if let Some(v) = self.vehicles.get_mut(&who) {
                        client_handle_response(&mut v.m, &mut v.cp, mh.level.as_ref(), &r);
                    }
                }
                _ => {}
            }
        }
    }

    /// Non-blocking: pump the socket, apply what arrived (combat: replication / RPCs; movement: ReplicatedMovement,
    /// move answers), and simulate a frame when it is due. Before calling, the host queues this frame's combat input
    /// on `c.world` (own pawn `c.own`) and sets `move_input` (and `horse_input` while driving a horse).
    pub fn poll(&mut self, now: f64) -> ClientPoll {
        if self.ended {
            return ClientPoll::Ended;
        }
        self.c.receive(&mut self.t);
        let mut out = ClientPoll::Waiting;
        for m in std::mem::take(&mut self.c.control) {
            match m {
                Msg::JoinResult { error, player_id, tick_n, now: snow } if !self.joined => {
                    if !error.is_empty() {
                        self.ended = true;
                        return ClientPoll::Refused(error);
                    }
                    self.joined = true;
                    self.c.player_state_mut().player_id = player_id;
                    self.c.world.set_clock(tick_n, snow);
                    out = ClientPoll::Joined { player_id };
                }
                Msg::FrameEnd { quit: true, .. } => {
                    self.ended = true;
                    return ClientPoll::Ended;
                }
                _ => {}
            }
        }
        self.apply_movement_msgs();
        if !self.joined || out != ClientPoll::Waiting {
            return out;
        }
        if self.next_tick.is_nan() {
            self.next_tick = now;
        }
        if now < self.next_tick {
            return ClientPoll::Waiting;
        }
        self.next_tick = if now - self.next_tick > 4.0 * self.dt { now + self.dt } else { self.next_tick + self.dt };
        self.frame += 1;
        // ping: the reliable channel's smoothed round trip feeds AMordhauPlayerState::UpdatePing (UNCONFIRMED: the
        // engine measures it from its own packet acks)
        let rtt = self.t.srtt;
        self.c.player_state_mut().update_ping(rtt);
        self.c.step(&mut self.t);
        if let Some(mh) = &self.movement {
            let dt = self.dt as f32;
            let level = mh.level.as_ref();
            let own = self.c.own.clone();
            let own_attached = self.attachments.contains_key(&own);
            if let (Some((m, cp)), false) = (self.own_move.as_mut(), own_attached) {
                m.motion_blocks_climb = self.c.world.net_motion_blocks_climb(&own);
                let (c, t, own_climb, climbs, gen) = (&mut self.c, &mut self.t, &mut self.own_climb, &mut self.climbs_requested, self.own_gen);
                // the actor tick's part between the input and the movement tick: AttemptClimb's ServerSetClimbLocation
                // (Vector_NetQuantize) then RequestClimb (AssignNetMotion), and the climb's OnTick (climb_drive), so the
                // move saved this frame carries the climb position
                let mut actor_tick = |m: &mut ExeMovement| {
                    if let Some((target, slow)) = m.climb_request.take() {
                        let q = crate::quant::net_quantize(target);
                        t.send(0, &Msg::ServerSetClimbLocation { who: own.clone(), gen, target: [q.x, q.y, q.z] }, c.id);
                        let off = [target.x - m.location.x, target.y - m.location.y, target.z - m.location.z];
                        if c.world.net_request_climb(&own, off, slow) {
                            *climbs += 1;
                        }
                        c.flush(t);
                    }
                    let cur = c.world.net_climb_motion(&own);
                    climb_drive(m, level, cur, own_climb);
                };
                if let Some(pkt) = client_frame_with(m, cp, level, dt, &self.move_input, &mut actor_tick) {
                    self.t.send_unreliable(0, &Msg::ServerMove { who: own.clone(), gen: self.own_gen, data: pkt });
                }
            } else if let Some((m, _)) = self.own_move.as_mut() {
                m.world_time += dt; // attached: unpossessed, no moves (it follows its horse below)
            }
            for (n, v) in self.vehicles.iter_mut() {
                if let Some(pkt) = client_frame_horse(&mut v.m, &mut v.cp, level, dt, &self.horse_input) {
                    self.t.send_unreliable(0, &Msg::ServerMove { who: n.clone(), gen: v.gen, data: pkt });
                }
                let req = v.m.horse.as_ref().map_or(0, |h| h.rear_requests);
                while v.rear_sent < req {
                    v.rear_sent += 1;
                    self.t.send(0, &Msg::ServerRequestRearing { who: n.clone(), gen: v.gen }, self.c.id);
                }
            }
            let alive: Vec<String> = self.c.reps.clone();
            let kinds = &self.kinds;
            self.proxies.retain(|n, _| alive.contains(n) || kinds.get(n) == Some(&KIND_HORSE));
            if !alive.contains(&self.c.own) {
                if let Some((_, mut cp)) = self.own_move.take() {
                    cp.saved_moves.clear();
                    self.past_moves.push(cp);
                }
            }
            for p in self.proxies.values_mut() {
                p.tick(level, dt, &mh.smooth);
            }
            // attached pawns follow their parent's mesh (AActor::OnRep_AttachmentReplication rva=0x2e348d0 attaches the
            // root to the parent component with the replicated relative transform; the parent's drawn transform carries
            // the child)
            for (who, (parent, rel, yaw_off)) in self.attachments.clone() {
                let pm = match (self.vehicles.get(&parent), self.proxies.get(&parent)) {
                    (Some(v), _) => Some((attach_parent_xf(&v.m), v.m.velocity)),
                    (None, Some(p)) => {
                        let (l, y) = attach_parent_xf(&p.m);
                        Some(((FVector { x: l.x + p.offset.x, y: l.y + p.offset.y, z: l.z + p.offset.z }, y), p.m.velocity))
                    }
                    _ => None,
                };
                let Some((px, vel)) = pm else { continue };
                let loc = from_relative(px, rel);
                let yaw = px.1 + yaw_off;
                if who == self.c.own {
                    if let Some((m, _)) = self.own_move.as_mut() {
                        m.location = loc;
                        m.set_yaw(yaw);
                        m.velocity = vel;
                    }
                } else if let Some(p) = self.proxies.get_mut(&who) {
                    p.m.location = loc;
                    p.m.set_yaw(yaw);
                    p.m.velocity = vel;
                    p.offset = FVector::ZERO;
                }
            }
        }
        self.t.pump();
        ClientPoll::Stepped(self.frame)
    }

    pub fn flush(&mut self, timeout: std::time::Duration) {
        flush(&mut self.t, timeout);
    }

    /// Apply the placed actors' properties that arrived to the client's world (their OnReps run there), in order
    pub fn apply_world(&mut self, w: &mut dyn crate::world_rep::RepWorld) -> usize {
        let n = self.world_props.len();
        for (a, v, x) in std::mem::take(&mut self.world_props) {
            w.apply_rep(a, &v, x);
        }
        n
    }
}

//! sim_net.rs - the network phase (r5 item 6, docs/RUST_RUNTIME.md R6): a `SimBackend` over rust-net's free-running
//! host / join nodes (mh_net::node ServerNode / ClientNode, docs/RUST_NET.md sections 3-4) on real UDP.
//!   --host PORT|ADDR:PORT [--net-clients N]  this process runs the server (Duel: mh-mode GameMode over the DU ModeData, as
//!                                  mh-net-node does) and joins it itself over loopback as player "A" (a listen server:
//!                                  the engine's listen server has no socket between its local player and the server;
//!                                  here the local player is an ordinary client, UNCONFIRMED stand-in). The server waits
//!                                  for N logins (default 2: the local player + one remote) before its first frame.
//!   --join ADDR [--net-name B]     this process is a client of ADDR (a runtime --host or mh-net-node server).
//! The view drawn is the local client's: its own pawn predicted (movement saved moves + combat motion prediction),
//! the other pawns as simulated proxies (ProxyMove), combat state from its replicated World.
//! Data: the spec matrix + paks through mh-host (combat spec, character records, DU ModeData, Kismet), movement on the
//! map's collision (mh_level::collision::CollisionWorld, one per node), pawns spawn on the map's PlayerStarts (pawn
//! "A" on the first, the others round robin from the second).
//! Weapon: mh-net-node's longsword for every pawn (NetDuelMatch gives all pawns one weapon; a client predicts with the
//! spec it built, so host and clients must agree: UNCONFIRMED stand-in for the loadout replication).
//! Not ported here (UNCONFIRMED): the server's weapon-trace hit detection (mh-net-node scripts contacts; the runtime's
//! traces live in mh-sim, which this backend does not run), bots, poses (the bodies play the idle clip).

use crate::sim::{FighterView, FrameInput, SimBackend};
use mh_mode::{GameMode, Kismet};
use mh_net::node::{ClientNode, ClientPoll, MovementHost, ServerNode, ServerPoll, NET_SERVER_MAX_TICK_RATE};
use mordhau_core::combat::{Input, World};
use mordhau_core::ue::FVector;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// the weapon of every net pawn (mh-net-node's LS, core/crates/mh-net/src/bin/mh-net-node.rs)
pub const NET_WEAPON: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
/// the client's frame time: 60 Hz like mh-net-node's clients (NetClientMaxTickRate 144 caps it, node.rs)
const CLIENT_DT: f64 = 1.0 / 60.0;

#[derive(Clone, Debug)]
pub enum NetRole {
    Host { bind: String, clients: usize },
    Join { addr: String },
}

pub struct NetBackend {
    pub matrix: Arc<mh_spec::Spec>,
    pub role: NetRole,
    pub name: String,
    pub server: Option<ServerNode<World, GameMode>>,
    pub client: Option<ClientNode<World>>,
    pub cam_world: Option<mh_level::collision::CollisionWorld>,
    t0: Instant,
    /// fighter id -> pawn name (id 0 = the own pawn)
    pub ids: Vec<String>,
    pub log: Vec<serde_json::Value>,
    pub server_frames: u64,
    pub client_frames: u64,
    pub player_id: Option<i32>,
    pub error: Option<String>,
    pub bound: Option<String>,
}

impl NetBackend {
    pub fn new(matrix: Arc<mh_spec::Spec>, role: NetRole, name: &str) -> NetBackend {
        NetBackend {
            matrix,
            role,
            name: name.into(),
            server: None,
            client: None,
            cam_world: None,
            t0: Instant::now(),
            ids: vec![name.into()],
            log: Vec::new(),
            server_frames: 0,
            client_frames: 0,
            player_id: None,
            error: None,
            bound: None,
        }
    }

    fn id_of(&mut self, pawn: &str) -> u32 {
        if let Some(i) = self.ids.iter().position(|n| n == pawn) {
            return i as u32;
        }
        self.ids.push(pawn.into());
        (self.ids.len() - 1) as u32
    }
}

/// which start a pawn spawns on: "A" (the host's local player) the first, the others one of the rest by name
/// (UNCONFIRMED stand-in for the mode's ChoosePlayerStart)
fn start_index(n: &str, len: usize) -> usize {
    if n == "A" || len < 2 {
        0
    } else {
        1 + n.bytes().map(|b| b as usize).sum::<usize>() % (len - 1)
    }
}

/// PlayerStart (capsule centre, UE cm) and yaw (degrees) of every start of the map, in LevelData order
fn starts(d: &mh_level::LevelData) -> Vec<(FVector, f32)> {
    d.spawns
        .iter()
        .map(|s| {
            let t = s.xf.translation();
            // the actor's +X axis (column 0 of the UE-space transform) gives the yaw
            let yaw = (s.xf.m[1][0]).atan2(s.xf.m[0][0]).to_degrees() as f32;
            (FVector::new(t[0] as f32, t[1] as f32, t[2] as f32), yaw)
        })
        .collect()
}

impl SimBackend for NetBackend {
    fn name(&self) -> &'static str {
        match self.role {
            NetRole::Host { .. } => "mh-net host",
            NetRole::Join { .. } => "mh-net client",
        }
    }

    fn attach_level(&mut self, vfs: Option<&Arc<mh_pak::Vfs>>, map: &str) -> Result<(), String> {
        let vfs = vfs.ok_or("mh-net needs the paks")?.clone();
        let data = mh_host::Data { matrix: (*self.matrix).clone(), vfs: vfs.clone() };
        let spec = Rc::new(data.combat_spec(&[NET_WEAPON])?);
        let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
        let d = mh_level::read(&pk, map);
        let src = mh_assets::pak_source::PakSource::new(vfs.clone());
        let tris = |pkg: &str| -> Option<Vec<[f64; 3]>> {
            let m = mh_assets::static_mesh::lod0(&src, pkg).ok()?;
            Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
        };
        let st = Rc::new(starts(&d));
        if st.is_empty() {
            return Err(format!("{map}: no PlayerStarts"));
        }
        let records = data.character_records()?;
        let own_name = self.name.clone();
        let mk_host = |records: mh_character::CharacterRecords| -> MovementHost {
            let st = st.clone();
            let own = own_name.clone();
            MovementHost {
                level: Box::new(mh_level::collision::CollisionWorld::build(&pk, &d, Some(&tris))),
                records,
                // pawn "A" (the host's local player / the first) on the first start, the others on the next ones by name
                spawn: Box::new(move |n: &str| {
                    let _ = &own;
                    st[start_index(n, st.len())]
                }),
                cfg: Default::default(),
                smooth: Default::default(),
                horse: None,
            }
        };
        let addr = match &self.role {
            NetRole::Host { bind, clients } => {
                let dt = 1.0 / NET_SERVER_MAX_TICK_RATE;
                let mode = GameMode::new(data.mode_data("DU")?, Arc::new(Kismet::from_json(&data.kismet_json()?)?), mordhau_core::ue::CrtRand::new(1));
                let mut srv = ServerNode::bind(bind, World::new(spec.clone(), dt), mode, NET_WEAPON, "", dt, *clients)
                    .map_err(|e| format!("bind {bind}: {e}"))?;
                srv.movement = Some(mk_host(records.clone()));
                let a = srv.local_addr();
                self.bound = Some(a.to_string());
                self.server = Some(srv);
                format!("127.0.0.1:{}", a.port())
            }
            NetRole::Join { addr } => addr.clone(),
        };
        let sa: std::net::SocketAddr = addr.parse().map_err(|e| format!("{addr}: {e}"))?;
        // endpoint id: unique per server (mh-net-node clients use --id 1, 2): the host's own player takes 1, a joining
        // runtime 2 + its process id mod 1000 (UNCONFIRMED: the engine's net driver numbers connections itself)
        let id = if self.server.is_some() { 1 } else { 2 + std::process::id() % 1000 };
        let mut c = ClientNode::join(sa, id, &self.name, "", World::new(spec, CLIENT_DT), CLIENT_DT).map_err(|e| format!("join {addr}: {e}"))?;
        c.movement = Some(mk_host(records));
        // the control yaw starts at the own pawn's start (the spawn state arrives a frame later; a default 0 would turn
        // the pawn on its first move)
        c.move_input.yaw = st[start_index(&self.name, st.len())].1;
        self.client = Some(c);
        self.cam_world = Some(mh_level::collision::CollisionWorld::build(&pk, &d, Some(&tris)));
        self.log.push(serde_json::json!({"t": self.t0.elapsed().as_secs_f64(), "attached": map, "server": self.bound, "client_to": addr, "endpoint": id, "starts": st.len()}));
        Ok(())
    }

    /// the own pawn is the server's to spawn (the mode's RestartPlayer); the runtime's spawn request maps to it
    fn spawn(&mut self, _loc: [f32; 3], _yaw: f32, _team: Option<i64>) -> u32 {
        0
    }

    fn tick(&mut self, _dt: f32, inputs: &HashMap<u32, FrameInput>) {
        let now = self.t0.elapsed().as_secs_f64();
        if let Some(s) = &mut self.server {
            // a frame when due; the host's frame loop never blocks (ServerNode::poll)
            if let ServerPoll::Stepped(n) = s.poll(now, &mut |_, _| {}) {
                self.server_frames = n;
            }
        }
        let Some(c) = &mut self.client else { return };
        let own = c.c.own.clone();
        // the control yaw: the input's, else the pawn's own (ExeInput.yaw is the actor yaw applied each frame; a
        // default 0 would turn the pawn)
        let own_yaw = c.own_move.as_ref().map(|(m, _)| m.yaw);
        if inputs.get(&0).and_then(|i| i.yaw).is_none() {
            if let Some(y) = own_yaw {
                c.move_input.yaw = y;
            }
        }
        if let Some(i) = inputs.get(&0) {
            let mi = &mut c.move_input;
            mi.fwd = i.fwd;
            mi.right = i.right;
            mi.jump = i.jump;
            mi.sprint = i.sprint;
            mi.crouch = i.crouch;
            if let Some(y) = i.yaw {
                mi.yaw = y;
            }
            if !own.is_empty() && c.c.world.fighter_index(&own).is_some() {
                let w = &mut c.c.world;
                if i.release_block {
                    w.input(Input::ReleaseBlock { who: own.clone() });
                }
                if let Some(bt) = i.parry {
                    w.input(Input::Parry { who: own.clone(), bt });
                }
                if i.feint {
                    w.input(Input::Feint { who: own.clone() });
                }
                if let Some((mv, angle)) = i.attack {
                    w.input(Input::Attack { who: own.clone(), mv, angle });
                }
                if i.toggle_mode {
                    w.input(Input::ToggleMode { who: own.clone() });
                }
            }
        }
        match c.poll(now) {
            ClientPoll::Joined { player_id } => {
                self.player_id = Some(player_id);
                self.log.push(serde_json::json!({"t": now, "joined": player_id}));
            }
            ClientPoll::Refused(e) => {
                self.log.push(serde_json::json!({"t": now, "refused": e}));
                self.error = Some(e);
            }
            ClientPoll::Stepped(n) => self.client_frames = n,
            ClientPoll::Ended => {
                if self.error.is_none() {
                    self.error = Some("session ended".into());
                    self.log.push(serde_json::json!({"t": now, "ended": true}));
                }
            }
            ClientPoll::Waiting => {}
        }
        // new pawns get the next fighter ids (own = 0)
        let names: Vec<String> = self.client.as_ref().map(|c| c.c.reps.clone()).unwrap_or_default();
        for n in names {
            self.id_of(&n);
        }
    }

    fn ticks(&self) -> u64 {
        self.client_frames
    }

    fn fighters(&self) -> Vec<FighterView> {
        let Some(c) = &self.client else { return Vec::new() };
        // ids are stable: own = 0, others in order of first sight (tick registers them; a name not registered yet gets
        // the index tick will give it)
        let mut ids = self.ids.clone();
        let mut out = Vec::new();
        for n in c.c.reps.iter() {
            let Some(loc) = c.pawn_location(n) else { continue };
            let id = match ids.iter().position(|x| x == n) {
                Some(i) => i,
                None => {
                    ids.push(n.clone());
                    ids.len() - 1
                }
            };
            let (yaw, vel, falling) = if *n == c.c.own {
                c.own_move.as_ref().map(|(m, _)| (m.yaw, m.velocity, m.mode == mh_character::Mode::Falling)).unwrap_or((0.0, FVector::ZERO, false))
            } else {
                c.proxies.get(n).map(|p| (p.m.yaw, p.m.velocity, p.m.mode == mh_character::Mode::Falling)).unwrap_or((0.0, FVector::ZERO, false))
            };
            let fi = c.c.world.fighter_index(n);
            let f = fi.map(|i| &c.c.world.fighters[i]);
            out.push(FighterView {
                id: id as u32,
                team: f.map(|f| f.team),
                loc: [loc.x, loc.y, loc.z],
                vel: [vel.x, vel.y, vel.z],
                yaw,
                half_height: 96.0, // AMordhauCharacter ctor CapsuleHalfHeight 96 (mh-character exe.rs)
                falling,
                crouched: false,
                look_up: f.map(|f| f.look_up_value as f32).unwrap_or(0.0),
                state: fi.map(|i| crate::sim_core::state_name(&c.c.world, i)).unwrap_or_default(),
                health: f.map(|f| f.health as f32).unwrap_or(0.0),
                stamina: f.map(|f| f.stamina).unwrap_or(0),
            });
        }
        out
    }

    fn combat(&self) -> Option<&World> {
        self.client.as_ref().map(|c| &c.c.world)
    }

    fn fighter_index(&self, id: u32) -> Option<usize> {
        let c = self.client.as_ref()?;
        c.c.world.fighter_index(self.ids.get(id as usize)?)
    }

    fn id_of(&self, name: &str) -> Option<u32> {
        self.ids.iter().position(|n| n == name).map(|i| i as u32)
    }

    fn weapon_path(&self, id: u32) -> Option<String> {
        self.fighters().iter().any(|v| v.id == id).then(|| NET_WEAPON.to_string())
    }

    fn camera_sweep(&self, start: [f32; 3], end: [f32; 3], radius: f32) -> Option<([f32; 3], bool, String)> {
        let w = self.cam_world.as_ref()?;
        // as MhSim::camera_sweep: ECC_WorldStatic objects (ComputeCameraPOV rva 0x14b5520)
        let f = |b: u32| w.bodies.get(b as usize).is_some_and(crate::sim::camera_blocker);
        let h = w.sweep(start.map(|x| x as f64), end.map(|x| x as f64), radius as f64, radius as f64, &f).into_iter().next()?;
        Some((h.location.map(|x| x as f32), h.start_penetrating, w.bodies[h.body as usize].name.clone()))
    }

    fn debug(&self) -> serde_json::Value {
        let c = self.client.as_ref();
        let s = self.server.as_ref();
        serde_json::json!({
            "role": format!("{:?}", self.role), "name": self.name, "bound": self.bound, "error": self.error,
            "player_id": self.player_id, "server_frames": self.server_frames, "client_frames": self.client_frames,
            "own": c.map(|c| c.c.own.clone()), "joined": c.map(|c| c.joined), "reps": c.map(|c| c.c.reps.clone()),
            "proxies": c.map(|c| c.proxies.keys().cloned().collect::<Vec<_>>()),
            "own_location": c.and_then(|c| c.own_move.as_ref().map(|(m, _)| [m.location.x, m.location.y, m.location.z])),
            "ping": c.map(|c| c.c.get_ping()),
            "room_game": c.and_then(|c| c.c.pc_props.get(mh_net::duel_match::ROOM_GAME).cloned()),
            "server_pawns": s.map(|s| s.srv.reps.iter().map(|n| (n.clone(), s.pawn_movement(n).map(|m| [m.location.x, m.location.y, m.location.z]))).collect::<Vec<_>>()),
            "server_clients": s.map(|s| s.m.ctrl_of.len()),
            "moves_received": s.map(|s| s.moves_received),
            "log": self.log,
        })
    }
}

/// leaving: a host ends the session (every client is told, reliable) and flushes; a client flushes what it sent
impl Drop for NetBackend {
    fn drop(&mut self) {
        if let Some(s) = &mut self.server {
            s.end();
            s.flush(std::time::Duration::from_secs(1));
        }
        if let Some(c) = &mut self.client {
            c.flush(std::time::Duration::from_millis(300));
        }
    }
}

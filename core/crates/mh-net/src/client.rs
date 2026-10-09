//! One client machine: its own copy of the world (authority false), its own pawn as the autonomous proxy (Role 2:
//! input, prediction, server RPCs) and every other pawn as a simulated proxy (Role 1: driven by replicated properties
//! only). Engine-agnostic, like NetServer. Port of godot/game/net/net_client.gd.
//!
//! Machine settings the ping rules read: GetPing = this client's own (local player controller's) AMordhauPlayerState
//! PingMedian (UMordhauUtilityLibrary::GetPing rva=0x1624d00 with bUseMedian), TimeDilation 1.0 (the AWorldSettings
//! constructor value), m.NetcodeType 0 and m.PingExtrapolationFactor 0.5 (CVar defaults, consts.rs). They reach the
//! pawns through each one's NetSlot (`sync_ctx`) before every step and every receive.

use std::collections::BTreeMap;

use crate::enums;
use crate::layout;
use crate::msg::Msg;
use crate::netmotion::FNetMotion;
use crate::pawn::{with_rep, LinkCtx, Pawn};
use crate::player_state::NetPlayerState;
use crate::rep::{NetMotionRep, Out};
use crate::transport::Transport;
use crate::world::NetWorld;

pub struct NetClient<W: NetWorld> {
    /// transport endpoint
    pub id: u32,
    pub world: W,
    pub ctx: LinkCtx,
    /// the pawn this client owns ("" = none)
    pub own: String,
    /// fighter names with a rep, in add order
    pub reps: Vec<String>,
    /// owner endpoint -> NetPlayerState (this client's copies; its own is player_states[id])
    pub player_states: BTreeMap<u32, NetPlayerState>,
    /// this client's PlayerController properties as last received (e.g. ReplicatedRoomGame)
    pub pc_props: BTreeMap<String, serde_json::Value>,
    pub pc_rep_notifies: u32,
    /// server RPCs ProcessRemoteFunction refused to send (no owning connection)
    pub dropped_rpcs: u32,
    /// session-control messages received (JoinResult / FrameEnd) for a process loop, in order
    pub control: Vec<Msg>,
    /// character-movement messages received (ReplicatedMovement / move answers), in order
    pub moves: Vec<Msg>,
}

impl<W: NetWorld> NetClient<W> {
    pub fn new(endpoint: u32, mut world: W) -> Self {
        world.set_authority(false);
        let mut player_states = BTreeMap::new();
        player_states.insert(endpoint, NetPlayerState::new());
        NetClient {
            id: endpoint,
            world,
            ctx: LinkCtx { net_mode: enums::NET_MODE_CLIENT as i64, ..Default::default() },
            own: String::new(),
            reps: vec![],
            player_states,
            pc_props: BTreeMap::new(),
            pc_rep_notifies: 0,
            dropped_rpcs: 0,
            control: vec![],
            moves: vec![],
        }
    }

    pub fn now(&self) -> f64 {
        self.world.now()
    }

    pub fn player_state(&self) -> &NetPlayerState {
        &self.player_states[&self.id]
    }

    pub fn player_state_mut(&mut self) -> &mut NetPlayerState {
        self.player_states.get_mut(&self.id).unwrap()
    }

    /// UMordhauUtilityLibrary::GetPing rva=0x1624d00 (bUseMedian): the local player state's PingMedian
    pub fn get_ping(&self) -> f64 {
        self.player_state().ping_median
    }

    fn sync_ctx(&mut self) {
        self.ctx.ping = self.get_ping();
        let ctx = self.ctx;
        for nm in &self.reps {
            self.world.set_ctx(nm, ctx);
        }
    }

    pub fn add_fighter(&mut self, nm: &str, weapon: &str, left: &str, owned: bool, owner: Option<u32>) {
        let role = if owned { enums::ROLE_AUTONOMOUS_PROXY } else { enums::ROLE_SIMULATED_PROXY };
        let owner = if owned { Some(self.id) } else { owner };
        if owned {
            self.own = nm.to_string();
        }
        let mut combat_team = None;
        if let Some(o) = owner {
            let ps = self.player_states.entry(o).or_default();
            if ps.team >= 0 {
                combat_team = Some(ps.combat_team());
            }
        }
        let ctx = LinkCtx { ping: self.get_ping(), ..self.ctx };
        let mut rep = NetMotionRep::new(nm);
        rep.owner = owner;
        self.world.add_fighter(nm, weapon, left, role, false, rep, ctx);
        if let Some(t) = combat_team {
            self.world.set_team(nm, t);
        }
        self.reps.retain(|n| n != nm);
        self.reps.push(nm.to_string());
    }

    pub fn remove_fighter(&mut self, nm: &str) {
        self.world.remove_fighter(nm);
        self.reps.retain(|n| n != nm);
        if self.own == nm {
            self.own.clear();
        }
    }

    /// UNetDriver::ProcessRemoteFunction rva=0x323cf80: a server RPC goes out only through the actor's own connection
    /// (APawn::GetNetConnection); a client has one only for the pawn it owns, so a call on any other pawn is dropped.
    pub fn send_server_rpc(&mut self, t: &mut dyn Transport, msg: &Msg) {
        if self.own.is_empty() || msg.who() != self.own {
            self.dropped_rpcs += 1;
            return;
        }
        t.send(0, msg, self.id);
    }

    /// the rules' queued server RPCs go out through the gate above (a client RPC queued here would be an error:
    /// client RPCs are sent by the authority only)
    pub fn flush(&mut self, t: &mut dyn Transport) {
        for nm in self.reps.clone() {
            let Some(rep) = self.world.rep_mut(&nm) else { continue };
            let out: Vec<Out> = rep.outbox.drain(..).collect();
            for o in out {
                if let Out::Server(m) = o {
                    self.send_server_rpc(t, &m);
                }
            }
        }
    }

    /// one client tick: the world steps (its own inputs run on the autonomous pawn: prediction), its RPCs go out
    pub fn step(&mut self, t: &mut dyn Transport) {
        self.sync_ctx();
        self.world.step(&mut |_| {});
        self.flush(t);
    }

    /// The owning client starts a dodge in direction `dir` (UE axes): ServerRequestDodge + its own OnDodged
    /// (NetMotionRep::request_dodge). Returns the packed yaw byte, None without a pawn.
    pub fn request_dodge(&mut self, t: &mut dyn Transport, dir: [f32; 2]) -> Option<u8> {
        let own = self.own.clone();
        let r = {
            let mut p = self.world.net_pawn(&own)?;
            with_rep(&mut p, |r, p| r.request_dodge(p, dir))
        };
        self.flush(t);
        r
    }

    /// The client's weapon trace hit `victim` (a cosmetic hit: clients do not process hits) -> the owning client's
    /// ServerSuggestHitDetection rule (NetMotionRep::cosmetic_hit). Returns whether it was sent.
    pub fn cosmetic_hit(&mut self, t: &mut dyn Transport, victim: &str, bone: &str) -> bool {
        let own = self.own.clone();
        let (Some(a), Some(v)) = (self.world.pawn_info(&own), self.world.pawn_info(victim)) else { return false };
        let Some(vr) = self.world.rep(victim).cloned() else { return false };
        let Some(rep) = self.world.rep_mut(&own) else { return false };
        let sent = rep.cosmetic_hit(a.role, &vr, &v, bone);
        self.flush(t);
        sent
    }

    /// Server messages due now, in order (RPCs of a frame before its property updates), applied between ticks: the
    /// client's clock already reads the server tick that sent them, so a motion the server started at tick n starts at
    /// tick n here too with no latency.
    pub fn receive(&mut self, t: &mut dyn Transport) {
        t.poll();
        self.sync_ctx();
        for (_, msg) in t.receive(self.id) {
            if msg.is_control() {
                self.control.push(msg);
                continue;
            }
            if msg.is_movement() {
                self.moves.push(msg);
                continue;
            }
            match msg {
                Msg::Spawn { name, owner, weapon, left, team } => {
                    self.add_fighter(&name, &weapon, &left, owner == self.id, Some(owner));
                    let ps = self.player_states.entry(owner).or_default();
                    if ps.replicated_team != team {
                        ps.replicated_team = team;
                        ps.on_rep_replicated_team();
                        let ct = ps.combat_team();
                        self.world.set_team(&name, ct);
                    }
                }
                Msg::Destroy { name } => self.remove_fighter(&name),
                Msg::PcProp { prop, v } => {
                    if self.pc_props.get(&prop) != Some(&v) {
                        self.pc_props.insert(prop, v);
                        self.pc_rep_notifies += 1;
                    }
                }
                Msg::ClientSetNetMotion { who, nm, t: st } => {
                    // AMordhauCharacter::ClientSetNetMotion_Implementation rva=0x1535cb0 -> OnClientSetNetMotion
                    if let Some(mut p) = self.world.net_pawn(&who) {
                        with_rep(&mut p, |r, p| r.on_client_set_net_motion(p, FNetMotion::from_bytes(nm), st));
                    }
                }
                Msg::Prop { who, prop, v } => {
                    let Some(mut p) = self.world.net_pawn(&who) else { continue };
                    let Some(mut rep) = p.take_rep() else { continue };
                    let ps = rep.owner.and_then(|o| self.player_states.get_mut(&o));
                    layout::apply(&mut rep, &mut p, &prop, &v, ps);
                    p.put_rep(rep);
                }
                _ => {}
            }
        }
        self.flush(t);
    }

    pub fn rep(&self, nm: &str) -> Option<&NetMotionRep> {
        self.world.rep(nm)
    }

    pub fn rep_mut(&mut self, nm: &str) -> Option<&mut NetMotionRep> {
        self.world.rep_mut(nm)
    }
}

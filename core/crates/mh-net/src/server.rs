//! The authority: one world (Role 3 for every pawn), the connections that own pawns, the server half of the RPCs and
//! the property replication of layout.rs. Engine-agnostic: messages go to a Transport. Port of
//! godot/game/net/net_server.gd.
//!
//! Machine settings the ping rules read: a dedicated server has no local player controller, so
//! UMordhauUtilityLibrary::GetPing rva=0x1624d00 returns 0 here (UGameplayStatics::GetPlayerController(0) is null) and
//! so does the attack's EstimatedNetworkDelay. TimeDilation is AWorldSettings +0x2e8 (1.0 from its constructor).

use std::collections::BTreeMap;

use crate::enums::{self, combat};
use crate::game_session::{get_int_option, NetGameSession};
use crate::layout::{self, PropValue};
use crate::msg::Msg;
use crate::netmotion::FNetMotion;
use crate::pawn::{with_rep, LinkCtx, Pawn};
use crate::player_state::NetPlayerState;
use crate::rep::{NetMotionRep, Out};
use crate::transport::Transport;
use crate::world::NetWorld;

pub struct NetServer<W: NetWorld> {
    pub world: W,
    pub ctx: LinkCtx,
    /// fighter names with a rep, in add order
    pub reps: Vec<String>,
    /// fighter name -> client endpoint (1..) that owns the pawn
    pub owner_of: BTreeMap<String, u32>,
    /// client endpoint -> NetPlayerState (the authority copy of each controller's player state)
    pub player_states: BTreeMap<u32, NetPlayerState>,
    /// client endpoint -> fighter -> prop -> last value sent
    shadow: BTreeMap<u32, BTreeMap<String, BTreeMap<&'static str, PropValue>>>,
    /// client endpoint -> controller prop -> last value sent
    pc_shadow: BTreeMap<u32, BTreeMap<String, serde_json::Value>>,
    /// ServerAssignNetMotion calls OnServerAssignNetMotion dropped (stale Id)
    pub rejected: u32,
    /// server RPCs that arrived from a connection that does not own the pawn
    pub not_owner: u32,
    /// login approval, player ids, slots (AMordhauGameSession)
    pub game_session: NetGameSession,
    pending: Vec<Msg>,
    pub trace: Vec<String>,
    /// session-control messages received (Join / FrameDone) for a process loop, in order: (from, msg)
    pub control: Vec<(u32, Msg)>,
    /// character-movement messages received (ServerMove), owner-checked, in arrival order: (from, msg)
    pub moves: Vec<(u32, Msg)>,
    /// movement messages for a pawn that no longer exists
    pub stale_actor: u32,
    /// pawns with movement only (no combat fighter: horses) -> the client endpoint that drives them (node.rs keeps it)
    pub movement_owner_of: BTreeMap<String, u32>,
    /// every pawn's spawn payload (owner endpoint 0 = the server: a bot), for a connection that logs in later (its
    /// actor channels open with the pawns that exist)
    pub spawn_info: BTreeMap<String, (u32, String, String, u8)>,
}

impl<W: NetWorld> NetServer<W> {
    pub fn new(mut world: W) -> Self {
        world.set_authority(true);
        let mut gs = NetGameSession::new();
        gs.net_mode = enums::NET_MODE_SERVER;
        gs.post_init_properties(0);
        NetServer {
            world,
            ctx: LinkCtx { net_mode: enums::NET_MODE_SERVER as i64, ..Default::default() },
            reps: vec![],
            owner_of: BTreeMap::new(),
            player_states: BTreeMap::new(),
            shadow: BTreeMap::new(),
            pc_shadow: BTreeMap::new(),
            rejected: 0,
            not_owner: 0,
            game_session: gs,
            pending: vec![],
            trace: vec![],
            control: vec![],
            moves: vec![],
            stale_actor: 0,
            movement_owner_of: BTreeMap::new(),
            spawn_info: BTreeMap::new(),
        }
    }

    pub fn now(&self) -> f64 {
        self.world.now()
    }

    /// The login gate, AGameModeBase::PreLogin rva=0x30e5ae0 -> AMordhauGameSession::ApproveLogin: "" = approved, else
    /// the error the engine sends the client before closing its connection (NMT_Failure, engine).
    pub fn pre_login(&self, options: &str) -> String {
        self.game_session.pre_login(options, true)
    }

    /// A client connection logs in (approved by pre_login): its controller's player state exists from now on (no pawn
    /// yet); AGameSession::RegisterPlayer gives it a PlayerId and AGameMode::PostLogin counts it. A spectator is a
    /// login with ?SpectatorOnly=1 (UNCONFIRMED: the engine's bOnlySpectator setup in AGameModeBase::Login is not
    /// traced; MustSpectate reads it).
    pub fn login(&mut self, client: u32, options: &str) -> &NetPlayerState {
        if !self.player_states.contains_key(&client) {
            let mut ps = NetPlayerState::new();
            ps.spectator = get_int_option(options, "SpectatorOnly", 0) == 1;
            self.game_session.register_player(&mut ps);
            self.game_session.post_login(ps.spectator, false);
            self.player_states.insert(client, ps);
            self.shadow.insert(client, BTreeMap::new());
            self.pc_shadow.insert(client, BTreeMap::new());
        }
        &self.player_states[&client]
    }

    /// A client connection closes: AGameMode::Logout rva=0x30e2310 counts it out; its pawns go (K2_DestroyActor of
    /// each). (AMordhauGameSession::UnregisterPlayer rva=0x15aca50 -> AGameSession::UnregisterPlayer: online session
    /// only.)
    pub fn logout(&mut self, t: &mut dyn Transport, client: u32) {
        let Some(ps) = self.player_states.get(&client) else { return };
        let spectator = ps.spectator;
        let owned: Vec<String> = self.owner_of.iter().filter(|(_, c)| **c == client).map(|(n, _)| n.clone()).collect();
        for nm in owned {
            self.destroy_pawn(t, &nm);
        }
        self.game_session.logout(spectator, false);
        self.player_states.remove(&client);
        self.shadow.remove(&client);
        self.pc_shadow.remove(&client);
    }

    /// A player's pawn on the server: Role 3, driven by a PlayerController on a remote client (not locally controlled).
    pub fn add_player(&mut self, client: u32, nm: &str, weapon: &str, left: &str) {
        self.login(client, "");
        let team = self.player_states[&client].team;
        let combat_team = self.player_states[&client].combat_team();
        let mut rep = NetMotionRep::new(nm);
        rep.owner = Some(client);
        self.world.add_fighter(nm, weapon, left, enums::ROLE_AUTHORITY, true, rep, self.ctx);
        if team >= 0 {
            self.world.set_team(nm, combat_team);
        }
        self.reps.retain(|n| n != nm);
        self.reps.push(nm.to_string());
        self.owner_of.insert(nm.to_string(), client);
    }

    /// The game mode spawns a pawn for a controller: on the server, then on every client (UE opens an actor channel and
    /// the client spawns the replicated actor; the payload here is what the world needs to build it: owner, weapon,
    /// left hand). The actor-channel mechanics are engine code (UE 4.26 UActorChannel / UPackageMapClient):
    /// engine-UNCONFIRMED.
    pub fn spawn_pawn(&mut self, t: &mut dyn Transport, client: u32, nm: &str, weapon: &str, left: &str) {
        self.add_player(client, nm, weapon, left);
        let team = self.player_states[&client].replicated_team;
        self.spawn_info.insert(nm.into(), (client, weapon.into(), left.into(), team));
        for (c, sh) in self.shadow.iter_mut() {
            sh.remove(nm);
            t.send(*c, &Msg::Spawn { name: nm.into(), owner: client, weapon: weapon.into(), left: left.into(), team }, 0);
        }
    }

    /// A pawn the server controls (an AI bot's: AMordhauAIController possesses it; no connection owns it, so no
    /// client RPCs and no owner-only properties): Role 3, not remote controlled, its channel opens on every client
    /// with owner 0. `team`: the mode's team (255 = none), the ReplicatedTeam byte the clients get.
    pub fn spawn_server_pawn(&mut self, t: &mut dyn Transport, nm: &str, weapon: &str, left: &str, team: i64) {
        let rep = NetMotionRep::new(nm);
        self.world.add_fighter(nm, weapon, left, enums::ROLE_AUTHORITY, false, rep, self.ctx);
        if team >= 0 {
            // AMordhauPlayerState team -> the combat team (NetPlayerState::combat_team: the same index)
            self.world.set_team(nm, team);
        }
        self.reps.retain(|n| n != nm);
        self.reps.push(nm.to_string());
        self.world.net_add_bot(nm);
        let tb = if team >= 0 { team as u8 } else { 255 };
        self.spawn_info.insert(nm.into(), (0, weapon.into(), left.into(), tb));
        for (c, sh) in self.shadow.iter_mut() {
            sh.remove(nm);
            t.send(*c, &Msg::Spawn { name: nm.into(), owner: 0, weapon: weapon.into(), left: left.into(), team: tb }, 0);
        }
    }

    /// A connection that logged in after pawns spawned: their actor channels open for it (the engine replicates every
    /// relevant actor to a new connection)
    pub fn send_existing_pawns(&mut self, t: &mut dyn Transport, client: u32) {
        for (nm, (owner, w, l, team)) in &self.spawn_info {
            if self.reps.contains(nm) {
                t.send(client, &Msg::Spawn { name: nm.clone(), owner: *owner, weapon: w.clone(), left: l.clone(), team: *team }, 0);
            }
        }
    }

    /// K2_DestroyActor of a pawn: gone on the server, its channel closed on every client.
    pub fn destroy_pawn(&mut self, t: &mut dyn Transport, nm: &str) {
        if !self.reps.iter().any(|n| n == nm) {
            return;
        }
        self.flush(t);
        self.world.remove_fighter(nm);
        self.reps.retain(|n| n != nm);
        self.owner_of.remove(nm);
        self.spawn_info.remove(nm);
        for (c, sh) in self.shadow.iter_mut() {
            sh.remove(nm);
            t.send(*c, &Msg::Destroy { name: nm.into() }, 0);
        }
    }

    /// The rules' queued client RPCs go to the owning connection; a pawn no client owns has none (the call would run
    /// on the server, where OnClientSetNetMotion's net-mode test drops it). A server RPC queued on the authority runs
    /// locally in the GDScript port's terms: an error, dropped.
    pub fn flush(&mut self, t: &mut dyn Transport) {
        for nm in self.reps.clone() {
            let Some(rep) = self.world.rep_mut(&nm) else { continue };
            let out: Vec<Out> = rep.outbox.drain(..).collect();
            for o in out {
                if let Out::Owner(m) = o {
                    if let Some(c) = self.owner_of.get(m.who()) {
                        t.send(*c, &m, 0);
                    }
                }
            }
        }
    }

    /// Messages from clients, applied at the start of the server's next tick (the call phase of NetWorld::step), so a
    /// motion the client started at its tick n starts at the server's tick n too with no latency.
    pub fn receive(&mut self, t: &mut dyn Transport) {
        t.poll();
        for (from, msg) in t.receive(0) {
            if msg.is_control() {
                self.control.push((from, msg));
                continue;
            }
            let who = msg.who();
            let owner = self.owner_of.get(who).or_else(|| if msg.is_movement() { self.movement_owner_of.get(who) } else { None });
            if msg.is_movement() && owner.is_none() {
                // a move for a pawn that is gone (its actor channel closed): dropped by the engine without an owner check
                self.stale_actor += 1;
                continue;
            }
            if owner != Some(&from) {
                // guard: the sending client does not own the pawn (receiver side engine-UNCONFIRMED, layout.rs)
                self.not_owner += 1;
                continue;
            }
            if msg.is_movement() {
                self.moves.push((from, msg));
                continue;
            }
            self.pending.push(msg);
        }
    }

    /// TickDispatch's RPC execution: the received client RPCs run now, on the world's current clock - the previous
    /// frame's TimeSeconds, as in the exe, where UWorld::Tick rva=0x31b0df0 broadcasts TickDispatch (0x1431b0ff9)
    /// before it advances TimeSeconds (+0x598, written at 0x1431b11e0). A motion an RPC starts gets that StartTime.
    /// The free-running node (node.rs) calls this on every receive; the lockstep NetSession keeps the reference's
    /// model (RPCs in the next step's call phase).
    pub fn dispatch(&mut self, t: &mut dyn Transport) {
        if self.pending.is_empty() {
            return;
        }
        let msgs: Vec<Msg> = self.pending.drain(..).collect();
        let mut rejected = 0u32;
        let mut trace = Vec::new();
        for m in msgs {
            apply_rpc(&mut self.world, m, &mut rejected, &mut trace);
        }
        self.rejected += rejected;
        self.trace.extend(trace);
        self.flush(t);
    }

    /// One server tick: the world steps with the received RPCs in its call phase; the rules' client RPCs go out.
    pub fn step(&mut self, t: &mut dyn Transport) {
        let pending: Vec<Msg> = self.pending.drain(..).collect();
        let mut rejected = 0u32;
        let mut trace = Vec::new();
        let mut pending = Some(pending);
        self.world.step(&mut |w: &mut W| {
            if let Some(msgs) = pending.take() {
                for m in msgs {
                    apply_rpc(w, m, &mut rejected, &mut trace);
                }
            }
        });
        self.rejected += rejected;
        self.trace.extend(trace);
        self.flush(t);
    }

    /// End of the server frame: every property layout.rs lists, to every connection its condition allows, when it
    /// differs from what that connection was last sent. First the authority's pre-replication writes:
    /// AAdvancedCharacter::PreReplication rva=0x1499d60 (ReplicatedLookUpValue) and the ReplicatedCharacterFlags bits
    /// AAdvancedCharacter::LODTick rva=0x14887b0 keeps (NetMotionRep::write_character_flags).
    pub fn replicate(&mut self, t: &mut dyn Transport) {
        self.flush(t);
        for nm in &self.reps {
            if let Some(mut p) = self.world.net_pawn(nm) {
                with_rep(&mut p, |r, p| {
                    r.write_character_flags(p);
                    r.write_look_up(p);
                });
            }
        }
        for (client, sh) in self.shadow.iter_mut() {
            for nm in &self.reps {
                let Some(rep) = self.world.rep(nm) else { continue };
                let shn = sh.entry(nm.clone()).or_default();
                let owner = self.owner_of.get(nm).copied();
                let is_owner = owner == Some(*client);
                let ps = owner.and_then(|o| self.player_states.get(&o));
                for &(prop, cond) in layout::CHARACTER_PROPS {
                    if !layout::condition_allows(cond, is_owner) {
                        continue;
                    }
                    let Some(v) = layout::value_of(rep, prop, ps) else { continue };
                    match shn.get(prop) {
                        Some(old) if *old == v => continue,
                        None if v.is_initial() => {
                            shn.insert(prop, v);
                            continue;
                        }
                        _ => {}
                    }
                    shn.insert(prop, v.clone());
                    t.send(*client, &Msg::Prop { who: nm.clone(), prop: prop.to_string(), v }, 0);
                }
            }
        }
    }

    /// A property of the client's own PlayerController (BP_DuelPlayerController ReplicatedRoomGame: package
    /// Mordhau/Content/Mordhau/Blueprints/GameModes/Duel/BP_DuelPlayerController, PropertyFlags Net | RepNotify, no
    /// condition). A PlayerController is relevant only to its owning connection (engine: APlayerController
    /// bOnlyRelevantToOwner, engine-UNCONFIRMED), so it goes to that client only, when it changed.
    pub fn replicate_controller_prop(&mut self, t: &mut dyn Transport, client: u32, prop: &str, v: serde_json::Value) {
        let Some(sh) = self.pc_shadow.get_mut(&client) else { return };
        if sh.get(prop) == Some(&v) {
            return;
        }
        sh.insert(prop.to_string(), v.clone());
        t.send(client, &Msg::PcProp { prop: prop.to_string(), v }, 0);
    }

    pub fn rep(&self, nm: &str) -> Option<&NetMotionRep> {
        self.world.rep(nm)
    }

    pub fn rep_mut(&mut self, nm: &str) -> Option<&mut NetMotionRep> {
        self.world.rep_mut(nm)
    }
}

/// a received server RPC of a pawn, run on the authority
fn apply_rpc<W: NetWorld>(w: &mut W, msg: Msg, rejected: &mut u32, trace: &mut Vec<String>) {
    match msg {
        Msg::ServerAssignNetMotion { who, nm, last } => {
            // AMordhauCharacter::ServerAssignNetMotion_Implementation rva=0x1567ab0 -> OnServerAssignNetMotion
            let Some(mut p) = w.net_pawn(&who) else { return };
            let ok = with_rep(&mut p, |r, p| r.on_server_assign_net_motion(p, FNetMotion::from_bytes(nm), last));
            if ok == Some(false) {
                *rejected += 1;
                trace.push(format!("{} ServerAssignNetMotion id {} rejected", who, nm[0]));
            }
        }
        Msg::ServerDropParry { who, id } => {
            // AMordhauCharacter::ServerDropParry_Implementation rva=0x1567ad0
            w.server_drop_parry_implementation(&who, id);
        }
        Msg::ServerSuggestHitDetection { who, other, bone } => {
            // AMordhauCharacter::ServerSuggestHitDetection_Implementation rva=0x1567d80 (OtherCharacter null -> nothing)
            if w.pawn_info(&other).is_some() {
                server_suggest_hit_detection(w, &who, &other, &bone);
            }
        }
        Msg::ServerRequestDodge { who, yaw } => {
            // AMordhauCharacter::ServerRequestDodge_Implementation rva=0x1567ca0
            if let Some(mut p) = w.net_pawn(&who) {
                with_rep(&mut p, |r, p| r.server_request_dodge(p, yaw));
            }
        }
        _ => {}
    }
}

/// The server half, AMordhauCharacter::ServerSuggestHitDetection_Implementation rva=0x1567d80 (_Validate rva=0x7bf3e0
/// returns true): OtherCharacter valid, alive, and (IsRagdollFallingOrGettingUp or bCanReceiveClientsideHits); this
/// character's MotionSystem LastAttackMotion with a Weapon; OtherCharacter not yet in the weapon's ActorSetCache ->
///   add it (ActorIgnoreCache + ActorSetCache); ComputeMeleeDamage + TakeDamage (NetWorld::suggest_hit_damage)
///   LastAttackMotion still the current motion: !Other bWillStopMelee (+0x9b1) and !AttackInfo.bStopOnHit (+0xed0) ->
///     Stage != Recovery: AssignNetMotionDynamicParam((MotionDynamicParam & 0xf) | 0x10) (`and dl, 0xf; or dl, 0x10`:
///     `AMordhauCharacter::ServerSuggestHitDetection_Implementation` at 0x14156820e); else AssignNetMotion {Blocked,
///     Param0 4 = Hit} (0x40600)
/// Bit 0x10 is not one UAttackMotion::OnDynamicParamChanged_Implementation reads (it tests 1, 2 and 4), so a suggested
/// hit does not set bHasHit.
pub fn server_suggest_hit_detection<W: NetWorld>(w: &mut W, who: &str, other: &str, bone: &str) -> bool {
    let victim_ok = match w.pawn_info(other) {
        Some(o) => !o.dead && w.rep(other).is_some_and(|r| r.accepts_suggested_hit(o.now)),
        None => false,
    };
    let reject = |w: &mut W| {
        if let Some(r) = w.rep_mut(who) {
            r.rejected_suggestions += 1;
        }
    };
    if !victim_ok {
        reject(w);
        return false;
    }
    let Some(view) = w.suggest_hit_prepare(who, other) else {
        reject(w);
        return false;
    };
    w.suggest_hit_damage(who, other, bone);
    if view.is_current {
        if !view.victim_will_stop_melee && !view.stop_on_hit {
            if !view.in_recovery {
                if let Some(mut p) = w.net_pawn(who) {
                    with_rep(&mut p, |r, p| {
                        let v = (p.net().dynamic_param & 0xf) | 0x10;
                        r.assign_net_motion_dynamic_param(p, v);
                    });
                }
            }
        } else {
            w.assign_net_blocked(who, combat::BLOCKED_HIT, 0, 0.0);
        }
    }
    true
}

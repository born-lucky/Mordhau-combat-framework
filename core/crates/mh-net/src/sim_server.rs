//! The server's world as a full simulation: mh-sim's `Sim` (rust-combat) behind `NetWorld`, so a `ServerNode<SimWorld,
//! _>` is the authority of a real match:
//!   - combat: the Sim's exe-exact World with mh-net's NetRules installed (set_authority): the clients' motion RPCs
//!     (ServerAssignNetMotion, ServerDropParry, ...) run on it in TickDispatch (NetServer::dispatch), its motions,
//!     parries, feints, hits and deaths replicate through the rep layout and ClientSetNetMotion as for any World;
//!   - hit detection on the server's posed bodies: the Sim's weapon traces (the attacker's weapon swept through the
//!     victims' physics-asset bodies, posed by the Sim's pose source) - the exe's authority-side detection, not
//!     scripted contacts. The clients do not process hits (NetWorld::set_authority(false) on theirs);
//!   - movement: a remote player's capsule is its ServerMove result (node.rs ServerPawn), copied in before the step
//!     (`net_set_movement`; mh-sim's `net_moved` keeps the Sim from moving it); a server bot is moved by the Sim from its
//!     AI's inputs and the node replicates that (`net_movement`);
//!   - bots: mh-mode's AI (BT + profile + Kismet) on every server-controlled pawn (`net_add_bot`), perceiving every
//!     fighter (a BotBody per fighter, as mh-runtime's MhSim does).
//! Not a game rule: the seam between mh-net's server and the Sim.

use std::sync::Arc;

use mh_character::ExeMovement;
use mh_sim::sim::{FighterDesc, Sim, SimInput};
use mordhau_core::combat::World;
use mordhau_core::ue::FVector;

use crate::core_world::CorePawn;
use crate::pawn::NetCtx;
use crate::rep::NetMotionRep;
use crate::world::{ClimbView, LastAttackView, NetWorld, PawnInfo};

/// what a server bot runs (mh-mode ai): the Kismet literals, the bot profile and behaviour tree, the walk speed its
/// body reports
#[derive(Clone)]
pub struct BotCfg {
    pub kismet: Arc<mh_mode::kismet::Kismet>,
    pub profile: mh_mode::ai::Profile,
    pub tree: mh_mode::ai::TreeDef,
    pub max_walk_speed: f64,
}

pub struct SimWorld {
    pub sim: Sim,
    /// where a pawn spawns (capsule centre, yaw): the mode's player start
    pub spawn: Box<dyn Fn(&str) -> (FVector, f32)>,
    pub bots: Option<BotCfg>,
    /// the pose source before each step (mh-sim's ClipPoser plays the motions' clips; None = the Sim's own: its
    /// animation graph when enabled, else the reference pose)
    pub poser: Option<mh_sim::anim::ClipPoser>,
    /// extra inputs for this step (a host's local fighters), consumed by the step
    pub inputs: Vec<(usize, SimInput)>,
    /// wall time of the last steps (s), for the host's tick budget
    pub step_times: Vec<f64>,
}

impl SimWorld {
    pub fn new(sim: Sim, spawn: Box<dyn Fn(&str) -> (FVector, f32)>) -> SimWorld {
        SimWorld { sim, spawn, bots: None, poser: None, inputs: vec![], step_times: vec![] }
    }

    fn fi(&self, who: &str) -> Option<usize> {
        self.sim.combat.fighter_index(who)
    }

    /// a BotBody for fighter `fi` (every fighter has one: the bots perceive players too)
    fn add_body(&mut self, fi: usize) {
        let Some(cfg) = &self.bots else { return };
        let mut bots = self.sim.bots.take().unwrap_or_else(|| mh_mode::ai::Bots { k: cfg.kismet.clone(), ..Default::default() });
        let mut body = mh_mode::ai::BotBody::new(&self.sim.combat.fighters[fi].name);
        body.location = self.sim.movers[fi].location;
        body.yaw = self.sim.yaw[fi] as f64;
        body.weapon_length = self.sim.combat.fighters[fi].tracer.length;
        body.max_walk_speed = cfg.max_walk_speed;
        body.pawn = mh_mode::ai::combat_host::pawn_view(&self.sim.combat, fi);
        body.weapon = mh_mode::ai::combat_host::weapon_view(&self.sim.combat, fi);
        body.inventory = vec![mh_mode::ai::InventoryItem { weapon: body.weapon.clone(), is_fists: false, ranged: false }];
        bots.add_body(body);
        self.sim.bot_fighter.push(fi);
        self.sim.bots = Some(bots);
    }
}

impl NetWorld for SimWorld {
    type P<'a> = CorePawn<'a>;

    fn now(&self) -> f64 {
        self.sim.combat.now
    }
    fn tick_n(&self) -> u64 {
        NetWorld::tick_n(&self.sim.combat)
    }
    fn set_clock(&mut self, tick_n: u64, now: f64) {
        NetWorld::set_clock(&mut self.sim.combat, tick_n, now);
    }
    fn set_authority(&mut self, authority: bool) {
        NetWorld::set_authority(&mut self.sim.combat, authority);
    }
    fn add_fighter(&mut self, name: &str, weapon: &str, left: &str, role: u8, remote_controlled: bool, rep: NetMotionRep, ctx: NetCtx) {
        let (location, yaw) = (self.spawn)(name);
        let fi = self.sim.add_fighter(&FighterDesc { name: name.into(), weapon: weapon.into(), left: left.into(), team: 255, location, yaw });
        let f = &mut self.sim.combat.fighters[fi];
        f.role = role as i64;
        f.remote_controlled = remote_controlled;
        f.net_slot.ctx = ctx;
        f.net_state = Some(Box::new(rep));
        self.add_body(fi);
    }
    fn remove_fighter(&mut self, name: &str) {
        self.sim.net_moved.remove(name);
        self.sim.remove_fighter(name);
    }
    fn net_pawn<'a>(&'a mut self, name: &str) -> Option<CorePawn<'a>> {
        NetWorld::net_pawn(&mut self.sim.combat, name)
    }
    fn pawn_info(&self, name: &str) -> Option<PawnInfo> {
        NetWorld::pawn_info(&self.sim.combat, name)
    }
    fn rep(&self, name: &str) -> Option<&NetMotionRep> {
        NetWorld::rep(&self.sim.combat, name)
    }
    fn rep_mut(&mut self, name: &str) -> Option<&mut NetMotionRep> {
        NetWorld::rep_mut(&mut self.sim.combat, name)
    }
    fn set_ctx(&mut self, name: &str, ctx: NetCtx) {
        NetWorld::set_ctx(&mut self.sim.combat, name, ctx);
    }
    fn set_team(&mut self, name: &str, team: i64) {
        NetWorld::set_team(&mut self.sim.combat, name, team);
    }
    fn fighter_names(&self) -> Vec<String> {
        NetWorld::fighter_names(&self.sim.combat)
    }

    /// The Sim's frame: the RPCs (call phase; the node dispatches them before the step already), the pose source, then
    /// Sim::step with the bots' and the host's inputs (combat clock and inputs, actor ticks, movement of the pawns the
    /// Sim moves, poses, traces, projectiles, horses, the bots' AI)
    fn step(&mut self, call_phase: &mut dyn FnMut(&mut Self)) {
        let t0 = std::time::Instant::now();
        call_phase(self);
        let mut inputs = self.sim.bot_inputs();
        inputs.extend(std::mem::take(&mut self.inputs));
        if let Some(p) = self.poser.as_mut() {
            p.pose(&mut self.sim);
        }
        self.sim.step(&inputs);
        self.step_times.push(t0.elapsed().as_secs_f64());
    }

    fn suggest_hit_prepare(&mut self, attacker: &str, victim: &str) -> Option<LastAttackView> {
        NetWorld::suggest_hit_prepare(&mut self.sim.combat, attacker, victim)
    }
    fn suggest_hit_damage(&mut self, attacker: &str, victim: &str, bone: &str) {
        NetWorld::suggest_hit_damage(&mut self.sim.combat, attacker, victim, bone);
    }
    fn assign_net_blocked(&mut self, who: &str, reason: u8, flags: u8, time_s: f64) {
        NetWorld::assign_net_blocked(&mut self.sim.combat, who, reason, flags, time_s);
    }
    fn server_drop_parry_implementation(&mut self, who: &str, motion_id: u8) {
        NetWorld::server_drop_parry_implementation(&mut self.sim.combat, who, motion_id);
    }
    fn deaths(&self) -> Vec<String> {
        NetWorld::deaths(&self.sim.combat)
    }
    fn net_request_climb(&mut self, who: &str, offset: [f32; 3], slow: bool) -> bool {
        NetWorld::net_request_climb(&mut self.sim.combat, who, offset, slow)
    }
    fn net_climb_motion(&self, who: &str) -> Option<ClimbView> {
        NetWorld::net_climb_motion(&self.sim.combat, who)
    }
    fn net_motion_blocks_climb(&self, who: &str) -> bool {
        NetWorld::net_motion_blocks_climb(&self.sim.combat, who)
    }

    fn net_set_movement(&mut self, who: &str, m: &ExeMovement) {
        let Some(fi) = self.fi(who) else { return };
        let s = &mut self.sim.movers[fi];
        s.location = m.location;
        s.velocity = m.velocity;
        s.mode = m.mode;
        s.set_yaw(m.yaw);
        s.world_time = m.world_time;
        self.sim.yaw[fi] = m.yaw;
        self.sim.net_moved.insert(who.to_string());
    }
    fn net_movement(&self, who: &str) -> Option<ExeMovement> {
        let fi = self.fi(who)?;
        let mut m = self.sim.movers[fi].clone();
        m.set_yaw(self.sim.yaw[fi]);
        Some(m)
    }
    fn net_add_bot(&mut self, who: &str) {
        let (Some(fi), Some(cfg)) = (self.fi(who), self.bots.clone()) else { return };
        let Some(body) = self.sim.bot_fighter.iter().position(|&f| f == fi) else { return };
        let now = self.sim.combat.now;
        let Some(mut bots) = self.sim.bots.take() else { return };
        let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut self.sim.combat, fighter_of: self.sim.bot_fighter.clone() };
        bots.add_bot(body, cfg.profile.clone(), Some(&cfg.tree), now, &mut host);
        self.sim.bots = Some(bots);
    }
}

/// the combat world of a SimWorld (for readers that want mordhau-core's World)
pub fn combat(w: &SimWorld) -> &World {
    &w.sim.combat
}

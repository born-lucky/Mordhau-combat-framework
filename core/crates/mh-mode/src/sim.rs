//! BotSim: bots on a mordhau-core combat World, the reference's BotDuel hookup (godot/game/ai/bot_duel.gd) for
//! closed-loop runs (tests; rust-combat's mh-sim facade wraps `CombatHost` the same way and adds movement and traces).
//! Frame: World::step (the combat tick), every body refreshed from its fighter, then every bot's AI frame at the
//! world time (UE ticks the AI controller / BT component and the character's motion component in tick groups whose
//! relative order is not taken from decompiled code: UNCONFIRMED, as the reference).

use crate::ai::body::{BodyId, BotBody, InventoryItem};
use crate::ai::bt::TreeDef;
use crate::ai::combat_host::{pawn_view, refresh_body, weapon_view, CombatHost};
use crate::ai::controller::Bots;
use crate::ai::profile::Profile;
use crate::kismet::Kismet;
use mordhau_core::combat::World;
use mordhau_core::ue::FVector;
use std::sync::Arc;

pub struct BotSim {
    pub world: World,
    pub bots: Bots,
    /// body index -> fighter index
    pub fighter_of: Vec<usize>,
}

impl BotSim {
    pub fn new(world: World, k: Arc<Kismet>) -> BotSim {
        BotSim { world, bots: Bots { k, ..Default::default() }, fighter_of: Vec::new() }
    }

    /// a body for fighter `fi` at a UE-space location / yaw (BotDuel.add_body); `max_walk_speed`: the character's
    /// CharMoveComp MaxWalkSpeed (BotData.max_walk_speed)
    pub fn add_body(&mut self, fi: usize, location: FVector, yaw: f64, weapon_length: f64, team: i64, max_walk_speed: f64) -> BodyId {
        let mut b = BotBody::new(&self.world.fighters[fi].name);
        b.location = location;
        b.yaw = yaw;
        b.weapon_length = weapon_length;
        b.team = team;
        b.max_walk_speed = max_walk_speed;
        b.pawn = pawn_view(&self.world, fi);
        b.weapon = weapon_view(&self.world, fi);
        b.inventory = vec![InventoryItem { weapon: b.weapon.clone(), is_fists: false, ranged: false }];
        self.fighter_of.push(fi);
        self.bots.add_body(b)
    }

    /// BotDuel.add_bot (MordhauBotController.make: profile copy, tree, BeginPlay; then every bot's perception)
    pub fn add_bot(&mut self, body: BodyId, profile: Profile, tree: Option<&TreeDef>) -> usize {
        let now = self.world.now;
        let mut host = CombatHost { world: &mut self.world, fighter_of: self.fighter_of.clone() };
        self.bots.add_bot(body, profile, tree, now, &mut host)
    }

    pub fn refresh(&mut self) {
        for (b, &fi) in self.fighter_of.iter().enumerate() {
            refresh_body(&self.world, fi, &mut self.bots.bodies[b]);
        }
    }

    /// one frame: the combat step, the bodies refreshed, every bot's AI frame
    pub fn step(&mut self) {
        self.world.step();
        self.refresh();
        let (dt, now) = (self.world.dt, self.world.now);
        let mut host = CombatHost { world: &mut self.world, fighter_of: self.fighter_of.clone() };
        self.bots.tick(dt, now, &mut host);
    }
}

//! The game mode inside the Sim: mh-mode's GameMode (AMordhauGameMode + the mode Blueprints) driven by the Sim's
//! fighters, as the closed-loop host of mh-mode's tests does it.
//!
//! Frame (Sim::step, before the combat step): every pawn's location is reported (Ctrl.location, Godot metres);
//! GameMode::tick(dt); its events are acted on:
//! - spawn_pawn: the pawn is spawned at the chosen PlayerStart's origin (AMordhauGameMode::ChoosePlayerStart
//!   0x15876e0 picks it; RestartPlayer 0x15a6d50). The controller's loadout weapon is used, any old pawn is removed.
//! - destroy_pawn / kill_pawn: the pawn leaves the world (UnPossessAndDestroyPawn; RequestedAssignTeam's kill).
//!
//! After the combat step, the drain events feed scoring:
//! - hit: GameMode::on_damage (victim, instigator, applied damage). AMordhauCharacter::TakeDamage ->
//!   AMordhauGameMode damage history.
//! - died: GameMode::on_killed (killer = the last hit's attacker, weapon = its weapon's class name, kick = the
//!   killing attack's move is EAttackMove Kick). AMordhauGameMode::OnKilled_Implementation rva=0x159d6f0.
//!
//! UNCONFIRMED:
//! - PlayerStart yaw: PlayerStartDef keeps only the origin; the host fills SimMode::start_yaw from the level.
//! - The Godot -> UE axis map is (x, z, y) * 100, inverting ue_static_mesh.gd to_godot.
//! - Mode bots are controllers only. Host AI is attached with Sim::bots as before.
//! - Damage types other than melee are not reported.

use crate::sim::{FighterDesc, Sim};
use mh_mode::event::Ev;
use mh_mode::game_mode::{CtrlId, GameMode};
use mordhau_core::ue::FVector;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Godot metres (Y up) -> UE cm (Z up)
pub fn godot_to_ue(v: [f32; 3]) -> FVector {
    FVector::new(v[0] * 100.0, v[2] * 100.0, v[1] * 100.0)
}
/// UE cm -> Godot metres
pub fn ue_to_godot(v: FVector) -> FVector {
    FVector::new(v.x * 0.01, v.z * 0.01, v.y * 0.01)
}

pub struct SimMode {
    pub gm: GameMode,
    /// controller name -> (right, left) weapon paths of its pawn
    pub loadout: HashMap<String, (String, String)>,
    /// the mode's events of the last frames (drained by `Sim::drain_mode`)
    pub events: Vec<Ev>,
    /// PlayerStart actor name -> its yaw (UE degrees). AGameModeBase::RestartPlayerAtPlayerStart (engine source, not
    /// disassembled) spawns at the start's rotation with pitch / roll zeroed; ModeData's PlayerStartDef keeps only the
    /// origin, so the host fills this from the level (mh_level Spawn.xf). Absent = yaw 0
    pub start_yaw: HashMap<String, f32>,
    /// victim -> the last attacker that hit it
    last_hit: HashMap<String, String>,
    hits0: usize,
}

impl SimMode {
    pub fn new(gm: GameMode) -> SimMode {
        SimMode { gm, loadout: HashMap::new(), start_yaw: HashMap::new(), events: Vec::new(), last_hit: HashMap::new(), hits0: 0 }
    }
    fn ctrl(&self, name: &str) -> Option<CtrlId> {
        self.gm.by_name(name)
    }
}

impl Sim {
    /// Run this game mode inside the Sim (replaces any previous one)
    pub fn set_mode(&mut self, gm: GameMode) {
        let mut m = SimMode::new(gm);
        m.hits0 = self.combat.hits.len();
        self.mode = Some(m);
    }

    /// A controller joins (AGameModeBase::Login + PostLogin; GameMode::login): its pawns carry `weapon` / `left`
    pub fn join(&mut self, name: &str, bot: bool, weapon: &str, left: &str) -> Option<CtrlId> {
        let m = self.mode.as_mut()?;
        m.loadout.insert(name.to_string(), (weapon.to_string(), left.to_string()));
        Some(m.gm.login(name, bot, -1))
    }

    /// The mode's events since the last call (spawn_pawn / possess / kill_feed / score / match_ended ...)
    pub fn drain_mode(&mut self) -> Vec<Ev> {
        self.mode.as_mut().map(|m| std::mem::take(&mut m.events)).unwrap_or_default()
    }

    /// Remove a fighter (its pawn left the world): the combat World's remove_fighter plus every per-fighter vector
    pub fn remove_fighter(&mut self, name: &str) {
        let Some(fi) = self.combat.fighter_index(name) else { return };
        self.release_horse_pawn(fi, true);
        self.combat.remove_fighter(name);
        self.movers.remove(fi);
        self.yaw.remove(fi);
        self.poses.remove(fi);
        if fi < self.fanim.len() {
            self.fanim.remove(fi);
        }
        self.posed.borrow_mut().remove(name);
        if fi < self.climbs.len() {
            self.climbs.remove(fi);
        }
        for b in self.bot_fighter.iter_mut() {
            if *b > fi {
                *b -= 1;
            }
        }
    }

    /// before the combat step: pawn locations in, GameMode::tick, spawn / destroy events out
    pub(crate) fn mode_pre(&mut self) {
        let Some(mut m) = self.mode.take() else { return };
        for fi in 0..self.combat.fighters.len() {
            if let Some(c) = m.ctrl(&self.combat.fighters[fi].name) {
                m.gm.ctrls[c].location = ue_to_godot(self.movers[fi].location);
            }
        }
        m.gm.tick(self.dt as f64);
        for e in m.gm.drain() {
            let who = e.get("who").map(|a| a.as_s().to_string()).unwrap_or_default();
            match e.kind {
                "spawn_pawn" => {
                    let origin = match e.get("xf") {
                        Some(mh_mode::event::Arg::V(v)) => *v,
                        _ => [0.0; 3],
                    };
                    let team = e.get("team").map(|a| a.as_i()).unwrap_or(-1);
                    let start = e.get("start").map(|a| a.as_s().to_string()).unwrap_or_default();
                    let yaw = m.start_yaw.get(&start).copied().unwrap_or(0.0);
                    let (w, l) = m.loadout.get(&who).cloned().unwrap_or_default();
                    self.remove_fighter(&who);
                    m.last_hit.remove(&who);
                    let fi = self.add_fighter(&FighterDesc { name: who.clone(), weapon: w, left: l, team, location: godot_to_ue(origin), yaw });
                    self.combat.fighters[fi].team = team;
                }
                "destroy_pawn" | "kill_pawn" => self.remove_fighter(&who),
                _ => {}
            }
            m.events.push(e);
        }
        self.mode = Some(m);
    }

    /// after the combat step: hits -> on_damage, deaths -> on_killed
    pub(crate) fn mode_post(&mut self) {
        let Some(mut m) = self.mode.take() else { return };
        let evs: Vec<Map<String, Value>> = self.combat.hits[m.hits0..].to_vec();
        m.hits0 = self.combat.hits.len();
        // hits first: the combat emits "died" before the killing "hit" event of the same ApplyDamage
        let mut ordered: Vec<&Map<String, Value>> = evs.iter().filter(|e| e.get("kind").and_then(|v| v.as_str()) == Some("hit")).collect();
        ordered.extend(evs.iter().filter(|e| e.get("kind").and_then(|v| v.as_str()) == Some("died")));
        for e in ordered {
            let s = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            match e.get("kind").and_then(|v| v.as_str()) {
                Some("hit") => {
                    let (a, v) = (s("attacker"), s("victim"));
                    let dmg = e.get("applied").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    if let (Some(ca), Some(cv)) = (m.ctrl(&a), m.ctrl(&v)) {
                        m.gm.on_damage(cv, ca, dmg);
                    }
                    m.last_hit.insert(v, a);
                }
                Some("died") => {
                    let v = s("who");
                    let killer = m.last_hit.get(&v).cloned();
                    let (weapon, kick) = match killer.as_deref().and_then(|k| self.combat.fighter_index(k)) {
                        Some(ki) => {
                            let f = &self.combat.fighters[ki];
                            let w = f.weapon_equip.as_ref().map(|w| w.path.rsplit('/').next().unwrap_or("").to_string()).unwrap_or_default();
                            let kick = f.motion.and_then(|id| self.combat.m(ki, id).attack().map(|a| a.mv == mordhau_core::combat::enums::mv::KICK)).unwrap_or(false);
                            (w, kick)
                        }
                        None => (String::new(), false),
                    };
                    let kc = killer.as_deref().and_then(|k| m.ctrl(k));
                    let vc = m.ctrl(&v);
                    m.gm.on_killed(kc, vc, 0, &weapon, kick);
                    // the dead pawn detaches from its controller (APawn::DetachFromControllerPendingDestroy from the
                    // character's death; UNCONFIRMED: AMordhauCharacter's Die path not disassembled): the controller
                    // has no pawn, so CanAskForSpawn / the bot respawn can restart it; the corpse stays in the world
                    // until the new pawn replaces it
                    if let Some(c) = vc {
                        m.gm.ctrls[c].has_pawn = false;
                    }
                }
                _ => {}
            }
        }
        m.events.extend(m.gm.drain());
        self.mode = Some(m);
    }
}

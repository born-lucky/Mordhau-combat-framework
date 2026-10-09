//! The bots on mordhau-core's combat World (the seam agreed with rust-combat r2: their mh-sim facade wraps
//! `CombatHost` and calls `Bots::tick` after `World::step`, in the frame order of the reference's BotDuel
//! (godot/game/ai/bot_duel.gd): the combat step, every body's view refreshed, then every bot's AI frame).
//!
//!   pawn_view    the MotionSystem fields the tasks read (bot_body.gd / the motion classes), from World state
//!   weapon_view  RightHandEquipment's WeaponData fields the tasks read
//!   CombatHost   BotHost over a World: the fighter input entry points called at once, mid-frame, as the reference's
//!                MordhauBotController calls MotionSystem.request_* (AMordhauCharacter::RequestAttack rva=0x15644c0,
//!                RequestParry rva=0x1564860 (allow_ftp = the reference default true), RequestFeint rva=0x15646b0),
//!                not through the queued Input; navmesh / traces / stimuli stay the defaults (no level geometry)

use super::body::{BodyId, BotBody, BotHost, MotionView, PawnView, WeaponView};
use mordhau_core::combat::World;

/// identity of the current motion object: the slab slot plus its StartTime (a slot is reused by later motions; the
/// reference compares object identity)
fn motion_identity(slot: u32, start: f64) -> i64 {
    ((slot as i64) << 40) ^ (start.to_bits() as i64 & 0xff_ffff_ffff)
}

/// the fighter's state as the bot tasks read it (MotionSystem.motion + stamina / health bytes)
pub fn pawn_view(w: &World, fi: usize) -> PawnView {
    let f = &w.fighters[fi];
    let mut v = MotionView { net_id: f.net.id, ..Default::default() };
    if let (Some(id), Some(m)) = (w.cur(fi), w.cur_m(fi)) {
        v.kind = m.kind().to_string();
        v.id = motion_identity(id.0, m.start_time);
        v.start_time = m.start_time;
        if let Some(a) = m.attack() {
            v.is_attack = true;
            v.stage = a.stage;
            v.release_end = a.release_end;
            v.windup_end = a.windup_end;
            v.has_chambered = a.b_has_chambered;
            v.has_hit = a.b_has_hit;
            v.attack_type = a.ty;
            v.mv = a.mv;
            v.angle_target = a.angle_target;
            // UAttackMotion::GetEarlyReleaseDuration rva=0x1622870: (ReleaseEnd - WindupEnd) * EarlyRelease * TimeFactor
            v.early_release_duration = (a.release_end - a.windup_end) * a.early_release * a.early_release_tf;
        }
        if let Some(p) = m.parry() {
            v.is_parry = true;
            v.riposte_window_start = p.riposte_window_start;
            if let Some(pd) = m.def.parry.as_ref() {
                v.riposte_window_base = pd.riposte_window_base;
                v.riposte_window_extra = pd.non_held_parry_extension_and_riposte_window_extra;
            }
        }
        v.is_feinted = m.feinted().is_some();
    }
    PawnView { motion: v, stamina_byte: w.stamina_byte(fi), health_byte: w.health_byte(fi) }
}

/// RightHandEquipment's combat numbers the tasks read
pub fn weapon_view(w: &World, fi: usize) -> Option<WeaponView> {
    let wd = w.fighters[fi].weapon.as_ref()?;
    Some(WeaponView {
        id: wd.id.clone(),
        chain: Vec::new(),
        native_class: wd.native_class.clone(),
        b_can_attack: wd.b_can_attack,
        stab_damage0: wd.stab.damage.first().copied().unwrap_or(0.0) as f64,
        strike_damage0: wd.strike.damage.first().copied().unwrap_or(0.0) as f64,
    })
}

/// refresh a body from its fighter after the combat step (pawn view, weapon, death, team byte)
pub fn refresh_body(w: &World, fi: usize, b: &mut BotBody) {
    b.pawn = pawn_view(w, fi);
    b.weapon = weapon_view(w, fi);
    b.is_dead = w.fighters[fi].dead;
}

pub struct CombatHost<'a> {
    pub world: &'a mut World,
    /// body index -> fighter index
    pub fighter_of: Vec<usize>,
}

impl BotHost for CombatHost<'_> {
    fn request_attack(&mut self, body: BodyId, mv: i64, angle: f64) -> PawnView {
        let fi = self.fighter_of[body];
        self.world.request_attack(fi, mv, angle);
        pawn_view(self.world, fi)
    }
    fn request_parry(&mut self, body: BodyId, bt: i64) -> PawnView {
        let fi = self.fighter_of[body];
        self.world.request_parry(fi, bt, true);
        pawn_view(self.world, fi)
    }
    fn request_feint(&mut self, body: BodyId) -> PawnView {
        let fi = self.fighter_of[body];
        self.world.request_feint(fi);
        pawn_view(self.world, fi)
    }
}

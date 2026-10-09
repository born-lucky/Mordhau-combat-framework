//! mh-net on mordhau-core's combat World: `NetWorld` for World, `Pawn` for a fighter view (CorePawn), and `NetRules`,
//! the `NetHooks` the combat code calls on a networked machine (mordhau-core combat/net.rs). This is what the
//! reference's NetMotionRep / MotionSystem.net_rep pair does in godot/game/net + game/combat; the rules themselves are
//! rep.rs / pawn.rs. Per-fighter replication state lives in `Fighter::net_state` (a NetMotionRep), the motion-creation
//! data in `Fighter::net_slot`.

use std::rc::Rc;

use mordhau_core::combat::system::DAMAGE_MELEE;
use mordhau_core::combat::{damage, MotionId, MotionKind, NetHooks, NetMotion, World};
use serde_json::json;

use crate::netmotion::FNetMotion;
use crate::pawn::{attack_lag, attack_ping_compensation, with_rep, AttackLag, CompMotion, NetCtx, NetSlot, Pawn};
use crate::rep::NetMotionRep;
use crate::world::{ClimbView, LastAttackView, NetWorld, PawnInfo};

pub fn to_fnm(n: &NetMotion) -> FNetMotion {
    FNetMotion {
        id: (n.id & 0xff) as u8,
        motion_type: (n.motion_type & 0xff) as u8,
        param0: (n.param0 & 0xff) as u8,
        param1: (n.param1 & 0xff) as u8,
        param2: (n.param2 & 0xff) as u8,
        dynamic_param: (n.dynamic_param & 0xff) as u8,
    }
}

pub fn from_fnm(n: &FNetMotion) -> NetMotion {
    NetMotion {
        id: n.id as i64,
        motion_type: n.motion_type as i64,
        param0: n.param0 as i64,
        param1: n.param1 as i64,
        param2: n.param2 as i64,
        dynamic_param: n.dynamic_param as i64,
    }
}

/// one fighter of a combat World, as the rules see it
pub struct CorePawn<'a> {
    pub w: &'a mut World,
    pub fi: usize,
}

impl Pawn for CorePawn<'_> {
    fn name(&self) -> &str {
        &self.w.fighters[self.fi].name
    }
    fn role(&self) -> u8 {
        self.w.fighters[self.fi].role as u8
    }
    fn is_dead(&self) -> bool {
        self.w.fighters[self.fi].dead
    }
    fn now(&self) -> f64 {
        self.w.now
    }
    fn net(&self) -> FNetMotion {
        to_fnm(&self.w.fighters[self.fi].net)
    }
    fn set_net(&mut self, nm: FNetMotion) {
        self.w.fighters[self.fi].net = from_fnm(&nm);
    }
    fn slot(&self) -> &NetSlot {
        &self.w.fighters[self.fi].net_slot
    }
    fn slot_mut(&mut self) -> &mut NetSlot {
        &mut self.w.fighters[self.fi].net_slot
    }
    fn take_rep(&mut self) -> Option<Box<NetMotionRep>> {
        self.w.fighters[self.fi].net_state.take()?.downcast::<NetMotionRep>().ok()
    }
    fn put_rep(&mut self, r: Box<NetMotionRep>) {
        self.w.fighters[self.fi].net_state = Some(r);
    }
    fn rep_ref(&self) -> Option<&NetMotionRep> {
        self.w.fighters[self.fi].net_state.as_ref()?.downcast_ref::<NetMotionRep>()
    }
    fn motion(&self) -> Option<MotionId> {
        self.w.fighters[self.fi].motion
    }
    fn begin_net_motion(&mut self) {
        self.w.begin_net_motion(self.fi);
    }
    fn on_dynamic_param_changed(&mut self, old: u8, new: u8) {
        if let Some(c) = self.w.fighters[self.fi].motion {
            self.w.motion_on_dynamic_param_changed(self.fi, c, old as i64, new as i64);
        }
    }
    fn q(&self) -> fn(f64) -> f64 {
        self.w.qf()
    }
    /// MotionSystem.trace_event: world.trace_event(name + " " + s)
    fn trace_event(&mut self, s: &str) {
        let line = format!("{} {}", self.w.fighters[self.fi].name, s);
        self.w.trace_event(&line);
    }
    fn health(&self) -> i64 {
        self.w.fighters[self.fi].health
    }
    fn health_range(&self) -> (i64, i64) {
        let s = &self.w.fighters[self.fi].health_stat;
        (s.min_value, s.max_value)
    }
    fn set_health_internal(&mut self, v: i64) {
        self.w.set_health_value(self.fi, v);
    }
    fn stamina(&self) -> i64 {
        self.w.fighters[self.fi].stamina
    }
    fn stamina_range(&self) -> (i64, i64) {
        let s = &self.w.fighters[self.fi].stamina_stat;
        (s.min_value, s.max_value)
    }
    fn set_stamina_internal(&mut self, v: i64) {
        self.w.set_stamina_value(self.fi, v);
    }
    /// MotionSystem.offset_stamina (authority only); its replicated write is skipped here because the caller (a rule
    /// holding this fighter's rep) writes the byte itself
    fn offset_stamina_raw(&mut self, v: i64) -> bool {
        let old = self.w.fighters[self.fi].stamina;
        self.w.offset_stamina(self.fi, v);
        self.w.fighters[self.fi].stamina != old
    }
    fn stop_stamina_regen(&mut self, t: f64) {
        self.w.stop_stamina_regen(self.fi, t);
    }
    fn airborne(&self) -> bool {
        self.w.fighters[self.fi].airborne
    }
    fn set_airborne(&mut self, b: bool) {
        self.w.fighters[self.fi].airborne = b;
    }
    fn look_up_value(&self) -> f64 {
        self.w.fighters[self.fi].look_up_value
    }
    fn set_look_up_value(&mut self, v: f64) {
        self.w.fighters[self.fi].look_up_value = v;
    }
    fn look_limits(&self) -> (f64, f64) {
        let c = &self.w.fighters[self.fi].character;
        (c.look_up_limit, c.look_down_limit)
    }
    fn dodge_stamina_cost(&self) -> i64 {
        self.w.fighters[self.fi].character.dodge_stamina_cost
    }
    fn knockback_parry(&self) -> f64 {
        self.w.fighters[self.fi].character.knockback_parry
    }
    /// actor_xf basis x (the reference: (sys.actor_xf as Transform3D).basis.x), None without a transform
    fn forward(&self) -> Option<[f32; 3]> {
        let x = self.w.fighters[self.fi].actor_xf.as_ref()?;
        Some([x.rows[0].x, x.rows[1].x, x.rows[2].x])
    }
    fn set_team(&mut self, team: i64) {
        self.w.fighters[self.fi].team = team;
    }
}

/// the current motion of fighter `fi` as GetAttackCompensationStartTime reads it
pub fn comp_motion(w: &World, fi: usize, id: MotionId, mv: i64) -> Option<CompMotion> {
    let m = w.fighters[fi].motions.get(id.0 as usize)?.as_ref()?;
    let (ca, end) = (m.b_can_attack, m.end_time);
    Some(match &m.k {
        MotionKind::Attack(a) => CompMotion::Attack { stage: a.stage as u8, can_attack: ca, end_time: end },
        MotionKind::Parry(p) => CompMotion::Parry { total_blocks: p.total_blocks as i32, can_attack: ca, end_time: end },
        MotionKind::Idle => CompMotion::Idle { coming_from: m.coming_from.and_then(|c| comp_motion(w, fi, c, mv)).map(Box::new), can_attack: ca, end_time: end },
        MotionKind::Blocked(b) => CompMotion::Blocked { reason: b.reason as u8, from_move: b.from_move as u8, can_attack: ca, end_time: end },
        MotionKind::Feinted(_) => CompMotion::Feinted {
            can_attack_from_feint_lockout: w.attack_motion_defaults(fi, mv).attack.as_ref().map(|a| a.b_can_attack_from_feint_lockout).unwrap_or(false),
            can_attack: ca,
            end_time: end,
        },
        _ => CompMotion::Other { can_attack: ca, end_time: end },
    })
}

/// the NetHooks a networked combat World calls (installed by NetWorld::set_authority)
pub struct NetRules;

impl NetHooks for NetRules {
    fn assign(&self, w: &mut World, fi: usize, nm: NetMotion) {
        let mut p = CorePawn { w, fi };
        if with_rep(&mut p, |r, p| r.assign_net_motion(p, to_fnm(&nm))).is_none() {
            // a fighter without replication state on a networked machine: the offline path
            w_offline_assign(p.w, fi, nm);
        }
    }
    fn assign_dynamic_param(&self, w: &mut World, fi: usize, v: i64) {
        let mut p = CorePawn { w, fi };
        if with_rep(&mut p, |r, p| r.assign_net_motion_dynamic_param(p, (v & 0xff) as u8)).is_none() {
            let f = &mut p.w.fighters[fi];
            let old = f.net.dynamic_param;
            f.net.dynamic_param = v & 0xff;
            let nv = f.net.dynamic_param;
            if old != nv {
                if let Some(c) = p.w.fighters[fi].motion {
                    p.w.motion_on_dynamic_param_changed(fi, c, old, nv);
                }
            }
        }
    }
    fn send_server_drop_parry(&self, w: &mut World, fi: usize, motion_id: i64) {
        if let Some(r) = w.rep_mut_fi(fi) {
            r.send_server_drop_parry((motion_id & 0xff) as u8);
        }
    }
    fn stat_written(&self, w: &mut World, fi: usize, health: bool) {
        let mut p = CorePawn { w, fi };
        with_rep(&mut p, |r, p| if health { r.write_replicated_health(p) } else { r.write_replicated_stamina(p) });
    }
    fn attack_ping_compensation(&self, w: &World, fi: usize, mv: i64) -> f64 {
        let f = &w.fighters[fi];
        let cm = f.motion.and_then(|c| comp_motion(w, fi, c, mv));
        attack_ping_compensation(f.role as u8, &f.net_slot.ctx, w.now, cm.as_ref(), (mv & 0xff) as u8)
    }
    fn attack_lag(&self, w: &mut World, fi: usize, m: MotionId) {
        let f = &w.fighters[fi];
        let (role, ctx) = (f.role as u8, f.net_slot.ctx);
        let local = f.net_slot.is_initiated_locally(Some(m));
        let a = w.att(fi, m);
        let mut lag = AttackLag { lag_reduction: a.lag_reduction, lag_induction: a.lag_induction, miss_recovery: a.ai.miss_recovery };
        attack_lag(role, &ctx, local, (a.mv & 0xff) as u8, a.angle_target, &mut lag);
        let a = w.att_mut(fi, m);
        a.lag_reduction = lag.lag_reduction;
        a.lag_induction = lag.lag_induction;
        a.ai.miss_recovery = lag.miss_recovery;
    }
    fn receive_block(&self, w: &mut World, fi: usize, attacker_move: i64, shield_wall: bool) {
        let mut p = CorePawn { w, fi };
        with_rep(&mut p, |r, p| r.on_receive_block(p, (attacker_move & 0xff) as u8, shield_wall));
    }
}

/// the offline body of AssignNetMotion (MotionSystem._assign without net_rep)
fn w_offline_assign(w: &mut World, fi: usize, nm: NetMotion) {
    w.fighters[fi].net = nm;
    w.begin_net_motion(fi);
}

trait RepAccess {
    fn rep_fi(&self, fi: usize) -> Option<&NetMotionRep>;
    fn rep_mut_fi(&mut self, fi: usize) -> Option<&mut NetMotionRep>;
}

impl RepAccess for World {
    fn rep_fi(&self, fi: usize) -> Option<&NetMotionRep> {
        self.fighters.get(fi)?.net_state.as_ref()?.downcast_ref::<NetMotionRep>()
    }
    fn rep_mut_fi(&mut self, fi: usize) -> Option<&mut NetMotionRep> {
        self.fighters.get_mut(fi)?.net_state.as_mut()?.downcast_mut::<NetMotionRep>()
    }
}

impl NetWorld for World {
    type P<'a> = CorePawn<'a>;

    fn now(&self) -> f64 {
        self.now
    }
    fn tick_n(&self) -> u64 {
        self.tick_n as u64
    }
    fn set_clock(&mut self, tick_n: u64, now: f64) {
        self.tick_n = tick_n as i64;
        self.now = now;
    }
    fn set_authority(&mut self, authority: bool) {
        self.authority = authority;
        self.net = Some(Rc::new(NetRules));
    }
    fn add_fighter(&mut self, name: &str, weapon: &str, left: &str, role: u8, remote_controlled: bool, rep: NetMotionRep, ctx: NetCtx) {
        let fi = World::add_fighter(self, name, weapon, left);
        let f = &mut self.fighters[fi];
        f.role = role as i64;
        f.remote_controlled = remote_controlled;
        f.net_slot.ctx = ctx;
        f.net_state = Some(Box::new(rep));
    }
    fn remove_fighter(&mut self, name: &str) {
        World::remove_fighter(self, name);
    }
    fn net_pawn<'a>(&'a mut self, name: &str) -> Option<CorePawn<'a>> {
        let fi = self.fighter_index(name)?;
        Some(CorePawn { w: self, fi })
    }
    fn pawn_info(&self, name: &str) -> Option<PawnInfo> {
        let f = &self.fighters[self.fighter_index(name)?];
        Some(PawnInfo { role: f.role as u8, dead: f.dead, now: self.now, net: to_fnm(&f.net) })
    }
    fn rep(&self, name: &str) -> Option<&NetMotionRep> {
        self.rep_fi(self.fighter_index(name)?)
    }
    fn rep_mut(&mut self, name: &str) -> Option<&mut NetMotionRep> {
        let fi = self.fighter_index(name)?;
        self.rep_mut_fi(fi)
    }
    fn set_ctx(&mut self, name: &str, ctx: NetCtx) {
        if let Some(fi) = self.fighter_index(name) {
            self.fighters[fi].net_slot.ctx = ctx;
        }
    }
    fn set_team(&mut self, name: &str, team: i64) {
        if let Some(fi) = self.fighter_index(name) {
            self.fighters[fi].team = team;
        }
    }
    fn fighter_names(&self) -> Vec<String> {
        self.fighters.iter().map(|f| f.name.clone()).collect()
    }
    fn step(&mut self, call_phase: &mut dyn FnMut(&mut Self)) {
        self.step_with(&mut |w: &mut World| call_phase(w));
    }
    fn suggest_hit_prepare(&mut self, attacker: &str, victim: &str) -> Option<LastAttackView> {
        let (a, v) = (self.fighter_index(attacker)?, self.fighter_index(victim)?);
        let last = self.fighters[a].last_attack_motion?;
        if self.fighters[a].weapon.is_none() {
            return None;
        }
        let vid = self.fighters[v].id;
        if self.fighters[a].tracer.actor_ignore_cache.contains(&vid) {
            return None;
        }
        self.fighters[a].tracer.actor_ignore_cache.push(vid);
        let at = self.att(a, last);
        Some(LastAttackView {
            is_current: self.fighters[a].motion == Some(last),
            in_recovery: at.stage == 2, // EAttackStage::Recovery
            stop_on_hit: at.ai.b_stop_on_hit,
            victim_will_stop_melee: self.fighters[v].character.b_will_stop_melee,
        })
    }
    fn suggest_hit_damage(&mut self, attacker: &str, victim: &str, bone: &str) {
        let (Some(a), Some(v)) = (self.fighter_index(attacker), self.fighter_index(victim)) else { return };
        let Some(last) = self.fighters[a].last_attack_motion else { return };
        let ai = self.att(a, last).ai.clone();
        let q = self.qf();
        let dmg = damage::compute(&self.spec.constants, &self.fighters[v], &ai, bone, q);
        let applied = self.take_damage(v, dmg, Some(a), DAMAGE_MELEE);
        let health = self.fighters[v].health;
        self.trace_event(&format!("{attacker} suggested hit {victim} {bone} {dmg:.2}"));
        self.emit_event(json!({"kind": "hit", "attacker": attacker, "victim": victim, "bone": bone, "damage": dmg,
            "applied": applied, "health": health, "friendly": false, "suggested": true}));
    }
    fn assign_net_blocked(&mut self, who: &str, reason: u8, flags: u8, time_s: f64) {
        if let Some(fi) = self.fighter_index(who) {
            World::assign_net_blocked(self, fi, reason as i64, flags as i64, time_s);
        }
    }
    fn server_drop_parry_implementation(&mut self, who: &str, motion_id: u8) {
        if let Some(fi) = self.fighter_index(who) {
            World::server_drop_parry_implementation(self, fi, motion_id as i64);
        }
    }
    fn deaths(&self) -> Vec<String> {
        self.hits
            .iter()
            .filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("died"))
            .filter_map(|e| e.get("who").and_then(|w| w.as_str()).map(|s| s.to_string()))
            .collect()
    }

    fn net_request_climb(&mut self, who: &str, offset: [f32; 3], slow: bool) -> bool {
        let Some(fi) = self.fighter_index(who) else { return false };
        World::request_climb(self, fi, mordhau_core::ue::FVector::new(offset[0], offset[1], offset[2]), slow);
        true
    }

    fn net_climb_motion(&self, who: &str) -> Option<ClimbView> {
        let fi = self.fighter_index(who)?;
        let id = self.cur(fi)?;
        let c = self.climb(fi, id)?;
        let start = self.m(fi, id).start_time;
        // the class timings as mh-sim's climb glue passes them (climb_on_begin swaps the slow ones in itself)
        let k = mordhau_core::combat::climb::ClimbingMotion::ctor();
        let data = mh_character::exe_climb::ClimbMotionData {
            end_extra: k.climb_recovery_duration as f32,
            vertical_start: k.authority_move_up_start_time as f32,
            horizontal_start: k.authority_move_lateral_start_time as f32,
            horizontal_duration: k.authority_move_lateral_duration as f32,
            slow_end_extra: k.slow_climb_recovery_duration as f32,
            slow_vertical_start: k.slow_authority_move_up_start_time as f32,
            slow_horizontal_start: k.slow_authority_move_lateral_start_time as f32,
            slow_horizontal_duration: k.slow_authority_move_lateral_duration as f32,
            turn_cap_yaw: k.turncaps.0 as f32,
            turn_cap_pitch: k.turncaps.1 as f32,
        };
        Some(ClimbView { key: (id.0, (start as f32).to_bits()), start_time: start as f32, params: c.params, data })
    }

    fn net_motion_blocks_climb(&self, who: &str) -> bool {
        // as mh-sim climb.rs climb_gates (rust-combat's reading of BP_MordhauCharacter AttemptClimb)
        let Some(fi) = self.fighter_index(who) else { return false };
        self.cur_m(fi).map(|m| m.kind() == "Climbing" || m.is_parry() || m.attack().map(|a| a.stage != 2).unwrap_or(false)).unwrap_or(false)
    }
}

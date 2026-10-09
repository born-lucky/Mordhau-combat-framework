//! TEST FIXTURE, NOT A GAME RULE. A tiny deterministic stand-in for the combat crate's world (mordhau-core is being
//! written in parallel and has no CombatState yet), implementing mh-net's `NetWorld` / `Pawn` seams exactly the way
//! MotionSystem / CombatState do in the GDScript reference (godot/game/combat/motion_system.gd: _assign, _change ->
//! init_motion before OnBegin, begin_net_motion, assign_net_motion_dynamic_param, offset_stamina authority-only,
//! server_drop_parry_implementation). Motion durations and damage are made-up fixture numbers: these tests prove
//! the replication protocol (server + clients end tick-identical to a server-only run), not combat values. When
//! mordhau-core's world lands, the same harness runs against it through the same traits.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use mh_net::enums::{self, combat};
use mh_net::pawn::{with_rep, MotionId, NetCtx, NetSlot, Pawn};
use mh_net::rep::NetMotionRep;
use mh_net::world::{LastAttackView, NetWorld, PawnInfo};
use mh_net::FNetMotion;

// fixture durations (s) - NOT game data
pub const WINDUP: f64 = 0.4;
pub const RELEASE: f64 = 0.2;
pub const RECOVERY: f64 = 0.3;
pub const PARRY: f64 = 0.5;
pub const FEINTED: f64 = 0.3;
pub const BLOCKED: f64 = 0.6;
pub const FLINCH: f64 = 0.4;
pub const DAMAGE: i64 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Idle,
    Attack,
    Parry,
    Feinted,
    Blocked,
    Flinch,
    Stun,
    Disarmed,
}

#[derive(Clone, Debug)]
pub struct Motion {
    pub serial: u64,
    pub kind: Kind,
    pub start: f64,
    pub end: Option<f64>,
    pub expected_delay: f64,
    pub has_hit: bool,
    pub blocks: u8,
    pub reason: u8,
}

impl Motion {
    pub fn windup_end(&self) -> f64 {
        self.start + WINDUP
    }
    pub fn release_end(&self) -> f64 {
        self.start + WINDUP + RELEASE
    }
    /// 0 windup, 1 release, 2 recovery (EAttackStage order)
    pub fn stage(&self, now: f64) -> u8 {
        if now < self.windup_end() - 1e-6 {
            0
        } else if now < self.release_end() - 1e-6 {
            1
        } else {
            2
        }
    }
}

pub type Log = Rc<RefCell<Vec<String>>>;

pub struct ToyPawn {
    pub name: String,
    pub role: u8,
    pub remote_controlled: bool,
    pub dead: bool,
    pub now: f64,
    pub net: FNetMotion,
    pub slot: NetSlot,
    pub rep: Option<Box<NetMotionRep>>,
    pub motion: Motion,
    serial: u64,
    pub health: i64,
    pub stamina: i64,
    pub airborne: bool,
    pub look_up: f64,
    pub team: i64,
    pub ignore: Vec<String>,
    pub last_attack: Option<u64>,
    log: Log,
    pub deaths: Rc<RefCell<Vec<String>>>,
    pub stamina_regen_stops: u32,
}

impl ToyPawn {
    fn change(&mut self, kind: Kind) {
        self.serial += 1;
        let serial = self.serial;
        // MotionSystem._change: init_motion (bInitiatedLocally / bWasConfirmedByAuthority / ExpectedDelay) before OnBegin
        let delay = self.slot.init_motion(MotionId(serial as u32));
        let end = match kind {
            Kind::Idle => None,
            Kind::Attack => Some(self.now + WINDUP + RELEASE + RECOVERY),
            Kind::Parry => Some(self.now + PARRY),
            Kind::Feinted => Some(self.now + FEINTED),
            Kind::Blocked => Some(self.now + BLOCKED),
            Kind::Flinch => Some(self.now + FLINCH - delay),
            Kind::Stun | Kind::Disarmed => Some(self.now + BLOCKED),
        };
        self.motion = Motion { serial, kind, start: self.now, end, expected_delay: delay, has_hit: false, blocks: 0, reason: self.net.param0 };
        if kind == Kind::Attack {
            self.last_attack = Some(serial);
            self.ignore.clear();
        }
        self.log.borrow_mut().push(format!("{:.3} {} {:?}", self.now, self.name, kind));
    }

    /// MotionSystem._assign: NewNetMotion.Id = NetMotion.Id + 1; with net the role-dependent rest runs in mh-net
    pub fn assign(&mut self, mut nm: FNetMotion) {
        nm.id = self.net.id.wrapping_add(1);
        if self.rep.is_some() {
            with_rep(self, |r, p| r.assign_net_motion(p, nm));
            return;
        }
        self.net = nm;
        self.begin_net_motion();
    }

    /// MotionSystem.assign_net_motion_dynamic_param
    pub fn assign_dyn(&mut self, v: u8) {
        if self.rep.is_some() {
            with_rep(self, |r, p| r.assign_net_motion_dynamic_param(p, v));
            return;
        }
        let old = self.net.dynamic_param;
        self.net.dynamic_param = v;
        if old != v {
            self.on_dynamic_param_changed(old, v);
        }
    }

    fn set_health(&mut self, v: i64) {
        let old = self.health;
        self.health = v.clamp(0, 100);
        if old > 0 && self.health < 1 && !self.dead {
            self.dead = true;
            self.log.borrow_mut().push(format!("{:.3} {} died", self.now, self.name));
            self.deaths.borrow_mut().push(self.name.clone());
        }
    }

    /// authority damage: health change with bReplicate (WriteReplicatedStat)
    pub fn take_damage(&mut self, d: i64) {
        if self.role != enums::ROLE_AUTHORITY {
            return;
        }
        let v = self.health - d;
        self.set_health(v);
        if self.rep.is_some() {
            with_rep(self, |r, p| r.write_replicated_health(p));
        }
    }

    /// MotionSystem.offset_stamina (authority only; writes ReplicatedStamina)
    pub fn offset_stamina(&mut self, v: i64) {
        if self.offset_stamina_raw(v) && self.rep.is_some() {
            with_rep(self, |r, p| r.write_replicated_stamina(p));
        }
    }

    fn tick(&mut self) {
        if let Some(e) = self.motion.end {
            if self.now >= e - 1e-6 && !self.dead {
                self.change(Kind::Idle);
            }
        }
    }
}

impl Pawn for ToyPawn {
    fn name(&self) -> &str {
        &self.name
    }
    fn role(&self) -> u8 {
        self.role
    }
    fn is_dead(&self) -> bool {
        self.dead
    }
    fn now(&self) -> f64 {
        self.now
    }
    fn net(&self) -> FNetMotion {
        self.net
    }
    fn set_net(&mut self, nm: FNetMotion) {
        self.net = nm;
    }
    fn slot(&self) -> &NetSlot {
        &self.slot
    }
    fn slot_mut(&mut self) -> &mut NetSlot {
        &mut self.slot
    }
    fn take_rep(&mut self) -> Option<Box<NetMotionRep>> {
        self.rep.take()
    }
    fn put_rep(&mut self, r: Box<NetMotionRep>) {
        self.rep = Some(r);
    }
    fn rep_ref(&self) -> Option<&NetMotionRep> {
        self.rep.as_deref()
    }
    fn motion(&self) -> Option<MotionId> {
        Some(MotionId(self.motion.serial as u32))
    }
    fn begin_net_motion(&mut self) {
        let kind = match self.net.motion_type {
            combat::NET_ATTACK => Kind::Attack,
            combat::NET_PARRY => Kind::Parry,
            combat::NET_FEINTED => Kind::Feinted,
            combat::NET_BLOCKED => Kind::Blocked,
            combat::NET_FLINCHED => Kind::Flinch,
            combat::NET_STUNNED => Kind::Stun,
            combat::NET_DISARMED => Kind::Disarmed,
            _ => return,
        };
        self.change(kind);
        if self.net.dynamic_param != 0 {
            let d = self.net.dynamic_param;
            self.on_dynamic_param_changed(0, d);
        }
    }
    fn on_dynamic_param_changed(&mut self, _old: u8, new: u8) {
        match self.motion.kind {
            Kind::Attack => self.motion.has_hit = new & 1 != 0,
            Kind::Parry => self.motion.blocks = new,
            _ => {}
        }
    }
    fn health(&self) -> i64 {
        self.health
    }
    fn health_range(&self) -> (i64, i64) {
        (0, 100)
    }
    fn set_health_internal(&mut self, v: i64) {
        self.set_health(v);
    }
    fn stamina(&self) -> i64 {
        self.stamina
    }
    fn stamina_range(&self) -> (i64, i64) {
        (0, 100)
    }
    fn set_stamina_internal(&mut self, v: i64) {
        self.stamina = v.clamp(0, 100);
    }
    fn offset_stamina_raw(&mut self, v: i64) -> bool {
        if self.role != enums::ROLE_AUTHORITY {
            return false;
        }
        let old = self.stamina;
        self.stamina = (self.stamina + v).clamp(0, 100);
        self.stamina != old
    }
    fn stop_stamina_regen(&mut self, _t: f64) {
        self.stamina_regen_stops += 1;
    }
    fn airborne(&self) -> bool {
        self.airborne
    }
    fn set_airborne(&mut self, b: bool) {
        self.airborne = b;
    }
    fn look_up_value(&self) -> f64 {
        self.look_up
    }
    fn set_look_up_value(&mut self, v: f64) {
        self.look_up = v;
    }
    fn look_limits(&self) -> (f64, f64) {
        (60.0, 50.0)
    }
    fn dodge_stamina_cost(&self) -> i64 {
        10
    }
    fn knockback_parry(&self) -> f64 {
        100.0
    }
    fn forward(&self) -> Option<[f32; 3]> {
        None
    }
    fn set_team(&mut self, team: i64) {
        self.team = team;
    }
}

#[derive(Clone, Debug)]
pub enum Input {
    Attack(String),
    Parry(String),
    Feint(String),
    ReleaseBlock(String),
    Contact(String, String),
    /// the authority forces a motion (a hit's flinch, ...): MotionSystem._assign
    Force(String, FNetMotion),
}

pub struct ToyWorld {
    pub dt: f64,
    pub tick_n: u64,
    pub now: f64,
    pub authority: bool,
    pub pawns: Vec<ToyPawn>,
    sched: Vec<(u64, Input)>,
    pub log: Log,
    pub deaths: Rc<RefCell<Vec<String>>>,
    /// hits processed on this machine: (t, attacker, victim, suggested)
    pub hits: Vec<(f64, String, String, bool)>,
}

impl ToyWorld {
    pub fn new(dt: f64) -> Self {
        ToyWorld { dt, tick_n: 0, now: 0.0, authority: true, pawns: vec![], sched: vec![], log: Rc::new(RefCell::new(vec![])), deaths: Rc::new(RefCell::new(vec![])), hits: vec![] }
    }

    pub fn tick_of(&self, t: f64) -> u64 {
        (t / self.dt).round() as u64
    }

    pub fn at(&mut self, t: f64, i: Input) {
        let k = self.tick_of(t);
        self.sched.push((k, i));
    }

    pub fn p(&self, n: &str) -> &ToyPawn {
        self.pawns.iter().find(|p| p.name == n).unwrap()
    }

    pub fn pm(&mut self, n: &str) -> &mut ToyPawn {
        self.pawns.iter_mut().find(|p| p.name == n).unwrap()
    }

    pub fn timeline(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    pub fn outcome(&self) -> String {
        self.pawns.iter().map(|p| format!("{} {}/{} dead {}", p.name, p.health, p.stamina, p.dead)).collect::<Vec<_>>().join(", ")
    }

    pub fn run_until(&mut self, t: f64) {
        while self.now < t - self.dt * 0.5 {
            self.step(&mut |_| {});
        }
    }

    fn apply(&mut self, i: Input) {
        let now = self.now;
        let who = match &i {
            Input::Attack(w) | Input::Parry(w) | Input::Feint(w) | Input::ReleaseBlock(w) | Input::Force(w, _) => w.clone(),
            Input::Contact(a, v) => {
                if self.pawn(a).is_some() && self.pawn(v).is_some() {
                    let (a, v) = (a.clone(), v.clone());
                    self.contact(&a, &v);
                }
                return;
            }
        };
        let Some(p) = self.pawn_mut(&who) else { return };
        match i {
            Input::Attack(_) => {
                if !p.dead && p.motion.kind == Kind::Idle {
                    p.assign(FNetMotion::new(combat::NET_ATTACK, combat::MOVE_RIGHT_STRIKE, 0, 0, 0));
                }
            }
            Input::Parry(_) => {
                if !p.dead {
                    p.assign(FNetMotion::new(combat::NET_PARRY, 0, 0, 0, 0));
                }
            }
            Input::Feint(_) => {
                if !p.dead && p.motion.kind == Kind::Attack && p.motion.stage(now) == 0 {
                    p.assign(FNetMotion::new(combat::NET_FEINTED, 0, combat::MOVE_RIGHT_STRIKE, 0, 0));
                }
            }
            Input::ReleaseBlock(_) => {
                if p.motion.kind == Kind::Parry {
                    // the machine with the input stops holding; a client tells the server (ServerDropParry)
                    if p.role != enums::ROLE_AUTHORITY {
                        let id = p.net.id;
                        if let Some(r) = p.rep.as_mut() {
                            r.send_server_drop_parry(id);
                        }
                    }
                    p.change(Kind::Idle);
                }
            }
            Input::Force(_, nm) => p.assign(nm),
            Input::Contact(..) => {}
        }
    }

    /// the authority's hit processing (clients process no hits)
    fn contact(&mut self, a: &str, v: &str) {
        if !self.authority {
            return;
        }
        let now = self.now;
        let (am, ad) = {
            let p = self.p(a);
            (p.motion.clone(), p.dead)
        };
        if ad || am.kind != Kind::Attack || am.stage(now) != 1 || am.has_hit {
            return;
        }
        if self.p(v).dead {
            return;
        }
        if self.p(v).motion.kind == Kind::Parry {
            let blocks = self.p(v).motion.blocks.wrapping_add(1);
            self.pm(v).assign_dyn(blocks);
            {
                let vp = self.pm(v);
                if vp.rep.is_some() {
                    with_rep(vp, |r, p| r.on_receive_block(p, combat::MOVE_RIGHT_STRIKE, false));
                }
            }
            self.pm(a).assign(FNetMotion::new(combat::NET_BLOCKED, combat::BLOCKED_PARRY, 0, 0, 0));
            self.hits.push((now, a.into(), v.into(), false));
            return;
        }
        let d = self.p(a).net.dynamic_param | 1;
        self.pm(a).assign_dyn(d);
        self.pm(v).assign(FNetMotion::new(combat::NET_FLINCHED, 0, 0, 0, 0));
        self.pm(v).take_damage(DAMAGE);
        self.hits.push((now, a.into(), v.into(), false));
    }
}

impl NetWorld for ToyWorld {
    type P<'a> = &'a mut ToyPawn;

    fn now(&self) -> f64 {
        self.now
    }
    fn tick_n(&self) -> u64 {
        self.tick_n
    }
    fn set_clock(&mut self, tick_n: u64, now: f64) {
        self.tick_n = tick_n;
        self.now = now;
    }
    fn set_authority(&mut self, authority: bool) {
        self.authority = authority;
    }
    fn add_fighter(&mut self, name: &str, _weapon: &str, _left: &str, role: u8, remote_controlled: bool, rep: NetMotionRep, ctx: NetCtx) {
        self.add_local(name, role, remote_controlled);
        let p = self.pawns.last_mut().unwrap();
        p.rep = Some(Box::new(rep));
        p.slot.ctx = ctx;
    }
    fn net_pawn<'a>(&'a mut self, name: &str) -> Option<&'a mut ToyPawn> {
        self.pawns.iter_mut().find(|p| p.name == name)
    }
    fn pawn_info(&self, name: &str) -> Option<PawnInfo> {
        self.pawn(name).map(|p| PawnInfo { role: p.role, dead: p.dead, now: p.now, net: p.net })
    }
    fn rep(&self, name: &str) -> Option<&NetMotionRep> {
        self.pawn(name).and_then(|p| p.rep.as_deref())
    }
    fn rep_mut(&mut self, name: &str) -> Option<&mut NetMotionRep> {
        self.pawn_mut(name).and_then(|p| p.rep.as_deref_mut())
    }
    fn set_ctx(&mut self, name: &str, ctx: NetCtx) {
        if let Some(p) = self.pawn_mut(name) {
            p.slot.ctx = ctx;
        }
    }
    fn set_team(&mut self, name: &str, team: i64) {
        if let Some(p) = self.pawn_mut(name) {
            p.team = team;
        }
    }
    fn remove_fighter(&mut self, name: &str) {
        self.pawns.retain(|p| p.name != name);
    }
    fn fighter_names(&self) -> Vec<String> {
        self.pawns.iter().map(|p| p.name.clone()).collect()
    }
    fn step(&mut self, call_phase: &mut dyn FnMut(&mut Self)) {
        self.step_impl(call_phase);
    }
    fn suggest_hit_prepare(&mut self, attacker: &str, victim: &str) -> Option<LastAttackView> {
        self.suggest_hit_prepare_impl(attacker, victim)
    }
    fn suggest_hit_damage(&mut self, attacker: &str, victim: &str, bone: &str) {
        self.suggest_hit_damage_impl(attacker, victim, bone)
    }
    fn assign_net_blocked(&mut self, who: &str, reason: u8, flags: u8, time_s: f64) {
        self.assign_net_blocked_impl(who, reason, flags, time_s)
    }
    fn server_drop_parry_implementation(&mut self, who: &str, motion_id: u8) {
        self.server_drop_parry_impl(who, motion_id)
    }
    fn deaths(&self) -> Vec<String> {
        self.deaths.borrow().clone()
    }
}

impl ToyWorld {
    pub fn pawn(&self, name: &str) -> Option<&ToyPawn> {
        self.pawns.iter().find(|p| p.name == name)
    }
    pub fn pawn_mut(&mut self, name: &str) -> Option<&mut ToyPawn> {
        self.pawns.iter_mut().find(|p| p.name == name)
    }
    /// a fighter without net (the server-only reference run)
    pub fn add_local(&mut self, name: &str, role: u8, remote_controlled: bool) {
        let mut p = ToyPawn {
            name: name.into(),
            role,
            remote_controlled,
            dead: false,
            now: self.now,
            net: FNetMotion::default(),
            slot: NetSlot::default(),
            rep: None,
            motion: Motion { serial: 0, kind: Kind::Idle, start: 0.0, end: None, expected_delay: 0.0, has_hit: false, blocks: 0, reason: 0 },
            serial: 0,
            health: 100,
            stamina: 100,
            airborne: false,
            look_up: 0.0,
            team: 255,
            ignore: vec![],
            last_attack: None,
            log: self.log.clone(),
            deaths: self.deaths.clone(),
            stamina_regen_stops: 0,
        };
        // the spawn's first Idle (never confirmed), not logged as a motion change
        p.serial = 1;
        p.slot.init_motion(MotionId(1));
        p.motion.serial = 1;
        self.pawns.push(p);
    }
    fn step_impl(&mut self, call_phase: &mut dyn FnMut(&mut Self)) {
        self.tick_n += 1;
        self.now = (self.tick_n as f64 * self.dt as f64) as f64;
        let now = self.now;
        for p in self.pawns.iter_mut() {
            p.now = now;
        }
        let k = self.tick_n;
        let due: Vec<Input> = self.sched.iter().filter(|(t, _)| *t == k).map(|(_, i)| i.clone()).collect();
        self.sched.retain(|(t, _)| *t != k);
        for i in due {
            self.apply(i);
        }
        call_phase(self);
        for p in self.pawns.iter_mut() {
            p.tick();
        }
    }
    fn suggest_hit_prepare_impl(&mut self, attacker: &str, victim: &str) -> Option<LastAttackView> {
        let now = self.now;
        let a = self.pm(attacker);
        a.last_attack?;
        if a.ignore.iter().any(|n| n == victim) {
            return None;
        }
        a.ignore.push(victim.into());
        let is_current = a.motion.kind == Kind::Attack && Some(a.motion.serial) == a.last_attack;
        let in_recovery = is_current && a.motion.stage(now) == 2;
        Some(LastAttackView { is_current, in_recovery, stop_on_hit: false, victim_will_stop_melee: false })
    }
    fn suggest_hit_damage_impl(&mut self, attacker: &str, victim: &str, _bone: &str) {
        let now = self.now;
        self.pm(victim).take_damage(DAMAGE / 2);
        self.hits.push((now, attacker.into(), victim.into(), true));
    }
    fn assign_net_blocked_impl(&mut self, who: &str, reason: u8, flags: u8, _time_s: f64) {
        self.pm(who).assign(FNetMotion::new(combat::NET_BLOCKED, reason, flags, 0, 0));
    }
    pub fn server_drop_parry_impl(&mut self, who: &str, motion_id: u8) {
        let p = self.pm(who);
        if p.dead || p.net.id != motion_id || p.motion.kind != Kind::Parry {
            return;
        }
        p.change(Kind::Idle);
    }
}

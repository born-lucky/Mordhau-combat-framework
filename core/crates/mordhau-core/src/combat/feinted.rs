//! UFeintedMotion: the lockout after a feint (godot/game/combat/feinted_motion.gd;
//! extract/native/decomp/UFeintedMotion.cpp). Constants: MotionDefs.Feinted (BP_FeintedMotion).

use super::enums::{self, at};
use super::motion::MotionId;
use super::world::World;
use crate::ue::{clampf, maxf, Vec2};

#[derive(Default)]
pub struct FeintedMotion {
    pub lock_out_time: f64,      // +0xd8 LockOutTime
    pub strike_lockout: f64,     // +0xd4
    pub stab_lockout: f64,       // +0xd0
    pub feint_type: i64,         // +0xdc Type (EFeintType)
    pub from_move: i64,          // +0xdd FromMove
    pub b_has_queued_move: bool, // +0xde
    pub queued_move: i64,        // +0xdf
    pub queued_angle: f64,       // +0xe0
    pub queue_execute_time: f64, // +0xe4
}

/// FMath::GetMappedRangeValueClamped as inlined in UFeintedMotion::OnBegin: alpha = clamp((v - In.X) / (In.Y - In.X),
/// 0, 1); degenerate In range (|In.Y - In.X| <= SMALL_NUMBER) -> 1 if v >= In.Y else 0
fn map_clamped(v: f64, inr: Vec2, outr: Vec2, small: f64, q: fn(f64) -> f64) -> f64 {
    let span = q(inr.yf() - inr.xf());
    let a = if span.abs() > small { clampf(q(q(v - inr.xf()) / span), 0.0, 1.0) } else if v >= inr.yf() { 1.0 } else { 0.0 };
    q(q(q(outr.yf() - outr.xf()) * a) + outr.xf())
}

impl World {
    fn fe_mut(&mut self, fi: usize, id: MotionId) -> &mut FeintedMotion {
        self.mm(fi, id).feinted_mut().unwrap()
    }

    /// from UFeintedMotion::OnBegin_Implementation rva=0x1660240
    pub(crate) fn feinted_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let def = self.m(fi, id).def.clone();
        let fd = def.feinted.as_ref().unwrap();
        let small = self.spec.constants.small_number;
        let net = self.fighters[fi].net;
        let start = self.m(fi, id).start_time;
        let last = self.fighters[fi].last_attack_motion;
        {
            let f = self.fe_mut(fi, id);
            f.from_move = net.param1;
            f.feint_type = net.param0;
            f.lock_out_time = 0.4; // 0x3ecccccd, the value UFeintedMotion::OnBegin_Implementation stores before LastAttackMotion is read
        }
        if let Some(l) = last {
            // how late in the feintable part of the windup the feint came: 0 = at attack start, 1 = at the deadline
            let lm = self.m(fi, l);
            let la = lm.attack().unwrap();
            let deadline = q(q(la.windup_end - la.lag_induction) - lm.def.attack().feint_window);
            let span = q(deadline - lm.start_time);
            let rem = maxf(q(deadline - start), 0.0);
            let mut late = 1.0;
            if 0.0 < span {
                late = clampf(q(q(span - rem) / span), 0.0, 1.0);
            }
            let mut lock = la.ai.feint_lock_out; // AttackInfo+0x20 FeintLockOut
            let combo = la.ty == at::COMBO;
            let cwi = la.ai.combo_windup_increase;
            if combo {
                lock = maxf(lock, cwi);
            }
            let (mut sl, mut stl) = (0.0, 0.0);
            if let Some(w) = self.fighters[fi].weapon.clone() {
                // RightHandEquipment as AMordhauWeapon
                sl = map_clamped(w.strike.windup, fd.strike_and_stab_lockout_in, fd.strike_and_stab_lockout_out, small, q); // Weapon+0x13f0
                stl = map_clamped(w.stab.windup, fd.strike_and_stab_lockout_in, fd.strike_and_stab_lockout_out, small, q); // Weapon+0xf50
                if combo {
                    sl = maxf(cwi, sl);
                    stl = maxf(cwi, stl);
                }
                let curve = &fd.strike_and_stab_late_feint_adjustment_curve;
                if !curve.is_empty() {
                    // CombatData.curve_eval(curve_keys(curve), late): default (constant) extrapolation
                    let keys = &self.spec.curves.get(curve).unwrap_or_else(|| panic!("spec: no curve {curve}")).keys;
                    let adj = self.spec.curve_eval(keys, late, "RCCE_Constant", "RCCE_Constant");
                    sl = q(sl + adj);
                    stl = q(stl + adj);
                }
                stl = q(stl + fd.extra_stab_lockout);
                sl = q(fd.extra_strike_lockout + sl);
                lock = maxf(sl, stl);
            }
            {
                let f = self.fe_mut(fi, id);
                f.lock_out_time = lock;
                f.strike_lockout = sl;
                f.stab_lockout = stl;
            }
            self.attack_on_feinted(fi, l);
        }
        let lock = self.m(fi, id).feinted().unwrap().lock_out_time;
        self.fighters[fi].next_kick_time = q(fd.slow_kick_duration + start); // MotionSystem+0xb0
        self.mm(fi, id).end_time = q(lock + start);
        self.fighters[fi].last_feinted_motion = Some(id);
    }

    /// from UFeintedMotion::OnTick_Implementation rva=0x16683a0
    pub(crate) fn feinted_on_tick(&mut self, fi: usize, id: MotionId) {
        let f = self.m(fi, id).feinted().unwrap();
        if f.b_has_queued_move && f.queue_execute_time <= self.now {
            let (qm, qa) = (f.queued_move, f.queued_angle);
            self.assign_net_attack_motion(fi, at::REGULAR, qm, qa);
        }
    }

    /// from UFeintedMotion::OnEnded_Implementation rva=0x1663810
    pub(crate) fn feinted_on_ended(&mut self, fi: usize, id: MotionId) {
        let f = self.m(fi, id).feinted().unwrap();
        if f.b_has_queued_move && self.is_current(fi, id) {
            let (qm, qa) = (f.queued_move, f.queued_angle);
            self.assign_net_attack_motion(fi, at::REGULAR, qm, qa);
            if !self.is_current(fi, id) {
                return;
            }
        }
        self.base_on_ended(fi, id);
    }

    /// from UFeintedMotion::ProcessAttack_Implementation rva=0x166b590
    pub(crate) fn feinted_process_attack(&mut self, fi: usize, id: MotionId, m: i64, angle: f64) -> bool {
        let q = self.qf();
        let req = self.attack_motion_defaults(fi, m);
        if req.attack().b_can_attack_from_feint_lockout {
            // CDO of the requested class
            self.fe_mut(fi, id).b_has_queued_move = false;
            self.assign_net_attack_motion(fi, at::REGULAR, m, angle);
            if !self.is_current(fi, id) {
                return true;
            }
        }
        let t = self.now;
        let queue_window = self.m(fi, id).def.feinted.as_ref().unwrap().queue_window;
        let end = self.m(fi, id).end_time;
        let f = self.fe_mut(fi, id);
        let mut delay = 0.0;
        if f.strike_lockout != 0.0 && f.stab_lockout != 0.0 {
            if enums::is_strike(m) {
                delay = q(f.strike_lockout - f.lock_out_time);
            } else if enums::is_stab(m) {
                delay = q(f.stab_lockout - f.lock_out_time);
            }
        }
        if q(delay + end) <= q(t + queue_window) {
            f.queue_execute_time = q(delay + end);
            f.queued_angle = angle;
            f.queued_move = m;
            f.b_has_queued_move = true;
        }
        false
    }

    /// from UFeintedMotion::ProcessBlock_Implementation rva=0x166bac0: parry at once, block side from the feinted move
    pub(crate) fn feinted_process_block(&mut self, fi: usize, id: MotionId, b: i64) -> bool {
        let m = self.m(fi, id);
        let from = m.feinted().unwrap().from_move;
        let mut b2 = b;
        if enums::is_left(from) {
            b2 = if b == 0 { 1 } else { b };
        } else if b == 1 {
            b2 = 0;
        }
        if !m.b_can_block {
            return false;
        }
        self.assign_net_parry(fi, b2);
        true
    }
}

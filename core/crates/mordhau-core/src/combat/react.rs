//! Hit reactions: UFlinchMotion, UStunMotion, UDisarmedMotion (godot/game/combat/flinch_motion.gd, stun_motion.gd,
//! disarmed_motion.gd; extract/native/decomp/UFlinchMotion.cpp, UStunMotion.cpp, UDisarmedMotion.cpp).

use super::motion::{MotionId, MotionKind};
use super::world::World;
use crate::ue::clampf;
use serde_json::json;

pub struct FlinchMotion {
    pub speed_factor: f64,            // +0x64 SpeedFactor
    pub b_disables_atmospherics: bool, // UMordhauMotion +0x6f (ctor UFlinchMotion::UFlinchMotion rva=0x1646930 sets it)
    pub b_disables_offhand_ik: bool,  // UMordhauMotion +0x72 (ctor sets it)
    pub offhand_ik_change_speed: f64, // UMordhauMotion +0x78 (ctor rva=0x1646930 store 10.0)
    pub b_has_played_foley: bool,     // +0xa1
}

impl FlinchMotion {
    pub fn new() -> FlinchMotion {
        FlinchMotion { speed_factor: 1.0, b_disables_atmospherics: true, b_disables_offhand_ik: true, offhand_ik_change_speed: 10.0, b_has_played_foley: false }
    }
}

#[derive(Default)]
pub struct StunMotion {
    pub b_will_disarm: bool, // +0xa0
    pub direction: f64,      // Param0 unpacked: byte / 255 x 360 - 180 (degrees)
}

#[derive(Default)]
pub struct DisarmedMotion {
    pub direction: f64,
}

impl World {
    /// from UFlinchMotion::OnBegin_Implementation rva=0x16606a0 (timing part)
    pub(crate) fn flinch_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let c = self.spec.constants.clone();
        let dyn_ = self.fighters[fi].net.dynamic_param;
        let m = self.mm(fi, id);
        let (sf, fd, plo) = (m.def.base.speed_factor, m.def.react.flinch_duration, m.def.react.parry_lock_out_time);
        let speed = q(clampf(q(dyn_ as f64 * c.byte_to_unit), 0.0, 1.0) * sf);
        if let MotionKind::Flinch(f) = &mut m.k {
            f.speed_factor = speed;
        }
        m.speed_factor = speed;
        m.end_time = q(q(fd + m.start_time) - m.expected_delay);
        // OnBegin (decomp UFlinchMotion.cpp 197-200): bCanBlock = true, then false while ParryLockOutTime (+0xa8) > 0
        m.b_can_block = !(0.0 < plo);
        let (end, start) = (m.end_time, m.start_time);
        let f = &mut self.fighters[fi];
        f.next_attack_time = end; // MotionSystem +0xb4
        f.next_kick_time = q(end + c.flinch_kick_delay); // +0xb0, + 0.25
        f.easy_parry_until_time = q(start + c.easy_parry_window); // character +0xe94, + 0.5
    }

    /// from UFlinchMotion::OnTick_Implementation rva=0x1668430 (disasm 0x141668456..0x1416684a4)
    pub(crate) fn flinch_on_tick(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let t = self.now;
        let cosm = self.spec.constants.flinch_recover_cosmetics;
        let m = self.mm(fi, id);
        if q(m.start_time + m.def.react.parry_lock_out_time) <= t {
            m.b_can_block = true;
        }
        let mut foley = false;
        if q(m.start_time + cosm) < t {
            if let MotionKind::Flinch(f) = &mut m.k {
                f.b_disables_atmospherics = false;
                f.b_disables_offhand_ik = false;
                f.offhand_ik_change_speed = 2.5; // immediate 0x40200000
                if !f.b_has_played_foley {
                    foley = true;
                    f.b_has_played_foley = true;
                }
            }
        }
        if foley {
            let name = self.fighters[fi].name.clone();
            self.emit_event(json!({"kind": "flinch_foley", "who": name}));
        }
    }

    /// from UStunMotion::OnBegin_Implementation rva=0x16628e0 (gameplay part)
    pub(crate) fn stun_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let c = self.spec.constants.clone();
        let net = self.fighters[fi].net;
        let m = self.mm(fi, id);
        m.end_time = q(q(m.def.react.stun_duration + m.start_time) - m.expected_delay);
        let disarm = net.param2 != 0;
        let from = m.coming_from;
        if let MotionKind::Stun(s) = &mut m.k {
            s.b_will_disarm = disarm;
            s.direction = q(q(clampf(q(net.param0 as f64 * c.byte_to_unit), 0.0, 1.0) * 360.0) - 180.0);
        }
        if disarm {
            self.drop_for_disarm(fi, from);
        }
        // OnBegin (decomp UStunMotion.cpp, vcall 0x9e8 SetTurnCaps(180, 90)); reverted by OnLeave rva=0x1666640
        // (src/Mordhau/Private/Motions/StunMotion.cpp, byte-matched: SetTurnCaps(-1, -1))
        self.set_turn_caps(fi, 180.0, 90.0);
    }

    /// UStunMotion::OnLeave_Implementation rva=0x1666640 (byte-matched src StunMotion.cpp)
    pub(crate) fn stun_on_leave(&mut self, fi: usize) {
        self.set_turn_caps(fi, -1.0, -1.0);
    }

    /// from UStunMotion::OnTick_Implementation rva=0x166a1e0 (disasm 0x14166a227..0x14166a237): NextAvailableStunTime
    /// (+0xb8) = now + StunGracePeriodExtraTime (+0xa4)
    pub(crate) fn stun_on_tick(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let g = self.m(fi, id).def.react.stun_grace_period_extra_time;
        self.fighters[fi].next_available_stun_time = q(self.now + g);
    }

    /// from UDisarmedMotion::OnBegin_Implementation rva=0x165ee70 (disasm 0x14165ef81..0x14165efa2)
    pub(crate) fn disarmed_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let c = self.spec.constants.clone();
        let net = self.fighters[fi].net;
        let m = self.mm(fi, id);
        m.end_time = q(q(m.def.react.recovery_time + m.start_time) - m.expected_delay);
        let end = m.end_time;
        let from = m.coming_from;
        if let MotionKind::Disarmed(d) = &mut m.k {
            d.direction = q(q(clampf(q(net.param0 as f64 * c.byte_to_unit), 0.0, 1.0) * 360.0) - 180.0);
        }
        self.fighters[fi].next_kick_time = q(end + c.disarmed_kick_delay);
        self.drop_for_disarm(fi, from);
    }
}

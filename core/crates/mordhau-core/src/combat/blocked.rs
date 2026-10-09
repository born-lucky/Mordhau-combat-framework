//! UBlockedMotion: the attacker after its attack was parried, chambered, clashed, stopped by the world or by a hit
//! (godot/game/combat/blocked_motion.gd; extract/native/decomp/UBlockedMotion.cpp, all 8 functions; fields
//! types/UBlockedMotion.h, FBlockResult.h). NetMotion (FNetMotion::Blocked rva=0x1615f50): Param0 = Reason, Param1 =
//! the FBlockResult bools as bits (bIsStun 1, bIsDisarm 2, bIsRanged 4, bIsCancel 8, bPartyFlag 16,
//! bRequiresSelfBlockEvent 32, bClashOnParry 64), Param2 = time x 40 as a byte. Animation requests are recorded as
//! fields for the animation layer (third person).

use super::enums::{self, at, br, mr, mv};
use super::motion::MotionId;
use super::world::World;
use crate::ue::{clampf, normalized_time, range_fraction};
use serde_json::json;

pub struct BlockedMotion {
    pub reason: i64,                      // FBlockResult.Reason (+0xa9)
    pub b_is_stun: bool,                  // +0xaa
    pub b_is_disarm: bool,                // +0xab
    pub b_is_ranged: bool,                // +0xac
    pub b_is_cancel: bool,                // +0xad
    pub b_party_flag: bool,               // +0xae
    pub b_requires_self_block_event: bool, // +0xaf
    pub b_clash_on_parry: bool,           // +0xb0
    pub surface: i64,                     // +0xb1
    pub from_move: i64,                   // FromMove (+0xa0)
    pub from_attack: Option<MotionId>,    // LastAttackMotion at begin
    pub b_has_queued_move: bool,          // +0xa1
    pub queued_move: i64,                 // +0xa8
    pub queued_angle: f64,                // +0xa4
    pub original_movement_restriction: i64, // +0xca
    pub b_has_faded_out_procedural: bool, // +0xb2
    pub b_do_release_bounce_procedural: bool, // +0xb3
    pub stop_anim_fade: f64,              // last StopAnim blend asked (-1 = none)
    pub kick_hit_stop_anim_rate: f64,     // SetAnimRate(FromAttackMontage, KickHitStopAnimRate) (-1 = none)
    pub bounce_montage: String,
    pub bounce_additive: String,
    pub bounce_anim_position: f64,        // SetAnimPosition of this tick (-1 = none)
}

impl Default for BlockedMotion {
    fn default() -> Self {
        BlockedMotion {
            reason: 0,
            b_is_stun: false,
            b_is_disarm: false,
            b_is_ranged: false,
            b_is_cancel: false,
            b_party_flag: false,
            b_requires_self_block_event: false,
            b_clash_on_parry: false,
            surface: 0,
            from_move: 0,
            from_attack: None,
            b_has_queued_move: false,
            queued_move: 0,
            queued_angle: 0.0,
            original_movement_restriction: 0,
            b_has_faded_out_procedural: false,
            b_do_release_bounce_procedural: false,
            stop_anim_fade: -1.0,
            kick_hit_stop_anim_rate: -1.0,
            bounce_montage: String::new(),
            bounce_additive: String::new(),
            bounce_anim_position: -1.0,
        }
    }
}

impl World {
    fn bl(&self, fi: usize, id: MotionId) -> &BlockedMotion {
        self.m(fi, id).blocked().unwrap()
    }
    fn bl_mut(&mut self, fi: usize, id: MotionId) -> &mut BlockedMotion {
        self.mm(fi, id).blocked_mut().unwrap()
    }

    /// from UBlockedMotion::OnBegin_Implementation rva=0x165dc20 (decomp UBlockedMotion.cpp 8-447)
    pub(crate) fn blocked_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let Some(atk) = self.fighters[fi].last_attack_motion else { return }; // LastAttackMotion null -> return
        let def = self.m(fi, id).def.clone();
        let bd = def.blocked.as_ref().unwrap();
        let c = self.spec.constants.clone();
        let net = self.fighters[fi].net;
        let flags = net.param1; // byte 7: bit n -> FBlockResult byte n + 1 (decomp 83-98)
        let t = q(net.param2 as f64 * c.blocked_time_unit_s); // byte x 0.025
        let am = self.m(fi, atk);
        let a = am.attack().unwrap();
        let from_move = a.mv;
        let (a_early, a_lrn, a_miss) = (a.early_release, a.last_release_norm, a.ai.miss_stamina_cost);
        let aad = am.def.clone();
        let reason = net.param0;
        {
            let b = self.bl_mut(fi, id);
            b.from_attack = Some(atk);
            b.reason = reason;
            b.b_is_stun = flags & 1 != 0;
            b.b_is_disarm = flags & 2 != 0;
            b.b_is_ranged = flags & 4 != 0;
            b.b_is_cancel = flags & 8 != 0;
            b.b_party_flag = flags & 16 != 0;
            b.b_requires_self_block_event = flags & 32 != 0;
            b.b_clash_on_parry = flags & 64 != 0;
            b.surface = 0;
            b.from_move = from_move;
        }
        let lim = bd.parried_recovery_time_limits;
        let mut rec = clampf(q(t + bd.parried_recovery_time_offset), lim.xf(), lim.yf());
        match reason {
            br::WORLD => {
                self.mm(fi, id).b_can_block = false;
                // a world hit at or after EarlyRelease costs (int)-(MissStaminaCost +0xedc x WorldMissStaminaFactor)
                if a_early <= a_lrn {
                    self.offset_stamina(fi, crate::ue::cvttss2si(-q(a_miss * bd.world_miss_stamina_factor)));
                }
                rec = bd.world_recovery_time;
                self.mm(fi, id).movement_restriction = bd.movement_restriction_world;
            }
            br::CHAMBER => {
                let cl = bd.chambered_recovery_time_limits;
                rec = clampf(q(t + bd.chambered_recovery_time_offset), cl.xf(), cl.yf());
            }
            br::CLASH => rec = aad.attack().clashed_recovery, // +0xabc
            br::HIT => rec = aad.attack().hit_stop_recovery,  // +0xab8
            _ => {}
        }
        // the attack's weapon is told it was blocked unless bRequiresSelfBlockEvent, for Parry / Chamber / Hit
        // (AMordhauWeapon::OnWasBlocked_Implementation rva=0x16340a0: sparks and sounds only)
        if !self.bl(fi, id).b_requires_self_block_event && (reason == br::PARRY || reason == br::CHAMBER || reason == br::HIT) {
            let name = self.fighters[fi].name.clone();
            let mut ev = json!({"kind": "was_blocked", "who": name, "reason": reason, "move": from_move});
            if self.exe() {
                // exe mode only (goldens): FBlockResult.bIsCancel (+4): OnWasBlocked_Implementation rva=0x16340a0 plays
                // HitCancelSound instead of WasBlockedSound (decomp AMordhauWeapon.cpp, the `Result->bIsCancel` branch)
                ev["cancel"] = json!(self.bl(fi, id).b_is_cancel);
            }
            self.emit_event(ev);
        }
        if self.exe() && !self.authority && self.bl(fi, id).b_requires_self_block_event {
            // OnWasBlocked_Implementation rva=0x16340a0 with bRequiresSelfBlockEvent (+6): a world trace, then
            // OnHit(null actor, Move, no bone, the point, tier 0, Result->Surface) (the environment hit sound).
            // On the authority UAttackMotion::HandleBlockingHit rva=0x162afa0 already raised it through AssignNetBlock
            // -> OnRep_NetBlock with the surface and point (worldhit.rs); here only a client's copy of the motion.
            let name = self.fighters[fi].name.clone();
            let surf = self.bl(fi, id).surface;
            self.emit_event(json!({"kind": "world_hit", "who": name, "move": from_move, "surface": surf}));
        }
        if reason == br::HIT {
            self.mm(fi, id).movement_restriction = bd.movement_restriction_hit;
        }
        if from_move == mv::KICK {
            let m = self.mm(fi, id);
            let cur = m.movement_restriction;
            m.movement_restriction = mr::NO_MOVEMENT; // immediate 3 (OnBegin disasm 0x14165dfb6)
            m.blocked_mut().unwrap().original_movement_restriction = cur;
        }
        let m = self.mm(fi, id);
        m.end_time = q(q(rec + m.start_time) - m.expected_delay);
        let end = m.end_time;
        // NextKickTime (disasm 0x14165dfbf..0x14165dfe9): World / Hit leave it; Chamber / Clash EndTime + 0.3; Parry + 0.2
        if reason == br::CHAMBER || reason == br::CLASH {
            self.fighters[fi].next_kick_time = q(end + c.blocked_kick_delay_chamber);
        } else if reason == br::PARRY {
            self.fighters[fi].next_kick_time = q(end + c.blocked_kick_delay_parry);
        }
        self.blocked_begin_animation(fi, id, atk);
        // a move the blocked attack had queued is replayed into this motion's ProcessAttack when locally controlled
        // and the block was a Clash of a non-kick or a Hit of a kick (decomp 423-445)
        let a = self.att(fi, atk);
        if a.b_has_queued_move && self.is_locally_controlled(fi) {
            let replay = (reason == br::CLASH && from_move != mv::KICK) || (reason == br::HIT && from_move == mv::KICK);
            if replay {
                let (qm, qa) = (a.queued_move, a.queued_angle);
                self.blocked_process_attack(fi, id, qm, qa);
            }
        }
    }

    /// The animation half of OnBegin (decomp 196-422), third person
    fn blocked_begin_animation(&mut self, fi: usize, id: MotionId, atk: MotionId) {
        let q = self.qf();
        let def = self.m(fi, id).def.clone();
        let bd = def.blocked.as_ref().unwrap();
        let c = self.spec.constants.clone();
        let has_montage = self.attack_has_montage(fi, atk);
        let am = self.m(fi, atk);
        let a = am.attack().unwrap();
        let a_bounce = a.bounce_montage.clone();
        let a_additive = am.def.attack().bounce_additive.clone();
        let (start, end) = (self.m(fi, id).start_time, self.m(fi, id).end_time);
        let b = self.bl_mut(fi, id);
        let (reason, from_move) = (b.reason, b.from_move);
        if a_bounce.is_empty() {
            if reason == br::CLASH || enums::is_stab(from_move) {
                match reason {
                    br::HIT => b.stop_anim_fade = bd.stab_hit_stop_fade_out_time,
                    br::WORLD => b.stop_anim_fade = bd.stab_world_fade_out_time,
                    br::CLASH => b.stop_anim_fade = bd.clash_fade_out_time,
                    _ => {
                        let (mut rng, mut fade) = (bd.stab_parry_min_max_range, bd.stab_parry_fade_out_time);
                        if reason != br::PARRY {
                            rng = bd.stab_chambered_min_max_range;
                            fade = bd.stab_chambered_fade_out_time;
                        }
                        let a = range_fraction(rng.xf(), rng.yf(), q(end - start), c.small_number);
                        b.stop_anim_fade = q(q(q(fade.yf() - fade.xf()) * a) + fade.xf());
                    }
                }
            } else {
                b.b_do_release_bounce_procedural = true;
            }
        } else if reason == br::HIT {
            if has_montage {
                b.kick_hit_stop_anim_rate = bd.kick_hit_stop_anim_rate;
            }
            if bd.kick_hit_stop_blend_out_time != c.blocked_no_blend_out {
                b.stop_anim_fade = bd.kick_hit_stop_blend_out_time;
            }
        } else {
            b.bounce_montage = a_bounce;
        }
        let mut bounce = !a_additive.is_empty();
        if enums::is_strike(from_move) {
            bounce = bounce && reason != br::HIT;
        }
        if bounce && from_move != mv::KICK {
            b.bounce_additive = a_additive;
        }
    }

    /// from UBlockedMotion::OnTick_Implementation rva=0x16671b0 (decomp 454-728): reads LastAttackMotion each tick;
    /// returns at once without one or without its montage
    pub(crate) fn blocked_on_tick(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let Some(atk) = self.fighters[fi].last_attack_motion else { return };
        let Some(from) = self.bl(fi, id).from_attack else { return };
        if !self.attack_has_montage(fi, from) {
            return;
        }
        let def = self.m(fi, id).def.clone();
        let bd = def.blocked.as_ref().unwrap();
        let c = self.spec.constants.clone();
        let t = self.now;
        let (start, end) = (self.m(fi, id).start_time, self.m(fi, id).end_time);
        if self.bl(fi, id).from_move == mv::KICK && q(start + c.blocked_bounce_window) < t {
            let o = self.bl(fi, id).original_movement_restriction;
            self.mm(fi, id).movement_restriction = o;
        }
        let b = self.bl(fi, id);
        if !b.b_do_release_bounce_procedural {
            return;
        }
        let reason = b.reason;
        let (until_fade, fade_out, duration);
        match reason {
            br::HIT => {
                until_fade = bd.hit_stop_time_until_fade;
                fade_out = bd.hit_stop_fade_out_time;
                duration = bd.hit_stop_bounce_duration;
            }
            br::WORLD => {
                until_fade = bd.world_time_until_fade;
                fade_out = bd.world_fade_out_time;
                duration = bd.world_bounce_duration;
            }
            _ => {
                let (mut rng, mut tuf, mut fo, mut bdur) = (bd.parry_min_max_range, bd.parry_time_until_fade, bd.parry_fade_out_time, bd.parry_bounce_duration);
                if reason != br::PARRY {
                    rng = bd.chamber_min_max_range;
                    tuf = bd.chamber_time_until_fade;
                    fo = bd.chamber_fade_out_time;
                    bdur = bd.chamber_bounce_duration;
                }
                let a = range_fraction(rng.xf(), rng.yf(), q(end - start), c.small_number);
                until_fade = q(q(q(tuf.yf() - tuf.xf()) * a) + tuf.xf());
                fade_out = q(q(q(fo.yf() - fo.xf()) * a) + fo.xf());
                duration = q(q(q(bdur.yf() - bdur.xf()) * a) + bdur.xf());
            }
        }
        if q(until_fade + start) < t && !b.b_has_faded_out_procedural {
            let bm = self.bl_mut(fi, id);
            bm.b_has_faded_out_procedural = true;
            bm.stop_anim_fade = fade_out;
        }
        if q(start + c.blocked_bounce_window) <= t {
            return;
        }
        let nt = q(normalized_time(start, q(start + duration), t));
        let am = self.m(fi, atk);
        let a = am.attack().unwrap();
        let lrn = a.last_release_norm;
        let length = self.fighters[fi].tracer.length; // the attack's Weapon +0x1bec Length
        let ad = am.def.attack();
        let (curve, scale);
        if reason == br::HIT {
            curve = bd.hit_stop_bounce_curve.clone();
            scale = bd.hit_stop_bounce_scale_curve.clone();
        } else {
            if !enums::is_strike(a.mv) {
                return;
            }
            if reason == br::WORLD {
                curve = ad.world_bounce_curve.clone();
                scale = ad.world_bounce_scale_curve.clone();
            } else {
                curve = if lrn <= c.blocked_late_bounce_release { ad.parry_bounce_curve.clone() } else { ad.parry_late_bounce_curve.clone() };
                scale = ad.parry_bounce_scale_curve.clone();
            }
        }
        if curve.is_empty() || scale.is_empty() {
            return;
        }
        let mut release_scale = c.blocked_unit_scale;
        let rs = if reason == br::HIT { &bd.hit_stop_release_scale_curve } else { &bd.release_scale_curve };
        if !rs.is_empty() {
            release_scale = self.spec.curve_value(rs, length);
        }
        let pos = q(q(q(q(self.spec.curve_value(&curve, nt) * release_scale) * self.spec.curve_value(&scale, lrn)) + q(lrn * c.blocked_bounce_half)) + c.blocked_bounce_half);
        self.bl_mut(fi, id).bounce_anim_position = clampf(pos, 0.0, 1.0);
    }

    /// from UBlockedMotion::OnLeave_Implementation rva=0x1665730: a procedural bounce not faded out yet stops the
    /// montage (blend 0.5, blocked_leave_fade .rdata 0x143fe4e04)
    pub(crate) fn blocked_on_leave(&mut self, fi: usize, id: MotionId) {
        let fade = self.spec.constants.blocked_leave_fade;
        let b = self.bl_mut(fi, id);
        if b.b_do_release_bounce_procedural && !b.b_has_faded_out_procedural {
            b.b_has_faded_out_procedural = true;
            b.stop_anim_fade = fade;
        }
    }

    /// from UBlockedMotion::ProcessAttack_Implementation rva=0x166b320
    pub(crate) fn blocked_process_attack(&mut self, fi: usize, id: MotionId, m: i64, angle: f64) -> bool {
        let Some(last) = self.fighters[fi].last_attack_motion else { return false };
        let b = self.bl(fi, id);
        let from_kick = b.from_move == mv::KICK;
        let mut clash_from_kick = false;
        if b.reason == br::CLASH && from_kick {
            clash_from_kick = true;
            if m == mv::KICK {
                return false;
            }
        } else {
            if b.reason == br::CHAMBER {
                return false;
            }
            if b.reason == br::HIT && from_kick && m != mv::KICK {
                self.assign_net_attack_motion(fi, at::COMBO, m, angle);
                return !self.is_current(fi, id);
            }
            if b.reason != br::CLASH {
                return self.blocked_queue(fi, id, m, angle, clash_from_kick);
            }
        }
        if !from_kick {
            let (ok, cm) = self.att(fi, last).convert_to_combo(m);
            if !ok || !self.can_perform_attack(fi, cm) {
                return false;
            }
            let ty = if self.bl(fi, id).b_clash_on_parry { at::POST_CLASH_SLOW } else { at::POST_CLASH };
            self.assign_net_attack_motion(fi, ty, cm, angle);
            return !self.is_current(fi, id);
        }
        self.blocked_queue(fi, id, m, angle, clash_from_kick)
    }

    fn blocked_queue(&mut self, fi: usize, id: MotionId, m: i64, angle: f64, clash_from_kick: bool) -> bool {
        let q = self.qf();
        let def = self.m(fi, id).def.clone();
        let bd = def.blocked.as_ref().unwrap();
        let end = self.m(fi, id).end_time;
        let now = self.now;
        let b = self.bl_mut(fi, id);
        let hit_window = clash_from_kick || b.reason == br::HIT;
        let w = if hit_window { bd.queue_window_hit } else { bd.queue_window };
        if q(end - w) <= now {
            b.queued_angle = angle;
            b.b_has_queued_move = true;
            b.queued_move = m;
        }
        false
    }

    /// from UBlockedMotion::OnEnded_Implementation rva=0x1663650
    pub(crate) fn blocked_on_ended(&mut self, fi: usize, id: MotionId) {
        let b = self.bl(fi, id);
        if b.b_has_queued_move {
            let (qm, qa) = (b.queued_move, b.queued_angle);
            self.assign_net_attack_motion(fi, at::REGULAR, qm, qa);
            if !self.is_current(fi, id) {
                return;
            }
        }
        self.base_on_ended(fi, id);
    }

    /// from UBlockedMotion::GetAttackCompensationStartTime rva=0x165c460
    pub(crate) fn blocked_attack_compensation_start_time(&self, fi: usize, id: MotionId, m: i64) -> f64 {
        let mo = self.m(fi, id);
        let b = mo.blocked().unwrap();
        let from_kick = b.from_move == mv::KICK;
        let follow_up = (b.reason == br::CLASH && !from_kick) || (b.reason == br::HIT && from_kick && m != mv::KICK);
        if follow_up && mo.b_can_attack {
            return 0.0;
        }
        mo.end_time
    }
}

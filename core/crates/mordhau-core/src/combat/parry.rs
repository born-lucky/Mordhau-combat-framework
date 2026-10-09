//! UParryMotion (godot/game/combat/parry_motion.gd; extract/native/decomp/UParryMotion.cpp, fields
//! types/UParryMotion.h). The dynamic param byte: low nibble = parry recovery request (1 Fail, 2 Miss), high nibble =
//! TotalBlocks. Not ported: animation/additives, forward parry geometry and parry angles (the contact stands for
//! them), the fists ragdoll + impulse of a fists stamina break.

use super::enums::{self, bt, mv, pr, ps};
use super::melee_hit::segment_box;
use super::motion::MotionId;
use super::world::World;
use crate::ue::cvtss2si;
use crate::ue::trunc_i;
use serde_json::json;

#[derive(Default)]
pub struct ParryMotion {
    pub block_type: i64,                       // +0x532
    pub b_is_shield_wall: bool,                // +0x46a
    pub b_is_block_holdable: bool,             // +0x539
    pub b_is_miss_parry: bool,                 // +0x46b
    pub b_detected_any_non_friendly_attack: bool, // +0x469
    pub b_requested_drop: bool,                // +0x533
    pub stage: i64,                            // +0x53a EParryStage
    pub total_blocks: i64,                     // +0x538
    pub parry_end: f64,                        // +0x53c
    pub recovery_start_time: f64,              // +0x540
    pub riposte_window_start: f64,             // +0x544
    pub b_has_queued_move: bool,               // +0x558
    pub queued_move_time: f64,                 // +0x55c
    pub queued_angle: f64,                     // +0x560
    pub queued_move: i64,                      // +0x564
    pub held_stamina_drain: f64,               // +0x568
    pub cumulative_stamina_drain: f64,         // +0x56c
    pub parry_up_time: f64,                    // +0x514 (instance copy)
    pub parry_recovery_time: f64,              // +0x500
    pub minimum_held_parry_time: f64,          // +0x4f8
    pub non_held_parry_extension_time: f64,    // +0x4b0
    pub backpedal_speed_factor: f64,           // UMordhauMotion BackpedalSpeedFactor (from WeaponPtr)
    pub recovery_type: i64,                    // EParryRecoveryType passed to EnterParryRecovery (-1 = none)
    pub last_blocked_move: i64,                // the FNetBlock BlockedMove of CheckAttackParry rva=0x1616cd0 (-1 = none)
    /// +BlockedAttacks TMap<attack, time> (CheckParry rva=0x164ed40: a BlockCollider hit records TimeSeconds; a later
    /// body hit of that attack is parried within Timed/HeldBlockMemoryDuration); keys geometry::attack_key
    pub blocked_attacks: Vec<(u64, f64)>,
}

impl World {
    fn pr(&self, fi: usize, id: MotionId) -> &ParryMotion {
        self.m(fi, id).parry().unwrap()
    }
    fn pr_mut(&mut self, fi: usize, id: MotionId) -> &mut ParryMotion {
        self.mm(fi, id).parry_mut().unwrap()
    }

    /// from UParryMotion::GetMovementRestriction rva=0x165cc30 (disasm 0x14165cc45..0x14165cc83): Recovery ->
    /// Successful (+0x4a4) / Failed (+0x4a5) ParryRecoveryMovementRestriction; Parry -> WeaponPtr's
    /// BlockMovementRestriction (+0x1a34), or the motion's own field without a WeaponPtr
    pub(crate) fn parry_movement_restriction(&self, fi: usize, id: MotionId) -> i64 {
        let m = self.m(fi, id);
        let p = m.parry().unwrap();
        let pd = m.def.parry();
        if p.stage == ps::RECOVERY {
            return if p.total_blocks != 0 { pd.successful_parry_recovery_movement_restriction } else { pd.failed_parry_recovery_movement_restriction };
        }
        match self.parry_weapon(fi) {
            Some(w) => enums::movement_restriction_of(&w.block_movement_restriction),
            None => m.movement_restriction,
        }
    }

    /// from UParryMotion::OnBegin_Implementation rva=0x1660f70 (WeaponPtr reads disasm 0x141661212 [rsi+0x1a08],
    /// 0x14166121b [rsi+0x1a0c], 0x141661241 [rsi+0x19b0], 0x141661250 [rsi+0x1a10])
    pub(crate) fn parry_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let def = self.m(fi, id).def.clone();
        let pd = def.parry();
        let c = self.spec.constants.clone();
        let block_param = self.fighters[fi].net.param0;
        self.fighters[fi].last_parry_motion = Some(id);
        if self.exe() {
            // OnBegin_Implementation (decomp UParryMotion.cpp 970): EnableBlockCollider
            self.fighters[fi].block_collider_enabled = true;
        }
        let wd = self.parry_weapon(fi).expect("parry without a weapon");
        let cf = self.m(fi, id).coming_from;
        let live_riposte_cost = cf.and_then(|c| self.m(fi, c).attack()).and_then(|a| {
            (a.ty == enums::at::RIPOSTE && a.stage != enums::stage::RECOVERY && !a.b_has_hit).then_some(a.ai.feint_cost)
        });
        {
            let p = self.pr_mut(fi, id);
            p.recovery_type = -1;
            p.last_blocked_move = -1;
            p.parry_up_time = pd.parry_up_time;
            p.parry_recovery_time = pd.parry_recovery_time;
            p.minimum_held_parry_time = pd.minimum_held_parry_time;
            p.non_held_parry_extension_time = pd.non_held_parry_extension_time;
            p.block_type = block_param;
            if p.block_type == bt::SHIELD_WALL {
                p.b_is_shield_wall = true;
                p.block_type = 0;
            }
            p.b_is_block_holdable = wd.b_is_parry_held;
            p.held_stamina_drain = wd.parry_held_stamina_drain;
            p.backpedal_speed_factor = wd.parry_backpedal_speed_factor;
        }
        // UParryMotion::OnBegin rva=0x1660f70: shield wall SpeedFactor = ShieldWallSpeedFactor (decomp 808),
        // BackpedalSpeedFactor = the weapon's ParryBackpedalSpeedFactor (decomp 835)
        {
            let (sw, bp) = { let p = self.pr(fi, id); (p.b_is_shield_wall, p.backpedal_speed_factor) };
            let m = self.mm(fi, id);
            if sw {
                m.speed_factor = m.def.base.shield_wall_speed_factor;
            }
            m.backpedal_speed_factor = bp;
        }
        let p = self.pr(fi, id);
        let offset = if !p.b_is_block_holdable && !p.b_is_shield_wall { wd.parry_window_offset } else { 0.0 };
        // parrying out of an unfinished, unhit riposte costs that riposte's FeintCost (feint-to-parry)
        if let Some(cost) = live_riposte_cost {
            self.offset_stamina(fi, -cost);
            let p = self.pr_mut(fi, id);
            if p.b_is_block_holdable {
                p.minimum_held_parry_time = pd.minimum_held_riposte_parry_time;
            }
        }
        let start = self.m(fi, id).start_time;
        let p = self.pr_mut(fi, id);
        if p.b_is_shield_wall {
            p.parry_up_time = pd.shield_wall_raise_time;
        }
        p.parry_recovery_time = q(p.parry_recovery_time - offset);
        p.parry_up_time = q(offset + p.parry_up_time);
        p.parry_end = q(start + c.parry_open_end); // 1e6 s (.rdata 0x144349db0)
        let pe = p.parry_end;
        self.mm(fi, id).end_time = pe;
        // parry_angle proof B2: OnBegin (decomp UParryMotion.cpp 919-934) SetTurnCaps (vcall +0x9e8) with the parry
        // weapon's ParryTurnCap (+0x19f0), or ShieldWallTurnCap (+0x19f8) in a shield wall; (450, 450) without one
        let sw = self.pr(fi, id).b_is_shield_wall;
        let cap = match self.parry_weapon(fi) {
            Some(w) if sw => (w.shield_wall_turn_cap.xf(), w.shield_wall_turn_cap.yf()),
            Some(w) => (w.parry_turn_cap.xf(), w.parry_turn_cap.yf()),
            None => (450.0, 450.0),
        };
        self.set_turn_caps(fi, cap.0, cap.1);
    }

    /// from UParryMotion::OnTick_Implementation rva=0x1668980 (state part)
    pub(crate) fn parry_on_tick(&mut self, fi: usize, id: MotionId, dt: f64) {
        let q = self.qf();
        let t = self.now;
        let start = self.m(fi, id).start_time;
        let p = self.pr(fi, id);
        if p.stage == ps::PARRY && q(q(p.parry_up_time + start) + p.non_held_parry_extension_time) < t && !p.b_is_block_holdable && !p.b_is_shield_wall {
            self.parry_stop_holding(fi, id);
        }
        if self.pr(fi, id).stage != ps::PARRY {
            return;
        }
        let p = self.pr_mut(fi, id);
        if 0.0 < p.held_stamina_drain {
            p.cumulative_stamina_drain = q(q(p.held_stamina_drain * dt) + p.cumulative_stamina_drain);
            if 1.0 < p.cumulative_stamina_drain {
                p.cumulative_stamina_drain -= 1.0;
                self.offset_stamina(fi, -1);
            }
        }
        self.detect_attack_traces(fi, id);
        // held parry released after MinimumHeldParryTime -> ServerDropParry (locally controlled / authority gate)
        let p = self.pr(fi, id);
        if !p.b_is_shield_wall
            && p.b_is_block_holdable
            && self.drops_parry_locally(fi)
            && !self.fighters[fi].holding_block
            && !p.b_requested_drop
            && q(p.minimum_held_parry_time + start) < t
        {
            self.pr_mut(fi, id).b_requested_drop = true;
            self.server_drop_parry(fi);
        }
    }

    /// UParryMotion::OnTick_Implementation rva=0x1668980, Parry stage (decomp UParryMotion.cpp 1290-1381): each
    /// AttackTracesMemory entry of another, non-friendly owner line-traced against this character's BlockCollider
    /// (vcall +0x848 LineTraceComponent) sets bDetectedAnyNonFriendlyAttack. Needs the actor transform; headless
    /// fighters without one skip it.
    fn detect_attack_traces(&mut self, fi: usize, id: MotionId) {
        if self.pr(fi, id).b_detected_any_non_friendly_attack {
            return;
        }
        let Some(axf) = self.fighters[fi].actor_xf else { return };
        let bc = self.spec.block_collider;
        let mut box_xf = crate::ue::Xform::IDENTITY;
        box_xf.origin = bc.centre;
        let box_xf = axf.mul(&box_xf);
        let me = self.fighters[fi].id;
        let mut hit = false;
        for e in &self.attack_traces_memory {
            if e.owner == me {
                continue;
            }
            if self.is_friendly(self.fighter_by_id(e.owner), Some(fi)) {
                continue;
            }
            if segment_box(e.start, e.end, &box_xf, bc.half) >= 0.0 {
                hit = true;
                break;
            }
        }
        if hit {
            self.pr_mut(fi, id).b_detected_any_non_friendly_attack = true;
        }
    }

    /// from UParryMotion::StopHolding rva=0x166db50 (disasm 0x14166db50..0x14166dbc9):
    /// AssignNetMotionDynamicParam(((TotalBlocks == 0 && bDetectedAnyNonFriendlyAttack) ? 2 : 1) | (dyn & 0xf0))
    pub(crate) fn parry_stop_holding(&mut self, fi: usize, id: MotionId) {
        let p = self.pr(fi, id);
        let detected = if p.total_blocks == 0 { p.b_detected_any_non_friendly_attack } else { false };
        let request = if detected { 2 } else { 1 };
        let d = self.fighters[fi].net.dynamic_param;
        self.assign_net_motion_dynamic_param(fi, request | (d & 0xf0));
    }

    /// from UParryMotion::OnDynamicParamChanged_Implementation rva=0x1663210
    pub(crate) fn parry_on_dynamic_param_changed(&mut self, fi: usize, id: MotionId, old: i64, nv: i64) {
        let q = self.qf();
        let def = self.m(fi, id).def.clone();
        let pd = def.parry();
        let c = self.spec.constants.clone();
        let t = self.now;
        let (start, ed) = (self.m(fi, id).start_time, self.m(fi, id).expected_delay);
        let req = enums::lo(nv);
        self.pr_mut(fi, id).total_blocks = enums::hi(nv);
        let p = self.pr(fi, id);
        if req != 0 && p.stage == ps::PARRY {
            let (sw, hold, tb, prt) = (p.b_is_shield_wall, p.b_is_block_holdable, p.total_blocks, p.parry_recovery_time);
            self.pr_mut(fi, id).parry_end = start;
            if tb == 0 || sw {
                let rec;
                let mut kind = pr::FAIL;
                if !sw && !hold && pd.miss_parry_recovery_time != 0.0 {
                    self.pr_mut(fi, id).b_is_miss_parry = req == 2;
                    if req == 2 {
                        rec = pd.miss_parry_recovery_time;
                        kind = pr::MISS;
                    } else {
                        rec = prt;
                    }
                } else {
                    self.pr_mut(fi, id).b_is_miss_parry = false;
                    rec = if sw { pd.shield_wall_recovery_time } else if hold { pd.held_parry_recovery_time } else { prt };
                }
                self.mm(fi, id).end_time = q(q(rec + t) - ed);
                self.enter_parry_recovery(fi, id, kind);
            } else {
                let rec2 = if hold { pd.held_parry_success_recovery_time } else { pd.parry_success_recovery_time };
                self.mm(fi, id).end_time = q(q(rec2 + t) - ed);
                self.enter_parry_recovery(fi, id, pr::SUCCESS);
            }
        }
        if self.pr(fi, id).total_blocks == enums::hi(old) {
            return;
        }
        // a new block landed: open the riposte window; fire a riposte queued shortly before the block
        let mut up = c.shield_wall_riposte_window; // shield-wall branch (.rdata 0x143fe4e00)
        let p = self.pr(fi, id);
        if !p.b_is_shield_wall {
            up = p.parry_up_time;
            if !p.b_is_block_holdable && enums::hi(old) != 0 {
                return;
            }
            let hold = p.b_is_block_holdable;
            let pm = self.pr_mut(fi, id);
            pm.riposte_window_start = t;
            if !hold {
                pm.non_held_parry_extension_time = pd.non_held_parry_extension_and_riposte_window_extra;
            }
            // (vehicle rider: a separate .rdata constant; no vehicles here, not ported)
        } else {
            self.pr_mut(fi, id).riposte_window_start = t;
        }
        let p = self.pr(fi, id);
        if !p.b_has_queued_move {
            return;
        }
        if t <= q(up + p.queued_move_time) {
            let (qm, qa) = (p.queued_move, p.queued_angle);
            self.assign_net_attack_motion(fi, enums::at::RIPOSTE, qm, qa);
        }
    }

    /// from UParryMotion::EnterParryRecovery rva=0x1650540 (state part)
    fn enter_parry_recovery(&mut self, fi: usize, id: MotionId, kind: i64) {
        let now = self.now;
        let p = self.pr_mut(fi, id);
        p.recovery_type = kind;
        p.recovery_start_time = now;
        p.stage = ps::RECOVERY;
        // UParryMotion::EnterParryRecovery rva=0x1650540 (decomp 1482): BackpedalSpeedFactor = 1
        self.mm(fi, id).backpedal_speed_factor = 1.0;
    }

    /// riposte window length of UParryMotion::ProcessAttack_Implementation rva=0x166b870
    fn riposte_window(&self, fi: usize, id: MotionId) -> f64 {
        let q = self.qf();
        let m = self.m(fi, id);
        let pd = m.def.parry();
        let p = m.parry().unwrap();
        let base = pd.riposte_window_base;
        if p.b_is_shield_wall {
            return self.spec.constants.shield_wall_riposte_window;
        }
        if !p.b_is_block_holdable {
            return q(base + pd.non_held_parry_extension_and_riposte_window_extra);
        }
        if self.fighters[fi].holding_block {
            return q(base + pd.held_riposte_window_extra);
        }
        base
    }

    /// from UParryMotion::ProcessAttack_Implementation rva=0x166b870
    pub(crate) fn parry_process_attack(&mut self, fi: usize, id: MotionId, m: i64, angle: f64) -> bool {
        let q = self.qf();
        let t = self.now;
        let start = self.m(fi, id).start_time;
        {
            let p = self.pr_mut(fi, id);
            p.b_has_queued_move = true;
            p.queued_move = m;
            if !p.b_is_shield_wall {
                if m == mv::STAB && p.block_type == 0 {
                    p.queued_move = mv::ALT_STAB;
                } else if m == mv::ALT_STAB && p.block_type == 1 {
                    p.queued_move = mv::STAB;
                }
            } else if m == mv::ALT_STAB {
                p.queued_move = mv::STAB;
            }
            p.queued_angle = angle;
            if enums::is_stab(p.queued_move) {
                p.queued_angle = 0.0;
            }
            p.queued_move_time = t;
        }
        let win = self.riposte_window(fi, id);
        let p = self.pr(fi, id);
        if (p.b_is_shield_wall && q(p.parry_up_time + start) < t && p.stage != ps::RECOVERY) || (p.total_blocks != 0 && t <= q(win + p.riposte_window_start)) {
            let (qm, qa) = (p.queued_move, p.queued_angle);
            self.assign_net_attack_motion(fi, enums::at::RIPOSTE, qm, qa);
            return !self.is_current(fi, id);
        }
        if p.b_is_shield_wall {
            return false;
        }
        if p.b_is_block_holdable {
            if !self.fighters[fi].holding_block && (p.total_blocks != 0 || q(p.minimum_held_parry_time + start) < t) {
                return self.base_process_attack(fi, id, m, angle);
            }
            return false;
        }
        if p.total_blocks != 0 {
            return self.base_process_attack(fi, id, m, angle);
        }
        false
    }

    /// from UParryMotion::ProcessBlock_Implementation rva=0x166bbb0
    pub(crate) fn parry_process_block(&mut self, fi: usize, id: MotionId, b: i64) -> bool {
        let m = self.m(fi, id);
        let p = m.parry().unwrap();
        if !p.b_is_block_holdable {
            if p.total_blocks == 0 {
                return false;
            }
        } else if p.stage != ps::RECOVERY {
            return false;
        }
        if p.b_is_shield_wall && p.stage == ps::PARRY {
            return false;
        }
        let mut b2 = b;
        if b < 2 && p.block_type < 2 {
            b2 = p.block_type;
        }
        if !m.b_can_block {
            return false;
        }
        self.assign_net_parry(fi, b2);
        true
    }

    /// from UParryMotion::ProcessFeint_Implementation rva=0x166bd40: shield wall lowered after its raise time
    pub(crate) fn parry_process_feint(&mut self, fi: usize, id: MotionId) -> bool {
        let q = self.qf();
        let start = self.m(fi, id).start_time;
        let p = self.pr(fi, id);
        if p.b_is_shield_wall && p.stage == ps::PARRY && !p.b_requested_drop && q(p.parry_up_time + start) < self.now {
            self.server_drop_parry(fi);
            self.pr_mut(fi, id).b_requested_drop = true;
            return true;
        }
        false
    }

    /// from UParryMotion::CheckParry rva=0x164ed40 (decomp UParryMotion.cpp 975-1275): can this parry (the defender
    /// `fi`'s LastParryMotion `id`, current or not) block `attacker`'s attack? Non-geometry gates in the exe's order;
    /// the BlockCollider forward test / BlockedAttacks / parry angles are the caller's contact.
    pub fn check_parry(&mut self, fi: usize, id: MotionId, attacker: usize) -> bool {
        let q = self.qf();
        let t = self.now;
        let m = self.m(fi, id);
        let p = m.parry().unwrap();
        let pd = m.def.parry();
        if p.stage == ps::PARRY {
            if p.b_is_shield_wall && t < q(p.parry_up_time + m.start_time) {
                return false;
            }
        } else {
            let in_flinch = self.cur_m(fi).map(|c| c.is_flinch()).unwrap_or(false);
            if !in_flinch || p.b_is_shield_wall {
                return false;
            }
            if q(pd.parry_in_flinch_duration_max + m.leave_time) < t {
                return false;
            }
            if !p.b_is_block_holdable && q(q(p.parry_up_time + m.start_time) + p.non_held_parry_extension_time) < t {
                return false;
            }
        }
        let Some(wd) = self.parry_weapon(fi) else { return false };
        let Some(atk) = self.cur_m(attacker).and_then(|m| m.attack()) else { return false };
        let Some(aw) = atk.weapon.as_ref() else { return false };
        let friendly = self.is_friendly(Some(fi), Some(attacker));
        if !(atk.stage == enums::stage::RELEASE || !friendly) {
            return false;
        }
        let mut mask = wd.parry_mask;
        if !p.b_is_block_holdable && !p.b_is_shield_wall {
            mask |= 2;
        }
        if (mask & aw.attack_mask) == 0 {
            return false;
        }
        let windup = atk.stage == enums::stage::WINDUP;
        if !friendly {
            self.pr_mut(fi, id).b_detected_any_non_friendly_attack = true;
        }
        !windup
    }

    /// from UParryMotion::OnLeave_Implementation rva=0x1665b10 (state part): a parry left before its recovery enters
    /// it (Fail without a block, else Success)
    pub(crate) fn parry_on_leave(&mut self, fi: usize, id: MotionId) {
        let p = self.pr(fi, id);
        if p.stage != ps::RECOVERY {
            let k = if p.total_blocks == 0 { pr::FAIL } else { pr::SUCCESS };
            self.enter_parry_recovery(fi, id, k);
        }
        // parry_angle proof B2: OnLeave_Implementation rva=0x1665b10 (decomp UParryMotion.cpp 2119-2120) SetTurnCaps(-1, -1)
        self.set_turn_caps(fi, -1.0, -1.0);
        // OnLeave_Implementation rva=0x1665b10 (byte-matched): DisableBlockCollider unless the new motion is a flinch
        if self.exe() && !self.cur_m(fi).map(|m| m.is_flinch()).unwrap_or(false) {
            self.fighters[fi].block_collider_enabled = false;
        }
    }

    /// from UParryMotion::ReceiveBlock rva=0x166bf90 (drain = float(int(StaminaDrain)) [+ ExtraStaminaDrainVsHeldBlock]
    /// from CheckParry disasm 0x14164f2a3..0x14164f308; ChamberFTPExtraStaminaDrain loop `while (i < 5)`; WeaponPtr
    /// BlockStaminaNegation +0x1a38 / Clamp +0x1a3c disasm 0x14166c1a0..0x14166c1ba)
    pub fn receive_block(&mut self, fi: usize, id: MotionId, drain: f64, attacker_move: i64, attacker: Option<usize>) {
        let q = self.qf();
        let def = self.m(fi, id).def.clone();
        let pd = def.parry();
        let c = self.spec.constants.clone();
        let t = self.now;
        let mut drain = drain;
        self.pr_mut(fi, id).last_blocked_move = attacker_move;
        if t <= self.fighters[fi].easy_parry_until_time {
            drain = trunc_i(pd.easy_parry_stamina_cost) as f64;
        }
        if let Some(a) = attacker {
            if let Some(atk_we) = self.cur_m(a).and_then(|m| m.attack()).map(|x| x.windup_end) {
                let mut old = self.fighters[fi].last_attack_motion;
                for _ in 0..5 {
                    let Some(mut o) = old else { break };
                    if self.chamber_valid_ignore_timing(fi, o, a) {
                        let oa = self.att(fi, o);
                        if oa.ty == enums::at::MORPH && oa.previous_last_attack.is_some() {
                            o = oa.previous_last_attack.unwrap();
                        }
                        let om = self.m(fi, o);
                        if q(atk_we - om.def.attack().chamber_window) <= om.start_time {
                            self.offset_stamina(fi, trunc_i(-pd.chamber_ftp_extra_stamina_drain));
                            break;
                        }
                    }
                    old = self.att(fi, o).previous_last_attack;
                }
            }
        }
        if drain <= 0.0 {
            if drain < 0.0 {
                self.offset_stamina(fi, trunc_i(-drain));
            }
        } else if self.is_current(fi, id) {
            let wd = self.parry_weapon(fi).unwrap(); // WeaponPtr (+0x550)
            let clamp = wd.block_stamina_clamp;
            let v = q(drain - wd.block_stamina_negation);
            let mut f = clamp.xf();
            if clamp.xf() <= v {
                f = if v < clamp.yf() { v } else { clamp.yf() };
            }
            if self.pr(fi, id).b_is_shield_wall {
                f = -(cvtss2si(q(c.shield_wall_drain_round_bias - q(q(f + f) * pd.shield_wall_stamina_drain_factor))) >> 1) as f64;
            }
            self.offset_stamina(fi, trunc_i(-f));
        }
        if self.stamina_byte(fi) != 0 || attacker_move == mv::RANGED {
            if let Some(h) = self.net.clone() {
                // game/net: AssignNetBlock(Parry) + ApplyBackwardsKnockbackIfNotInKnockback (net.rs)
                let sw = self.pr(fi, id).b_is_shield_wall;
                h.receive_block(self, fi, attacker_move, sw);
            }
            if self.is_current(fi, id) {
                self.fighters[fi].easy_parry_until_time = q(t + pd.easy_parry_duration);
                let tb = self.pr(fi, id).total_blocks;
                let step = if tb + 1 < 11 { 1 } else { -1 };
                let d = self.fighters[fi].net.dynamic_param;
                self.assign_net_motion_dynamic_param(fi, enums::pack(step + tb, enums::lo(d)));
            }
        } else {
            self.parry_stamina_break(fi, id, attacker_move);
        }
    }

    /// The stamina-break branch of UParryMotion::ReceiveBlock rva=0x166bf90 (stamina byte 0, not ranged): perks not
    /// ported (stun false); WeaponPtr !bAllowDrop (+0xcb8), bDestroyEquipmentOnDeath (+0x11e0) or
    /// bAlwaysStunInsteadOfDisarm (+0x1219) -> stun without disarm; direction -90 for a LeftStrike else 90; fists:
    /// += BlockStaminaRecover only (ragdoll not ported).
    fn parry_stamina_break(&mut self, fi: usize, id: MotionId, attacker_move: i64) {
        let recover = self.m(fi, id).def.parry().block_stamina_recover;
        let wd = self.parry_weapon(fi);
        let eq = self.parry_weapon_equip(fi);
        let ch = self.fighters[fi].character.clone();
        let mut stun = false;
        let mut disarm = true;
        if wd.as_ref().map(|w| !w.b_allow_drop).unwrap_or(false) || ch.b_destroy_equipment_on_death || ch.b_always_stun_instead_of_disarm {
            disarm = false;
            stun = true;
        }
        let direction = if attacker_move == mv::LEFT_STRIKE { -90.0 } else { 90.0 };
        let is_shield = eq.as_ref().map(|e| e.is_shield).unwrap_or(false);
        let is_fists = eq.as_ref().map(|e| self.spec.is_class_of(&e.native, "AFistsWeapon")).unwrap_or(false);
        let name = self.fighters[fi].name.clone();
        let what = if is_fists { "fists" } else if stun { "stun" } else { "disarm" };
        self.trace_event(&format!("{name} parry stamina break: {what}"));
        if is_fists {
            self.offset_stamina(fi, recover);
            self.emit_event(json!({"kind": "stamina_break", "who": name, "fists": true, "stun": false, "disarm": false}));
            return;
        }
        if !stun {
            if !is_shield {
                self.offset_stamina(fi, recover);
            }
            self.assign_net_disarmed(fi, direction);
        } else {
            self.offset_stamina(fi, recover);
            self.assign_net_stunned(fi, direction, 0, disarm);
        }
        self.emit_event(json!({"kind": "stamina_break", "who": name, "stun": stun, "disarm": disarm}));
    }
}

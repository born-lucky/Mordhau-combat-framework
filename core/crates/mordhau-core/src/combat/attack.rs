//! UAttackMotion (+ the UStrikeMotion / UStabMotion / UKickMotion overrides that change timing or rules), ported
//! from godot/game/combat/attack_motion.gd (extract/native/decomp/UAttackMotion.cpp, UStrikeMotion.cpp,
//! UStabMotion.cpp, UKickMotion.cpp; field offsets UAttackMotion.h). Not ported (no effect on the state trace): the
//! montage playback, angling additives, supersprint, turn caps, sounds; the AutoBlend windup-offset search needs the
//! animation layer's poses (the reference runs it only with MotionSystem.anim_poses attached; headless it keeps the
//! class default offset, as here).

use super::enums::{self, at, br, ft, mr, mv, stage};
use super::motion::MotionId;
use super::world::{TraceMemory, World};
use crate::data::{AttackDef, AttackInfo, MotionDef, WeaponData};
use crate::ue::{clampf, maxf, minf, trunc_i, FVector};
use std::rc::Rc;

#[derive(Default)]
pub struct AttackMotion {
    pub native: String,                 // UStrikeMotion / UStabMotion / UKickMotion ...
    pub weapon: Option<Rc<WeaponData>>, // +0x10c8 Weapon (FindWeapon)
    pub weapon_actor: Option<super::equipment::EquipmentId>, // same captured actor, separate from class data
    pub ai: AttackInfo,                 // +0xe38 AttackInfo, a copy of the weapon's, then ModifyAttackInfo'd
    pub ty: i64,                        // +0x108d EAttackType
    pub mv: i64,                        // +0x108e EAttackMove
    pub stage: i64,                     // +0x10e9 EAttackStage
    pub windup_end: f64,                // +0x1090
    pub release_end: f64,               // +0x1094
    pub lag_induction: f64,             // +0x1098 (0 offline)
    pub lag_reduction: f64,             // +0x109c (0 offline)
    pub angle_target: f64,              // +0x1080
    pub last_release_norm: f64,         // +0x1084
    pub last_windup_norm: f64,          // +0x1088
    pub b_has_queued_move: bool,        // +0x10e1
    pub queued_move: i64,               // +0x10e8
    pub queued_angle: f64,              // +0x10e4
    pub b_has_hit: bool,                // +0x10ea
    pub b_has_hit_friendly: bool,       // +0x108c
    /// +0x10eb bHasHitIncludingCosmeticHit: set with bHasHit (OnDynamicParamChanged), by a cosmetic hit on a character
    /// mesh in OnLateTick (a network client only, not modelled) and by SetHasHitIncludingCosmeticHit; read by the anim
    /// instance's hit-effect IK (mh-sim procedural.rs HitEffect)
    pub b_has_hit_including_cosmetic_hit: bool,
    pub b_has_chambered: bool,          // +0x10f4
    pub b_has_considered_combo: bool,   // +0x1070
    pub b_is_combo_from_miss: bool,     // +0xa93
    pub b_riposte_ate_feint_input: bool, // +0xb99
    pub coming_from_move: i64,          // +0xe2a
    pub previous_last_attack: Option<MotionId>, // +0x10d0
    pub first_hit_release_norm: f64,    // +0x10f0 FirstHitIncludingCosmeticReleaseNormalizedTime
    pub first_hit_time: f64,            // +0x10ec
    pub global_damage_modifier: f64,    // +0xa84
    pub early_release: f64,             // +0xb48
    pub early_release_tf: f64,          // +0xb4c
    pub windup_anim_offset: f64,        // +0x10a4
    pub windup_curve: String,           // +0xdf8
    pub release_curve: String,          // +0xe18
    pub hit_actors: Vec<u32>,           // weapon +0xe98 ActorSetCache (Fighter::id)
    pub b_has_last_trace: bool,         // +0xa2
    pub last_trace_start: FVector,      // +0xa4
    pub last_trace_end: FVector,        // +0xb0
    pub bounce_montage: String,         // +0xae0
    pub b_is_air_kick: bool,            // UKickMotion +0x1114
    /// +0xd98 BlockedAttacks TSet<TWeakObjectPtr<UAttackMotion>> (CheckChamber / CheckAttackParry: attacks that entered
    /// this attacker's BlockCollider); keys geometry::attack_key
    pub blocked_attacks: Vec<u64>,
}

impl AttackMotion {
    /// SetHasHitIncludingCosmeticHit RVA163dff0: latch stored LRN on the first transition only.
    pub fn set_has_hit_including_cosmetic_hit(&mut self) {
        if !self.b_has_hit_including_cosmetic_hit {
            self.first_hit_release_norm = self.last_release_norm;
            self.b_has_hit_including_cosmetic_hit = true;
        }
    }

    pub fn is_strike_class(&self) -> bool {
        self.native == "UStrikeMotion"
    }
    pub fn is_stab_class(&self) -> bool {
        self.native == "UStabMotion"
    }
    /// from UAttackMotion::IsInLiveRecovery rva=0x162c130: (Stage & 0xfd) == 0
    pub fn is_in_live_recovery(&self) -> bool {
        (self.stage & 0xfd) == 0
    }
    /// from UAttackMotion::ConvertToCombo rva=0x1619d40 (ok, move)
    pub fn convert_to_combo(&self, req: i64) -> (bool, i64) {
        let cur = self.mv;
        if cur == req {
            if req > 3 {
                return (false, req);
            }
            return (true, enums::flip_side(req));
        }
        if cur > 3 || req > 3 {
            return (true, req);
        }
        if enums::is_left(cur) != enums::is_left(req) {
            return (true, req);
        }
        (true, enums::flip_side(req))
    }
    /// from UStrikeMotion::ModifyRequestedMorphAttack rva=0x165d960 / UStabMotion rva=0x165d900; base rva=0x7bf350
    /// no-op. morph_stab_angle_scale = 60 (.rdata 0x143fe4e38)
    pub fn modify_requested_morph_attack(&self, m: i64, angle: f64, morph_stab_angle_scale: f64) -> (i64, f64) {
        if !(self.is_strike_class() || self.is_stab_class()) {
            return (m, angle);
        }
        if !(enums::is_strike(m) || enums::is_stab(m)) {
            return (m, angle);
        }
        let mut m = m;
        let mut angle = angle;
        if enums::is_left(m) != enums::is_left(self.mv) {
            m = enums::flip_side(m);
        }
        if self.is_strike_class() && enums::is_stab(m) {
            angle = self.angle_target * morph_stab_angle_scale;
        }
        (m, angle)
    }
    /// from UStrikeMotion::ModifyRequestedBlockType rva=0x165d8c0 / UStabMotion rva=0x165d880; base rva=0x154fc80
    pub fn modify_requested_block_type(&self, b: i64) -> i64 {
        if self.is_strike_class() {
            if enums::is_left(self.mv) {
                return if b == 1 { 0 } else { b };
            }
            return if b == 0 { 1 } else { b };
        }
        if self.is_stab_class() {
            if enums::is_left(self.mv) {
                return if b == 0 { 1 } else { b };
            }
            return if b == 1 { 0 } else { b };
        }
        b
    }
    fn queue(&mut self, m: i64, angle: f64) -> bool {
        self.queued_angle = angle;
        self.b_has_queued_move = true;
        self.queued_move = m;
        false
    }
}

/// AMordhauWeapon::GetBaseAttackInfo rva=0x1620c50: Kick -> KickAttack, Stab/AltStab -> StabAttack, Couch ->
/// CouchAttack, Bash -> BashAttack, anything else -> StrikeAttack
pub fn attack_info_for(w: &WeaponData, m: i64) -> &AttackInfo {
    match m {
        mv::KICK => &w.kick,
        mv::STAB | mv::ALT_STAB => &w.stab,
        mv::COUCH => &w.couch,
        mv::BASH => &w.bash,
        _ => &w.strike,
    }
}

/// AttackAutoBlend.settings: UAttackMotion::PrepareAnimationData_Implementation rva=0x1637140 outputs
/// (EarlyRelease, EarlyReleaseTimeFactor; Riposte or PostClash take the Riposte* pair)
pub fn early_release_settings(d: &AttackDef, ty: i64) -> (f64, f64) {
    if ty == at::RIPOSTE || ty == at::POST_CLASH {
        return (d.riposte_early_release, d.riposte_early_release_time_factor);
    }
    (d.early_release, d.early_release_time_factor)
}

/// UAttackMotion::GetSmoothedWindUpNormalizedTime rva=0x1628bf0 (AttackAutoBlend.smoothed_windup)
pub fn smoothed_windup(d: &AttackDef, start: f64, windup_end: f64, release_end: f64, offset: f64, x: f64, small: f64) -> f64 {
    if !d.enable_windup_smoothing {
        return x;
    }
    let rel = release_end - windup_end;
    let span = 1.0 - (offset + offset);
    if rel.abs() > small && span.abs() > small {
        let ec = d.windup_smoothing_exponent_clamp;
        return x.powf(clampf((windup_end - start) / (rel * span), ec.xf(), ec.yf()));
    }
    x
}

/// Exe form of GetSmoothedWindUpNormalizedTime rva=0x1628bf0 (byte-matched C, src/.../AttackMotion.cpp: float
/// OffsetTerm / ReleaseTerm, FMath::Pow = powf on binary32)
pub fn smoothed_windup_exe(d: &AttackDef, start: f64, windup_end: f64, release_end: f64, offset: f64, x: f64, small: f64) -> f64 {
    use crate::ue::f32r as q;
    if !d.enable_windup_smoothing {
        return x;
    }
    let off = q(1.0 - q(offset + offset));
    let rel = q(release_end - windup_end);
    if rel.abs() > small && off.abs() > small {
        let ec = d.windup_smoothing_exponent_clamp;
        let e = clampf(q(q(windup_end - start) / q(rel * off)), ec.xf(), ec.yf());
        return (x as f32).powf(e as f32) as f64;
    }
    x
}

impl World {
    fn adef(&self, fi: usize, id: MotionId) -> Rc<MotionDef> {
        self.m(fi, id).def.clone()
    }

    /// Montage (+0x10b0) != null: taken as set whenever the attack has something to play (its Animation, or the kick
    /// montages for a UKickMotion). UNCONFIRMED: that PlayAnim returns a montage on a dedicated server.
    pub fn attack_has_montage(&self, fi: usize, id: MotionId) -> bool {
        let m = self.m(fi, id);
        m.def.attack().animation != "" || self.spec.is_class_of(&m.attack().unwrap().native, "UKickMotion")
    }

    /// from UAttackMotion::GetMovementRestriction rva=0x16245a0 (disasm 0x1416245a0..0x1416245c2): bIsComboFromMiss
    /// -> PartialSprint; Recovery -> !bHasHit; else None. UKickMotion::GetMovementRestriction rva=0x165cbe0: airborne
    /// (vcall +0x9e0) -> JumpKickAirMovementRestriction (+0x110c), else the field (+0x61).
    pub(crate) fn attack_movement_restriction(&self, fi: usize, id: MotionId) -> i64 {
        let m = self.m(fi, id);
        let a = m.attack().unwrap();
        if a.native == "UKickMotion" {
            if self.fighters[fi].airborne {
                return m.def.kick.as_ref().map(|k| k.jump_kick_air_movement_restriction).unwrap_or(0);
            }
            return m.movement_restriction;
        }
        if a.b_is_combo_from_miss {
            return mr::PARTIAL_SPRINT;
        }
        if a.stage == stage::RECOVERY {
            return if a.b_has_hit { mr::NONE } else { mr::PARTIAL_SPRINT };
        }
        mr::NONE
    }

    /// from UAttackMotion::OnBegin_Implementation rva=0x162eda0 (timeline part) + ModifyAttackInfo + ComputeWindup
    pub(crate) fn attack_on_begin(&mut self, fi: usize, id: MotionId) {
        let def = self.adef(fi, id);
        let ad = def.attack();
        let c = self.spec.constants.clone();
        let native = def.base.native.clone();
        let weapon = self.find_weapon(fi, &native);
        let weapon_actor = self.find_weapon_actor(fi, &native);
        let self_id = self.fighters[fi].id;
        self.fighters[fi].tracer.actor_ignore_cache = vec![self_id]; // OnBegin: ActorIgnoreCache = [owner]
        if self.exe() {
            // OnBegin_Implementation (decomp 2253): EnableBlockCollider; OnAttackStarted enables the ClashCollider
            self.fighters[fi].block_collider_enabled = true;
            self.fighters[fi].clash_collider_enabled = true;
            // OnBegin_Implementation (decomp UAttackMotion.cpp 2240) -> AMordhauWeapon::OnAttackStarted_Implementation
            // rva=0x162eb40 -> ResetTracers (vcall +0x7d8): the new attack's first PrepareForTracing is invalid, so
            // nothing is swept from where the weapon was when the previous attack ended (exe.rs difference 9)
            self.fighters[fi].tracer.reset_tracers();
            if let Some(h) = self.trace_host.clone() {
                h.reset(self, fi);
            }
        }
        let last = self.fighters[fi].last_attack_motion;
        let net = self.fighters[fi].net;
        {
            let start = self.m(fi, id).start_time;
            let _ = start;
        }
        let last_move = last.map(|l| self.att(fi, l).mv);
        let last_hit = last.map(|l| self.att(fi, l).b_has_hit).unwrap_or(false);
        let last_morph_cost = last.map(|l| self.att(fi, l).ai.morph_cost);
        {
            let a = self.att_mut(fi, id);
            a.native = native.clone();
            a.weapon = weapon.clone();
            a.weapon_actor = weapon_actor;
            a.global_damage_modifier = 1.0;
            a.early_release_tf = 1.0;
            if let Some(l) = last {
                a.previous_last_attack = Some(l);
                a.coming_from_move = last_move.unwrap();
            }
            a.ty = enums::hi(net.param0); // Param0 = SetHI(EAttackType) | SetLO(EAttackMove)
        }
        let mut extra = 0.0;
        let ty = self.att(fi, id).ty;
        if ty == at::MISS_COMBO {
            self.att_mut(fi, id).ty = at::COMBO;
            // authority: the previous attack hit after the combo was queued -> dynamic param bit 4
            // (OnBegin disasm 0x14162ef3c..0x14162ef59: PreviousLastAttackMotion->bHasHit, MotionDynamicParam | 4)
            let late = enums::DYN_PARENT_HIT_LATE;
            let prev = self.att(fi, id).previous_last_attack;
            if prev.is_some() && last_hit && (net.dynamic_param | late) != net.dynamic_param {
                self.fighters[fi].net.dynamic_param = net.dynamic_param | late;
            }
            if (self.fighters[fi].net.dynamic_param & late) == 0 {
                self.att_mut(fi, id).b_is_combo_from_miss = true;
            }
        } else if ty == at::POST_CLASH_SLOW {
            self.att_mut(fi, id).ty = at::POST_CLASH;
            extra = ad.clash_on_parry_follow_up_windup;
        }
        let ty = self.att(fi, id).ty;
        // angle byte -> [-1, 1], clamped to AnglingLimits (+0xf0); x byte_to_unit (1/255)
        let q = self.qf();
        let a = q(q(clampf(q(net.param1 as f64 * c.byte_to_unit), 0.0, 1.0) * 2.0) - 1.0);
        let lim = ad.angling_limits;
        {
            let am = self.att_mut(fi, id);
            am.mv = enums::lo(net.param0);
            am.angle_target = if a >= lim.xf() { a } else { lim.xf() };
            if a >= lim.yf() {
                am.angle_target = lim.yf();
            }
        }
        // Morph: the attack being morphed out of pays its MorphCost (authority only)
        if ty == at::MORPH {
            if let Some(cost) = last_morph_cost {
                self.offset_stamina(fi, -cost);
            }
        }
        let src = weapon.clone().or_else(|| self.fighters[fi].weapon.clone()).expect("attack without a weapon");
        let mvv = self.att(fi, id).mv;
        let ai = attack_info_for(&src, mvv).clone();
        let (er, ertf) = early_release_settings(ad, ty);
        {
            let am = self.att_mut(fi, id);
            am.ai = ai;
            // _prepare_curves: UAttackMotion::PrepareAnimationData_Implementation rva=0x1637140 (third person)
            am.windup_curve = ad.windup_curve.clone();
            am.release_curve = ad.release_curve.clone();
            if ty == at::COMBO {
                am.windup_curve = ad.combo_windup_curve.clone();
            } else if ty == at::MORPH {
                am.windup_curve = ad.morph_windup_curve.clone();
            } else if ty == at::RIPOSTE {
                am.release_curve = ad.riposte_release_curve.clone();
            }
            am.early_release = er;
            am.early_release_tf = ertf;
            am.windup_anim_offset = ad.windup_animation_start_time_offset;
            am.bounce_montage = ad.bounce_montage.clone();
        }
        self.modify_attack_info(fi, id);
        let start = self.m(fi, id).start_time;
        let am = self.att_mut(fi, id);
        // UAttackMotion::ComputeWindup_Implementation rva=0x1618de0: return AttackInfo.Windup
        am.ai.windup = q(q(am.ai.windup + extra) - q(net.param2 as f64 * c.net_lag_unit_s));
        if let Some(h) = self.net.clone() {
            // game/net: EstimatedNetworkDelay -> LagReduction / LagInduction / MissRecovery (net.rs)
            h.attack_lag(self, fi, id);
        }
        let am = self.att_mut(fi, id);
        am.ai.windup = q(q(am.lag_reduction + am.lag_induction) + am.ai.windup);
        am.windup_end = q(am.ai.windup + start);
        am.release_end = q(am.windup_end + am.ai.release);
        let end = q(am.release_end + am.ai.miss_recovery);
        self.mm(fi, id).end_time = end;
    }

    /// from UAttackMotion::ModifyAttackInfo_Implementation rva=0x162d870 (+ UStabMotion override rva=0x165d830)
    fn modify_attack_info(&mut self, fi: usize, id: MotionId) {
        let def = self.adef(fi, id);
        let ad = def.attack();
        let q = self.qf();
        let ch = self.fighters[fi].character.clone(); // AMordhauCharacter Melee*Modifier
        let am = self.att_mut(fi, id);
        let mut w = q(am.ai.windup * ch.melee_windup_modifier);
        if am.ty == at::COMBO {
            if am.coming_from_move != mv::KICK {
                w = q(q(ch.melee_combo_extra_windup_modifier * am.ai.combo_windup_increase) + w);
                if am.b_is_combo_from_miss {
                    w = q(w + am.ai.miss_combo_extra_windup_increase);
                }
            }
        } else if am.ty == at::MORPH {
            w = q(w * ad.morph_windup_modifier);
        } else if am.ty == at::RIPOSTE {
            w = q(w * ad.riposte_windup_modifier);
        }
        am.ai.windup = w;
        // perk 6 (TurnCaps), vehicle bCanCombo: not modelled (no perks, no vehicles)
        if am.b_is_combo_from_miss {
            am.ai.miss_stamina_cost = q(ad.miss_twice_stamina_cost_multiplier * am.ai.miss_stamina_cost);
        }
        am.ai.release = q(ch.melee_release_modifier * am.ai.release);
        am.ai.miss_recovery = q(ch.melee_miss_recovery_modifier * am.ai.miss_recovery);
        if am.is_stab_class() {
            // Weapon (+0x10c8) +0xf38 StabReleaseModifier (UStabMotion::ModifyAttackInfo_Implementation, byte-matched)
            am.ai.release = q(am.weapon.as_ref().unwrap().stab_release_modifier * am.ai.release);
        }
        if def.kick.is_some() {
            self.modify_kick_attack_info(fi, id);
        }
    }

    /// from UKickMotion::ModifyAttackInfo_Implementation rva=0x165d6e0 (decomp UKickMotion.cpp 111-177), after the base
    fn modify_kick_attack_info(&mut self, fi: usize, id: MotionId) {
        let def = self.adef(fi, id);
        let kd = def.kick.as_ref().unwrap();
        let ch = self.fighters[fi].character.clone();
        let c = self.spec.constants.clone();
        let f = &self.fighters[fi];
        let src = f.left_weapon.clone().or_else(|| f.weapon.clone()).or_else(|| self.att(fi, id).weapon.clone());
        let tier3 = super::damage::armor_tier(f, &c.kick_tier_bone) == 3;
        let (airborne, airborne_time) = (f.airborne, f.airborne_time);
        let q = self.qf();
        let am = self.att_mut(fi, id);
        if let Some(s) = src {
            am.bounce_montage = s.kick_bounce.clone();
        }
        if am.ty == at::COMBO && am.coming_from_move == mv::KICK {
            am.ai.windup = q(am.ai.windup - q(ch.melee_combo_extra_windup_modifier * am.ai.combo_windup_increase));
        }
        am.ai.windup = q(am.ai.windup + c.kick_windup_extra); // 0.3 (.rdata 0x1441231e4)
        if tier3 {
            for d in am.ai.damage.iter_mut() {
                *d = (kd.kick_damage_modifier_tier3_legs * *d as f64) as f32; // PackedFloat32Array store
            }
        }
        if airborne && airborne_time < kd.max_airborne_time_for_jump_kick_anim {
            am.b_is_air_kick = true;
            am.ai.stamina_drain = kd.jump_kick_stamina_drain;
            am.ai.windup = q(kd.jump_kick_extra_windup + am.ai.windup);
        }
    }

    /// from UAttackMotion::OnTick_Implementation rva=0x16328c0 (state/timer part)
    pub(crate) fn attack_on_tick(&mut self, fi: usize, id: MotionId) {
        let def = self.adef(fi, id);
        let ad = def.attack();
        let c = self.spec.constants.clone();
        let t = self.now;
        let q = self.qf();
        let start = self.m(fi, id).start_time;
        if self.att(fi, id).stage == stage::WINDUP {
            let a = self.att(fi, id);
            if a.ty == at::RIPOSTE || a.b_has_chambered {
                self.fighters[fi].easy_parry_until_time = q(t + c.easy_parry_window);
            }
            let a = self.att(fi, id);
            if q(ad.min_windup_time_before_morphing + start) < t && a.b_has_queued_move {
                let (qm, qa) = (a.queued_move, a.queued_angle);
                self.attack_process_attack(fi, id, qm, qa); // UMordhauMotion::ProcessAttack -> virtual dispatch
                if !self.is_current(fi, id) {
                    return;
                }
            }
            let a = self.att_mut(fi, id);
            if a.windup_end < t {
                a.b_has_queued_move = false;
                a.stage = stage::RELEASE;
                a.hit_actors.clear(); // TSet<AActor*>::Empty(weapon + 0xe98)
            }
        }
        let a = self.att(fi, id);
        if a.release_end - a.lag_induction < t && !a.b_has_considered_combo && a.ai.b_can_combo {
            self.att_mut(fi, id).b_has_considered_combo = true;
            let a = self.att(fi, id);
            if a.b_has_queued_move && self.fighters[fi].stamina > 0 && (a.b_has_hit || a.ai.b_can_miss_combo) {
                let combo_type = if a.b_has_hit { at::COMBO } else { at::MISS_COMBO };
                let (qm, qa) = (a.queued_move, a.queued_angle);
                self.assign_net_attack_motion(fi, combo_type, qm, qa);
                if !self.is_current(fi, id) {
                    return;
                }
            }
        }
        if self.att(fi, id).stage == stage::RELEASE {
            self.fighters[fi].easy_parry_until_time = 0.0;
            if self.att(fi, id).release_end < t {
                self.att_mut(fi, id).b_has_queued_move = false;
                self.enter_recovery(fi, id);
                let a = self.att(fi, id);
                if !a.b_has_hit {
                    let cost = trunc_i(-a.ai.miss_stamina_cost);
                    self.offset_stamina(fi, cost);
                }
            }
        }
        let s = self.att(fi, id).stage;
        if s == stage::WINDUP || s == stage::RELEASE {
            self.update_normalized_times(fi, id, t);
        }
    }

    /// Original OnTick RVA16328c0: segmented release, curve, then view-target post-hit power.
    /// The second output is the unaccelerated early fraction's spine alpha; recomputation never compounds power.
    pub fn attack_release_state(&self, fi: usize, id: MotionId, t: f64) -> (f64, f64) {
        let a = self.att(fi, id);
        let q = self.qf();
        let (we, re, er) = (a.windup_end, a.release_end, a.early_release);
        let early_end = q(q(q(q(re - we) * er) * a.early_release_tf) + we);
        let raw_early = if t <= we { Some(0.0) } else if t < early_end && we < early_end {
            let r = q(q(t - we) / q(early_end - we));
            if r < 1.0 { Some(r) } else { None }
        } else { None };
        let (mut r, spine_alpha) = if t < early_end {
            if let Some(raw) = raw_early {
                let c = clampf(raw, 0.0, 1.0);
                (q(er * raw), q(q(c * c) * q(3.0 - q(2.0 * c))))
            } else {
                (q(q(q(1.0 - er) * self.release_late_fraction(early_end, re, t)) + er), 1.0)
            }
        } else {
            (q(q(q(1.0 - er) * self.release_late_fraction(early_end, re, t)) + er), 1.0)
        };
        if !a.release_curve.is_empty() { r = q(self.spec.curve_value(&a.release_curve, r)); }
        let h = a.first_hit_release_norm;
        // Exact original native small-number bits, separate from configurable combat tolerances.
        if a.b_has_hit_including_cosmetic_hit && a.ai.hit_effect_speed_up_exponent > 1.0
            && q(h - 1.0).abs() > f32::from_bits(0x322b_cc77) as f64 && self.f(fi).is_view_target {
            let remaining = q(1.0 - h);
            let base = q(1.0 - q(q(r - h) / remaining));
            let power = if self.exe() { (base as f32).powf(a.ai.hit_effect_speed_up_exponent as f32) as f64 }
                else { base.powf(a.ai.hit_effect_speed_up_exponent) };
            r = q(q(q(1.0 - power) * remaining) + h);
        }
        (r, spine_alpha)
    }

    fn release_late_fraction(&self, early_end: f64, re: f64, t: f64) -> f64 {
        if t >= re { 1.0 } else if t <= early_end { 0.0 } else if early_end < re {
            self.qf()(self.qf()(t - early_end) / self.qf()(re - early_end))
        } else { 1.0 }
    }

    fn update_normalized_times(&mut self, fi: usize, id: MotionId, t: f64) {
        let def = self.adef(fi, id);
        let ad = def.attack();
        let small = self.spec.constants.small_number;
        let ws = self.m(fi, id).start_time; // bIncludeMissingDeltaTime is off offline
        let a = self.att(fi, id);
        let q = self.qf();
        let exe = self.exe();
        let (we, re) = (a.windup_end, a.release_end);
        let mut wn = 1.0;
        if t < we {
            wn = if t <= ws { 0.0 } else if ws < we { q(q(t - ws) / q(we - ws)) } else { 1.0 };
        }
        let (st, offset, wcurve) = (a.stage, a.windup_anim_offset, a.windup_curve.clone());
        let mut lwn = wn;
        let mut lrn = a.last_release_norm;
        if st == stage::WINDUP {
            lwn = if exe { smoothed_windup_exe(ad, ws, we, re, offset, wn, small) } else { smoothed_windup(ad, ws, we, re, offset, wn, small) };
            // OnTick_Implementation rva=0x16328c0: WindUpCurve (+0xdf8) applied in place to LastWindupNormalizedTime
            if !wcurve.is_empty() {
                lwn = q(self.spec.curve_value(&wcurve, lwn));
            }
        } else if st == stage::RELEASE {
            lrn = self.attack_release_state(fi, id, t).0;
        }
        let a = self.att_mut(fi, id);
        a.last_windup_norm = lwn;
        a.last_release_norm = lrn;
    }

    /// from UAttackMotion::EnterRecovery rva=0x161b7b0 (state part; also MaybeStoreLastTraceInGameStateMemory)
    fn enter_recovery(&mut self, fi: usize, id: MotionId) {
        self.maybe_store_last_trace(fi, id);
        let dnm = self.m(fi, id).def.attack().b_do_not_make_recovery_flinchable;
        if !dnm {
            self.mm(fi, id).b_is_flinchable = true;
        }
        self.att_mut(fi, id).stage = stage::RECOVERY;
    }

    /// from UAttackMotion::OnLeave_Implementation rva=0x16323e0 (trace-memory part): leaving during Release stores
    /// the last trace too
    pub(crate) fn attack_on_leave(&mut self, fi: usize, id: MotionId) {
        // OnLeave_Implementation rva=0x16323e0 (decomp 5922-5924): SetTurnCaps(-1, -1)
        self.set_turn_caps(fi, -1.0, -1.0);
        if self.att(fi, id).stage == stage::RELEASE {
            self.maybe_store_last_trace(fi, id);
        }
        if !self.exe() {
            return;
        }
        // exe (decomp UAttackMotion.cpp OnLeave_Implementation, exe.rs differences 11-12), the new motion already current:
        //  - Release, !bHasHit and (the new motion IsA UAttackMotion or !Interrupted; ChangeMotion_Internal's Interrupt
        //    passes true): stamina -(int)MissStaminaCost (a miss-combo out of Release pays the miss)
        //  - Recovery, the new motion a UParryMotion, !bHasHit, bCanCombo and bUseSeamlessCFTPInRecovery: -FeintCost
        //  - DisableBlockCollider (rva=0x1538cc0)
        let new = self.cur_m(fi).map(|m| (m.is_attack(), m.is_parry())).unwrap_or((false, false));
        let a = self.att(fi, id);
        let seamless = self.m(fi, id).def.attack().b_use_seamless_cftp_in_recovery;
        if a.stage == stage::RELEASE && !a.b_has_hit && new.0 {
            let c = trunc_i(-a.ai.miss_stamina_cost);
            self.offset_stamina(fi, c);
        }
        let a = self.att(fi, id);
        if a.stage == stage::RECOVERY && new.1 && !a.b_has_hit && a.ai.b_can_combo && seamless {
            let c = -a.ai.feint_cost;
            self.offset_stamina(fi, c);
        }
        self.fighters[fi].block_collider_enabled = false;
        self.fighters[fi].clash_collider_enabled = false;
    }

    /// from UAttackMotion::MaybeStoreLastTraceInGameStateMemory rva=0x162d740: append FLineTraceMemoryEntry
    /// {LastTraceStart, LastTraceEnd, now + TraceMemoryStayDuration (+0xa58), Owner} to AttackTracesMemory (+0x530)
    fn maybe_store_last_trace(&mut self, fi: usize, id: MotionId) {
        let a = self.att(fi, id);
        if !a.b_has_last_trace {
            return;
        }
        let e = TraceMemory {
            start: a.last_trace_start,
            end: a.last_trace_end,
            destroy_time: self.now + self.m(fi, id).def.attack().trace_memory_stay_duration,
            owner: self.fighters[fi].id,
        };
        self.attack_traces_memory.push(e);
    }

    /// from UAttackMotion::IsInEarlyRelease rva=0x162c110; UStrikeMotion::IsInEarlyRelease rva=0x165d610 adds the
    /// look-up terms (GetRawLookUpValue rva=0x1485240 / LookUpLimit +0x8a0)
    pub fn attack_is_in_early_release(&self, fi: usize, id: MotionId) -> bool {
        let m = self.m(fi, id);
        let a = m.attack().unwrap();
        let ad = m.def.attack();
        let mut er = a.early_release;
        if a.is_strike_class() {
            let f = &self.fighters[fi];
            let q = self.qf();
            let limit = f.character.look_up_limit;
            let look_up = q(maxf(q(f.look_up_value), 0.0) / limit);
            let reversed = -a.angle_target;
            let non_undercut_term = q(q(q(minf(reversed, 0.0) + 1.0) * look_up) * ad.extra_early_release_for_look_up_non_undercuts);
            let overhead_term = q(q(maxf(reversed, 0.0) * look_up) * ad.extra_early_release_for_look_up_overheads);
            er = q(q(non_undercut_term + overhead_term) + er);
        }
        a.stage == stage::RELEASE && a.last_release_norm < er
    }

    /// from UAttackMotion::CanMorphInto rva=0x1616820 (class != own class); UStrikeMotion::CanMorphInto rva=0x164ec90
    /// refuses any UStrikeMotion class; UStabMotion::CanMorphInto rva=0x164ebe0 refuses UStabMotion classes
    fn can_morph_into(&self, fi: usize, id: MotionId, target: &MotionDef) -> bool {
        let m = self.m(fi, id);
        let a = m.attack().unwrap();
        let tn = &target.base.native;
        if a.is_strike_class() && tn == "UStrikeMotion" {
            return false;
        }
        if a.is_stab_class() && tn == "UStabMotion" {
            return false;
        }
        target.base.path != m.bp
    }

    /// from UAttackMotion::ProcessAttack_Implementation rva=0x1637cf0
    pub(crate) fn attack_process_attack(&mut self, fi: usize, id: MotionId, m: i64, angle: f64) -> bool {
        let def = self.adef(fi, id);
        let ad = def.attack();
        let t = self.now;
        let start = self.m(fi, id).start_time;
        let end = self.m(fi, id).end_time;
        let a = self.att(fi, id);
        if a.stage == stage::RELEASE && a.ai.b_can_combo {
            let (ok, cm) = a.convert_to_combo(m);
            if !ok {
                return false;
            }
            if !self.can_perform_attack(fi, cm) {
                return false;
            }
            if self.att(fi, id).b_has_queued_move {
                return false;
            }
            return self.att_mut(fi, id).queue(cm, angle);
        }
        if a.stage == stage::WINDUP {
            let req = self.attack_motion_defaults(fi, m);
            let auto_feint = req.attack().b_can_auto_feint_to_attack; // CDO of the requested class
            let (mm, ma) = a.modify_requested_morph_attack(m, angle, self.spec.constants.morph_stab_angle_scale);
            let tgt = self.attack_motion_defaults(fi, mm);
            let mut morph_ok = false;
            let mw = ad.morph_window;
            let q = self.qf();
            if mw != 0.0 {
                let lim = minf(q(ad.max_morph_total_time - a.lag_induction), q(q(q(a.windup_end - start) - mw) - a.lag_induction));
                morph_ok = t < q(lim + start);
            }
            if a.ty == at::REGULAR && self.can_morph_into(fi, id, &tgt) && morph_ok {
                if self.fighters[fi].stamina < a.ai.morph_cost {
                    return false;
                }
                if q(ad.min_windup_time_before_morphing + start) < t {
                    if mm == mv::KICK {
                        let f = &mut self.fighters[fi];
                        f.next_kick_time = maxf(q(t + ad.morph_kick_extra_time), f.next_kick_time);
                    }
                    self.assign_net_attack_motion(fi, at::MORPH, mm, ma);
                    return !self.is_current(fi, id);
                }
                return self.att_mut(fi, id).queue(mm, ma);
            }
            // not a morph: feint into the attack if the requested class allows it (bCanAutoFeintToAttack)
            if !auto_feint || !self.can_morph_into(fi, id, &req) || !self.attack_process_feint(fi, id) {
                return false;
            }
            if let Some(c) = self.fighters[fi].motion {
                if self.m(fi, c).feinted().is_some() {
                    self.feinted_process_attack(fi, c, m, angle);
                }
            }
            return false;
        }
        if t < end - ad.recovery_queue_window {
            return false;
        }
        self.att_mut(fi, id).queue(m, angle)
    }

    /// from UAttackMotion::ProcessFeint_Implementation rva=0x1638220; UKickMotion::ProcessFeint_Implementation
    /// rva=0x166bd20 adds: an air kick (bIsAirKick +0x1114) cannot be feinted
    pub(crate) fn attack_process_feint(&mut self, fi: usize, id: MotionId) -> bool {
        let def = self.adef(fi, id);
        let ad = def.attack();
        let a = self.att(fi, id);
        if a.native == "UKickMotion" && a.b_is_air_kick {
            return false;
        }
        if a.ty == at::RIPOSTE {
            if !a.b_riposte_ate_feint_input {
                self.att_mut(fi, id).b_riposte_ate_feint_input = true;
                if !ad.b_is_riposte_feintable {
                    return true;
                }
            } else if !ad.b_is_riposte_feintable {
                return false;
            }
        }
        let t = self.now;
        let a = self.att(fi, id);
        let cost = if a.b_has_chambered { a.ai.chamber_feint_cost } else { a.ai.feint_cost };
        let can_pay = cost <= self.fighters[fi].stamina;
        let q = self.qf();
        let in_window = a.stage == stage::WINDUP && !(q(q(a.windup_end - a.lag_induction) - ad.feint_window) < t);
        if can_pay && in_window {
            let mut f = ft::REGULAR;
            if a.ty == at::COMBO {
                f = ft::COMBO;
            } else if a.b_has_chambered {
                f = ft::CHAMBER;
            }
            let m = a.mv;
            self.assign_net_feint(fi, f, m);
            return true;
        }
        self.att_mut(fi, id).b_has_queued_move = false;
        false
    }

    /// from UAttackMotion::ProcessBlock_Implementation rva=0x16380a0
    pub(crate) fn attack_process_block(&mut self, fi: usize, id: MotionId, b: i64) -> bool {
        let def = self.adef(fi, id);
        let ad = def.attack();
        let m = self.m(fi, id);
        let a = m.attack().unwrap();
        if a.weapon.is_none() {
            return false;
        }
        let t = self.now;
        if !a.b_has_hit {
            let riposte_parry = a.b_riposte_ate_feint_input
                && a.ty == at::RIPOSTE
                && a.stage == stage::WINDUP
                && !(self.qf()(self.qf()(a.windup_end - a.lag_induction) - ad.riposte_windup_can_parry_window) < t);
            if !riposte_parry {
                if a.stage == stage::RECOVERY && ad.b_use_seamless_cftp_in_recovery && a.ai.b_can_combo {
                    if self.fighters[fi].stamina < a.ai.feint_cost {
                        return false;
                    }
                } else {
                    // PostClash windup parry (decomp UAttackMotion.cpp 7518-7539): ComingFromMotion IsA UBlockedMotion
                    // whose FBlockResult.bClashOnParry (+0xb0) is set
                    if a.ty != at::POST_CLASH || a.stage != stage::WINDUP {
                        return false;
                    }
                    if self.qf()(self.qf()(a.windup_end - a.lag_induction) - ad.clash_on_parry_can_parry_window) <= t {
                        return false;
                    }
                    let clash = m.coming_from.and_then(|c| self.m(fi, c).blocked()).map(|b| b.b_clash_on_parry).unwrap_or(false);
                    if !clash {
                        return false;
                    }
                }
            }
        } else if a.stage == stage::RELEASE && !ad.b_can_block_from_release_after_hit {
            return false;
        }
        let b2 = a.modify_requested_block_type(b);
        if !m.b_can_block {
            return false;
        }
        self.assign_net_parry(fi, b2);
        true
    }

    /// from UAttackMotion::OnFeinted rva=0x16312d0 (stamina part)
    pub(crate) fn attack_on_feinted(&mut self, fi: usize, id: MotionId) {
        let a = self.att(fi, id);
        let cost = if a.b_has_chambered { a.ai.chamber_feint_cost } else { a.ai.feint_cost };
        if cost != 0 {
            self.offset_stamina(fi, -cost);
        }
    }

    /// from UAttackMotion::OnDynamicParamChanged_Implementation rva=0x1631060: bit 4 drops
    /// MissComboExtraWindupIncrease from a miss-combo whose parent hit late; bit 1: hit -> EndTime = ReleaseEnd +
    /// HitRecovery (owning client: - LagInduction, clamped at 0); bit 2: chambered
    pub(crate) fn attack_on_dynamic_param_changed(&mut self, fi: usize, id: MotionId, nv: i64) {
        let t = self.now;
        let hit_recovery = self.m(fi, id).def.attack().hit_recovery;
        let role = self.fighters[fi].role;
        let mut end = self.m(fi, id).end_time;
        let q = self.qf();
        let a = self.att_mut(fi, id);
        if (nv & enums::DYN_PARENT_HIT_LATE) != 0 && a.b_is_combo_from_miss {
            a.b_is_combo_from_miss = false;
            if a.stage == stage::WINDUP {
                let x = a.ai.miss_combo_extra_windup_increase;
                a.ai.windup = q(a.ai.windup - x);
                a.windup_end = q(a.windup_end - x);
                end = q(end - x);
                a.release_end = q(a.release_end - x);
            }
        }
        if (nv & enums::DYN_HIT) != 0 && !a.b_has_hit {
            // OnDynamicParamChanged_Implementation rva=0x1631060 (decomp UAttackMotion.cpp 7361-7364)
            a.set_has_hit_including_cosmetic_hit();
            end = a.release_end;
            a.first_hit_time = t;
            let rec = if role != 2 { hit_recovery } else { maxf(q(hit_recovery - a.lag_induction), 0.0) };
            end = q(rec + end);
        }
        a.b_has_chambered = (nv & enums::DYN_CHAMBERED) != 0;
        a.b_has_hit = (nv & enums::DYN_HIT) != 0;
        self.mm(fi, id).end_time = end;
    }

    /// from UAttackMotion::OnEnded_Implementation rva=0x1631260: a move queued in recovery after a hit starts a
    /// Regular attack
    pub(crate) fn attack_on_ended(&mut self, fi: usize, id: MotionId) {
        let a = self.att(fi, id);
        if a.b_has_queued_move && a.b_has_hit {
            let (qm, qa) = (a.queued_move, a.queued_angle);
            self.assign_net_attack_motion(fi, at::REGULAR, qm, qa);
            if !self.is_current(fi, id) {
                return;
            }
        }
        self.base_on_ended(fi, id);
    }

    /// from UAttackMotion::CheckChamberIsValidIgnoreTiming rva=0x1617740 (this = the would-be chamberer `fi`'s attack
    /// `id`, other = the attacking fighter): !bCannotChamber (+0xb8d); the other's motion an attack; the game mode's
    /// CanChamber (AMordhauGameMode::CanChamber_Implementation rva=0x1586d60: !IsFriendly); Regular, or a Morph judged
    /// by its morphed-from attack; both stabs valid; else mirrored strikes with |angle diff| <
    /// ToChamberAttackAngleTolerance (+0xa80)
    pub fn chamber_valid_ignore_timing(&self, fi: usize, id: MotionId, other: usize) -> bool {
        if self.fighters[fi].character.b_cannot_chamber {
            return false;
        }
        let Some(om) = self.cur_m(other).and_then(|m| m.attack()) else { return false };
        if self.mode_rules.is_some() && self.is_friendly(Some(fi), Some(other)) {
            return false;
        }
        let m = self.m(fi, id);
        let a = m.attack().unwrap();
        let (mut mvv, mut ang) = (a.mv, a.angle_target);
        if a.ty == at::MORPH && a.previous_last_attack.is_some() {
            let p = self.att(fi, a.previous_last_attack.unwrap());
            mvv = p.mv;
            ang = p.angle_target;
        } else if a.ty != at::REGULAR && a.ty != at::MORPH {
            return false;
        }
        if enums::is_stab(mvv) && enums::is_stab(om.mv) {
            return true;
        }
        if mvv == mv::LEFT_STRIKE {
            if om.mv != mv::RIGHT_STRIKE {
                return false;
            }
        } else if mvv == mv::RIGHT_STRIKE {
            if om.mv != mv::LEFT_STRIKE {
                return false;
            }
        } else {
            return false;
        }
        self.qf()(ang - om.angle_target).abs() < m.def.attack().to_chamber_attack_angle_tolerance
    }

    /// The non-geometry gates of UAttackMotion::CheckChamber rva=0x1617140 (decomp 4730-5070): valid ignoring timing,
    /// Windup and now - StartTime < ChamberWindow (a Morph: those of its morphed-from attack), then the masks: the
    /// other attack's Weapon AttackMask (+0x19a8) & this attack's Weapon ParryMask (+0x19ac) != 0. Geometry = contact.
    pub fn chamber_gate(&self, fi: usize, id: MotionId, other: usize) -> bool {
        if !self.chamber_valid_ignore_timing(fi, id, other) {
            return false;
        }
        let m = self.m(fi, id);
        let a = m.attack().unwrap();
        let mut s = m.start_time;
        let mut cw = m.def.attack().chamber_window;
        if a.ty == at::MORPH {
            if let Some(p) = a.previous_last_attack {
                s = self.m(fi, p).start_time;
                cw = self.m(fi, p).def.attack().chamber_window;
            }
        }
        if !(a.stage == stage::WINDUP && self.qf()(self.now - s) < cw) {
            return false;
        }
        let ow = self.cur_m(other).and_then(|m| m.attack()).and_then(|a| a.weapon.clone());
        let (Some(ow), Some(w)) = (ow, a.weapon.as_ref()) else { return false };
        (ow.attack_mask & w.parry_mask) != 0
    }

    /// used by the blocked port: the attack's `ad` and fields
    pub(crate) fn _unused_br(&self) -> i64 {
        br::PARRY
    }
}

//! What a released attack does to the characters its weapon trace touches (godot/game/combat/melee_hit.gd).
//! Ports (extract/native/decomp/UAttackMotion.cpp unless noted):
//!   ExecuteAttackTracingAndLogic rva=0x161c390 + AMordhauWeapon::SampleTracers rva=0x163c430 / SampleTracer
//!     rva=0x163be10 (one query for all segments; hits nearest-first across actors; parrying hands skipped);
//!   ProcessHitForBlocking rva=0x1638380 (parry, then the victim's attack: chamber; else damage);
//!   ProcessHitForDamage rva=0x1638a60; CheckChamber rva=0x1617140; HandleWasParried rva=0x162b4d0;
//!   GetFastestAttackWindup rva=0x1622f40. UNCONFIRMED: the collision response of trace channel 0xf and the engine's
//!   multi-trace distance sort.

use super::damage;
use super::enums::{self, at, br, mv, ps, stage};
use super::motion::MotionId;
use super::world::{HitComp, RawHit, World, WorldBlock};
use crate::ue::{trunc_i, FVector, Xform};
use serde_json::json;

/// Segment [a, b] against an oriented box: entry fraction in [0, 1], or -1 (slab test in box space)
pub fn segment_box(a: FVector, b: FVector, box_xf: &Xform, half: FVector) -> f64 {
    let inv = box_xf.affine_inverse();
    let p = inv.apply(a);
    let d = inv.apply(b) - p;
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    for k in 0..3 {
        let (dk, pk, hk) = (d.get(k) as f64, p.get(k) as f64, half.get(k) as f64);
        if dk.abs() < 1e-12 {
            if pk.abs() > hk {
                return -1.0;
            }
            continue;
        }
        let mut ta = (-hk - pk) / dk;
        let mut tb = (hk - pk) / dk;
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = crate::ue::maxf(t0, ta);
        t1 = crate::ue::minf(t1, tb);
        if t0 > t1 {
            return -1.0;
        }
    }
    t0
}

struct CacheHit {
    victim: usize,
    comp: HitComp,
    trace_start: FVector,
    trace_end: FVector,
    t: f64,
    /// FHitResult bBlockingHit as SampleTracer sets it (the world was hit by this tracer or an earlier one)
    blocking: bool,
}

impl World {
    /// All body boxes of `victim` crossed by [a, b]: (t, bone), nearest first. Bones without a transform are skipped.
    fn trace_bodies(&self, a: FVector, b: FVector, victim: usize) -> Vec<(f64, String)> {
        let v = &self.fighters[victim];
        let mut hits = Vec::new();
        for bx in &self.spec.body_boxes {
            let Some(bxf) = v.bone_xf.get(&bx.bone) else { continue };
            let t = segment_box(a, b, &bxf.mul(&bx.xf), bx.half);
            if t >= 0.0 {
                hits.push((t, bx.bone.clone()));
            }
        }
        hits.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
        hits
    }

    /// from UAttackMotion::ExecuteAttackTracingAndLogic rva=0x161c390 (character hits only)
    pub(crate) fn trace_and_process(&mut self, fi: usize) {
        let Some(atk) = self.fighters[fi].motion else { return };
        if self.m(fi, atk).attack().map(|a| a.stage) != Some(stage::RELEASE) {
            return;
        }
        // a TraceHost (mh-sim) supplies the swept segments' raw hits and the world's BlockingHit; else the reference
        // tracer's segments
        let host = self.trace_host.clone();
        let (host_hits, cs, ce, blocking, last_dir) = match &host {
            Some(h) => {
                let s = h.sample_ex(self, fi);
                (Some(s.segments), s.cur_start, s.cur_end, s.blocking, s.last_dir)
            }
            None => (None, self.fighters[fi].tracer.cur_start, self.fighters[fi].tracer.cur_end, None, FVector::ZERO),
        };
        let segs = if host_hits.is_some() { Vec::new() } else { self.fighters[fi].tracer.segments() };
        {
            let a = self.att_mut(fi, atk);
            a.global_damage_modifier = 1.0;
            // after SampleTracers, Stage == Release: LastTraceStart/End = CurrentTraceStart/End, bHasLastTrace = 1
            a.last_trace_start = cs;
            a.last_trace_end = ce;
            a.b_has_last_trace = true;
        }
        // the world (decomp UAttackMotion.cpp 6137-6174): a BlockingHit with an actor, Stage == Release (the gate
        // above) and !bDisableWorldCollision (+0xa46) -> HandleBlockingHit when the impact, projected on
        // LastTraceStart..LastTraceEnd, lies closer to LastTraceStart (the hilt side) than the trigger distance
        let world_on = !self.m(fi, atk).def.attack().b_disable_world_collision;
        if let Some((_, wb)) = blocking.as_ref().filter(|(_, wb)| world_on && wb.actor) {
            if self.world_collision_triggers(fi, atk, wb.impact_point) {
                let wb = wb.clone();
                self.handle_blocking_hit(fi, atk, &wb, last_dir);
                return;
            }
        }
        let mut ignored = self.fighters[fi].tracer.actor_ignore_cache.clone();
        let mut cache: Vec<CacheHit> = Vec::new();
        for (a, b) in segs {
            let raw = self.raw_segment_hits(fi, a, b);
            self.sample_tracer(fi, raw, &mut ignored, &mut cache, false);
        }
        for (k, raw) in host_hits.unwrap_or_default().into_iter().enumerate() {
            // SampleTracer 0x14163c2d0: every hit result's bBlockingHit = whether the BlockingHit was found by this
            // tracer or an earlier one
            let flagged = blocking.as_ref().is_some_and(|(i, _)| k >= *i);
            self.sample_tracer(fi, raw, &mut ignored, &mut cache, flagged);
        }
        for h in cache {
            let geo = self.exe() && self.fighters[fi].geom.is_some() && self.fighters[h.victim].geom.is_some();
            let r = if geo {
                let ev0 = self.hits.len();
                // ProcessHitForBlocking's world branch reads the hit's bBlockingHit and the BlockingHit
                let world = if h.blocking && world_on { blocking.as_ref().map(|(_, wb)| wb.clone()).filter(|wb| wb.actor) } else { None };
                let r = self.process_hit_geo(fi, h.victim, &h.comp, h.trace_start, h.trace_end, world.as_ref(), last_dir);
                // the sweep's hit point (FHitResult ImpactPoint, approximated as Start + t (End - Start) of the
                // swept segment; UNCONFIRMED: the shape-surface point): OnTookDamage rva=0x14923c0 passes it to
                // AMordhauWeapon::OnHit_Implementation rva=0x1631430 as the sound / blood location
                let (ts, te, t) = (h.trace_start, h.trace_end, h.t as f32);
                let p = [ts.x + (te.x - ts.x) * t, ts.y + (te.y - ts.y) * t, ts.z + (te.z - ts.z) * t];
                for e in self.hits[ev0..].iter_mut() {
                    let k = e.get("kind").and_then(|k| k.as_str()).unwrap_or("");
                    if matches!(k, "hit" | "parry" | "active_parry" | "chamber" | "clash") && !e.contains_key("impact") {
                        e.insert("impact".into(), json!(p));
                    }
                }
                r
            } else {
                let bone = if let HitComp::Body(b) = &h.comp { b.clone() } else { continue };
                self.process_hit(fi, h.victim, &bone)
            };
            if !self.is_current(fi, atk) || r {
                return;
            }
        }
    }

    /// One AMordhauWeapon::SampleTracer rva=0x163be10 segment: a hit on NAME_LeftHand (0x145723678) / NAME_RightHand
    /// (0x145723650) of a character parrying in Stage Parry is dropped (disasm 0x14163c20e..0x14163c25c)
    fn raw_segment_hits(&self, fi: usize, a: FVector, b: FVector) -> Vec<RawHit> {
        let mut single = Vec::new();
        for vi in 0..self.fighters.len() {
            if vi == fi {
                continue;
            }
            for (t, bone) in self.trace_bodies(a, b, vi) {
                single.push(RawHit { t, victim: vi, comp: HitComp::Body(bone), trace_start: a, trace_end: b });
            }
        }
        single
    }

    /// The SampleTracer rva=0x163be10 rules over one segment's raw hits: the owner, dead pawns and ignored actors (the
    /// shared query's AddIgnoredActors) are skipped; hits are taken nearest first (stable: equal distances keep the
    /// order given). Each kept hit adds its actor to the ignored actors unless the component is the character's
    /// BlockCollider (+0xf60) or the weapon's BlockCollider (+0x1a58) (decomp AMordhauWeapon.cpp 2493-2500). The
    /// weapons are actors of their own: a ClashCollider (+0x1a50) hit ignores the weapon actor (keyed `id | 1 << 30`),
    /// a shield BlockCollider hit ignores nothing, and neither is dropped because the owner's body was hit.
    fn sample_tracer(&self, fi: usize, raw: Vec<RawHit>, ignored: &mut Vec<u32>, cache: &mut Vec<CacheHit>, blocking: bool) {
        const WEAPON_ACTOR: u32 = 0x4000_0000;
        let key = |h: &RawHit| -> Option<u32> {
            let vid = self.fighters[h.victim].id;
            match h.comp {
                HitComp::Body(_) | HitComp::BlockCollider => Some(vid),
                HitComp::Clash => Some(vid | WEAPON_ACTOR),
                HitComp::ShieldBlock => None,
            }
        };
        let mut single: Vec<RawHit> = raw
            .into_iter()
            .filter(|h| h.victim != fi && !self.fighters[h.victim].dead && key(h).is_none_or(|k| !ignored.contains(&k)))
            .collect();
        single.sort_by(|x, y| x.t.partial_cmp(&y.t).unwrap());
        for h in single {
            if key(&h).is_some_and(|k| ignored.contains(&k)) {
                continue;
            }
            match &h.comp {
                HitComp::Body(bone) => {
                    let parrying = self.cur_m(h.victim).and_then(|m| m.parry()).map(|p| p.stage == ps::PARRY).unwrap_or(false);
                    if (bone == "LeftHand" || bone == "RightHand") && parrying {
                        continue;
                    }
                    ignored.push(self.fighters[h.victim].id);
                }
                HitComp::BlockCollider | HitComp::ShieldBlock => {}
                HitComp::Clash => ignored.push(self.fighters[h.victim].id | WEAPON_ACTOR),
            }
            cache.push(CacheHit { victim: h.victim, comp: h.comp, trace_start: h.trace_start, trace_end: h.trace_end, t: h.t, blocking });
        }
    }

    /// ProcessHitForBlocking then ProcessHitForDamage for one hit. true = stop tracing.
    pub(crate) fn process_hit(&mut self, a: usize, v: usize, bone: &str) -> bool {
        let Some(atk) = self.fighters[a].motion else { return true };
        let Some(am) = self.m(a, atk).attack() else { return true };
        if am.hit_actors.contains(&self.fighters[v].id) {
            // weapon ActorSetCache (+0xe98): one hit per actor per release
            return false;
        }
        if !self.process_hit_for_blocking(a, v, atk) {
            return !self.is_current(a, atk);
        }
        self.process_damage(a, v, atk, bone)
    }

    /// from UAttackMotion::ProcessHitForBlocking rva=0x1638380 (decomp 4034-4403; character victims, body boxes):
    /// 1. the victim's LastParryMotion when the attack bCanBeParriedInEarlyRelease (+0xa6c) or is not in early release:
    ///    CheckParry -> (bDrainAllStamOnBlock -100, 0xffffff9c at 0x14164f2cf) ReceiveBlock -> HandleWasParried; chip
    ///    damage unless in live recovery; stop.
    /// 2. the victim's current motion an attack and this attack not in live recovery: CheckAttackParry (body hits never
    ///    qualify), CheckChamber -> chip damage; stop. (CheckClash rva=0x16179a0 needs clash colliders: not ported.)
    /// 3. otherwise damage. Returns true = go on to ProcessHitForDamage.
    fn process_hit_for_blocking(&mut self, a: usize, v: usize, atk: MotionId) -> bool {
        let live_recovery = self.att(a, atk).is_in_live_recovery();
        if let Some(pm) = self.fighters[v].last_parry_motion {
            let early_ok = self.m(a, atk).def.attack().b_can_be_parried_in_early_release || !self.attack_is_in_early_release(a, atk);
            if early_ok && self.check_parry(v, pm, a) {
                let ai = &self.att(a, atk).ai;
                let mut drain = trunc_i(ai.stamina_drain) as f64;
                let (extra, drain_all, amv) = (ai.extra_stamina_drain_vs_held_block, ai.b_drain_all_stam_on_block, self.att(a, atk).mv);
                if self.m(v, pm).parry().unwrap().b_is_block_holdable {
                    drain += extra;
                }
                if drain_all {
                    self.offset_stamina(v, -100);
                }
                self.receive_block(v, pm, drain, amv, Some(a));
                let (vn, an) = (self.fighters[v].name.clone(), self.fighters[a].name.clone());
                self.trace_event(&format!("{vn} parried {an}"));
                let ev_i = self.hits.len();
                self.emit_event(json!({"kind": "parry", "attacker": an, "victim": vn, "move": amv}));
                self.handle_was_parried(a, v, atk, Some(pm));
                self.tag_disarm(ev_i, a);
                if !live_recovery {
                    self.chip_damage(a, v, atk);
                }
                return false;
            }
        }
        let Some(dm) = self.fighters[v].motion else { return true };
        if !self.m(v, dm).is_attack() || live_recovery {
            return true;
        }
        if self.chamber_gate(v, dm, a) {
            self.check_chamber(a, v, atk, dm);
            self.chip_damage(a, v, atk);
            return false;
        }
        true
    }

    /// Chip damage of ProcessHitForBlocking (decomp 4245-4275): ChipDamagePercentageOnBlock > 0 -> ComputeMeleeDamage
    /// on NAME_Spine1 (global 0x145723618) x the percentage, through TakeDamage (vcall +0x590)
    fn chip_damage(&mut self, a: usize, v: usize, atk: MotionId) {
        let q = self.qf();
        let ai = self.att(a, atk).ai.clone();
        if ai.chip_damage_percentage_on_block > 0.0 {
            let d = q(damage::compute(&self.spec.constants, &self.fighters[v], &ai, "Spine1", q) * ai.chip_damage_percentage_on_block);
            self.take_damage(v, d, Some(a), super::system::DAMAGE_MELEE);
        }
    }

    /// The stamina-break branch of UAttackMotion::CheckChamber rva=0x1617140 (decomp `GetStatValue == 0 && 0 < cost`)
    fn chamber_stamina_break(&mut self, a: usize, v: usize, atk: MotionId, dm: MotionId) {
        let amv = self.att(a, atk).mv;
        let direction = if amv == mv::LEFT_STRIKE { -90.0 } else { 90.0 };
        let dm_drop = self.att(v, dm).weapon.as_ref().map(|w| w.b_allow_drop).unwrap_or(false);
        let mut stun = false;
        let mut disarm = false;
        if !dm_drop || self.fighters[v].character.b_destroy_equipment_on_death || self.fighters[a].character.b_always_stun_instead_of_disarm {
            stun = true;
            disarm = false;
        }
        let recover = self.m(v, dm).def.attack().chamber_stamina_recover;
        self.offset_stamina(v, recover);
        if !stun {
            disarm = true;
            self.assign_net_disarmed(v, direction);
        } else {
            self.assign_net_stunned(v, direction, 0, disarm);
        }
        self.offset_stamina(a, recover);
        self.assign_net_blocked(a, br::CLASH, stun as i64 | if disarm { 2 } else { 0 }, 0.0);
        let (vn, an) = (self.fighters[v].name.clone(), self.fighters[a].name.clone());
        self.trace_event(&format!("{vn} chamber stamina break: {}", if stun { "stun" } else { "disarm" }));
        self.emit_event(json!({"kind": "stamina_break", "who": vn, "by": an, "stun": stun, "disarm": disarm}));
    }

    /// from UAttackMotion::HandleWasParried rva=0x162b4d0 / PutUsInBlockedMotionFromParry rva=0x163a7d0
    fn handle_was_parried(&mut self, a: usize, p: usize, atk: MotionId, pm: Option<MotionId>) {
        let pmot = self.cur_m(p).unwrap();
        let mut disarmed = pmot.kind() == "Disarmed";
        let stunned = pmot.kind() == "Stun";
        if stunned {
            if let super::motion::MotionKind::Stun(s) = &pmot.k {
                if s.b_will_disarm {
                    disarmed = true;
                }
            }
        }
        if disarmed || stunned {
            self.assign_net_blocked(a, br::CLASH, stunned as i64 | if disarmed { 2 } else { 0 }, 0.0);
            if let Some(pm) = pm {
                let r = self.m(p, pm).def.parry().block_stamina_recover;
                self.offset_stamina(a, r);
            }
            return;
        }
        if self.att(a, atk).ai.b_will_clash_when_parried {
            // FBlockResult {reason 3, byte 7 = 1} (CheckChamber 0x141617649..0x141617671) -> Param1 bit 0x40
            // (FNetMotion::Blocked rva=0x1615f50, byte 7 -> or al, 0x40 at 0x141615fe1)
            self.assign_net_blocked(a, br::CLASH, 64, 0.0);
            return;
        }
        let w = self.fastest_attack_windup(p);
        self.assign_net_blocked(a, br::PARRY, 0, w);
    }

    /// from UAttackMotion::GetFastestAttackWindup rva=0x1622f40: min(StabAttack.Windup x RiposteWindupModifier of the
    /// Stab motion class, StrikeAttack.Windup x that of the RightStrike class); no weapon -> 0.4 (.rdata 0x1441231f0)
    pub fn fastest_attack_windup(&self, c: usize) -> f64 {
        let q = self.qf();
        let Some(w) = &self.fighters[c].weapon else { return self.spec.constants.no_weapon_fastest_windup };
        let stab = q(w.stab.windup * self.attack_motion_defaults(c, mv::STAB).attack().riposte_windup_modifier);
        let strike = q(w.strike.windup * self.attack_motion_defaults(c, mv::RIGHT_STRIKE).attack().riposte_windup_modifier);
        crate::ue::minf(strike, stab)
    }

    /// from UAttackMotion::CheckChamber rva=0x1617140 after its gates: bDrainAllStamOnBlock costs the chamberer 100;
    /// it pays ChamberCost unless already chambered (stamina 0 -> stamina break); dynamic bit Chambered; the attacker
    /// goes to Blocked(Clash, bClashOnParry) if bWillClashWhenParried, else Blocked(Chamber, chamberer WindupEnd - now)
    fn check_chamber(&mut self, a: usize, v: usize, atk: MotionId, dm: MotionId) {
        let q = self.qf();
        if self.att(a, atk).ai.b_drain_all_stam_on_block {
            self.offset_stamina(v, -100);
        }
        let d = self.att(v, dm);
        let cost = if d.b_has_chambered { 0 } else { d.ai.chamber_cost };
        self.offset_stamina(v, -cost);
        if self.fighters[v].stamina == 0 && 0 < cost {
            self.chamber_stamina_break(a, v, atk, dm);
            return;
        }
        let dy = self.fighters[v].net.dynamic_param;
        self.assign_net_motion_dynamic_param(v, dy | enums::DYN_CHAMBERED);
        let d = self.att(v, dm);
        let mut we = d.windup_end;
        if d.ty == enums::at::MORPH {
            if let Some(p) = d.previous_last_attack {
                we = self.att(v, p).windup_end;
            }
        }
        if self.att(a, atk).ai.b_will_clash_when_parried {
            self.assign_net_blocked(a, br::CLASH, 64, 0.0); // FNetMotion::Blocked rva=0x1615f50 (bit 0x40)
        } else {
            let t = q(we - self.now);
            self.assign_net_blocked(a, br::CHAMBER, 0, t);
        }
        let (vn, an) = (self.fighters[v].name.clone(), self.fighters[a].name.clone());
        self.trace_event(&format!("{vn} chambered {an}"));
        let ev_i = self.hits.len();
        self.emit_event(json!({"kind": "chamber", "attacker": an, "victim": vn}));
        self.tag_disarm(ev_i, a);
    }

    /// from UAttackMotion::ProcessHitForDamage rva=0x1638a60 (character victim; CanContinueTracingAfterDealingDamage
    /// rva=0x16164f0 via vcall +0x398 at 0x141639b94; friendly kept in [rbp-0x60] at 0x141639263; bHasHitFriendly
    /// 0x1416392ae..0x1416392c5). Returns true (stop tracing).
    fn process_damage(&mut self, a: usize, v: usize, atk: MotionId, bone: &str) -> bool {
        let q = self.qf();
        let am = self.m(a, atk);
        let ad = am.def.clone();
        let ad = ad.attack();
        let at_ = am.attack().unwrap();
        if enums::is_strike(at_.mv) && q(1.0 - ad.strike_animation_normalized_recovery_offset) <= at_.last_release_norm {
            // exe: ProcessHitForDamage's early outs return true = continue tracing (ExecuteAttackTracingAndLogic
            // rva=0x161c390 stops only on false); the reference stops (exe.rs difference 10). UNCONFIRMED: where this
            // strike-recovery test sits inside the 7004-byte body
            return !self.exe();
        }
        let vid = self.fighters[v].id;
        self.att_mut(a, atk).hit_actors.push(vid);
        self.fighters[a].tracer.actor_ignore_cache.push(vid);
        let mut factor = self.att(a, atk).global_damage_modifier;
        // the mounted speed factor replaces GlobalDamageModifier (horse.rs mounted_damage_factor)
        if let Some(k) = self.mounted_damage_factor(a, v, bone) {
            factor = k;
        }
        let victim_flinchable = self.cur_m(v).map(|m| m.b_is_flinchable).unwrap_or(false);
        if let Some(vm) = self.cur_m(v) {
            if let Some(va) = vm.attack() {
                if va.ty == enums::at::RIPOSTE && va.stage != stage::RECOVERY {
                    let from_wall = vm.coming_from.and_then(|c| self.m(v, c).parry()).map(|p| p.b_is_shield_wall).unwrap_or(false);
                    if !from_wall {
                        factor = q(factor * vm.def.attack().riposte_trade_damage_factor);
                    }
                }
            }
        }
        let friendly = self.is_friendly(Some(a), Some(v));
        if self.att(a, atk).b_has_hit_friendly {
            factor = q(factor * ad.post_friendly_hit_modifier);
        }
        let am = self.att_mut(a, atk);
        am.b_has_hit_friendly = am.b_has_hit_friendly || friendly;
        let ai = am.ai.clone();
        let dmg = q(damage::compute(&self.spec.constants, &self.fighters[v], &ai, bone, q) * factor);
        if ai.stamina_damage != 0.0 {
            self.offset_stamina(v, -trunc_i(ai.stamina_damage));
        }
        // Presentation identity must be captured before damage/death callbacks alter hands.
        // UAttackMotion::Weapon is the captured actor, not whichever weapon is held at drain time.
        let blood_weapon = self.att(a, atk).weapon_actor.and_then(|id| self.equipment_actor(id)
            .map(|actor| (id, actor.path.clone(), actor.alternate_mode)));
        let applied = self.take_damage(v, dmg, Some(a), super::system::DAMAGE_MELEE);
        let (an, vn) = (self.fighters[a].name.clone(), self.fighters[v].name.clone());
        self.trace_event(&format!("{an} hit {vn} {bone} {dmg:.2}"));
        let health = self.fighters[v].health;
        // the hit-sound / effect inputs of AMordhauWeapon::OnHit_Implementation rva=0x1631430 (decomp
        // AMordhauWeapon.cpp 560-760): Move (StabHitSound for Stab / AltStab, else StrikeHitSound), the victim's armour
        // tier at the bone (GetArmorTierForBone rva=0x153ee30; cue int param "ArmorTier"), HitLocation (IsHead 0, IsLeg 2,
        // else 1; "HitLocation"), SurfaceType = the victim's UDamageableComponent.Surface (OnTookDamage rva=0x14923c0),
        // copied at BeginPlay rva=0x1459880 from AAdvancedCharacter CharacterSurface (+0xae8) = 1 (ctor decomp
        // AAdvancedCharacter.cpp 1625; no Blueprint override); anything but 1 plays BlockedSound
        let tier = damage::armor_tier(&self.fighters[v], bone);
        let hit_location = if damage::is_head(&self.spec.constants, bone) { 0 } else if damage::is_leg(&self.spec.constants, bone) { 2 } else { 1 };
        let mv_ = self.m(a, atk).attack().map(|x| x.mv).unwrap_or(-1);
        let mut ev = json!({"kind": "hit", "attacker": an, "victim": vn, "bone": bone, "damage": dmg,
            "applied": applied, "health": health, "friendly": friendly});
        if self.exe() {
            // exe mode only: the GDScript reference's events (goldens) carry no presentation fields
            if let Some((id, path, alternate)) = blood_weapon {
                ev["weapon_actor"] = json!({"slot": id.slot, "generation": id.generation});
                ev["weapon_class"] = json!(path);
                ev["weapon_alternate"] = json!(alternate);
            }
            for (k, x) in [("move", json!(mv_)), ("tier", json!(tier)), ("hit_location", json!(hit_location)), ("surface", json!(1))] {
                ev[k] = x;
            }
        }
        self.emit_event(ev);
        let team_flinch = self.mode_rules.as_ref().map(|r| r.team_damage_flinch != 0).unwrap_or(false);
        if victim_flinchable && !self.fighters[v].dead && (!friendly || team_flinch) {
            self.assign_net_flinched(v, 0.0, false, ai.flinch_duration_modifier, ai.flinch_speed_modifier);
        }
        let reward = q(ai.hit_stamina_reward + self.fighters[a].character.extra_stamina_on_hit as f64);
        if reward != 0.0 && !friendly {
            self.offset_stamina(a, trunc_i(reward));
        }
        let killed = self.fighters[v].dead;
        let stop_on_team = !self.fighters[a].character.b_is_hit_stop_on_team_hits_disabled
            && !self.mode_rules.as_ref().map(|r| r.b_is_hit_stop_on_team_hits_disabled).unwrap_or(false);
        let cont = (killed && !ad.b_stop_on_hit_on_kills)
            || (!(friendly && stop_on_team) && !self.fighters[v].character.b_will_stop_melee && !ai.b_stop_on_hit);
        if !cont {
            self.assign_net_blocked(a, br::HIT, 0, 0.0);
            return true;
        }
        let dy = self.fighters[a].net.dynamic_param;
        self.assign_net_motion_dynamic_param(a, dy | enums::DYN_HIT);
        // exe: CanContinueTracingAfterDealingDamage true -> AssignNetMotionDynamicParam(| 1), return true = keep tracing
        // (ProcessHitForDamage tail): one release can damage several characters; the reference stops after the first
        // (exe.rs difference 10)
        !self.exe()
    }
}

/// The exe's hit pipeline with real components (exe mode, both characters with geometry). Returns true = stop tracing
/// (ExecuteAttackTracingAndLogic rva=0x161c390: ProcessHitForBlocking false -> next hit; ProcessHitForDamage false ->
/// stop; the motion changed -> stop).
impl World {
    fn akey(&self, fi: usize, id: MotionId) -> u64 {
        super::geometry::attack_key(self.fighters[fi].id, id.0, self.m(fi, id).start_time)
    }

    pub(crate) fn process_hit_geo(&mut self, a: usize, v: usize, comp: &HitComp, ts: FVector, te: FVector, world: Option<&WorldBlock>, last_dir: FVector) -> bool {
        let Some(atk) = self.fighters[a].motion else { return true };
        if self.m(a, atk).attack().is_none() {
            return true;
        }
        if self.att(a, atk).hit_actors.contains(&self.fighters[v].id) {
            // ActorSetCache: ProcessHitForBlocking's weapon branch (FindId != -1 -> false) and ProcessHitForDamage's
            // (return true): skip
            return false;
        }
        // ProcessHitForBlocking's world branch (decomp UAttackMotion.cpp 4153-4166, before the parry checks): a hit on a
        // character (any component but its BlockCollider) with bBlockingHit, !bDisableWorldCollision, a valid
        // BlockingHit with an actor and Stage == Release -> HandleBlockingHit, return false (the motion is now Blocked)
        if let (Some(wb), true) = (world, matches!(comp, HitComp::Body(_))) {
            if self.att(a, atk).stage == stage::RELEASE {
                self.handle_blocking_hit(a, atk, wb, last_dir);
                return true;
            }
        }
        let damage = self.process_hit_for_blocking_geo(a, v, atk, comp, ts, te);
        if !self.is_current(a, atk) {
            return true;
        }
        if !damage {
            return false;
        }
        match comp {
            HitComp::Body(bone) => {
                let bone = bone.clone();
                self.process_damage(a, v, atk, &bone)
            }
            // a shield hit that goes on to damage (bCanTraceHitUsingShieldBlockCollider, e.g. kicks): the FHitResult has
            // no bone (NAME_None; UNCONFIRMED: ProcessHitForDamage's handling of the shield actor, taken as the owner)
            HitComp::ShieldBlock => self.process_damage(a, v, atk, ""),
            // a ClashCollider hit that reaches ProcessHitForDamage: the actor is the weapon, not a character, so no
            // character damage (UNCONFIRMED: the AMordhauActor / destructible branches of its 7004-byte body; taken as
            // returning true = continue)
            _ => false,
        }
    }

    /// UAttackMotion::ProcessHitForBlocking rva=0x1638380 with components (decomp UAttackMotion.cpp 4034-4403).
    /// true = go on to ProcessHitForDamage.
    /// A shield's BlockCollider (decomp 4340-4372: the hit component == the weapon's BlockCollider and the weapon IsA
    /// AMordhauShield -> bVar5 / cVar13): CheckParry treats it as any non-character-BlockCollider hit; without a parry
    /// the hit stops at the shield unless the attack has bCanTraceHitUsingShieldBlockCollider (decomp 4322-4324,
    /// UKickMotion ctor sets it). The passive-block branch (bCanBlockMeleePassively +0x1c90 & AttackMask ->
    /// PassiveBlockDamageModifier) is not ported: no shield in the paks sets bCanBlockMeleePassively.
    fn process_hit_for_blocking_geo(&mut self, a: usize, v: usize, atk: MotionId, comp: &HitComp, ts: FVector, te: FVector) -> bool {
        let early = self.attack_is_in_early_release(a, atk);
        let live = self.att(a, atk).is_in_live_recovery();
        let is_shield = *comp == HitComp::ShieldBlock;
        let shield_through = is_shield && self.m(a, atk).def.attack().b_can_trace_hit_using_shield_block_collider;
        let is_block = *comp == HitComp::BlockCollider;
        let is_clash = *comp == HitComp::Clash;
        let dm = self.fighters[v].motion.filter(|m| self.m(v, *m).is_attack());
        if !is_clash {
            if let Some(pm) = self.fighters[v].last_parry_motion {
                let early_ok = self.m(a, atk).def.attack().b_can_be_parried_in_early_release || !early;
                if early_ok && self.check_parry_geo(v, pm, a, atk, comp, ts, te) {
                    self.apply_parry(a, v, atk, pm);
                    if !live {
                        self.chip_damage(a, v, atk);
                    }
                    return false;
                }
            }
        }
        match dm {
            Some(dm) if !live => {
                let fists = self.att(a, atk).weapon.as_ref().map(|w| self.weapon_is_fists(w)).unwrap_or(false);
                let body_clash = !(self.att(a, atk).mv == mv::KICK || fists);
                if self.check_attack_parry(v, dm, a, atk, comp, ts, te) {
                    self.chip_damage(a, v, atk);
                    return false;
                }
                if is_shield {
                    return shield_through;
                }
                if !is_clash {
                    if self.check_chamber_geo(v, dm, a, atk, comp, ts, te) {
                        self.chip_damage(a, v, atk);
                        return false;
                    }
                    if body_clash {
                        return !is_block;
                    }
                }
                if is_block || self.check_clash(v, dm, a, atk, comp) {
                    return false;
                }
                true
            }
            _ if is_shield => shield_through,
            _ => !is_block,
        }
    }

    /// IsA AFistsWeapon of a weapon record (native class chain)
    fn weapon_is_fists(&self, w: &crate::data::WeaponData) -> bool {
        let cls = format!("A{}", w.native_class);
        self.spec.is_class_of(&cls, "AFistsWeapon")
    }

    /// The success tail of CheckParry rva=0x164ed40 (decomp 1249-1272): ModifyParryResult (native pass-through),
    /// StaminaDrain = int(StaminaDrain) (+ ExtraStaminaDrainVsHeldBlock when holdable), bDrainAllStamOnBlock -100,
    /// ReceiveBlock, OnBlockedMelee; then ProcessHitForBlocking's HandleWasParried
    fn apply_parry(&mut self, a: usize, v: usize, atk: MotionId, pm: MotionId) {
        let ai = &self.att(a, atk).ai;
        let mut drain = trunc_i(ai.stamina_drain) as f64;
        let (extra, drain_all, amv) = (ai.extra_stamina_drain_vs_held_block, ai.b_drain_all_stam_on_block, self.att(a, atk).mv);
        if self.m(v, pm).parry().unwrap().b_is_block_holdable {
            drain = trunc_i(self.qf()(drain + extra)) as f64;
        }
        if drain_all {
            self.offset_stamina(v, -100);
        }
        self.receive_block(v, pm, drain, amv, Some(a));
        let (vn, an) = (self.fighters[v].name.clone(), self.fighters[a].name.clone());
        self.trace_event(&format!("{vn} parried {an}"));
        let ev_i = self.hits.len();
        self.emit_event(json!({"kind": "parry", "attacker": an, "victim": vn, "move": amv}));
        self.handle_was_parried(a, v, atk, Some(pm));
        self.tag_disarm(ev_i, a);
    }

    /// exe mode only (goldens): the block event's FBlockResult.bIsDisarm (byte +2) as the attacker's Blocked motion
    /// carries it after HandleWasParried (AMordhauWeapon::OnBlocked_Implementation rva=0x16306d0 picks the disarm cue)
    fn tag_disarm(&mut self, ev_i: usize, attacker: usize) {
        if !self.exe() || ev_i >= self.hits.len() {
            return;
        }
        let d = self.cur_m(attacker).and_then(|m| m.blocked()).map(|b| b.b_is_disarm).unwrap_or(false);
        self.hits[ev_i].insert("disarm".into(), json!(d));
    }

    /// UParryMotion::CheckParry rva=0x164ed40 with its geometry (decomp 975-1275): timing gates as check_parry; then a
    /// BlockCollider hit records the attack in BlockedAttacks (time), marks bDetectedAnyNonFriendlyAttack, and for a
    /// timed parry needs bCanBeParriedByForwardCollider (+ ...InEarlyRelease when in early release), not friendly, and
    /// TestForwardParry; any other hit needs the attack in BlockedAttacks within Timed/HeldBlockMemoryDuration; then
    /// the attack out of Windup and the angle tests (camera - attacker root with MaxParryAngle, else the trace
    /// direction with MaxParryWeaponAngle; an attacker within 1e-4 (2D) of the camera passes)
    #[allow(clippy::too_many_arguments)]
    fn check_parry_geo(&mut self, v: usize, pm: MotionId, a: usize, atk: MotionId, comp: &HitComp, ts: FVector, te: FVector) -> bool {
        let q = self.qf();
        let t = self.now;
        let m = self.m(v, pm);
        let p = m.parry().unwrap();
        let pd = m.def.parry().clone();
        if p.stage == ps::PARRY {
            if p.b_is_shield_wall && t < q(p.parry_up_time + m.start_time) {
                return false;
            }
        } else {
            let in_flinch = self.cur_m(v).map(|c| c.is_flinch()).unwrap_or(false);
            if p.b_is_shield_wall || !in_flinch || q(pd.parry_in_flinch_duration_max + m.leave_time) < t {
                return false;
            }
            if !p.b_is_block_holdable && q(q(p.parry_up_time + m.start_time) + p.non_held_parry_extension_time) < t {
                return false;
            }
        }
        let (holdable, wall) = (p.b_is_block_holdable, p.b_is_shield_wall);
        let Some(wd) = self.parry_weapon(v) else { return false };
        let friendly = self.is_friendly(Some(v), Some(a));
        let am = self.m(a, atk);
        let at_ = am.attack().unwrap();
        let ad = am.def.attack();
        let Some(aw) = at_.weapon.clone() else { return false };
        // fVar21 (`bad`): !bCanBeParriedByForwardCollider || (!...InEarlyRelease && IsInEarlyRelease); friendly too
        let mut bad = !ad.b_can_be_parried_by_forward_collider
            || (!ad.b_can_be_parried_by_forward_collider_in_early_release && self.attack_is_in_early_release(a, atk));
        if !(at_.stage == stage::RELEASE || !friendly) {
            return false;
        }
        if friendly {
            bad = true;
        }
        let mut mask = wd.parry_mask;
        if !holdable && !wall {
            mask |= 2;
        }
        if (mask & aw.attack_mask) == 0 {
            return false;
        }
        let key = self.akey(a, atk);
        let stage_windup = self.att(a, atk).stage == stage::WINDUP;
        let dg = self.fighters[v].geom.unwrap();
        match comp {
            HitComp::BlockCollider => {
                let p = self.mm(v, pm).parry_mut().unwrap();
                match p.blocked_attacks.iter_mut().find(|e| e.0 == key) {
                    Some(e) => e.1 = t,
                    None => p.blocked_attacks.push((key, t)),
                }
                if !friendly {
                    p.b_detected_any_non_friendly_attack = true;
                }
                if holdable || bad {
                    return false;
                }
                let fwd = self.fighters[v].character.block_collider_forward_parry_distance;
                if !super::geometry::test_forward_parry(&dg, (fwd.x, fwd.y), ts, te) {
                    return false;
                }
            }
            _ => {
                let p = self.m(v, pm).parry().unwrap();
                let Some(e) = p.blocked_attacks.iter().find(|e| e.0 == key) else { return false };
                let mem = if !holdable && !wall { pd.timed_block_memory_duration } else { pd.held_block_memory_duration };
                if mem < q(t - e.1) {
                    return false;
                }
            }
        }
        if stage_windup {
            return false;
        }
        let ag = self.fighters[a].geom.unwrap();
        let dx = ag.root.x - dg.camera_loc.x;
        let dy = ag.root.y - dg.camera_loc.y;
        if (dy * dy + dx * dx).sqrt() < 1e-4 {
            return true;
        }
        let dir = FVector::new(dg.camera_loc.x - ag.root.x, dg.camera_loc.y - ag.root.y, dg.camera_loc.z - ag.root.z);
        super::geometry::check_simple_block_directional(&dg, dir, pd.max_parry_angle as f32)
            || super::geometry::check_simple_block_directional(&dg, te - ts, pd.max_parry_weapon_angle as f32)
    }

    /// UAttackMotion::CheckAttackParry rva=0x1616cd0 (decomp 5504-5690; this = the defender's attack `dm`): a Riposte
    /// not in Recovery, not a clash-collider hit, now <= StartTime + ActiveParryWindow (no shield boxes here), not out
    /// of a shield-wall parry against a kick; ParryMask & AttackMask; CheckSimpleBlock(attacker root,
    /// MaxParryAngleForChamberAndActiveParry) or the trace direction with MaxParryWeaponAngleForChamberAndActiveParry;
    /// a BlockCollider hit adds the attack to BlockedAttacks and needs (bCanBeParriedByForwardColliderInEarlyRelease or
    /// not early release), TestForwardParry and not friendly; other hits need the attack in BlockedAttacks. Success:
    /// AssignNetBlock (mh-net event), the attacker's HandleWasPassivelyParried rva=0x162b710 (byte-matched) ->
    /// PutUsInBlockedMotionFromParry rva=0x163a7d0, stamina -ActiveParryStaminaCost (not for a negative cost out of a
    /// shield wall)
    #[allow(clippy::too_many_arguments)]
    fn check_attack_parry(&mut self, v: usize, dm: MotionId, a: usize, atk: MotionId, comp: &HitComp, ts: FVector, te: FVector) -> bool {
        let q = self.qf();
        if *comp == HitComp::Clash {
            return false;
        }
        let m = self.m(v, dm);
        let d = m.attack().unwrap();
        let dd = m.def.attack().clone();
        if d.ty != at::RIPOSTE || d.stage == stage::RECOVERY {
            return false;
        }
        if !(self.now <= q(dd.active_parry_window + m.start_time)) {
            return false;
        }
        let from_wall = m.coming_from.and_then(|c| self.m(v, c).parry()).map(|p| p.b_is_shield_wall).unwrap_or(false);
        if from_wall && self.att(a, atk).mv == mv::KICK {
            return false;
        }
        let (Some(dw), Some(aw)) = (d.weapon.clone(), self.att(a, atk).weapon.clone()) else { return false };
        if (dw.parry_mask & aw.attack_mask) == 0 {
            return false;
        }
        let (dg, ag) = (self.fighters[v].geom.unwrap(), self.fighters[a].geom.unwrap());
        let ok = super::geometry::check_simple_block(&dg, ag.root, dd.max_parry_angle_for_chamber_and_active_parry as f32)
            || super::geometry::check_simple_block_directional(&dg, te - ts, dd.max_parry_weapon_angle_for_chamber_and_active_parry as f32);
        if !ok {
            return false;
        }
        let key = self.akey(a, atk);
        if *comp == HitComp::BlockCollider {
            let dmm = self.att_mut(v, dm);
            if !dmm.blocked_attacks.contains(&key) {
                dmm.blocked_attacks.push(key);
            }
            let ad = self.m(a, atk).def.attack();
            let early_ok = ad.b_can_be_parried_by_forward_collider_in_early_release || !self.attack_is_in_early_release(a, atk);
            let fwd = self.fighters[v].character.block_collider_forward_parry_distance;
            if !(early_ok && super::geometry::test_forward_parry(&dg, (fwd.x, fwd.y), ts, te)) {
                return false;
            }
            if self.is_friendly(Some(v), Some(a)) {
                return false;
            }
        } else if !self.att(v, dm).blocked_attacks.contains(&key) {
            return false;
        }
        let (vn, an) = (self.fighters[v].name.clone(), self.fighters[a].name.clone());
        self.trace_event(&format!("{vn} active-parried {an}"));
        let ev_i = self.hits.len();
        self.emit_event(json!({"kind": "active_parry", "attacker": an, "victim": vn}));
        let clash = self.att(a, atk).ai.b_will_clash_when_parried;
        if clash {
            self.assign_net_blocked(a, br::CLASH, 64, 0.0);
        } else {
            let w = self.fastest_attack_windup(v);
            self.assign_net_blocked(a, br::PARRY, 0, w);
        }
        self.tag_disarm(ev_i, a);
        let cost = dd.active_parry_stamina_cost;
        if cost != 0 && (cost >= 0 || !from_wall) {
            self.offset_stamina(v, -cost);
        }
        true
    }

    /// UAttackMotion::CheckChamber rva=0x1617140 with its geometry (decomp 4730-5070): chamber_gate, then
    /// CheckSimpleBlock(attacker root, MaxParryAngleForChamberAndActiveParry) or the trace direction with
    /// MaxParryWeaponAngleForChamberAndActiveParry; a BlockCollider hit adds the attack to BlockedAttacks, refuses an
    /// early-release attack without bCanBeParriedByForwardColliderInEarlyRelease (+0xa6f) and needs TestForwardParry;
    /// other hits need the attack in BlockedAttacks. Then the chamber itself (check_chamber).
    #[allow(clippy::too_many_arguments)]
    fn check_chamber_geo(&mut self, v: usize, dm: MotionId, a: usize, atk: MotionId, comp: &HitComp, ts: FVector, te: FVector) -> bool {
        if !self.chamber_gate(v, dm, a) {
            return false;
        }
        let dd = self.m(v, dm).def.attack().clone();
        let (dg, ag) = (self.fighters[v].geom.unwrap(), self.fighters[a].geom.unwrap());
        let ok = super::geometry::check_simple_block(&dg, ag.root, dd.max_parry_angle_for_chamber_and_active_parry as f32)
            || super::geometry::check_simple_block_directional(&dg, te - ts, dd.max_parry_weapon_angle_for_chamber_and_active_parry as f32);
        if !ok {
            return false;
        }
        let key = self.akey(a, atk);
        if *comp == HitComp::BlockCollider {
            let dmm = self.att_mut(v, dm);
            if !dmm.blocked_attacks.contains(&key) {
                dmm.blocked_attacks.push(key);
            }
            let ad = self.m(a, atk).def.attack();
            if !ad.b_can_be_parried_by_forward_collider_in_early_release && self.attack_is_in_early_release(a, atk) {
                return false;
            }
            let fwd = self.fighters[v].character.block_collider_forward_parry_distance;
            if !super::geometry::test_forward_parry(&dg, (fwd.x, fwd.y), ts, te) {
                return false;
            }
        } else if !self.att(v, dm).blocked_attacks.contains(&key) {
            return false;
        }
        self.check_chamber(a, v, atk, dm);
        true
    }

    /// UAttackMotion::CanClashWith rva=0x16164b0 (byte-matched src/.../AttackMotion.cpp): not a Riposte; in Release
    /// past ClashableAfter = (PostClash ? 0 : EarlyRelease + EarlyReleaseIsClashableAfter)
    fn can_clash_with(&self, fi: usize, id: MotionId) -> bool {
        let m = self.m(fi, id);
        let a = m.attack().unwrap();
        if a.ty == at::RIPOSTE {
            return false;
        }
        let after = if a.ty == at::POST_CLASH { 0.0 } else { self.qf()(a.early_release + m.def.attack().early_release_is_clashable_after) };
        a.stage == stage::RELEASE && after < a.last_release_norm
    }

    /// UAttackMotion::CheckClash rva=0x16179a0 (decomp 5072-5300; this = the defender's attack `dm`): both attacks with
    /// weapons; AMordhauGameMode::CanClash (rva=0x1586d60: !IsFriendly); a fists attacker only on the defender's arm /
    /// forearm / hand on the side of its move (NAME_Left* when IsLeft); AttackMask & AttackMask; both CanClashWith;
    /// CheckSimpleBlock both ways (each root against the other's camera) with the defender attack's ClashAngle.
    /// Success: both Blocked(Clash) - the defender plain, the attacker with bPartyFlag (FBlockResult byte 5 -> Param1
    /// bit 16) - and ApplyBackwardsKnockbackIfNotInKnockback(KnockbackClash) on both (`knockback` events)
    fn check_clash(&mut self, v: usize, dm: MotionId, a: usize, atk: MotionId, comp: &HitComp) -> bool {
        let (Some(dw), Some(aw)) = (self.att(v, dm).weapon.clone(), self.att(a, atk).weapon.clone()) else { return false };
        if self.mode_rules.is_some() && self.is_friendly(Some(v), Some(a)) {
            return false;
        }
        if self.weapon_is_fists(&aw) {
            let bone = if let HitComp::Body(b) = comp { b.as_str() } else { "" };
            let left = enums::is_left(self.att(a, atk).mv);
            let ok = if left { ["LeftArm", "LeftForeArm", "LeftHand"] } else { ["RightArm", "RightForeArm", "RightHand"] };
            if !ok.iter().any(|b| b.eq_ignore_ascii_case(bone)) {
                return false;
            }
        }
        if (dw.attack_mask & aw.attack_mask) == 0 || !self.can_clash_with(v, dm) || !self.can_clash_with(a, atk) {
            return false;
        }
        let ang = self.m(v, dm).def.attack().clash_angle as f32;
        let (dg, ag) = (self.fighters[v].geom.unwrap(), self.fighters[a].geom.unwrap());
        if !(super::geometry::check_simple_block(&dg, ag.root, ang) && super::geometry::check_simple_block(&ag, dg.root, ang)) {
            return false;
        }
        self.assign_net_blocked(v, br::CLASH, 0, 0.0);
        self.assign_net_blocked(a, br::CLASH, 16, 0.0);
        let (vn, an) = (self.fighters[v].name.clone(), self.fighters[a].name.clone());
        self.trace_event(&format!("{an} clashed {vn}"));
        let ev_i = self.hits.len();
        self.emit_event(json!({"kind": "clash", "attacker": an, "victim": vn}));
        self.tag_disarm(ev_i, a);
        self.emit_event(json!({"kind": "knockback", "who": vn, "what": "KnockbackClash"}));
        self.emit_event(json!({"kind": "knockback", "who": an, "what": "KnockbackClash"}));
        true
    }
}

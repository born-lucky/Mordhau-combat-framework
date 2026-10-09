//! A melee weapon against the world (walls, terrain, props): the BlockingHit side of the attack trace.
//! (extract/native/decomp/UAttackMotion.cpp, AMordhauWeapon.cpp, AMordhauCharacter.cpp)
//!
//! - AMordhauWeapon::SampleTracers rva=0x163c430 / SampleTracer rva=0x163be10 (the host, mh-sim trace.rs): the first
//!   tracer whose UWorld::LineTraceSingleByObjectType(ECC_WorldStatic) hits gives the BlockingHit; 4 extra
//!   environment-only tracers past the hilt when bUsesExtraEnvironmentTracers (+0x1a18) and the move is not a Kick /
//!   Bash. The weapon trace proper (LineTraceMultiByChannel 0xf = "Weapon", DefaultEngine.ini: default Ignore) does
//!   not see level geometry.
//! - UAttackMotion::ExecuteAttackTracingAndLogic rva=0x161c390 (decomp 6137-6174): in Release, a BlockingHit with an
//!   actor and !bDisableWorldCollision triggers when |ClosestPointOnSegment(Impact, LastTraceStart, LastTraceEnd) -
//!   LastTraceStart| < max(45, min(WorldCollisionPercentageTriggerCurve(LastReleaseNormalizedTime) x TraceLength,
//!   TraceLength - WorldCollisionAbsoluteTriggerCurve(LastReleaseNormalizedTime))) (each curve only when set; none:
//!   TraceLength) -> HandleBlockingHit, stop.
//! - UAttackMotion::ProcessHitForBlocking rva=0x1638380 (decomp 4153-4166): a character hit flagged bBlockingHit (the
//!   world was hit by its tracer or an earlier one) -> HandleBlockingHit (melee_hit.rs process_hit_geo).
//! - UAttackMotion::HandleBlockingHit rva=0x162afa0 (decomp 6203-6349): FBlockResult {Reason = Hit when the actor
//!   bCanBeDamaged or the attack bHasHit, else World; bRequiresSelfBlockEvent; Surface = the BlockingHit's
//!   PhysMaterial SurfaceType} -> AMordhauCharacter::AssignNetBlock rva=0x1530ec0 (-> OnRep_NetBlock rva=0x155a4e0 ->
//!   AMordhauWeapon::OnWasBlocked_Implementation rva=0x16340a0: the impact sound / particles by surface); World:
//!   ApplyBackwardsKnockbackIfNotInKnockback(KnockbackWorld) rva=0x1530600; then AssignNetMotion(Blocked, Reason,
//!   flags with bRequiresSelfBlockEvent = 32) -> UBlockedMotion (blocked.rs: WorldRecoveryTime, world stamina,
//!   MovementRestrictionWorld, the world bounce curves).

use super::enums::br;
use super::motion::MotionId;
use super::world::{World, WorldBlock};
use crate::ue::{maxf, FVector};
use serde_json::json;

/// FMath::ClosestPointOnSegment rva=0x18a2bf0 (disassembled, binary32): Dot1 = (P - A).(B - A) as (y*y + x*x) + z*z;
/// Dot1 <= 0 -> A; |B - A|^2 <= Dot1 -> B; else A + (B - A) x (Dot1 / Dot2)
pub fn closest_point_on_segment(p: FVector, a: FVector, b: FVector) -> FVector {
    let (sx, sy, sz) = (b.x - a.x, b.y - a.y, b.z - a.z);
    let (vx, vy, vz) = (p.x - a.x, p.y - a.y, p.z - a.z);
    let dot1 = (vy * sy + vx * sx) + vz * sz;
    if !(dot1 > 0.0) {
        return a;
    }
    let dot2 = (sy * sy + sx * sx) + sz * sz;
    if !(dot2 > dot1) {
        return b;
    }
    let t = dot1 / dot2;
    FVector::new(t * sx + a.x, t * sy + a.y, t * sz + a.z)
}

/// The 45 cm floor of the world collision trigger distance (ExecuteAttackTracingAndLogic, decomp 6166:
/// `if (fVar13 <= 45.0) fVar13 = 45.0`)
pub const WORLD_COLLISION_MIN_TRIGGER: f32 = 45.0;
/// OnWasBlocked_Implementation rva=0x16340a0 (decomp AMordhauWeapon.cpp 1603-1605, 1652-1654, 1670-1672): the point
/// 30 cm along the trace from its start, the 50 cm world re-trace back along LastObservedTraceDirection, and the 10 cm
/// pull-back from its impact
pub const WAS_BLOCKED_POINT_ALONG: f32 = 30.0;
pub const WAS_BLOCKED_RETRACE: f32 = 50.0;
pub const WAS_BLOCKED_PULLBACK: f32 = 10.0;
/// AMordhauCharacter::OnRep_NetBlock rva=0x155a4e0 (decomp AMordhauCharacter.cpp 6752): nothing before the world's
/// TimeSeconds reaches 7
pub const NET_BLOCK_MIN_WORLD_TIME: f64 = 7.0;

impl World {
    /// ExecuteAttackTracingAndLogic rva=0x161c390 (decomp UAttackMotion.cpp 6141-6170): whether the BlockingHit's
    /// impact lies within the trigger distance of LastTraceStart (the attack's LastTraceStart / LastTraceEnd are this
    /// tick's CurrentTraceStart / CurrentTraceEnd, stored just before)
    pub(crate) fn world_collision_triggers(&self, fi: usize, atk: MotionId, impact: FVector) -> bool {
        let m = self.m(fi, atk);
        let a = m.attack().unwrap();
        let ad = m.def.attack();
        let (s, e) = (a.last_trace_start, a.last_trace_end);
        let (dx, dy, dz) = (e.x - s.x, e.y - s.y, e.z - s.z);
        let trace_len = ((dy * dy + dx * dx) + dz * dz).sqrt();
        let p = closest_point_on_segment(impact, s, e);
        let (px, py, pz) = (p.x - s.x, p.y - s.y, p.z - s.z);
        let dist = ((py * py + px * px) + pz * pz).sqrt();
        let lrn = a.last_release_norm;
        let mut pct = trace_len;
        if !ad.world_collision_percentage_trigger_curve.is_empty() {
            pct = self.spec.curve_value(&ad.world_collision_percentage_trigger_curve, lrn) as f32 * trace_len;
        }
        let mut thr = pct;
        if !ad.world_collision_absolute_trigger_curve.is_empty() {
            let abs = trace_len - self.spec.curve_value(&ad.world_collision_absolute_trigger_curve, lrn) as f32;
            thr = if pct <= abs { pct } else { abs };
        }
        let thr = maxf(thr as f64, WORLD_COLLISION_MIN_TRIGGER as f64) as f32;
        dist < thr
    }

    /// UAttackMotion::HandleBlockingHit rva=0x162afa0 for fighter `fi`'s attack `atk` (see the module docs).
    /// `last_dir`: the weapon's LastObservedTraceDirection.
    pub(crate) fn handle_blocking_hit(&mut self, fi: usize, atk: MotionId, wb: &WorldBlock, last_dir: FVector) {
        let q = self.qf();
        let (mv, has_hit) = {
            let a = self.att(fi, atk);
            (a.mv, a.b_has_hit)
        };
        // InDamage for the blocking actor's TakeDamage (vcall +0x590): SurfaceType 1 -> Damage[0], 2 -> WoodDamage, else
        // StoneDamage (x StructureDamageModifier / StructureRepairModifier of the attacker: not ported, UNCONFIRMED 1);
        // 0 in early release with bNoDamageInEarlyRelease (+0xa6d). Level geometry ignores it (no damage handler);
        // a damageable actor's response (destructibles) is the host's (the "damage" field of the event)
        let ai = self.att(fi, atk).ai.clone();
        let mut dmg = match wb.surface {
            1 => ai.damage.first().copied().unwrap_or(0.0) as f64,
            2 => ai.wood_damage,
            _ => ai.stone_damage,
        };
        if self.attack_is_in_early_release(fi, atk) && self.m(fi, atk).def.attack().b_no_damage_in_early_release {
            dmg = 0.0;
        }
        let reason = if wb.can_be_damaged || has_hit { br::HIT } else { br::WORLD };
        // AssignNetBlock -> OnRep_NetBlock -> OnWasBlocked(Result, BlockedMove) on the right-hand weapon (the
        // KickWeapon for a kick, decomp AMordhauCharacter.cpp 6838-6870)
        if self.now >= NET_BLOCK_MIN_WORLD_TIME {
            self.on_was_blocked_self_event(fi, atk, mv, reason, wb, last_dir, q(dmg));
        }
        if reason == br::WORLD {
            // ApplyBackwardsKnockbackIfNotInKnockback(KnockbackWorld +0xe88): the movement side is the host's
            // (mh-sim applies "knockback" events carrying an amount: Knockback(-Forward x Amount))
            let kw = self.fighters[fi].character.knockback_world;
            let name = self.fighters[fi].name.clone();
            self.emit_event(json!({"kind": "knockback", "who": name, "what": "KnockbackWorld", "amount": kw}));
        }
        let name = self.fighters[fi].name.clone();
        self.trace_event(&format!("{name} hit the world ({})", if reason == br::WORLD { "World" } else { "Hit" }));
        // FNetMotion {MotionType 6 Blocked, Param0 Reason, Param1 the FBlockResult bits (bRequiresSelfBlockEvent = 32),
        // Param2 0} (decomp 6311-6347)
        self.assign_net_blocked(fi, reason, 32, 0.0);
    }

    /// AMordhauWeapon::OnWasBlocked_Implementation rva=0x16340a0 with bRequiresSelfBlockEvent (decomp
    /// AMordhauWeapon.cpp 1585-1773, 1832-1849): PointWorld = TraceStart + normal(TraceEnd - TraceStart) x 30; a
    /// LineTraceSingleByObjectType(WorldStatic) from it to PointWorld - LastObservedTraceDirection x 50; a hit moves
    /// PointWorld (and the sound point) to its ImpactPoint - LastObservedTraceDirection x 10. Then OnHit(null actor,
    /// Move, NAME_None, SoundPointWorld, tier 0, Result.Surface) (the impact sound by surface) and
    /// ImpactParticlesBySurface[Surface] at PointWorld, rotation MakeFromXZ(-LastObservedTraceDirection, Up).
    /// UNCONFIRMED (not ported): the view target's Camera-channel re-trace of the sound point from its Spine1 socket
    /// (decomp 1677-1769; the local player's own presentation).
    fn on_was_blocked_self_event(&mut self, fi: usize, atk: MotionId, mv: i64, reason: i64, wb: &WorldBlock, last_dir: FVector, damage: f64) {
        let a = self.att(fi, atk);
        let (s, e) = (a.last_trace_start, a.last_trace_end);
        let d = crate::ue::ue_safe_normal(FVector::new(e.x - s.x, e.y - s.y, e.z - s.z));
        let mut point = FVector::new(d.x * WAS_BLOCKED_POINT_ALONG + s.x, d.y * WAS_BLOCKED_POINT_ALONG + s.y, d.z * WAS_BLOCKED_POINT_ALONG + s.z);
        let back = FVector::new(point.x - last_dir.x * WAS_BLOCKED_RETRACE, point.y - last_dir.y * WAS_BLOCKED_RETRACE, point.z - last_dir.z * WAS_BLOCKED_RETRACE);
        if let Some(h) = self.trace_host.clone().and_then(|h| h.world_static_trace(point, back)) {
            point = FVector::new(h.x - last_dir.x * WAS_BLOCKED_PULLBACK, h.y - last_dir.y * WAS_BLOCKED_PULLBACK, h.z - last_dir.z * WAS_BLOCKED_PULLBACK);
        }
        let name = self.fighters[fi].name.clone();
        let bi = wb.impact_point;
        self.emit_event(json!({"kind": "world_hit", "who": name, "move": mv, "reason": reason, "surface": wb.surface,
            "component": wb.component, "impact": [point.x, point.y, point.z], "blocking_impact": [bi.x, bi.y, bi.z],
            "trace_dir": [last_dir.x, last_dir.y, last_dir.z], "kick": mv == super::enums::mv::KICK, "damage": damage}));
    }
}

//! Ranged hits on characters: the character branch of AMordhauProjectile::ProcessProjectileHit_Implementation
//! rva=0x15ebfd0 (extract/native/decomp/AMordhauProjectile.cpp, function lines 9-1774), the combat half of the
//! projectile (rust-combat r4). The flight and the sweep are mh-character's projectile.rs (`Projectile::tick` gives
//! the `ProjectileHit`s); the host passes each character hit here.
//!
//! What the exe does for a character `Defender` (decomp lines cited are of AMordhauProjectile.cpp):
//!  1. friendly = AMordhauGameState::IsFriendly(OwningController, Defender) (371-374).
//!  2. the defender's current motion (MotionSystem +0x688 ->Motion): a UParryMotion in Stage Parry (+0x53a == 0),
//!     not bIsBlockHoldable (+0x539) and not bIsShieldWall (+0x46a) -> its WeaponPtr (+0x550) (386-433). With that
//!     weapon and (ParryMask (+0x19ac) & the projectile's AttackMask (+0x594)) != 0 (478-480):
//!     UParryMotion::CheckSimpleBlock(Defender, ImpactPoint - normalize(Velocity) * 200, 80) (482-518) ->
//!     parried: UParryMotion::ReceiveBlock(motion, 0, Move 7 = Ranged, nullptr) (531), the projectile bounces with
//!     ParryBounceForce; no damage.
//!  3. damage multiplier F (450-476): 1, or the defender's current UAttackMotion's RiposteTradeDamageFactor (+0xa9c)
//!     when it is a Riposte (Type +0x108d == 1) not in Recovery (Stage +0x10e9 != 2).
//!  4. for a character (DamageableComponent kind +0xae8 == 1) damage = AMordhauCharacter::ComputeRangedDamage
//!     (vcall +0x950, rva=0x145aed0 = an ICF jump to vcall +0x948 ComputeMeleeDamage with the projectile's Damage
//!     (+0x718) / HeadBonus (+0x728) / LegBonus (+0x738) arrays and the hit bone) (1312-1316); HitKnockback > 0 ->
//!     Defender->Knockback(forward * HitKnockback) (vcall +0x998; the host's, returned) (1317-1366).
//!  5. FMordhauDamageEvent(damage * F * DamageMultiplier (+0x598), Hit, EMordhauDamageType Ranged, ...) and
//!     Defender->TakeDamage (vcall +0x590) (1371-1423): the ordinary take_damage path (ModifyDamage's
//!     ReceivedRangedDamageModifier, last chance on Ranged).
//!  6. flinch (1372-1418): bShouldFlinch (+0x81c) true -> flinch unless friendly or the current motion is not
//!     bIsFlinchable (+0x60); bShouldFlinch false -> flinch only when the defender is drawing / reloading
//!     (URangedDrawMotion / UReloadMotion with +0xe65 clear: not modelled here, those motions are not in the core).
//! There is NO distance falloff: the damage arrays are indexed by armour tier only, as for melee (GetPercentage-
//! OfMaxVelocityClamped feeds only the bounce forces, lines 294 / 332 / 1665 / 1680).
//! UNCONFIRMED: the flinch's duration / speed modifiers (1, 1 here; the damage event's flinch flag is consumed inside
//! TakeDamage, not followed), the stamina side of ReceiveBlock for Move Ranged beyond parry.rs's port.

use super::damage;
use super::enums::{at, stage};
use super::motion::MotionKind;
use super::system::DAMAGE_RANGED;
use super::world::World;
use crate::data::AttackInfo;
use crate::ue::FVector;
use serde_json::json;

/// AMordhauProjectile's damage data (class defaults chain)
#[derive(Clone, Debug, Default)]
pub struct ProjectileDamage {
    pub damage: Vec<f32>,     // +0x718
    pub head_bonus: Vec<f32>, // +0x728
    pub leg_bonus: Vec<f32>,  // +0x738
    pub damage_multiplier: f64, // +0x598 (ctor 1)
    pub attack_mask: i64,     // +0x594
    pub hit_knockback: f32,   // +0x750
    pub should_flinch: bool,  // +0x81c
}

/// What a projectile hit on a character did
#[derive(Clone, Debug, PartialEq)]
pub enum ProjectileOutcome {
    /// parried by the defender's parry (ReceiveBlock ran); the projectile bounces (ParryBounceForce)
    Parried,
    /// damage dealt (as applied after ModifyDamage), and the knockback impulse the host applies (zero when none)
    Damaged { damage: f64, applied: f64, knockback: FVector },
    /// the defender is dead / unknown
    Ignored,
}

pub const MOVE_RANGED: i64 = 7;

impl World {
    /// The character branch of ProcessProjectileHit (module docs). `shooter`: the owning pawn's fighter (friendly
    /// test, kill credit); `velocity`: the projectile's velocity at the hit; `forward`: its root's forward vector
    /// (knockback direction); `impact`: Hit.ImpactPoint (UE cm).
    #[allow(clippy::too_many_arguments)]
    pub fn projectile_hit(&mut self, defender: usize, shooter: Option<usize>, p: &ProjectileDamage, bone: &str, impact: FVector, velocity: FVector, forward: FVector) -> ProjectileOutcome {
        let q = self.qf();
        if self.fighters[defender].dead {
            return ProjectileOutcome::Ignored;
        }
        let friendly = shooter.is_some() && self.is_friendly(shooter, Some(defender));
        // 2. the parry test
        if let Some(pm) = self.fighters[defender].motion {
            let m = self.m(defender, pm);
            if let MotionKind::Parry(pr) = &m.k {
                let open = pr.stage == 0 && !pr.b_is_block_holdable && !pr.b_is_shield_wall;
                // the parry motion's WeaponPtr = the weapon it parries with (World::parry_weapon)
                let mask = self.parry_weapon(defender).map(|w| w.parry_mask).unwrap_or(0);
                if open && (mask & p.attack_mask) != 0 {
                    if let Some(g) = self.fighters[defender].geom.as_ref() {
                        let n = crate::ue::ue_safe_normal(velocity);
                        let from = FVector::new(impact.x - n.x * 200.0, impact.y - n.y * 200.0, impact.z - n.z * 200.0);
                        if super::geometry::check_simple_block(g, from, 80.0) {
                            self.receive_block(defender, pm, 0.0, MOVE_RANGED, None);
                            let name = self.fighters[defender].name.clone();
                            self.trace_event(&format!("{name} parried a projectile"));
                            self.emit_event(json!({"kind": "projectile_parried", "victim": name}));
                            return ProjectileOutcome::Parried;
                        }
                    }
                }
            }
        }
        // 3. the riposte trade factor
        let mut factor = 1.0;
        let mut flinchable = true;
        if let Some(cm) = self.cur_m(defender) {
            flinchable = cm.b_is_flinchable;
            if let Some(a) = cm.attack() {
                if a.stage != stage::RECOVERY && a.ty == at::RIPOSTE {
                    factor = cm.def.attack().riposte_trade_damage_factor;
                }
            }
        }
        // 4. ComputeRangedDamage = ComputeMeleeDamage with the projectile's arrays
        let ai = AttackInfo { damage: p.damage.clone(), head_bonus: p.head_bonus.clone(), leg_bonus: p.leg_bonus.clone(), ..Default::default() };
        let base = damage::compute(&self.spec.constants, &self.fighters[defender], &ai, bone, q);
        let knockback = if 0.0 < p.hit_knockback {
            let f = crate::ue::ue_safe_normal(forward);
            FVector::new(p.hit_knockback * f.x, p.hit_knockback * f.y, p.hit_knockback * f.z)
        } else {
            FVector::ZERO
        };
        // 5. the damage event
        let dmg = q(q(base * factor) * p.damage_multiplier);
        let applied = self.take_damage(defender, dmg, shooter, DAMAGE_RANGED);
        let vn = self.fighters[defender].name.clone();
        let an = shooter.map(|s| self.fighters[s].name.clone()).unwrap_or_default();
        self.trace_event(&format!("{an} shot {vn} {bone} {dmg:.2}"));
        let health = self.fighters[defender].health;
        self.emit_event(json!({"kind": "hit", "ranged": true, "attacker": an, "victim": vn, "bone": bone, "damage": dmg,
            "applied": applied, "health": health, "friendly": friendly}));
        // 6. flinch
        if p.should_flinch && !friendly && flinchable && !self.fighters[defender].dead {
            self.assign_net_flinched(defender, 0.0, false, 1.0, 1.0);
        }
        ProjectileOutcome::Damaged { damage: dmg, applied, knockback }
    }
}

/// ProjectileDamage from a projectile class's defaults (a `serde_json` object of the merged CDO chain); absent values
/// are the AMordhauProjectile ctor's (rva=0x15b5660, decomp AMordhauProjectile.cpp 5640 AttackMask 1, 5736
/// bShouldFlinch true, 5766 DamageMultiplier 1); absent arrays are empty (no damage), HitKnockback 0
pub fn projectile_damage_from_defaults(d: &serde_json::Map<String, serde_json::Value>) -> ProjectileDamage {
    let arr = |k: &str| -> Vec<f32> { d.get(k).and_then(|v| v.as_array()).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default() };
    let f = |k: &str, z: f64| d.get(k).and_then(|v| v.as_f64()).unwrap_or(z);
    ProjectileDamage {
        damage: arr("Damage"),
        head_bonus: arr("HeadBonus"),
        leg_bonus: arr("LegBonus"),
        damage_multiplier: crate::ue::f32r(f("DamageMultiplier", 1.0)),
        attack_mask: d.get("AttackMask").and_then(|v| v.as_i64()).unwrap_or(1),
        hit_knockback: f("HitKnockback", 0.0) as f32,
        should_flinch: d.get("bShouldFlinch").and_then(|v| v.as_bool()).unwrap_or(true),
    }
}

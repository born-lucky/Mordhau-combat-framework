//! How much a melee hit on a bone does (godot/game/combat/melee_damage.gd): AMordhauCharacter::ComputeMeleeDamage
//! rva=0x1536fb0, GetArmorTierForBone rva=0x153ee30, IsHead rva=0x154aa40, IsLeftLeg rva=0x154ad60, IsRightLeg
//! rva=0x154b880, IsLeg rva=0x154aed0.
//!   tier = DamageArmorTierOverride (+0x7f4) unless -1, else WearableProtectionCoverageMap[bone] (+0xb18)->ArmorClass
//!   damage = Damage[min(tier, Num-1)] + HeadBonus[..] on NAME_Head, or + LegBonus[..] on a leg bone
//!            (x LegDamageBonusModifierAirborne +0xe58 when airborne). FName comparison ignores case.

use super::system::Fighter;
use crate::data::{AttackInfo, Constants};

fn is_in(bone: &str, names: &[String]) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(bone))
}

pub fn is_head(c: &Constants, bone: &str) -> bool {
    is_in(bone, &c.head_bones)
}

pub fn is_leg(c: &Constants, bone: &str) -> bool {
    is_in(bone, &c.left_leg_bones) || is_in(bone, &c.right_leg_bones)
}

fn at(a: &[f32], tier: i64) -> f64 {
    let i = tier.min(a.len() as i64 - 1);
    if i >= 0 && (i as usize) < a.len() { a[i as usize] as f64 } else { 0.0 }
}

/// AMordhauCharacter::GetArmorTierForBone rva=0x153ee30
pub fn armor_tier(who: &Fighter, bone: &str) -> i64 {
    let mut tier = who.armor_tier_override;
    if tier == -1 {
        tier = 0;
        for (k, v) in &who.wearable_coverage {
            if k.eq_ignore_ascii_case(bone) {
                tier = *v;
            }
        }
    }
    tier
}

/// AMordhauCharacter::BuildCharacter rva=0x15319e0 (decomp AMordhauCharacter.cpp 5840-6000): the
/// WearableProtectionCoverageMap (+0xb18), bone -> the wearable in slot Head (Head, Neck), UpperChest (Hips, LowerBack,
/// Spine, Spine1, both Hand / Forearm / Arm) and Legs (both UpLeg / Leg / Foot). GetArmorTierForBone rva=0x153ee30
/// returns the mapped wearable's ArmorClass (+0x1bc), 0 for an unmapped bone or an empty slot (`None` here).
pub fn wearable_coverage(head: Option<u8>, upper_chest: Option<u8>, legs: Option<u8>) -> Vec<(String, i64)> {
    const HEAD: [&str; 2] = ["Head", "Neck"];
    const UPPER: [&str; 10] = ["Hips", "LowerBack", "Spine", "Spine1", "LeftHand", "LeftForearm", "LeftArm", "RightHand", "RightForearm", "RightArm"];
    const LEGS: [&str; 6] = ["LeftUpLeg", "LeftLeg", "LeftFoot", "RightUpLeg", "RightLeg", "RightFoot"];
    let mut out = Vec::new();
    for (bones, ac) in [(&HEAD[..], head), (&UPPER[..], upper_chest), (&LEGS[..], legs)] {
        for b in bones {
            out.push((b.to_string(), ac.unwrap_or(0) as i64));
        }
    }
    out
}

/// AMordhauCharacter::ComputeMeleeDamage rva=0x1536fb0
pub fn compute(c: &Constants, victim: &Fighter, ai: &AttackInfo, bone: &str, q: fn(f64) -> f64) -> f64 {
    let tier = armor_tier(victim, bone);
    let mut d = at(&ai.damage, tier);
    if is_head(c, bone) {
        d = q(d + at(&ai.head_bonus, tier));
    } else if is_leg(c, bone) {
        let mut lb = at(&ai.leg_bonus, tier);
        if victim.airborne {
            lb = q(lb * victim.character.leg_damage_bonus_modifier_airborne);
        }
        d = q(d + lb);
    }
    d
}

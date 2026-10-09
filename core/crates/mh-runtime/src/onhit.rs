//! onhit.rs - which weapon sounds a combat event plays, and with which SoundCue parameters, ported from the exe
//! (fidelity-audit r1). The runtime used to play the attacker weapon's StrikeHitSound / StabHitSound / BlockedSound /
//! HitCancelSound with no cue parameters (mh_audio::WeaponSounds::for_event, names-only mapping); the game instead:
//!
//! - a damaging melee hit on a character: UDamageableComponent::OnTookDamage rva=0x14923c0 calls the attacker's
//!   AMordhauWeapon::OnHit(victim, Move, Bone, Point, Tier = victim GetArmorTierForBone(Bone) (vtable +0x958),
//!   SurfaceType = the victim's UDamageableComponent Surface) -> OnHit_Implementation rva=0x1631430 (decomp
//!   AMordhauWeapon.cpp 371-780) plays TWO sounds:
//!     A: StrikeHitSound (Move < 2) / StabHitSound (Move 2, 3), or BlockedSound when SurfaceType != 1, at volume 1 /
//!        pitch 1, int params HitLocation (IsHead 0 / IsLeg 2 / else 1), ArmorTier (Tier), SurfaceType;
//!     B: EnvironmentHitSound at volume lerp(EnvironmentVolumeScaleByDamageOut, a_v) and pitch
//!        lerp(EnvironmentPitchScaleByDamageOut, a_p), a = clamp01((damage - In.X) / (In.Y - In.X)) with damage =
//!        |AttackInfo.Damage[tier'] + (HitLocation' == 0 ? HeadBonus[tier'] : 0)| from StrikeAttack (Move < 2),
//!        StabAttack (Move 2, 3) or KickAttack (else); tier' = Tier for flesh else 2, HitLocation' = 1 when not flesh;
//!        int params HitLocation', ArmorTier tier', SurfaceType; bool IsStab (Move 2/3 && !bStopOnHit) and
//!        IsSourceViewTarget (the attacker is the view target).
//! - OnHit with no advanced-character recipient (OnWasBlocked's self event): both sounds go through
//!   PlayEquipmentSoundWorld rva=0x155dd20 at fixed volume/pitch 1/1, preserving those cue parameters. The damage
//!   mapping applies only to the character-recipient branch (AMordhauWeapon.cpp 712-724).
//! - a block: the defender's AMordhauCharacter::OnRep_NetBlock rva=0x155a4e0 -> its weapon's
//!   OnBlocked_Implementation rva=0x16306d0: BlockedSound with int param Reason (0 parry, 1 chamber, 2 clash,
//!   3 disarm), plus BlockedViewTargetSweetener when the defender is the view target.
//! - the attacker's blocked attack: OnWasBlocked_Implementation rva=0x16340a0: WasBlockedSound with Reason (1 chamber,
//!   2 clash, else 0) unless the reason is Hit / World; HitCancelSound instead when FBlockResult.bIsCancel.
//! Class defaults: AMordhauWeapon ctor (AMordhauWeapon.cpp 2159-2166): EnvironmentPitchScaleByDamageIn (25, 75) /
//! Out (1.1, 0.9), EnvironmentVolumeScaleByDamageIn (10, 30) / Out (0.9, 1.0), overridden by the Blueprint CDO chain.

use std::collections::HashMap;

/// The OnHit / OnBlocked / OnWasBlocked fields of a weapon Blueprint (CDO chain over the C++ ctor values)
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct WeaponHitData {
    pub strike_hit: String,
    pub stab_hit: String,
    pub environment_hit: String,
    pub blocked: String,
    pub was_blocked: String,
    pub hit_cancel: String,
    pub blocked_view_target_sweetener: String,
    /// SecondStrikeHitSound / SecondStabHitSound / SecondEnvironmentHitSound (swapped in by SwitchMode)
    pub second_strike_hit: String,
    pub second_stab_hit: String,
    pub second_environment_hit: String,
    pub env_pitch_in: (f32, f32),
    pub env_pitch_out: (f32, f32),
    pub env_volume_in: (f32, f32),
    pub env_volume_out: (f32, f32),
}

/// one sound to start: cue package, cue parameters, the component's volume / pitch multipliers
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct HitCue {
    pub cue: String,
    pub params: Vec<(String, f64)>,
    pub volume: f64,
    pub pitch: f64,
}

impl HitCue {
    pub fn param_map(&self) -> HashMap<String, f64> {
        self.params.iter().cloned().collect()
    }
}

fn v2(d: &serde_json::Map<String, serde_json::Value>, k: &str, dflt: (f32, f32)) -> (f32, f32) {
    match d.get(k) {
        Some(serde_json::Value::Object(o)) => (
            o.get("X").and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(dflt.0),
            o.get("Y").and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(dflt.1),
        ),
        _ => dflt,
    }
}

impl WeaponHitData {
    pub fn from_defaults(d: &serde_json::Map<String, serde_json::Value>) -> WeaponHitData {
        let g = |k: &str| mh_assets::material::ue_pkg_path(d.get(k).unwrap_or(&serde_json::Value::Null));
        WeaponHitData {
            strike_hit: g("StrikeHitSound"),
            stab_hit: g("StabHitSound"),
            environment_hit: g("EnvironmentHitSound"),
            blocked: g("BlockedSound"),
            was_blocked: g("WasBlockedSound"),
            hit_cancel: g("HitCancelSound"),
            blocked_view_target_sweetener: g("BlockedViewTargetSweetener"),
            second_strike_hit: g("SecondStrikeHitSound"),
            second_stab_hit: g("SecondStabHitSound"),
            second_environment_hit: g("SecondEnvironmentHitSound"),
            // AMordhauWeapon ctor rva (AMordhauWeapon.cpp 2159-2166)
            env_pitch_in: v2(d, "EnvironmentPitchScaleByDamageIn", (25.0, 75.0)),
            env_pitch_out: v2(d, "EnvironmentPitchScaleByDamageOut", (1.1, 0.9)),
            env_volume_in: v2(d, "EnvironmentVolumeScaleByDamageIn", (10.0, 30.0)),
            env_volume_out: v2(d, "EnvironmentVolumeScaleByDamageOut", (0.9, 1.0)),
        }
    }
    /// the weapon in its alternate mode: AMordhauWeapon::SwitchMode_Implementation rva=0x1640a00 swaps StrikeHitSound,
    /// StabHitSound and EnvironmentHitSound with their Second* fields (mordhau-core Fighter.alternate_mode)
    pub fn switched(&self) -> WeaponHitData {
        let mut w = self.clone();
        std::mem::swap(&mut w.strike_hit, &mut w.second_strike_hit);
        std::mem::swap(&mut w.stab_hit, &mut w.second_stab_hit);
        std::mem::swap(&mut w.environment_hit, &mut w.second_environment_hit);
        w
    }
    pub fn read(rd: &mh_pak::Reader, weapon_bp: &str) -> WeaponHitData {
        WeaponHitData::from_defaults(&rd.defaults(weapon_bp))
    }
}

/// The attack's numbers OnHit reads (FAttackInfo Damage +0x60 / HeadBonus +0x70 / bStopOnHit +0x98)
#[derive(Clone, Debug, Default)]
pub struct AttackNums {
    pub damage: Vec<f32>,
    pub head_bonus: Vec<f32>,
    pub stop_on_hit: bool,
}

/// FMath::GetMappedRangeValueClamped's alpha as compiled in OnHit_Implementation (decomp 669-708): a 1e-8 degenerate
/// range gives 0 below In.Y, else 1
fn alpha(v: f32, i: (f32, f32)) -> f32 {
    let r = i.1 - i.0;
    if r.abs() > 1e-8 {
        ((v - i.0) / r).clamp(0.0, 1.0)
    } else if v < i.1 {
        0.0
    } else {
        1.0
    }
}

/// AMordhauWeapon::OnHit_Implementation rva=0x1631430: the two sounds (A then B). `hit_location`: 0 head / 1 body /
/// 2 leg (IsHead / IsLeg of the bone), `tier`: the victim's armour tier at the bone, `surface`: EPhysicalSurface
/// (1 = Flesh), `source_view_target`: the attacker is the view target.
pub fn on_hit(w: &WeaponHitData, mv: i64, hit_location: i64, tier: i64, surface: i64, ai: Option<&AttackNums>, source_view_target: bool) -> Vec<HitCue> {
    let mut out = Vec::new();
    let flesh = surface == 1;
    let a = if !flesh {
        &w.blocked
    } else if (2..=3).contains(&mv) {
        &w.stab_hit
    } else {
        &w.strike_hit
    };
    if !a.is_empty() {
        out.push(HitCue {
            cue: a.clone(),
            params: vec![("HitLocation".into(), hit_location as f64), ("ArmorTier".into(), tier as f64), ("SurfaceType".into(), surface as f64)],
            volume: 1.0,
            pitch: 1.0,
        });
    }
    let hl2 = if flesh { hit_location } else { 1 };
    let tier2 = if flesh { tier } else { 2 };
    let mut dmg = 0.0f32;
    if let Some(ai) = ai {
        if let Some(d) = ai.damage.get(tier2 as usize) {
            dmg = *d;
        }
        if hl2 == 0 {
            if let Some(h) = ai.head_bonus.get(tier2 as usize) {
                dmg += *h;
            }
        }
    }
    let dmg = dmg.abs();
    let ap = alpha(dmg, w.env_pitch_in);
    let av = alpha(dmg, w.env_volume_in);
    let vol = (w.env_volume_out.1 - w.env_volume_out.0) * av + w.env_volume_out.0;
    let pit = (w.env_pitch_out.1 - w.env_pitch_out.0) * ap + w.env_pitch_out.0;
    let is_stab = (2..=3).contains(&mv) && !ai.is_some_and(|a| a.stop_on_hit);
    if !w.environment_hit.is_empty() {
        out.push(HitCue {
            cue: w.environment_hit.clone(),
            params: vec![
                ("HitLocation".into(), hl2 as f64),
                ("ArmorTier".into(), tier2 as f64),
                ("SurfaceType".into(), surface as f64),
                ("IsStab".into(), if is_stab { 1.0 } else { 0.0 }),
                ("IsSourceViewTarget".into(), if source_view_target { 1.0 } else { 0.0 }),
            ],
            volume: vol as f64,
            pitch: pit as f64,
        });
    }
    out
}

/// OnWasBlocked's OnHit(null, Move, None, Point, 0, Surface) (rva=0x16340a0 -> 0x1631430): Bone None is body1.
/// The null-character branch uses PlayEquipmentSoundWorld (rva=0x155dd20), whose SpawnSoundAtLocation has literal
/// volume/pitch 1/1 (AMordhauEquipment.cpp 7951-7965). Only IsStab needs the selected weapon AttackInfo's stop flag;
/// ParentCharacter::IsViewTarget supplies source_view_target even for a world recipient.
pub fn on_world_hit(w: &WeaponHitData, mv: i64, surface: i64, stop_on_hit: bool, source_view_target: bool) -> Vec<HitCue> {
    let ai = AttackNums { stop_on_hit, ..Default::default() };
    let mut out = on_hit(w, mv, 1, 0, surface, Some(&ai), source_view_target);
    for cue in &mut out {
        cue.volume = 1.0;
        cue.pitch = 1.0;
    }
    out
}

/// EBlockedReason (mordhau-core enums::br): 0 Parry, 1 Chamber, 2 World, 3 Clash, 4 Hit
pub const BR_PARRY: i64 = 0;
pub const BR_CHAMBER: i64 = 1;
pub const BR_WORLD: i64 = 2;
pub const BR_CLASH: i64 = 3;
pub const BR_HIT: i64 = 4;

/// AMordhauWeapon::OnBlocked_Implementation rva=0x16306d0 (the defender's weapon)
pub fn on_blocked(w: &WeaponHitData, reason: i64, disarm: bool, defender_view_target: bool) -> Vec<HitCue> {
    let mut out = Vec::new();
    if !w.blocked.is_empty() {
        let r = if disarm {
            3
        } else if reason == BR_CHAMBER {
            1
        } else if reason == BR_CLASH {
            2
        } else {
            0
        };
        out.push(HitCue { cue: w.blocked.clone(), params: vec![("Reason".into(), r as f64)], volume: 1.0, pitch: 1.0 });
    }
    if defender_view_target && !w.blocked_view_target_sweetener.is_empty() {
        out.push(HitCue { cue: w.blocked_view_target_sweetener.clone(), params: vec![], volume: 1.0, pitch: 1.0 });
    }
    out
}

/// AMordhauWeapon::OnWasBlocked_Implementation rva=0x16340a0 (the attacker's weapon; decomp 1774-1800)
pub fn on_was_blocked(w: &WeaponHitData, reason: i64, is_cancel: bool) -> Vec<HitCue> {
    if is_cancel {
        return if w.hit_cancel.is_empty() { vec![] } else { vec![HitCue { cue: w.hit_cancel.clone(), params: vec![], volume: 1.0, pitch: 1.0 }] };
    }
    if reason == BR_HIT || reason == BR_WORLD || w.was_blocked.is_empty() {
        return vec![];
    }
    let r = if reason == BR_CHAMBER {
        1
    } else if reason == BR_CLASH {
        2
    } else {
        0
    };
    vec![HitCue { cue: w.was_blocked.clone(), params: vec![("Reason".into(), r as f64)], volume: 1.0, pitch: 1.0 }]
}

/// The blood HitEffect class a damaged character plays: BloodHitEffect when the bone's armour tier < 2 or the victim is
/// the view target, else BloodMetalHitEffect (AMordhauCharacter::OnTookDamage_Implementation rva=0x155be10, decomp
/// AMordhauCharacter.cpp 5232-5240). true = the metal (armoured) effect.
pub fn metal_hit_effect(tier: i64, victim_view_target: bool) -> bool {
    !(tier < 2 || victim_view_target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w() -> WeaponHitData {
        WeaponHitData::from_defaults(&serde_json::Map::from_iter([
            ("StrikeHitSound".to_string(), serde_json::json!("/Game/S/StrikeHit.StrikeHit")),
            ("StabHitSound".to_string(), serde_json::json!("/Game/S/StabHit.StabHit")),
            ("EnvironmentHitSound".to_string(), serde_json::json!("/Game/S/Env.Env")),
            ("BlockedSound".to_string(), serde_json::json!("/Game/S/Blocked.Blocked")),
            ("WasBlockedSound".to_string(), serde_json::json!("/Game/S/WasBlocked.WasBlocked")),
            ("HitCancelSound".to_string(), serde_json::json!("/Game/S/Cancel.Cancel")),
        ]))
    }

    /// OnHit_Implementation rva=0x1631430 with the AMordhauWeapon ctor ranges: a stab to the head of a tier-1 victim,
    /// Damage[1] 30 + HeadBonus[1] 20 = 50 -> volume alpha clamp((50-10)/20) = 1 -> 1.0, pitch alpha (50-25)/50 = 0.5
    /// -> 1.1 + (0.9-1.1) * 0.5 = 1.0
    #[test]
    fn hit_sounds_and_params() {
        let ai = AttackNums { damage: vec![40.0, 30.0, 20.0, 10.0], head_bonus: vec![30.0, 20.0, 10.0, 5.0], stop_on_hit: false };
        let c = on_hit(&w(), 2, 0, 1, 1, Some(&ai), true);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].cue, "Mordhau/Content/S/StabHit");
        assert_eq!(c[0].param_map()["HitLocation"], 0.0);
        assert_eq!(c[0].param_map()["ArmorTier"], 1.0);
        assert_eq!(c[0].param_map()["SurfaceType"], 1.0);
        assert_eq!((c[0].volume, c[0].pitch), (1.0, 1.0));
        assert_eq!(c[1].cue, "Mordhau/Content/S/Env");
        assert!((c[1].volume - 1.0).abs() < 1e-6, "{}", c[1].volume);
        assert!((c[1].pitch - 1.0).abs() < 1e-6, "{}", c[1].pitch);
        assert_eq!(c[1].param_map()["IsStab"], 1.0);
        assert_eq!(c[1].param_map()["IsSourceViewTarget"], 1.0);
        // a strike to the body of a tier-3 victim: Damage[3] 10 -> volume 0.9, pitch alpha 0 -> 1.1
        let c = on_hit(&w(), 1, 1, 3, 1, Some(&ai), false);
        assert_eq!(c[0].cue, "Mordhau/Content/S/StrikeHit");
        assert!((c[1].volume - 0.9).abs() < 1e-6 && (c[1].pitch - 1.1).abs() < 1e-6);
        assert_eq!(c[1].param_map()["IsStab"], 0.0);
        // a non-flesh surface: BlockedSound, and B uses tier' 2 / HitLocation' 1 (no head bonus)
        let c = on_hit(&w(), 0, 0, 0, 3, Some(&ai), false);
        assert_eq!(c[0].cue, "Mordhau/Content/S/Blocked");
        assert_eq!(c[1].param_map()["ArmorTier"], 2.0);
        assert_eq!(c[1].param_map()["HitLocation"], 1.0);
        // bStopOnHit stabs are not IsStab
        let ai2 = AttackNums { stop_on_hit: true, ..ai };
        assert_eq!(on_hit(&w(), 3, 1, 0, 1, Some(&ai2), false)[1].param_map()["IsStab"], 0.0);
    }

    #[test]
    fn block_reasons() {
        assert_eq!(on_blocked(&w(), BR_PARRY, false, false)[0].params, vec![("Reason".to_string(), 0.0)]);
        assert_eq!(on_blocked(&w(), BR_CHAMBER, false, false)[0].params, vec![("Reason".to_string(), 1.0)]);
        assert_eq!(on_blocked(&w(), BR_CLASH, false, false)[0].params, vec![("Reason".to_string(), 2.0)]);
        assert_eq!(on_blocked(&w(), BR_PARRY, true, false)[0].params, vec![("Reason".to_string(), 3.0)]);
        assert_eq!(on_was_blocked(&w(), BR_CHAMBER, false)[0].cue, "Mordhau/Content/S/WasBlocked");
        assert!(on_was_blocked(&w(), BR_HIT, false).is_empty());
        assert!(on_was_blocked(&w(), BR_WORLD, false).is_empty());
        assert_eq!(on_was_blocked(&w(), BR_PARRY, true)[0].cue, "Mordhau/Content/S/Cancel");
        assert!(metal_hit_effect(2, false) && !metal_hit_effect(1, false) && !metal_hit_effect(3, true));
        // SwitchMode_Implementation rva=0x1640a00 swaps the hit sounds with the Second* ones
        let mut a = w();
        a.second_strike_hit = "Mordhau/Content/S/Strike2".into();
        let b = a.switched();
        assert_eq!(b.strike_hit, "Mordhau/Content/S/Strike2");
        assert_eq!(b.second_strike_hit, "Mordhau/Content/S/StrikeHit");
    }

    /// Native PlayEquipmentSoundWorld uses fixed 1/1, regardless of the character-hit damage mapping defaults.
    /// This also checks the null-recipient/nonflesh parameter distinction and both stab sides.
    #[test]
    fn world_contact_sound_multipliers_and_surface_params() {
        let mut data = w();
        data.env_volume_out = (0.25, 0.5);
        data.env_pitch_out = (1.8, 1.6);
        for mv in [0, 1, 2, 3, 4] {
            let c = on_world_hit(&data, mv, 2, false, false);
            assert_eq!(c.len(), 2);
            assert_eq!(c[0].cue, "Mordhau/Content/S/Blocked");
            assert_eq!(c[1].cue, "Mordhau/Content/S/Env");
            for cue in &c {
                assert_eq!((cue.volume, cue.pitch), (1.0, 1.0));
                assert_eq!(cue.param_map()["HitLocation"], 1.0);
                assert_eq!(cue.param_map()["SurfaceType"], 2.0);
            }
            assert_eq!(c[0].param_map()["ArmorTier"], 0.0);
            assert_eq!(c[1].param_map()["ArmorTier"], 2.0);
            assert_eq!(c[1].param_map()["IsStab"], if mv == 2 || mv == 3 { 1.0 } else { 0.0 });
            assert_eq!(c[1].param_map()["IsSourceViewTarget"], 0.0);
        }
        // Character recipients retain their native damage mapping; a 75-damage flesh hit gives pitch0.9.
        let ai = AttackNums { damage: vec![75.0; 4], ..Default::default() };
        let character = on_hit(&w(), 0, 1, 0, 1, Some(&ai), false);
        assert!((character[1].pitch - 0.9).abs() < 1e-6);
    }

    /// A null recipient does not imply a nonflesh surface. Parent view-target and selected attack stop flags
    /// remain independent, including switched cue names; no invented IsStopOnHit parameter is sent.
    #[test]
    fn world_contact_preserves_stab_stop_and_view_target() {
        let mut data = w();
        data.second_stab_hit = "Mordhau/Content/S/Stab2".into();
        data.second_environment_hit = "Mordhau/Content/S/Env2".into();
        let data = data.switched();
        for mv in [2, 3] {
            for stop in [false, true] {
                for view_target in [false, true] {
                    let c = on_world_hit(&data, mv, 1, stop, view_target);
                    assert_eq!(c[0].cue, "Mordhau/Content/S/Stab2");
                    assert_eq!(c[1].cue, "Mordhau/Content/S/Env2");
                    let p = c[1].param_map();
                    assert_eq!(p["ArmorTier"], 0.0);
                    assert_eq!(p["HitLocation"], 1.0);
                    assert_eq!(p["SurfaceType"], 1.0);
                    assert_eq!(p["IsStab"], if stop { 0.0 } else { 1.0 });
                    assert_eq!(p["IsSourceViewTarget"], if view_target { 1.0 } else { 0.0 });
                    assert!(!p.contains_key("IsStopOnHit"));
                    assert_eq!((c[1].volume, c[1].pitch), (1.0, 1.0));
                }
            }
        }
    }
}

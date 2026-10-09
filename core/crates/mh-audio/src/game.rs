//! Character sound triggers as the shipped game plays them (Mordhau-Win64-Shipping.exe rvas; data from the
//! Blueprint CDOs). Hosts (mh-runtime bridge.rs) turn sim events / animation notifies into `Trigger`s and send the
//! returned `PlayCue`s. Coverage by category (the full list with status is in docs/RUST_RUNTIME.md section 11):
//!
//! - Whoosh + release foley: UAttackMotion::OnTick_Implementation 0x16328c0, once per attack on its first release tick (flag
//!   +0x1071): weapon vtable +0x7f0 = AMordhauWeapon::StartWoosh 0x1640170 with WooshTimeFactor (UAttackMotion ctor
//!   0x1612bae: 0.5) x the motion's release duration (+0xe54; its identity as release duration UNCONFIRMED);
//!   StartWoosh plays StrikeWooshSound, or StabWooshSound for attack types 2 / 3 (+0xda8 - 2 <= 1), at 0.75 along the
//!   weapon trace from its start (0x141640224), x WooshVolumeMultiplierViewTarget when locally viewed, with the cue
//!   float parameter "ReleaseTime"; then the character's ReleaseFoley (+0x1250, PlayCharacterSound vtable +0x908)
//!   with the same "ReleaseTime".
//! - Armour foley: AMordhauCharacter::PlayArmorFoley 0x155d920 (fades the last foley out over 0.1 s, plays the cue
//!   attached, int parameter = GetArmorTierForBone): NonSnappyArmorFoley (+0x1238) on UAttackMotion /
//!   UDropEquipmentMotion / UEquipmentSwitchMotion / ... OnBegin and UFlinchMotion::OnTick; SnappyArmorFoley (+0x1230)
//!   on UParryMotion / UFeintedMotion OnBegin; CrouchStartSound / CrouchEndSound on OnStartCrouch / OnEndCrouch.
//! - Footsteps: anim notifies LeftFootLanded / RightFootLanded on the locomotion clips (42 each) -> DoFootstep ->
//!   AAdvancedCharacter::PlayFootstepSound 0x1498960: FootstepSound at the FeetBones socket + FootstepSoundZOffset,
//!   volume = GetMappedRangeValueClamped(speed, VolumeRangeIn, Out) (a zero-width range is a step at In.Y) x
//!   ViewTarget / Ally / Enemy modifier (IsFriendly with the view target), pitch likewise, int params Surface,
//!   ArmorTier (FootstepArmorTier +0x768), foot; bool params Crouched, IsViewTarget; the view target uses
//!   ViewTargetAttenuationOverride.
//! - Voice: UCharacterVoiceComponent::PlayAttackYell 0x14982b0 (voice pack AttackYell, x
//!   AttackYellVolumeMultiplierViewTarget when locally viewed) from URangedReleaseMotion::OnBegin and Blueprint calls;
//!   PlayHurtYell from UDisarmedMotion::OnBegin and Blueprint; OnTakeDamage 0x1492150; PlayDeathYell 0x14987a0
//!   (Blueprint). Which Blueprint events call them (and with what chance) is UNCONFIRMED: hosts send AttackYell on
//!   melee attack begin, Hurt on damage, Death on death until the event graph is ported.

use crate::sources::{Footsteps, Relation, VoicePack};
use crate::PlayCue;
use bevy::prelude::Vec3;
use mh_pak::Reader;
use serde_json::Value;

fn path(v: Option<&Value>) -> String {
    mh_assets::material::strip_index(&mh_assets::material::ue_pkg_path(v.unwrap_or(&Value::Null))).to_string()
}

/// BP_MordhauCharacter CDO sounds
#[derive(Clone, Debug, Default)]
pub struct CharacterSounds {
    pub snappy_foley: String,
    pub non_snappy_foley: String,
    pub release_foley: String,
    pub crouch_start: String,
    pub crouch_end: String,
    pub dodge: String,
    pub fall_damage: String,
    pub view_target_attenuation: Option<mh_assets::sound_cue::Attenuation>,
    pub footsteps: Footsteps,
}

impl CharacterSounds {
    pub fn read(rd: &Reader) -> CharacterSounds {
        let d = rd.defaults(crate::sources::CHARACTER_BP);
        let vta = path(d.get("ViewTargetAttenuationOverride"));
        CharacterSounds {
            snappy_foley: path(d.get("SnappyArmorFoley")),
            non_snappy_foley: path(d.get("NonSnappyArmorFoley")),
            release_foley: path(d.get("ReleaseFoley")),
            crouch_start: path(d.get("CrouchStartSound")),
            crouch_end: path(d.get("CrouchEndSound")),
            dodge: path(d.get("DodgeSound")),
            fall_damage: path(d.get("FallDamageSound")),
            view_target_attenuation: (!vta.is_empty()).then(|| mh_assets::sound_cue::attenuation(rd, &vta)).flatten(),
            footsteps: Footsteps::read(rd),
        }
    }
}

/// AMordhauWeapon woosh data (CDO chain)
#[derive(Clone, Debug, Default)]
pub struct WeaponWoosh {
    pub strike: String,
    pub stab: String,
    pub view_target_mult: f64,
}

impl WeaponWoosh {
    pub fn read(rd: &Reader, weapon_bp: &str) -> WeaponWoosh {
        let d = rd.defaults(weapon_bp);
        WeaponWoosh {
            strike: path(d.get("StrikeWooshSound")),
            stab: path(d.get("StabWooshSound")),
            view_target_mult: d.get("WooshVolumeMultiplierViewTarget").and_then(Value::as_f64).unwrap_or(1.0),
        }
    }
}

/// UAttackMotion ctor 0x1612bae
pub const WOOSH_TIME_FACTOR: f64 = 0.5;
/// AMordhauWeapon::StartWoosh 0x141640224: the woosh source sits 0.75 along the trace
pub const WOOSH_TRACE_FRACTION: f32 = 0.75;

/// one sound-worthy moment of a character
#[derive(Clone, Debug)]
pub enum Trigger {
    /// the attack's first release tick: trace start / end (Bevy m), stab (attack type 2 / 3), release duration (s)
    Release { trace_start: Vec3, trace_end: Vec3, stab: bool, release_duration: f64 },
    /// motion OnBegin (sim "motion" event kind: Attack, Parry, Feinted, Flinch, ...)
    MotionBegin { kind: String },
    CrouchStart,
    CrouchEnd,
    /// LeftFootLanded (0) / RightFootLanded (1) notify; surface = EPhysicalSurface under the foot; speed cm/s
    FootLanded { foot: usize, surface: usize, speed_cm_s: f64, crouched: bool, foot_pos: Vec3 },
    AttackYell,
    Hurt,
    Death,
    Dodge,
    FallDamage,
}

/// a character's sound context
#[derive(Clone, Debug)]
pub struct Speaker {
    pub pos: Vec3,
    pub weapon: WeaponWoosh,
    pub voice: Option<VoicePack>,
    pub armor_tier: i64,
    /// is this the character the camera views
    pub view_target: bool,
    /// friendly to the view target
    pub friendly: bool,
}

/// the cues a trigger plays (empty when the game plays none)
pub fn cues(c: &CharacterSounds, s: &Speaker, t: &Trigger) -> Vec<PlayCue> {
    let at = |cue: &str, pos: Vec3| PlayCue { cue: cue.to_string(), pos, ..Default::default() };
    let mut out = vec![];
    match t {
        Trigger::Release { trace_start, trace_end, stab, release_duration } => {
            let pos = *trace_start + (*trace_end - *trace_start) * WOOSH_TRACE_FRACTION;
            let rt = WOOSH_TIME_FACTOR * release_duration;
            let w = if *stab { &s.weapon.stab } else { &s.weapon.strike };
            if !w.is_empty() {
                let mut p = at(w, pos);
                p.params.insert("ReleaseTime".into(), rt);
                if s.view_target {
                    p.volume = s.weapon.view_target_mult;
                }
                out.push(p);
            }
            if !c.release_foley.is_empty() {
                let mut p = at(&c.release_foley, s.pos);
                p.params.insert("ReleaseTime".into(), rt);
                out.push(p);
            }
        }
        Trigger::MotionBegin { kind } => {
            // armour foley per motion class (callers of PlayNonSnappy / PlaySnappyArmorFoley)
            let f = match kind.as_str() {
                "Attack" | "Flinch" | "RangedDraw" | "Reload" | "RangedCancel" => &c.non_snappy_foley,
                "Parry" | "Feinted" => &c.snappy_foley,
                _ => return out,
            };
            if !f.is_empty() {
                let mut p = at(f, s.pos);
                p.params.insert("ArmorTier".into(), s.armor_tier as f64);
                out.push(p);
            }
        }
        Trigger::CrouchStart | Trigger::CrouchEnd => {
            let f = if matches!(t, Trigger::CrouchStart) { &c.crouch_start } else { &c.crouch_end };
            if !f.is_empty() {
                let mut p = at(f, s.pos);
                p.params.insert("ArmorTier".into(), s.armor_tier as f64);
                out.push(p);
            }
        }
        Trigger::FootLanded { foot, surface, speed_cm_s, crouched, foot_pos } => {
            let rel = if s.view_target {
                Relation::ViewTarget
            } else if s.friendly {
                Relation::Ally
            } else {
                Relation::Enemy
            };
            let (mut params, vol, pitch) = c.footsteps.step(*surface, *speed_cm_s, s.armor_tier, *crouched, rel);
            params.insert("Foot".into(), *foot as f64);
            if !c.footsteps.cue.is_empty() {
                let mut p = at(&c.footsteps.cue, *foot_pos);
                p.params = params;
                p.volume = vol;
                p.pitch = pitch;
                if s.view_target {
                    p.attenuation = c.view_target_attenuation.clone();
                }
                out.push(p);
            }
        }
        Trigger::AttackYell | Trigger::Hurt | Trigger::Death => {
            let ev = match t {
                Trigger::AttackYell => "AttackYell",
                Trigger::Hurt => "Hurt",
                _ => "Death",
            };
            if let Some(cue) = s.voice.as_ref().and_then(|v| v.cue(ev)) {
                out.push(at(cue, s.pos));
            }
        }
        Trigger::Dodge => {
            if !c.dodge.is_empty() {
                out.push(at(&c.dodge, s.pos))
            }
        }
        Trigger::FallDamage => {
            if !c.fall_damage.is_empty() {
                out.push(at(&c.fall_damage, s.pos))
            }
        }
    }
    out
}

/// Out-of-breath sound: UStaminaStatComponent::TickStat (0x1519900, the breathing part 0x15199ab..0x1519a2f) plays
/// the voice pack's Breathing cue (UCharacterVoiceComponent::PlayBreathingSound 0x1498390 -> PlayMouthSound, volume 1)
/// when bPlaysOutOfBreathSound, stamina < BreathingSoundPlayBelowStamina and the character's view distance (+0x80c)
/// < BreathingSoundMaxDistance, and the previous breath (LastBreath) is no longer playing. Defaults from the
/// UStaminaStatComponent ctor 0x14e6fc0: bPlaysOutOfBreathSound true, 16, 1500 cm. (+0x80c as the distance to the
/// view and the folded check at 0x15199e7 are UNCONFIRMED.)
pub const BREATH_BELOW_STAMINA: i32 = 16;
pub const BREATH_MAX_DISTANCE_CM: f32 = 1500.0;

pub fn breathing_due(stamina: i32, view_distance_cm: f32, last_breath_playing: bool) -> bool {
    !last_breath_playing && stamina < BREATH_BELOW_STAMINA && BREATH_MAX_DISTANCE_CM > view_distance_cm
}

/// Voice commands: UCharacterVoiceComponent::PerformVoiceCommand 0x1497c30 plays the pack's VoiceCommands cue as a
/// mouth sound (volume 1) with int parameters "Type" = command & 0xF and "SubType" = command >> 4 (0x1497d9d /
/// 0x1497dc3); nothing plays when the user's master x voice volume <= 6e-5 (0x1497d7e). Type 10 also plays
/// FaceBattlecryAnimation.
pub fn voice_command(voice: &VoicePack, command: u8, pos: Vec3) -> Option<PlayCue> {
    let cue = voice.cue("VoiceCommands")?;
    let mut p = PlayCue { cue: cue.to_string(), pos, ..Default::default() };
    p.params.insert("Type".into(), (command & 0xf) as f64);
    p.params.insert("SubType".into(), (command >> 4) as f64);
    Some(p)
}

/// a sound an animation plays at a notify (AnimNotify_PlaySound, or a Blueprint notify such as PlayCharacterSound):
/// FAnimNotifyEvent LinkValue (s) and the notify object's Sound / VolumeMultiplier / PitchMultiplier / AttachName
#[derive(Clone, Debug, PartialEq)]
pub struct NotifySound {
    pub time: f64,
    pub notify: String,
    pub sound: String,
    pub volume: f64,
    pub pitch: f64,
    pub attach: String,
}

/// every sound notify of an animation package (sequence or montage)
pub fn anim_notify_sounds(rd: &Reader, anim_pkg: &str) -> Vec<NotifySound> {
    let Some(ex) = rd.read(anim_pkg) else { return vec![] };
    let mut out = vec![];
    for e in &ex {
        let Some(ns) = e.pointer("/Properties/Notifies").and_then(Value::as_array) else { continue };
        for n in ns {
            let Some(r) = n.get("Notify").and_then(|o| o.get("ObjectName")).and_then(Value::as_str) else { continue };
            let name = r.rsplit([':', '.']).next().unwrap_or("").trim_end_matches('\'');
            let Some(o) = ex.iter().find(|x| x.get("Name").and_then(Value::as_str) == Some(name)) else { continue };
            let p = o.get("Properties").cloned().unwrap_or(Value::Null);
            let sound = path(p.get("Sound"));
            if sound.is_empty() {
                continue;
            }
            out.push(NotifySound {
                time: n.get("LinkValue").and_then(Value::as_f64).unwrap_or(0.0),
                notify: o.get("Type").and_then(Value::as_str).unwrap_or("").to_string(),
                sound,
                volume: p.get("VolumeMultiplier").and_then(Value::as_f64).unwrap_or(1.0),
                pitch: p.get("PitchMultiplier").and_then(Value::as_f64).unwrap_or(1.0),
                attach: p.get("AttachName").and_then(Value::as_str).unwrap_or("").to_string(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the character's Blueprint sounds resolve, a release plays the longsword's strike woosh 0.75 along the trace
    /// plus the release foley, parries play the snappy foley, footsteps the human footstep cue
    #[test]
    fn character_triggers() {
        let rd = mh_pak::Reader::new(std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks")));
        let c = CharacterSounds::read(&rd);
        assert!(c.snappy_foley.ends_with("SC_SnappyArmorFoley") && c.release_foley.ends_with("SC_ReleaseFoley"), "{c:?}");
        let w = WeaponWoosh::read(&rd, "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword");
        assert!(!w.strike.is_empty() && !w.stab.is_empty(), "{w:?}");
        let s = Speaker { pos: Vec3::ZERO, weapon: w.clone(), voice: None, armor_tier: 1, view_target: false, friendly: false };
        let r = cues(&c, &s, &Trigger::Release { trace_start: Vec3::ZERO, trace_end: Vec3::new(1.0, 0.0, 0.0), stab: false, release_duration: 0.4 });
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].cue, w.strike);
        assert!((r[0].pos.x - 0.75).abs() < 1e-6 && (r[0].params["ReleaseTime"] - 0.2).abs() < 1e-9);
        let p = cues(&c, &s, &Trigger::MotionBegin { kind: "Parry".into() });
        assert_eq!(p[0].cue, c.snappy_foley);
        let f = cues(&c, &s, &Trigger::FootLanded { foot: 0, surface: 6, speed_cm_s: 300.0, crouched: false, foot_pos: Vec3::ZERO });
        assert!(f[0].cue.ends_with("SC_HumanFootstep") && f[0].params["Surface"] == 6.0);
        assert!((f[0].volume - 1.05 * c.footsteps.mod_enemy).abs() < 1e-9, "{}", f[0].volume);
        // emote notify sounds (Salute: AnimNotify_PlaySound SC_EquipArmorT2 x 0.352381, pitch 1.554916)
        let n = anim_notify_sounds(&rd, "Mordhau/Content/Mordhau/Animations/RawClips/Misc/Emotes/New/Salute");
        assert!(n.iter().any(|x| x.sound.ends_with("SC_EquipArmorT2") && (x.volume - 0.352381).abs() < 1e-6), "{n:?}");
        assert!(breathing_due(10, 500.0, false) && !breathing_due(16, 500.0, false) && !breathing_due(10, 1500.0, false));
    }
}

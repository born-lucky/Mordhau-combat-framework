//! Bot hearing (rust-mode-ai r6): the character sounds that the shipped game routes through
//! AMordhauCharacter::PlayCharacterSound rva=0x155dac0 (vtable +0x908) become noise events for the bots
//! (mh_mode::ai::Bots::report_noise): location = the character's root, Loudness = the cue's VolumeMultiplier
//! (USoundBase vtable +0x2a0 = USoundCue::GetVolumeMultiplier, the slot read from the exe's USoundCue vtable; a cue
//! that does not serialize VolumeMultiplier has the USoundCue ctor's 0.75, rva=0x3450830: most of the foley cues do
//! not, e.g. SC_SnappyArmorFoley / SC_ReleaseFoley / SC_CrouchStart / SC_Dodge; SC_NonSnappyArmorFoley says 1.0).
//! Every caller of vtable +0x908 in the decomp, and its mapping to this sim's events:
//!   AMordhauCharacter::PlayArmorFoley rva=0x155d920 (BP_MordhauCharacter CDO cues): NonSnappyArmorFoley on attack /
//!     equipment switch / drop begins and flinches, SnappyArmorFoley on parry / feinted begins, CrouchStartSound /
//!     CrouchEndSound on crouch start / end                                         -> "motion" / "crouch_*"
//!   UAttackMotion::OnTick_Implementation rva=0x16328c0: ReleaseFoley on the first release tick       -> "release"
//!   AMordhauCharacter::OnDodged rva=0x1551be0: DodgeSound                                            -> "dodge"
//!   AAdvancedCharacter::OnTookDamage_Implementation rva=0x14926a0: only the Fall branch plays through +0x908
//!     (FallDamageSound; the Melee / Ranged branches play no character sound there)               -> "fall_damage"
//!   UEquipmentSwitchMotion::OnTick_Implementation rva=0x16681b0: the new equipment's EquipSound (AMordhauEquipment
//!     +0xc80), at the motion's switch point (here: at its begin, UNCONFIRMED timing)      -> motion EquipmentSwitch
//!   UReloadMotion::OnBegin_Implementation rva=0x1662640: the ranged equipment's ReloadSound (+0xa78)  -> Reload
//!   URangedDrawMotion::OnTick_Implementation rva=0x16694b0: DrawSound (+0xc90) at DrawSoundPlayAtNormalizedTime
//!     (here: at the draw's begin, UNCONFIRMED timing)                                            -> RangedDraw
//!   URangedCancelMotion::OnBegin_Implementation rva=0x1661cc0: RangedCancelSound (+0xca0)            -> RangedCancel
//!   UClimbingMotion::OnBegin_Implementation rva=0x165e550: ClimbSound, which neither the ctor nor BP_ClimbingMotion's
//!     CDO sets (null): no noise
//! Not mapped (no event in this sim): burning (UCharacterBurnableComponent / UBurnableComponent
//! StartBurningCosmetic), dismemberment (UDismemberableComponent::QueueDismember), UHealthStatComponent::TickStat,
//! AMordhauEquipment::SetAmmo / OnRep_Ammo, vehicles, Blueprint BP_PlayCharacterSound calls. Footsteps
//! (AAdvancedCharacter::PlayFootstepSound rva=0x1498960) and voice yells (UCharacterVoiceComponent) do not go through
//! +0x908: they make no AI noise.

use mh_pak::Reader;
use std::collections::HashMap;

pub const CHARACTER_BP: &str = "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter";
/// USoundCue ctor rva=0x3450830: VolumeMultiplier 0.75
pub const CUE_DEFAULT_VOLUME: f32 = 0.75;

/// the loudness (cue VolumeMultiplier) of each noise-making character sound
#[derive(Clone, Debug)]
pub struct NoiseLoudness {
    /// the character's own sounds (BP_MordhauCharacter CDO): snappy / non_snappy / release / crouch_start /
    /// crouch_end / dodge / fall_damage
    pub by_kind: HashMap<&'static str, f32>,
    /// (equipment class path, kind) -> loudness for the equipment's sounds: equip / reload / draw / cancel. Absent =
    /// that equipment has no such cue (no noise)
    pub by_weapon: HashMap<(String, &'static str), f32>,
}

impl Default for NoiseLoudness {
    /// every character sound at the USoundCue ctor default (until `read` gives the real cues); no equipment sounds
    fn default() -> Self {
        let mut by_kind = HashMap::new();
        for k in ["snappy", "non_snappy", "release", "crouch_start", "crouch_end", "dodge", "fall_damage"] {
            by_kind.insert(k, CUE_DEFAULT_VOLUME);
        }
        NoiseLoudness { by_kind, by_weapon: HashMap::new() }
    }
}

fn cue_path(d: &serde_json::Map<String, serde_json::Value>, prop: &str) -> String {
    mh_assets::material::strip_index(&mh_assets::material::ue_pkg_path(d.get(prop).unwrap_or(&serde_json::Value::Null))).to_string()
}

impl NoiseLoudness {
    /// BP_MordhauCharacter's CDO sound properties -> their cues' VolumeMultiplier (mh-assets sound_cue)
    pub fn read(rd: &Reader) -> NoiseLoudness {
        Self::read_with_weapons(rd, &[])
    }

    /// `read` plus each equipment class's EquipSound / ReloadSound / DrawSound / RangedCancelSound
    pub fn read_with_weapons(rd: &Reader, weapons: &[&str]) -> NoiseLoudness {
        let d = rd.defaults(CHARACTER_BP);
        let mut out = NoiseLoudness::default();
        for (k, prop) in [
            ("snappy", "SnappyArmorFoley"),
            ("non_snappy", "NonSnappyArmorFoley"),
            ("release", "ReleaseFoley"),
            ("crouch_start", "CrouchStartSound"),
            ("crouch_end", "CrouchEndSound"),
            ("dodge", "DodgeSound"),
            ("fall_damage", "FallDamageSound"),
        ] {
            let path = cue_path(&d, prop);
            match (path.is_empty(), mh_assets::sound_cue::cue(rd, &path)) {
                (true, _) => {
                    out.by_kind.remove(k);
                }
                (false, Some(c)) => {
                    out.by_kind.insert(k, c.volume as f32);
                }
                (false, None) => {}
            }
        }
        for w in weapons.iter().filter(|w| !w.is_empty()) {
            let d = rd.defaults(w);
            for (k, prop) in [("equip", "EquipSound"), ("reload", "ReloadSound"), ("draw", "DrawSound"), ("cancel", "RangedCancelSound")] {
                let path = cue_path(&d, prop);
                if path.is_empty() {
                    continue;
                }
                let v = mh_assets::sound_cue::cue(rd, &path).map(|c| c.volume as f32).unwrap_or(CUE_DEFAULT_VOLUME);
                out.by_weapon.insert((w.to_string(), k), v);
            }
        }
        out
    }

    /// the noises of one sim event (header mapping): (sound kind, from the fighter's equipment)
    pub fn kinds_of(ev: &serde_json::Map<String, serde_json::Value>) -> Vec<(&'static str, bool)> {
        let Some(kind) = ev.get("kind").and_then(|v| v.as_str()) else { return vec![] };
        match kind {
            "motion" => match ev.get("motion").and_then(|v| v.as_str()).unwrap_or("") {
                "Attack" | "DropEquipment" | "Flinch" => vec![("non_snappy", false)],
                "EquipmentSwitch" => vec![("non_snappy", false), ("equip", true)],
                "Parry" | "Feinted" => vec![("snappy", false)],
                "Reload" => vec![("reload", true)],
                "RangedDraw" => vec![("draw", true)],
                "RangedCancel" => vec![("cancel", true)],
                _ => vec![],
            },
            "release" => vec![("release", false)],
            "crouch_start" => vec![("crouch_start", false)],
            "crouch_end" => vec![("crouch_end", false)],
            "dodge" => vec![("dodge", false)],
            "fall_damage" => vec![("fall_damage", false)],
            _ => vec![],
        }
    }

    /// the loudness of a noise of `kind` made by a fighter holding `weapon` (None: that sound does not exist)
    pub fn loudness(&self, kind: &'static str, weapon_specific: bool, weapon: &str) -> Option<f32> {
        if weapon_specific {
            self.by_weapon.get(&(weapon.to_string(), kind)).copied()
        } else {
            self.by_kind.get(kind).copied()
        }
    }
}

//! Non-combat sound sources from the game's Blueprint data: footsteps (BP_MordhauCharacter CDO) and voice packs
//! (Blueprints/Voices/BP_* CDOs).
//!
//! Footsteps: FootstepSound = SC_HumanFootstep, whose graph switches on the cue parameters `Surface` (int:
//! EPhysicalSurface, DefaultEngine.ini PhysicalSurfaces: 1 Flesh, 2 Wood, 3 Metal, 4 Stone, 5 Dirt, 6 Grass,
//! 7 Pebbles, 8 Wet, 9 Sand, 10 Snow; 0 Default), `ArmorTier` (int), `Crouched` and `IsViewTarget` (bool). The CDO's
//! velocity ranges map the character speed to volume / pitch (FootstepVolumeVelocityRangeIn -> Out,
//! FootstepPitchVelocityRangeIn -> Out), times FootstepVolumeModifierViewTarget (locally viewed) / Ally (IsFriendly
//! with the view target) / Enemy, as AAdvancedCharacter::PlayFootstepSound 0x1498960 does (0x1498b3b / 0x1498bb7 /
//! 0x1498bc1; the volume also x the footstep perk's factor, perk 7, not modelled).
//!
//! Voice packs: the CDO's cue per event (AttackYell, Hurt, Death, Breathing, Screaming, VoiceCommands) and
//! PitchLimits (a per-character pitch between X and Y: how it is picked is UNCONFIRMED).

use mh_pak::Reader;
use serde_json::Value;
use std::collections::HashMap;

pub const SURFACES: [&str; 11] = ["Default", "Flesh", "Wood", "Metal", "Stone", "Dirt", "Grass", "Pebbles", "Wet", "Sand", "Snow"];
pub const CHARACTER_BP: &str = "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter";

fn v2(d: &serde_json::Map<String, Value>, k: &str) -> [f64; 2] {
    let g = |c: &str| d.get(k).and_then(|v| v.get(c)).and_then(Value::as_f64).unwrap_or(0.0);
    [g("X"), g("Y")]
}
fn path(v: Option<&Value>) -> String {
    mh_assets::material::strip_index(&mh_assets::material::ue_pkg_path(v.unwrap_or(&Value::Null))).to_string()
}

/// FMath::GetMappedRangeValueClamped as AAdvancedCharacter::PlayFootstepSound 0x1498a4a inlines it: |In.Y - In.X|
/// <= 1e-8 -> a step (x >= In.Y -> 1, else 0), else the clamped fraction; then lerp(Out.X, Out.Y)
pub fn map_clamped(x: f64, i: [f64; 2], o: [f64; 2]) -> f64 {
    let t = if (i[1] - i[0]).abs() > 1e-8 { ((x - i[0]) / (i[1] - i[0])).clamp(0.0, 1.0) } else if x >= i[1] { 1.0 } else { 0.0 };
    o[0] + t * (o[1] - o[0])
}

#[derive(Clone, Debug, Default)]
pub struct Footsteps {
    pub cue: String,
    pub volume_in: [f64; 2],
    pub volume_out: [f64; 2],
    pub pitch_in: [f64; 2],
    pub pitch_out: [f64; 2],
    pub mod_view_target: f64,
    pub mod_ally: f64,
    pub mod_enemy: f64,
    /// FootstepParticles[surface] (dust per surface; empty = none)
    pub particles: Vec<String>,
}

/// who hears the step: the view target itself, an ally, an enemy
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    ViewTarget,
    Ally,
    Enemy,
}

impl Footsteps {
    pub fn read(rd: &Reader) -> Footsteps {
        let d = rd.defaults(CHARACTER_BP);
        let f = |k: &str| d.get(k).and_then(Value::as_f64).unwrap_or(1.0);
        Footsteps {
            cue: path(d.get("FootstepSound")),
            volume_in: v2(&d, "FootstepVolumeVelocityRangeIn"),
            volume_out: v2(&d, "FootstepVolumeVelocityRangeOut"),
            pitch_in: v2(&d, "FootstepPitchVelocityRangeIn"),
            pitch_out: v2(&d, "FootstepPitchVelocityRangeOut"),
            mod_view_target: f("FootstepVolumeModifierViewTarget"),
            mod_ally: f("FootstepVolumeModifierAlly"),
            mod_enemy: f("FootstepVolumeModifierEnemy"),
            particles: d.get("FootstepParticles").and_then(Value::as_array).map(|a| a.iter().map(|x| path(Some(x))).collect()).unwrap_or_default(),
        }
    }

    /// the cue parameters, volume and pitch multipliers of one step
    pub fn step(&self, surface: usize, speed_cm_s: f64, armor_tier: i64, crouched: bool, rel: Relation) -> (HashMap<String, f64>, f64, f64) {
        let mut p = HashMap::new();
        p.insert("Surface".to_string(), surface as f64);
        p.insert("ArmorTier".to_string(), armor_tier as f64);
        p.insert("Crouched".to_string(), if crouched { 1.0 } else { 0.0 });
        p.insert("IsViewTarget".to_string(), if rel == Relation::ViewTarget { 1.0 } else { 0.0 });
        let m = match rel {
            Relation::ViewTarget => self.mod_view_target,
            Relation::Ally => self.mod_ally,
            Relation::Enemy => self.mod_enemy,
        };
        (p, map_clamped(speed_cm_s, self.volume_in, self.volume_out) * m, map_clamped(speed_cm_s, self.pitch_in, self.pitch_out))
    }
}

#[derive(Clone, Debug, Default)]
pub struct VoicePack {
    pub bp: String,
    pub name: String,
    /// event -> cue (AttackYell, Hurt, Death, Breathing, Screaming, VoiceCommands)
    pub cues: Vec<(String, String)>,
    pub pitch_limits: [f64; 2],
}

pub const VOICE_EVENTS: [&str; 6] = ["AttackYell", "Hurt", "Death", "Breathing", "Screaming", "VoiceCommands"];

impl VoicePack {
    pub fn read(rd: &Reader, bp: &str) -> VoicePack {
        let d = rd.defaults(bp);
        VoicePack {
            bp: bp.to_string(),
            name: d.get("ItemName").and_then(|n| n.get("SourceString")).and_then(Value::as_str).unwrap_or("").to_string(),
            cues: VOICE_EVENTS.iter().map(|e| (e.to_string(), path(d.get(*e)))).filter(|x| !x.1.is_empty()).collect(),
            pitch_limits: v2(&d, "PitchLimits"),
        }
    }

    /// every voice pack Blueprint in the paks
    pub fn all(rd: &Reader) -> Vec<VoicePack> {
        let mut v: Vec<String> = rd
            .vfs
            .list()
            .filter(|p| p.starts_with("Mordhau/Content/Mordhau/Blueprints/Voices/BP_") && p.ends_with(".uasset"))
            .map(|p| p.trim_end_matches(".uasset").to_string())
            .collect();
        v.sort();
        v.iter().map(|b| VoicePack::read(rd, b)).filter(|p| !p.cues.is_empty()).collect()
    }

    pub fn cue(&self, event: &str) -> Option<&str> {
        self.cues.iter().find(|c| c.0 == event).map(|c| c.1.as_str())
    }
}

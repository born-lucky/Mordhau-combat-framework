//! Which bot the game makes (rust-mode-ai r6): AMordhauAIController::BeginPlay rva=0x14f0ba0 (decomp
//! AMordhauAIController.cpp:1991-2220) after its RandomFloat / NextRandomFloatAssignment draws (controller.rs
//! begin_play), on the authority without a passed customization actor:
//!   1. the BotProfile: DefaultBotProfile's CDO (BP_MordhauAIController's CDO has none: extract/json
//!      Mordhau/Content/Mordhau/AI/BP_MordhauAIController.json), else UMordhauSingleton BotProfiles[i] with
//!      i = min(int(f32(rand() & 0x7fff) * 3.051851e-05 * f32(n)), n - 1) (BP_MordhauSingleton CDO: 35 BOT_* classes).
//!   2. a UBotProfile copied from that CDO: BehaviorProfile class, bRandomize* flags, UseBotLoadoutProfileID,
//!      bUseRandomBotLoadoutProfileID (UBotProfile ctor rva=0x144dfd0: bRandomizeBehavior false, bRandomizeName true,
//!      bRandomizeAppearance / Face / Skills / Equipment / Wearables false, bRandomizeVoice true, UseBotLoadoutProfileID
//!      -1; the BOT_* CDOs override). The id out of range and bUseRandomBotLoadoutProfileID -> a rand() pick the same
//!      way; a valid id -> CharacterProfile = Singleton BotCharacterProfiles[id] (its GearCustomization.Equipment ids
//!      index Singleton Equipment = the weapons).
//!   3. bRandomizeAppearance -> FAppearanceCustomization::Randomize (draws not ported: no BOT_* sets it);
//!      bRandomizeVoice -> VoicePitch = min(int(f32(rand() & 0x7fff) * 0.007812738), 255), then with a non-empty
//!      voice list (male 27 / female 9) Voice = the usual index pick (one more rand());
//!      bRandomizeEquipment -> FCharacterGearCustomization::RandomizeEquipment(.., 4) (BOT_FootsoldierRed / Blue:
//!      its draws are not ported, UNCONFIRMED: the stream diverges after it for those two); Wearables / Skills: no
//!      BOT_* sets them. UBotProfile::AssignToController rva=0x1458b20: a new UBotBehaviorProfile of the
//!      BehaviorProfile class (its class defaults: the spec's ENT_BOT_BOTBEHAVIOR_* record).
//!   4. no BehaviorProfile, or bRandomizeBehavior -> a plain UBotBehaviorProfile + Randomize rva=0x149c190
//!      (profile.rs randomize; no BOT_* in the roster sets it).
//! The roster's resulting behaviour classes: BOTBEHAVIOR_Boloncd 19, Worthless 11, Worthless2 1, Godlike 1 (Seymour),
//! Ranged 3 (the archers). BOTBEHAVIOR_Knight is not among them.

use super::bt_read::Loader;
use mordhau_core::ue::CrtRand;
use serde_json::Value;

pub const SINGLETON: &str = "Mordhau/Content/Mordhau/Blueprints/BP_MordhauSingleton";

#[derive(Clone, Debug, PartialEq)]
pub struct BotProfileDef {
    /// the BOT_* class ("BOT_Duelist")
    pub name: String,
    /// its BehaviorProfile class as the spec names it ("BOTBEHAVIOR_Boloncd"); "" = none
    pub behavior: String,
    pub randomize_behavior: bool,
    pub randomize_appearance: bool,
    pub randomize_voice: bool,
    pub randomize_equipment: bool,
    pub loadout_id: i64,
    pub random_loadout: bool,
}

#[derive(Clone, Debug, Default)]
pub struct BotRoster {
    pub profiles: Vec<BotProfileDef>,
    /// Singleton BotCharacterProfiles: per entry its equipment package paths (Equipment ids -> Singleton Equipment)
    /// and bIsFemale
    pub loadouts: Vec<(Vec<String>, bool)>,
    pub male_voices: usize,
    pub female_voices: usize,
}

/// the bot BeginPlay made
#[derive(Clone, Debug, PartialEq)]
pub struct BotChoice {
    pub profile: usize,
    pub name: String,
    pub behavior: String,
    /// Randomize the behaviour (step 4)
    pub randomize_behavior: bool,
    pub loadout: Option<usize>,
    /// the loadout's equipment packages (empty: the BOT_* class's own CharacterProfile, not read here)
    pub weapons: Vec<String>,
}

fn cdo_props(ex: &[Value]) -> serde_json::Map<String, Value> {
    ex.iter()
        .find(|e| e.get("Name").and_then(|n| n.as_str()).map(|n| n.starts_with("Default__")).unwrap_or(false))
        .and_then(|e| e.get("Properties").and_then(|p| p.as_object()).cloned())
        .unwrap_or_default()
}

fn obj_pkg(v: &Value) -> Option<String> {
    let p = v.get("ObjectPath")?.as_str()?;
    Some(p.rsplit_once('.').map(|x| x.0).unwrap_or(p).to_string())
}

/// "/Game/Mordhau/.../BP_Longsword.BP_Longsword_C" -> "Mordhau/Content/Mordhau/.../BP_Longsword"
fn soft_pkg(s: &str) -> String {
    let p = s.split('.').next().unwrap_or(s);
    format!("Mordhau/Content/{}", p.trim_start_matches("/Game/"))
}

/// the index pick the BeginPlay code inlines: min(int(f32(rand() & 0x7fff) * 3.051851e-05 * f32(n)), n - 1)
pub fn pick(rng: &mut CrtRand, n: usize) -> usize {
    let r = (rng.rand() & 0x7fff) as f32;
    let i = (r * 3.051851e-05f32 * n as f32) as i64;
    i.min(n as i64 - 1).max(0) as usize
}

impl BotRoster {
    /// from the packages: BP_MordhauSingleton's CDO and every BOT_* class's CDO (parent BP_BotProfile: only
    /// bRandomizeFace; the rest the UBotProfile ctor's defaults)
    pub fn from_exports(load: Loader) -> Result<BotRoster, String> {
        let s = cdo_props(&load(SINGLETON).ok_or("no BP_MordhauSingleton")?);
        let mut r = BotRoster::default();
        for b in s.get("BotProfiles").and_then(|v| v.as_array()).ok_or("singleton: no BotProfiles")? {
            let pkg = obj_pkg(b).ok_or("BotProfiles: bad reference")?;
            let p = cdo_props(&load(&pkg).ok_or_else(|| format!("no {pkg}"))?);
            let flag = |k: &str, d: bool| p.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
            let behavior = p
                .get("BehaviorProfile")
                .and_then(|v| v.get("ObjectName"))
                .and_then(|v| v.as_str())
                .map(|o| o.split('\'').nth(1).unwrap_or(o).trim_end_matches("_C").to_string())
                .unwrap_or_default();
            r.profiles.push(BotProfileDef {
                name: pkg.rsplit('/').next().unwrap_or(&pkg).to_string(),
                behavior,
                randomize_behavior: flag("bRandomizeBehavior", false),
                randomize_appearance: flag("bRandomizeAppearance", false),
                randomize_voice: flag("bRandomizeVoice", true),
                randomize_equipment: flag("bRandomizeEquipment", false),
                loadout_id: p.get("UseBotLoadoutProfileID").and_then(|v| v.as_i64()).unwrap_or(-1),
                random_loadout: flag("bUseRandomBotLoadoutProfileID", false),
            });
        }
        let equipment: Vec<String> = s
            .get("Equipment")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(|e| e.get("AssetPathName").and_then(|x| x.as_str()).unwrap_or("None").to_string()).collect())
            .unwrap_or_default();
        for c in s.get("BotCharacterProfiles").and_then(|v| v.as_array()).into_iter().flatten() {
            let ids: Vec<i64> = c["GearCustomization"]["Equipment"].as_array().map(|a| a.iter().filter_map(|e| e["Id"].as_i64()).collect()).unwrap_or_default();
            let weapons = ids
                .iter()
                .filter(|&&i| i > 0)
                .filter_map(|&i| equipment.get(i as usize))
                .filter(|p| p.as_str() != "None")
                .map(|p| soft_pkg(p))
                .collect();
            r.loadouts.push((weapons, c["AppearanceCustomization"]["bIsFemale"].as_bool().unwrap_or(false)));
        }
        r.male_voices = s.get("MaleVoices").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        r.female_voices = s.get("FemaleVoices").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        Ok(r)
    }

    /// steps 1-3 on `rng` (the world's rand() stream, right after the controller's two RandomFloat draws)
    pub fn choose(&self, rng: &mut CrtRand) -> Option<BotChoice> {
        if self.profiles.is_empty() {
            return None;
        }
        let i = pick(rng, self.profiles.len());
        let p = &self.profiles[i];
        let n = self.loadouts.len() as i64;
        let mut id = p.loadout_id;
        let mut loadout = None;
        if id >= 0 && id < n {
            loadout = Some(id as usize);
        } else if p.random_loadout {
            id = if n < 1 { 0 } else { pick(rng, n as usize) as i64 };
            if id >= 0 && id < n {
                loadout = Some(id as usize);
            }
        }
        let female = loadout.map(|l| self.loadouts[l].1).unwrap_or(false);
        if p.randomize_voice {
            let _pitch = ((rng.rand() & 0x7fff) as f32 * 0.007812738f32) as i64; // VoicePitch, min 255
            let nv = if female { self.female_voices } else { self.male_voices };
            if nv > 0 {
                let _voice = pick(rng, nv);
            }
        }
        Some(BotChoice {
            profile: i,
            name: p.name.clone(),
            behavior: p.behavior.clone(),
            randomize_behavior: p.randomize_behavior || p.behavior.is_empty(),
            loadout,
            weapons: loadout.map(|l| self.loadouts[l].0.clone()).unwrap_or_default(),
        })
    }
}

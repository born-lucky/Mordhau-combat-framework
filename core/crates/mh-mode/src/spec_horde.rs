//! `ModeData::from_spec_class`: the Horde and Battle Royale records from the spec matrix (rust-mode-ai r4; the six
//! Blueprint-ported modes are spec_load.rs, written by the sheets builder). The sheets builder generates these
//! entities from this crate's own exporter readers (godot/tools/golden/mode.gd `_run_data` / `_horde_data` /
//! `_horde_extras` / `_br_data`):
//!   ENT_MODE_HRD / ENT_MODE_BR   MordhauModeData of HRD_Camp / BR_Feitoria (scoring_*, state_*, meta_*, starts, ..;
//!                                BR also round_start_duration): mapped by ModeData::from_spec (spec_load.rs) for BR,
//!                                and the same way here for HRD with its ext from:
//!   ENT_HRDD_HORDE               config_* (HordeConfig, UE names in the record), squad_waves / squads / enemies (the
//!                                dump's shape), damage_by_player_count, extras_* (horde_extras::HordeExtras), and
//!                                skill_enum_names / skill_enum_values (E_HordeSkill UEnum::Names: horde_extras::
//!                                E_HORDE_SKILL is checked against them in tests/spec_equiv_hrd.rs).
//! The legacy Waves row count is not in the record (the dump never exported it either: 0; only read when
//! IsSquadSpawningEnabled is false, which no CDO sets).

use crate::data::{HordeData, ModeData};
use mh_spec::Spec;
use serde_json::{json, Map, Value};

fn err(e: impl std::fmt::Display) -> String {
    format!("spec: {e}")
}

/// HordeConfig's record names (data.rs serde renames) <- the spec's config_<snake> fields
const CONFIG: [(&str, &str); 17] = [
    ("WaveEnemySpawnOffset", "config_wave_enemy_spawn_offset"),
    ("IsSquadSpawningEnabled", "config_is_squad_spawning_enabled"),
    ("MinPlayerDifficultyModifier", "config_min_player_difficulty_modifier"),
    ("MaxPlayerDifficultyModifier", "config_max_player_difficulty_modifier"),
    ("ConsoleDifficultyModifier", "config_console_difficulty_modifier"),
    ("MinPlayerDelayModifier", "config_min_player_delay_modifier"),
    ("MaxPlayerDelayModifier", "config_max_player_delay_modifier"),
    ("ConsoleDelayModifier", "config_console_delay_modifier"),
    ("MaxEnemiesInWave", "config_max_enemies_in_wave"),
    ("TeamKillCoinPunishment", "config_team_kill_coin_punishment"),
    ("NewPlayerOnJoinGoldMultiplier", "config_new_player_on_join_gold_multiplier"),
    ("MaxNewPlayerOnJoinGold", "config_max_new_player_on_join_gold"),
    ("DeadWaveCompletionMultiplier", "config_dead_wave_completion_multiplier"),
    ("KillScoreChange", "config_kill_score_change"),
    ("Wave", "config_wave"),
    ("Coins", "config_coins"),
    ("SkillPoints", "config_skill_points"),
];

/// the extras' record keys <- the spec's extras_<key> fields
const EXTRAS: [&str; 9] = ["grave", "chests", "purchasables", "buy_menu", "di_buy_menu", "skills", "skill_prerequisites", "merchant_purchasables", "demon"];

impl ModeData {
    /// HRD or BR from the spec matrix (= ModeData::from_spec, which dispatches HRD here; kept for its callers)
    pub fn from_spec_class(spec: &Spec, id: &str) -> Result<ModeData, String> {
        ModeData::from_spec(spec, id)
    }
}

/// ENT_MODE_HRD (MordhauModeData of HRD_Camp) with its ext = HordeData from ENT_HRDD_HORDE
pub(crate) fn hrd_from_spec(spec: &Spec) -> Result<ModeData, String> {
    let mut v = spec.nested("ENT_MODE_HRD", &["scoring", "state", "meta", "rules", "table"]).map_err(err)?;
    if let Some(Value::Array(starts)) = v.get_mut("starts") {
        for s in starts.iter_mut() {
            let f: Vec<Value> = s["xf"]["origin"].as_array().into_iter().flatten().map(|x| json!(x.as_f64().unwrap_or(0.0) as f32 as f64)).collect();
            s["origin"] = Value::Array(f);
        }
    }
    let h = horde_data(spec)?;
    let mut ext = serde_json::to_value(&h).map_err(err)?;
    ext["id"] = json!("HRD");
    v["ext"] = ext;
    serde_json::from_value(v).map_err(|e| format!("ModeData::from_spec(HRD): {e}"))
}

/// HordeData from ENT_HRDD_HORDE
pub fn horde_data(spec: &Spec) -> Result<HordeData, String> {
    let r = spec.record("ENT_HRDD_HORDE").map_err(err)?;
    let mut config = Map::new();
    for (ue, f) in CONFIG {
        config.insert(ue.into(), spec.value(r.entity, &format!("FLD_HRDD_{}", f.to_uppercase())).map_err(err)?.clone());
    }
    let j = |f: &str| -> Result<Value, String> { Ok(r.json(f).map_err(err)?.clone()) };
    let mut extras = Map::new();
    for k in EXTRAS {
        extras.insert(k.into(), j(&format!("extras_{k}"))?);
    }
    let v = json!({
        "config": Value::Object(config),
        "squad_waves": j("squad_waves")?,
        "squads": j("squads")?,
        "enemies": j("enemies")?,
        "damage_by_player_count": r.f32_list("damage_by_player_count").map_err(err)?,
        "extras": Value::Object(extras),
    });
    serde_json::from_value(v).map_err(|e| format!("HordeData from spec: {e}"))
}

/// E_HordeSkill from the spec: (names, values) sorted by value
pub fn skill_enum(spec: &Spec) -> Result<Vec<(String, i64)>, String> {
    let r = spec.record("ENT_HRDD_HORDE").map_err(err)?;
    let n = r.string_list("skill_enum_names").map_err(err)?;
    let v = spec.i64_list(r.entity, "FLD_HRDD_SKILL_ENUM_VALUES").map_err(err)?;
    Ok(n.into_iter().map(String::from).zip(v).collect())
}

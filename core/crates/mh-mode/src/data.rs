//! The mode constants as plain records: what godot/game/mode/mordhau_mode_data.gd (MordhauModeData) and each mode's
//! inner data class (FfaMode.FfaData, TdmMode.TdmData, SkmMode.SkmData, DuelMode.DuelData, TfMode.TfData,
//! FlMode.FlData) read through the typed readers of godot/components/ue/records/mode_data.gd (ModeData). The values
//! come from the game's packages (class defaults down the Blueprint chain over the native ctors, placed actors of the
//! map); this crate never reads packages (no I/O): a host (mh-spec / mh-pak, or the golden exporter
//! godot/tools/export_golden_mode.gd today) fills these records, serde-deserializable from JSON.
//!
//! Sources, per record (cited in the reference):
//!   ScoringDef     AMordhauGameMode ctor rva=0x157a930 + the Blueprint chain (BP_MordhauGameMode KillScoreChange 100 /
//!                  TeamKillScoreChange -100; each mode's own CDO over it)
//!   MatchStateDef  AMordhauGameState ctor rva=0x157e160 + chain: MatchDurationMax, TeamCount, bIsTeamMode, WarmupEnd,
//!                  bAllowSpawning, TeamNames, bUsesAutoAssign
//!   Metadata       UGameModeMetadata CDO: Name, Prefix, Description
//!   PlayerStartDef the map's AMordhauPlayerStart actors (Team ctor -2 = any, AMordhauPlayerStart ctor 0x15b5450;
//!                  bIsSpawnDisabled; root transform origin, Godot metres)
//!   ControlPointDef  AControlPoint ctor rva=0x14e30e0 + BP_CapturePoint / BP_SkirmishCapturePoint chain + the placed
//!                  actor (OwningTeam, prerequisites, SpawnPoints)

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScoringDef {
    pub kill_score_change: f64,
    pub team_kill_score_change: f64,
    pub kill_team_score_change: f64,
    pub team_kill_team_score_change: f64,
    pub player_respawn_time: f64,
    pub b_players_spawn_in_waves: bool,
    pub b_suicide_decrements_kills: bool,
    pub b_team_kills_decrement_killer_kills: bool,
    pub b_team_kills_increment_killed_deaths: bool,
    pub assist_score_factor: f64,
    pub assist_damage_to_count_as_kill: i64,
    /// ctor true; HandleMatchHasStarted 0x1599ee0 clears it
    pub b_is_scoring_disabled: bool,
    pub spawn_protection_duration: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MatchStateDef {
    /// int seconds; zero-filled 0 = no limit unless a Blueprint CDO sets it (BP_DeathmatchGameState 1200)
    pub match_duration_max: i64,
    pub team_count: i64,
    pub b_is_team_mode: bool,
    /// ctor -1; AMordhauGameState::BeginPlay 0x1584900 stores -1 on the server (disasm 0x141584f18)
    pub warmup_end: f64,
    pub default_warmup_time: f64,
    /// TeamNames (FText), BP_MordhauGameState
    pub team_names: Vec<String>,
    pub b_uses_auto_assign: bool,
    /// ctor true; read by AMordhauGameMode::ControllerCanRestart_Implementation 0x1587e50 (0x141587f2d, +0x696)
    pub b_allow_spawning: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub prefix: String,
    pub description: String,
}

/// a placed AMordhauPlayerStart (the reference's ModeData.PlayerStartDef; only the transform's origin is read by the
/// rules: ChoosePlayerStart's distances and the spawn event)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerStartDef {
    pub name: String,
    /// Team +0x254 (ctor -2 = any team)
    pub team: i64,
    /// bIsSpawnDisabled +0x250 (AControlPoint::UpdateSpawns writes it)
    pub b_is_spawn_disabled: bool,
    /// world transform origin, Godot metres (f32 components, Godot Vector3)
    pub origin: [f32; 3],
}

/// AControlPoint class defaults + placed actor (ModeData.ControlPointDef). The two speed curves (UCurveFloat
/// CaptureSpeedCurve / NeutralizeSpeedCurve) are only ever evaluated at the integer presence lead
/// (AControlPoint::UpdateCaptureProgress rva=0x151b3f0: float(p1 - p2) or 0), so the record carries the curve's value
/// at lead 0, 1, 2, ... (`capture_speed[i]` = UCurveFloat::GetFloatValue(i)); the host evaluates the FRichCurve
/// (CombatData.curve_value in the reference). Empty = no curve (the reference's _curve: 0).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ControlPointDef {
    pub name: String,
    pub cls: String,
    pub display_name: String,
    pub owning_team: i64,
    pub capturing_team: i64,
    pub award_score_interval: f64,
    pub award_score_capturing: i64,
    pub award_score_captured: i64,
    pub award_score_neutralizing: i64,
    pub award_score_neutralized: i64,
    pub uncapture_speed: f64,
    pub network_smooth_time: f64,
    pub b_prevent_spawning_if_contested: bool,
    pub b_is_capturable: bool,
    pub b_should_pause_capture_if_enemy_near: bool,
    pub b_is_hidden_point: bool,
    pub capture_speed_curve: String,
    pub neutralize_speed_curve: String,
    #[serde(default)]
    pub capture_speed: Vec<f64>,
    #[serde(default)]
    pub neutralize_speed: Vec<f64>,
    /// BP_SkirmishCapturePoint TimeToUnlock (-1: not a variable of this class)
    pub time_to_unlock: f64,
    pub locked: bool,
    pub objective_win_delay: f64,
    pub team1_prerequisites: Vec<String>,
    pub team2_prerequisites: Vec<String>,
    pub spawn_points: Vec<String>,
}

impl ControlPointDef {
    /// UCurveFloat::GetFloatValue at an integer lead (header); no curve -> 0. A lead past the sampled range takes the
    /// last sample (UNCONFIRMED: the host samples up to the most players a match holds).
    pub fn curve(samples: &[f64], lead: f64) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let i = if lead <= 0.0 { 0 } else { lead as usize };
        samples[i.min(samples.len() - 1)]
    }
}

/// BP_SkirmishGameMode / BP_SkirmishGameState variables (SkmMode.SkmData)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkmData {
    pub round_points_per_team: Vec<f64>,
    pub capture_point_time_to_win: f64,
    pub coins_per_kill: i64,
    pub coins_per_round_loss: i64,
    pub coins_per_round_win: i64,
    pub extra_coins_per_loss_streak: i64,
    pub max_loss_streak: i64,
    pub round_duration: f64,
    pub late_round_spawn_duration: f64,
    pub round_end_duration: f64,
    pub round_start_duration: f64,
    pub win_condition_rounds: i64,
    pub last_winner: i64,
    pub team_swap_at_halftime: bool,
    /// E_SkirmishRoundStage display name -> value
    pub stage: BTreeMap<String, i64>,
}

/// BP_DuelGameMode / BP_DuelGameState (and BP_Group3v3GameMode over them) variables (DuelMode.DuelData)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DuelData {
    pub rounds_to_win: i64,
    pub max_people_per_room: i64,
    pub time_to_wait_for_players: f64,
    pub time_until_map_change: f64,
    pub map_prefixes: BTreeMap<String, String>,
    pub default_warmup_time: f64,
    pub allow_health_regen: bool,
    pub mode_name: String,
    pub mode_prefix: String,
    pub mode_description: String,
    /// E_DuelRoundStage display name -> value
    pub stage: BTreeMap<String, i64>,
    /// E_DuelRoomState display name -> value
    pub room_state: BTreeMap<String, i64>,
}

impl DuelData {
    /// game mode class for a map name by prefix (UGameMapsSettings GameModeMapPrefixes; "DU_Arena" -> BP_DuelGameMode)
    pub fn mode_for_map(&self, map_name: &str) -> String {
        let p = map_name.split('_').next().unwrap_or("");
        self.map_prefixes.get(p).cloned().unwrap_or_default()
    }
}

/// BP_FrontlineGameMode / BP_FrontlineGameState variables + the map's placed capture points (FlMode.FlData)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FlData {
    pub death_ticket_cost: f64,
    pub ticket_drain_interval: f64,
    pub ticket_drain_amount: f64,
    pub team1_starting_tickets: f64,
    pub team2_starting_tickets: f64,
    pub defending_team: i64,
    pub initial_stage_time: f64,
    pub control_points: Vec<ControlPointDef>,
}

/// BP_HordeGameMode / BP_HordePlayerState class-default scalars (UE property names in the record)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HordeConfig {
    #[serde(rename = "WaveEnemySpawnOffset")]
    pub wave_enemy_spawn_offset: f32,
    #[serde(rename = "IsSquadSpawningEnabled")]
    pub is_squad_spawning_enabled: bool,
    #[serde(rename = "MinPlayerDifficultyModifier")]
    pub min_difficulty: f32,
    #[serde(rename = "MaxPlayerDifficultyModifier")]
    pub max_difficulty: f32,
    #[serde(rename = "ConsoleDifficultyModifier")]
    pub console_difficulty: f32,
    #[serde(rename = "MinPlayerDelayModifier")]
    pub min_delay: f32,
    #[serde(rename = "MaxPlayerDelayModifier")]
    pub max_delay: f32,
    #[serde(rename = "ConsoleDelayModifier")]
    pub console_delay: f32,
    #[serde(rename = "MaxEnemiesInWave")]
    pub max_enemies_in_wave: i64,
    #[serde(rename = "TeamKillCoinPunishment")]
    pub team_kill_coin_punishment: i64,
    #[serde(rename = "NewPlayerOnJoinGoldMultiplier")]
    pub new_player_on_join_gold_multiplier: f32,
    #[serde(rename = "MaxNewPlayerOnJoinGold")]
    pub max_new_player_on_join_gold: f32,
    #[serde(rename = "DeadWaveCompletionMultiplier")]
    pub dead_wave_completion_multiplier: f32,
    /// the parent BP_MordhauGameMode KillScoreChange (100): the kill path's FTrunc(KillScoreChange)
    #[serde(rename = "KillScoreChange")]
    pub kill_score_change: f32,
    /// Wave (CDO -1)
    #[serde(rename = "Wave")]
    pub wave: i64,
    /// BP_HordePlayerState Coins (200) / SkillPoints (5)
    #[serde(rename = "Coins")]
    pub coins: i64,
    #[serde(rename = "SkillPoints")]
    pub skill_points: i64,
}

/// STRUCT_HordeSquadWaveInfo
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SquadWave {
    pub completion_reward: i64,
    pub difficulty_pool: f32,
    /// SquadInfo asset names
    pub mandatory_squads: Vec<String>,
}

/// a SquadInfo asset (USquadInfo ctor rva=0x166fb50 under the asset's properties)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SquadDef {
    pub name: String,
    pub spawn_min_wave: i64,
    pub spawn_max_wave: i64,
    pub max_squads_per_wave: i64,
    pub difficulty: f32,
    pub delay_before_spawn: i64,
    pub spawn_order_weight: i64,
    pub add_to_squad_pool: bool,
    pub squad_id: i64,
    /// Members (TMap order): EnemyDatabase key, count
    pub members: Vec<(String, i64)>,
}

/// an EnemyDatabase entry (STRUCT_HordeEnemyInfo) + its class's KillReward (class defaults)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnemyDef {
    pub key: String,
    pub character_class: String,
    pub behavior: String,
    pub kill_reward: i64,
    /// CustomizationVariants count (EnemyVariantCustomizationActor actors, SetupCustomizationReplicationActors)
    pub customization_variants: i64,
    pub equipment: Vec<String>,
    pub secondary_equipment: Vec<String>,
}

/// the Horde records (data_hrd_br golden header, godot/tools/golden/mode.gd _horde_data)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HordeData {
    pub config: HordeConfig,
    pub squad_waves: Vec<SquadWave>,
    /// every SquadInfo asset, the database order (asset registry order UNCONFIRMED: package path order)
    pub squads: Vec<SquadDef>,
    pub enemies: Vec<EnemyDef>,
    /// FC_HordeDamageModifier at team-0 counts 0, 1, 2, ... (GetFloatValue of the integer count, ProgressWave@2286)
    pub damage_by_player_count: Vec<f32>,
    /// the legacy Waves row count (only read when IsSquadSpawningEnabled is false)
    #[serde(default)]
    pub legacy_wave_count: usize,
    /// graves, chests, purchasables, the buy menu, the skill tree, Demon Invasion (r3: horde_extras)
    #[serde(default)]
    pub extras: crate::horde_extras::HordeExtras,
}

/// BP_BattleRoyaleGameState RoundStartDuration (5)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BrData {
    pub round_start_duration: f32,
}

/// the mode-specific variables of the mode's own Blueprints (the inner data classes)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "id")]
pub enum ModeDataExt {
    /// BP_DeathmatchGameMode ScoreToWin (3000)
    #[serde(rename = "FFA")]
    Ffa { score_to_win: f64 },
    /// BP_TeamDeathmatchGameMode TeamScoreToWin (1000)
    #[serde(rename = "TDM")]
    Tdm { team_score_to_win: f64 },
    #[serde(rename = "SKM")]
    Skm(SkmData),
    #[serde(rename = "DU")]
    Duel(DuelData),
    /// BP_Group3v3GameMode : BP_DuelGameMode (TfMode.TfData : DuelMode.DuelData)
    #[serde(rename = "TF")]
    Tf(DuelData),
    #[serde(rename = "FL")]
    Fl(FlData),
    #[serde(rename = "HRD")]
    Horde(HordeData),
    #[serde(rename = "BR")]
    Br(BrData),
}

/// MordhauModeData: the constants every mode reads, plus its own Blueprint's (ext)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModeData {
    pub game_mode: String,
    pub game_state: String,
    pub map_json: String,
    pub scoring: ScoringDef,
    pub state: MatchStateDef,
    pub meta: Metadata,
    pub starts: Vec<PlayerStartDef>,
    /// AMordhauAIController bAutoRespawn (BP_MordhauAIController chain)
    pub bot_auto_respawn: bool,
    /// UBotBehaviorProfile DefaultTeam (native ctor; -2 = auto-assign)
    pub bot_default_team: i64,
    pub ok: bool,
    pub ext: ModeDataExt,
}

impl ModeData {
    /// the mode's id (its map prefix, ModeTable row id)
    pub fn id(&self) -> &'static str {
        match self.ext {
            ModeDataExt::Ffa { .. } => "FFA",
            ModeDataExt::Tdm { .. } => "TDM",
            ModeDataExt::Skm(_) => "SKM",
            ModeDataExt::Duel(_) => "DU",
            ModeDataExt::Tf(_) => "TF",
            ModeDataExt::Fl(_) => "FL",
            ModeDataExt::Horde(_) => "HRD",
            ModeDataExt::Br(_) => "BR",
        }
    }
    pub fn from_json(text: &str) -> Result<ModeData, String> {
        serde_json::from_str(text).map_err(|e| format!("ModeData: {e}"))
    }
}

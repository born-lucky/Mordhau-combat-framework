//! Demon Invasion (rust-mode-ai r3): BP_DemonHordeGamemode : BP_HordeGameMode with BP_DemonHordeGamestate :
//! BP_HordeGameState, ported from their bytecode (scripts/kismet; `Function@N` = statement index). The classic Horde
//! code checks the game state class and skips its wave machinery for Demon Invasion (BP_HordeGameMode ubergraph:
//! begin play @2414-@3037 skips the kill objective, BP_HordeSpawn gathering and the squad database; the tick's
//! wave logic @3225; OnKilled's squad refill @6043 and the respawn clamp @6878). StartHordeMatch is overridden
//! (HasStarted = true; BP_SetupDemonHorde[0].StartGame, the map's scripted setup actor: host side).
//!
//! The game state runs the stages (RULE_HRD_HORDE_SPEC; the stage / objective actors are the map's):
//!   WaveSpawningCurrentStage (ubergraph @6179): CanSpawnEnemies -> a burst: i = 0, stop = false (@4342 / @2704);
//!     while i <= FTrunc(MaxTotalEnemies * (GetDifficultyMultiplier * 0.7)) and !stop: HandleCurrentStageEnemies,
//!     Delay(0.25), i + 1 (@177-@1653); then (@469-@1585) m = MapRangeUnclamped(difficulty, 0.5, 1, 3, 1), two
//!     RandomFloatInRange(m / 1.5, m) draws (@702: a range check that only prints, @1393: the timer),
//!     WaveSpawningCurrentStage again in FClamp(stop ? 0.1 : WaveSpawnInterval * roll, 1, 20); !CanSpawnEnemies ->
//!     again in 0.1 (@2720).
//!   HandleCurrentStageEnemies@0-@3325: authority, !HasMatchEnded, CurrentStage > 0, the stage's registered enemies <
//!     Round(FMin(MaxTotalEnemies * difficulty, 80)) -> SelectEnemyToSpawn(HordeEnemies), the enemy spawned at
//!     FindSpecific / FindRandomEnemySpawnLocation (host), KillReward = FTrunc(MapRangeClamped(KillReward, 0, 50, 20,
//!     150)), Team 2, a random CustomizationVariants profile, RegisterEnemy (+ OnCharacterDied -> UnregisterEnemy).
//!   DelayNewWave@6217: CanSpawnEnemies = false, RetriggerableDelay(StageSpawnDelay + 10) -> true (@1771).
//!   NewStageWaveReset@6198: DelayNewWave, stop = true (@2663).
//!   AllowSpawn@6407: bAllowSpawning = true, RetriggerableDelay(1) -> false (@1783-@1898).
//!   Begin / StopMatchTimer (@6036 / @6154): IncrementMatchElapsedTime every second (+1.0); begin also runs
//!     WaveSpawningCurrentStage; stop clears it and MatchElapsedTime = 0 (@1718).
//!   OnObjectiveCompleted / Failed (functions): StageCompleted / Failed = true; authority, CanProgressObjective (the
//!     objective is in StageSettings[CurrentStage].ActorArray) -> every valid stage actor must have the tag
//!     'Completed' / 'Failed' (a missing tag clears the flag and breaks) -> Set Current Stage(CurrentStage + 1) /
//!     FailStage; else PrintString "WARNING! Objective attempted to complete wrongly".
//!   FailStage: EndMatch, MatchEndInfo {null, WinnerTeam 1, WinnerScore 1, 0, false}.
//!   RespawnDeadPlayers: AllowSpawn; every BP_HordePlayerGrave, last to first: not Expired -> ForceRespawn; destroyed.
//!   GetDifficultyMultiplier: DifficultyScaling(NumberOfAlivePlayers) * (config DemonHorde.BaseDifficulty, else 1) *
//!     PrestigeTier. NumberOfAlivePlayers: PlayerArray pawns that are BP_DemonHordeCharacter and !GetIsDead.
//!   Get Late Joiner Free Coins: FTrunc(FC_DemonHordeJIPCoins(SafeDivide(float(byte(CurrentStage - 1)),
//!     float(StageSettings.Num)))).
//!   ModifyScoreByMatchParams: every BP_MordhauPlayerState: AddScore(FTrunc(FClamp(Score - Score *
//!     MapRangeClamped(MatchElapsedTime, 600, 2400, 1.5, 1) * (config Difficulty ? MapRangeClamped(it, 1, 2.75, 1, 4)
//!     : 1), 0, 1e8))).
//!   ResetPrestige (@2002, authority begin play @2797): PrestigeTier = config DemonHorde.Difficulty, else 1.
//!   HandleMatchEndInfo (@5866 -> @5382, @5347): the HUD (match_result_for); authority, once (@4304 DoOnce), unless
//!     config DisablePrestige: a win -> Difficulty = (Difficulty < 1 ? 0.65 : valid ? Difficulty : 1) + 0.35, Fails = 0
//!     (@3015-@3424); a loss -> Fails + 1, Difficulty = n > 7 ? 0.4 : n > 5 ? 0.6 : n > 3 ? 0.8 : 1 with n = the new
//!     Fails + 1 (@3459-@4116).
//! The game mode (BP_DemonHordeGamemode): K2_OnLogout (@420) -> RetriggerableDelay(10) -> PlayerArray empty: no
//! BP_DemonHordePlayerController left -> config Difficulty = 1; ChangeLevel(current level) either way (@15-@305).
//! Set Current Stage (no decodable script: UNCONFIRMED) is modelled as CurrentStage = value + OnRep_CurrentStage
//! (ActivateCurrentStageObjectives after StageActivationDelay, 0.01 at stage 1; the "Stage Completed"
//! announcement). The custom config vars (GetCustomConfigVar_*) persist across the level reloads: DemonConfig, owned
//! by the host.

use crate::game_mode::GameMode;
use crate::horde::EnemyId;
use crate::horde_extras::{random_float_in_range, DemonStage};
use std::collections::BTreeMap;

/// the "DemonHorde" custom config vars (UMordhauGameInstance custom config, persists across ChangeLevel)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DemonConfig {
    pub difficulty: Option<f32>,
    pub base_difficulty: Option<f32>,
    pub fails: Option<i64>,
    pub disable_prestige: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct DemonState {
    pub config: DemonConfig,
    /// the map's StageSettings (key = stage byte)
    pub stages: BTreeMap<u8, DemonStage>,
    pub current_stage: u8,
    pub stage_completed: bool,
    pub stage_failed: bool,
    pub can_spawn_enemies: bool,
    pub prestige_tier: f32,
    pub match_elapsed_time: f32,
    pub match_timer: Option<f32>,
    /// StageEnemies: stage -> the registered enemies (a TSet)
    pub stage_enemies: BTreeMap<u8, Vec<EnemyId>>,
    /// the burst: (next spawn time, i, cap)
    pub burst: Option<(f32, i64)>,
    /// Temp_bool_Variable_10 (NewStageWaveReset's stop)
    pub stop: bool,
    /// WaveSpawningCurrentStage timer
    pub wave_timer: Option<f32>,
    /// DelayNewWave's retriggerable delay -> CanSpawnEnemies = true
    pub can_spawn_at: Option<f32>,
    /// AllowSpawn's retriggerable delay -> bAllowSpawning = false
    pub allow_spawn_until: Option<f32>,
    /// OnRep_CurrentStage's ActivateCurrentStageObjectives timer
    pub activate_objectives_at: Option<f32>,
    /// the game mode's logout delay -> ChangeLevel
    pub reload_at: Option<f32>,
    pub has_ended_once: bool,
    pub holy_gun: bool,
}

/// FMath::GetMappedRangeValueClamped (UKismetMathLibrary::MapRangeClamped): Lerp(c, d, Clamp((v - a) / (b - a), 0, 1))
/// (UE 4.26 GetRangePct: a zero divisor -> (v >= b ? 1 : 0); engine source, UNCONFIRMED)
pub fn map_range_clamped(v: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    let div = b - a;
    let pct = if div == 0.0 { if v >= b { 1.0 } else { 0.0 } } else { (v - a) / div };
    let t = pct.clamp(0.0, 1.0);
    c + t * (d - c)
}

/// MapRangeUnclamped: Lerp(c, d, (v - a) / (b - a))
pub fn map_range_unclamped(v: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    let div = b - a;
    let pct = if div == 0.0 { if v >= b { 1.0 } else { 0.0 } } else { (v - a) / div };
    c + pct * (d - c)
}

impl GameMode {
    /// turn a Horde mode into Demon Invasion (the BP_DemonHordeGamemode / BP_DemonHordeGamestate classes): the map's
    /// stage rows and the persisted config vars. Begin play: ResetPrestige (authority @2797), BeginMatchTimer (@6036)
    pub fn set_demon_invasion(&mut self, stages: Vec<DemonStage>, config: DemonConfig) {
        let mut st = DemonState { config, can_spawn_enemies: true, ..Default::default() }; // CDO CanSpawnEnemies true
        for s in stages {
            st.stages.insert(s.stage as u8, s);
        }
        st.prestige_tier = st.config.difficulty.unwrap_or(1.0);
        self.horde_mut().demon = Some(st);
        self.demon_begin_match_timer();
    }

    pub fn demon(&self) -> &DemonState {
        self.horde().demon.as_ref().expect("not Demon Invasion")
    }
    fn demon_mut(&mut self) -> &mut DemonState {
        self.horde_mut().demon.as_mut().expect("not Demon Invasion")
    }

    /// BP_DemonHordeGamemode StartHordeMatch: HasStarted = true, BP_SetupDemonHorde[0].StartGame (host event)
    pub(crate) fn demon_start_horde_match(&mut self) {
        self.horde_mut().has_started = true;
        self.ev("demon_start_game");
    }

    /// NumberOfAlivePlayers (game state): pawns of PlayerArray that are BP_DemonHordeCharacter and not dead
    pub fn demon_alive_players(&self) -> i64 {
        self.controllers.iter().filter(|&&c| !self.ctrls[c].is_bot && self.ctrls[c].has_pawn && self.ctrls[c].alive).count() as i64
    }

    /// GetDifficultyMultiplier: DifficultyScaling(float(alive)) * (BaseDifficulty or 1) * PrestigeTier
    pub fn demon_difficulty(&self) -> f32 {
        let n = self.demon_alive_players();
        let s = &self.horde_extras().demon.difficulty_scaling;
        let curve = if s.is_empty() { 0.0 } else { s[(n.max(0) as usize).min(s.len() - 1)] };
        let d = self.demon();
        curve * d.config.base_difficulty.unwrap_or(1.0) * d.prestige_tier
    }

    /// Get Late Joiner Free Coins (the JIP curve sampled per stage count; None outside the exported table)
    pub fn demon_late_joiner_coins(&self) -> Option<i64> {
        let n = self.demon().stages.len();
        let row = self.horde_extras().demon.jip_coins.get(n.checked_sub(1)?)?;
        Some(*row.get(self.demon().current_stage as usize)? as i64)
    }

    /// ModifyScoreByMatchParams (header)
    pub fn demon_modify_score_by_match_params(&mut self) {
        let d = self.demon().clone();
        let t = map_range_clamped(d.match_elapsed_time, 600.0, 2400.0, 1.5, 1.0);
        let k = match d.config.difficulty {
            Some(v) => map_range_clamped(v, 1.0, 2.75, 1.0, 4.0),
            None => 1.0,
        };
        for c in self.controllers.clone() {
            if self.ctrls[c].is_bot {
                continue;
            }
            let s = self.ctrls[c].score as f32;
            let add = (s - s * t * k).clamp(0.0, 1e8) as i64;
            self.add_score(c, add);
        }
    }

    fn demon_begin_match_timer(&mut self) {
        let now = self.now as f32;
        self.demon_mut().match_timer = Some(now + 1.0);
        self.demon_wave_spawning_current_stage();
    }

    /// StopMatchTimer (@6154 -> @1708)
    pub fn demon_stop_match_timer(&mut self) {
        let d = self.demon_mut();
        d.match_timer = None;
        d.match_elapsed_time = 0.0;
    }

    /// WaveSpawningCurrentStage (header)
    pub fn demon_wave_spawning_current_stage(&mut self) {
        let now = self.now as f32;
        if !self.demon().can_spawn_enemies {
            self.demon_mut().wave_timer = Some(now + 0.1);
            return;
        }
        {
            let d = self.demon_mut();
            d.stop = false;
            d.burst = Some((now, 0));
        }
        self.demon_burst_step();
    }

    fn demon_burst_cap(&self) -> i64 {
        let st = self.demon().stages.get(&self.demon().current_stage).cloned().unwrap_or_default();
        (st.max_total_enemies as f32 * (self.demon_difficulty() * 0.7)) as i64
    }

    /// one iteration of the burst loop @177: spawn and wait 0.25, or end the burst and schedule the next wave
    fn demon_burst_step(&mut self) {
        let Some((_, i)) = self.demon().burst else { return };
        let now = self.now as f32;
        if i <= self.demon_burst_cap() && !self.demon().stop {
            self.demon_handle_current_stage_enemies();
            self.demon_mut().burst = Some((now + 0.25, i + 1));
            return;
        }
        self.demon_mut().burst = None;
        let st = self.demon().stages.get(&self.demon().current_stage).cloned().unwrap_or_default();
        let m = map_range_unclamped(self.demon_difficulty(), 0.5, 1.0, 3.0, 1.0);
        let _check = random_float_in_range(&mut self.rng, m / 1.5, m); // @702 (the debug range check)
        let r = random_float_in_range(&mut self.rng, m / 1.5, m); // @1393
        let next = if self.demon().stop { 0.1 } else { st.wave_spawn_interval * r };
        self.demon_mut().wave_timer = Some(now + next.clamp(1.0, 20.0));
    }

    /// HandleCurrentStageEnemies (header): the spawn request as an event (the host spawns the pawn, then calls
    /// demon_register_enemy with the remapped kill reward)
    fn demon_handle_current_stage_enemies(&mut self) {
        if self.match_state == crate::game_mode::MatchState::WaitingPostMatch || self.demon().current_stage == 0 {
            return;
        }
        let stage = self.demon().current_stage;
        let st = self.demon().stages.get(&stage).cloned().unwrap_or_default();
        if let Some(v) = self.demon().stage_enemies.get(&stage) {
            let cap = (st.max_total_enemies as f32 * self.demon_difficulty()).min(80.0);
            let cap = mordhau_core::ue::cvtss2si((cap + cap + 0.5) as f64) >> 1; // FMath::RoundToInt
            if v.len() as i64 >= cap {
                return;
            }
        }
        let Some(key) = self.demon_select_enemy(&st.horde_enemies) else { return };
        self.ev_args("demon_spawn_enemy", vec![("key", key.into()), ("stage", (stage as i64).into())]);
    }

    /// SelectEnemyToSpawn@0-@1610: windows [cum, cum + w] per key in map order, r = RandomFloatInRange(0, total), the
    /// first window with InRange(r, lo, hi, inclusive) wins; none -> Array_Random of the keys
    pub fn demon_select_enemy(&mut self, list: &[(String, f32)]) -> Option<String> {
        let mut acc = 0.0f32;
        let mut win = Vec::new();
        for (k, w) in list {
            win.push((k.clone(), acc, w + acc));
            acc = w + acc;
        }
        let r = random_float_in_range(&mut self.rng, 0.0, acc);
        for (k, lo, hi) in &win {
            if r >= *lo && r <= *hi {
                return Some(k.clone());
            }
        }
        let i = self.array_random(win.len())?;
        Some(win[i].0.clone())
    }

    /// the DI kill reward remap at spawn (HandleCurrentStageEnemies@2199): FTrunc(MapRangeClamped(KillReward, 0, 50,
    /// 20, 150))
    pub fn demon_kill_reward(kill_reward: i64) -> i64 {
        map_range_clamped(kill_reward as f32, 0.0, 50.0, 20.0, 150.0) as i64
    }

    /// RegisterEnemy@0-@636: StageEnemies[CurrentStage] += enemy
    pub fn demon_register_enemy(&mut self, e: EnemyId) {
        let s = self.demon().current_stage;
        let d = self.demon_mut();
        let v = d.stage_enemies.entry(s).or_default();
        if !v.contains(&e) {
            v.push(e);
        }
    }

    /// RegisteredEnemyDied -> UnregisterEnemy: removed from its stage's set (UnregisterEnemy's search over every
    /// stage: UNCONFIRMED detail)
    pub fn demon_unregister_enemy(&mut self, e: EnemyId) {
        for v in self.demon_mut().stage_enemies.values_mut() {
            v.retain(|&x| x != e);
        }
    }

    /// DelayNewWave (header)
    pub fn demon_delay_new_wave(&mut self) {
        let now = self.now as f32;
        let st = self.demon().stages.get(&self.demon().current_stage).cloned().unwrap_or_default();
        let d = self.demon_mut();
        d.can_spawn_enemies = false;
        d.can_spawn_at = Some(now + st.stage_spawn_delay + 10.0);
    }

    /// NewStageWaveReset (header)
    pub fn demon_new_stage_wave_reset(&mut self) {
        self.demon_delay_new_wave();
        self.demon_mut().stop = true;
    }

    /// AllowSpawn (header)
    pub fn demon_allow_spawn(&mut self) {
        let now = self.now as f32;
        self.allow_spawning = true;
        self.demon_mut().allow_spawn_until = Some(now + 1.0);
    }

    /// RespawnDeadPlayers (header)
    pub fn demon_respawn_dead_players(&mut self) {
        self.demon_allow_spawn();
        let graves: Vec<_> = self.horde().graves.iter().rev().map(|g| (g.id, g.expired)).collect();
        for (id, expired) in graves {
            if !expired {
                self.grave_force_respawn(id);
            }
            self.destroy_grave(id);
        }
    }

    /// OnObjectiveCompleted: `in_stage` = CanProgressObjective, `all_tagged` = every valid stage actor has 'Completed'
    pub fn demon_objective_completed(&mut self, in_stage: bool, all_tagged: bool) {
        self.demon_mut().stage_completed = true;
        if !in_stage {
            self.ev("objective_wrong");
            return;
        }
        self.demon_mut().stage_completed = all_tagged;
        if all_tagged {
            let s = self.demon().current_stage.wrapping_add(1);
            self.demon_set_current_stage(s);
        }
    }

    /// OnObjectiveFailed: as completed with 'Failed' -> FailStage
    pub fn demon_objective_failed(&mut self, in_stage: bool, all_tagged: bool) {
        self.demon_mut().stage_failed = true;
        if !in_stage {
            self.ev("objective_wrong");
            return;
        }
        self.demon_mut().stage_failed = all_tagged;
        if all_tagged {
            self.demon_fail_stage();
        }
    }

    /// Set Current Stage (UNCONFIRMED body) + OnRep_CurrentStage
    pub fn demon_set_current_stage(&mut self, s: u8) {
        let now = self.now as f32;
        let delay = if s == 1 { 0.01 } else { self.horde_extras().demon.stage_activation_delay };
        let has = self.demon().stages.contains_key(&s);
        {
            let d = self.demon_mut();
            d.current_stage = s;
            if has {
                d.activate_objectives_at = Some(now + delay);
            }
        }
        self.ev_args("demon_stage", vec![("stage", (s as i64).into())]);
    }

    /// FailStage (header)
    pub fn demon_fail_stage(&mut self) {
        self.end_match();
        self.set_end_info(None, 1, 1.0, false);
    }

    /// HandleMatchEndInfo's authority half (header): the config vars for the next match
    pub(crate) fn demon_on_match_end_info(&mut self) {
        if self.demon().has_ended_once {
            return;
        }
        self.demon_mut().has_ended_once = true;
        if self.demon().config.disable_prestige.unwrap_or(false) {
            return;
        }
        let win = self.match_end_info.as_ref().map(|e| e.winner_team == 0).unwrap_or(false);
        let cfg = &mut self.demon_mut().config;
        if win {
            let cur = cfg.difficulty.unwrap_or(0.0); // GetCustomConfigVar_Float of a missing var: 0 (UNCONFIRMED)
            let base = if cur < 1.0 { 0.65 } else { cfg.difficulty.unwrap_or(1.0) };
            cfg.difficulty = Some(base + 0.35);
            cfg.fails = Some(0);
        } else {
            let f = cfg.fails.unwrap_or(0) + 1;
            cfg.fails = Some(f);
            let n = f + 1;
            cfg.difficulty = Some(if n > 7 {
                0.4
            } else if n > 5 {
                0.6
            } else if n > 3 {
                0.8
            } else {
                1.0
            });
        }
    }

    /// BP_DemonHordeGamemode K2_OnLogout (header): the 10 s retriggerable reload check
    pub(crate) fn demon_on_logout(&mut self) {
        let now = self.now as f32;
        self.demon_mut().reload_at = Some(now + 10.0);
    }

    /// the game state's timers and latent actions (after the mode's tick; order UNCONFIRMED)
    pub(crate) fn demon_tick(&mut self) {
        if self.horde().demon.is_none() {
            return;
        }
        let now = self.now as f32;
        let d = self.demon().clone();
        if let Some(t) = d.match_timer {
            if now >= t {
                let dm = self.demon_mut();
                dm.match_elapsed_time += 1.0;
                dm.match_timer = Some(t + 1.0);
            }
        }
        if let Some(t) = d.can_spawn_at {
            if now >= t {
                let dm = self.demon_mut();
                dm.can_spawn_enemies = true;
                dm.can_spawn_at = None;
            }
        }
        if let Some(t) = d.allow_spawn_until {
            if now >= t {
                self.allow_spawning = false;
                self.demon_mut().allow_spawn_until = None;
            }
        }
        if let Some(t) = d.activate_objectives_at {
            if now >= t {
                self.demon_mut().activate_objectives_at = None;
                let s = self.demon().current_stage as i64;
                self.ev_args("demon_activate_objectives", vec![("stage", s.into())]);
            }
        }
        if let Some((t, _)) = d.burst {
            if now >= t {
                self.demon_burst_step();
            }
        }
        if let Some(t) = d.wave_timer {
            if now >= t {
                self.demon_mut().wave_timer = None;
                self.demon_wave_spawning_current_stage();
            }
        }
        if let Some(t) = d.reload_at {
            if now >= t {
                self.demon_mut().reload_at = None;
                let players = self.controllers.iter().filter(|&&c| !self.ctrls[c].is_bot).count();
                if players == 0 {
                    self.demon_mut().config.difficulty = Some(1.0);
                    self.ev("change_level");
                }
            }
        }
    }

    /// whether the classic Horde graph's Demon-Invasion casts skip its own logic
    pub(crate) fn is_demon(&self) -> bool {
        matches!(&self.ext, crate::game_mode::ModeExt::Horde(h) if h.demon.is_some())
    }

}

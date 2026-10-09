//! Horde (BP_HordeGameMode : BP_MordhauGameMode, game state BP_HordeGameState, player state BP_HordePlayerState),
//! the classic squad-wave mode. There is no GDScript reference: ported directly from the Blueprint bytecode
//! (scripts/kismet/pp.py on extract/raw; dumps state/grok_tmp/horde_gm.txt / horde_gs.txt; `Function@N` = the
//! in-memory statement index the dump prints). docs/HORDE_SPEC.md (Grok, r2) was reviewed against that bytecode by
//! rust-mode-ai r2: its sampled cites hold; the corrections are marked "(review)" below.
//!
//! Kismet `float` is binary32: every float here is f32 and every operation rounds as the exe's ss ops do (the
//! exe-exact model; there is no reference to match bit for bit). World time is read as GetTimeSeconds() = the f32 of
//! the mode clock. GetGameTimeInSeconds (UKismetSystemLibrary) and GetTimeSeconds (UGameplayStatics) both return
//! UWorld::GetTimeSeconds in UE 4.26 (UNCONFIRMED: engine source, not disassembled): one clock.
//!
//! Flow (ubergraph = ExecuteUbergraph_BP_HordeGameMode):
//!   ReceiveTick@7540: only with the BP_HordeGameState cast and IsMatchInProgress (@7645, @7679).
//!     !HasStarted: GetPlayerCountsPerTeam(false, true)[0] > 0 -> StartHordeMatch (@8247-@8398).
//!     HasStarted: GetPlayerCountsPerTeam(true, false)[0] == 0 && !bAllowSpawning && Graves.Num() == 0 &&
//!       !IsValid(CurrentlySpawningPlayerStart) && IsSpawnQueueEmpty() -> TriggerDefeat (@7703-@8232); else the wave
//!       logic @3225: WaveHasSpawned (squads @3340: live spawned + unspawned squad members = EnemiesRemaining; the
//!       next squad when GetGameTimeInSeconds() > TimeAtLastSquadSpawn + DelayBeforeSpawn * GetSpawnDelayRatio
//!       (@4659-@5270); publish STRUCT_HordeMatchInfo (@5312); EnemiesRemaining == 0 -> ProgressWave (@5840)); else
//!       WaveStartTime + WaveEnemySpawnOffset < GetTimeSeconds() -> bAllowSpawning = false, SpawnSquadWave (@4349-@4588).
//!   StartHordeMatch@0: HasStarted = true; ProgressWave (@116; the BP_DemonHordeGamestate cast fails in classic).
//!   ProgressWave: graves destroyed, SpawnedEnemies cleared (@21-@331); the completion reward paid to every
//!     MordhauPlayerController (@527-@4418) and TotalAwardedGoldPerPlayer += it (@2980); Wave + 1 (@1188),
//!     KillObjective OnWaveProgressed (@1301); valid row: bAllowSpawning = true, WaveStartTime = now,
//!     NextWaveStartTime = now + 25, WaveHasSpawned = false (@1435-@2030), RoundReceivedDamageModifier =
//!     ReceivedDamageByPlayerNum(GetPlayerCountsPerTeam(false, false)[0]) (@2142-@2656), team-0 controllers with a
//!     valid profile and no AdvancedCharacter pawn -> UnpossessAndDestroyPawn(c, true) (@4924-@5760); else the win:
//!     EndMatch, MatchEndInfo {null, 0, 1, 0, false} (@4619-@4801).
//!   OnKilled@8413: Killer != KilledPlayer, the killer a MordhauPlayerController with a BP_HordePlayerState:
//!     a BP_HordeEnemy pawn -> GiveClientScoreBP(0, FTrunc(KillScoreChange)), AddKills(1), Coins += KillReward
//!     (@8717-@8956), then the squad refill (@6043-@6844); else a PlayerController killed -> Coins = max(Coins - 50, 0)
//!     (@9083-@9300). Then the parent OnKilled (@9423), then, while bAllowSpawning, a killed MordhauPlayerController's
//!     NextRespawnTime = FMin(NextRespawnTime, NextWaveStartTime - 1) (@7075-@7355).
//!     (review) the squad refill runs only on a kill by a player (inside the killer-cast branch), not on every enemy
//!     death; the coin block runs before the parent OnKilled, the respawn clamp after it.
//!   K2_PostLogin@9501: parent, then HandlePostLogin (Wave > 0, not a leaver: SkillPoints += byte(Wave), Coins +=
//!     Round(FClamp(TotalAwardedGoldPerPlayer * 1.1, 0, 6000) / 10) * 10).
//!   K2_OnLogout@9544: LeaverPlayFabIDs += id, graves of the leaver destroyed, parent; NumPlayers == 0 ->
//!     TriggerDefeat (@9788-@9832).
//! Graves (r3 correction): BP_HordePlayerGrave's ReceiveBeginPlay@1644 adds itself to this mode's Graves and its
//! ReceiveDestroyed@1823 removes it, so the defeat check waits for every grave (live or expired, until its
//! AutoRevive) - horde_extras.rs. Demon Invasion (demon_horde.rs) shares this graph: the BP_DemonHordeGamestate casts
//! skip begin play's setup (@2414-@3037), the wave logic (@3225), OnKilled's squad refill (@6043) and respawn clamp
//! (@6878). Graves, chests, purchasables, the buy menu and the skill tree: horde_extras.rs.
//! Native OnKilled for a killed enemy: AMordhauGameMode::OnKilled_Implementation rva=0x159d6f0 credits a kill / score
//! only when the killed controller has a MordhauPlayerState; BP_HordeAIController's CDO has bWantsPlayerState = false
//! (extract/json/Mordhau/Content/Mordhau/AI/BP_HordeAIController.json), so an enemy's death gives no native kill,
//! death, assist or score: only this Blueprint's AddKills(1) / coins / GiveClientScoreBP (horde_enemy_killed). The
//! native function's NextRespawnTime write lands on the AI controller (+0x568, bAutoRespawn false: unused), and its
//! assist "thanks" voice line (a rand() draw when an AI assister's +0x3ec target is the killed pawn and it has a
//! profile) is not modelled: an RNG difference only when AI assist the kill (UNCONFIRMED effect on the stream).
//! Not ported (UNCONFIRMED / out of scope): the legacy `Waves` path (SpawnWave: IsSquadSpawningEnabled is true on
//! the CDO, so the tick never reaches it), the skills' combat effects (BP_HordeCharacter ApplySkills /
//! ModifyOutgoingDamage), the kill objective's own graph, bot stats, vehicles (ProgressWave's vehicle-driver branch).
//! Rule ids (data_gen/spec/rules.json): RULE_HRD_HordeGameMode_ReceiveTick, _StartHordeMatch, _ProgressWave,
//! _TriggerDefeat, _SpawnSquadWave, _Generate_Squads_in_Wave, _Find_Squad, _SpawnSquad, _GetDifficultyRatio,
//! _GetSpawnDelayRatio, _OnKilled, _K2_PostLogin, _HandlePostLogin, _K2_OnLogout, _ClearSquadWaveSpawningData,
//! _SortBySpawnOrderWeightMagnitude, _Sort_by_Squad_Difficulty, _SpawnEquipmentFor, _PrepareAIControllers,
//! _SetupCustomizationReplicationActors, _ExecuteUbergraph_BP_HordeGameMode, _SpawnWave (not ported);
//! RULE_HRD_HordeGameState_HandleMatchEndInfo, _OnRep_ReplicatedHordeMatchInfo, _ReceiveTick, _ReceiveBeginPlay,
//! _ShouldHideSpawnInfoText, _ExecuteUbergraph.

use crate::data::{HordeData, SquadDef};
use crate::event::Arg;
use crate::game_mode::{CtrlId, GameMode, ModeExt};
use crate::kismet_rand::{random_integer, random_integer_in_range, shuffle};
use std::collections::BTreeMap;

pub type EnemyId = usize;

/// a spawned BP_HordeEnemy (the host owns the pawn; it reports the death with horde_enemy_killed)
#[derive(Clone, Debug, PartialEq)]
pub struct HordeEnemy {
    /// EnemyDatabase key
    pub key: String,
    /// BP_HordeEnemy Squad (SetIntPropertyByName 'Squad' = CurrentSquadIndex after the increment, SpawnSquad@3230)
    pub squad: i64,
    pub kill_reward: i64,
    /// HordeSpawns slot (after the wave's shuffle) and the host spawner id it maps to
    pub spawner: usize,
    /// local offset in the spawner's transform: ((n / 6) * 130, (n % 6) * 130, 0) (SpawnSquad@2534-@2740)
    pub offset: [f32; 2],
    /// GetIsDead
    pub dead: bool,
    /// destroyed (IsValid false)
    pub gone: bool,
}

/// STRUCT_HordeWaveSpawningData
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaveSpawningData {
    pub total_enemies_spawned: i64,
    pub total_squad_spawned: i64,
    pub time_at_last_squad_spawn: f32,
    pub current_squad_index: i64,
    pub spawned_enemies_per_spawner: Vec<i64>,
    pub spawned_squads_per_spawner: Vec<i64>,
}

/// STRUCT_HordeMatchInfo (BP_HordeGameState ReplicatedHordeMatchInfo; CDO zeros)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HordeMatchInfo {
    pub wave: u8,
    pub enemies_remaining: u8,
    pub next_wave_start_time: f32,
    pub spawned_enemies: u8,
}

/// BP_HordePlayerState fields the mode writes
#[derive(Clone, Debug, PartialEq)]
pub struct HordePlayer {
    pub coins: i64,
    /// byte
    pub skill_points: i64,
    /// ReplicatedSkills (53 bytes: the player state construction loop 0..52)
    pub skills: Vec<u8>,
    /// BP_HordePlayerController SpecialSkill (the one ultimate, SkillsUpdated)
    pub special_skill: u8,
    /// BP_HordePlayerController PurchaseTrigger (byte, + 1 per ammo restock bought)
    pub purchase_trigger: u8,
}

impl HordePlayer {
    pub fn new(coins: i64, skill_points: i64) -> HordePlayer {
        HordePlayer { coins, skill_points, skills: vec![0; crate::horde_extras::SKILL_COUNT], special_skill: 0, purchase_trigger: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct HordeState {
    /// Wave (CDO -1)
    pub wave: i64,
    pub has_started: bool,
    pub wave_start_time: f32,
    pub wave_has_spawned: bool,
    /// SquadDatabase: indices into HordeData.squads (begin play: every SquadInfo asset with bAddToSquadPool, asset
    /// registry order UNCONFIRMED: the host's order)
    pub squad_database: Vec<usize>,
    pub squads_to_spawn: Vec<usize>,
    /// AddToFront / AddToEnd (members never cleared by the graph: they accumulate across waves)
    pub add_to_front: Vec<usize>,
    pub add_to_end: Vec<usize>,
    pub spawning: WaveSpawningData,
    /// SquadMembersRemaining: squad index -> members spawned by that SpawnSquad call
    pub squad_members_remaining: BTreeMap<i64, i64>,
    /// HordeSpawns: host spawner ids, in the order of the last shuffle
    pub horde_spawns: Vec<usize>,
    pub enemies: Vec<HordeEnemy>,
    /// SpawnedEnemies
    pub spawned_enemies: Vec<EnemyId>,
    pub enemies_remaining: i64,
    pub enemies_spawned: i64,
    pub total_awarded_gold_per_player: i64,
    pub leavers: Vec<String>,
    pub match_info: HordeMatchInfo,
    pub round_received_damage_modifier: f32,
    pub difficulty_modifier: f32,
    pub delay_modifier: f32,
    pub players: BTreeMap<CtrlId, HordePlayer>,
    /// the mode's Graves array: BP_HordePlayerGrave ReceiveBeginPlay@1644 Add / ReceiveDestroyed@1823 RemoveItem
    pub graves: Vec<crate::horde_extras::Grave>,
    pub next_grave_id: crate::horde_extras::GraveId,
    /// the placed chests (BP_HordeGameState Purchasables members)
    pub chests: Vec<crate::horde_extras::Chest>,
    /// Some: BP_DemonHordeGamemode / BP_DemonHordeGamestate (demon_horde.rs)
    pub demon: Option<crate::demon_horde::DemonState>,
    /// a BP_HordeKillObjective is placed (KillObjectiveCached valid)
    pub kill_objective: bool,
    /// IsConsoleServer
    pub console_server: bool,
}

impl HordeState {
    pub fn new(d: &HordeData, spawners: usize) -> HordeState {
        HordeState {
            wave: d.config.wave,
            has_started: false,
            wave_start_time: 0.0,
            wave_has_spawned: false,
            squad_database: (0..d.squads.len()).filter(|&i| d.squads[i].add_to_squad_pool).collect(),
            squads_to_spawn: Vec::new(),
            add_to_front: Vec::new(),
            add_to_end: Vec::new(),
            spawning: WaveSpawningData::default(),
            squad_members_remaining: BTreeMap::new(),
            horde_spawns: (0..spawners).collect(),
            enemies: Vec::new(),
            spawned_enemies: Vec::new(),
            enemies_remaining: 0,
            enemies_spawned: 0,
            total_awarded_gold_per_player: 0,
            leavers: Vec::new(),
            match_info: HordeMatchInfo::default(),
            round_received_damage_modifier: 0.0,
            difficulty_modifier: 0.0,
            delay_modifier: 0.0,
            players: BTreeMap::new(),
            graves: Vec::new(),
            next_grave_id: 0,
            chests: Vec::new(),
            demon: None,
            kill_objective: false,
            console_server: false,
        }
    }
}

impl GameMode {
    /// the map's BP_HordeSpawn actors (host spawner ids 0..n), gathered at begin play (ubergraph @2845)
    pub fn set_horde_spawners(&mut self, n: usize) {
        self.horde_mut().horde_spawns = (0..n).collect();
    }

    pub fn horde(&self) -> &HordeState {
        match &self.ext {
            ModeExt::Horde(h) => h,
            _ => panic!("not a Horde mode"),
        }
    }
    pub fn horde_mut(&mut self) -> &mut HordeState {
        match &mut self.ext {
            ModeExt::Horde(h) => h,
            _ => panic!("not a Horde mode"),
        }
    }
    pub fn horde_data(&self) -> &HordeData {
        match &self.d.ext {
            crate::data::ModeDataExt::Horde(h) => h,
            _ => panic!("not a Horde mode"),
        }
    }
    fn t32(&self) -> f32 {
        self.now as f32
    }
    /// GetAllActorsOfClass(MordhauPlayerController): the logged-in players (AI enemies and bots are AIControllers)
    fn player_controllers(&self) -> Vec<CtrlId> {
        self.controllers.iter().copied().filter(|&c| !self.ctrls[c].is_bot).collect()
    }

    /// GetDifficultyRatio@0-@805: n = MordhauPlayerController count; n > 0 -> Min + (Max - Min) * (float(n - 1) / 5.0)
    /// (times ConsoleDifficultyModifier on a console server), else MinPlayerDifficultyModifier
    pub fn get_difficulty_ratio(&mut self) -> f32 {
        let c = self.horde_data().config.clone();
        let n = self.player_controllers().len() as i64;
        let v = if n > 0 {
            let base = (c.max_difficulty - c.min_difficulty) * ((n - 1) as f32 / 5.0) + c.min_difficulty;
            if self.horde().console_server {
                base * c.console_difficulty
            } else {
                base
            }
        } else {
            c.min_difficulty
        };
        self.horde_mut().difficulty_modifier = v;
        v
    }

    /// GetSpawnDelayRatio@0-@805: n > 0 -> Min + (Max - Min) * (float(n - 1) / 5.0) (times ConsoleDelayModifier on a
    /// console server), else MaxPlayerDelayModifier
    pub fn get_spawn_delay_ratio(&mut self) -> f32 {
        let c = self.horde_data().config.clone();
        let n = self.player_controllers().len() as i64;
        let v = if n > 0 {
            let base = c.min_delay + (c.max_delay - c.min_delay) * ((n - 1) as f32 / 5.0);
            if self.horde().console_server {
                base * c.console_delay
            } else {
                base
            }
        } else {
            c.max_delay
        };
        self.horde_mut().delay_modifier = v;
        v
    }

    pub(crate) fn horde_receive_tick(&mut self) {
        if !self.in_progress() {
            return;
        }
        if !self.horde().has_started {
            if self.player_counts_per_team(false, true).first().copied().unwrap_or(0) > 0 {
                self.start_horde_match();
            }
            return;
        }
        let living0 = self.player_counts_per_team(true, false).first().copied().unwrap_or(0);
        // CurrentlySpawningPlayerStart: valid while the spawn queue has a controller between its spawn and finalize
        // steps (UNCONFIRMED mapping of AMordhauGameMode +0x3c8 onto the port's queue)
        if living0 == 0 && !self.allow_spawning && self.horde().graves.is_empty() && self.spawns.spawning.is_none() && self.spawns.queue.is_empty() {
            self.trigger_defeat();
            return;
        }
        if self.is_demon() {
            return; // @3225-@3311: a BP_DemonHordeGamestate skips the wave logic
        }
        let squads = self.horde_data().config.is_squad_spawning_enabled;
        if self.horde().wave_has_spawned {
            if !squads {
                return; // legacy SpawnWave path: not ported (header)
            }
            // @3354-@1801
            let h = self.horde();
            let mut spawned = 0;
            for &e in &h.spawned_enemies {
                if !h.enemies[e].gone && !h.enemies[e].dead {
                    spawned += 1;
                }
            }
            let mut remaining = 0;
            let mut t = h.spawning.current_squad_index;
            while t <= h.squads_to_spawn.len() as i64 - 1 {
                remaining += self.horde_data().squads[h.squads_to_spawn[t as usize]].members.iter().map(|m| m.1).sum::<i64>();
                t += 1;
            }
            let remaining = spawned + remaining;
            {
                let h = self.horde_mut();
                h.enemies_spawned = spawned;
                h.enemies_remaining = remaining;
            }
            // @4659-@5270
            let cur = self.horde().spawning.current_squad_index;
            if cur < self.horde().squads_to_spawn.len() as i64 {
                let ratio = self.get_spawn_delay_ratio();
                let sq = self.horde().squads_to_spawn[cur as usize];
                let delay = self.horde_data().squads[sq].delay_before_spawn as f32 * ratio;
                if self.t32() > self.horde().spawning.time_at_last_squad_spawn + delay {
                    self.spawn_squad(sq);
                }
            }
            // @5312-@5804
            {
                let h = self.horde_mut();
                h.match_info.enemies_remaining = remaining.clamp(0, 255) as u8;
                h.match_info.spawned_enemies = spawned.clamp(0, 255) as u8;
            }
            self.horde_match_info_event();
            if remaining == 0 {
                self.progress_wave();
            }
        } else {
            // @4349-@4588
            let off = self.horde_data().config.wave_enemy_spawn_offset;
            if self.horde().wave_start_time + off < self.t32() {
                self.allow_spawning = false;
                if squads {
                    self.spawn_squad_wave();
                }
            }
        }
    }

    fn horde_match_info_event(&mut self) {
        let mi = self.horde().match_info;
        self.ev_args(
            "horde_match_info",
            vec![
                ("wave", (mi.wave as i64).into()),
                ("enemies_remaining", (mi.enemies_remaining as i64).into()),
                ("next_wave_start_time", (mi.next_wave_start_time as f64).into()),
                ("spawned_enemies", (mi.spawned_enemies as i64).into()),
            ],
        );
    }

    /// StartHordeMatch@0-@116 (RULE_HRD_HordeGameMode_StartHordeMatch); BP_DemonHordeGamemode overrides it
    pub fn start_horde_match(&mut self) {
        if self.is_demon() {
            return self.demon_start_horde_match();
        }
        self.horde_mut().has_started = true;
        self.progress_wave();
    }

    /// TriggerDefeat@100-@368: EndMatch, MatchEndInfo {null, WinnerTeam 1, WinnerScore 1, 0, false}
    pub fn trigger_defeat(&mut self) {
        self.end_match();
        self.set_end_info(None, 1, 1.0, false);
    }

    /// ProgressWave (header)
    pub fn progress_wave(&mut self) {
        // @21-@294 every BP_HordePlayerGrave destroyed (ReceiveDestroyed: Graves.RemoveItem)
        for id in self.horde().graves.iter().map(|g| g.id).collect::<Vec<_>>() {
            self.destroy_grave(id);
        }
        self.ev("destroy_graves");
        self.horde_mut().spawned_enemies.clear(); // @331
        let pcs = self.player_controllers(); // @479
        let d = self.horde_data().clone();
        let squads = d.config.is_squad_spawning_enabled;
        let wave = self.horde().wave;
        if squads && wave >= 0 && (wave as usize) < d.squad_waves.len() {
            let reward = d.squad_waves[wave as usize].completion_reward; // @597
            for c in pcs.clone() {
                // ConsideredChar: the pawn as MordhauCharacter (a vehicle's driver: vehicles not ported)
                let coin = if self.ctrls[c].has_pawn {
                    let who = self.name(c);
                    self.ev_args("round_ended", vec![("who", who.clone().into())]); // @3251 BP_HordeCharacter.RoundEnded
                    self.ev_args("offset_health", vec![("who", who.clone().into()), ("amount", 100i64.into())]); // @3288
                    self.ev_args("client_score", vec![("who", who.into()), ("reason", 14i64.into()), ("score", reward.into())]);
                    reward
                } else {
                    // @4049-@4391: FTrunc(float(reward) * DeadWaveCompletionMultiplier)
                    let v = (reward as f32 * d.config.dead_wave_completion_multiplier) as i64;
                    let who = self.name(c);
                    self.ev_args("client_score", vec![("who", who.into()), ("reason", 14i64.into()), ("score", v.into())]);
                    v
                };
                // @3455-@4012
                if let Some(p) = self.horde_mut().players.get_mut(&c) {
                    p.coins += coin;
                    p.skill_points = (p.skill_points + 1) & 0xff;
                }
            }
            self.horde_mut().total_awarded_gold_per_player += reward; // @2980
        }
        let wave = wave + 1; // @1188
        self.horde_mut().wave = wave;
        if self.horde().kill_objective {
            self.ev_args("kill_objective_wave_progressed", vec![("wave", wave.into())]); // @1301
        }
        let valid = if squads {
            wave >= 0 && (wave as usize) < d.squad_waves.len()
        } else {
            wave >= 0 && (wave as usize) < d.legacy_wave_count
        };
        if !valid {
            // @4619-@4801 the win
            self.end_match();
            self.set_end_info(None, 0, 1.0, false);
            return;
        }
        // @1435-@2030
        self.allow_spawning = true;
        let now = self.t32();
        {
            let h = self.horde_mut();
            h.wave_start_time = now;
            h.match_info.next_wave_start_time = now + d.config.wave_enemy_spawn_offset;
            h.wave_has_spawned = false;
        }
        self.horde_match_info_event();
        if squads {
            // @2142-@2656 ReceivedDamageByPlayerNum.GetFloatValue(float(team-0 count))
            let n = self.player_counts_per_team(false, false).first().copied().unwrap_or(0);
            let s = &d.damage_by_player_count;
            let m = if s.is_empty() { 0.0 } else { s[(n.max(0) as usize).min(s.len() - 1)] };
            self.horde_mut().round_received_damage_modifier = m;
            self.ev_args("round_received_damage_modifier", vec![("value", (m as f64).into())]);
        }
        // @4924-@5760 (a team-0 player with a valid profile and no AdvancedCharacter pawn)
        for c in pcs {
            if !self.ctrls[c].has_pawn && self.ctrls[c].team == 0 {
                self.unpossess_and_destroy_pawn(c, true);
            }
        }
    }

    /// SpawnSquadWave@0-@1689
    pub fn spawn_squad_wave(&mut self) {
        self.horde_mut().wave_has_spawned = true;
        self.clear_squad_wave_spawning_data();
        {
            let h = match &mut self.ext {
                ModeExt::Horde(h) => h,
                _ => unreachable!(),
            };
            shuffle(&mut self.rng, &mut h.horde_spawns); // @126
            let n = h.horde_spawns.len();
            h.spawning.spawned_enemies_per_spawner = vec![0; n];
            h.spawning.spawned_squads_per_spawner = vec![0; n];
        }
        self.generate_squads_in_wave();
        let d = self.horde_data();
        let total: i64 = self.horde().squads_to_spawn.iter().map(|&s| d.squads[s].members.iter().map(|m| m.1).sum::<i64>()).sum();
        let cur = self.horde().spawning.current_squad_index;
        if let Some(&sq) = self.horde().squads_to_spawn.get(cur as usize) {
            self.spawn_squad(sq); // @1247
        } // an empty list reads None there (Array_Get out of range): no spawn (UNCONFIRMED)
        let wave = self.horde().wave;
        {
            let h = self.horde_mut();
            h.match_info = HordeMatchInfo { wave: (wave & 0xff) as u8, enemies_remaining: total.clamp(0, 255) as u8, next_wave_start_time: 0.0, spawned_enemies: 0 };
        }
        self.horde_match_info_event();
    }

    /// ClearSquadWaveSpawningData@0-@372
    fn clear_squad_wave_spawning_data(&mut self) {
        let h = self.horde_mut();
        h.spawning = WaveSpawningData::default();
        h.squad_members_remaining.clear();
    }

    /// Generate Squads in Wave@0-@2823
    pub fn generate_squads_in_wave(&mut self) {
        let mut valid = true;
        let ratio = self.get_difficulty_ratio();
        let wave = self.horde().wave as usize;
        let d = self.horde_data().clone();
        let mut pool = d.squad_waves[wave].difficulty_pool * ratio; // @48
        {
            let h = match &mut self.ext {
                ModeExt::Horde(h) => h,
                _ => unreachable!(),
            };
            h.squads_to_spawn.clear();
            shuffle(&mut self.rng, &mut h.squad_database); // @181
        }
        while pool > 0.0 && valid {
            match self.find_squad(pool) {
                None => valid = false, // @2242
                Some(s) => {
                    pool -= d.squads[s].difficulty; // @393
                    let w = d.squads[s].spawn_order_weight;
                    let h = self.horde_mut();
                    if w < 0 {
                        h.add_to_front.push(s); // @2324
                    }
                    if w > 0 {
                        h.add_to_end.push(s); // @2463
                    }
                    h.squads_to_spawn.push(s); // @2532
                }
            }
        }
        {
            let h = match &mut self.ext {
                ModeExt::Horde(h) => h,
                _ => unreachable!(),
            };
            shuffle(&mut self.rng, &mut h.squads_to_spawn); // @489
        }
        // @553-@1522: SortAndFilterArrayByFunction(SortBySpawnOrderWeightMagnitude): |A| > |B| first (sort stability
        // UNCONFIRMED: native helper not decoded); front entries inserted at their index, end entries appended from the
        // last one back
        let mag = |s: &usize| d.squads[*s].spawn_order_weight.abs();
        let h = self.horde_mut();
        let mut front = h.add_to_front.clone();
        front.sort_by(|a, b| mag(b).cmp(&mag(a)));
        for (i, s) in front.into_iter().enumerate() {
            h.squads_to_spawn.insert(i.min(h.squads_to_spawn.len()), s);
        }
        let mut end = h.add_to_end.clone();
        end.sort_by(|a, b| mag(b).cmp(&mag(a)));
        for s in end.into_iter().rev() {
            h.squads_to_spawn.push(s);
        }
    }

    /// Find Squad@0-@2253: the first mandatory squad of the row not already chosen; else a RandomInteger pick among
    /// the database squads whose window holds (Wave >= SpawnMinWave - 1 && Wave < SpawnMaxWave), Difficulty <= the
    /// pool, and fewer than MaxSquadsPerWave copies chosen; none -> null
    pub fn find_squad(&mut self, min_difficulty: f32) -> Option<usize> {
        let d = self.horde_data();
        let h = self.horde();
        let wave = h.wave;
        for m in &d.squad_waves[wave as usize].mandatory_squads {
            let Some(i) = d.squads.iter().position(|s| &s.name == m) else { continue };
            if !h.squads_to_spawn.contains(&i) {
                return Some(i); // @1746-@1824
            }
        }
        let mut suitable = Vec::new();
        for &db in &h.squad_database {
            let s: &SquadDef = &d.squads[db];
            if wave >= s.spawn_min_wave - 1 && wave < s.spawn_max_wave && s.difficulty <= min_difficulty {
                let amount = h.squads_to_spawn.iter().filter(|&&x| x == db).count() as i64;
                if amount < s.max_squads_per_wave {
                    suitable.push(db);
                }
            }
        }
        if suitable.is_empty() {
            return None;
        }
        let i = random_integer(&mut self.rng, suitable.len() as i64);
        Some(suitable[i as usize])
    }

    /// SpawnSquad@0-@5567: one random spawner slot for the whole squad; every member spawned there in the squad's
    /// Members map order (an EnemyDatabase key that is missing is skipped), each with a random customization actor and
    /// its equipment picks; SquadMembersRemaining[index] = members spawned
    pub fn spawn_squad(&mut self, sq: usize) {
        let d = self.horde_data().clone();
        let n_spawns = self.horde().horde_spawns.len() as i64;
        let slot = random_integer_in_range(&mut self.rng, 0, n_spawns - 1); // @534
        let t = self.t32(); // @603
        {
            let h = self.horde_mut();
            h.spawning.total_squad_spawned += 1;
            h.spawning.time_at_last_squad_spawn = t;
            h.spawning.current_squad_index += 1;
            if let Some(v) = h.spawning.spawned_squads_per_spawner.get_mut(slot as usize) {
                *v += 1;
            }
        }
        let squad_index = self.horde().spawning.current_squad_index;
        let mut amount = 0;
        for (key, count) in &d.squads[sq].members {
            let Some(e) = d.enemies.iter().find(|e| &e.key == key) else { continue };
            let mut i = 0;
            while i <= count - 1 {
                // @2240 customization pick, then the spawn, then the two equipment picks
                let _variant = random_integer_in_range(&mut self.rng, 0, e.customization_variants - 1);
                let n = self.horde().spawning.spawned_enemies_per_spawner.get(slot as usize).copied().unwrap_or(0);
                let offset = [((n / 6) as f32) * 130.0, ((n % 6) as f32) * 130.0];
                let spawner = self.horde().horde_spawns.get(slot as usize).copied().unwrap_or(0);
                let ai = self.horde().spawning.total_enemies_spawned;
                let id = {
                    let h = self.horde_mut();
                    h.enemies.push(HordeEnemy { key: key.clone(), squad: squad_index, kill_reward: e.kill_reward, spawner, offset, dead: false, gone: false });
                    let id = h.enemies.len() - 1;
                    if let Some(v) = h.spawning.spawned_enemies_per_spawner.get_mut(slot as usize) {
                        *v += 1;
                    }
                    h.spawned_enemies.push(id);
                    id
                };
                let eq = if e.equipment.is_empty() { -1 } else { random_integer_in_range(&mut self.rng, 0, e.equipment.len() as i64 - 1) };
                let eq2 = if e.secondary_equipment.is_empty() {
                    -1
                } else {
                    random_integer_in_range(&mut self.rng, 0, e.secondary_equipment.len() as i64 - 1)
                };
                // AIControllers[TotalEnemiesSpawned]: the pool holds MaxEnemiesInWave (PrepareAIControllers@42); past
                // it Array_Get gives None and the pawn is left unpossessed (UNCONFIRMED)
                let possessed = ai < d.config.max_enemies_in_wave;
                self.ev_args(
                    "horde_spawn",
                    vec![
                        ("enemy", (id as i64).into()),
                        ("key", key.as_str().into()),
                        ("class", e.character_class.as_str().into()),
                        ("behavior", e.behavior.as_str().into()),
                        ("squad", squad_index.into()),
                        ("spawner", (spawner as i64).into()),
                        ("offset", Arg::V([offset[0], offset[1], 0.0])),
                        ("variant", _variant.into()),
                        ("equipment", eq.into()),
                        ("secondary", eq2.into()),
                        ("possessed", possessed.into()),
                    ],
                );
                {
                    let h = self.horde_mut();
                    h.spawning.total_enemies_spawned += 1;
                }
                amount += 1;
                i += 1;
            }
        }
        self.horde_mut().squad_members_remaining.insert(squad_index, amount); // @5276
    }

    /// OnKilled@8418-@9386 for a killed player (the coin block before the parent)
    pub(crate) fn horde_on_killed_pre(&mut self, killer: Option<CtrlId>, killed: Option<CtrlId>) {
        let (Some(k), Some(v)) = (killer, killed) else { return };
        if k == v || self.ctrls[k].is_bot || self.ctrls[v].is_bot {
            return; // Killer != KilledPlayer; killer a MordhauPlayerController; KilledPlayer a PlayerController
        }
        let pun = self.horde_data().config.team_kill_coin_punishment;
        if let Some(p) = self.horde_mut().players.get_mut(&k) {
            p.coins = (p.coins + pun).max(0); // @9190-@9300
        }
    }

    /// @7075-@7355 after the parent OnKilled: while bAllowSpawning, a killed MordhauPlayerController's NextRespawnTime =
    /// FMin(NextRespawnTime, NextWaveStartTime - 1)
    pub(crate) fn horde_on_killed_post(&mut self, killed: Option<CtrlId>) {
        let Some(v) = killed else { return };
        if self.is_demon() {
            return; // @6878-@6964
        }
        if !self.allow_spawning || self.ctrls[v].is_bot {
            return;
        }
        let nw = self.horde().match_info.next_wave_start_time - 1.0;
        let nrt = self.ctrls[v].next_respawn_time as f32;
        self.ctrls[v].next_respawn_time = nrt.min(nw) as f64;
    }

    /// a BP_HordeEnemy died (the host's death report): OnKilled@8413 with the enemy's AI controller as KilledPlayer
    /// (RULE_HRD_HordeGameMode_OnKilled). The parent's native OnKilled credits nothing for a controller without a
    /// MordhauPlayerState (header); BP_MordhauGameMode's AddKillNotify still posts the kill feed (event).
    pub fn horde_enemy_killed(&mut self, enemy: EnemyId, killer: Option<CtrlId>) {
        self.horde_mut().enemies[enemy].dead = true;
        if let Some(k) = killer.filter(|&k| !self.ctrls[k].is_bot) {
            let d = self.horde_data().config.clone();
            let reward = self.horde().enemies[enemy].kill_reward;
            let who = self.name(k);
            self.ev_args("client_score", vec![("who", who.into()), ("reason", 0i64.into()), ("score", (d.kill_score_change as i64).into())]); // @8754
            self.ctrls[k].kills += 1; // AddKills(1)
            if let Some(p) = self.horde_mut().players.get_mut(&k) {
                p.coins += reward; // @8866
            }
            // @6144-@6844 squad refill: the stored count is read and written back unchanged (@6374), then
            // (count - 1) == 1 and CurrentSquadIndex in range -> SpawnSquad(next)
            let squad = self.horde().enemies[enemy].squad;
            if d.is_squad_spawning_enabled && !self.is_demon() {
                if let Some(&v) = self.horde().squad_members_remaining.get(&squad) {
                    self.horde_mut().squad_members_remaining.insert(squad, v);
                    let cur = self.horde().spawning.current_squad_index;
                    if v - 1 == 1 && cur < self.horde().squads_to_spawn.len() as i64 {
                        let sq = self.horde().squads_to_spawn[cur as usize];
                        self.spawn_squad(sq);
                    }
                }
            }
        }
        let key = self.horde().enemies[enemy].key.clone();
        let kn = killer.map(|k| self.name(k)).unwrap_or_default();
        self.ev_args("kill_notify", vec![("killer", kn.into()), ("killed", key.into()), ("flags", 0i64.into()), ("weapon", "".into())]);
    }

    /// the host destroyed an enemy pawn (IsValid false)
    pub fn horde_enemy_gone(&mut self, enemy: EnemyId) {
        self.horde_mut().enemies[enemy].gone = true;
    }

    /// HandlePostLogin@0-@1056 (after the parent K2_PostLogin). `playfab_id`: the controller's PlayFab id (the port
    /// uses the controller name: UNCONFIRMED)
    pub(crate) fn horde_post_login(&mut self, c: CtrlId) {
        if self.ctrls[c].is_bot {
            return;
        }
        let c0 = self.horde_data().config.clone();
        self.horde_mut().players.insert(c, HordePlayer::new(c0.coins, c0.skill_points));
        let wave = self.horde().wave;
        if wave <= 0 || self.horde().leavers.contains(&self.ctrls[c].name) {
            return;
        }
        let total = self.horde().total_awarded_gold_per_player;
        // Multiply_IntFloat, FClamp, / 10.0, Round (FMath::RoundToInt: cvtss2si(2x + 0.5) >> 1), * 10
        let x = (total as f32 * c0.new_player_on_join_gold_multiplier).clamp(0.0, c0.max_new_player_on_join_gold) / 10.0;
        let grant = (mordhau_core::ue::cvtss2si((x + x + 0.5) as f64) >> 1) * 10;
        if let Some(p) = self.horde_mut().players.get_mut(&c) {
            p.skill_points = (p.skill_points + (wave & 0xff)) & 0xff;
            p.coins += grant;
        }
    }

    /// K2_OnLogout@9544-@9832: the leaver's id remembered, its graves destroyed; NumPlayers == 0 -> TriggerDefeat.
    /// NumPlayers: AGameMode::Logout decrements it before K2_OnLogout (UE 4.26 engine, UNCONFIRMED).
    pub(crate) fn horde_on_logout(&mut self, c: CtrlId) {
        if self.ctrls[c].is_bot {
            return;
        }
        let n = self.name(c);
        self.horde_mut().leavers.push(n.clone());
        for id in self.horde().graves.iter().filter(|g| g.owner == c).map(|g| g.id).collect::<Vec<_>>() {
            self.destroy_grave(id);
        }
        self.ev_args("destroy_graves_of", vec![("who", n.into())]);
        if self.is_demon() {
            self.demon_on_logout(); // BP_DemonHordeGamemode K2_OnLogout (@420): after the parent
        }
        let left = self.player_controllers().into_iter().filter(|&o| o != c).count();
        if left == 0 {
            self.trigger_defeat();
        }
    }
}

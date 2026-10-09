//! Battle Royale (BP_BattleRoyaleGameMode : BP_MordhauGameMode, game state BP_BattleRoyaleGameState). No GDScript
//! reference: ported from the Blueprint bytecode (python scripts/kismet/pp.py extract/raw
//! Mordhau/Content/Mordhau/Blueprints/GameModes/BattleRoyale/BP_BattleRoyaleGameMode <out>: 365 lines; the game state
//! 130 lines, partly undecoded: its ubergraph head prints out of order). `Function@N` = in-memory statement index.
//! Floats are binary32 (Kismet float); the clock is the f32 of the mode clock.
//!
//!   ReceiveTick (ubergraph @466): only with the BP_BattleRoyaleGameState cast and IsMatchInProgress.
//!     DoOnce (@596-@1589): InitializedMatch = true, AlreadyDiedArray.Clear, StartRoundStart, StartRound.
//!     WarmupEnd + RoundStartDuration < GetTimeSeconds (@621-@778): DoOnce BattleRoyaleCircle.ActivateCircle
//!     (@1590-@1691); Countdown = byte(GetPlayerCountsPerTeam(true, false)[0]) when it differs (+OnRep_Countdown)
//!     (@813-@1278); !(bAllowSpawning || living > 1) -> EndRound (@1279-@1443).
//!   StartRoundStart@0-@1739: equipment with bAllowCleanup and projectiles destroyed; AlreadyDiedArray.Clear;
//!     bAllowSpawning = true; every Controller whose MordhauPlayerState Team >= 0: UnpossessAndDestroyPawn(c, true).
//!   StartRound@0-@635: BattleRoyaleSpawnManager.ActivateSpawns; bIsScoringDisabled = false; bAllowSpawning = false;
//!     AlreadyDiedArray.Clear; Countdown = byte(GetPlayerCountsPerTeam(false, false)[0]) (+OnRep).
//!   EndRound@0-@1526: the first BP_MordhauPlayerState in PlayerArray with bIsAlive: EndMatch, MatchEndInfo {that
//!     player, 0, 0, 0, false}, its BP_MordhauPlayerController owner AddScore(10000) (@625-@1059); none: EndMatch,
//!     MatchEndInfo {null, 0, 0, 0, Draw} (@1124-@1392). Then bIsScoringDisabled = true (@1097).
//!   OnKilled (ubergraph @1717): InitializedMatch, a valid KilledPlayer with a PlayerState: DeadCount (byte) + 1;
//!     Countdown = byte(max(living - 1, 0)) when it differs (@1999-@2601); a BP_BattleRoyalePlayerController killed:
//!     PlacementPosition = Countdown + 1 (byte add, @3011-@3265); its BP_BattleRoyalePlayerState Placement =
//!     DeadCount (@2638-@2853); AlreadyDiedArray.Add(KilledPlayer) (@3302); then the parent OnKilled (@3371).
//!     The living count still includes the dying player (the parent, which clears bIsAlive, runs after: the "- 1").
//!   K2_PostLogin (ubergraph @3445): parent; !bAllowSpawning -> LateJoinArray.Add, bLateJoiner = true (@3599-@3903).
//!   ControllerCanRestart@0-@393: parent && !LateJoinArray.Contains(c) && (!AlreadyDiedArray.Contains(c) ||
//!     !IsMatchInProgress).
//!   BP_BattleRoyaleGameState ShouldBlockPawnInput: parent || IsInGetReady (MatchState InProgress and WarmupEnd +
//!     RoundStartDuration > GetServerWorldTimeSeconds); ReceiveTick (local): while in get-ready, announcement
//!     Format(text, FCeil(FMax(WarmupEnd + RoundStartDuration - now, 0))) for 0.1 s when that count != 0;
//!     OnRep_Countdown: 1 < Countdown <= 20 -> announcement Format(text, {a} = Countdown) for 2 s + SC_BattleRoyaleCountdown.
//!   HandleMatchEndInfo (r3: the game state re-dumped with a script locator that checks the in-memory size, so its
//!     ubergraph decodes from @0; FText literals printed): ubergraph @15-@602: the local player's MordhauPlayerState ==
//!     MatchEndInfo.Winner -> ShowMatchResult(true, "victory", "") + ShowBREndScreenDelayed; nobody else gets a result
//!     (game_mode.rs match_result_for). Get-ready announcement: ShowAnnouncement("Get ready", Format("-{0}-", n), 0.1)
//!     (@2305-@2351); OnRep_Countdown: Format("-{a}-", Countdown) for 2 s (@547-@593).
//!   Chests: BP_BattleRoyaleChest (horde_extras.rs Chest: SpawnContents / GetRandomItem; the BR chests never respawn,
//!     RespawnTime 0 -> SetLifeSpan(2)).
//! Rule ids (data_gen/spec/rules.json): RULE_BR_BattleRoyaleGameMode_ReceiveTick, _StartRoundStart, _StartRound,
//! _EndRound, _OnKilled, _K2_PostLogin, _ControllerCanRestart, _MoreThanOnePlayerAlive, _ReceiveBeginPlay,
//! _ExecuteUbergraph_BP_BattleRoyaleGameMode; RULE_BR_BattleRoyaleGameState_HandleMatchEndInfo, _IsInGetReady,
//! _ShouldBlockPawnInput, _OnRep_Countdown, _ReceiveTick, _ShouldHideSpawnInfoText, _ExecuteUbergraph.
//! UNCONFIRMED: the circle / spawn manager (events for the host); PrintString of the placement (debug).

use crate::game_mode::{CtrlId, GameMode, ModeExt};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct BrState {
    pub initialized_match: bool,
    do_once_start: bool,
    do_once_circle: bool,
    /// DeadCount (byte)
    pub dead_count: i64,
    /// BP_BattleRoyaleGameState Countdown (byte)
    pub countdown: i64,
    pub already_died: Vec<CtrlId>,
    pub late_join: Vec<CtrlId>,
    /// BP_BattleRoyalePlayerState Placement
    pub placement: BTreeMap<CtrlId, i64>,
    /// BP_BattleRoyalePlayerController PlacementPosition (byte)
    pub placement_position: BTreeMap<CtrlId, i64>,
    /// BP_BattleRoyalePlayerState bLateJoiner
    pub late_joiner: Vec<CtrlId>,
    pub circle_active: bool,
}

impl GameMode {
    pub fn br(&self) -> &BrState {
        match &self.ext {
            ModeExt::Br(b) => b,
            _ => panic!("not a Battle Royale mode"),
        }
    }
    fn br_mut(&mut self) -> &mut BrState {
        match &mut self.ext {
            ModeExt::Br(b) => b,
            _ => panic!("not a Battle Royale mode"),
        }
    }
    fn br_rsd(&self) -> f32 {
        match &self.d.ext {
            crate::data::ModeDataExt::Br(b) => b.round_start_duration,
            _ => 0.0,
        }
    }
    /// WarmupEnd + RoundStartDuration (float)
    fn br_ready_at(&self) -> f32 {
        self.d.state.warmup_end as f32 + self.br_rsd()
    }

    fn set_countdown(&mut self, v: i64) {
        self.br_mut().countdown = v & 0xff;
        self.ev_args("countdown", vec![("countdown", (v & 0xff).into())]);
    }

    pub(crate) fn br_receive_tick(&mut self) {
        if !self.in_progress() {
            return;
        }
        if !self.br().do_once_start {
            let b = self.br_mut();
            b.do_once_start = true;
            b.initialized_match = true;
            b.already_died.clear();
            self.br_start_round_start();
            self.br_start_round();
        }
        if self.br_ready_at() < self.now as f32 {
            if !self.br().do_once_circle {
                self.br_mut().do_once_circle = true;
                self.br_mut().circle_active = true;
                self.ev("activate_circle"); // BattleRoyaleCircle.ActivateCircle (when a circle is placed)
            }
            let living = self.player_counts_per_team(true, false).first().copied().unwrap_or(0);
            if self.br().countdown != (living & 0xff) {
                self.set_countdown(living);
            }
            if !(self.allow_spawning || living > 1) {
                self.br_end_round();
            }
        }
    }

    /// StartRoundStart (header)
    fn br_start_round_start(&mut self) {
        self.ev("cleanup_world");
        self.br_mut().already_died.clear();
        self.allow_spawning = true;
        for c in self.controllers.clone() {
            if self.ctrls[c].team >= 0 {
                self.unpossess_and_destroy_pawn(c, true);
            }
        }
    }

    /// StartRound (header)
    fn br_start_round(&mut self) {
        self.ev("activate_spawns");
        self.scoring_disabled = false;
        self.allow_spawning = false;
        self.br_mut().already_died.clear();
        let n = self.player_counts_per_team(false, false).first().copied().unwrap_or(0);
        self.set_countdown(n);
    }

    /// EndRound (header)
    fn br_end_round(&mut self) {
        let winner = self.controllers.iter().copied().find(|&c| self.ctrls[c].alive);
        self.end_match();
        match winner {
            Some(w) => {
                self.set_end_info(Some(w), 0, 0.0, false);
                if !self.ctrls[w].is_bot {
                    self.add_score(w, 10000);
                }
            }
            None => self.set_end_info(None, 0, 0.0, true),
        }
        self.scoring_disabled = true;
    }

    /// OnKilled (header), before the parent
    pub(crate) fn br_on_killed_pre(&mut self, killed: Option<CtrlId>) {
        let Some(k) = killed else { return };
        if !self.br().initialized_match {
            return;
        }
        let dc = (self.br().dead_count + 1) & 0xff;
        self.br_mut().dead_count = dc;
        let living = self.player_counts_per_team(true, false).first().copied().unwrap_or(0);
        let v = (living - 1).max(0) & 0xff;
        if v != self.br().countdown {
            self.set_countdown(v);
        }
        if !self.ctrls[k].is_bot {
            let pp = (self.br().countdown + 1) & 0xff;
            self.br_mut().placement_position.insert(k, pp);
        }
        self.br_mut().placement.insert(k, dc);
        self.br_mut().already_died.push(k);
    }

    pub(crate) fn br_post_login(&mut self, c: CtrlId) {
        if !self.allow_spawning {
            let b = self.br_mut();
            b.late_join.push(c);
            b.late_joiner.push(c);
        }
    }

    pub(crate) fn br_controller_can_restart(&self, c: CtrlId, parent: bool) -> bool {
        let b = self.br();
        parent && !b.late_join.contains(&c) && (!b.already_died.contains(&c) || !self.in_progress())
    }

    /// IsInGetReady
    pub fn br_in_get_ready(&self) -> bool {
        self.in_progress() && self.br_ready_at() > self.now as f32
    }

    /// the get-ready countdown the local HUD shows (FCeil(FMax(WarmupEnd + RoundStartDuration - now, 0)); 0 = none)
    pub fn br_get_ready_count(&self) -> i64 {
        if !self.br_in_get_ready() {
            return 0;
        }
        (self.br_ready_at() - self.now as f32).max(0.0).ceil() as i64
    }
}

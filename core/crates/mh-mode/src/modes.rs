//! The mode Blueprints' overrides of MordhauGameMode (godot/game/mode/{ffa,tdm,skm,duel,tf,fl}_mode.gd), as
//! `impl GameMode` blocks dispatched from game_mode.rs, plus each mode's own state (SkmState, DuelState, FlState).
//! Literals: Kismet keys (dm_* / tdm_* / skm_* / duel / tf_* / fl_*), each {value, src} in mode_kismet.json.

use crate::control_point::{ControlPoint, CpCtx, NO_TEAM};
use crate::data::{DuelData, FlData, ModeDataExt, SkmData};
use crate::event::Arg;
use crate::game_mode::{CtrlId, GameMode, ModeExt, RoomGame, RoundInfo};
use crate::kismet::vb;
use mordhau_core::ue::maxf;
use std::collections::BTreeMap;

// ==== FFA (BP_DeathmatchGameMode : BP_MordhauGameMode, game state BP_DeathmatchGameState) =========================
// Everything shared (native OnKilled scoring, respawn, spawn choice, auto-assign, spawn queue, time limit) is the base;
// the Deathmatch Blueprints override:
//   OnScoreChanged (BP_DeathmatchGameMode ubergraph 462): Score >= ScoreToWin while in progress -> EndMatch,
//     MatchEndInfo {Winner = that player, WinnerTeam 0, WinnerScore, OtherScore 0, Draw false}
//   MatchTimeRanOut: parent (bMatchTimeRanOut, EndMatch), then FindWinner -> MatchEndInfo
//   FindWinner: GameState PlayerArray members with Team == 0, SortPlayers, first
//   HandleMatchEndInfo (BP_DeathmatchGameState): match_result_for
// Constants: ScoreToWin 3000 (the Blueprint's own variable); AssistScoreFactor 0; MatchDurationMax 1200.
impl GameMode {
    fn score_to_win(&self) -> f64 {
        match self.d.ext {
            ModeDataExt::Ffa { score_to_win } => score_to_win,
            _ => 0.0,
        }
    }

    pub(crate) fn ffa_on_score_changed(&mut self, c: CtrlId, _old: f64) {
        if self.ctrls[c].score >= self.score_to_win() && self.in_progress() {
            self.end_match();
            let (t, s) = (self.k.i("dm_end_winner_team"), self.ctrls[c].score);
            self.set_end_info(Some(c), t, s, false);
        }
    }

    pub(crate) fn ffa_match_time_ran_out(&mut self) {
        let w = self.find_winner();
        let s = w.map(|w| self.ctrls[w].score).unwrap_or(0.0);
        let t = self.k.i("dm_end_winner_team");
        self.set_end_info(w, t, s, false);
    }

    /// FindWinner: team 0 members of PlayerArray, sorted, first
    pub fn find_winner(&self) -> Option<CtrlId> {
        let team0: Vec<CtrlId> = self.controllers.iter().copied().filter(|&c| self.ctrls[c].team == 0).collect();
        self.sort_players(&team0).first().copied()
    }
}

// ==== TDM (BP_TeamDeathmatchGameMode, game state BP_TeamDeathmatchGameState) ======================================
//   OnTeamScoreChanged(Team) (ubergraph 10): in progress and TeamScores[Team] >= TeamScoreToWin (1000) -> EndMatch,
//     MatchEndInfo {Winner null, WinnerTeam = Team, WinnerScore = TeamScores[Team], OtherScore 0, Draw false}
//   MatchTimeRanOut (ubergraph 703): parent; TeamScores[0] == TeamScores[1] -> Draw; else the higher team wins
//   (no OnScoreChanged override: a player's score never ends the match)
impl GameMode {
    pub(crate) fn tdm_on_team_score_changed(&mut self, team: i64, _old: f64) {
        let to_win = match self.d.ext {
            ModeDataExt::Tdm { team_score_to_win } => team_score_to_win,
            _ => return,
        };
        let s = self.team_scores[team as usize];
        if self.in_progress() && s >= to_win {
            self.end_match();
            self.set_end_info(None, team, s, false);
        }
    }

    pub(crate) fn tdm_match_time_ran_out(&mut self) {
        let (s0, s1) = (self.team_scores[0], self.team_scores[1]);
        if s0 == s1 {
            let t = self.k.i("tdm_tie_winner_team");
            self.set_end_info(None, t, 0.0, true);
        } else if s0 > s1 {
            self.set_end_info(None, 0, s0, false);
        } else {
            self.set_end_info(None, 1, s1, false);
        }
    }
}

// ==== SKM (BP_SkirmishGameMode : BP_MordhauGameMode, game state BP_SkirmishGameState) ==============================
// Team rounds, no respawn inside a round. Ported from the bytecode (scripts/kismet; statement indices in brackets;
// literals: "skm_*" keys):
// Round flow, ReceiveTick (ubergraph 2550), only while IsMatchInProgress, switch GameState RoundInfo.Stage:
//   WaitingForPlayers (0): AlreadyDiedArray.Clear [1972]; CanStartRound -> StartRoundStart [1924]
//   RoundStart (1):  now >= StartTime + RoundStartDuration (5) -> CanStartRound ? StartRound : StartWaitingForPlayers
//   RoundPlay (2):   now >= StartTime + RoundDuration (150) -> EndRound(false) [3138]; else: now >= StartTime +
//                    LateRoundSpawnDuration (10) -> bAllowSpawning = false [204]; !MoreThanOneTeamAlive -> EndRound(true)
//                    [355]; else a capture point owned by a team (OwningTeam != 255): RoundPointsPerTeam[team] += 1,
//                    UpdateStateReplicatedPoints, >= CapturePointTimeToWin -> EndRound(false) [431..884]
//   RoundEnd (3):    CanStartRound ? (now >= StartTime + RoundEndDuration (5) -> StartRoundStart) : StartWaitingForPlayers
// StartWaitingForPlayers / StartRoundStart / StartRound / EndRound(IsWipe) / MoreThanOneTeamAlive / CanStartRound /
// OnKilled (ubergraph 915) / ControllerCanRestart / OnRep_RoundInfo / client ReceiveTick / ShouldBlockPawnInput /
// HandleMatchEndInfo / GetScoreboardTimeInProgress: on each function below (same texts as skm_mode.gd).
// Not ported (UNCONFIRMED / out of scope): the economy (coins: events only). The capture point is a ControlPoint
// (BP_SkirmishCapturePoint); no shipped SKM map places one, so set_capture_point is for hosts / tests.
#[derive(Clone, Debug)]
pub struct SkmState {
    /// GameState RoundInfo (CDO: Stage 0, Winner 0, 0)
    pub round_info: RoundInfo,
    /// BP_SkirmishGameMode RoundPointsPerTeam (float)
    pub round_points_per_team: Vec<f64>,
    /// BP_SkirmishGameState RoundPointsPerTeam (bytes)
    pub state_round_points: Vec<i64>,
    /// AlreadyDiedArray
    pub already_died: Vec<CtrlId>,
    /// BP_SkirmishGameMode CapturePoint: an index into the mode's control points
    pub capture_point: Option<usize>,
    pub has_swapped_teams: bool,
    /// GameState TeamSwapAtHalftime
    pub team_swap_at_halftime: bool,
    /// GameState LastObservedRoundStage
    pub last_observed_round_stage: i64,
    /// GameState LastWinner
    pub last_winner: i64,
    /// GameState LossStreak
    pub loss_streak: i64,
}

impl SkmState {
    pub fn new(sd: &SkmData) -> SkmState {
        SkmState {
            round_info: RoundInfo::default(),
            round_points_per_team: sd.round_points_per_team.clone(),
            state_round_points: vec![0; sd.round_points_per_team.len()],
            already_died: Vec::new(),
            capture_point: None,
            has_swapped_teams: false,
            team_swap_at_halftime: sd.team_swap_at_halftime,
            last_observed_round_stage: 0,
            last_winner: sd.last_winner,
            loss_streak: 0,
        }
    }
}

impl GameMode {
    pub fn skm(&self) -> &SkmState {
        match &self.ext {
            ModeExt::Skm(s) => s,
            _ => panic!("not a Skirmish mode"),
        }
    }
    fn skm_mut(&mut self) -> &mut SkmState {
        match &mut self.ext {
            ModeExt::Skm(s) => s,
            _ => panic!("not a Skirmish mode"),
        }
    }
    pub fn skm_data(&self) -> &SkmData {
        match &self.d.ext {
            ModeDataExt::Skm(s) => s,
            _ => panic!("not a Skirmish mode"),
        }
    }

    /// the round's capture point (BP_SkirmishGameMode CapturePoint): ticked with the world's control points
    pub fn set_capture_point(&mut self, cp: ControlPoint) -> usize {
        let i = self.add_control_point(cp);
        self.skm_mut().capture_point = Some(i);
        i
    }

    pub fn stage(&self) -> i64 {
        self.skm().round_info.stage
    }

    pub fn stage_name(&self, st: i64) -> String {
        let m = match &self.d.ext {
            ModeDataExt::Skm(s) => &s.stage,
            ModeDataExt::Duel(d) | ModeDataExt::Tf(d) => &d.stage,
            _ => return "?".into(),
        };
        m.iter().find(|(_, &v)| v == st).map(|(n, _)| n.clone()).unwrap_or_else(|| "?".into())
    }

    pub(crate) fn skm_receive_tick(&mut self, _dt: f64) {
        if !self.in_progress() {
            return;
        }
        let sd = self.skm_data().clone();
        let st = self.stage();
        let t0 = self.skm().round_info.start_time;
        let now = self.now;
        let q = self.q();
        if st == sd.stage["WaitingForPlayers"] {
            self.skm_mut().already_died.clear();
            if self.can_start_round() {
                self.start_round_start();
            }
        } else if st == sd.stage["RoundStart"] {
            if now >= q(t0 + sd.round_start_duration) {
                if self.can_start_round() {
                    self.start_round();
                } else {
                    self.start_waiting_for_players();
                }
            }
        } else if st == sd.stage["RoundPlay"] {
            if now >= q(t0 + sd.round_duration) {
                self.end_round(false);
                return;
            }
            if now >= q(t0 + sd.late_round_spawn_duration) {
                self.allow_spawning = false;
            }
            if !self.more_than_one_team_alive() {
                self.end_round(true);
            } else if let Some(cp) = self.skm().capture_point {
                let owner = self.control_points[cp].owning_team;
                if owner != self.k.i("skm_cp_no_team") {
                    let add = self.k.f("skm_cp_points_per_tick");
                    let s = self.skm_mut();
                    s.round_points_per_team[owner as usize] = q(s.round_points_per_team[owner as usize] + add);
                    self.update_state_replicated_points();
                    if self.skm().round_points_per_team[owner as usize] >= sd.capture_point_time_to_win {
                        self.end_round(false);
                    }
                }
            }
        } else if st == sd.stage["RoundEnd"] {
            if self.can_start_round() {
                if now >= q(t0 + sd.round_end_duration) {
                    self.start_round_start();
                }
            } else {
                self.start_waiting_for_players();
            }
        }
    }

    fn skm_counts(&self, args_key: &str) -> Vec<i64> {
        let a = self.k.arr(args_key);
        self.player_counts_per_team(vb(&a[0]), vb(&a[1]))
    }

    fn teams_with_players(counts: &[i64]) -> usize {
        counts.iter().filter(|&&n| n > 0).count()
    }

    /// MoreThanOneTeamAlive: bAllowSpawning -> true; else >= 2 teams with living players (GetPlayerCountsPerTeam(true, false))
    pub fn more_than_one_team_alive(&self) -> bool {
        if self.allow_spawning {
            return true;
        }
        Self::teams_with_players(&self.skm_counts("skm_alive_counts_args")) >= 2
    }

    /// CanStartRound: >= 2 teams with players (GetPlayerCountsPerTeam(false, true))
    pub fn can_start_round(&self) -> bool {
        Self::teams_with_players(&self.skm_counts("skm_start_counts_args")) >= 2
    }

    /// UMordhauUtilityLibrary::GetMaxIndexWithDraw 0x1623f00: (index of the first maximum, draw) (a later value equal to
    /// the running maximum sets draw, a larger one clears it); an empty array -> -1 (draw untouched: false here)
    pub fn max_index_with_draw(a: &[i64]) -> (i64, bool) {
        if a.is_empty() {
            return (-1, false);
        }
        let mut mx = a[0];
        let mut mi = 0;
        let mut draw = false;
        for (i, &v) in a.iter().enumerate().skip(1) {
            if v == mx {
                draw = true;
            } else if v > mx {
                draw = false;
                mx = v;
                mi = i as i64;
            }
        }
        (mi, draw)
    }

    fn set_round_info(&mut self, st: i64, winner: i64, t: f64) {
        self.skm_mut().round_info = RoundInfo { stage: st, winner, start_time: t };
        self.ev_args("round_info", vec![("stage", st.into()), ("winner", winner.into()), ("start_time", t.into())]);
        self.on_rep_round_info();
    }

    /// StartWaitingForPlayers: scoring off, AlreadyDied cleared, RoundInfo {0, 0, now}, bAllowSpawning true
    fn start_waiting_for_players(&mut self) {
        self.scoring_disabled = true;
        self.skm_mut().already_died.clear();
        let (st, now) = (self.k.i("skm_wfp_stage"), self.now);
        self.set_round_info(st, 0, now);
        self.allow_spawning = true;
    }

    /// StartRoundStart: scoring off; [0] StoreGear (event); [578] TeamSwapAtHalftime && !HasSwappedTeams &&
    /// TeamScores[0] + TeamScores[1] == 6 -> SwapTeams; [936] world cleanup (event); [2024] RoundPointsPerTeam[i] =
    /// Temp_float_Variable (never written in the graph: 0), UpdateStateReplicatedPoints; [2355] CapturePoint.RoundStarted;
    /// [2431] AlreadyDied cleared, RoundInfo {1, 0, now}, bAllowSpawning true, every controller with Team >= 0:
    /// UnpossessAndDestroyPawn(c, RestartPlayer = true)
    fn start_round_start(&mut self) {
        self.scoring_disabled = true;
        self.ev("store_gear"); // [0] BP_EconomyCharacter StoreGear per controller's pawn
        let s = self.skm();
        if s.team_swap_at_halftime && !s.has_swapped_teams && self.team_scores[0] + self.team_scores[1] == self.k.f("skm_halftime_rounds") {
            self.swap_teams();
        }
        self.ev("cleanup_world"); // [936]
        for v in self.skm_mut().round_points_per_team.iter_mut() {
            *v = 0.0; // [2024]
        }
        self.update_state_replicated_points();
        if let Some(cp) = self.skm().capture_point {
            // [2355] CapturePoint.RoundStarted
            self.ev("capture_point_round_started");
            let now = self.now;
            let k = self.k.clone();
            let mut awards = Vec::new();
            {
                let mut cx = CpCtx { ctrls: &mut self.ctrls, starts: &mut self.starts, awards: &mut awards, q: self.precision.q() };
                self.control_points[cp].round_started(now, &k, &mut cx);
            }
            for (c, s) in awards {
                self.add_score(c, s);
            }
        }
        self.skm_mut().already_died.clear(); // [2431]
        let (st, now) = (self.k.i("skm_round_start_stage"), self.now);
        self.set_round_info(st, 0, now);
        self.allow_spawning = true;
        for c in self.controllers.clone() {
            if self.ctrls[c].team >= 0 {
                self.unpossess_and_destroy_pawn(c, true);
            }
        }
    }

    /// StartRound: scoring on, AlreadyDied cleared, RoundInfo {2, 0, now}
    fn start_round(&mut self) {
        self.scoring_disabled = false;
        self.skm_mut().already_died.clear();
        let (st, now) = (self.k.i("skm_play_stage"), self.now);
        self.set_round_info(st, 0, now);
    }

    /// EndRound(IsWipe): wipe, or GameState RoundPointsPerTeam[0] == [1]: Winner = GetMaxIndexWithDraw(living players
    /// per team) (draw -> 255); else Winner = [0] > [1] ? 0 : 1. Winner 255: RoundInfo {3, 255, now}, bAllowSpawning
    /// false. Otherwise AddTeamScore(Winner, 1); TeamScores[Winner] >= WinConditionRounds (7) -> EndMatch, MatchEndInfo
    /// {null, Winner, TeamScores[Winner], 0, false} (RoundInfo is left in RoundPlay); TeamSwapAtHalftime && TeamScores
    /// sum >= (WinConditionRounds - 1) * 2 -> EndMatch, MatchEndInfo {null, 0, 0, 0, Draw}; else RoundInfo {3, Winner,
    /// now}, bAllowSpawning false. Always: scoring off.
    fn end_round(&mut self, is_wipe: bool) {
        let sp = self.skm().state_round_points.clone();
        let by_counts = is_wipe || sp[0] == sp[1];
        let draw_winner = self.k.i("skm_draw_winner");
        let winner = if by_counts {
            let (i, draw) = Self::max_index_with_draw(&self.skm_counts("skm_end_counts_args"));
            if draw {
                draw_winner
            } else {
                i & 0xff
            }
        } else if sp[0] > sp[1] {
            0
        } else {
            1
        };
        let mut to_round_end = true;
        if !(by_counts && winner == draw_winner) {
            let pts = self.k.f("skm_round_win_points");
            self.add_team_score(winner, pts);
            let wcr = self.skm_data().win_condition_rounds as f64;
            let ts = self.team_scores[winner as usize];
            if ts >= wcr {
                self.end_match();
                self.set_end_info(None, winner, ts, false);
                to_round_end = false;
            } else if self.skm().team_swap_at_halftime
                && self.team_scores[0] + self.team_scores[1]
                    >= (wcr - self.k.f("skm_halftime_draw_sub")) * self.k.f("skm_halftime_draw_mul")
            {
                self.end_match();
                self.set_end_info(None, 0, 0.0, true);
                to_round_end = false;
            }
        }
        if to_round_end {
            let (st, now) = (self.k.i("skm_end_stage"), self.now);
            self.set_round_info(st, winner, now);
            self.allow_spawning = false;
        }
        self.scoring_disabled = true;
    }

    /// UpdateStateReplicatedPoints: GameState RoundPointsPerTeam = Conv_IntToByte(FFloor(points)) per team
    fn update_state_replicated_points(&mut self) {
        let s = self.skm_mut();
        s.state_round_points = s.round_points_per_team.iter().map(|v| (v.floor() as i64) & 0xff).collect();
    }

    /// SwapTeams: every controller with Team >= 0 to the other team (0 <-> 1), economy reset; then TeamScores swapped,
    /// LastWinner 255, LossStreak 0
    fn swap_teams(&mut self) {
        self.skm_mut().has_swapped_teams = true;
        for c in self.controllers.clone() {
            let t = self.ctrls[c].team;
            if t >= 0 {
                self.set_team(c, if t == 0 { 1 } else { 0 });
                let who = self.name(c);
                self.ev_args("reset_economy", vec![("who", who.into())]);
            }
        }
        self.team_scores.swap(0, 1);
        let s = self.skm_mut();
        s.last_winner = 255;
        s.loss_streak = 0;
    }

    /// OnKilled (ubergraph 915): Stage != 0 and in progress -> AlreadyDiedArray.Add(killed); a non-friendly killer gets
    /// CoinsPerKill (economy: event); then the parent OnKilled (on_killed dispatch)
    pub(crate) fn skm_on_killed(&mut self, killer: Option<CtrlId>, killed: Option<CtrlId>) {
        let Some(kd) = killed else { return };
        if self.stage() != self.k.i("skm_kill_waiting_stage") && self.in_progress() {
            self.skm_mut().already_died.push(kd);
            if let Some(kr) = killer {
                if !self.is_friendly(Some(kr), Some(kd), false) {
                    let (who, coins) = (self.name(kr), self.skm_data().coins_per_kill);
                    self.ev_args("give_coins", vec![("who", who.into()), ("coins", coins.into())]);
                }
            }
        }
    }

    /// OnRep_RoundInfo (BP_SkirmishGameState; the server calls it after every RoundInfo write): on a stage change,
    /// LastObservedRoundStage = Stage; RoundStart -> destroy dead characters and fire fields (event); RoundEnd ->
    /// LastWinner != 255 && == Winner ? LossStreak + 1 : 0; LastWinner = Winner; HUD "Draw!" (Winner 255) or
    /// Format("{a} has won the round.", a = GetTeamName(Winner)) for RoundEndDuration s; round coins (economy: events)
    fn on_rep_round_info(&mut self) {
        let ri = self.skm().round_info;
        let st = ri.stage;
        if st == self.skm().last_observed_round_stage {
            return;
        }
        self.skm_mut().last_observed_round_stage = st;
        let sd = self.skm_data().clone();
        if st == sd.stage["RoundStart"] {
            self.ev("destroy_corpses");
        } else if st == sd.stage["RoundEnd"] {
            let w = ri.winner;
            let s = self.skm_mut();
            if s.last_winner != 255 && s.last_winner == w {
                s.loss_streak += 1;
            } else {
                s.loss_streak = 0;
            }
            s.last_winner = w;
            let (text, key) = if w == self.k.i("skm_draw_winner_gs") {
                (self.k.s("skm_draw_text"), "skm_draw_text")
            } else {
                (self.k.s("skm_round_won_format").replace("{a}", &self.team_name(w)), "skm_round_won_format")
            };
            let src = self.k.src(key).to_string();
            self.ev_args(
                "announce",
                vec![("text", text.into()), ("subtext", "".into()), ("duration", sd.round_end_duration.into()), ("src", src.into())],
            );
            let ls = self.skm().loss_streak;
            for c in self.controllers.clone() {
                let t = self.ctrls[c].team;
                if t == 0 || t == 1 {
                    let coins = if (t & 0xff) == w {
                        sd.coins_per_round_win
                    } else {
                        sd.coins_per_round_loss + sd.extra_coins_per_loss_streak * sd.max_loss_streak.min(ls)
                    };
                    let who = self.name(c);
                    self.ev_args("give_coins", vec![("who", who.into()), ("coins", coins.into())]);
                }
            }
        }
    }

    /// ReceiveTick on the local machine (BP_SkirmishGameState, local player, InProgress): LastObservedRoundStage 0 ->
    /// "Waiting for players" 0.1 s; 1 -> FCeil(StartTime + RoundStartDuration - now) != 0 -> "Round starting"
    /// Format("-{0}-") 0.1 s. HUD commands (kind "announce", no "t").
    pub(crate) fn skm_client_tick(&self, _me: CtrlId) -> Vec<crate::event::Ev> {
        use crate::event::Ev;
        if !self.in_progress() {
            return Vec::new();
        }
        let k = &self.k;
        let lo = self.skm().last_observed_round_stage;
        if lo == k.i("skm_hud_waiting_stage") {
            let v = k.arr("skm_waiting");
            return vec![Ev::new("announce")
                .with("text", crate::kismet::vs(&v[0]))
                .with("subtext", "")
                .with("duration", crate::kismet::vf(&v[1]))
                .with("src", k.src("skm_waiting"))];
        }
        if lo == k.i("skm_hud_round_start_stage") {
            let n = (self.skm().round_info.start_time + self.skm_data().round_start_duration - self.now).ceil() as i64;
            if n != 0 {
                let v = k.arr("skm_round_starting");
                return vec![Ev::new("announce")
                    .with("text", crate::kismet::vs(&v[0]))
                    .with("duration", crate::kismet::vf(&v[1]))
                    .with("src", k.src("skm_round_starting"))
                    .with("subtext", k.s("skm_count_format").replace("{0}", &n.to_string()))];
            }
        }
        Vec::new()
    }

    /// GetScoreboardTimeInProgress (BP_SkirmishGameState): stage 0 -> 200 days; 1 / 2 / 3 -> FCeil(FMax(StartTime +
    /// duration - now, 0)); else the parent (None)
    pub(crate) fn skm_scoreboard_seconds_in_progress(&self) -> Option<i64> {
        let t0 = self.skm().round_info.start_time;
        let sd = self.skm_data();
        let c = |dur: f64| maxf(t0 + dur - self.now, 0.0).ceil() as i64;
        match self.stage() {
            0 => Some(self.k.i("skm_sb_no_limit_days") * 86400),
            1 => Some(c(sd.round_start_duration)),
            2 => Some(c(sd.round_duration)),
            3 => Some(c(sd.round_end_duration)),
            _ => None,
        }
    }
}

// ==== DU (BP_DuelGameMode : BP_MordhauGameMode) and TF (BP_Group3v3GameMode : BP_DuelGameMode) =====================
// Room based: every pair of players gets a "room" (ST_DuelRoom) with its own game (ST_DuelRoomGame: Team1Wins,
// Team2Wins, RoundInfo). Round flow (TickRoom):
//   WaitingForPlayers --2 players & stats--> WaitingToStart (+10 s) --RestartRoom--> RoundStart (+5 s, input blocked)
//   --> RoundPlay (until one team has nobody alive) --AwardRoundWin--> RoundEnd (+2 s) --RestartRoom--> RoundStart ...
//   AwardRoundWin with RoundsToWin (5) wins -> FinishRoomGame -> room Terminated -> next TickRoom destroys it.
// Every tick each room's game is copied to its controllers (ReplicatedRoomGame), which is all the HUD reads.
// Port adaptations (UNCONFIRMED: no shipped equivalent, needed to play offline), as the reference:
//   - local play takes the IsPlayInEditor() branches (HandleUnhandledPlayers handles a controller with no
//     MatchmakingMatchID; HandleNewPlayer joins any open room) instead of PlayFab matchmaking;
//   - IsRoomReadyToStart's AreStatsAvailable() (PlayFab stats) is taken as true;
//   - bots are logged in like players; AwardDuelMMR / reward drops / kicks are events only;
//   - a player spawns when AssignTeam gives it a team.
// TF: CDO MaxPeoplePerRoom 6 etc.; AssignTeam (ubergraph 2587) by the PlayFab match's TeamID ("FreeGuard" -> 1, else
// 0; local matchmaking stand-in: every second login is FreeGuard); FinishRoomGame (ubergraph 3146) scores every member;
// ComputeTeamMMR without stats -> 0 / 0; ShouldBlockPawnInput (BP_Group3v3GameState).
const LOCAL_PLAY: bool = true;
/// TfMode LOCAL_MATCH: the one local match every login joins (header: UNCONFIRMED stand-in)
pub const LOCAL_MATCH: &str = "local";

/// ST_DuelRoom
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub controllers: Vec<CtrlId>,
    pub game: RoomGame,
    pub team1_mmr: i64,
    pub team2_mmr: i64,
    pub state: i64,
    pub match_id: String,
}

/// a PlayFab match member (Matches: match id -> {members: [{entity, team_id}]})
#[derive(Clone, Debug, PartialEq)]
pub struct MatchMember {
    pub entity: String,
    pub team_id: String,
}

#[derive(Clone, Debug)]
pub struct DuelState {
    /// Rooms (ST_DuelRoom)
    pub rooms: Vec<Room>,
    /// UnhandledControllers
    pub unhandled: Vec<CtrlId>,
    /// Matches (PlayFab); offline: empty (TF: the local stand-in)
    pub matches: BTreeMap<String, Vec<MatchMember>>,
    pub time_until_map_change: f64,
    /// GameSession AllowsJoin
    pub join_allowed: bool,
}

impl DuelState {
    pub fn new(dd: &DuelData) -> DuelState {
        DuelState {
            rooms: Vec::new(),
            unhandled: Vec::new(),
            matches: BTreeMap::new(),
            time_until_map_change: dd.time_until_map_change,
            join_allowed: true,
        }
    }
}

impl GameMode {
    pub fn duel(&self) -> &DuelState {
        match &self.ext {
            ModeExt::Duel(s) | ModeExt::Tf(s) => s,
            _ => panic!("not a room mode"),
        }
    }
    fn duel_mut(&mut self) -> &mut DuelState {
        match &mut self.ext {
            ModeExt::Duel(s) | ModeExt::Tf(s) => s,
            _ => panic!("not a room mode"),
        }
    }
    pub fn duel_data(&self) -> &DuelData {
        match &self.d.ext {
            ModeDataExt::Duel(d) | ModeDataExt::Tf(d) => d,
            _ => panic!("not a room mode"),
        }
    }
    fn is_tf(&self) -> bool {
        matches!(self.ext, ModeExt::Tf(_))
    }
    fn room_state_v(&self, n: &str) -> i64 {
        self.duel_data().room_state[n]
    }
    fn duel_stage_v(&self, n: &str) -> i64 {
        self.duel_data().stage[n]
    }

    /// the local controller's ReplicatedRoomGame stage == the kismet literal (BP_DuelGameState / BP_Group3v3GameState
    /// ShouldBlockPawnInput)
    pub(crate) fn room_stage_is(&self, c: CtrlId, key: &str) -> bool {
        match &self.ctrls[c].replicated_room_game {
            Some(g) => g.round.stage == self.k.i(key),
            None => false,
        }
    }

    /// TfMode.post_login: local matchmaking stand-in (header): every login joins one local match; every second member
    /// is FreeGuard
    pub(crate) fn tf_post_login(&mut self, c: CtrlId) {
        let fg = self.k.s("tf_freeguard_team_id");
        let name = self.name(c);
        let mem = self.duel_mut().matches.entry(LOCAL_MATCH.to_string()).or_default();
        let tid = if mem.len() % 2 == 1 { fg } else { String::new() };
        mem.push(MatchMember { entity: name, team_id: tid });
        self.ctrls[c].match_id = LOCAL_MATCH.to_string();
    }

    /// BP_DuelGameMode ReceiveTick (ubergraph 633): only while IsMatchInProgress:
    ///   Sequence [0] HandleUnhandledPlayers; for i = 0; i <= Rooms.Length - 1; i++ (length re-read every iteration:
    ///   DestroyRoom removes rooms while looping) TickRoom(i) if Rooms.IsValidIndex(i)
    ///   [1] TimeUntilMapChange -= dt; < 0: GameSession AllowJoin(false) if it AllowsJoin; GetNumPlayers() == 0 and no
    ///   reserved slots: EndMatch
    pub(crate) fn duel_receive_tick(&mut self, dt: f64) {
        if !self.in_progress() {
            return;
        }
        self.handle_unhandled_players();
        let mut i: i64 = 0;
        while i <= self.duel().rooms.len() as i64 - 1 {
            if i >= 0 && (i as usize) < self.duel().rooms.len() {
                self.tick_room(i as usize);
            }
            i += 1;
        }
        let q = self.q();
        let s = self.duel_mut();
        s.time_until_map_change = q(s.time_until_map_change - dt);
        if self.duel().time_until_map_change < 0.0 {
            if self.duel().join_allowed {
                self.duel_mut().join_allowed = false;
                self.ev("disallow_join");
            }
            if self.controllers.is_empty() {
                self.match_state = crate::game_mode::MatchState::WaitingPostMatch;
                self.ev("end_match");
            }
        }
    }

    /// HandleUnhandledPlayers: a controller with a MatchmakingMatchID waits for its PlayFab match
    /// (RequestMatchmakingMatch); with none, only IsPlayInEditor() handles it (here: always, see header). Handled ones
    /// get HandleNewPlayer + ResetController and leave UnhandledControllers.
    fn handle_unhandled_players(&mut self) {
        let mut to_remove = Vec::new();
        for c in self.duel().unhandled.clone() {
            let mid = self.ctrls[c].match_id.clone();
            if !mid.is_empty() && !self.duel().matches.contains_key(&mid) {
                self.ev_args("request_match", vec![("match_id", mid.into())]);
                continue;
            }
            self.handle_new_player(c);
            let who = self.name(c);
            self.ev_args("reset_controller", vec![("who", who.into())]);
            to_remove.push(c);
        }
        for c in to_remove {
            let u = &mut self.duel_mut().unhandled;
            if let Some(p) = u.iter().position(|&o| o == c) {
                u.remove(p);
            }
        }
    }

    fn new_round(stage: i64, start: f64, winner: i64) -> RoundInfo {
        RoundInfo { stage, winner, start_time: start }
    }

    /// HandleNewPlayer: join the first room that is Initializing (RoomState 0) with 0 < Controllers < MaxPeoplePerRoom
    /// and the same MatchID (or IsPlayInEditor); else make a room {Controllers [NewPlayer], Game {0, 0, RoundInfo {Stage
    /// 0, Winner 0, StartTime now + TimeToWaitForPlayers}}, MMR 0/0, RoomState 0}. Then AssignTeam(room, player).
    fn handle_new_player(&mut self, c: CtrlId) {
        let dd = self.duel_data().clone();
        let init = dd.room_state["Initializing"];
        let mid = self.ctrls[c].match_id.clone();
        let mut room_idx: Option<usize> = None;
        for (i, r) in self.duel().rooms.iter().enumerate() {
            let n = r.controllers.len() as i64;
            if n > 0 && n < dd.max_people_per_room && r.state == init && (r.match_id == mid || LOCAL_PLAY) {
                room_idx = Some(i);
                break;
            }
        }
        let who = self.name(c);
        let room_idx = match room_idx {
            None => {
                let st = self.k.i("room_wait_for_players_stage");
                let now = self.now;
                let q = self.q();
                self.duel_mut().rooms.push(Room {
                    controllers: vec![c],
                    game: RoomGame { team1_wins: 0, team2_wins: 0, round: Self::new_round(st, q(now + dd.time_to_wait_for_players), 0) },
                    team1_mmr: 0,
                    team2_mmr: 0,
                    state: init,
                    match_id: mid,
                });
                let i = self.duel().rooms.len() - 1;
                self.ev_args("room_created", vec![("room", (i as i64).into()), ("who", who.into())]);
                i
            }
            Some(i) => {
                self.add_controller_to_room(c, i);
                self.ev_args("room_joined", vec![("room", (i as i64).into()), ("who", who.into())]);
                i
            }
        };
        self.assign_team(room_idx, c);
        self.restart_player(c); // spawn on team assignment (header: UNCONFIRMED)
    }

    /// AssignTeam: SetTeam(Rooms[i].Controllers.Length - 1) (Duel); TF overrides (ubergraph 2587)
    fn assign_team(&mut self, room_idx: usize, c: CtrlId) {
        if self.is_tf() {
            let mid = self.ctrls[c].match_id.clone();
            if let Some(mem) = self.duel().matches.get(&mid).cloned() {
                let name = self.name(c);
                for m in mem {
                    if m.entity == name {
                        let fg = m.team_id == self.k.s("tf_freeguard_team_id");
                        let t = if fg { self.k.i("tf_team_freeguard") } else { self.k.i("tf_team_other") };
                        self.set_team(c, t);
                        return;
                    }
                }
            } else if LOCAL_PLAY {
                let t = self.k.i("tf_pie_team");
                self.set_team(c, t);
            } else {
                let text = format!("AssignTeam: Failed to find match {} for player {}", mid, self.name(c));
                self.ev_args("log", vec![("text", text.into())]);
            }
            return;
        }
        let t = self.duel().rooms[room_idx].controllers.len() as i64 - 1;
        self.set_team(c, t);
    }

    /// AddControllerToRoom: every member and the newcomer add each other to InInstanceWithControllers; append
    fn add_controller_to_room(&mut self, c: CtrlId, room_idx: usize) {
        let arr = self.duel().rooms[room_idx].controllers.clone();
        for &o in &arr {
            if !self.ctrls[c].in_instance_with.contains(&o) {
                self.ctrls[c].in_instance_with.push(o);
            }
            if !self.ctrls[o].in_instance_with.contains(&c) {
                self.ctrls[o].in_instance_with.push(c);
            }
        }
        self.duel_mut().rooms[room_idx].controllers.push(c);
    }

    /// TickRoom: Sequence
    ///   [0] RoomState == Terminated: nothing, else switch RoundInfo.Stage
    ///   [1] copy Game to every member's ReplicatedRoomGame (+ OnRep on the server: the host's client view does that);
    ///       then RoomState == Terminated -> DestroyRoom(i, false)
    fn tick_room(&mut self, i: usize) {
        let term = self.room_state_v("Terminated");
        let r = self.duel().rooms[i].clone();
        let now = self.now;
        if r.state != term {
            let g = r.game;
            let st = g.round.stage;
            if st == self.duel_stage_v("WaitingForPlayers") {
                if g.round.start_time < now {
                    self.destroy_room(i, true);
                    return; // goto 2925: no replication this tick
                }
                if self.is_room_ready_to_start(i) {
                    self.prepare_and_start_room(i);
                }
            } else if st == self.duel_stage_v("WaitingToStart") {
                if g.round.start_time < now {
                    self.restart_room(i);
                }
            } else if st == self.duel_stage_v("RoundStart") {
                if g.round.start_time < now {
                    let rnd = Self::new_round(self.k.i("play_stage"), self.k.f("play_start_time"), 0);
                    self.set_room_game_round(i, rnd);
                }
            } else if st == self.duel_stage_v("RoundPlay") {
                // Team1Living / Team2Living: members alive, by Team == 0 or not
                let (mut t1, mut t2) = (0, 0);
                for &c in &r.controllers {
                    if self.ctrls[c].team == 0 {
                        if self.ctrls[c].alive {
                            t1 += 1;
                        }
                    } else if self.ctrls[c].alive {
                        t2 += 1;
                    }
                }
                if t1 == 0 || t2 == 0 {
                    self.award_round_win(i, if t1 > t2 { 0 } else { 1 }); // SelectInt(0, 1, Team1Living > Team2Living)
                }
            } else if st == self.duel_stage_v("RoundEnd") {
                if g.round.start_time < now {
                    self.restart_room(i);
                }
            }
        }
        let r = self.duel().rooms[i].clone();
        for &c in &r.controllers {
            self.ctrls[c].replicated_room_game = Some(r.game);
        }
        if r.state == term {
            self.destroy_room(i, false);
        }
    }

    /// IsRoomReadyToStart: Controllers.Length == MaxPeoplePerRoom and every member AreStatsAvailable (offline: true)
    fn is_room_ready_to_start(&self, i: usize) -> bool {
        self.duel().rooms[i].controllers.len() as i64 == self.duel_data().max_people_per_room
    }

    /// PrepareAndStartRoom: ComputeTeamMMR (1000 / 1000, Succeeded false; TF: no PlayFab stats -> 0 / 0); RoomState =
    /// Ready; RoundInfo {Stage 1, 0, now + 10}
    fn prepare_and_start_room(&mut self, i: usize) {
        let (m1, m2, st) = (self.k.i("team1_mmr"), self.k.i("team2_mmr"), self.k.i("prepare_room_state"));
        {
            let r = &mut self.duel_mut().rooms[i];
            r.team1_mmr = m1;
            r.team2_mmr = m2;
            r.state = st;
        }
        let rnd = Self::new_round(self.k.i("prepare_stage"), (self.q())(self.now + self.k.f("prepare_wait_s")), 0);
        self.set_room_game_round(i, rnd);
        self.ev_args("room_ready", vec![("room", (i as i64).into())]);
        if self.is_tf() {
            let r = &mut self.duel_mut().rooms[i];
            r.team1_mmr = 0;
            r.team2_mmr = 0;
        }
    }

    /// SetRoomGameRound: Game = {Team1Wins, Team2Wins (kept), RoundInfo = NewRound}
    fn set_room_game_round(&mut self, i: usize, rnd: RoundInfo) {
        self.duel_mut().rooms[i].game.round = rnd;
        self.ev_args(
            "stage",
            vec![("room", (i as i64).into()), ("stage", rnd.stage.into()), ("start_time", rnd.start_time.into())],
        );
    }

    /// RestartRoom: Sequence [0] destroy every MordhauActor owned by a member (IsAnyInstanceOwner: dropped weapons...)
    /// [1] UnpossessAndDestroyPawn(member, RestartPlayer = true) [2] RoundInfo {Stage 2, 0, now + 5}
    fn restart_room(&mut self, i: usize) {
        self.ev_args("destroy_owned_actors", vec![("room", (i as i64).into())]);
        let rp = self.k.b("restart_restart_player");
        for c in self.duel().rooms[i].controllers.clone() {
            self.unpossess_and_destroy_pawn(c, rp);
        }
        let rnd = Self::new_round(self.k.i("restart_stage"), (self.q())(self.now + self.k.f("restart_wait_s")), 0);
        self.set_room_game_round(i, rnd);
    }

    /// AwardRoundWin(Winner): wins += 1 for the winner's team; RoundInfo {Stage 4, Winner, now + 2}; SetRoomGame;
    /// Team1Wins == RoundsToWin or Team2Wins == RoundsToWin -> FinishRoomGame(Team1Wins == RoundsToWin ? 0 : 1)
    fn award_round_win(&mut self, i: usize, winner: i64) {
        let g = self.duel().rooms[i].game;
        let w1 = (g.team1_wins + if winner == 0 { 1 } else { 0 }) & 0xff; // Add_ByteByte
        let w2 = (g.team2_wins + if winner == 1 { 1 } else { 0 }) & 0xff;
        let rnd = Self::new_round(self.k.i("end_stage"), (self.q())(self.now + self.k.f("end_wait_s")), winner & 0xff);
        self.duel_mut().rooms[i].game = RoomGame { team1_wins: w1, team2_wins: w2, round: rnd };
        self.ev_args(
            "round_won",
            vec![("room", (i as i64).into()), ("winner", winner.into()), ("team1_wins", w1.into()), ("team2_wins", w2.into())],
        );
        let rtw = self.duel_data().rounds_to_win;
        if w1 == rtw || w2 == rtw {
            self.finish_room_game(i, if w1 == rtw { 0 } else { 1 });
        }
    }

    /// FinishRoomGame(Winner) (Duel): SetRoomTerminated; the member on the winning team gets AddScore(100000);
    /// AwardDuelMMR, NewMMR (+OnRep -> end screen), TriggerRewardDropForPlayer for both (PlayFab: events only).
    /// TF (ubergraph 3146): every member AddScore(Team == Winner ? 100000 : 0), AwardTeamfightMMR (events)
    fn finish_room_game(&mut self, i: usize, winner: i64) {
        self.set_room_terminated(i);
        let members = self.duel().rooms[i].controllers.clone();
        if self.is_tf() {
            for c in members {
                let won = self.ctrls[c].team == winner;
                let s = if won { self.k.i("tf_winner_score") } else { self.k.i("tf_loser_score") };
                self.add_score(c, s);
                let who = self.name(c);
                self.ev_args("award_mmr", vec![("who", who.into()), ("won", won.into())]);
            }
            self.ev_args("match_finished", vec![("room", (i as i64).into()), ("winner_team", winner.into())]);
            return;
        }
        let mut won = None;
        let mut lost = None;
        for c in members {
            if self.ctrls[c].team == winner {
                won = Some(c);
                let s = self.k.i("winner_score");
                self.add_score(c, s);
            } else {
                lost = Some(c);
            }
        }
        let wn = won.map(|c| self.name(c)).unwrap_or_default();
        let ln = lost.map(|c| self.name(c)).unwrap_or_default();
        self.ev_args(
            "match_finished",
            vec![("room", (i as i64).into()), ("winner_team", winner.into()), ("winner", wn.clone().into()), ("loser", ln.clone().into())],
        );
        if won.is_some() && lost.is_some() {
            self.ev_args("award_mmr", vec![("winner", wn.into()), ("loser", ln.into())]);
        }
    }

    /// SetRoomTerminated: RoomState = 2
    fn set_room_terminated(&mut self, i: usize) {
        let st = self.k.i("terminated_room_state");
        self.duel_mut().rooms[i].state = st;
    }

    /// DestroyRoom(i, KickPlayers): SetRoomTerminated; kick members if asked; Matches.Remove(MatchID); Rooms.Remove(i)
    fn destroy_room(&mut self, i: usize, kick: bool) {
        self.set_room_terminated(i);
        if kick {
            for c in self.duel().rooms[i].controllers.clone() {
                let who = self.name(c);
                self.ev_args("kick", vec![("who", who.into())]);
            }
        }
        let mid = self.duel().rooms[i].match_id.clone();
        self.duel_mut().matches.remove(&mid);
        self.ev_args("room_destroyed", vec![("room", (i as i64).into()), ("kick", kick.into())]);
        self.duel_mut().rooms.remove(i);
    }

    /// HandlePlayerLeaving: drop from UnhandledControllers; find the non-terminated room holding the player; if the
    /// room is Ready, look for another member on the leaver's team - if there is one, PenalizeForLeavingActiveGame
    /// (empty function) and remove the leaver, else FinishRoomGame(the other team); an Initializing room is destroyed.
    pub(crate) fn handle_player_leaving(&mut self, c: CtrlId) {
        {
            let u = &mut self.duel_mut().unhandled;
            if let Some(p) = u.iter().position(|&o| o == c) {
                u.remove(p);
            }
        }
        let term = self.room_state_v("Terminated");
        let found = self.duel().rooms.iter().position(|r| r.state != term && r.controllers.contains(&c));
        let Some(found) = found else { return };
        let st = self.duel().rooms[found].state;
        if st == self.room_state_v("Ready") {
            let team = self.ctrls[c].team;
            let other_team_member = self.duel().rooms[found].controllers.iter().any(|&o| o != c && self.ctrls[o].team == team);
            if other_team_member {
                let arr = &mut self.duel_mut().rooms[found].controllers;
                if let Some(p) = arr.iter().position(|&o| o == c) {
                    arr.remove(p);
                }
            } else {
                self.finish_room_game(found, if team == 0 { 1 } else { 0 });
            }
        } else if st == self.room_state_v("Initializing") {
            self.destroy_room(found, true);
        }
    }

    pub fn room_of(&self, c: CtrlId) -> Option<usize> {
        self.duel().rooms.iter().position(|r| r.controllers.contains(&c))
    }
}

// ==== FL (BP_FrontlineGameMode : BP_MordhauGameMode, game state BP_FrontlineGameState) =============================
// Two teams, team scores are tickets, capture points (ControlPoint) with prerequisites and point-bound spawns. Ported
// from the bytecode (scripts/kismet; statement indices in brackets; literals "fl_*"):
//   BP_FrontlineGameState ReceiveBeginPlay [1695]: authority -> TeamScores[0] = Team1StartingTickets, [1] =
//     Team2StartingTickets (a BP_PushModeInfo actor switches to push mode: not ported)
//   BP_FrontlineGameState ReceiveTick [2339] (authority, IsMatchInProgress): Team1CapturePoints == 0 or
//     Team2CapturePoints == 0 -> (not push) [965] Team2CapturePoints == 0 ? SetTeamScore(1, 0) : SetTeamScore(0, 0);
//     else TicketDrainCounter += dt, >= TicketDrainInterval -> -= interval, DrainTickets [2884]
//   DrainTickets: the team holding fewer points loses TicketDrainAmount (equal: both lose 1 [975]); both <= 0 ->
//     EndMatch, MatchEndInfo Draw [420]; then SetTeamScore(0, max(T1, 0)), SetTeamScore(1, max(T2, 0)) [672]
//   BP_FrontlineGameMode OnKilled [2684]: parent, then the killed player's team (0 / 1): SetTeamScore(team,
//     max(TeamScores[team] - DeathTicketCost, 0))
//   OnTeamScoreChanged [1579]: TeamScores[Team] == 0 and in progress -> EndMatch; MatchEndInfo winner = the other team
//   MatchTimeRanOut [10]: parent; [0] == [1] -> Draw (WinnerTeam 0); else the team with more tickets wins
// Not ported (UNCONFIRMED / out of scope): push mode, objectives, the spawn select screen, vehicles / horses, the HUD.
#[derive(Clone, Debug, Default)]
pub struct FlState {
    /// BP_FrontlineGameState TicketDrainCounter
    pub ticket_drain_counter: f64,
}

impl GameMode {
    pub fn fl_data(&self) -> &FlData {
        match &self.d.ext {
            ModeDataExt::Fl(f) => f,
            _ => panic!("not a Frontline mode"),
        }
    }
    fn fl_mut(&mut self) -> &mut FlState {
        match &mut self.ext {
            ModeExt::Fl(s) => s,
            _ => panic!("not a Frontline mode"),
        }
    }

    /// FlMode._init: ReceiveBeginPlay [1730] / [1786] tickets; the placed capture points, prerequisites and spawn
    /// points resolved by actor name, each added (BeginPlay) in placement order
    pub(crate) fn fl_init(&mut self) {
        let fd = self.fl_data().clone();
        self.team_scores[0] = fd.team1_starting_tickets;
        self.team_scores[1] = fd.team2_starting_tickets;
        let idx = |n: &str| fd.control_points.iter().position(|c| c.name == n);
        let sidx = |starts: &Vec<crate::data::PlayerStartDef>, n: &str| starts.iter().position(|s| s.name == n);
        let mut cps = Vec::new();
        for def in &fd.control_points {
            let mut cp = ControlPoint::new(def.clone());
            cp.team1_prerequisites = def.team1_prerequisites.iter().map(|n| idx(n)).collect();
            cp.team2_prerequisites = def.team2_prerequisites.iter().map(|n| idx(n)).collect();
            cp.spawn_points = def.spawn_points.iter().map(|n| sidx(&self.starts, n)).collect();
            cps.push(cp);
        }
        for cp in cps {
            self.add_control_point(cp);
        }
    }

    pub(crate) fn fl_receive_tick(&mut self, dt: f64) {
        if !self.in_progress() {
            return;
        }
        if self.team1_capture_points == 0 || self.team2_capture_points == 0 {
            let v = self.k.f("fl_no_points_score");
            if self.team2_capture_points == 0 {
                self.set_team_score(1, v);
            } else {
                self.set_team_score(0, v);
            }
            return;
        }
        let iv = self.fl_data().ticket_drain_interval;
        let q = self.q();
        let c = q(self.fl_mut().ticket_drain_counter + dt);
        self.fl_mut().ticket_drain_counter = c;
        if c >= iv {
            self.fl_mut().ticket_drain_counter = q(c - iv);
            self.drain_tickets();
        }
    }

    /// BP_FrontlineGameState DrainTickets (header)
    pub fn drain_tickets(&mut self) {
        let (mut t1, mut t2) = (self.team_scores[0], self.team_scores[1]);
        let amt = self.fl_data().ticket_drain_amount;
        let q = self.q();
        if self.team1_capture_points > self.team2_capture_points {
            t2 = q(t2 - amt);
        } else if self.team2_capture_points > self.team1_capture_points {
            t1 = q(t1 - amt);
        } else {
            let tie = self.k.f("fl_tie_drain");
            t1 = q(t1 - tie);
            t2 = q(t2 - tie);
        }
        let floor_v = self.k.f("fl_ticket_floor");
        if t1 <= floor_v && t2 <= floor_v {
            self.end_match();
            self.set_end_info(None, 0, 0.0, true);
        }
        self.set_team_score(0, maxf(t1, floor_v));
        self.set_team_score(1, maxf(t2, floor_v));
    }

    /// BP_FrontlineGameMode OnKilled (header): parent first (dispatch), then the death ticket
    pub(crate) fn fl_on_killed(&mut self, killed: Option<CtrlId>) {
        let Some(kd) = killed else { return };
        let t = self.ctrls[kd].team;
        if t == 0 || t == 1 {
            let v = maxf((self.q())(self.team_scores[t as usize] - self.fl_data().death_ticket_cost), self.k.f("fl_ticket_floor_on_death"));
            self.set_team_score(t, v);
        }
    }

    pub(crate) fn fl_on_team_score_changed(&mut self, team: i64, _old: f64) {
        if !(0..=1).contains(&team) {
            return;
        }
        if self.team_scores[team as usize] == self.k.f("fl_out_of_tickets") && self.in_progress() {
            self.end_match();
            let other = if team == 0 { 1 } else { 0 };
            let s = self.team_scores[other as usize];
            self.set_end_info(None, other, s, false);
        }
    }

    pub(crate) fn fl_match_time_ran_out(&mut self) {
        let (s0, s1) = (self.team_scores[0], self.team_scores[1]);
        if s0 == s1 {
            self.set_end_info(None, 0, 0.0, true);
        } else if s0 > s1 {
            self.set_end_info(None, 0, s0, false);
        } else {
            self.set_end_info(None, 1, s1, false);
        }
    }
}

/// a capture point of a Skirmish round with this team as its captor (tests / hosts): helper for the NO_TEAM check
pub fn is_owned(cp: &ControlPoint) -> bool {
    cp.owning_team != NO_TEAM
}

#[allow(dead_code)]
fn _arg_unused(_a: Arg) {}

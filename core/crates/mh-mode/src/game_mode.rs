//! MordhauGameMode (godot/game/mode/mordhau_game_mode.gd): the base every mode port extends, as every mode Blueprint
//! extends BP_MordhauGameMode : AMordhauGameMode (native) : AGameMode. Engine-neutral: plain data + tick(dt); the host
//! logs controllers in, reports deaths / damage / pawn locations / capture-area overlaps, and acts on the events that
//! come out (spawn a pawn at a start, possess, destroy, kill feed, match end). The game state's server-side fields
//! (TeamScores, ElapsedTime, MatchEndInfo, bAllowSpawning) live here too: the port keeps GameMode + GameState together.
//!
//! Class tree (extract/decomp Blueprint parents) -> `ModeExt`, the per-mode state; each virtual below dispatches on it
//! exactly where the mode's Blueprint overrides the function (modes.rs):
//!   MordhauGameMode   AMordhauGameMode (extract/native/decomp/AMordhauGameMode.cpp) + BP_MordhauGameMode
//!     Ffa             BP_DeathmatchGameMode       : BP_MordhauGameMode
//!     Tdm             BP_TeamDeathmatchGameMode   : BP_MordhauGameMode
//!     Skm             BP_SkirmishGameMode         : BP_MordhauGameMode
//!     Duel            BP_DuelGameMode             : BP_MordhauGameMode
//!       Tf            BP_Group3v3GameMode         : BP_DuelGameMode
//!     Fl              BP_FrontlineGameMode        : BP_MordhauGameMode
//!
//! Native sources (constants: ModeData / consts::mode):
//!   tick          AMordhauGameMode::Tick rva=0x15ab000: Super::Tick (AActor::Tick -> ReceiveTick = the Blueprint tick,
//!                 AGameMode match state), one spawn-queue step, then InProgress with MatchDurationMax >= 1 and
//!                 ElapsedTime >= MatchDurationMax -> MatchTimeRanOut
//!   scoring       AMordhauGameMode::OnKilled_Implementation rva=0x159d6f0 (+ BP_MordhauGameMode OnKilled -> AddKillNotify);
//!                 AMordhauPlayerState::AddScore rva=0x15c3bf0 -> OnScoreChanged; AddTeamScore 0x1582410 / SetTeamScore
//!                 0x15a7960 -> OnTeamScoreChanged
//!   time limit    native AMordhauGameMode::MatchTimeRanOut_Implementation 0x159d1c0 = bMatchTimeRanOut + vtable +0x830
//!                 (AGameMode::EndMatch, resolved from the vftable)
//!   respawn       OnKilled: NextRespawnTime = now + PlayerRespawnTime (waves: ceil(now / t) * t); bots:
//!                 AMordhauAIController::Tick 0x15185b0 (bAutoRespawn, no pawn -> bWantsRespawn; NextRespawnTime < now and
//!                 ControllerCanRestart -> RestartPlayer); players: AMordhauPlayerController::CanAskForSpawn 0x15c8a90
//!                 (NextRespawnTime < server time, LastAskedForSpawnTime + 1 < real time or never asked) -> AskForSpawn
//!   restart gate  AMordhauGameMode::ControllerCanRestart_Implementation 0x1587e50: a MordhauPlayerState with Team != -1
//!                 and GameState bAllowSpawning (0x141587f2d `cmp byte ptr [rbx + 0x696], 0`); RestartPlayer
//!                 (AMordhauGameMode::RestartPlayer 0x15a6d50) itself does not check it
//!   spawn point   AMordhauGameMode::ChoosePlayerStart_Implementation 0x15876e0 / IsSpawnpointAllowed_Implementation 0x159c1b0 /
//!                 AMordhauPlayerStart::IsAllowedSpawnFor_Implementation 0x15dee60 / GetSpawnpointPreference_Implementation 0x1599660 /
//!                 AMordhauPlayerStart::GetSpawnPreferenceFor_Implementation 0x15da470
//!   match start   AMordhauGameMode::ReadyToStartMatch_Implementation 0x15a42b0 (WarmupEnd -1 or <= now, NumPlayers + NumBots > 0),
//!                 HandleMatchHasStarted 0x1599ee0 (bIsScoringDisabled = false); HandleMatchHasEnded 0x1599e10 sets it back
//!   teams         AMordhauGameMode::RequestedAssignTeam_Implementation 0x15a6070 (parties not ported)
//!   counts        AMordhauGameState::GetPlayerCountsPerTeam 0x1598050 (server: the world's controllers)
//!   PlayerId      AGameSession::RegisterPlayer rva=0x30e7fe0 (NextPlayerID++) when a session exists, else a login counter
//! Port adaptations / not ported (UNCONFIRMED, as in the reference):
//!   - AGameState::DefaultTimer (++ElapsedTime once a second while in progress) and the AGameMode::Tick match-state order
//!     are UE 4.26 engine code (not disassembled); EndMatch -> WaitingPostMatch likewise
//!   - the player's team at login: an auto-assign request (RequestedAssignTeam(-2)); in the game
//!     AMordhauPlayerController::ServerSetPartyInfo_Implementation 0x15f9630 asks when the GameState's auto-assign flag
//!     is set and AMordhauGameState::BeginPlay 0x1584900 clears it on a non-dedicated server (team select screen)
//!   - the player's spawn screen: the player asks for a spawn as soon as CanAskForSpawn allows (no button press)
//!   - UWorld::EncroachingBlockingGeometry in the spawn preference (-10) tests only live pawns' capsules (radius 50 cm,
//!     half height 96 cm) against the start; level geometry is not tested
//!   - spawn token distance (frontline tokens; arena starts serialize none) is equal for every start, so it never decides
//!   - start iteration order = the map package's export order (TActorIterator order not reproduced)
//!   - the bot "thanks" voice line of assists; map vote / ChangeLevel after the match (events only)
//!   - ShouldBlockPawnInput's warmup branch (AMordhauGameState::ShouldBlockPawnInput_Implementation 0x15a8340): no warmup
//!   - GetPlayerCountsPerTeam's bOnlyWithValidProfiles flag (AMordhauPlayerController byte) is taken as set
//!   - bIsAlive: the native setter is not ported; the host reports deaths / respawns (set_alive)
//!
//! Floats: GDScript `float` = f64 here; Godot Vector3 = mordhau_core::ue::FVector (f32 components), as the reference.

use crate::consts::mode as K;
use crate::control_point::{ControlPoint, CpCtx};
use crate::data::{ModeData, ModeDataExt, PlayerStartDef};
use crate::event::{Arg, Ev};
use crate::kismet::Kismet;
use crate::br::BrState;
use crate::horde::HordeState;
use crate::modes::{DuelState, FlState, SkmState};
use crate::spawn_queue::{SpawnQueue, SpawnStep};
use mordhau_core::ue::{clampf, maxf, minf, CrtRand, FVector};
use std::sync::Arc;

pub type CtrlId = usize;

/// AMordhauCharacter::AMordhauCharacter rva=0x1524d90 stores CapsuleRadius 50 (0x42480000) and CapsuleHalfHeight 96
/// (0x42c00000) (godot/game/character/mordhau_character.gd CAPSULE_RADIUS / CAPSULE_HALF_HEIGHT); CM = 0.01 m per cm
/// (DESIGN.md §4 coordinates)
pub const CAPSULE_RADIUS: f64 = 50.0;
pub const CAPSULE_HALF_HEIGHT: f64 = 96.0;
pub const CM: f64 = 0.01;

/// Numeric model (the exe/compat split of mordhau-core / mh-character):
///   Exe        the shipped exe: Kismet / native floats are binary32, every listed operation rounds to f32 (`q`), the
///              world clock is UWorld TimeSeconds (an f32 accumulated by DeltaSeconds). Default (`GameMode::new`).
///   Reference  the GDScript reference bit for bit (f64 GDScript floats; clock accumulated in f64): the golden traces.
///              `GameMode::new_reference`, behind the `reference_compat` feature.
/// Exe-exact sites (each `q(..)`): the clock, scores / team scores / tickets, respawn times, assists and damage
/// history, spawn preference, round / room times, the control point's progress and score times. Integer rules are the
/// same in both models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    Exe,
    Reference,
}

impl Default for Precision {
    fn default() -> Self {
        Precision::Exe
    }
}

impl Precision {
    /// the rounding of one float operation: binary32 in Exe, identity in Reference
    pub fn q(self) -> fn(f64) -> f64 {
        match self {
            Precision::Exe => mordhau_core::ue::f32r,
            Precision::Reference => |x| x,
        }
    }
}

/// AGameMode MatchState (EnteringMap is passed at construction)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchState {
    WaitingToStart,
    InProgress,
    WaitingPostMatch,
}

impl MatchState {
    pub fn name(self) -> &'static str {
        match self {
            MatchState::WaitingToStart => "WaitingToStart",
            MatchState::InProgress => "InProgress",
            MatchState::WaitingPostMatch => "WaitingPostMatch",
        }
    }
}

/// AMordhauCharacter DamageHistory entry: {who, t, damage}
#[derive(Clone, Debug, PartialEq)]
pub struct DamageEntry {
    pub who: CtrlId,
    pub t: f64,
    pub damage: f64,
}

/// STRUCT_DuelRoundInfo / Skirmish RoundInfo {Stage, Winner, StartTime}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoundInfo {
    pub stage: i64,
    pub winner: i64,
    pub start_time: f64,
}

/// ST_DuelRoomGame {Team1Wins, Team2Wins, RoundInfo}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoomGame {
    pub team1_wins: i64,
    pub team2_wins: i64,
    pub round: RoundInfo,
}

/// a controller + its MordhauPlayerState (+ the Blueprint controller fields the modes read)
#[derive(Clone, Debug)]
pub struct Ctrl {
    pub name: String,
    /// an AMordhauAIController (takes the AI tick)
    pub is_bot: bool,
    /// APlayerState PlayerId (post_login: the session's RegisterPlayer id, else login order)
    pub player_id: i64,
    /// Team (-1 until assigned)
    pub team: i64,
    /// bIsAlive
    pub alive: bool,
    /// Score (float)
    pub score: f64,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    pub has_pawn: bool,
    /// pawn location (Godot metres; the host keeps it current)
    pub location: FVector,
    /// AMordhauCharacter DamageHistory
    pub damage_history: Vec<DamageEntry>,
    /// AMordhauCharacter CurrentCapturePoint +0xd80 (an index into the mode's control points)
    pub capture_point: Option<usize>,
    /// AMordhauCharacter CurrentCapturePointTime +0xd88
    pub capture_point_time: f64,
    /// AMordhauPlayerController +0x9c0 / AMordhauAIController +0x568 NextRespawnTime
    pub next_respawn_time: f64,
    /// AMordhauAIController bWantsRespawn
    pub wants_respawn: bool,
    /// AMordhauPlayerController LastAskedForSpawnTime
    pub last_asked_for_spawn: f64,
    /// MatchmakingMatchID ("" offline)
    pub match_id: String,
    /// BP_DuelPlayerController NewMMR
    pub new_mmr: i64,
    /// InInstanceWithControllers (AddControllerToRoom)
    pub in_instance_with: Vec<CtrlId>,
    /// ReplicatedRoomGame (copied every TickRoom); None = HasReplicatedRoomGame false
    pub replicated_room_game: Option<RoomGame>,
}

impl Ctrl {
    pub fn new(name: &str, bot: bool) -> Ctrl {
        Ctrl {
            name: name.to_string(),
            is_bot: bot,
            player_id: 0,
            team: -1,
            alive: false,
            score: 0.0,
            kills: 0,
            deaths: 0,
            assists: 0,
            has_pawn: false,
            location: FVector::ZERO,
            damage_history: Vec::new(),
            capture_point: None,
            capture_point_time: 0.0,
            next_respawn_time: 0.0,
            wants_respawn: false,
            last_asked_for_spawn: 0.0,
            match_id: String::new(),
            new_mmr: 0,
            in_instance_with: Vec::new(),
            replicated_room_game: None,
        }
    }
    pub fn has_replicated(&self) -> bool {
        self.replicated_room_game.is_some()
    }
}

/// STRUCT_MatchEndInfo {Winner (player name, "" for none), WinnerTeam, WinnerScore, OtherScore, Draw}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatchEndInfo {
    pub winner: String,
    pub winner_team: i64,
    pub winner_score: f64,
    pub other_score: f64,
    pub draw: bool,
}

/// the HUD command of HandleMatchEndInfo (ShowMatchResult)
#[derive(Clone, Debug, PartialEq)]
pub struct MatchResult {
    pub victory: bool,
    pub text: String,
    pub subtext: String,
}

/// per-mode state of the mode's own Blueprints (the reference's subclasses' variables)
#[derive(Clone, Debug)]
pub enum ModeExt {
    Ffa,
    Tdm,
    Skm(SkmState),
    Duel(DuelState),
    Tf(DuelState),
    Fl(FlState),
    Horde(HordeState),
    Br(BrState),
}

#[derive(Clone, Debug)]
pub struct GameMode {
    pub d: ModeData,
    pub k: Arc<Kismet>,
    pub rng: CrtRand,
    /// GetTimeSeconds (= GetServerWorldTimeSeconds: one process)
    pub now: f64,
    pub match_state: MatchState,
    /// AGameState ElapsedTime (int seconds)
    pub elapsed_time: i64,
    timer: f64,
    /// bIsScoringDisabled
    pub scoring_disabled: bool,
    /// bMatchTimeRanOut
    pub match_time_ran_out: bool,
    pub match_end_info: Option<MatchEndInfo>,
    /// GameState bAllowSpawning
    pub allow_spawning: bool,
    /// every controller ever logged in (stable ids); `controllers` is the GameState PlayerArray
    pub ctrls: Vec<Ctrl>,
    pub controllers: Vec<CtrlId>,
    pub spawns: SpawnQueue,
    pub events: Vec<Ev>,
    /// AMordhauGameState TeamScores (TeamCount entries, 0 at start - UNCONFIRMED init)
    pub team_scores: Vec<f64>,
    /// AMordhauGameState AllCapturePoints (BeginPlay collects every AControlPoint)
    pub control_points: Vec<ControlPoint>,
    pub team1_capture_points: i64,
    pub team2_capture_points: i64,
    next_id: i64,
    /// the map's player starts (ModeData.starts; AControlPoint::UpdateSpawns writes bIsSpawnDisabled on them)
    pub starts: Vec<PlayerStartDef>,
    pub ext: ModeExt,
    pub precision: Precision,
}

impl GameMode {
    /// the mode for its data (ModeTable row "mode": FfaMode.new(d), ...). `rng`: the world's CRT rand() stream
    /// (UeRand header: one stream shared by every caller in a world).
    pub fn new(d: ModeData, k: Arc<Kismet>, rng: CrtRand) -> GameMode {
        GameMode::with_precision(d, k, rng, Precision::Exe)
    }

    /// the GDScript reference reproduced bit for bit (golden traces)
    #[cfg(feature = "reference_compat")]
    pub fn new_reference(d: ModeData, k: Arc<Kismet>, rng: CrtRand) -> GameMode {
        GameMode::with_precision(d, k, rng, Precision::Reference)
    }

    fn with_precision(d: ModeData, k: Arc<Kismet>, rng: CrtRand, precision: Precision) -> GameMode {
        let team_scores = vec![0.0; d.state.team_count.max(0) as usize];
        let ext = match &d.ext {
            ModeDataExt::Ffa { .. } => ModeExt::Ffa,
            ModeDataExt::Tdm { .. } => ModeExt::Tdm,
            ModeDataExt::Skm(s) => ModeExt::Skm(SkmState::new(s)),
            ModeDataExt::Duel(dd) => ModeExt::Duel(DuelState::new(dd)),
            ModeDataExt::Tf(dd) => ModeExt::Tf(DuelState::new(dd)),
            ModeDataExt::Fl(_) => ModeExt::Fl(FlState::default()),
            // one BP_HordeSpawn until the host reports the map's spawners (set_horde_spawners)
            ModeDataExt::Horde(h) => ModeExt::Horde(HordeState::new(h, 1)),
            ModeDataExt::Br(_) => ModeExt::Br(BrState::default()),
        };
        let mut m = GameMode {
            scoring_disabled: d.scoring.b_is_scoring_disabled,
            allow_spawning: d.state.b_allow_spawning,
            starts: d.starts.clone(),
            d,
            k,
            rng,
            now: 0.0,
            match_state: MatchState::WaitingToStart,
            elapsed_time: 0,
            timer: 0.0,
            match_time_ran_out: false,
            match_end_info: None,
            ctrls: Vec::new(),
            controllers: Vec::new(),
            spawns: SpawnQueue::default(),
            events: Vec::new(),
            team_scores,
            control_points: Vec::new(),
            team1_capture_points: 0,
            team2_capture_points: 0,
            next_id: 0,
            ext,
            precision,
        };
        if let ModeExt::Fl(_) = m.ext {
            m.fl_init();
        }
        m
    }

    /// the rounding of one float operation in this mode's numeric model
    #[inline]
    pub fn q(&self) -> fn(f64) -> f64 {
        self.precision.q()
    }

    pub(crate) fn ev(&mut self, kind: &'static str) -> &mut Ev {
        let t = self.now;
        self.events.push(Ev::new(kind).with("t", t));
        self.events.last_mut().unwrap()
    }

    pub(crate) fn ev_args(&mut self, kind: &'static str, args: Vec<(&'static str, Arg)>) {
        let e = self.ev(kind);
        e.args.extend(args);
    }

    pub fn drain(&mut self) -> Vec<Ev> {
        std::mem::take(&mut self.events)
    }

    pub fn by_name(&self, n: &str) -> Option<CtrlId> {
        self.controllers.iter().copied().find(|&c| self.ctrls[c].name == n)
    }

    pub fn in_progress(&self) -> bool {
        self.match_state == MatchState::InProgress
    }

    pub fn has(&self, c: CtrlId) -> bool {
        self.controllers.contains(&c)
    }

    pub(crate) fn name(&self, c: CtrlId) -> String {
        self.ctrls[c].name.clone()
    }

    // ---- login / logout ------------------------------------------------------------------------------------------
    /// a new controller object (not logged in yet): the host's PlayerController / AIController spawn
    pub fn add_ctrl(&mut self, name: &str, bot: bool) -> CtrlId {
        self.ctrls.push(Ctrl::new(name, bot));
        self.ctrls.len() - 1
    }

    /// add_ctrl + post_login
    pub fn login(&mut self, name: &str, bot: bool, session_player_id: i64) -> CtrlId {
        let c = self.add_ctrl(name, bot);
        self.post_login(c, session_player_id);
        c
    }

    /// AGameModeBase::PostLogin (PlayerArray) then the Blueprint K2_PostLogin. PlayerId: the one the game session gave
    /// the player state at login (AGameSession::RegisterPlayer rva=0x30e7fe0, NextPlayerID++) when a session exists
    /// (session_player_id >= 0); offline there is no session object, so the mode keeps the same login-order counter.
    /// TfMode overrides (local matchmaking stand-in, modes.rs).
    pub fn post_login(&mut self, c: CtrlId, session_player_id: i64) {
        if let ModeExt::Tf(_) = self.ext {
            self.tf_post_login(c);
        }
        if session_player_id >= 0 {
            self.ctrls[c].player_id = session_player_id;
        } else {
            self.ctrls[c].player_id = self.next_id;
            self.next_id += 1;
        }
        self.controllers.push(c);
        if self.ctrls[c].is_bot {
            self.ctrls[c].wants_respawn = self.d.bot_auto_respawn;
        }
        self.k2_post_login(c);
    }

    /// overridden by BP_DuelGameMode K2_PostLogin; otherwise the player's auto-assign request (header: UNCONFIRMED)
    pub fn k2_post_login(&mut self, c: CtrlId) {
        match &mut self.ext {
            ModeExt::Duel(s) | ModeExt::Tf(s) => s.unhandled.push(c),
            _ => {
                if !self.ctrls[c].is_bot {
                    self.request_assign_team(c, -2);
                }
            }
        }
        // the Horde / Battle Royale K2_PostLogin call the parent first (ubergraph @9501 / @3445)
        match self.ext {
            ModeExt::Horde(_) => self.horde_post_login(c),
            ModeExt::Br(_) => self.br_post_login(c),
            _ => {}
        }
    }

    pub fn logout(&mut self, c: CtrlId) {
        self.k2_on_logout(c);
        self.controllers.retain(|&o| o != c);
    }

    /// BP K2_OnLogout (BP_DuelGameMode overrides -> HandlePlayerLeaving)
    pub fn k2_on_logout(&mut self, c: CtrlId) {
        match self.ext {
            ModeExt::Duel(_) | ModeExt::Tf(_) => self.handle_player_leaving(c),
            ModeExt::Horde(_) => self.horde_on_logout(c),
            _ => {}
        }
    }

    /// the host reports deaths and respawns that do not go through on_killed: bIsAlive (the native setter is not
    /// ported - UNCONFIRMED)
    pub fn set_alive(&mut self, c: CtrlId, v: bool) {
        self.ctrls[c].alive = v;
    }

    // ---- tick ----------------------------------------------------------------------------------------------------
    pub fn tick(&mut self, dt: f64) {
        let q = self.q();
        self.now = match self.precision {
            // UWorld::Tick: TimeSeconds += DeltaSeconds (both float) before the tick groups
            Precision::Exe => (self.now as f32 + dt as f32) as f64,
            Precision::Reference => self.now + dt,
        };
        // AGameState::DefaultTimer: once a second, ++ElapsedTime while the match is in progress (engine, UNCONFIRMED)
        self.timer = q(self.timer + dt);
        while self.timer >= 1.0 {
            self.timer = q(self.timer - 1.0);
            if self.in_progress() {
                self.elapsed_time += 1;
            }
        }
        for c in self.controllers.clone() {
            if self.ctrls[c].is_bot {
                self.bot_tick(c);
            } else {
                self.player_tick(c);
            }
        }
        // every AControlPoint actor ticks (AControlPoint::Tick rva=0x1517270), then AMordhauGameState::Tick rva=0x15ab790
        // recounts the points (UpdateCapturePointData); actor tick order vs the game mode: UNCONFIRMED
        let ip = self.in_progress();
        let now = self.now;
        for i in 0..self.control_points.len() {
            let owners = |v: &Vec<Option<usize>>, cps: &Vec<ControlPoint>| -> Vec<i64> {
                v.iter().flatten().map(|&p| cps[p].owning_team).collect()
            };
            let t1 = owners(&self.control_points[i].team1_prerequisites, &self.control_points);
            let t2 = owners(&self.control_points[i].team2_prerequisites, &self.control_points);
            let mut awards = Vec::new();
            {
                let mut cx = CpCtx { ctrls: &mut self.ctrls, starts: &mut self.starts, awards: &mut awards, q: self.precision.q() };
                self.control_points[i].tick(dt, now, ip, &t1, &t2, &mut cx);
            }
            self.apply_awards(awards);
        }
        self.update_capture_point_data();
        self.receive_tick(dt);
        // the Horde actors (graves, chests) and the Demon Invasion game state's timers (order vs the mode: UNCONFIRMED)
        self.horde_actors_tick(dt);
        self.match_state_step();
        self.spawn_queue_step();
        if self.in_progress() && self.d.state.match_duration_max >= 1 && self.elapsed_time >= self.d.state.match_duration_max {
            self.match_time_ran_out_();
        }
    }

    /// a control point's AddScore calls (its add_score hook = AMordhauPlayerState::AddScore)
    fn apply_awards(&mut self, awards: Vec<(CtrlId, i64)>) {
        for (c, s) in awards {
            self.add_score(c, s);
        }
    }

    /// AMordhauGameState BeginPlay rva=0x1584900 adds every AControlPoint to AllCapturePoints and calls
    /// UpdateCapturePointData; the point's own BeginPlay (AControlPoint::BeginPlay rva=0x14f0030) runs too. Its score
    /// events go to AMordhauPlayerState::AddScore (add_score). Returns the point's index.
    pub fn add_control_point(&mut self, cp: ControlPoint) -> usize {
        self.control_points.push(cp);
        let i = self.control_points.len() - 1;
        let mut awards = Vec::new();
        {
            let mut cx = CpCtx { ctrls: &mut self.ctrls, starts: &mut self.starts, awards: &mut awards, q: self.precision.q() };
            self.control_points[i].begin_play(&mut cx);
        }
        self.apply_awards(awards);
        self.update_capture_point_data();
        i
    }

    /// a pawn begins / ends overlapping a point's capture area (the host's overlap query)
    pub fn cp_begin_overlap(&mut self, cp: usize, c: CtrlId) {
        self.control_points[cp].begin_overlap(cp, c, &mut self.ctrls);
    }
    pub fn cp_end_overlap(&mut self, cp: usize, c: CtrlId) {
        self.control_points[cp].end_overlap(cp, c, &mut self.ctrls);
    }
    pub fn point(&self, n: &str) -> Option<usize> {
        self.control_points.iter().position(|cp| cp.name == n)
    }

    /// AMordhauGameState::UpdateCapturePointData rva=0x15aca70: Team1CapturePoints / Team2CapturePoints = the points that
    /// are not bIsHiddenPoint (+0x24a) with OwningTeam (+0x2e0) 0 / 1. The topological progress (push mode) is not
    /// ported (UNCONFIRMED: HUD only).
    pub fn update_capture_point_data(&mut self) {
        self.team1_capture_points = 0;
        self.team2_capture_points = 0;
        for cp in &self.control_points {
            if cp.d.b_is_hidden_point {
                continue;
            }
            if cp.owning_team == 0 {
                self.team1_capture_points += 1;
            } else if cp.owning_team == 1 {
                self.team2_capture_points += 1;
            }
        }
    }

    /// the Blueprint ReceiveTick (Duel, Skirmish, Frontline's game state); nothing in BP_MordhauGameMode
    pub fn receive_tick(&mut self, dt: f64) {
        match self.ext {
            ModeExt::Skm(_) => self.skm_receive_tick(dt),
            ModeExt::Duel(_) | ModeExt::Tf(_) => self.duel_receive_tick(dt),
            ModeExt::Fl(_) => self.fl_receive_tick(dt),
            ModeExt::Horde(_) => self.horde_receive_tick(),
            ModeExt::Br(_) => self.br_receive_tick(),
            _ => {}
        }
    }

    /// AMordhauGameMode::ControllerCanRestart_Implementation 0x1587e50 (Skirmish overrides)
    pub fn controller_can_restart(&self, c: CtrlId) -> bool {
        let base = self.has(c) && self.ctrls[c].team != -1 && self.allow_spawning;
        match &self.ext {
            ModeExt::Skm(s) => base && !s.already_died.contains(&c),
            ModeExt::Br(_) => self.br_controller_can_restart(c, base),
            _ => base,
        }
    }

    /// AMordhauAIController::Tick 0x15185b0: no team -> RequestedAssignTeam(BehaviorProfile DefaultTeam); then respawn
    fn bot_tick(&mut self, c: CtrlId) {
        if self.ctrls[c].team == -1 {
            let t = self.d.bot_default_team;
            self.request_assign_team(c, t);
        }
        if self.d.bot_auto_respawn && !self.ctrls[c].has_pawn {
            self.ctrls[c].wants_respawn = true;
        }
        let cc = &self.ctrls[c];
        if cc.wants_respawn && cc.next_respawn_time < self.now && self.controller_can_restart(c) && !self.ctrls[c].has_pawn
            && !self.spawns.is_queued(c)
        {
            self.ctrls[c].wants_respawn = false;
            self.restart_player(c);
        }
    }

    /// AMordhauPlayerController::CanAskForSpawn 0x15c8a90 -> AskForSpawn 0x15c4f00 -> server RestartPlayer (the spawn
    /// screen confirms at once: UNCONFIRMED)
    fn player_tick(&mut self, c: CtrlId) {
        if self.ctrls[c].has_pawn || self.spawns.is_queued(c) {
            return;
        }
        let cc = &self.ctrls[c];
        if cc.next_respawn_time < self.now
            && ((self.q())(cc.last_asked_for_spawn + 1.0) < self.now || cc.last_asked_for_spawn == 0.0)
            && self.in_progress()
            && self.controller_can_restart(c)
        {
            self.ctrls[c].last_asked_for_spawn = self.now;
            self.restart_player(c);
        }
    }

    /// AGameMode::Tick: WaitingToStart -> InProgress on AMordhauGameMode::ReadyToStartMatch_Implementation (0x15a42b0);
    /// StartMatch -> HandleMatchHasStarted 0x1599ee0
    fn match_state_step(&mut self) {
        if self.match_state != MatchState::WaitingToStart {
            return;
        }
        let we = self.d.state.warmup_end;
        if (we == -1.0 || we <= self.now) && !self.controllers.is_empty() {
            self.match_state = MatchState::InProgress;
            self.scoring_disabled = false;
            self.ev("match_started");
        }
    }

    /// AGameMode::EndMatch: InProgress -> WaitingPostMatch; AMordhauGameMode::HandleMatchHasEnded 0x1599e10: scoring off
    pub fn end_match(&mut self) {
        if self.match_state != MatchState::InProgress {
            return;
        }
        self.match_state = MatchState::WaitingPostMatch;
        self.scoring_disabled = true;
        self.ev("match_ended");
    }

    /// MatchTimeRanOut: native (bMatchTimeRanOut, EndMatch), extended by the FFA / TDM / FL Blueprints
    pub fn match_time_ran_out_(&mut self) {
        self.match_time_ran_out = true;
        self.end_match();
        match self.ext {
            ModeExt::Ffa => self.ffa_match_time_ran_out(),
            ModeExt::Tdm => self.tdm_match_time_ran_out(),
            ModeExt::Fl(_) => self.fl_match_time_ran_out(),
            _ => {}
        }
    }

    pub(crate) fn set_end_info(&mut self, winner: Option<CtrlId>, winner_team: i64, score: f64, draw: bool) {
        let info = MatchEndInfo {
            winner: winner.map(|w| self.name(w)).unwrap_or_default(),
            winner_team,
            winner_score: score,
            other_score: 0.0,
            draw,
        };
        self.ev_args(
            "match_end_info",
            vec![
                ("winner", info.winner.clone().into()),
                ("winner_team", winner_team.into()),
                ("winner_score", score.into()),
                ("other_score", 0.0.into()),
                ("draw", draw.into()),
            ],
        );
        self.match_end_info = Some(info);
        // OnRep_MatchEndInfo -> HandleMatchEndInfo: the Demon Invasion game state's authority half (config vars)
        if self.is_demon() {
            self.demon_on_match_end_info();
        }
    }

    // ---- teams ---------------------------------------------------------------------------------------------------
    /// AMordhauGameMode::RequestedAssignTeam_Implementation 0x15a6070 (parties not ported): Team -2 = auto: keep a team
    /// the player already has, else start at team int(rand01 * TeamCount) (clamped to TeamCount - 1) and take the
    /// first team with fewer players than the current pick (PlayerArray members with Team >= 0 counted). Then, when
    /// the team is valid (-2 < Team < TeamCount) and differs: the pawn dies (TakeDamage 1e8, host event), SetTeam,
    /// Controller->Reset.
    pub fn request_assign_team(&mut self, c: CtrlId, team: i64) {
        let tc = self.d.state.team_count;
        let mut team = team;
        if team == -2 {
            team = self.ctrls[c].team;
            if team == -1 {
                let mut counts = vec![0i64; tc.max(0) as usize];
                for &o in &self.controllers {
                    let t = self.ctrls[o].team;
                    if t >= 0 && t < tc {
                        counts[t as usize] += 1;
                    }
                }
                if tc == 0 {
                    team = 0;
                } else {
                    let r = (self.rng.rand() & 0x7fff) as f64;
                    let mut pick = ((r * K::TEAM_RAND_SCALE * tc as f64) as i64).min(tc - 1);
                    let mut best = counts[pick as usize];
                    for i in 0..tc {
                        if counts[i as usize] < best {
                            best = counts[i as usize];
                            pick = i;
                        }
                    }
                    team = pick;
                }
            }
        }
        if team > -2 && team < tc && self.ctrls[c].team != team {
            if self.ctrls[c].has_pawn {
                self.on_killed(Some(c), Some(c), 0, "", false); // the pawn takes 1e8 damage instigated by its own controller
                self.ctrls[c].has_pawn = false;
                let who = self.name(c);
                self.ev_args("kill_pawn", vec![("who", who.into())]);
            }
            self.set_team(c, team);
        }
    }

    /// AMordhauPlayerState::SetTeam
    pub fn set_team(&mut self, c: CtrlId, team: i64) {
        self.ctrls[c].team = team;
        let who = self.name(c);
        self.ev_args("team", vec![("who", who.into()), ("team", team.into())]);
    }

    /// AMordhauGameMode::AddTeamScore 0x1582410: 0 <= Team < TeamScores.Num -> SetTeamScore(Team, Amount + old)
    pub fn add_team_score(&mut self, team: i64, amount: f64) {
        if team >= 0 && (team as usize) < self.team_scores.len() {
            let v = (self.q())(self.team_scores[team as usize] + amount);
            self.set_team_score(team, v);
        }
    }

    /// AMordhauGameMode::SetTeamScore 0x15a7960 -> AMordhauGameMode::OnTeamScoreChanged (Blueprint event)
    pub fn set_team_score(&mut self, team: i64, v: f64) {
        let old = self.team_scores[team as usize];
        self.team_scores[team as usize] = v;
        self.ev_args("team_score", vec![("team", team.into()), ("score", v.into()), ("old", old.into())]);
        self.on_team_score_changed(team, old);
    }

    /// Blueprint OnTeamScoreChanged (TDM / FL override)
    pub fn on_team_score_changed(&mut self, team: i64, old: f64) {
        match self.ext {
            ModeExt::Tdm => self.tdm_on_team_score_changed(team, old),
            ModeExt::Fl(_) => self.fl_on_team_score_changed(team, old),
            _ => {}
        }
    }

    /// Blueprint OnScoreChanged (FFA overrides)
    pub fn on_score_changed(&mut self, c: CtrlId, old: f64) {
        if let ModeExt::Ffa = self.ext {
            self.ffa_on_score_changed(c, old);
        }
    }

    /// AMordhauGameState::GetPlayerCountsPerTeam 0x1598050 (server branch): per team, the controllers whose
    /// MordhauPlayerState has Team >= 0 (and bIsAlive when bOnlyLiving); bOnlyWithValidProfiles: header
    pub fn player_counts_per_team(&self, only_living: bool, _only_with_valid_profiles: bool) -> Vec<i64> {
        let mut out = vec![0i64; self.d.state.team_count.max(0) as usize];
        for &c in &self.controllers {
            let cc = &self.ctrls[c];
            if cc.team >= 0 && (cc.team as usize) < out.len() && (cc.alive || !only_living) {
                out[cc.team as usize] += 1;
            }
        }
        out
    }

    /// AMordhauGameState::IsFriendly_Implementation 0x159bc10 (player states): same state -> bIsFriendlyIfSelf; else
    /// team mode and the same team (neither 255)
    pub fn is_friendly(&self, a: Option<CtrlId>, b: Option<CtrlId>, friendly_if_self: bool) -> bool {
        let (Some(a), Some(b)) = (a, b) else { return false };
        if a == b {
            return friendly_if_self;
        }
        self.d.state.b_is_team_mode && self.ctrls[a].team == self.ctrls[b].team && self.ctrls[a].team != 255
    }

    /// UMordhauUtilityLibrary::MordhauPlayerStateSortPredicate 0x162e7d0: Score desc, Kills desc, Deaths asc, player
    /// name (case-sensitive code-unit compare) asc, PlayerId asc (UMordhauUtilityLibrary::SortPlayers 0x163f310)
    pub fn sort_players(&self, a: &[CtrlId]) -> Vec<CtrlId> {
        let mut s = a.to_vec();
        s.sort_by(|&x, &y| {
            let (x, y) = (&self.ctrls[x], &self.ctrls[y]);
            use std::cmp::Ordering::*;
            if x.score != y.score {
                return if x.score > y.score { Less } else { Greater };
            }
            if x.kills != y.kills {
                return y.kills.cmp(&x.kills);
            }
            if x.deaths != y.deaths {
                return x.deaths.cmp(&y.deaths);
            }
            if x.name != y.name {
                return x.name.cmp(&y.name);
            }
            x.player_id.cmp(&y.player_id)
        });
        s
    }

    // ---- scoring -------------------------------------------------------------------------------------------------
    /// AMordhauPlayerState::AddScore 0x15c3bf0 -> AMordhauGameMode::OnScoreChanged
    pub fn add_score(&mut self, c: CtrlId, amount: i64) {
        let old = self.ctrls[c].score;
        self.ctrls[c].score = (self.q())(old + amount as f64);
        let (who, score) = (self.name(c), self.ctrls[c].score);
        self.ev_args("score", vec![("who", who.into()), ("score", score.into()), ("old", old.into())]);
        self.on_score_changed(c, old);
    }

    /// AMordhauCharacter::TakeDamage 0x156e980 DamageHistory: alive pawn, an instigator, damage > 0: the instigator's
    /// entry gets damage added when still fresh (now <= time + 20) or replaced when stale, time = now; other stale
    /// entries are dropped; a new instigator is appended
    pub fn on_damage(&mut self, victim: CtrlId, instigator: CtrlId, damage: f64) {
        if damage <= 0.0 || !self.ctrls[victim].alive {
            return;
        }
        let now = self.now;
        let q = self.q();
        let mut found = false;
        let mut keep = Vec::new();
        for mut e in std::mem::take(&mut self.ctrls[victim].damage_history) {
            let fresh = now <= q(e.t + K::ASSIST_WINDOW);
            if e.who == instigator {
                e.damage = if fresh { q(e.damage + damage) } else { damage };
                e.t = now;
                found = true;
                keep.push(e);
            } else if fresh {
                keep.push(e);
            }
        }
        if !found {
            keep.push(DamageEntry { who: instigator, t: now, damage });
        }
        self.ctrls[victim].damage_history = keep;
    }

    /// OnKilled: AMordhauGameMode::OnKilled_Implementation 0x159d6f0 then BP_MordhauGameMode OnKilled ->
    /// AddKillNotify (on_killed_base). killer: None for a death without an instigating controller. damage_type:
    /// EMordhauDamageType byte; weapon: the DamageAgent's display name (EquipmentName), kick: the agent is a KickWeapon.
    /// Skirmish's Blueprint OnKilled runs first, then calls the parent; Frontline's calls the parent first.
    pub fn on_killed(&mut self, killer: Option<CtrlId>, killed: Option<CtrlId>, damage_type: i64, weapon: &str, kick: bool) {
        match self.ext {
            ModeExt::Skm(_) => {
                self.skm_on_killed(killer, killed);
                self.on_killed_base(killer, killed, damage_type, weapon, kick);
            }
            ModeExt::Fl(_) => {
                self.on_killed_base(killer, killed, damage_type, weapon, kick);
                self.fl_on_killed(killed);
            }
            ModeExt::Horde(_) => {
                self.horde_on_killed_pre(killer, killed);
                self.on_killed_base(killer, killed, damage_type, weapon, kick);
                self.horde_on_killed_post(killed);
            }
            ModeExt::Br(_) => {
                self.br_on_killed_pre(killed);
                self.on_killed_base(killer, killed, damage_type, weapon, kick);
            }
            _ => self.on_killed_base(killer, killed, damage_type, weapon, kick),
        }
    }

    fn on_killed_base(&mut self, killer: Option<CtrlId>, killed: Option<CtrlId>, damage_type: i64, weapon: &str, kick: bool) {
        let Some(killed) = killed else { return };
        let q = self.q();
        self.ctrls[killed].alive = false;
        let sc = self.d.scoring.clone();
        // NextRespawnTime (waves: ceil(now / t) * t)
        let mut nr = self.now;
        if sc.player_respawn_time > 0.0 {
            nr = if !sc.b_players_spawn_in_waves {
                q(self.now + sc.player_respawn_time)
            } else {
                q(q(self.now / sc.player_respawn_time).ceil() * sc.player_respawn_time)
            };
        }
        self.ctrls[killed].next_respawn_time = nr;
        if !self.scoring_disabled {
            self.assists(killer, killed);
            let k = killer.unwrap_or(killed);
            let friendly = self.is_friendly(Some(k), Some(killed), false);
            let mut counts_death = true;
            if killer.is_some() {
                if k == killed || friendly {
                    // suicide / team kill
                    if k == killed {
                        if sc.b_suicide_decrements_kills {
                            self.ctrls[k].kills -= 1;
                        }
                    } else if sc.b_team_kills_decrement_killer_kills {
                        self.ctrls[k].kills -= 1;
                    }
                    if self.d.state.b_is_team_mode {
                        let t = self.ctrls[k].team;
                        self.add_team_score(t, sc.team_kill_team_score_change);
                    }
                    self.add_score(k, sc.team_kill_score_change as i64);
                } else {
                    // kill
                    self.ctrls[k].kills += 1;
                    if self.d.state.b_is_team_mode {
                        let t = self.ctrls[k].team;
                        self.add_team_score(t, sc.kill_team_score_change);
                    }
                    self.add_score(k, sc.kill_score_change as i64);
                }
            }
            if friendly && !sc.b_team_kills_increment_killed_deaths {
                counts_death = false;
            }
            if counts_death {
                self.ctrls[killed].deaths += 1;
            }
        }
        // BP_MordhauGameMode OnKilled (ubergraph 791): killer with a player state -> AddKillNotify(killer, killed), else
        // AddKillNotify(killed, killed). AddKillNotify: DamageType 3 -> Flags 2, a KickWeapon agent -> Flags 1, else 0
        let kn = killer.unwrap_or(killed);
        let mut flags = 0;
        if damage_type == self.k.i("kf_flag_fall_damage_type") {
            flags = self.k.i("kf_flag_fall");
        } else if kick {
            flags = self.k.i("kf_flag_kick");
        }
        let (kn, kd) = (self.name(kn), self.name(killed));
        self.ev_args(
            "kill_notify",
            vec![("killer", kn.into()), ("killed", kd.into()), ("flags", flags.into()), ("weapon", weapon.into())],
        );
    }

    /// OnKilled's assist loop over the killed pawn's DamageHistory: an entry that is not the killer, not the killed
    /// controller, still fresh (now <= time + 20) and not friendly to the killed player: fraction = min(damage * 0.01,
    /// 1); points = RoundToInt(fraction * KillScoreChange) * AssistScoreFactor ((int)ROUND((f + f) * K + 0.5) >> 1,
    /// cvtss2si: round half to even); points > 0 in team mode -> AddScore(points), and damage <
    /// AssistDamageToCountAsKill -> +1 assist, else +1 kill
    fn assists(&mut self, killer: Option<CtrlId>, killed: CtrlId) {
        let sc = self.d.scoring.clone();
        let q = self.q();
        for e in self.ctrls[killed].damage_history.clone() {
            let a = e.who;
            if Some(a) == killer || a == killed || self.now > q(e.t + K::ASSIST_WINDOW) {
                continue;
            }
            if !self.has(a) || self.is_friendly(Some(killed), Some(a), false) {
                continue;
            }
            let frac = minf(q(e.damage * K::ASSIST_DAMAGE_SCALE), K::ASSIST_FRACTION_MAX);
            // (f + f) * K + 0.5 (exe: ss ops; the reference: 2.0 * frac * K + 0.5 in doubles)
            let pts = q((round_half_even(q(q(q(2.0 * frac) * sc.kill_score_change) + 0.5)) >> 1) as f64 * sc.assist_score_factor);
            if pts > 0.0 && self.d.state.b_is_team_mode {
                self.add_score(a, pts as i64);
                if e.damage < sc.assist_damage_to_count_as_kill as f64 {
                    self.ctrls[a].assists += 1;
                } else {
                    self.ctrls[a].kills += 1;
                }
                let who = self.name(a);
                self.ev_args("assist", vec![("who", who.into()), ("points", (pts as i64).into())]);
            }
        }
    }

    // ---- spawning ------------------------------------------------------------------------------------------------
    /// AMordhauGameMode::RestartPlayer 0x15a6d50 (no ControllerCanRestart check)
    pub fn restart_player(&mut self, c: CtrlId) {
        self.spawns.restart_player(c);
    }

    /// BP_MordhauGameMode UnpossessAndDestroyPawn: StopDriving if in a vehicle; Pawn.K2_DestroyActor; UnPossess;
    /// RestartPlayer(Controller) if asked
    pub fn unpossess_and_destroy_pawn(&mut self, c: CtrlId, restart: bool) {
        if self.ctrls[c].has_pawn {
            let who = self.name(c);
            self.ev_args("destroy_pawn", vec![("who", who.into())]);
        }
        self.ctrls[c].has_pawn = false;
        self.ctrls[c].alive = false;
        if restart {
            self.restart_player(c);
        }
    }

    fn spawn_queue_step(&mut self) {
        let list = self.controllers.clone();
        match self.spawns.tick(|c| list.contains(&c)) {
            SpawnStep::Spawn(c) => {
                let (start, origin, team) = self.spawn_pawn(c);
                let who = self.name(c);
                self.ev_args(
                    "spawn_pawn",
                    vec![("who", who.into()), ("start", start.into()), ("xf", Arg::V(origin)), ("team", team.into())],
                );
            }
            SpawnStep::Possess(c) => {
                self.ctrls[c].has_pawn = true;
                self.ctrls[c].alive = true; // bIsAlive with the possessed pawn (setter not ported - UNCONFIRMED)
                self.ctrls[c].damage_history.clear(); // a new pawn: its DamageHistory starts empty
                let who = self.name(c);
                self.ev_args("possess", vec![("who", who.into())]);
            }
            SpawnStep::Finalize(c) => {
                let who = self.name(c);
                self.ev_args("finalize_spawn", vec![("who", who.into())]);
            }
            SpawnStep::None => {}
        }
    }

    /// step 0 of the queue: SpawnDefaultPawnFor -> ChoosePlayerStart: (start name, transform origin, team)
    fn spawn_pawn(&mut self, c: CtrlId) -> (String, [f32; 3], i64) {
        match self.choose_player_start(c) {
            None => (String::new(), [0.0; 3], self.ctrls[c].team),
            Some(i) => {
                let o = self.starts[i].origin;
                self.ctrls[c].location = FVector::new(o[0], o[1], o[2]);
                (self.starts[i].name.clone(), o, self.ctrls[c].team)
            }
        }
    }

    /// AMordhauPlayerStart::IsAllowedSpawnFor_Implementation 0x15dee60: not disabled, and Team == the player's team or
    /// Team == -2 (any) with a team >= 0
    pub fn is_allowed_spawn_for(ps: &PlayerStartDef, c: &Ctrl) -> bool {
        if ps.b_is_spawn_disabled {
            return false;
        }
        c.team == ps.team || (ps.team == -2 && c.team >= 0)
    }

    /// AMordhauGameMode::GetSpawnpointPreference_Implementation 0x1599660
    pub fn spawn_preference(&mut self, ps: usize, c: CtrlId) -> f64 {
        let q = self.q();
        let mut pref = q((self.rng.rand() & 0x7fff) as f64 * K::SPAWN_RAND_SCALE); // GetSpawnPreferenceFor
        let o = self.starts[ps].origin;
        let origin = FVector::new(o[0], o[1], o[2]);
        let mut prox = 0.0;
        let mut total = 0.0;
        for &oc in &self.controllers {
            let oo = &self.ctrls[oc];
            if !(oo.alive && oo.has_pawn) {
                continue;
            }
            let dist_cm = q((oo.location - origin).length() as f64 * 100.0);
            let p = q(maxf(q(K::SPAWN_PROXIMITY_RANGE - dist_cm), 0.0) * K::SPAWN_PROXIMITY_SCALE);
            if p <= 0.0 {
                continue;
            }
            total = q(total + p);
            if self.d.state.b_is_team_mode && oo.team == self.ctrls[c].team {
                prox = q(prox + p);
            } else {
                prox = q(prox - p);
            }
        }
        pref = q(pref + q(clampf(prox, K::SPAWN_PROXIMITY_MIN, K::SPAWN_PROXIMITY_MAX) - q(total * K::SPAWN_CROWD_WEIGHT)));
        if self.encroached(origin, c) {
            pref = q(pref + K::SPAWN_BLOCKED_PENALTY);
        }
        pref
    }

    /// UWorld::EncroachingBlockingGeometry for the pawn at the start: another live pawn's capsule overlaps (header)
    fn encroached(&self, origin: FVector, c: CtrlId) -> bool {
        let r = CAPSULE_RADIUS * 2.0 * CM;
        let h = CAPSULE_HALF_HEIGHT * 2.0 * CM;
        for &oc in &self.controllers {
            let oo = &self.ctrls[oc];
            if oc == c || !(oo.alive && oo.has_pawn) {
                continue;
            }
            let dv = oo.location - origin;
            let l2 = (dv.x * dv.x + dv.z * dv.z).sqrt(); // Godot Vector2(dv.x, dv.z).length(), f32
            if (l2 as f64) < r && (dv.y.abs() as f64) < h {
                return true;
            }
        }
        false
    }

    /// ChoosePlayerStart_Implementation 0x15876e0: over the allowed starts, the first one, replaced by any later one
    /// with a higher preference (token distance equal for all: header)
    pub fn choose_player_start(&mut self, c: CtrlId) -> Option<usize> {
        let mut best: Option<usize> = None;
        let mut best_pref = 0.0;
        for i in 0..self.starts.len() {
            if !Self::is_allowed_spawn_for(&self.starts[i], &self.ctrls[c]) {
                continue;
            }
            let p = self.spawn_preference(i, c);
            if best.is_none() || p > best_pref {
                best = Some(i);
                best_pref = p;
            }
        }
        best
    }

    // ---- client-side queries (GameState) -------------------------------------------------------------------------
    /// ShouldBlockPawnInput: AMordhauGameState::ShouldBlockPawnInput_Implementation 0x15a8340 (header: no warmup in
    /// local play, so the base never blocks); BP_SkirmishGameState / BP_DuelGameState / BP_Group3v3GameState override
    pub fn should_block_input(&self, c: CtrlId) -> bool {
        match &self.ext {
            ModeExt::Skm(s) => s.last_observed_round_stage == self.k.i("skm_gs_block_stage"),
            ModeExt::Duel(_) => self.room_stage_is(c, "gs_block_input_stage"),
            ModeExt::Tf(_) => self.room_stage_is(c, "tf_gs_block_input_stage"),
            ModeExt::Br(_) => self.br_in_get_ready(),
            _ => false,
        }
    }

    /// BP_MordhauGameState GetScoreboardTime(InProgress) -> GetScoreboardTimeInProgress: MatchDurationMax > 0 ->
    /// max(MatchDurationMax - ElapsedTime, 0) s, else 200 days; WaitingToStart: WarmupEnd - now (0 here)
    pub fn scoreboard_seconds(&self) -> i64 {
        if self.in_progress() {
            return self.scoreboard_seconds_in_progress();
        }
        0
    }

    /// BP_MordhauGameState GetScoreboardTimeInProgress (BP_SkirmishGameState overrides)
    pub fn scoreboard_seconds_in_progress(&self) -> i64 {
        if let ModeExt::Skm(_) = self.ext {
            if let Some(v) = self.skm_scoreboard_seconds_in_progress() {
                return v;
            }
        }
        if self.d.state.match_duration_max > 0 {
            return (self.d.state.match_duration_max - self.elapsed_time).max(0);
        }
        self.k.i("sb_time_no_limit_days") * 86400
    }

    /// AMordhauGameState::GetTeamName_Implementation 0x1599c20: 0 <= Team < TeamNames.Num -> TeamNames[Team], else
    /// "%d" (Team -1's literal is not read: no caller here passes -1)
    pub fn team_name(&self, team: i64) -> String {
        if team >= 0 && (team as usize) < self.d.state.team_names.len() {
            return self.d.state.team_names[team as usize].clone();
        }
        team.to_string()
    }

    /// BP HandleMatchEndInfo (OnRep_MatchEndInfo) of the mode's game state, for the local player `me`: the HUD command
    /// (ShowMatchResult), or None (each mode overrides)
    pub fn match_result_for(&self, me: CtrlId, e: &MatchEndInfo) -> Option<MatchResult> {
        let k = &self.k;
        let team = self.ctrls[me].team;
        let r = |v: bool, t: &str, s: String| Some(MatchResult { victory: v, text: k.s(t), subtext: s });
        match &self.ext {
            ModeExt::Ffa => {
                if e.winner == self.ctrls[me].name || team == k.i("dm_spectator_team") {
                    r(true, "dm_victory", String::new())
                } else {
                    r(false, "dm_defeat", k.s("dm_winner_format").replace("{a}", &e.winner))
                }
            }
            ModeExt::Tdm => {
                if e.draw {
                    r(false, "tdm_draw", String::new())
                } else if team == e.winner_team || team == k.i("tdm_spectator_team") {
                    r(true, "tdm_victory", String::new())
                } else {
                    r(false, "tdm_defeat", String::new())
                }
            }
            ModeExt::Skm(_) => {
                if team == k.i("skm_spectator_team") || team == e.winner_team {
                    r(true, "skm_victory", String::new())
                } else {
                    r(false, "skm_defeat", String::new())
                }
            }
            ModeExt::Fl(_) => {
                if e.draw {
                    r(false, "fl_draw", String::new())
                } else if team == e.winner_team || team == k.i("fl_spectator_team") {
                    r(true, "fl_victory", String::new())
                } else {
                    r(false, "fl_defeat", String::new())
                }
            }
            // BP_DemonHordeGamestate HandleMatchEndInfo (@5382-@5535, @4839-@5157): WinnerTeam 0 -> (true, "victory",
            // "You defeated the Demon Horde!"); else (false, "defeat", no BP_DemonHordeCharacter alive (or none) ->
            // "You have all fallen to the Demon Horde", else "You failed to defeat the Demon Horde" (Temp_bool_12 is a
            // constant false: StageFailText is never shown))
            ModeExt::Horde(h) if h.demon.is_some() => {
                if e.winner_team == 0 {
                    Some(MatchResult { victory: true, text: "victory".into(), subtext: "You defeated the Demon Horde!".into() })
                } else {
                    let all_dead = !self.controllers.iter().any(|&c| !self.ctrls[c].is_bot && self.ctrls[c].has_pawn && self.ctrls[c].alive);
                    let sub = if all_dead { "You have all fallen to the Demon Horde" } else { "You failed to defeat the Demon Horde" };
                    Some(MatchResult { victory: false, text: "defeat".into(), subtext: sub.into() })
                }
            }
            // BP_HordeGameState HandleMatchEndInfo (ExecuteUbergraph_BP_HordeGameState@1541-@2068,
            // RULE_HRD_HordeGameState_HandleMatchEndInfo): WinnerTeam 0 -> ShowMatchResult(true, "victory", "");
            // else (false, "defeat", Format("Reached Wave {a}", Wave + 1)) (FText literals decoded r3)
            ModeExt::Horde(h) => {
                if e.winner_team == 0 {
                    Some(MatchResult { victory: true, text: "victory".into(), subtext: String::new() })
                } else {
                    let w = (h.match_info.wave as i64 + 1).to_string();
                    Some(MatchResult { victory: false, text: "defeat".into(), subtext: format!("Reached Wave {w}") })
                }
            }
            // BP_BattleRoyaleGameState HandleMatchEndInfo -> ubergraph @15-@602 (RULE_BR_BattleRoyaleGameState_
            // HandleMatchEndInfo): only the local player whose player state is MatchEndInfo.Winner gets
            // ShowMatchResult(true, "victory", "") and ShowBREndScreenDelayed; everyone else nothing (their end screen
            // came with their death)
            ModeExt::Br(_) => {
                if !e.winner.is_empty() && e.winner == self.ctrls[me].name {
                    Some(MatchResult { victory: true, text: "victory".into(), subtext: String::new() })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// the game state's Blueprint ReceiveTick on the local player's machine: HUD commands (Skirmish overrides)
    pub fn client_tick(&self, me: CtrlId) -> Vec<Ev> {
        if let ModeExt::Skm(_) = self.ext {
            return self.skm_client_tick(me);
        }
        Vec::new()
    }
}

/// the reference's _round_half_even (x86 cvtss2si under round-to-nearest-even)
pub fn round_half_even(x: f64) -> i64 {
    mordhau_core::ue::cvtss2si(x)
}

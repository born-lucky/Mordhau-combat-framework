//! A capture point (godot/game/mode/control_point.gd): native AControlPoint (extract/native/decomp/AControlPoint.cpp)
//! with the Blueprint subclasses' overrides the modes use (BP_CapturePoint / BP_TeamBaseCapturePoint for Frontline,
//! BP_SkirmishCapturePoint for Skirmish). Engine-neutral: the host reports the pawns that begin / end overlapping the
//! capture area (GameMode::cp_begin_overlap / cp_end_overlap); the game mode ticks every point once per frame and reads
//! owning_team / capture_progress. Constants: ControlPointDef (native ctor + Blueprint CDO chain + the placed actor).
//!
//! Native sources (Mordhau-Win64-Shipping.exe build 702625635, PDB names; offsets from extract/native/types/*.h):
//!   ctor          AControlPoint::AControlPoint rva=0x14e30e0 (OwningTeam / CapturingTeam 255, SpawnsTeam -1,
//!                 UnchangedCaptureProgressTime 999, LastSetUIProgress -1, NetUpdateFrequency 10)
//!   BeginPlay     AControlPoint::BeginPlay rva=0x14f0030 (server: a point placed with an owner starts captured)
//!   Tick          AControlPoint::Tick rva=0x1517270 (prerequisites, server progress / client smoothing, flashing)
//!   progress      AControlPoint::UpdateCaptureProgress rva=0x151b3f0, AControlPoint::SetCaptureProgress rva=0x1515740
//!   presence      AControlPoint::UpdatePresenceNumbers rva=0x151e920 (live characters, character Team byte 0 / 1),
//!                 AControlPoint::OnCaptureAreaBeginOverlap rva=0x1507fa0 / OnCaptureAreaEndOverlap rva=0x1508000
//!                 (AMordhauCharacter CurrentCapturePoint +0xd80 / CurrentCapturePointTime +0xd88)
//!   spawns        AControlPoint::UpdateSpawns rva=0x151fb90 (AMordhauPlayerStart bIsSpawnDisabled +0x250, Team +0x254)
//!   CanCapture    AControlPoint::CanCapture rva=0x14f2390
//!   setters       AControlPoint::SetCapturingTeam rva=0x1515b30, AControlPoint::SetOwningTeam rva=0x1515c30,
//!                 OnRep_CapturingTeam rva=0x150e4e0, OnRep_OwningTeam rva=0x150e540 (fire the *Changed event)
//!   client        AControlPoint::OnRep_ReplicatedCaptureProgress rva=0x150ecf0; Tick's non-authority branch smooths
//!                 CaptureProgress toward ReplicatedCaptureProgress / 255 over NetworkSmoothTime
//! Blueprint (scripts/kismet bytecode; literals: Kismet "skm_cp_*"):
//!   BP_SkirmishCapturePoint RoundStarted: SetCaptureProgress(0, 255, false) [5603]; Locked = true, OnRep_Locked
//!     (bIsCapturable = !Locked) [276]; RetriggerableDelay(TimeToUnlock) [183] -> Locked = false, OnRep_Locked [104]
//!   BP_SkirmishCapturePoint OnCapturingTeamChanged (local player): GetTimeSeconds > 3 and ShowAnnouncements and
//!     CapturingTeam != 255: GetTeamRelevance (local team 0 / 1: Team != local team; else 2) 0 -> "We are capturing the
//!     point!", 1 -> "Enemy is capturing the point!" (4 s) [307..1135].
//! Not ported (UNCONFIRMED / out of scope, as in the reference): the capture area's geometry (the host decides
//! overlaps); visuals / banners / widgets (events only); BP_CapturePoint objectives; push mode; actor tick order relative
//! to the game mode and the game state (the mode ticks points before its own Blueprint tick).

use crate::data::{ControlPointDef, PlayerStartDef};
use crate::event::Ev;
use crate::game_mode::{Ctrl, CtrlId};
use crate::kismet::{vb, vf, Kismet};
use mordhau_core::ue::clampf;

pub const NO_TEAM: i64 = 255;
/// AControlPoint ctor rva=0x14e30e0 stores UnchangedCaptureProgressTime 999 (0x4479c000) and NetUpdateFrequency 10;
/// Tick rva=0x1517270 drops NetUpdateFrequency to 1 after 2 s unchanged (0x3f800000 / 0x40000000 immediates)
pub const UNCHANGED_AT_CTOR: f64 = 999.0;
pub const NET_UPDATE_FREQUENCY_ACTIVE: f64 = 10.0;
pub const NET_UPDATE_FREQUENCY_IDLE: f64 = 1.0;
pub const NET_IDLE_AFTER_S: f64 = 2.0;
/// Tick rva=0x1517270 flashing threshold: bIsFlashing follows UnchangedCaptureProgressTime >= 0.5
pub const FLASH_AFTER_S: f64 = 0.5;
/// SetCaptureProgress rva=0x1515740: ReplicatedCaptureProgress = (byte)(int)(CaptureProgress * 255);
/// Tick / OnRep_ReplicatedCaptureProgress rva=0x150ecf0: CaptureProgress = Replicated * 0.003921569 (1/255)
pub const REP_SCALE: f64 = 255.0;
pub const REP_INV: f64 = 0.003921569;
/// Tick rva=0x1517270: client smoothing snaps when within 0.0001; unchanged when |delta| <= 1e-08
pub const SNAP_EPS: f64 = 0.0001;
pub const UNCHANGED_EPS: f64 = 1e-08;
/// SetCaptureProgress rva=0x1515740: AwardScoreInterval is used only when |interval| > 1e-08
pub const INTERVAL_EPS: f64 = 1e-08;
/// ClientReceiveScoreNoState reasons SetCaptureProgress rva=0x1515740 passes: 8 capturing, 9 captured,
/// 10 neutralizing, 11 neutralized (EScoreFeedReason values; names UNCONFIRMED)
pub const REASON_CAPTURING: i64 = 8;
pub const REASON_CAPTURED: i64 = 9;
pub const REASON_NEUTRALIZING: i64 = 10;
pub const REASON_NEUTRALIZED: i64 = 11;

/// what a point's tick may touch outside itself: the controllers (CurrentCapturePointTime, presence), the map's
/// player starts (UpdateSpawns) and the score awards (AMordhauPlayerState::AddScore, applied by the mode after)
pub struct CpCtx<'a> {
    pub ctrls: &'a mut [Ctrl],
    pub starts: &'a mut [PlayerStartDef],
    pub awards: &'a mut Vec<(CtrlId, i64)>,
    /// the mode's float rounding (GameMode::q: binary32 in exe mode)
    pub q: fn(f64) -> f64,
}

#[derive(Clone, Debug)]
pub struct ControlPoint {
    pub d: ControlPointDef,
    pub name: String,
    /// Role == ROLE_Authority (3)
    pub authority: bool,
    /// a dedicated server skips the client visuals (flashing)
    pub dedicated: bool,
    pub capture_progress: f64,
    pub replicated_capture_progress: i64,
    pub owning_team: i64,
    pub capturing_team: i64,
    pub unchanged_capture_progress_time: f64,
    pub net_update_frequency: f64,
    pub b_is_capturable: bool,
    /// +0x24c (not stored by the ctor: false until the first Tick)
    pub b_team1_owns_prerequisites: bool,
    pub b_team2_owns_prerequisites: bool,
    pub b_spawns_disabled: bool,
    pub spawns_team: i64,
    pub team1_presence: i64,
    pub team2_presence: i64,
    pub b_is_flashing: bool,
    pub b_has_ever_replicated_progress: bool,
    /// Team1PrerequisitePoints / Team2PrerequisitePoints: indices into the mode's control points (None = a name the
    /// map does not place, which the reference keeps as null and skips)
    pub team1_prerequisites: Vec<Option<usize>>,
    pub team2_prerequisites: Vec<Option<usize>>,
    /// SpawnPoints: indices into the mode's player starts
    pub spawn_points: Vec<Option<usize>>,
    /// characters overlapping the capture area
    pub overlapping: Vec<CtrlId>,
    /// OverlapsCache (UpdatePresenceNumbers)
    pub overlaps_cache: Vec<CtrlId>,
    pub locked: bool,
    unlock_at: f64,
    pub events: Vec<Ev>,
}

impl ControlPoint {
    pub fn new(def: ControlPointDef) -> ControlPoint {
        ControlPoint {
            name: def.name.clone(),
            authority: true,
            dedicated: false,
            capture_progress: 0.0,
            replicated_capture_progress: 0,
            owning_team: def.owning_team,
            capturing_team: def.capturing_team,
            unchanged_capture_progress_time: UNCHANGED_AT_CTOR,
            net_update_frequency: NET_UPDATE_FREQUENCY_ACTIVE,
            b_is_capturable: def.b_is_capturable,
            b_team1_owns_prerequisites: false,
            b_team2_owns_prerequisites: false,
            b_spawns_disabled: false,
            spawns_team: -1,
            team1_presence: 0,
            team2_presence: 0,
            b_is_flashing: false,
            b_has_ever_replicated_progress: false,
            team1_prerequisites: Vec::new(),
            team2_prerequisites: Vec::new(),
            spawn_points: Vec::new(),
            overlapping: Vec::new(),
            overlaps_cache: Vec::new(),
            locked: def.locked,
            unlock_at: -1.0,
            events: Vec::new(),
            d: def,
        }
    }

    fn ev(&mut self, e: Ev) {
        let mut e = e;
        e.args.insert(0, ("point", self.name.clone().into()));
        self.events.push(e);
    }

    pub fn drain(&mut self) -> Vec<Ev> {
        std::mem::take(&mut self.events)
    }

    /// AControlPoint::BeginPlay rva=0x14f0030 (server): OwningTeam != 255 -> CapturingTeam = OwningTeam (event),
    /// SetCaptureProgress(1, OwningTeam, false); then UpdateSpawns
    pub fn begin_play(&mut self, cx: &mut CpCtx) {
        if self.authority {
            if self.owning_team != NO_TEAM {
                let t = self.owning_team;
                self.set_capturing_team(t);
                self.set_capture_progress(1.0, t, false, 0.0, cx);
            }
            self.update_spawns(cx.starts);
        }
    }

    /// AControlPoint::CanCapture rva=0x14f2390: team 0 -> bTeam1OwnsPrerequisites, team 1 -> bTeam2..., else false
    pub fn can_capture(&self, team: i64) -> bool {
        match team {
            0 => self.b_team1_owns_prerequisites,
            1 => self.b_team2_owns_prerequisites,
            _ => false,
        }
    }

    /// AControlPoint::SetCapturingTeam rva=0x1515b30: on change, OnCapturingTeamChanged (BlueprintNativeEvent; native
    /// _Implementation rva=0x1508070 updates client visuals) -> event
    pub fn set_capturing_team(&mut self, t: i64) {
        if self.capturing_team != t {
            self.capturing_team = t;
            self.ev(Ev::new("capturing_team").with("team", t));
        }
    }

    /// AControlPoint::SetOwningTeam rva=0x1515c30
    pub fn set_owning_team(&mut self, t: i64) {
        if self.owning_team != t {
            self.owning_team = t;
            self.ev(Ev::new("owning_team").with("team", t));
        }
    }

    /// OnCaptureAreaBeginOverlap rva=0x1507fa0: a character -> UpdatePresenceNumbers; its CurrentCapturePoint = this
    /// and CurrentCapturePointTime = 0. `me`: this point's index in the mode
    pub fn begin_overlap(&mut self, me: usize, c: CtrlId, ctrls: &mut [Ctrl]) {
        if !self.overlapping.contains(&c) {
            self.overlapping.push(c);
        }
        self.update_presence_numbers(ctrls);
        ctrls[c].capture_point = Some(me);
        ctrls[c].capture_point_time = 0.0;
    }

    /// OnCaptureAreaEndOverlap rva=0x1508000: UpdatePresenceNumbers; if its CurrentCapturePoint is this, both cleared
    pub fn end_overlap(&mut self, me: usize, c: CtrlId, ctrls: &mut [Ctrl]) {
        self.overlapping.retain(|&o| o != c);
        self.update_presence_numbers(ctrls);
        if ctrls[c].capture_point == Some(me) {
            ctrls[c].capture_point = None;
            ctrls[c].capture_point_time = 0.0;
        }
    }

    /// AControlPoint::UpdatePresenceNumbers rva=0x151e920: OverlapsCache = the overlapping MordhauCharacters; each one
    /// not bIsDead (+0x504) counts for Team1Presence (character Team +0x660 == 0) or Team2Presence (== 1)
    pub fn update_presence_numbers(&mut self, ctrls: &[Ctrl]) {
        self.team1_presence = 0;
        self.team2_presence = 0;
        self.overlaps_cache = self.overlapping.clone();
        for &c in &self.overlaps_cache {
            let c = &ctrls[c];
            if !c.alive {
                continue;
            }
            if c.team == 0 {
                self.team1_presence += 1;
            } else if c.team == 1 {
                self.team2_presence += 1;
            }
        }
    }

    /// AControlPoint::Tick rva=0x1517270. match_in_progress: the authority game mode's AGameMode::IsMatchInProgress
    /// (vtable +0x820 call at 0x14151740f). t1_owners / t2_owners: the OwningTeam of each existing prerequisite point
    /// (read by the mode just before this point ticks, so an earlier point's change this frame is seen)
    pub fn tick(&mut self, dt: f64, now: f64, match_in_progress: bool, t1_owners: &[i64], t2_owners: &[i64], cx: &mut CpCtx) {
        self.bp_tick(now);
        // prerequisites: every Team1PrerequisitePoint owned by team 0 (every Team2... by team 1); empty = true
        let t1 = t1_owners.iter().all(|&o| o == 0);
        let t2 = t2_owners.iter().all(|&o| o == 1);
        if t1 != self.b_team1_owns_prerequisites {
            self.b_team1_owns_prerequisites = t1;
            if self.owning_team == 1 {
                self.ev(Ev::new(if t1 { "enemy_gained_prerequisites" } else { "enemy_lost_prerequisites" }));
            }
        }
        if t2 != self.b_team2_owns_prerequisites {
            self.b_team2_owns_prerequisites = t2;
            if self.owning_team == 0 {
                self.ev(Ev::new(if t2 { "enemy_gained_prerequisites" } else { "enemy_lost_prerequisites" }));
            }
        }
        let old = self.capture_progress;
        let q = cx.q;
        if self.authority {
            if match_in_progress {
                self.update_capture_progress(dt, cx);
            }
        } else {
            let target = q(self.replicated_capture_progress as f64 * REP_INV);
            self.capture_progress = q(q(q(dt / self.d.network_smooth_time) * q(target - old)) + old);
            if (self.capture_progress - target).abs() <= SNAP_EPS {
                self.capture_progress = target;
            }
        }
        if (self.capture_progress - old).abs() <= UNCHANGED_EPS {
            self.unchanged_capture_progress_time = q(self.unchanged_capture_progress_time + dt);
            if self.authority && self.unchanged_capture_progress_time > NET_IDLE_AFTER_S {
                self.net_update_frequency = NET_UPDATE_FREQUENCY_IDLE;
            }
        } else {
            self.unchanged_capture_progress_time = 0.0;
            if self.authority {
                self.net_update_frequency = NET_UPDATE_FREQUENCY_ACTIVE;
            }
            let p = self.capture_progress;
            self.ev(Ev::new("update_visuals").with("progress", p));
        }
        if !self.dedicated {
            let flash = self.unchanged_capture_progress_time >= FLASH_AFTER_S;
            if self.b_is_flashing == flash {
                self.b_is_flashing = !flash;
                self.ev(Ev::new(if self.b_is_flashing { "started_flashing" } else { "stopped_flashing" }));
            }
        }
    }

    /// AControlPoint::UpdateCaptureProgress rva=0x151b3f0
    pub fn update_capture_progress(&mut self, dt: f64, cx: &mut CpCtx) {
        if !self.b_is_capturable {
            return;
        }
        let p1 = self.team1_presence;
        let p2 = self.team2_presence;
        let t1 = p1 >= 1 && self.b_team1_owns_prerequisites;
        let t2 = p2 >= 1 && self.b_team2_owns_prerequisites;
        if t1 && t2 {
            if p1 == p2 {
                return;
            }
            if self.d.b_should_pause_capture_if_enemy_near {
                return;
            }
        }
        let team1_wins = t1 && p1 > p2;
        let team2_wins = t2 && p1 < p2;
        let nobody = !(team1_wins || team2_wins);
        let mut lead = 0.0;
        if team1_wins {
            lead = (p1 - p2) as f64;
        } else if team2_wins {
            lead = (p2 - p1) as f64;
        }
        let q = cx.q;
        let cap_speed = q(ControlPointDef::curve(&self.d.capture_speed, lead));
        let neu_speed = q(ControlPointDef::curve(&self.d.neutralize_speed, lead));
        if nobody && self.owning_team == NO_TEAM {
            let p = q(self.capture_progress - q(dt * self.d.uncapture_speed));
            self.set_capture_progress(p, NO_TEAM, false, dt, cx);
            return;
        }
        // the owner is challenged: Owning 0 -> team 2 leads; Owning 1 -> team 1 leads; unowned -> always
        let challenged = match self.owning_team {
            0 => team2_wins,
            1 => team1_wins,
            _ => true,
        };
        if !challenged {
            let p = q(q(cap_speed * dt) + self.capture_progress);
            let o = self.owning_team;
            self.set_capture_progress(p, o, true, dt, cx);
            return;
        }
        let cap = self.capturing_team;
        if cap != 0 && team1_wins {
            let p = q(self.capture_progress - q(neu_speed * dt));
            self.set_capture_progress(p, 0, true, dt, cx);
        } else if cap != 0 && cap != 1 && team2_wins {
            let p = q(self.capture_progress - q(neu_speed * dt));
            self.set_capture_progress(p, 1, true, dt, cx);
        } else if cap == 0 && team2_wins {
            let p = q(self.capture_progress - q(neu_speed * dt));
            self.set_capture_progress(p, 1, true, dt, cx);
        } else {
            let p = q(q(cap_speed * dt) + self.capture_progress);
            self.set_capture_progress(p, cap, true, dt, cx);
        }
    }

    /// AControlPoint::SetCaptureProgress rva=0x1515740 (authority only). dt: World DeltaTimeSeconds (score intervals)
    pub fn set_capture_progress(&mut self, p: f64, new_captor: i64, award: bool, dt: f64, cx: &mut CpCtx) {
        if !self.authority {
            return;
        }
        let mut award = award;
        let old = self.capture_progress;
        let p = clampf(p, 0.0, 1.0);
        let mut score = self.d.award_score_capturing;
        if p < old {
            score = self.d.award_score_neutralizing;
        }
        self.capture_progress = p;
        let mut reason = REASON_NEUTRALIZING;
        if old <= p {
            reason = REASON_CAPTURING;
        }
        if p != 0.0 {
            if p == 1.0 {
                self.set_capturing_team(new_captor);
                if old == self.capture_progress && self.owning_team == self.capturing_team {
                    return;
                }
                score = self.d.award_score_captured;
                reason = REASON_CAPTURED;
                self.capture_progress = 1.0;
                let ct = self.capturing_team;
                self.set_owning_team(ct);
            } else if old == p {
                return;
            }
        } else {
            if old == p && self.owning_team == NO_TEAM {
                if self.capturing_team == new_captor {
                    return;
                }
                score = self.d.award_score_neutralized;
                award = false;
            } else {
                score = self.d.award_score_neutralized;
            }
            self.set_capturing_team(new_captor);
            reason = REASON_NEUTRALIZED;
            self.set_owning_team(NO_TEAM);
        }
        self.unchanged_capture_progress_time = 0.0;
        if award {
            self.award(new_captor, score, reason, dt, cx);
        }
        self.update_spawns(cx.starts);
        self.replicated_capture_progress = ((self.capture_progress * REP_SCALE) as i64) & 0xff;
    }

    /// SetCaptureProgress's score loop over OverlapsCache: a live character with a MordhauPlayerState on the captor's
    /// team accumulates CurrentCapturePointTime += DeltaTime; captured / neutralized (reasons 9, 11) award at once and
    /// reset the time, capturing / neutralizing award each time floor(time / AwardScoreInterval) steps up; AddScore +
    /// ClientReceiveScoreNoState(reason, score) when score != 0. A character of another team has its time reset.
    fn award(&mut self, captor: i64, score: i64, reason: i64, dt: f64, cx: &mut CpCtx) {
        let cache = self.overlaps_cache.clone();
        for c in cache {
            if !cx.ctrls[c].alive {
                continue;
            }
            if cx.ctrls[c].team != captor {
                cx.ctrls[c].capture_point_time = 0.0;
                continue;
            }
            let iv = self.d.award_score_interval;
            if iv.abs() <= INTERVAL_EPS {
                continue;
            }
            let before = cx.ctrls[c].capture_point_time;
            cx.ctrls[c].capture_point_time = (cx.q)(dt + before);
            let give;
            if reason == REASON_CAPTURED || reason == REASON_NEUTRALIZED {
                cx.ctrls[c].capture_point_time = 0.0;
                give = score != 0;
            } else {
                give = ((cx.q)(before / iv) as i64) < ((cx.q)(cx.ctrls[c].capture_point_time / iv) as i64) && score != 0;
            }
            if give {
                cx.awards.push((c, score));
                let who = cx.ctrls[c].name.clone();
                self.ev(Ev::new("score").with("who", who).with("reason", reason).with("score", score));
            }
        }
    }

    /// AControlPoint::UpdateSpawns rva=0x151fb90: spawns enabled for OwningTeam when owned and (fully captured or not
    /// bPreventSpawningIfContested); on a change, each SpawnPoint: bIsSpawnDisabled = bSpawnsDisabled, or true when the
    /// start's Team differs from SpawnsTeam
    pub fn update_spawns(&mut self, starts: &mut [PlayerStartDef]) {
        let mut disable = true;
        if (self.capture_progress == 1.0 || !self.d.b_prevent_spawning_if_contested) && self.owning_team != NO_TEAM {
            disable = false;
        }
        let team = self.owning_team;
        let team_changed = self.owning_team != NO_TEAM && self.spawns_team != team;
        if disable != self.b_spawns_disabled || team_changed {
            self.b_spawns_disabled = disable;
            self.spawns_team = team;
            for s in self.spawn_points.iter().flatten() {
                let s = &mut starts[*s];
                s.b_is_spawn_disabled = self.b_spawns_disabled;
                if s.team != self.spawns_team {
                    s.b_is_spawn_disabled = true;
                }
            }
        }
    }

    /// AControlPoint::OnRep_ReplicatedCaptureProgress rva=0x150ecf0 (client): the first replication snaps the progress
    pub fn on_rep_replicated_capture_progress(&mut self) {
        if !self.b_has_ever_replicated_progress {
            self.unchanged_capture_progress_time = UNCHANGED_AT_CTOR;
            self.b_has_ever_replicated_progress = true;
            self.capture_progress = self.replicated_capture_progress as f64 * REP_INV;
        }
    }

    // ---- BP_SkirmishCapturePoint ----------------------------------------------------------------------------------
    pub fn is_skirmish_point(&self) -> bool {
        self.d.time_to_unlock >= 0.0
    }

    /// RoundStarted: SetCaptureProgress(0, 255, false); Locked = true (OnRep_Locked: bIsCapturable = !Locked);
    /// RetriggerableDelay(TimeToUnlock) then Locked = false (header)
    pub fn round_started(&mut self, now: f64, k: &Kismet, cx: &mut CpCtx) {
        let r = k.arr("skm_cp_round_reset");
        let (p, team, award) = (vf(&r[0]), vf(&r[1]) as i64, vb(&r[2]));
        self.set_capture_progress(p, team, award, 0.0, cx);
        self.set_locked(true);
        self.unlock_at = now + self.d.time_to_unlock;
    }

    fn set_locked(&mut self, v: bool) {
        self.locked = v;
        self.b_is_capturable = !self.locked;
        self.ev(Ev::new("locked").with("locked", v));
    }

    fn bp_tick(&mut self, now: f64) {
        if self.unlock_at >= 0.0 && now >= self.unlock_at {
            self.unlock_at = -1.0;
            self.set_locked(false);
        }
    }

    /// BP_SkirmishCapturePoint OnCapturingTeamChanged on the local machine (header): the announcement (text, duration)
    /// for local team `local_team` at time `now` for a "capturing_team" event, or None (relevance 2 = spectator / no
    /// team, or too early, or no captor)
    pub fn local_announcement(&self, e: &Ev, local_team: i64, now: f64, k: &Kismet) -> Option<(String, f64)> {
        if e.kind != "capturing_team" || !self.is_skirmish_point() {
            return None;
        }
        let team = e.get("team").map(|a| a.as_i()).unwrap_or(NO_TEAM);
        if !(now > k.f("skm_cp_announce_after_s")) || team == NO_TEAM {
            return None;
        }
        let mut rel = k.i("skm_cp_no_relevance");
        if local_team == 0 || local_team == 1 {
            rel = if team != local_team { 1 } else { 0 };
        }
        match rel {
            0 => Some((k.s("skm_cp_we_capturing"), k.f("skm_cp_announce_s"))),
            1 => Some((k.s("skm_cp_enemy_capturing"), k.f("skm_cp_announce_s"))),
            _ => None,
        }
    }
}

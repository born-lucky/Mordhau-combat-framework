//! BP_CapturePoint (Mordhau/Content/Mordhau/Blueprints/GameModes/Battle/BP_CapturePoint; native AControlPoint, which
//! mh-mode's `ControlPoint` owns) as its objectives see it: the Blueprint layer that turns objective progress into
//! capture progress, the objective win, and the prerequisite hand-off to the objectives. Bytecode:
//! state/world_kismet/BP_CapturePoint.txt (+ BP_HiddenCapturePoint.txt: same graph, CaptureArea = null in its
//! construction script, bIsHiddenPoint on the CDO).
//!
//! The capture state itself (OwningTeam, CaptureProgress, prerequisites) lives in mh-mode; the host forwards
//! mh-mode's owner changes (`World::set_capture_point_owner`) and its enemy_gained / enemy_lost_prerequisites events
//! (`World::capture_point_prerequisites`), and applies the `WorldEvent::CaptureProgress` / `TeamScore` /
//! `PushWinDelay` this module emits to mh-mode (ControlPoint::set_capture_progress / GameMode::set_team_score).

use crate::props::Props;
use crate::world::{ActorId, WorldEvent};

#[derive(Clone, Debug)]
pub struct CapturePoint {
    pub actor: ActorId,
    /// `Objectives` (TArray<TScriptInterface<IObjectiveInterface>>), resolved; None = a reference that does not
    /// resolve to a placed actor (IsValid false: counted as progress 1.0, ObjectivesChanged@1545)
    pub objectives: Vec<Option<ActorId>>,
    /// `DeliverySpots` (BP_ItemDeliverySpot:AnyObjectiveProgressChanged@226 reads it)
    pub delivery_spots: Vec<Option<ActorId>>,
    /// AControlPoint OwningTeam as mh-mode last reported it (EMordhauTeam byte; None = 255)
    pub owning_team: Option<u8>,
    pub objective_progress: f64,
    pub previous_progress: f64,
    pub objectives_completed: bool,
    pub hidden: bool,
    pub push: bool,
    pub win_delay: f64,
    pub complete_delay: f64,
    /// TriggerWinDelayed's Delay(ObjectiveWinDelay) (@9736)
    pub win_at: Option<f64>,
    /// CompleteCaptureDelayed's Delay(ObjectiveCompleteDelay) with PendingNewCaptor (@9800..@9880)
    pub complete_at: Option<(f64, Option<u8>)>,
}

/// K2Node_Select on OwningTeam: 0 -> 1, 1 -> 0, other -> the select's default pin (an unlinked byte: 0 [UNCONFIRMED:
/// the default pin's literal is not in the bytecode dump]; reported as None)
fn other_team(t: Option<u8>) -> Option<u8> {
    match t {
        Some(0) => Some(1),
        Some(1) => Some(0),
        _ => None,
    }
}

impl CapturePoint {
    pub fn new(actor: ActorId, p: &Props, objectives: Vec<Option<ActorId>>, delivery_spots: Vec<Option<ActorId>>) -> CapturePoint {
        let t = p.get("OwningTeam").map(|_| p.i("OwningTeam"));
        CapturePoint {
            actor,
            objectives,
            delivery_spots,
            owning_team: t.filter(|t| (0..255).contains(t)).map(|t| t as u8),
            objective_progress: p.f("ObjectiveProgress"),
            previous_progress: 0.0,
            objectives_completed: p.b("bObjectivesCompleted"),
            hidden: p.b("bIsHiddenPoint"),
            push: p.b("IsPushPoint"),
            win_delay: p.f("ObjectiveWinDelay"),
            complete_delay: p.f("ObjectiveCompleteDelay"),
            win_at: None,
            complete_at: None,
        }
    }

    /// ObjectivesChanged@0..@743: ObjectiveProgress = clamp(sum(GetObjectiveProgress) / Objectives.Num(), 0, 1),
    /// an invalid objective adding 1.0. `progress(o)` is the objective's GetObjectiveProgress
    pub fn compute_progress(&mut self, progress: &dyn Fn(ActorId) -> f64) {
        self.previous_progress = self.objective_progress;
        let mut s = 0.0f64;
        for o in &self.objectives {
            s += match o {
                Some(a) => progress(*a),
                None => 1.0,
            };
        }
        let n = self.objectives.len() as f64;
        // UKismetMathLibrary::Divide_FloatFloat: a 0 divisor returns 0 (Objectives empty never calls this, @2743)
        let p = if n > 0.0 { s / n } else { 0.0 };
        self.objective_progress = p.clamp(0.0, 1.0);
    }

    /// ObjectivesChanged@1615..@2786, the authority part after the objectives were told (OnAnyObjectiveProgressChanged
    /// @770..@1198): nothing once bObjectivesCompleted; a hidden point hands its progress to the enemy (@1660); a push
    /// point completes at 1 for the enemy (CompleteCaptureDelayed) and otherwise sets the enemy's 1 - progress (@1876); a
    /// normal point keeps OwningTeam at 1 - progress and at 1 sets 0.005 and starts TriggerWinDelayed (@2470); then
    /// progress 1 marks bObjectivesCompleted (@2650)
    pub fn authority_changed(&mut self, now: f64, ev: &mut Vec<WorldEvent>) {
        if self.objectives_completed {
            return; // @1645
        }
        let p = self.objective_progress;
        let cp = self.actor;
        if self.hidden {
            if p != 0.0 {
                ev.push(WorldEvent::CaptureProgress { capture_point: cp, progress: p, team: other_team(self.owning_team) }); // @1790
            }
        } else if self.push {
            if p != 0.0 {
                if p == 1.0 {
                    self.complete_capture_delayed(other_team(self.owning_team), now, ev); // @2054
                } else {
                    // @2233: the select maps OwningTeam 0 -> 1, 1 -> 0 (Temp_byte_Variable 1 / _1 0, @2124..@2206)
                    ev.push(WorldEvent::CaptureProgress { capture_point: cp, progress: 1.0 - p, team: other_team(self.owning_team) });
                    ev.push(WorldEvent::PushWinDelay { delay: self.win_delay }); // @2409
                }
            }
        } else if p == 1.0 {
            ev.push(WorldEvent::CaptureProgress { capture_point: cp, progress: 0.004999999888241291, team: self.owning_team }); // @2518
            self.win_at = Some(now + self.win_delay); // TriggerWinDelayed @2553 -> Delay @9736
        } else {
            ev.push(WorldEvent::CaptureProgress { capture_point: cp, progress: 1.0 - p, team: self.owning_team }); // @2610
        }
        if !self.objectives_completed && p == 1.0 {
            self.objectives_completed = true; // @2761
            ev.push(WorldEvent::ObjectivesCompleted { capture_point: cp }); // OnObjectivesCompleted @2772 (empty in BP)
        }
    }

    /// CompleteCaptureDelayed(NewCaptor) @9800: PendingNewCaptor; ObjectiveCompleteDelay 0 -> now (@9875), else after it
    fn complete_capture_delayed(&mut self, captor: Option<u8>, now: f64, ev: &mut Vec<WorldEvent>) {
        if self.complete_delay == 0.0 {
            self.complete_capture(captor, ev);
        } else {
            self.complete_at = Some((now + self.complete_delay, captor));
        }
    }

    /// @428: SetCaptureProgress(1.0, PendingNewCaptor, false); BP_FrontlineGameState.PushWinDelay = ObjectiveWinDelay
    fn complete_capture(&mut self, captor: Option<u8>, ev: &mut Vec<WorldEvent>) {
        ev.push(WorldEvent::CaptureProgress { capture_point: self.actor, progress: 1.0, team: captor });
        ev.push(WorldEvent::PushWinDelay { delay: self.win_delay });
    }

    /// The latent Delays. TriggerWinDelayed's continuation (@15): a Frontline game mode with the match in progress ->
    /// the owning team's score to 0 (owner 0 -> SetTeamScore(0, 0) @342, 1 -> SetTeamScore(1, 0) @385), any other
    /// owner -> SetCaptureProgress(0, OwningTeam) @306. `fl_in_progress` = Cast<BP_FrontlineGameMode> succeeded and
    /// IsMatchInProgress (else nothing, @101 / @157)
    pub fn tick(&mut self, now: f64, fl_in_progress: bool, ev: &mut Vec<WorldEvent>) {
        if let Some(t) = self.win_at {
            if now >= t {
                self.win_at = None;
                if fl_in_progress {
                    match self.owning_team {
                        Some(0) => ev.push(WorldEvent::TeamScore { team: 0, score: 0.0 }),
                        Some(1) => ev.push(WorldEvent::TeamScore { team: 1, score: 0.0 }),
                        t => ev.push(WorldEvent::CaptureProgress { capture_point: self.actor, progress: 0.0, team: t }),
                    }
                }
            }
        }
        if let Some((t, captor)) = self.complete_at {
            if now >= t {
                self.complete_at = None;
                self.complete_capture(captor, ev);
            }
        }
    }
}

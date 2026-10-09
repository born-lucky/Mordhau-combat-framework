//! The Frontline bridge between mh-world's BP_CapturePoint objective layer (capture.rs) and mh-mode's native
//! AControlPoint / BP_FrontlineGameMode (mh_mode::GameMode). A host owning both calls `FlBridge::sync` once per frame
//! after the game mode's tick (mode -> world: owners, prerequisites, match state) and `FlBridge::apply` with the
//! world's events (world -> mode).
//!
//! - World -> mode: `CaptureProgress { cp, progress, team }` becomes AControlPoint::SetCaptureProgress(progress, team or
//!   255, false) (rva 0x1515740, mh-mode `ControlPoint::set_capture_progress`) with its AddScore awards. `TeamScore`
//!   becomes SetTeamScore (BP_CapturePoint TriggerWinDelayed @342 / @385).
//! - Mode -> world: a changed OwningTeam goes to `World::set_capture_point_owner`. The enemy-prerequisite flips
//!   AControlPoint::Tick rva 0x1517270 raises (b_team1_owns_prerequisites while team 1 owns the point,
//!   b_team2_owns_prerequisites while team 0 does: mh-mode control_point.rs `tick`) go to
//!   `World::capture_point_prerequisites`. IsMatchInProgress goes to `World::fl_match_in_progress`.
//!
//! A capture point actor is matched to mh-mode's control point by actor name (ControlPointDef.name is the placed
//! actor's name).

use crate::world::{ActorId, World, WorldEvent};
use mh_mode::control_point::{CpCtx, NO_TEAM};
use mh_mode::GameMode;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct FlBridge {
    /// world capture point actor -> mh-mode control point index
    pub cp_index: BTreeMap<ActorId, usize>,
    /// last seen (OwningTeam, b_team1_owns_prerequisites, b_team2_owns_prerequisites) per control point
    last: BTreeMap<usize, (i64, bool, bool)>,
}

fn team_of(t: i64) -> Option<u8> {
    (0..255).contains(&t).then_some(t as u8)
}

impl FlBridge {
    pub fn new(w: &World, gm: &GameMode) -> FlBridge {
        let mut b = FlBridge::default();
        for cp in w.cps.keys() {
            if let Some(i) = gm.point(&w.actors[*cp].name) {
                b.cp_index.insert(*cp, i);
            }
        }
        b
    }

    /// Mode -> world, after GameMode::tick: match state, owners, prerequisite flips. Returns the world's events
    /// (a prerequisite change can activate / disable objectives; none today, kept for the contract)
    pub fn sync(&mut self, gm: &GameMode, w: &mut World) -> Vec<WorldEvent> {
        w.fl_match_in_progress = gm.in_progress();
        for (&cp, &i) in &self.cp_index {
            let c = &gm.control_points[i];
            let now = (c.owning_team, c.b_team1_owns_prerequisites, c.b_team2_owns_prerequisites);
            let prev = self.last.insert(i, now);
            if prev.map(|p| p.0) != Some(now.0) {
                w.set_capture_point_owner(cp, team_of(now.0));
            }
            let (p1, p2) = prev.map(|p| (p.1, p.2)).unwrap_or((false, false));
            // control_point.rs tick: a flip of team 1's flag while team 1 owns, of team 2's while team 0 owns
            if now.1 != p1 && now.0 == 1 {
                w.capture_point_prerequisites(cp, now.1);
            }
            if now.2 != p2 && now.0 == 0 {
                w.capture_point_prerequisites(cp, now.2);
            }
        }
        vec![]
    }

    /// World -> mode: the capture progress / team score events
    pub fn apply(&self, gm: &mut GameMode, ev: &[WorldEvent], dt: f64) {
        for e in ev {
            match e {
                WorldEvent::CaptureProgress { capture_point, progress, team } => {
                    let Some(&i) = self.cp_index.get(capture_point) else { continue };
                    let captor = team.map(|t| t as i64).unwrap_or(NO_TEAM);
                    let mut awards = Vec::new();
                    {
                        let q = gm.precision.q();
                        let (ctrls, starts) = (&mut gm.ctrls, &mut gm.starts);
                        let mut cx = CpCtx { ctrls, starts, awards: &mut awards, q };
                        gm.control_points[i].set_capture_progress(*progress, captor, false, dt, &mut cx);
                    }
                    for (c, s) in awards {
                        gm.add_score(c, s);
                    }
                    gm.update_capture_point_data();
                }
                WorldEvent::TeamScore { team, score } => gm.set_team_score(*team as i64, *score),
                _ => {}
            }
        }
    }
}

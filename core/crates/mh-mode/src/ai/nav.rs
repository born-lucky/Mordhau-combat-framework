//! The world queries a host answers from its navmesh / collision (rust-mode-ai r4): `NavQueries`, and `WithNav`, a
//! BotHost that adds them to another host (CombatHost answers the fighter inputs, the nav world the rest). mh-nav
//! implements NavQueries on a navmesh built from a map's collision (mh_level CollisionWorld through mh_character's
//! World trait).

use super::body::{BodyId, BotHost, PawnView, Stimulus};
use mordhau_core::ue::FVector;

pub trait NavQueries {
    /// UNavigationSystemV1::NavigationRaycast: true when the navmesh blocks the straight walk from a to b
    fn raycast(&mut self, a: FVector, b: FVector) -> bool;
    /// UNavigationSystemV1::GetRandomReachablePointInRadius
    fn random_reachable_point(&mut self, origin: FVector, radius: f64) -> Option<FVector>;
    /// the path the move request follows (AAIController::MoveToLocation -> FindPathSync): waypoints after the start,
    /// the last one the projected destination; None = no path
    fn find_path(&mut self, a: FVector, b: FVector) -> Option<Vec<FVector>>;
    /// FindPathToLocationSynchronously: Some(complete); the default: any path found counts as complete
    fn path_complete(&mut self, a: FVector, b: FVector) -> Option<bool> {
        Some(self.find_path(a, b).is_some())
    }
    /// UWorld::LineTraceTestByChannel against the static world (hearing): None = not answered
    fn line_blocked(&mut self, _a: FVector, _b: FVector) -> Option<bool> {
        None
    }
}

/// a BotHost (`inner`, e.g. CombatHost) plus a navmesh / collision world
pub struct WithNav<'a> {
    pub inner: &'a mut dyn BotHost,
    pub nav: &'a mut dyn NavQueries,
}

impl BotHost for WithNav<'_> {
    fn request_attack(&mut self, body: BodyId, mv: i64, angle: f64) -> PawnView {
        self.inner.request_attack(body, mv, angle)
    }
    fn request_parry(&mut self, body: BodyId, bt: i64) -> PawnView {
        self.inner.request_parry(body, bt)
    }
    fn request_feint(&mut self, body: BodyId) -> PawnView {
        self.inner.request_feint(body)
    }
    fn nav_raycast(&mut self, a: FVector, b: FVector) -> bool {
        self.nav.raycast(a, b)
    }
    fn has_nav(&self) -> bool {
        true
    }
    fn random_reachable_point(&mut self, origin: FVector, radius: f64) -> Option<FVector> {
        self.nav.random_reachable_point(origin, radius)
    }
    fn stimuli(&mut self, receiver: BodyId, target: BodyId) -> Option<Vec<Stimulus>> {
        self.inner.stimuli(receiver, target)
    }
    fn hearing_trace_blocked(&mut self, a: FVector, b: FVector) -> Option<bool> {
        self.nav.line_blocked(a, b).or_else(|| self.inner.hearing_trace_blocked(a, b))
    }
    fn path_complete(&mut self, a: FVector, b: FVector) -> Option<bool> {
        self.nav.path_complete(a, b)
    }
    fn sight_blocked(&mut self, a: FVector, b: FVector) -> Option<bool> {
        self.nav.line_blocked(a, b).or_else(|| self.inner.sight_blocked(a, b))
    }
    fn trace_first_hit(&mut self, from: FVector, to: FVector, ignore: BodyId) -> Option<Option<BodyId>> {
        self.inner.trace_first_hit(from, to, ignore)
    }
}

/// AAIController::MoveToLocation's path following for a host: the move request's destination becomes the next
/// corner of a navmesh path (FindPathSync + UPathFollowingComponent's segment advance: UNCONFIRMED thresholds), which
/// the host turns into movement input. Re-paths when the destination moves by more than `repath` cm or the path is
/// used up; a corner counts as reached within `reach` cm (2D).
#[derive(Clone, Debug)]
pub struct PathFollower {
    pub dest: Option<FVector>,
    pub path: Vec<FVector>,
    pub idx: usize,
    pub repath: f32,
    pub reach: f32,
}

impl Default for PathFollower {
    fn default() -> Self {
        PathFollower { dest: None, path: Vec::new(), idx: 0, repath: 50.0, reach: 40.0 }
    }
}

impl PathFollower {
    /// the point to steer at for a pawn at `me` moving to `dest`; None: no path (steer straight at dest)
    pub fn steer(&mut self, nav: &mut dyn NavQueries, me: FVector, dest: FVector) -> Option<FVector> {
        let d2 = |a: FVector, b: FVector| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
        let moved = self.dest.map(|d| d2(d, dest) > self.repath).unwrap_or(true);
        if moved || self.idx >= self.path.len() {
            self.dest = Some(dest);
            self.path = nav.find_path(me, dest).unwrap_or_default();
            self.idx = 0;
        }
        while self.idx + 1 < self.path.len() && d2(self.path[self.idx], me) < self.reach {
            self.idx += 1;
        }
        self.path.get(self.idx).copied()
    }

    pub fn clear(&mut self) {
        self.dest = None;
        self.path.clear();
        self.idx = 0;
    }
}

/// a BotHost (`inner`) plus the static world's line traces for the sight sense's line of sight (UAISense_Sight's
/// Visibility trace; `blocked(a, b)`: something blocks between a and b)
pub struct WithSight<'a> {
    pub inner: &'a mut dyn BotHost,
    pub blocked: &'a dyn Fn(FVector, FVector) -> bool,
}

impl BotHost for WithSight<'_> {
    fn request_attack(&mut self, body: BodyId, mv: i64, angle: f64) -> PawnView {
        self.inner.request_attack(body, mv, angle)
    }
    fn request_parry(&mut self, body: BodyId, bt: i64) -> PawnView {
        self.inner.request_parry(body, bt)
    }
    fn request_feint(&mut self, body: BodyId) -> PawnView {
        self.inner.request_feint(body)
    }
    fn nav_raycast(&mut self, a: FVector, b: FVector) -> bool {
        self.inner.nav_raycast(a, b)
    }
    fn has_nav(&self) -> bool {
        self.inner.has_nav()
    }
    fn random_reachable_point(&mut self, origin: FVector, radius: f64) -> Option<FVector> {
        self.inner.random_reachable_point(origin, radius)
    }
    fn stimuli(&mut self, receiver: BodyId, target: BodyId) -> Option<Vec<Stimulus>> {
        self.inner.stimuli(receiver, target)
    }
    fn hearing_trace_blocked(&mut self, a: FVector, b: FVector) -> Option<bool> {
        self.inner.hearing_trace_blocked(a, b).or_else(|| Some((self.blocked)(a, b)))
    }
    fn path_complete(&mut self, a: FVector, b: FVector) -> Option<bool> {
        self.inner.path_complete(a, b)
    }
    fn sight_blocked(&mut self, a: FVector, b: FVector) -> Option<bool> {
        Some((self.blocked)(a, b))
    }
    fn trace_first_hit(&mut self, from: FVector, to: FVector, ignore: BodyId) -> Option<Option<BodyId>> {
        self.inner.trace_first_hit(from, to, ignore)
    }
}

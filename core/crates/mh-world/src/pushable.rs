//! Pushables: native APushableActor (extract/native/decomp/APushableActor.cpp) under BP_FrontlinePushable
//! (Blueprints/GameModes/Battle/; the Frontline objective) and BP_SplinePushableActor (Blueprints/Interactables/; moves
//! along a TargetSplineActor's spline with its progress). Bytecode: state/world_kismet/BP_FrontlinePushable.txt,
//! BP_SplinePushableActor.txt.
//!
//! Authority side of APushableActor::Tick rva=0x166e8c0 (the non-authority network smoothing above it is mh-net's):
//! count the alive characters overlapping PushArea per team (AAdvancedCharacter +0x504 bIsDead, +0x660 Team,
//! extract/native/types/AAdvancedCharacter.h), pick a speed from the leading team's PushSpeedByPushers curve at the
//! lead (or AutoMoveSpeed when nobody is there and bAutoMoveIfAlone), and move Progress towards 1 (pushing) or the
//! lowest NonPullableThreshold at or below it (pulling, else 0) at that speed with FMath::FInterpConstantTo;
//! ReplicatedProgress = clamp((int)(Progress * 65535), 0, 65535); progress steps crossed award score.

use crate::ladder::Curve;
use crate::props::{finterp_to_constant, Props};
use crate::world::{ActorId, CharId, CharView, WorldEvent};
use crate::V;
use mh_level::xf::Xf;

/// EScoreFeedReason GiveClientScoreBP(0xc, ...) in APushableActor::Tick rva=0x166e8c0
pub const SCORE_PUSH: u8 = 12;

#[derive(Clone, Debug)]
pub struct Pushable {
    pub progress: f64,
    pub replicated_progress: u16,
    pub team1_presence: i32,
    pub team2_presence: i32,
    /// bIsPushingAllowed: ctor true (APushableActor ctor rva=0x1642ff0); BP_FrontlinePushable toggles it with the
    /// enemy's prerequisites (EnemyGainedPrerequisites@118 / EnemyLostPrerequisites@0)
    pub pushing_allowed: bool,
    pub pulling_allowed: bool,
    pub auto_move_if_alone: bool,
    pub auto_move_speed: f64,
    pub stop_if_contested: bool,
    pub team1_curve: Option<Curve>,
    pub team2_curve: Option<Curve>,
    /// sorted ascending at BeginPlay (APushableActor::BeginPlay rva=0x164ae40, Algo::IntroSort)
    pub non_pullable: Vec<f64>,
    pub step_to_award: f64,
    pub score_per_step: i64,
    /// BP_SplinePushableActor: the spline it rides and its range / offsets
    pub spline: Option<crate::spline::Spline>,
    pub spline_range: (f64, f64),
    pub spline_offsets: (f64, f64),
    /// the actor's current transform (moved along the spline)
    pub xf: Option<Xf>,
    /// client side (OnRep_ReplicatedProgress rva 0x1666670): ReplicatedProgressTarget, bHasReplicatedProgress,
    /// LastReplicatedProgressTime
    pub rep_target: f64,
    pub has_rep: bool,
    pub last_rep_time: f64,
    /// bIsNetworkInterpolationConstant (ctor false), NetworkInterpolationSpeed 4, NetworkInterpolationSpeedConstant
    /// 0.5 (APushableActor ctor rva 0x1642ff0)
    pub net_constant: bool,
    pub net_speed: f64,
    pub net_speed_constant: f64,
}

fn curve_of(pk: &mh_level::Pkgs, p: &Props, k: &str) -> Option<Curve> {
    let path = p.obj(k);
    if path.is_empty() {
        return None;
    }
    Some(Curve::load(pk, mh_level::level::pkg_of(&path)))
}

impl Pushable {
    pub fn new(pk: &mh_level::Pkgs, p: &Props, spline: Option<crate::spline::Spline>) -> Pushable {
        let mut th: Vec<f64> = p.arr("NonPullableThresholds").iter().filter_map(|v| v.as_f64()).collect();
        th.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let v2 = |k: &str| {
            let v = p.get(k);
            let g = |c: &str| v.and_then(|v| v.get(c)).and_then(|x| x.as_f64()).unwrap_or(0.0);
            (g("X"), g("Y"))
        };
        let progress = p.f("Progress").clamp(0.0, 1.0);
        Pushable {
            progress,
            replicated_progress: rep(progress),
            team1_presence: 0,
            team2_presence: 0,
            pushing_allowed: p.get("bIsPushingAllowed").and_then(|v| v.as_bool()).unwrap_or(true),
            pulling_allowed: p.b("bIsPullingAllowed"),
            auto_move_if_alone: p.b("bAutoMoveIfAlone"),
            auto_move_speed: p.f("AutoMoveSpeed"),
            stop_if_contested: p.b("bStopPushingIfContested"),
            team1_curve: curve_of(pk, p, "Team1PushSpeedByPushers"),
            team2_curve: curve_of(pk, p, "Team2PushSpeedByPushers"),
            non_pullable: th,
            step_to_award: p.f("ProgressStepToAwardScoreFor"),
            score_per_step: p.i("ScoreAwardedPerProgressStep"),
            spline,
            spline_range: v2("SplineRange"),
            spline_offsets: (p.f("SplineOffsetA"), p.f("SplineOffsetB")),
            xf: None,
            rep_target: progress,
            has_rep: false,
            last_rep_time: 0.0,
            net_constant: p.b("bIsNetworkInterpolationConstant"),
            net_speed: p.get("NetworkInterpolationSpeed").and_then(|v| v.as_f64()).unwrap_or(4.0),
            net_speed_constant: p.get("NetworkInterpolationSpeedConstant").and_then(|v| v.as_f64()).unwrap_or(0.5),
        }
    }

    /// One authority tick. `in_area`: the characters overlapping PushArea (the host's overlap test). Returns whether
    /// Progress changed (OnProgressUpdated fired)
    pub fn tick(&mut self, id: ActorId, dt: f64, in_area: &[&CharView], ev: &mut Vec<WorldEvent>) -> bool {
        let (mut t1, mut t2) = (0, 0);
        for c in in_area {
            if c.dead {
                continue;
            }
            match c.team {
                Some(0) => t1 += 1,
                Some(1) => t2 += 1,
                _ => {}
            }
        }
        self.team1_presence = t1;
        self.team2_presence = t2;
        let start = self.progress;
        let allowed = |s: f64, me: &Pushable| if (s > 0.0 && !me.pushing_allowed) || (s < 0.0 && !me.pulling_allowed) { 0.0 } else { s };
        let mut speed = 0.0;
        if t1 == t2 {
            if t1 == 0 && self.auto_move_if_alone {
                speed = allowed(self.auto_move_speed, self);
            }
        } else if t1 == 0 || t2 == 0 || !self.stop_if_contested {
            if t2 < t1 {
                if let Some(c) = &self.team1_curve {
                    speed = allowed(c.eval((t1 - t2) as f32 as f64), self);
                }
            } else if let Some(c) = &self.team2_curve {
                speed = allowed(c.eval((t2 - t1) as f32 as f64), self);
            }
        }
        if speed.abs() <= 1e-8 {
            return false;
        }
        let target = if speed > 0.0 {
            1.0
        } else {
            // the first (lowest) threshold at or below Progress, else 0
            self.non_pullable.iter().copied().find(|t| (t - self.progress).abs() <= 1e-8 || *t < self.progress).unwrap_or(0.0)
        };
        let old = self.progress;
        let new = finterp_to_constant(old, target, dt, speed.abs());
        let mut changed = false;
        if (old - new).abs() > 1e-8 {
            self.progress = new.clamp(0.0, 1.0);
            self.replicated_progress = rep(self.progress);
            changed = true;
            ev.push(WorldEvent::PushableProgress { actor: id, progress: self.progress });
        }
        // score: progress went up across a ProgressStepToAwardScoreFor boundary -> every alive character of the
        // leading team in the area with a MordhauPlayerController gets ScoreAwardedPerProgressStep / max(presence)
        if start < self.progress && self.step_to_award.abs() > 1e-8 && self.score_per_step > 0 {
            let inv = 1.0 / self.step_to_award;
            if (inv * start) as i64 != (self.progress * inv) as i64 {
                let lead = t1.max(t2) as i64;
                for c in in_area {
                    if c.dead {
                        continue;
                    }
                    let leading = match c.team {
                        Some(0) => t2 < t1,
                        Some(1) => t1 < t2,
                        _ => false,
                    };
                    if leading && c.has_player_controller {
                        ev.push(WorldEvent::Score { char: Some(c.id), team: c.team, kind: SCORE_PUSH, amount: self.score_per_step / lead });
                    }
                }
            }
        }
        changed
    }

    /// BP_SplinePushableActor:GetTransformAlongSplineOffset: OffsetA == OffsetB -> the spline transform at time
    /// Lerp(SplineRange, Progress) (constant velocity); else the location at distance Lerp(SplineRange, Progress) *
    /// length + OffsetA, facing the point at + OffsetB (MakeRotFromXZ with the spline's up there)
    pub fn spline_transform(&self) -> Option<Xf> {
        let s = self.spline.as_ref()?;
        let t = self.spline_range.0 + (self.spline_range.1 - self.spline_range.0) * self.progress;
        let (a, b) = self.spline_offsets;
        if a == b {
            return Some(s.transform_at_time(t));
        }
        let d = t * s.length();
        let pb = s.location_at_distance(d + b);
        let pa = s.location_at_distance(d + a);
        let up = s.up_at_distance(d + a);
        let x = crate::spline::normal([pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]], 1e-4);
        Some(crate::spline::make_rot_from_xz(x, up, pa))
    }
}

impl Pushable {
    /// APushableActor::OnRep_ReplicatedProgress rva=0x1666670: target = r * 1.5259022e-05; the first one sets Progress
    /// (OnProgressUpdated when it moved). Returns whether Progress changed
    pub fn on_rep_progress(&mut self, id: ActorId, r: u16, now: f64, ev: &mut Vec<WorldEvent>) -> bool {
        self.replicated_progress = r;
        self.last_rep_time = now;
        self.rep_target = (r as f32 * 1.5259022e-05f32) as f64;
        if self.has_rep {
            return false;
        }
        self.has_rep = true;
        let old = self.progress;
        self.progress = self.rep_target;
        let moved = (old - self.progress).abs() > 1e-8;
        if moved {
            ev.push(WorldEvent::PushableProgress { actor: id, progress: self.progress });
        }
        moved
    }

    /// The non-authority head of APushableActor::Tick rva=0x166e8c0: towards ReplicatedProgressTarget with
    /// FInterpConstantTo(NetworkInterpolationSpeedConstant) when bIsNetworkInterpolationConstant, the target is 1, or
    /// no rep for 1 s; else FMath::FInterpTo(NetworkInterpolationSpeed), snapping within 1e-4
    pub fn client_tick(&mut self, id: ActorId, dt: f64, now: f64, ev: &mut Vec<WorldEvent>) -> bool {
        let (t, p) = (self.rep_target, self.progress);
        if (t - p).abs() <= 1e-8 {
            return false;
        }
        let new = if self.net_constant || t == 1.0 || self.last_rep_time + 1.0 < now {
            finterp_to_constant(p, t, dt, self.net_speed_constant)
        } else {
            let n = finterp_to(p, t, dt, self.net_speed);
            if (n - t).abs() <= 0.0001 { t } else { n }
        };
        self.progress = new;
        if (p - new).abs() > 1e-8 {
            ev.push(WorldEvent::PushableProgress { actor: id, progress: new });
            return true;
        }
        false
    }
}

/// FMath::FInterpTo (UE 4.26 UnrealMathUtility.cpp): speed <= 0 -> target; dist^2 < 1e-8 -> target; else
/// current + dist * clamp(dt * speed, 0, 1)
pub fn finterp_to(current: f64, target: f64, dt: f64, speed: f64) -> f64 {
    if speed <= 0.0 {
        return target;
    }
    let d = target - current;
    if d * d < 1e-8 {
        return target;
    }
    current + d * (dt * speed).clamp(0.0, 1.0)
}

/// ReplicatedProgress = (uint16)clamp((int)(p * 65535), 0, 0xffff) (SetProgress rva=0x166d4c0)
fn rep(p: f64) -> u16 {
    let i = (p as f32 * 65535.0) as i32;
    i.clamp(0, 0xffff) as u16
}

/// BP_SplinePushableActor:OnProgressUpdated @1104..@1037: the actor moves to its new spline transform; (authority)
/// every actor its SkeletalMesh overlaps: characters not in a vehicle are carried by the move (K2_AddActorWorldOffset
/// of the location delta, @644); other MordhauActors that are not Frontline destroyables take 1e7 damage (@970)
pub fn carry(id: ActorId, prev: V, now_loc: V, chars: &[CharId], actors: &[ActorId], is_destroyable: &dyn Fn(ActorId) -> bool, in_vehicle: &dyn Fn(CharId) -> bool, ev: &mut Vec<WorldEvent>) {
    let d = [now_loc[0] - prev[0], now_loc[1] - prev[1], now_loc[2] - prev[2]];
    for c in chars {
        if !in_vehicle(*c) {
            ev.push(WorldEvent::MoveCharacter { char: *c, offset: d });
        }
    }
    for a in actors {
        if *a != id && !is_destroyable(*a) {
            ev.push(WorldEvent::DamageActor { actor: *a, amount: 10_000_000.0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(v: &[(f64, f64)]) -> Curve {
        Curve { keys: v.iter().map(|(t, x)| (*t, *x, "RCIM_Linear".to_string(), 0.0, 0.0)).collect() }
    }
    fn pushable() -> Pushable {
        Pushable {
            progress: 0.5,
            replicated_progress: rep(0.5),
            team1_presence: 0,
            team2_presence: 0,
            pushing_allowed: true,
            pulling_allowed: true,
            auto_move_if_alone: false,
            auto_move_speed: 0.0,
            stop_if_contested: true,
            team1_curve: Some(curve(&[(1.0, 0.01), (3.0, 0.03)])),
            team2_curve: Some(curve(&[(1.0, -0.02)])),
            non_pullable: vec![0.2, 0.4],
            step_to_award: 0.1,
            score_per_step: 10,
            spline: None,
            spline_range: (0.0, 1.0),
            spline_offsets: (0.0, 0.0),
            xf: None,
            rep_target: 0.5,
            has_rep: false,
            last_rep_time: 0.0,
            net_constant: false,
            net_speed: 4.0,
            net_speed_constant: 0.5,
        }
    }
    fn ch(id: CharId, team: u8) -> CharView {
        CharView { id, team: Some(team), has_player_controller: true, ..Default::default() }
    }

    /// APushableActor::Tick rva=0x166e8c0: the lead picks the leading team's curve; contested stops; pulling stops at
    /// the lowest threshold at or below; score per crossed step split by the leading presence
    #[test]
    fn tick_rules() {
        let (a, b, c, e) = (ch(1, 0), ch(2, 0), ch(3, 1), ch(4, 0));
        let mut p = pushable();
        let mut ev = vec![];
        // two team-1 pushers: curve(2) = 0.02 / s
        assert!(p.tick(7, 1.0, &[&a, &b], &mut ev));
        assert!((p.progress - 0.52).abs() < 1e-9 && (p.team1_presence, p.team2_presence) == (2, 0));
        assert_eq!(p.replicated_progress, (0.52f32 * 65535.0) as u16);
        // contested with bStopPushingIfContested: nothing
        assert!(!p.tick(7, 1.0, &[&a, &b, &c], &mut ev));
        // a dead pusher does not count
        let dead = CharView { dead: true, ..e.clone() };
        assert!(p.tick(7, 1.0, &[&c, &dead], &mut ev));
        assert!((p.progress - 0.50).abs() < 1e-9);
        // pulling towards the lowest threshold at or below 0.5: 0.2 (sorted thresholds, first match)
        for _ in 0..40 {
            p.tick(7, 1.0, &[&c], &mut ev);
        }
        assert!((p.progress - 0.2).abs() < 1e-9, "{}", p.progress);
        // not allowed to push: nothing
        p.pushing_allowed = false;
        assert!(!p.tick(7, 1.0, &[&a], &mut ev));
        // score: crossing 0.3 with 3 pushers -> 10 / 3 each
        p.pushing_allowed = true;
        p.progress = 0.295;
        ev.clear();
        let f = ch(5, 0);
        p.tick(7, 1.0, &[&a, &b, &f], &mut ev);
        let scores: Vec<_> = ev.iter().filter(|e| matches!(e, WorldEvent::Score { amount: 3, kind: 12, .. })).collect();
        assert_eq!(scores.len(), 3, "{ev:?}");
    }
}

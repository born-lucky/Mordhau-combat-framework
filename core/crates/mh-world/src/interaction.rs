//! The use-key target: UInteractionSystemComponent (native, the character's component). The host sweeps the shapes
//! `sweeps` returns against its collision (sphere radius 30, channel 18, complex, ignoring the character and its two
//! hand equipment, initial overlaps found) and hands the hits, mapped to world actors (mh-level
//! `CollisionWorld::body_actors`), to `World::interaction_target`.
//!
//! Constants: the ctor rva 0x14af0e0 (SweepDistance 170, NumberOfSweeps 8, SweepSphereRadius 30, SweepRadius 50,
//! ActorInteractionDistance 130). Selection: GetInteractionTarget rva 0x14bd220. Validation:
//! ValidateInteractionTarget rva 0x14e1780 (an Interactable whose CanInteract or CanHeldInteract holds; the
//! "can initiate UInteractWithMotion" part is the character's, `CharView` callers pass only characters that can).

use crate::V;

pub const SWEEP_DISTANCE: f64 = 170.0;
pub const NUMBER_OF_SWEEPS: usize = 8;
pub const SWEEP_SPHERE_RADIUS: f64 = 30.0;
pub const SWEEP_RADIUS: f64 = 50.0;
/// ECC channel index of the sweep (UWorld::SweepMultiByChannel(..., 0x12, ...) in GetInteractionTarget)
pub const SWEEP_CHANNEL: u8 = 18;

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
/// FVector::GetSafeNormal (tolerance 1e-8; zero vector below)
fn safe_normal(a: V) -> V {
    let l2 = dot(a, a);
    if l2 < 1e-8 {
        [0.0; 3]
    } else {
        mul(a, 1.0 / l2.sqrt())
    }
}

/// FVector::FindBestAxisVectors: Axis1 = (Z-dominant ? X : Z) made orthogonal to the vector; Axis2 = Axis1 x vector
fn find_best_axis_vectors(n: V) -> (V, V) {
    let (ax, ay, az) = (n[0].abs(), n[1].abs(), n[2].abs());
    let a = if az > ax && az > ay { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let a1 = safe_normal(sub(a, mul(n, dot(a, n))));
    (a1, cross(a1, n))
}

/// FVector::RotateAngleAxis (degrees, unit axis): Rodrigues
fn rotate_angle_axis(v: V, deg: f64, k: V) -> V {
    let (s, c) = deg.to_radians().sin_cos();
    add(add(mul(v, c), mul(cross(k, v), s)), mul(k, dot(k, v) * (1.0 - c)))
}

/// One sweep: start, end (CalculateSweepParameters rva 0x14b45f0)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    pub start: V,
    pub end: V,
}

/// The 8 sweeps from the eye viewpoint (APawn::GetActorEyesViewPoint, vtable +0x600) along the view direction:
/// length SweepDistance + 40 - SweepSphereRadius (GetInteractionTarget) times the mesh scale (`mesh_scale`, 1 for the
/// player mesh); sweep 0 on the view line, sweeps 1..7 offset SweepRadius around it at ((i - 1) / (N - 1)) * 360 deg
/// about the line, starting from FindBestAxisVectors' first axis of (start - end)
pub fn sweeps(eye: V, view_dir: V, mesh_scale: V) -> Vec<Sweep> {
    let len = SWEEP_DISTANCE + 40.0 - SWEEP_SPHERE_RADIUS;
    let f = safe_normal(view_dir);
    let end0 = add(eye, [f[0] * len * mesh_scale[0], f[1] * len * mesh_scale[1], f[2] * len * mesh_scale[2]]);
    let mut out = vec![Sweep { start: eye, end: end0 }];
    let mut back = safe_normal(sub(eye, end0));
    if back == [0.0; 3] {
        back = [0.0, 0.0, 1.0];
    }
    let (a1, _) = find_best_axis_vectors(back);
    for i in 1..NUMBER_OF_SWEEPS {
        let ang = ((i - 1) as f64 / (NUMBER_OF_SWEEPS - 1) as f64) * 360.0;
        let off = mul(rotate_angle_axis(a1, ang, back), SWEEP_RADIUS);
        out.push(Sweep { start: add(eye, off), end: add(end0, off) });
    }
    out
}

/// A hit of one of the sweeps on a world actor (a PhysicsProxy hit resolved to its owner by the host)
#[derive(Clone, Copy, Debug)]
pub struct SweepHit {
    pub sweep: usize,
    pub actor: crate::ActorId,
    /// the hit's impact point
    pub point: V,
}

/// FMath::PointDistToLine (line through `origin` along `dir`, normalized here)
pub fn point_dist_to_line(p: V, dir: V, origin: V) -> f64 {
    let d = safe_normal(dir);
    let v = sub(p, origin);
    let c = add(origin, mul(d, dot(v, d)));
    let e = sub(p, c);
    dot(e, e).sqrt()
}

/// GetInteractionTarget's filter and choice over validated hits: within SweepRadius + SweepSphereRadius (80) of the
/// view line, within ActorInteractionDistance (130) of the eye in XY (squared 16900), and (nearer than
/// SweepDistance + 40 to its sweep's start, or no target yet); the best is the one nearest the view line (the
/// threshold shrinks to each accepted hit's distance) [the further MordhauActor mesh-local test after the 16900
/// check is UNCONFIRMED and not ported]
pub fn choose(eye: V, view_dir: V, sw: &[Sweep], hits: &[SweepHit], valid: &dyn Fn(crate::ActorId) -> bool) -> Option<crate::ActorId> {
    let mut best: Option<crate::ActorId> = None;
    let mut thresh = SWEEP_RADIUS + SWEEP_SPHERE_RADIUS;
    let center = safe_normal(view_dir);
    for h in hits {
        let Some(s) = sw.get(h.sweep) else { continue };
        let d_start = dot(sub(s.start, h.point), sub(s.start, h.point)).sqrt();
        if !valid(h.actor) {
            continue;
        }
        let d_line = point_dist_to_line(h.point, center, eye);
        let (dx, dy) = (eye[0] - h.point[0], eye[1] - h.point[1]);
        if (d_start < SWEEP_DISTANCE + 40.0 || best.is_none()) && d_line < thresh && dx * dx + dy * dy < 16900.0 {
            best = Some(h.actor);
            thresh = d_line;
        }
    }
    best
}

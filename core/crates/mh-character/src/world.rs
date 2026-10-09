//! The collision queries the character movement makes, behind a trait the host implements (the Bevy runtime with its
//! level geometry, a Skyrim host with Havok, ...). In the exe these are PhysX scene queries made through
//! UWorld::ComponentSweepMulti (from UPrimitiveComponent::MoveComponentImpl rva=0x2fb4250, call at 0x142fb485e),
//! UWorld::SweepSingleByChannel rva=0x2f68ea0, UWorld::LineTraceSingleByChannel rva=0x2f5fd00 and
//! UWorld::OverlapBlockingTestByChannel rva=0x2f62150; the movement code above them (pull-back, hit selection, floor
//! and step logic) is ported in `exe::cmc`. The capsule axis is always world Z (the character's capsule never tilts).
//!
//! `BoxWorld` is a small exact implementation for tests: axis-aligned boxes and bounded planes (slopes), queried with
//! a vertical capsule (a vertical segment plus a radius: box (+) segment is a taller box, so a capsule sweep is a point
//! sweep against that box rounded by the radius, split into its convex pieces).

use crate::ue::FVector;
use crate::uemath::{sub, v};

/// FHitResult, the fields the character movement reads (layout of this build: flags byte, FaceIndex, Time +0x8,
/// Distance +0xc, Location +0x10, ImpactPoint +0x1c, Normal +0x28, ImpactNormal +0x34, TraceStart +0x40, TraceEnd +0x4c,
/// PenetrationDepth +0x58, as MoveAlongFloor rva=0x2f7bef0 and ComputeGroundMovementDelta rva=0x2f758a0 index it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitResult {
    pub blocking_hit: bool,
    pub start_penetrating: bool,
    pub time: f32,
    pub distance: f32,
    /// shape centre at the hit
    pub location: FVector,
    pub impact_point: FVector,
    /// swept-shape normal
    pub normal: FVector,
    /// surface normal of the hit geometry
    pub impact_normal: FVector,
    pub trace_start: FVector,
    pub trace_end: FVector,
    pub penetration_depth: f32,
    /// host id of the hit component (the movement base); None = no component ("fake" hit)
    pub component: Option<u32>,
    /// UPrimitiveComponent::CanCharacterStepUp(Pawn) of the hit component (CanCharacterStepUpOn == ECB_Yes)
    pub can_step_up: bool,
}

impl HitResult {
    /// FHitResult(float InTime): zeroed, Time = InTime
    pub fn new(time: f32) -> Self {
        let z = FVector::ZERO;
        HitResult {
            blocking_hit: false,
            start_penetrating: false,
            time,
            distance: 0.0,
            location: z,
            impact_point: z,
            normal: z,
            impact_normal: z,
            trace_start: z,
            trace_end: z,
            penetration_depth: 0.0,
            component: None,
            can_step_up: true,
        }
    }
    /// FHitResult::IsValidBlockingHit: bBlockingHit && !bStartPenetrating
    pub fn is_valid_blocking_hit(&self) -> bool {
        self.blocking_hit && !self.start_penetrating
    }
}

impl Default for HitResult {
    fn default() -> Self {
        HitResult::new(1.0)
    }
}

/// Geometric queries against the static world, capsule axis = world Z. Hits are the raw geometric results (PhysX):
/// the movement code applies the engine's pull-back and selection rules itself.
pub trait World {
    /// every blocking hit of a capsule (radius, half height including the caps) swept from start to end, earliest
    /// first; a shape the capsule already overlaps at start gives a start_penetrating hit at time 0 with the
    /// depenetration direction as normal and its depth as penetration_depth
    fn sweep_capsule(&self, start: FVector, end: FVector, radius: f32, half_height: f32) -> Vec<HitResult>;
    /// first blocking hit of a ray
    fn line_trace(&self, start: FVector, end: FVector) -> Option<HitResult>;
    /// whether a capsule at centre overlaps any blocking geometry
    fn overlap_capsule(&self, centre: FVector, radius: f32, half_height: f32) -> bool;
    /// every blocking hit of a box (half extent, rotation quaternion X, Y, Z, W) swept from start to end, earliest
    /// first (the projectile's BoxComponent). `responses`: the box's responses to object channels by name (0 Ignore,
    /// 1 Overlap, 2 Block; unlisted = Block), so a body blocks when its own response to the "Projectile" channel and
    /// the box's response to its object type are both Block. The default answers with the ray of its centre (exact
    /// for a box of zero extent; UNCONFIRMED beyond that) - mh-level's CollisionWorld overrides it with a real box sweep.
    fn sweep_box(&self, start: FVector, end: FVector, half_extent: FVector, quat: [f32; 4], responses: &[(String, u8)]) -> Vec<HitResult> {
        let _ = (half_extent, quat, responses);
        self.line_trace(start, end).into_iter().collect()
    }
    /// every blocking hit of a box swept from start to end on the object channel `channel` ("Vehicle" for a catapult's
    /// secondary boxes): a body blocks when its response to `channel` and the box's response to the body's object
    /// type (`responses`, unlisted = Block) are both Block. Default: the projectile query (`sweep_box`) for "Projectile",
    /// the same answer for any other channel (UNCONFIRMED for worlds that do not override it).
    fn sweep_box_channel(&self, start: FVector, end: FVector, half_extent: FVector, quat: [f32; 4], channel: &str, responses: &[(String, u8)]) -> Vec<HitResult> {
        let _ = channel;
        self.sweep_box(start, end, half_extent, quat, responses)
    }

    /// whether a box at `centre` overlaps blocking geometry of `channel` (UWorld::OverlapBlockingTestByChannel with a
    /// box shape). Default: a start-penetrating hit of a 0.01 cm box sweep.
    fn overlap_box(&self, centre: FVector, half_extent: FVector, quat: [f32; 4], channel: &str, responses: &[(String, u8)]) -> bool {
        let end = FVector { x: centre.x, y: centre.y, z: centre.z + 0.01 };
        self.sweep_box_channel(centre, end, half_extent, quat, channel, responses).iter().any(|h| h.start_penetrating)
    }
    /// `sweep_capsule` for a querying object of channel `channel` with the responses `responses` (0 Ignore, 1 Overlap,
    /// 2 Block; unlisted = Block): the catapult's CollisionCylinder ("Vehicle") and BackCapsule. Default: the Pawn
    /// query (worlds without collision channels).
    fn sweep_capsule_channel(&self, start: FVector, end: FVector, radius: f32, half_height: f32, channel: &str, responses: &[(String, u8)]) -> Vec<HitResult> {
        let _ = (channel, responses);
        self.sweep_capsule(start, end, radius, half_height)
    }
    /// `line_trace` on a querying channel (default: the Pawn query)
    fn line_trace_channel(&self, start: FVector, end: FVector, channel: &str, responses: &[(String, u8)]) -> Option<HitResult> {
        let _ = (channel, responses);
        self.line_trace(start, end)
    }
    /// `overlap_capsule` on a querying channel (default: the Pawn query)
    fn overlap_capsule_channel(&self, centre: FVector, radius: f32, half_height: f32, channel: &str, responses: &[(String, u8)]) -> bool {
        let _ = (channel, responses);
        self.overlap_capsule(centre, radius, half_height)
    }
    /// UWorld::FindTeleportSpot rva=0x31a3b60 for a pawn capsule: Some(the location, adjusted out of encroaching
    /// geometry) or None when nothing fits. The default only accepts a free spot as is; mh-level's CollisionWorld
    /// overrides it with the port of the engine's adjustment table.
    fn find_teleport_spot(&self, loc: FVector, radius: f32, half_height: f32) -> Option<FVector> {
        if self.overlap_capsule(loc, radius, half_height) {
            None
        } else {
            Some(loc)
        }
    }
}

// ---- BoxWorld ------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Shape {
    /// axis-aligned box
    Box { min: FVector, max: FVector },
    /// half-space n.p <= d (n unit), solid only inside the vertical prism [lo, hi] (x, y): a slope
    Plane { n: FVector, d: f32, lo: (f32, f32), hi: (f32, f32) },
}

/// Test world of boxes and bounded planes. f64 internally: it stands in for PhysX, whose results the exe takes as
/// given; the movement code consumes them as f32.
#[derive(Clone, Debug, Default)]
pub struct BoxWorld {
    pub shapes: Vec<Shape>,
    /// per-shape CanCharacterStepUpOn (default yes)
    pub no_step_up: Vec<usize>,
}

type D3 = [f64; 3];

fn d3(a: FVector) -> D3 {
    [a.x as f64, a.y as f64, a.z as f64]
}
fn fv(a: D3) -> FVector {
    v(a[0] as f32, a[1] as f32, a[2] as f32)
}
fn dsub(a: D3, b: D3) -> D3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dadd(a: D3, b: D3) -> D3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn dmul(a: D3, s: f64) -> D3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn ddot(a: D3, b: D3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn dnorm(a: D3) -> D3 {
    let l = ddot(a, a).sqrt();
    if l == 0.0 {
        a
    } else {
        dmul(a, 1.0 / l)
    }
}

/// entry/exit of a ray p + t*d against an AABB (slab test); None if missed
fn ray_box(p: D3, d: D3, lo: D3, hi: D3) -> Option<(f64, f64, usize, f64)> {
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    let (mut axis, mut sign) = (0usize, 0.0f64);
    for i in 0..3 {
        if d[i].abs() < 1e-12 {
            if p[i] < lo[i] || p[i] > hi[i] {
                return None;
            }
        } else {
            let (mut a, mut b) = ((lo[i] - p[i]) / d[i], (hi[i] - p[i]) / d[i]);
            let mut s = -1.0;
            if a > b {
                std::mem::swap(&mut a, &mut b);
                s = 1.0;
            }
            if a > t0 {
                t0 = a;
                axis = i;
                sign = s;
            }
            if b < t1 {
                t1 = b;
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some((t0, t1, axis, sign))
}

/// earliest entry t >= 0 of the ray into a sphere
fn ray_sphere(p: D3, d: D3, c: D3, r: f64) -> Option<f64> {
    let m = dsub(p, c);
    let a = ddot(d, d);
    if a < 1e-18 {
        return None;
    }
    let b = ddot(m, d);
    let cc = ddot(m, m) - r * r;
    let disc = b * b - a * cc;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    Some(t)
}

/// earliest entry of the ray into a finite axis-aligned cylinder (axis k, centre line through c, extent [a0, a1] on k)
fn ray_cylinder(p: D3, d: D3, k: usize, c: D3, a0: f64, a1: f64, r: f64) -> Option<f64> {
    let (i, j) = match k {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let (mx, my) = (p[i] - c[i], p[j] - c[j]);
    let (dx, dy) = (d[i], d[j]);
    let a = dx * dx + dy * dy;
    if a < 1e-18 {
        return None;
    }
    let b = mx * dx + my * dy;
    let cc = mx * mx + my * my - r * r;
    let disc = b * b - a * cc;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    let z = p[k] + t * d[k];
    if z < a0 || z > a1 {
        return None;
    }
    Some(t)
}

impl BoxWorld {
    pub fn new() -> Self {
        BoxWorld::default()
    }
    pub fn add_box(&mut self, min: FVector, max: FVector) -> usize {
        self.shapes.push(Shape::Box { min, max });
        self.shapes.len() - 1
    }
    /// a slope: solid below the plane through `point` with unit normal `n`, inside the prism [lo, hi] (x, y)
    pub fn add_plane(&mut self, n: FVector, point: FVector, lo: (f32, f32), hi: (f32, f32)) -> usize {
        let nd = dnorm(d3(n));
        let d = ddot(nd, d3(point));
        self.shapes.push(Shape::Plane { n: fv(nd), d: d as f32, lo, hi });
        self.shapes.len() - 1
    }

    /// signed clearance of a capsule (centre c, segment half length a, radius r) to shape s (<0 = overlap), with the
    /// outward separation direction and the closest point on the shape
    fn clearance(&self, s: &Shape, c: D3, a: f64, r: f64) -> (f64, D3, D3) {
        match s {
            Shape::Box { min, max } => {
                let (lo, hi) = (d3(*min), d3(*max));
                let px = c[0].clamp(lo[0], hi[0]);
                let py = c[1].clamp(lo[1], hi[1]);
                let (z0, z1) = (c[2] - a, c[2] + a);
                // closest points of the vertical segment [z0, z1] and the box: shared z when the ranges overlap
                let (pz, sz) = if z1 < lo[2] {
                    (lo[2], z1)
                } else if z0 > hi[2] {
                    (hi[2], z0)
                } else {
                    let z = c[2].clamp(lo[2].max(z0), hi[2].min(z1));
                    (z, z)
                };
                let segp = [c[0], c[1], sz];
                let bp = [px, py, pz];
                let dvec = dsub(segp, bp);
                let dist = ddot(dvec, dvec).sqrt();
                if dist > 1e-9 {
                    (dist - r, dmul(dvec, 1.0 / dist), bp)
                } else {
                    // segment axis inside the box: push out along the shallowest axis
                    let mut best = (f64::INFINITY, [0.0, 0.0, 1.0]);
                    for i in 0..3 {
                        let ext = if i == 2 { a } else { 0.0 };
                        let up = hi[i] - (c[i] - ext);
                        let dn = (c[i] + ext) - lo[i];
                        let mut nn = [0.0; 3];
                        if up < dn {
                            nn[i] = 1.0;
                            if up < best.0 {
                                best = (up, nn);
                            }
                        } else {
                            nn[i] = -1.0;
                            if dn < best.0 {
                                best = (dn, nn);
                            }
                        }
                    }
                    (-(best.0 + r), best.1, bp)
                }
            }
            Shape::Plane { n, d, .. } => {
                let nn = d3(*n);
                let sd = ddot(nn, c) - *d as f64 - a * nn[2].abs() - r;
                let foot = dsub(dsub(c, [0.0, 0.0, a * nn[2].signum()]), dmul(nn, r + sd));
                (sd, nn, foot)
            }
        }
    }

    fn in_prism(s: &Shape, p: D3) -> bool {
        match s {
            Shape::Plane { lo, hi, .. } => {
                p[0] >= lo.0 as f64 - 1e-6 && p[0] <= hi.0 as f64 + 1e-6 && p[1] >= lo.1 as f64 - 1e-6 && p[1] <= hi.1 as f64 + 1e-6
            }
            _ => true,
        }
    }

    /// earliest time in [0, 1] the capsule (point c with segment half length a, radius r) moving by dl touches s
    fn sweep_shape(&self, idx: usize, s: &Shape, c: D3, dl: D3, a: f64, r: f64) -> Option<(f64, D3)> {
        match s {
            Shape::Box { min, max } => {
                let lo = dsub(d3(*min), [0.0, 0.0, a]);
                let hi = dadd(d3(*max), [0.0, 0.0, a]);
                let mut best: Option<(f64, D3)> = None;
                let mut take = |t: f64, nrm: D3| {
                    if (0.0..=1.0).contains(&t) && best.map_or(true, |b| t < b.0) {
                        best = Some((t, nrm));
                    }
                };
                // three face slabs
                for k in 0..3 {
                    let mut l = lo;
                    let mut h = hi;
                    l[k] -= r;
                    h[k] += r;
                    if let Some((t0, _t1, ax, sg)) = ray_box(c, dl, l, h) {
                        if ax == k {
                            let mut nrm = [0.0; 3];
                            nrm[k] = sg;
                            take(t0, nrm);
                        }
                    }
                }
                // twelve edges
                for k in 0..3 {
                    let (i, j) = match k {
                        0 => (1, 2),
                        1 => (0, 2),
                        _ => (0, 1),
                    };
                    for ci in [lo[i], hi[i]] {
                        for cj in [lo[j], hi[j]] {
                            let mut cc = [0.0; 3];
                            cc[i] = ci;
                            cc[j] = cj;
                            if let Some(t) = ray_cylinder(c, dl, k, cc, lo[k], hi[k], r) {
                                let hp = dadd(c, dmul(dl, t));
                                let mut nrm = dsub(hp, cc);
                                nrm[k] = 0.0;
                                take(t, dnorm(nrm));
                            }
                        }
                    }
                }
                // eight corners
                for x in [lo[0], hi[0]] {
                    for y in [lo[1], hi[1]] {
                        for z in [lo[2], hi[2]] {
                            if let Some(t) = ray_sphere(c, dl, [x, y, z], r) {
                                let hp = dadd(c, dmul(dl, t));
                                take(t, dnorm(dsub(hp, [x, y, z])));
                            }
                        }
                    }
                }
                let _ = idx;
                best
            }
            Shape::Plane { n, d, .. } => {
                let nn = d3(*n);
                let s0 = ddot(nn, c) - *d as f64 - a * nn[2].abs() - r;
                let rate = ddot(nn, dl);
                if rate >= 0.0 {
                    return None;
                }
                let t = -s0 / rate;
                if !(0.0..=1.0).contains(&t) {
                    return None;
                }
                Some((t, nn))
            }
        }
    }

    fn hit_for(&self, idx: usize, s: &Shape, start: D3, dl: D3, t: f64, nrm: D3, a: f64, r: f64) -> HitResult {
        let loc = dadd(start, dmul(dl, t));
        let (_, _, bp) = self.clearance(s, loc, a, r);
        let impact_normal = match s {
            Shape::Plane { n, .. } => d3(*n),
            Shape::Box { min, max } => {
                // FindBoxOpposingNormal: of the faces the contact point lies on, the one most opposed to the sweep
                let (lo, hi) = (d3(*min), d3(*max));
                let dir = dnorm(dl);
                let mut best = (f64::INFINITY, nrm);
                for i in 0..3 {
                    for (face, sg) in [(lo[i], -1.0), (hi[i], 1.0)] {
                        if (bp[i] - face).abs() < 1e-4 {
                            let mut fnrm = [0.0; 3];
                            fnrm[i] = sg;
                            let dd = if ddot(dir, dir) > 0.0 { ddot(fnrm, dir) } else { -ddot(fnrm, nrm) };
                            if dd < best.0 {
                                best = (dd, fnrm);
                            }
                        }
                    }
                }
                best.1
            }
        };
        let mut h = HitResult::new(t as f32);
        h.blocking_hit = true;
        h.distance = (t * ddot(dl, dl).sqrt()) as f32;
        h.location = fv(loc);
        h.impact_point = fv(bp);
        h.normal = fv(nrm);
        h.impact_normal = fv(impact_normal);
        h.trace_start = fv(start);
        h.trace_end = fv(dadd(start, dl));
        h.component = Some(idx as u32);
        h.can_step_up = !self.no_step_up.contains(&idx);
        h
    }
}

impl World for BoxWorld {
    fn sweep_capsule(&self, start: FVector, end: FVector, radius: f32, half_height: f32) -> Vec<HitResult> {
        let (c, dl) = (d3(start), d3(sub(end, start)));
        let (r, a) = (radius as f64, (half_height - radius).max(0.0) as f64);
        let mut out = Vec::new();
        for (idx, s) in self.shapes.iter().enumerate() {
            let (cl, sep, bp) = self.clearance(s, c, a, r);
            if cl < -1e-6 && Self::in_prism(s, bp) {
                let mut h = HitResult::new(0.0);
                h.blocking_hit = true;
                h.start_penetrating = true;
                h.location = start;
                h.impact_point = fv(bp);
                h.normal = fv(sep);
                h.impact_normal = fv(sep);
                h.penetration_depth = (-cl) as f32;
                h.trace_start = start;
                h.trace_end = end;
                h.component = Some(idx as u32);
                h.can_step_up = !self.no_step_up.contains(&idx);
                out.push(h);
                continue;
            }
            if let Some((t, nrm)) = self.sweep_shape(idx, s, c, dl, a, r) {
                let h = self.hit_for(idx, s, c, dl, t, nrm, a, r);
                if Self::in_prism(s, d3(h.impact_point)) {
                    out.push(h);
                }
            }
        }
        out.sort_by(|x, y| x.time.partial_cmp(&y.time).unwrap());
        out
    }

    fn line_trace(&self, start: FVector, end: FVector) -> Option<HitResult> {
        let (p, dl) = (d3(start), d3(sub(end, start)));
        let mut best: Option<HitResult> = None;
        for (idx, s) in self.shapes.iter().enumerate() {
            let (t, nrm) = match s {
                Shape::Box { min, max } => match ray_box(p, dl, d3(*min), d3(*max)) {
                    Some((t0, _, ax, sg)) if (0.0..=1.0).contains(&t0) => {
                        let mut nn = [0.0; 3];
                        nn[ax] = sg;
                        (t0, nn)
                    }
                    _ => continue,
                },
                Shape::Plane { n, d, .. } => {
                    let nn = d3(*n);
                    let rate = ddot(nn, dl);
                    if rate >= 0.0 {
                        continue;
                    }
                    let t = (*d as f64 - ddot(nn, p)) / rate;
                    if !(0.0..=1.0).contains(&t) || !Self::in_prism(s, dadd(p, dmul(dl, t))) {
                        continue;
                    }
                    (t, nn)
                }
            };
            if best.map_or(true, |b| (t as f32) < b.time) {
                let mut h = HitResult::new(t as f32);
                h.blocking_hit = true;
                h.distance = (t * ddot(dl, dl).sqrt()) as f32;
                h.location = fv(dadd(p, dmul(dl, t)));
                h.impact_point = h.location;
                h.normal = fv(nrm);
                h.impact_normal = h.normal;
                h.trace_start = start;
                h.trace_end = end;
                h.component = Some(idx as u32);
                h.can_step_up = !self.no_step_up.contains(&idx);
                best = Some(h);
            }
        }
        best
    }

    fn overlap_capsule(&self, centre: FVector, radius: f32, half_height: f32) -> bool {
        let c = d3(centre);
        let (r, a) = (radius as f64, (half_height - radius).max(0.0) as f64);
        self.shapes.iter().any(|s| {
            let (cl, _, bp) = self.clearance(s, c, a, r);
            cl < 0.0 && Self::in_prism(s, bp)
        })
    }
}


/// A `World` whose capsule / line queries are made on one querying channel with one response set (the
/// UpdatedComponent's collision object type and response params: UCharacterMovementComponent's queries take
/// UpdatedPrimitive->GetCollisionObjectType() and InitCollisionParams). The catapult's movement runs through it.
pub struct ChannelWorld<'a> {
    pub inner: &'a dyn World,
    pub channel: &'a str,
    pub responses: &'a [(String, u8)],
}

impl World for ChannelWorld<'_> {
    fn sweep_capsule(&self, start: FVector, end: FVector, radius: f32, half_height: f32) -> Vec<HitResult> {
        self.inner.sweep_capsule_channel(start, end, radius, half_height, self.channel, self.responses)
    }
    fn line_trace(&self, start: FVector, end: FVector) -> Option<HitResult> {
        self.inner.line_trace_channel(start, end, self.channel, self.responses)
    }
    fn overlap_capsule(&self, centre: FVector, radius: f32, half_height: f32) -> bool {
        self.inner.overlap_capsule_channel(centre, radius, half_height, self.channel, self.responses)
    }
    fn sweep_box(&self, start: FVector, end: FVector, half_extent: FVector, quat: [f32; 4], responses: &[(String, u8)]) -> Vec<HitResult> {
        self.inner.sweep_box(start, end, half_extent, quat, responses)
    }
    fn sweep_box_channel(&self, start: FVector, end: FVector, half_extent: FVector, quat: [f32; 4], channel: &str, responses: &[(String, u8)]) -> Vec<HitResult> {
        self.inner.sweep_box_channel(start, end, half_extent, quat, channel, responses)
    }
    fn overlap_box(&self, centre: FVector, half_extent: FVector, quat: [f32; 4], channel: &str, responses: &[(String, u8)]) -> bool {
        self.inner.overlap_box(centre, half_extent, quat, channel, responses)
    }
    fn sweep_capsule_channel(&self, start: FVector, end: FVector, radius: f32, half_height: f32, channel: &str, responses: &[(String, u8)]) -> Vec<HitResult> {
        self.inner.sweep_capsule_channel(start, end, radius, half_height, channel, responses)
    }
    fn line_trace_channel(&self, start: FVector, end: FVector, channel: &str, responses: &[(String, u8)]) -> Option<HitResult> {
        self.inner.line_trace_channel(start, end, channel, responses)
    }
    fn overlap_capsule_channel(&self, centre: FVector, radius: f32, half_height: f32, channel: &str, responses: &[(String, u8)]) -> bool {
        self.inner.overlap_capsule_channel(centre, radius, half_height, channel, responses)
    }
    fn find_teleport_spot(&self, loc: FVector, radius: f32, half_height: f32) -> Option<FVector> {
        self.inner.find_teleport_spot(loc, radius, half_height)
    }
}

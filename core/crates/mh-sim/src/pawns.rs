//! The other characters' capsules in the movement's collision queries (state/proofs/pawn_collision.md,
//! EVD_MOV_020). In the exe every character's CollisionCylinder is a Pawn-profile body: ACharacter::ACharacter
//! (rva=0x2f27200, 0x142f27386: SetCollisionProfileName(UCollisionProfile::Pawn_ProfileName .data 0x1458fa378) on the
//! capsule; 0x142f2739a: CanCharacterStepUpOn (+0x219) = ECB_No). The Pawn profile (DefaultEngine.ini
//! [/Script/Engine.CollisionProfile] Profiles Name="Pawn", EditProfiles Name="Pawn") is object type Pawn and blocks the
//! Pawn channel, so the UCharacterMovementComponent sweeps (UpdatedComponent's object type and responses) stop at the
//! other capsules. AMordhauCharacter::AMordhauCharacter rva=0x1524d90 (decomp 2254-2255) sizes the capsule 50 / 96.
//! Rules kept: a dead character's capsule collides with nothing (AAdvancedCharacter::OnTookDamage rva=0x14926a0, decomp
//! 903-909: a capsule vcall with 0, then SetCollisionResponseToAllChannels(ECR_Ignore)); a rider's capsule is
//! "PawnOnVehicle" (AMordhauCharacter::OnAttachmentChanged rva=0x1550520), which only overlaps Pawn; a ragdoll-falling
//! character keeps its capsule (AAdvancedCharacter::SetIsRagdollFalling rva=0x14a0980 does not touch it). No team rule.
//! The capsules are vertical, so a capsule-vs-capsule sweep is a point sweep against one capsule of radius r1 + r2 and
//! cylinder half length a1 + a2 (the Minkowski sum).
//! Not ported (UNCONFIRMED here): ACharacter::SetBase / Mordhau's UAdvancedCharacterMovement::JumpOff rva=0x14884b0
//! (a character landing on another is pushed off, BounceOffBumpForce on the other), UWorld::FindTeleportSpot vs pawns.

use mh_character::ue::FVector;
use mh_character::world::{HitResult, World};

/// one other character's CollisionCylinder: centre (UpdatedComponent location), radius, current half height
#[derive(Clone, Copy, Debug)]
pub struct PawnCapsule {
    pub id: u32,
    pub centre: FVector,
    pub radius: f32,
    pub half_height: f32,
}

/// host component ids of the pawn capsules (above the static world's body indices)
pub const PAWN_COMPONENT_BASE: u32 = 0x8000_0000;

/// the static world plus the other characters' capsules on the Pawn channel
pub struct PawnWorld<'a> {
    pub inner: &'a dyn World,
    pub pawns: Vec<PawnCapsule>,
}

type D3 = [f64; 3];

fn d3(a: FVector) -> D3 {
    [a.x as f64, a.y as f64, a.z as f64]
}
fn fv(a: D3) -> FVector {
    FVector::new(a[0] as f32, a[1] as f32, a[2] as f32)
}
fn dot(a: D3, b: D3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// separation of point p from the vertical segment (centre c, half length a): the vector from the closest segment point
fn sep(p: D3, c: D3, a: f64) -> D3 {
    let z = (p[2] - c[2]).clamp(-a, a) + c[2];
    [p[0] - c[0], p[1] - c[1], p[2] - z]
}

/// earliest t in [0, 1] the point p + t dl enters the vertical capsule (centre c, cylinder half length a, radius r)
fn ray_capsule(p: D3, dl: D3, c: D3, a: f64, r: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    let mut take = |t: f64| {
        if (0.0..=1.0).contains(&t) && best.map_or(true, |b| t < b) {
            best = Some(t);
        }
    };
    // the cylinder side
    let (mx, my) = (p[0] - c[0], p[1] - c[1]);
    let qa = dl[0] * dl[0] + dl[1] * dl[1];
    if qa > 1e-18 {
        let qb = mx * dl[0] + my * dl[1];
        let disc = qb * qb - qa * (mx * mx + my * my - r * r);
        if disc >= 0.0 {
            let t = (-qb - disc.sqrt()) / qa;
            let z = p[2] + t * dl[2];
            if z >= c[2] - a && z <= c[2] + a {
                take(t);
            }
        }
    }
    // the two cap spheres
    let qa = dot(dl, dl);
    if qa > 1e-18 {
        for zc in [c[2] - a, c[2] + a] {
            let m = [p[0] - c[0], p[1] - c[1], p[2] - zc];
            let qb = dot(m, dl);
            let disc = qb * qb - qa * (dot(m, m) - r * r);
            if disc >= 0.0 {
                take((-qb - disc.sqrt()) / qa);
            }
        }
    }
    best
}

impl PawnWorld<'_> {
    /// blocking hits of the capsule (radius, half height) swept start -> end against the pawn capsules
    fn pawn_hits(&self, start: FVector, end: FVector, radius: f32, half_height: f32) -> Vec<HitResult> {
        let (p, e) = (d3(start), d3(end));
        let dl = [e[0] - p[0], e[1] - p[1], e[2] - p[2]];
        let a1 = (half_height - radius).max(0.0) as f64;
        let mut out = Vec::new();
        for o in &self.pawns {
            let c = d3(o.centre);
            let r = (radius + o.radius) as f64;
            let a = a1 + (o.half_height - o.radius).max(0.0) as f64;
            let s0 = sep(p, c, a);
            let dist = dot(s0, s0).sqrt();
            let mut h = HitResult::new(0.0);
            h.blocking_hit = true;
            h.trace_start = start;
            h.trace_end = end;
            h.component = Some(o.id);
            h.can_step_up = false; // ACharacter ctor 0x142f2739a: CanCharacterStepUpOn = ECB_No
            let rr = radius as f64;
            if dist < r - 1e-6 {
                // initial overlap: depenetration along the separation (MTD)
                let n = if dist > 1e-9 { [s0[0] / dist, s0[1] / dist, s0[2] / dist] } else { [0.0, 0.0, 1.0] };
                h.start_penetrating = true;
                h.location = start;
                h.normal = fv(n);
                h.impact_normal = h.normal;
                h.impact_point = fv([p[0] - n[0] * rr, p[1] - n[1] * rr, p[2] - n[2] * rr]);
                h.penetration_depth = (r - dist) as f32;
                out.push(h);
                continue;
            }
            let Some(t) = ray_capsule(p, dl, c, a, r) else { continue };
            let loc = [p[0] + dl[0] * t, p[1] + dl[1] * t, p[2] + dl[2] * t];
            let s = sep(loc, c, a);
            let l = dot(s, s).sqrt().max(1e-9);
            let n = [s[0] / l, s[1] / l, s[2] / l];
            h.time = t as f32;
            h.distance = (t * dot(dl, dl).sqrt()) as f32;
            h.location = fv(loc);
            h.normal = fv(n);
            h.impact_normal = h.normal;
            h.impact_point = fv([loc[0] - n[0] * rr, loc[1] - n[1] * rr, loc[2] - n[2] * rr]);
            out.push(h);
        }
        out
    }
}

fn by_time(x: &HitResult, y: &HitResult) -> std::cmp::Ordering {
    x.time.partial_cmp(&y.time).unwrap_or(std::cmp::Ordering::Equal)
}

impl World for PawnWorld<'_> {
    fn sweep_capsule(&self, start: FVector, end: FVector, radius: f32, half_height: f32) -> Vec<HitResult> {
        let mut out = self.inner.sweep_capsule(start, end, radius, half_height);
        if !self.pawns.is_empty() {
            out.extend(self.pawn_hits(start, end, radius, half_height));
            out.sort_by(by_time);
        }
        out
    }
    fn line_trace(&self, start: FVector, end: FVector) -> Option<HitResult> {
        let inner = self.inner.line_trace(start, end);
        let pawn = self.pawn_hits(start, end, 0.0, 0.0).into_iter().filter(|h| !h.start_penetrating).min_by(by_time);
        match (inner, pawn) {
            (Some(a), Some(b)) => Some(if b.time < a.time { b } else { a }),
            (a, b) => a.or(b),
        }
    }
    fn overlap_capsule(&self, centre: FVector, radius: f32, half_height: f32) -> bool {
        self.inner.overlap_capsule(centre, radius, half_height) || self.pawn_hits(centre, centre, radius, half_height).iter().any(|h| h.start_penetrating)
    }
    // the other queries (projectile / vehicle channels, teleport) stay the static world's
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

#[cfg(test)]
mod tests {
    use super::*;
    use mh_character::BoxWorld;

    #[test]
    fn capsule_sweep_stops_at_the_summed_radii() {
        let w = BoxWorld::new();
        let pw = PawnWorld { inner: &w, pawns: vec![PawnCapsule { id: PAWN_COMPONENT_BASE, centre: FVector::new(300.0, 0.0, 96.0), radius: 50.0, half_height: 96.0 }] };
        let h = pw.sweep_capsule(FVector::new(0.0, 0.0, 96.0), FVector::new(400.0, 0.0, 96.0), 50.0, 96.0);
        assert_eq!(h.len(), 1);
        assert!((h[0].location.x - 200.0).abs() < 1e-3, "{:?}", h[0].location);
        assert!((h[0].normal.x + 1.0).abs() < 1e-5 && !h[0].can_step_up);
        assert!(pw.overlap_capsule(FVector::new(220.0, 0.0, 96.0), 50.0, 96.0));
        assert!(!pw.overlap_capsule(FVector::new(199.0, 0.0, 96.0), 50.0, 96.0));
    }
}

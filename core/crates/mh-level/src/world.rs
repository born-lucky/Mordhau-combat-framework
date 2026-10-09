//! `mh_character::World` over the reference collision world (feature "world", default on): what mh-character's
//! movement needs to run headless on a real map. Queries use the Pawn channel and the Pawn profile's responses
//! (the character capsule's), CollisionWorld::blocks_pawn.

use crate::collision::{CollisionWorld, Hit, Resp, TOL};
use crate::gjk;
use mh_character::ue::FVector;
use mh_character::world::{HitResult, World};

fn d(a: FVector) -> [f64; 3] {
    [a.x as f64, a.y as f64, a.z as f64]
}
fn f(a: [f64; 3]) -> FVector {
    mh_character::uemath::v(a[0] as f32, a[1] as f32, a[2] as f32)
}

fn hit(w: &CollisionWorld, h: &Hit, start: FVector, end: FVector) -> HitResult {
    let mut r = HitResult::new(h.time as f32);
    let dl = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2) + (end.z - start.z).powi(2)).sqrt();
    r.blocking_hit = true;
    r.start_penetrating = h.start_penetrating;
    r.distance = h.time as f32 * dl;
    r.location = f(h.location);
    r.impact_point = f(h.impact_point);
    r.normal = f(h.normal);
    r.impact_normal = f(h.normal);
    r.trace_start = start;
    r.trace_end = end;
    r.penetration_depth = h.penetration_depth as f32;
    r.component = Some(h.body);
    r.can_step_up = w.bodies.get(h.body as usize).is_none_or(|b| b.can_step_up);
    r
}

impl World for CollisionWorld {
    fn sweep_capsule(&self, start: FVector, end: FVector, radius: f32, half_height: f32) -> Vec<HitResult> {
        self.sweep(d(start), d(end), radius as f64, half_height as f64, &|b| self.blocks_pawn(b))
            .iter()
            .map(|h| hit(self, h, start, end))
            .collect()
    }
    fn line_trace(&self, start: FVector, end: FVector) -> Option<HitResult> {
        self.trace(d(start), d(end), &|b| self.blocks_pawn(b)).map(|h| hit(self, &h, start, end))
    }
    fn overlap_capsule(&self, centre: FVector, radius: f32, half_height: f32) -> bool {
        self.overlap(d(centre), radius as f64, half_height as f64, &|b| self.blocks_pawn(b))
    }
    /// the capsule queries on another querying channel (rust-character r9: the catapult's "Vehicle" capsules),
    /// filtered by `blocks_channel`
    fn sweep_capsule_channel(&self, start: FVector, end: FVector, radius: f32, half_height: f32, channel: &str, responses: &[(String, u8)]) -> Vec<HitResult> {
        self.sweep(d(start), d(end), radius as f64, half_height as f64, &|b| self.blocks_channel(b, channel, responses))
            .iter()
            .map(|h| hit(self, h, start, end))
            .collect()
    }
    fn line_trace_channel(&self, start: FVector, end: FVector, channel: &str, responses: &[(String, u8)]) -> Option<HitResult> {
        self.trace(d(start), d(end), &|b| self.blocks_channel(b, channel, responses)).map(|h| hit(self, &h, start, end))
    }
    fn overlap_capsule_channel(&self, centre: FVector, radius: f32, half_height: f32, channel: &str, responses: &[(String, u8)]) -> bool {
        self.overlap(d(centre), radius as f64, half_height as f64, &|b| self.blocks_channel(b, channel, responses))
    }
    fn find_teleport_spot(&self, loc: FVector, radius: f32, half_height: f32) -> Option<FVector> {
        let mut l = d(loc);
        CollisionWorld::find_teleport_spot(self, &mut l, radius as f64, half_height as f64).then(|| f(l))
    }
    /// The projectile's swept box (rust-character r7, coordinated with mh-world): every blocking hit of the oriented
    /// box (half extent, quat X Y Z W) moved from start to end, earliest first, against the bodies that block the
    /// projectile BoxComp (`blocks_projectile`). Conservative advancement on GJK distance between the box's 8 corners
    /// and each convex shape (rounded by its radius), as `sweep` does for the capsule core. UNCONFIRMED approximation:
    /// the BoxComp has bTraceComplexOnMove (BP_MordhauProjectile BoxComp) and PhysX sweeps it against the per-poly mesh;
    /// here meshes are swept with their simple collision unless the world was built with complex-as-simple triangles.
    fn sweep_box(&self, start: FVector, end: FVector, half_extent: FVector, quat: [f32; 4], responses: &[(String, u8)]) -> Vec<HitResult> {
        self.sweep_box_channel(start, end, half_extent, quat, "Projectile", responses)
    }
    /// `sweep_box` on any object channel (rust-character r8: the catapult's secondary boxes on "Vehicle")
    fn sweep_box_channel(&self, start: FVector, end: FVector, half_extent: FVector, quat: [f32; 4], channel: &str, responses: &[(String, u8)]) -> Vec<HitResult> {
        let (s0, e0) = (d(start), d(end));
        let corners = box_corners(d(half_extent), quat);
        let dvec = [e0[0] - s0[0], e0[1] - s0[1], e0[2] - s0[2]];
        let ext = corners.iter().fold([0.0f64; 3], |m, c| [m[0].max(c[0].abs()), m[1].max(c[1].abs()), m[2].max(c[2].abs())]);
        let mut min = [0.0; 3];
        let mut max = [0.0; 3];
        for k in 0..3 {
            min[k] = s0[k].min(e0[k]) - ext[k] - TOL;
            max[k] = s0[k].max(e0[k]) + ext[k] + TOL;
        }
        let at = |t: f64| -> Vec<[f64; 3]> { corners.iter().map(|c| [s0[0] + c[0] + dvec[0] * t, s0[1] + c[1] + dvec[1] * t, s0[2] + c[2] + dvec[2] * t]).collect() };
        let mut hits = vec![];
        for sh in self.shapes_in(min, max, &|b| self.blocks_channel(b, channel, responses)) {
            let sh = &*sh;
            let rr = sh.radius;
            let a0 = at(0.0);
            let (dist, pa, pb) = gjk::distance(&a0, &sh.pts);
            let mut hit: Option<Hit> = None;
            // the hulls intersect (GJK distance 0) or the rounded shape reaches into the box: start penetrating
            if dist < rr - TOL || dist <= 1e-9 {
                let (depth, n) = if dist > 1e-9 {
                    (rr - dist, gjk::norm(gjk::sub(pa, pb)))
                } else {
                    let (dep, n) = gjk::penetration(&a0, &sh.pts, &[[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
                    (dep + rr, n)
                };
                let ip = if dist > 1e-9 { gjk::add(pb, gjk::mul(n, rr)) } else { pa };
                hit = Some(Hit { time: 0.0, impact_point: ip, normal: n, start_penetrating: true, penetration_depth: depth, ..Default::default() });
            } else if gjk::len(dvec) > 0.0 {
                let mut t = 0.0f64;
                for _ in 0..96 {
                    let (dist, pa, pb) = gjk::distance(&at(t), &sh.pts);
                    let gap = dist - rr;
                    let n = gjk::norm(gjk::sub(pa, pb));
                    if gap <= TOL {
                        hit = Some(Hit { time: t, impact_point: gjk::add(pb, gjk::mul(n, rr)), normal: n, ..Default::default() });
                        break;
                    }
                    let closing = -gjk::dot(dvec, n);
                    if closing <= 1e-12 {
                        break;
                    }
                    t += (gap - 0.5 * TOL) / closing;
                    if t > 1.0 {
                        break;
                    }
                }
            }
            if let Some(mut h) = hit {
                h.body = sh.body;
                h.location = gjk::add(s0, gjk::mul(dvec, h.time));
                hits.push(h);
            }
        }
        hits.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        hits.iter().map(|h| hit(self, h, start, end)).collect()
    }
}

/// The 8 corners of a box (half extent e) rotated by the quaternion q (X, Y, Z, W), relative to its centre
fn box_corners(e: [f64; 3], q: [f32; 4]) -> Vec<[f64; 3]> {
    let (x, y, z, w) = (q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64);
    let rot = |v: [f64; 3]| -> [f64; 3] {
        // v + 2w (q x v) + 2 q x (q x v)
        let t = [2.0 * (y * v[2] - z * v[1]), 2.0 * (z * v[0] - x * v[2]), 2.0 * (x * v[1] - y * v[0])];
        [v[0] + w * t[0] + (y * t[2] - z * t[1]), v[1] + w * t[1] + (z * t[0] - x * t[2]), v[2] + w * t[2] + (x * t[1] - y * t[0])]
    };
    (0..8).map(|i| rot([if i & 1 == 0 { -e[0] } else { e[0] }, if i & 2 == 0 { -e[1] } else { e[1] }, if i & 4 == 0 { -e[2] } else { e[2] }])).collect()
}

impl CollisionWorld {
    /// Whether a body blocks the projectile BoxComp: queries enabled, its response to the Projectile object channel
    /// (DefaultEngine.ini ECC_GameTraceChannel4 "Projectile", default Block) is Block, and the BoxComp's response to
    /// the body's object type (`responses`: mh_character ProjectileCfg::box_responses, the BoxComp's non-Block
    /// channels; unlisted = Block, the channel default for WorldStatic / WorldDynamic; UNCONFIRMED for game channels
    /// whose default is not Block) is Block.
    pub fn blocks_projectile(&self, body: u32, responses: &[(String, u8)]) -> bool {
        self.blocks_channel(body, "Projectile", responses)
    }

    /// `blocks_projectile` for any querying object channel (the body's response to `channel` and the querying shape's
    /// response to the body's object type, `responses` unlisted = Block)
    pub fn blocks_channel(&self, body: u32, channel: &str, responses: &[(String, u8)]) -> bool {
        let Some(b) = self.bodies.get(body as usize) else { return false };
        let mine = responses.iter().find(|(n, _)| *n == b.object_type).map_or(2, |r| r.1);
        b.queries() && b.response(channel) == Resp::Block && mine == 2
    }
}

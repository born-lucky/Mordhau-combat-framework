//! Convex distance queries for the CPU reference collision world (collision.rs). Every shape is a convex point set
//! "rounded" by a radius (sphere = 1 point + r, sphyl = 2 points + r, box = 8 points, convex hull = its points,
//! triangle = 3 points), so one GJK distance routine (Gilbert-Johnson-Keerthi; closest-point subroutines after Ericson,
//! Real-Time Collision Detection, 5.1 and 9.5) answers point / segment / capsule queries against all of them.
//! Not a port: PhysX itself is a closed-source engine dependency of the exe; this stands in for it (as mh-character's
//! BoxWorld does) and is checked by its tests, not by bytes. f64 throughout.

pub type V = [f64; 3];

#[inline]
pub fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
pub fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
pub fn mul(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
pub fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
pub fn len(a: V) -> f64 {
    dot(a, a).sqrt()
}
pub fn norm(a: V) -> V {
    let l = len(a);
    if l > 0.0 {
        mul(a, 1.0 / l)
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// support point of a point set along d (index)
fn support(pts: &[V], d: V) -> usize {
    let mut best = 0;
    let mut bv = f64::NEG_INFINITY;
    for (i, p) in pts.iter().enumerate() {
        let v = dot(*p, d);
        if v > bv {
            bv = v;
            best = i;
        }
    }
    best
}

#[derive(Clone, Copy)]
struct Sv {
    w: V, // a - b
    a: V,
    b: V,
}

/// Closest point of a simplex to the origin; reduces the simplex to the supporting feature and returns barycentrics
fn closest(s: &mut Vec<Sv>) -> (V, Vec<f64>) {
    match s.len() {
        1 => (s[0].w, vec![1.0]),
        2 => {
            let (a, b) = (s[0].w, s[1].w);
            let ab = sub(b, a);
            let t = -dot(a, ab) / dot(ab, ab).max(1e-300);
            if t <= 0.0 {
                s.truncate(1);
                (a, vec![1.0])
            } else if t >= 1.0 {
                s.remove(0);
                (b, vec![1.0])
            } else {
                (add(a, mul(ab, t)), vec![1.0 - t, t])
            }
        }
        3 => {
            let (a, b, c) = (s[0].w, s[1].w, s[2].w);
            let (bc, wts) = tri_closest(a, b, c);
            // keep the vertices with nonzero weight
            let keep: Vec<usize> = (0..3).filter(|&i| wts[i] > 0.0).collect();
            let ns: Vec<Sv> = keep.iter().map(|&i| s[i]).collect();
            let nw: Vec<f64> = keep.iter().map(|&i| wts[i]).collect();
            *s = ns;
            (bc, nw)
        }
        _ => {
            // tetrahedron: origin inside -> distance 0; else closest of the faces
            let p = [0.0, 0.0, 0.0];
            let (a, b, c, d) = (s[0].w, s[1].w, s[2].w, s[3].w);
            let faces = [[0, 1, 2, 3], [0, 2, 3, 1], [0, 3, 1, 2], [1, 3, 2, 0]];
            let mut best: Option<(f64, V, [usize; 3], [f64; 3])> = None;
            let mut inside = true;
            let pts = [a, b, c, d];
            for f in faces {
                let (x, y, z, o) = (pts[f[0]], pts[f[1]], pts[f[2]], pts[f[3]]);
                let n = cross(sub(y, x), sub(z, x));
                let sp = dot(sub(p, x), n);
                let so = dot(sub(o, x), n);
                if sp * so < 0.0 {
                    inside = false;
                    let (q, w) = tri_closest(x, y, z);
                    let dd = dot(q, q);
                    if best.as_ref().is_none_or(|b| dd < b.0) {
                        best = Some((dd, q, [f[0], f[1], f[2]], [w[0], w[1], w[2]]));
                    }
                }
            }
            if inside {
                let w = vec![0.25; 4];
                return ([0.0; 3], w);
            }
            let (_, q, idx, w) = best.unwrap();
            let keep: Vec<usize> = (0..3).filter(|&i| w[i] > 0.0).collect();
            let ns: Vec<Sv> = keep.iter().map(|&i| s[idx[i]]).collect();
            let nw: Vec<f64> = keep.iter().map(|&i| w[i]).collect();
            *s = ns;
            (q, nw)
        }
    }
}

/// Closest point to the origin on triangle abc and its barycentrics (Ericson 5.1.5)
pub fn tri_closest(a: V, b: V, c: V) -> (V, [f64; 3]) {
    let p = [0.0; 3];
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (add(a, mul(ab, v)), [1.0 - v, v, 0.0]);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (add(a, mul(ac, w)), [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (add(b, mul(sub(c, b), w)), [0.0, 1.0 - w, w]);
    }
    let den = 1.0 / (va + vb + vc);
    let v = vb * den;
    let w = vc * den;
    (add(a, add(mul(ab, v), mul(ac, w))), [1.0 - v - w, v, w])
}

/// Distance between the convex hulls of point sets `a` and `b` (radii not included) and the closest points on each.
/// Distance 0 = the hulls intersect (closest points then meaningless).
pub fn distance(a: &[V], b: &[V]) -> (f64, V, V) {
    let mut d = sub(a[0], b[0]);
    let mut s: Vec<Sv> = vec![];
    let ia = support(a, mul(d, -1.0));
    let ib = support(b, d);
    s.push(Sv { w: sub(a[ia], b[ib]), a: a[ia], b: b[ib] });
    let mut last = f64::INFINITY;
    for _ in 0..64 {
        let (v, w) = closest(&mut s);
        let vv = dot(v, v);
        if s.len() == 4 || vv < 1e-18 {
            let (pa, pb) = witness(&s, &w);
            return (0.0, pa, pb);
        }
        d = v;
        let ia = support(a, mul(d, -1.0));
        let ib = support(b, d);
        let wn = sub(a[ia], b[ib]);
        // no progress: v is the closest point of the Minkowski difference
        if vv - dot(v, wn) <= 1e-10 * vv.max(1.0) || vv >= last {
            let (pa, pb) = witness(&s, &w);
            return (vv.sqrt(), pa, pb);
        }
        last = vv;
        if s.iter().any(|x| x.w == wn) {
            let (pa, pb) = witness(&s, &w);
            return (vv.sqrt(), pa, pb);
        }
        s.push(Sv { w: wn, a: a[ia], b: b[ib] });
    }
    let (v, w) = closest(&mut s);
    let (pa, pb) = witness(&s, &w);
    (len(v), pa, pb)
}

fn witness(s: &[Sv], w: &[f64]) -> (V, V) {
    let mut pa = [0.0; 3];
    let mut pb = [0.0; 3];
    for (x, &k) in s.iter().zip(w) {
        pa = add(pa, mul(x.a, k));
        pb = add(pb, mul(x.b, k));
    }
    (pa, pb)
}

/// Penetration of two overlapping convex sets (cores intersect): the minimum translation of `a` along unit n that
/// separates them, searched over the face/edge-ish directions given plus a sphere of sample directions. Returns
/// (depth, n) with n pointing from b towards a. An approximation (EPA would be exact) used only for start-penetrating
/// results.
pub fn penetration(a: &[V], b: &[V], extra_dirs: &[V]) -> (f64, V) {
    let h = |pts: &[V], n: V| pts.iter().map(|p| dot(*p, n)).fold(f64::NEG_INFINITY, f64::max);
    let mut best = (f64::INFINITY, [0.0, 0.0, 1.0]);
    let mut try_dir = |n: V| {
        let n = norm(n);
        // translate a by depth*n until min over a along n >= max over b along n
        let depth = h(b, n) + h(a, mul(n, -1.0));
        if depth < best.0 {
            best = (depth, n);
        }
    };
    for &d in extra_dirs {
        try_dir(d);
        try_dir(mul(d, -1.0));
    }
    // a Fibonacci sphere of 256 directions
    let n = 256;
    for i in 0..n {
        let z = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
        let r = (1.0 - z * z).sqrt();
        let phi = i as f64 * 2.399963229728653;
        try_dir([r * phi.cos(), r * phi.sin(), z]);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segment_to_box_distance() {
        let bx: Vec<V> = (0..8).map(|i| [(i & 1) as f64 * 2.0 - 1.0, ((i >> 1) & 1) as f64 * 2.0 - 1.0, ((i >> 2) & 1) as f64 * 2.0 - 1.0]).collect();
        let seg = [[3.0, 0.0, -5.0], [3.0, 0.0, 5.0]];
        let (d, pa, pb) = distance(&seg, &bx);
        assert!((d - 2.0).abs() < 1e-9, "{d}");
        assert!((pa[0] - 3.0).abs() < 1e-9 && (pb[0] - 1.0).abs() < 1e-9);
        let (d, _, _) = distance(&[[0.5, 0.5, 0.5]], &bx);
        assert_eq!(d, 0.0);
        let (dep, n) = penetration(&[[0.8, 0.0, 0.0]], &bx, &[[1.0, 0.0, 0.0]]);
        assert!((dep - 0.2).abs() < 1e-9 && n[0] > 0.99, "{dep} {n:?}");
    }
    #[test]
    fn point_to_triangle() {
        let t = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
        let (d, _, pb) = distance(&[[2.0, 2.0, 5.0]], &t);
        assert!((d - 5.0).abs() < 1e-9 && pb[2].abs() < 1e-9);
    }
}

//! USplineComponent evaluation for the spline pushables (BP_SplinePushableActor:GetTransformAlongSplineOffset): the
//! component's FSplineCurves (Position / Rotation / ReparamTable, tagged properties of the placed component) and its
//! world transform. UE 4.26 Engine/Source/Runtime/Engine/Private/Components/SplineComponent.cpp semantics:
//! GetLocationAtDistanceAlongSpline = Position.Eval(ReparamTable.Eval(distance)); GetSplineLength =
//! ReparamTable.Points.Last().InVal; GetTransformAtTime(t, World, bUseConstantVelocity) = the transform at distance
//! t / Duration * length; rotation = FRotationMatrix::MakeFromXZ(Position derivative, Rotation quat * DefaultUpVector)
//! (GetQuaternionAtSplineInputKey); InterpCurve segments: linear / constant / FMath::CubicInterp with the leave /
//! arrive tangents scaled by the key spacing (FInterpCurve::Eval).

use crate::V;
use mh_level::xf::Xf;
use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct Point {
    pub in_val: f64,
    pub out: V,
    pub arrive: V,
    pub leave: V,
    /// CIM_Linear (0, the zero value) / CIM_Constant / CIM_Curve*
    pub mode: String,
}

#[derive(Clone, Debug, Default)]
pub struct Spline {
    pub position: Vec<Point>,
    /// Rotation keys (quaternions x, y, z, w)
    pub rotation: Vec<(f64, [f64; 4])>,
    /// ReparamTable: (distance, input key), linear
    pub reparam: Vec<(f64, f64)>,
    pub duration: f64,
    pub up: V,
    /// component -> world
    pub xf: Xf,
}

fn n(v: Option<&Value>, k: &str, d: f64) -> f64 {
    v.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).unwrap_or(d)
}
fn v3(v: Option<&Value>) -> V {
    [n(v, "X", 0.0), n(v, "Y", 0.0), n(v, "Z", 0.0)]
}
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// FVector::GetSafeNormal(tol): zero below tol (squared size)
pub fn normal(v: V, tol: f64) -> V {
    let s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if s == 1.0 {
        v
    } else if s < tol {
        [0.0; 3]
    } else {
        let k = 1.0 / s.sqrt();
        v.map(|c| c * k)
    }
}

/// UKismetMathLibrary::MakeRotFromXZ -> FRotationMatrix::MakeFromXZ(X, Z): X normal, Y = (Z x X).normal, Z' = X x Y;
/// as a transform at `loc`
pub fn make_rot_from_xz(x: V, z: V, loc: V) -> Xf {
    let nx = normal(x, 1e-8);
    let mut nz = normal(z, 1e-8);
    // parallel: UE picks another up (|Z . X| ~ 1); keep world Z then Y [UNCONFIRMED corner case]
    if (nx[0] * nz[0] + nx[1] * nz[1] + nx[2] * nz[2]).abs() > 1.0 - 1e-4 {
        nz = if nx[2].abs() < 1.0 - 1e-4 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
    }
    let ny = normal(cross(nz, nx), 1e-8);
    let nz = cross(nx, ny);
    let mut m = [[0.0; 4]; 3];
    for i in 0..3 {
        m[i][0] = nx[i];
        m[i][1] = ny[i];
        m[i][2] = nz[i];
        m[i][3] = loc[i];
    }
    Xf { m }
}

fn quat_rotate(q: [f64; 4], v: V) -> V {
    let u = [q[0], q[1], q[2]];
    let t = cross(u, v).map(|c| c * 2.0);
    let c2 = cross(u, t);
    [v[0] + q[3] * t[0] + c2[0], v[1] + q[3] * t[1] + c2[1], v[2] + q[3] * t[2] + c2[2]]
}

impl Spline {
    /// From a placed USplineComponent export's properties (Template-merged) and its world transform
    pub fn from_props(p: &serde_json::Map<String, Value>, xf: Xf) -> Spline {
        let curves = p.get("SplineCurves");
        let mut s = Spline { duration: p.get("Duration").and_then(|v| v.as_f64()).unwrap_or(1.0), up: [0.0, 0.0, 1.0], xf, ..Default::default() };
        if let Some(u) = p.get("DefaultUpVector") {
            s.up = v3(Some(u));
        }
        let pts = |k: &str| curves.and_then(|c| c.get(k)).and_then(|c| c.get("Points")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for q in pts("Position") {
            s.position.push(Point {
                in_val: n(Some(&q), "InVal", 0.0),
                out: v3(q.get("OutVal")),
                arrive: v3(q.get("ArriveTangent")),
                leave: v3(q.get("LeaveTangent")),
                mode: q.get("InterpMode").and_then(|v| v.as_str()).unwrap_or("CIM_Linear").to_string(),
            });
        }
        for q in pts("Rotation") {
            let o = q.get("OutVal");
            s.rotation.push((n(Some(&q), "InVal", 0.0), [n(o, "X", 0.0), n(o, "Y", 0.0), n(o, "Z", 0.0), n(o, "W", 1.0)]));
        }
        for q in pts("ReparamTable") {
            s.reparam.push((n(Some(&q), "InVal", 0.0), n(Some(&q), "OutVal", 0.0)));
        }
        // a closed loop evaluates one more segment back to the first point (FInterpCurve bIsLooped, LoopKeyOffset)
        let looped = curves.and_then(|c| c.get("Position")).and_then(|c| c.get("bIsLooped")).and_then(|v| v.as_bool()).unwrap_or(false);
        if looped && !s.position.is_empty() {
            let off = n(curves.and_then(|c| c.get("Position")), "LoopKeyOffset", 1.0);
            let mut f = s.position[0].clone();
            f.in_val = s.position.last().unwrap().in_val + off;
            s.position.push(f);
        }
        if s.reparam.is_empty() {
            s.build_reparam(10);
        }
        s
    }

    /// FInterpCurve::Eval of Position (and EvalDerivative): segment search, per-mode interpolation, clamped ends
    fn eval_pos(&self, key: f64) -> (V, V) {
        let p = &self.position;
        if p.is_empty() {
            return ([0.0; 3], [0.0; 3]);
        }
        if p.len() == 1 || key <= p[0].in_val {
            return (p[0].out, if p.len() > 1 { p[0].leave } else { [0.0; 3] });
        }
        let last = p.len() - 1;
        if key >= p[last].in_val {
            return (p[last].out, p[last].arrive);
        }
        let i = p.iter().position(|x| x.in_val > key).unwrap() - 1;
        let (a, b) = (&p[i], &p[i + 1]);
        let d = b.in_val - a.in_val;
        if d <= 0.0 {
            return (a.out, [0.0; 3]);
        }
        let t = (key - a.in_val) / d;
        match a.mode.as_str() {
            "CIM_Constant" => (a.out, [0.0; 3]),
            "CIM_Linear" => {
                let dv = sub(b.out, a.out);
                ([a.out[0] + dv[0] * t, a.out[1] + dv[1] * t, a.out[2] + dv[2] * t], dv.map(|c| c / d))
            }
            _ => {
                let mut pos = [0.0; 3];
                let mut der = [0.0; 3];
                for k in 0..3 {
                    let (p0, t0, p1, t1) = (a.out[k], a.leave[k] * d, b.out[k], b.arrive[k] * d);
                    let (t2, t3) = (t * t, t * t * t);
                    // FMath::CubicInterp
                    pos[k] = (2.0 * t3 - 3.0 * t2 + 1.0) * p0 + (t3 - 2.0 * t2 + t) * t0 + (t3 - t2) * t1 + (-2.0 * t3 + 3.0 * t2) * p1;
                    // FMath::CubicInterpDerivative / Diff
                    let ca = 6.0 * p0 + 3.0 * t0 + 3.0 * t1 - 6.0 * p1;
                    let cb = -6.0 * p0 - 4.0 * t0 - 2.0 * t1 + 6.0 * p1;
                    der[k] = ((ca * t + cb) * t + t0) / d;
                }
                (pos, der)
            }
        }
    }

    /// Rotation curve at a key: the bracketing keys slerped [UNCONFIRMED for CIM_Curve rotation keys, which UE
    /// evaluates with squad tangents; identical for the all-identity rotation curves]
    fn eval_rot(&self, key: f64) -> [f64; 4] {
        let r = &self.rotation;
        if r.is_empty() {
            return [0.0, 0.0, 0.0, 1.0];
        }
        if key <= r[0].0 {
            return r[0].1;
        }
        if key >= r.last().unwrap().0 {
            return r.last().unwrap().1;
        }
        let i = r.iter().position(|x| x.0 > key).unwrap() - 1;
        let (a, b) = (r[i], r[i + 1]);
        let t = (key - a.0) / (b.0 - a.0).max(1e-12);
        let mut q1 = b.1;
        let mut dot: f64 = (0..4).map(|k| a.1[k] * q1[k]).sum();
        if dot < 0.0 {
            q1 = q1.map(|c| -c);
            dot = -dot;
        }
        let (w0, w1) = if dot > 0.9995 {
            (1.0 - t, t)
        } else {
            let th = dot.acos();
            let s = th.sin();
            (((1.0 - t) * th).sin() / s, (t * th).sin() / s)
        };
        let q = [0, 1, 2, 3].map(|k| a.1[k] * w0 + q1[k] * w1);
        let l = q.iter().map(|c| c * c).sum::<f64>().sqrt().max(1e-12);
        q.map(|c| c / l)
    }

    /// USplineComponent::UpdateSpline's reparam table (ReparamStepsPerSegment samples per segment, distances by
    /// Gauss-Legendre segment length) when the component carries none [UNCONFIRMED: the cooked components carry it]
    fn build_reparam(&mut self, steps: usize) {
        let segs = self.position.len().saturating_sub(1);
        let mut dist = 0.0;
        self.reparam.push((0.0, self.position.first().map(|p| p.in_val).unwrap_or(0.0)));
        for sgi in 0..segs {
            let (k0, k1) = (self.position[sgi].in_val, self.position[sgi + 1].in_val);
            for st in 1..=steps {
                let (a, b) = (k0 + (k1 - k0) * (st - 1) as f64 / steps as f64, k0 + (k1 - k0) * st as f64 / steps as f64);
                // 5-point Gauss-Legendre over |dP/dkey|
                const X: [f64; 5] = [0.0, -0.538_469_310_105_683_1, 0.538_469_310_105_683_1, -0.906_179_845_938_664, 0.906_179_845_938_664];
                const W: [f64; 5] = [0.568_888_888_888_888_9, 0.478_628_670_499_366_5, 0.478_628_670_499_366_5, 0.236_926_885_056_189_1, 0.236_926_885_056_189_1];
                let mut l = 0.0;
                for (x, w) in X.iter().zip(W) {
                    let k = (a + b) / 2.0 + (b - a) / 2.0 * x;
                    let d = self.eval_pos(k).1;
                    l += w * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                }
                dist += l * (b - a) / 2.0;
                self.reparam.push((dist, b));
            }
        }
    }

    /// GetSplineLength
    pub fn length(&self) -> f64 {
        self.reparam.last().map(|r| r.0).unwrap_or(0.0)
    }

    /// GetInputKeyAtDistanceAlongSpline: ReparamTable.Eval (linear, clamped)
    pub fn key_at_distance(&self, d: f64) -> f64 {
        let r = &self.reparam;
        if r.is_empty() {
            return 0.0;
        }
        if d <= r[0].0 {
            return r[0].1;
        }
        if d >= r.last().unwrap().0 {
            return r.last().unwrap().1;
        }
        let i = r.iter().position(|x| x.0 > d).unwrap() - 1;
        let (a, b) = (r[i], r[i + 1]);
        a.1 + (b.1 - a.1) * (d - a.0) / (b.0 - a.0).max(1e-12)
    }

    /// GetTransformAtSplineInputKey(key, World, bUseScale false)
    pub fn transform_at_key(&self, key: f64) -> Xf {
        let (p, dp) = self.eval_pos(key);
        let up = quat_rotate(self.eval_rot(key), self.up);
        self.xf * make_rot_from_xz(dp, up, p)
    }

    /// GetLocationAtDistanceAlongSpline(d, World)
    pub fn location_at_distance(&self, d: f64) -> V {
        self.xf.apply(self.eval_pos(self.key_at_distance(d)).0)
    }

    /// GetRotationAtDistanceAlongSpline(d, World) -> GetUpVector: the world up of the spline frame there
    pub fn up_at_distance(&self, d: f64) -> V {
        let x = self.transform_at_key(self.key_at_distance(d));
        normal([x.m[0][2], x.m[1][2], x.m[2][2]], 1e-8)
    }

    /// GetTransformAtTime(t, World, bUseConstantVelocity true, bUseScale false)
    pub fn transform_at_time(&self, t: f64) -> Xf {
        let d = if self.duration != 0.0 { t / self.duration * self.length() } else { 0.0 };
        self.transform_at_key(self.key_at_distance(d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// a straight two-point curve spline along X: length 1000, location / facing / up at distances
    #[test]
    fn straight_spline() {
        let p = json!({"SplineCurves": {"Position": {"Points": [
            {"InVal": 0.0, "OutVal": {"X": 0.0, "Y": 0.0, "Z": 0.0}, "ArriveTangent": {"X": 1000.0, "Y": 0.0, "Z": 0.0}, "LeaveTangent": {"X": 1000.0, "Y": 0.0, "Z": 0.0}, "InterpMode": "CIM_CurveAuto"},
            {"InVal": 1.0, "OutVal": {"X": 1000.0, "Y": 0.0, "Z": 0.0}, "ArriveTangent": {"X": 1000.0, "Y": 0.0, "Z": 0.0}, "LeaveTangent": {"X": 1000.0, "Y": 0.0, "Z": 0.0}, "InterpMode": "CIM_CurveAuto"}
        ]}}});
        let s = Spline::from_props(p.as_object().unwrap(), mh_level::xf::IDENTITY);
        assert!((s.length() - 1000.0).abs() < 1e-6, "{}", s.length());
        let l = s.location_at_distance(250.0);
        assert!((l[0] - 250.0).abs() < 1e-6 && l[1].abs() < 1e-9);
        let x = s.transform_at_time(0.5);
        assert!((x.translation()[0] - 500.0).abs() < 1e-6);
        assert!((x.m[0][0] - 1.0).abs() < 1e-9 && (x.m[2][2] - 1.0).abs() < 1e-9);
        assert_eq!(s.up_at_distance(10.0), [0.0, 0.0, 1.0]);
    }
}

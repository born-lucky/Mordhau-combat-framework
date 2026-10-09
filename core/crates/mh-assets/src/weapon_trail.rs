//! The weapon-material smear of a swing: what `AMordhauWeapon::UpdateTrail_Implementation` 0x1641a50 writes into the
//! weapon's material instances every update, and what M_WeaponMaster's vertex shader does with it
//! (M_WeaponMaster/r0/112 TBasePassVSFNoLightMapPolicy__FLocalVertexFactory, the World Position Offset).
//!
//! UpdateTrail (decomp AMordhauWeapon.cpp 782-1210):
//! - TrailWeight param = TrailFactor x Weight (Weight from UAttackMotion: AMordhauWeapon::UpdateTrail(TrailWeight),
//!   0 when the attack leaves), 0 without a parent character or for a stab (LastObservedMove 2 / 3);
//! - MeshUp = TrailUp, MeshRight = TrailRight (mesh space; ctor 0x1611170: TrailUp (0, 0, 1), TrailRight (0, 1, 0),
//!   per weapon CDO; the alternate mode's Second* values UNCONFIRMED: RecalculateTracerPoints 0x163a940 swaps the
//!   factors, the up / right swap is assumed alike);
//! - TrailMinRight = 0; TrailMinUp = 15 x dot(mesh-space unit trace direction (TraceEnd - TraceStart, inverse
//!   rotated, / scale), TrailUp);
//! - TrailMotion = (LastTrailTransform(tip) - ComponentToWorld(tip)) x min(1, min(dt, 0.01) / max(dt, 0.001)), tip
//!   = TraceEnd in mesh space; LastTrailTransform = this update's transform afterwards, and reset to it when the
//!   previous Weight was 0.
//! - TrailFactor = DefaultTrailFactor (alternate mode SecondDefaultTrailFactor), lowered by parts' factors
//!   (RecalculateTracerPoints 0x163a940).
//! Vertex shader: r = max(dot(P, MeshRight) - TrailMinRight, 0), u = max(dot(P, MeshUp) - TrailMinUp, 0), P the
//! mesh-space position; WPO = r u TrailMotion TrailWeight when r u TrailWeight >= 1e-4.

/// UE vectors (cm, UE axes)
pub type V = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailParams {
    pub mesh_up: V,
    pub mesh_right: V,
    pub trail_min_up: f32,
    pub trail_min_right: f32,
    pub trail_motion: V,
    pub trail_weight: f32,
}

/// a component transform: rotation quaternion (x, y, z, w), translation, scale (UE)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub rot: [f32; 4],
    pub pos: V,
    pub scale: V,
}

fn qrot(q: [f32; 4], v: V) -> V {
    let u = [q[0], q[1], q[2]];
    let w = q[3];
    let c = |a: V, b: V| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = c(u, v).map(|x| 2.0 * x);
    let ut = c(u, t);
    [v[0] + w * t[0] + ut[0], v[1] + w * t[1] + ut[1], v[2] + w * t[2] + ut[2]]
}

impl Xf {
    pub fn transform(&self, p: V) -> V {
        let s = [p[0] * self.scale[0], p[1] * self.scale[1], p[2] * self.scale[2]];
        let r = qrot(self.rot, s);
        [r[0] + self.pos[0], r[1] + self.pos[1], r[2] + self.pos[2]]
    }
    /// inverse rotation and scale of a vector (FTransform::InverseTransformVector: a zero scale component gives 0)
    pub fn inverse_vector(&self, v: V) -> V {
        let r = qrot([-self.rot[0], -self.rot[1], -self.rot[2], self.rot[3]], v);
        let inv = |s: f32| if s.abs() <= 1e-8 { 0.0 } else { 1.0 / s };
        [r[0] * inv(self.scale[0]), r[1] * inv(self.scale[1]), r[2] * inv(self.scale[2])]
    }
    pub fn inverse_position(&self, p: V) -> V {
        self.inverse_vector([p[0] - self.pos[0], p[1] - self.pos[1], p[2] - self.pos[2]])
    }
}

/// per-weapon state: LastTrailTransform and the last Weight
#[derive(Clone, Copy, Debug, Default)]
pub struct WeaponTrailState {
    pub last: Option<Xf>,
    pub weight: f32,
}

/// one update: `weight` = the attack's TrailWeight, `factor` = TrailFactor, `stab` = LastObservedMove Stab / AltStab
#[allow(clippy::too_many_arguments)]
pub fn update(st: &mut WeaponTrailState, weight: f32, factor: f32, stab: bool, comp: Xf, trace_start: V, trace_end: V, trail_up: V, trail_right: V, dt: f32) -> TrailParams {
    let prev = st.weight;
    st.weight = weight;
    if prev.abs() <= 1e-8 || st.last.is_none() {
        st.last = Some(comp);
    }
    let last = st.last.unwrap_or(comp);
    let tip = comp.inverse_position(trace_end);
    let d = [trace_end[0] - trace_start[0], trace_end[1] - trace_start[1], trace_end[2] - trace_start[2]];
    let l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let dn = if l2 >= 1e-8 { d.map(|x| x / l2.sqrt()) } else { [0.0; 3] };
    let dl = comp.inverse_vector(dn);
    let min_up = 15.0 * (dl[0] * trail_up[0] + dl[1] * trail_up[1] + dl[2] * trail_up[2]);
    let a = last.transform(tip);
    let b = comp.transform(tip);
    let dtc = dt.max(0.001);
    let k = (dtc.min(0.01) / dtc).min(1.0);
    st.last = Some(comp);
    TrailParams {
        mesh_up: trail_up,
        mesh_right: trail_right,
        trail_min_up: min_up,
        trail_min_right: 0.0,
        trail_motion: [(a[0] - b[0]) * k, (a[1] - b[1]) * k, (a[2] - b[2]) * k],
        trail_weight: if stab { 0.0 } else { factor * weight },
    }
}

/// the vertex shader's offset (UE world cm) of a mesh-space position
pub fn offset(p: &TrailParams, local: V) -> V {
    let d = |a: V, b: V| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let r = (d(local, p.mesh_right) - p.trail_min_right).max(0.0);
    let u = (d(local, p.mesh_up) - p.trail_min_up).max(0.0);
    let w = r * u;
    if w * p.trail_weight >= 1e-4 { p.trail_motion.map(|m| m * w * p.trail_weight) } else { [0.0; 3] }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a blade along mesh +Z swung sideways: vertices near the tip and right of the trace line smear back along
    /// the swing; nothing moves before the attack weight rises or for a stab
    #[test]
    fn smear_follows_the_swing() {
        let id = |x: f32| Xf { rot: [0.0, 0.0, 0.0, 1.0], pos: [x, 0.0, 0.0], scale: [1.0; 3] };
        let mut st = WeaponTrailState::default();
        let up = [0.0, 0.0, 1.0];
        let right = [0.0, 1.0, 0.0];
        let p0 = update(&mut st, 0.0, 0.01, false, id(0.0), [0.0, 0.0, 10.0], [0.0, 0.0, 100.0], up, right, 1.0 / 60.0);
        assert_eq!(p0.trail_weight, 0.0);
        let p1 = update(&mut st, 1.0, 0.01, false, id(5.0), [5.0, 0.0, 10.0], [5.0, 0.0, 100.0], up, right, 1.0 / 60.0);
        // the first weighted update starts from this transform: no motion yet
        assert_eq!(p1.trail_motion, [0.0; 3]);
        assert!((p1.trail_min_up - 15.0).abs() < 1e-5);
        let p2 = update(&mut st, 1.0, 0.01, false, id(10.0), [10.0, 0.0, 10.0], [10.0, 0.0, 100.0], up, right, 1.0 / 60.0);
        // tip moved +5 in X: motion -5 x min(1, 0.01 / (1/60)) = -3
        assert!((p2.trail_motion[0] + 3.0).abs() < 1e-4, "{:?}", p2.trail_motion);
        let o = offset(&p2, [0.0, 2.0, 80.0]);
        assert!((o[0] - (-3.0 * 2.0 * 65.0 * 0.01)).abs() < 1e-3, "{o:?}");
        assert_eq!(offset(&p2, [0.0, -2.0, 80.0]), [0.0; 3]);
        let ps = update(&mut st, 1.0, 0.01, true, id(15.0), [15.0, 0.0, 10.0], [15.0, 0.0, 100.0], up, right, 1.0 / 60.0);
        assert_eq!(offset(&ps, [0.0, 2.0, 80.0]), [0.0; 3]);
    }
}

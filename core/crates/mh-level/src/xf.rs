//! UE-space transforms (cm, Z up, left-handed), as affine 3x4 matrices in f64, and the conversion to the glTF / Godot /
//! Bevy space the exported meshes live in. Port of the transform half of `ue_level.gd` (header, rot_quat, xf,
//! rel_xf, ftransform).
//!
//! UE: a placement is world = T + R(s * v) (FTransform.TransformPosition, CUE4Parse FTransform.cs:389), i.e. the affine
//! T * R * S; a component's world transform is the AttachParent chain product (USceneComponent).
//!
//! glTF space (CUE4Parse's glTF writer, Gltf.cs:25 UnitScale 0.01, 72/230 positions, 244-248 SwapYZ, 250 quaternion
//! (x, y, z, w) -> (x, z, y, -w)): g = SwapYZ(v * 0.01). With P = the Y/Z swap, the placement in glTF space is the
//! conjugate P M P on the linear part and 0.01 P T on the translation (`Xf::to_gltf`). ue_level.gd builds the same
//! matrix as Basis(SwapYZ(q)) * scale(sx, sz, sy) (SwapYZ R SwapYZ is the rotation with quaternion SwapYZ(q)), and
//! conjugation commutes with the chain product, so converting the composed UE matrix equals composing converted ones.

use serde_json::Value;

/// Affine transform, row-major 3x4: p' = m[r][0..3] . p + m[r][3]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub m: [[f64; 4]; 3],
}

pub const IDENTITY: Xf = Xf { m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]] };

impl Default for Xf {
    fn default() -> Self {
        IDENTITY
    }
}

impl std::ops::Mul for Xf {
    type Output = Xf;
    /// self after rhs: (self * rhs)(p) = self(rhs(p))
    fn mul(self, b: Xf) -> Xf {
        let a = &self.m;
        let mut m = [[0.0; 4]; 3];
        for r in 0..3 {
            for c in 0..4 {
                m[r][c] = a[r][0] * b.m[0][c] + a[r][1] * b.m[1][c] + a[r][2] * b.m[2][c] + if c == 3 { a[r][3] } else { 0.0 };
            }
        }
        Xf { m }
    }
}

impl Xf {
    /// T * R(q) * S, q a UE quaternion (x, y, z, w), normalized here (ue_level.gd quat() normalizes too)
    pub fn trs(t: [f64; 3], q: [f64; 4], s: [f64; 3]) -> Xf {
        let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        let [x, y, z, w] = if n > 0.0 { [q[0] / n, q[1] / n, q[2] / n, q[3] / n] } else { [0.0, 0.0, 0.0, 1.0] };
        let r = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
            [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
            [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
        ];
        let mut m = [[0.0; 4]; 3];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = r[i][j] * s[j];
            }
            m[i][3] = t[i];
        }
        Xf { m }
    }
    pub fn translation(&self) -> [f64; 3] {
        [self.m[0][3], self.m[1][3], self.m[2][3]]
    }
    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let m = &self.m;
        let f = |r: usize| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3];
        [f(0), f(1), f(2)]
    }
    /// The same placement in glTF / Godot / Bevy space (Y up, metres): linear part P M P, translation 0.01 P T
    pub fn to_gltf(&self) -> Xf {
        const P: [usize; 3] = [0, 2, 1];
        let mut m = [[0.0; 4]; 3];
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] = self.m[P[r]][P[c]];
            }
            m[r][3] = self.m[P[r]][3] * 0.01;
        }
        Xf { m }
    }
    /// Column-major 4x4 (glam `Mat4::from_cols_array` / `Affine3A::from_mat4` order)
    pub fn cols_array(&self) -> [f32; 16] {
        let m = &self.m;
        let mut o = [0f32; 16];
        for c in 0..4 {
            for r in 0..3 {
                o[c * 4 + r] = m[r][c] as f32;
            }
        }
        o[15] = 1.0;
        o
    }
    /// Inverse affine (None when singular)
    pub fn inverse(&self) -> Option<Xf> {
        let m = &self.m;
        let a = [[m[0][0], m[0][1], m[0][2]], [m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]]];
        let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
        if det.abs() < 1e-300 {
            return None;
        }
        let id = 1.0 / det;
        let inv = [
            [(a[1][1] * a[2][2] - a[1][2] * a[2][1]) * id, (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * id, (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * id],
            [(a[1][2] * a[2][0] - a[1][0] * a[2][2]) * id, (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * id, (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * id],
            [(a[1][0] * a[2][1] - a[1][1] * a[2][0]) * id, (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * id, (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * id],
        ];
        let t = [m[0][3], m[1][3], m[2][3]];
        let mut o = [[0.0; 4]; 3];
        for r in 0..3 {
            o[r][..3].copy_from_slice(&inv[r]);
            o[r][3] = -(inv[r][0] * t[0] + inv[r][1] * t[1] + inv[r][2] * t[2]);
        }
        Some(Xf { m: o })
    }
    /// Largest absolute element difference (tests)
    pub fn max_diff(&self, o: &Xf) -> f64 {
        let mut d: f64 = 0.0;
        for r in 0..3 {
            for c in 0..4 {
                d = d.max((self.m[r][c] - o.m[r][c]).abs());
            }
        }
        d
    }
}

fn num(v: Option<&Value>, k: &str, d: f64) -> f64 {
    v.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).unwrap_or(d)
}

pub fn vec3(v: Option<&Value>, d: f64) -> [f64; 3] {
    [num(v, "X", d), num(v, "Y", d), num(v, "Z", d)]
}

/// CUE4Parse FRotator.Quaternion (FRotator.cs:88-111): degrees in, UE quaternion (x, y, z, w) out
pub fn rot_quat(r: Option<&Value>) -> [f64; 4] {
    let h = std::f64::consts::PI / 360.0;
    let (p, y, o) = (num(r, "Pitch", 0.0) * h, num(r, "Yaw", 0.0) * h, num(r, "Roll", 0.0) * h);
    let (sp, cp, sy, cy, sr, cr) = (p.sin(), p.cos(), y.sin(), y.cos(), o.sin(), o.cos());
    [cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]
}

/// Relative transform of a SceneComponent. Absent properties are the USceneComponent defaults: zero location and
/// rotation, unit scale (only serialized, i.e. non-default, tagged properties are in the data)
pub fn rel_xf(p: &serde_json::Map<String, Value>) -> Xf {
    Xf::trs(vec3(p.get("RelativeLocation"), 0.0), rot_quat(p.get("RelativeRotation")), vec3(p.get("RelativeScale3D"), 1.0))
}

/// FTransform {Rotation{X,Y,Z,W}, Translation, Scale3D} (foliage TransformData, LevelTransform)
pub fn ftransform(t: &Value) -> Xf {
    let r = t.get("Rotation");
    Xf::trs(
        vec3(t.get("Translation"), 0.0),
        [num(r, "X", 0.0), num(r, "Y", 0.0), num(r, "Z", 0.0), num(r, "W", 1.0)],
        vec3(t.get("Scale3D"), 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn yaw_90_turns_x_to_y() {
        let x = Xf::trs([0.0; 3], rot_quat(Some(&json!({"Yaw": 90.0}))), [1.0; 3]);
        let p = x.apply([1.0, 0.0, 0.0]);
        assert!((p[0]).abs() < 1e-12 && (p[1] - 1.0).abs() < 1e-12, "{p:?}");
    }
    #[test]
    fn gltf_conversion_commutes_with_products() {
        let a = Xf::trs([10.0, 20.0, 30.0], rot_quat(Some(&json!({"Pitch": 10.0, "Yaw": 30.0, "Roll": 5.0}))), [1.0, 2.0, 3.0]);
        let b = Xf::trs([-5.0, 1.0, 7.0], rot_quat(Some(&json!({"Yaw": -70.0}))), [0.5, 0.5, 2.0]);
        assert!((a * b).to_gltf().max_diff(&(a.to_gltf() * b.to_gltf())) < 1e-12);
        // translation in metres, Y/Z swapped
        assert_eq!(a.to_gltf().translation(), [0.1, 0.3, 0.2]);
    }
}

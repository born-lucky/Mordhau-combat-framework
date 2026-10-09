//! Vector math for the character rules.
//!
//! `FVector`, `clampf`, `maxf`, `minf` are mordhau-core::ue's (owner rust-combat). `V3Ext` and the free functions
//! below are the Godot Vector3 / Vector2 / Basis operations the character reference calls that mordhau-core::ue lacks
//! (candidates to move there; listed in the rust-character r1 report).
//!
//! Float model (same as mordhau-core::ue): the GDScript reference computes scalars in f64 (GDScript `float`) and vectors
//! in Godot Vector3 / Vector2, whose components are f32 (real_t). A scalar multiplying a vector is converted to f32
//! first (`FVector::scale`); a component read in GDScript (`v.x`) is the f32 widened to f64 (`xf()`...).

pub use mordhau_core::ue::{clampf, maxf, minf, FVector};

// ---- additions the character rules need (candidates for mordhau-core::ue) ------------------------------------------

/// Godot CMP_EPSILON (core/math/math_defs.h), used by is_zero_approx
pub const CMP_EPSILON: f32 = 0.00001;

/// Godot Vector3 operations the character reference calls that mordhau-core::ue::FVector lacks.
pub trait V3Ext {
    /// components as GDScript reads them: the f32 widened to a GDScript float (f64)
    fn xf(self) -> f64;
    fn yf(self) -> f64;
    fn zf(self) -> f64;
    /// Godot Vector3::limit_length: l = length(); l > 0 and len < l -> v / l * len (f32, divide then multiply)
    fn limit_length(self, len: f64) -> FVector;
    /// unary minus (Vector3 operator-)
    fn neg(self) -> FVector;
    /// Godot Vector3::is_zero_approx: every |component| < CMP_EPSILON
    fn is_zero_approx(self) -> bool;
}

impl V3Ext for FVector {
    #[inline]
    fn xf(self) -> f64 {
        self.x as f64
    }
    #[inline]
    fn yf(self) -> f64 {
        self.y as f64
    }
    #[inline]
    fn zf(self) -> f64 {
        self.z as f64
    }
    fn limit_length(self, len: f64) -> FVector {
        let len = len as f32;
        let l = self.length();
        if l > 0.0 && len < l {
            let v = FVector::new(self.x / l, self.y / l, self.z / l);
            return FVector::new(v.x * len, v.y * len, v.z * len);
        }
        self
    }
    fn neg(self) -> FVector {
        FVector::new(-self.x, -self.y, -self.z)
    }
    fn is_zero_approx(self) -> bool {
        self.x.abs() < CMP_EPSILON && self.y.abs() < CMP_EPSILON && self.z.abs() < CMP_EPSILON
    }
}

/// Godot Vector2::angle_to: atan2(cross, dot) in f32 (Math::atan2(float, float) = atan2f); the result as a GDScript
/// float. Vectors given as (x, y) f32 pairs.
pub fn angle_to(a: (f32, f32), b: (f32, f32)) -> f64 {
    let cross = a.0 * b.1 - a.1 * b.0;
    let dot = a.0 * b.0 + a.1 * b.1;
    cross.atan2(dot) as f64
}

/// GDScript rad_to_deg / deg_to_rad (Math::rad_to_deg(double): y * (180 / Math_PI))
#[inline]
pub fn rad_to_deg(y: f64) -> f64 {
    y * (180.0 / std::f64::consts::PI)
}
#[inline]
pub fn deg_to_rad(y: f64) -> f64 {
    y * (std::f64::consts::PI / 180.0)
}

/// Godot Basis(axis, angle) (Basis::set_axis_angle, f32): rows of the rotation matrix, written term by term as Godot
/// does so signed zeros match. Returns the rows.
pub fn basis_axis_angle(axis: FVector, angle: f64) -> [[f32; 3]; 3] {
    let a = angle as f32;
    let sq = FVector::new(axis.x * axis.x, axis.y * axis.y, axis.z * axis.z);
    let cosine = a.cos();
    let mut r = [[0.0f32; 3]; 3];
    r[0][0] = sq.x + cosine * (1.0 - sq.x);
    r[1][1] = sq.y + cosine * (1.0 - sq.y);
    r[2][2] = sq.z + cosine * (1.0 - sq.z);
    let sine = a.sin();
    let t = 1.0 - cosine;
    let mut xyzt = axis.x * axis.y * t;
    let mut zyxs = axis.z * sine;
    r[0][1] = xyzt - zyxs;
    r[1][0] = xyzt + zyxs;
    xyzt = axis.x * axis.z * t;
    zyxs = axis.y * sine;
    r[0][2] = xyzt + zyxs;
    r[2][0] = xyzt - zyxs;
    xyzt = axis.y * axis.z * t;
    zyxs = axis.x * sine;
    r[1][2] = xyzt - zyxs;
    r[2][1] = xyzt + zyxs;
    r
}

/// Basis column c (GDScript `basis.x` / `.y` / `.z`)
pub fn basis_col(r: &[[f32; 3]; 3], c: usize) -> FVector {
    FVector::new(r[0][c], r[1][c], r[2][c])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limit_length_divides_then_scales() {
        let v = FVector::new(3.0, 0.0, 4.0);
        assert_eq!(v.limit_length(1.0), FVector::new(0.6, 0.0, 0.8));
        assert_eq!(v.limit_length(10.0), v);
        assert_eq!(FVector::ZERO.limit_length(1.0), FVector::ZERO);
    }
    #[test]
    fn yaw_zero_basis_faces_minus_z() {
        let b = basis_axis_angle(FVector::new(0.0, 1.0, 0.0), 0.0);
        assert_eq!(basis_col(&b, 2).neg(), FVector::new(-0.0, -0.0, -1.0));
        assert_eq!(basis_col(&b, 0), FVector::new(1.0, 0.0, 0.0));
    }
}

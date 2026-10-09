//! UE 4.26 vector math as the shipped exe computes it (pure binary32, SSE), for the exe-exact movement (`exe`).
//!
//! Every operation here is read off the disassembly of the functions that inline it, not from UE source memory:
//!  - `FMath::InvSqrt` = `rsqrtss` + two Newton-Raphson steps (y += y * (0.5 - (x * 0.5) * y * y), twice): inlined in
//!    UCharacterMovementComponent::CalcVelocity rva=0x2f70490 (0x142f70695..0x142f706d3) and every GetSafeNormal /
//!    GetClampedToMaxSize below. `rsqrtss` is a CPU table approximation: the result is bit-exact for the CPU that runs
//!    this code, as it is for the game on that CPU (UNCONFIRMED across CPU vendors; non-x86 builds fall back to 1/sqrt).
//!  - `FVector::SizeSquared` = (X*X + Y*Y) + Z*Z (addss order in IsExceedingMaxSpeed rva=0x2fb3720), `Size` = sqrtss of it.
//!  - `GetSafeNormal(Tol)`: SizeSquared == 1 -> itself; < Tol (SMALL_NUMBER 1e-8, .rdata 0x144014a88) -> zero; else
//!    * InvSqrt (CalcVelocity 0x142f70669..0x142f706e9).
//!  - `GetClampedToMaxSize(Max)`: Max < KINDA_SMALL_NUMBER (1e-4, .rdata 0x144022350) -> zero; SizeSquared > Max^2 ->
//!    * (InvSqrt(SizeSquared) * Max) (CalcVelocity 0x142f70d47..0x142f70dfe).
//!  - `A | B` = (X*BX + Y*BY) + Z*BZ; the exe sometimes commutes the first sum, which is exact.
//!  - MSVC compiled the engine with fast floating point (a division by a variable is often a multiplication by its
//!    reciprocal, e.g. PhysWalking's ActualDist / DesiredDist at 0x142f822c6); call sites follow the disassembly, these
//!    helpers do not hide such choices.
//! inv_sqrt, dot, cross, size_sq, safe_normal(_tol / _2d), clamped_to_max_size, plane_project and the SMALL /
//! KINDA_SMALL constants live in mordhau-core::ue (the ue_ block, moved there in r3) and are re-exported here.
//! FVector is mordhau-core::ue's (f32 components); these are free functions over it so the Godot-semantics methods
//! there (reference_compat) and the UE-semantics ones here cannot be mixed up.

use crate::ue::FVector;

pub use mordhau_core::ue::{
    ue_clamped_to_max_size as clamped_to_max_size, ue_cross as cross, ue_dot as dot, ue_inv_sqrt as inv_sqrt,
    ue_plane_project as plane_project, ue_safe_normal as safe_normal, ue_safe_normal_2d as safe_normal_2d,
    ue_safe_normal_tol as safe_normal_tol, ue_size_sq as size_sq, UE_KINDA_SMALL_NUMBER as KINDA_SMALL_NUMBER,
    UE_SMALL_NUMBER as SMALL_NUMBER,
};

#[inline]
pub fn v(x: f32, y: f32, z: f32) -> FVector {
    FVector::new(x, y, z)
}
#[inline]
pub fn add(a: FVector, b: FVector) -> FVector {
    v(a.x + b.x, a.y + b.y, a.z + b.z)
}
#[inline]
pub fn sub(a: FVector, b: FVector) -> FVector {
    v(a.x - b.x, a.y - b.y, a.z - b.z)
}
/// FVector * float (each component times s)
#[inline]
pub fn mul(a: FVector, s: f32) -> FVector {
    v(a.x * s, a.y * s, a.z * s)
}
#[inline]
pub fn neg(a: FVector) -> FVector {
    v(-a.x, -a.y, -a.z)
}
#[inline]
pub fn size(a: FVector) -> f32 {
    size_sq(a).sqrt()
}
#[inline]
pub fn size_sq_2d(a: FVector) -> f32 {
    a.x * a.x + a.y * a.y
}
#[inline]
pub fn size_2d(a: FVector) -> f32 {
    size_sq_2d(a).sqrt()
}
/// exact component compare (FVector::operator==, -0 == 0)
#[inline]
pub fn eq(a: FVector, b: FVector) -> bool {
    a.x == b.x && a.y == b.y && a.z == b.z
}
#[inline]
pub fn is_zero(a: FVector) -> bool {
    a.x == 0.0 && a.y == 0.0 && a.z == 0.0
}
/// FVector::IsNearlyZero(Tol): every |component| <= Tol
#[inline]
pub fn is_nearly_zero(a: FVector, tol: f32) -> bool {
    a.x.abs() <= tol && a.y.abs() <= tol && a.z.abs() <= tol
}
/// FVector::Equals(V, Tol): every |difference| <= Tol
#[inline]
pub fn equals(a: FVector, b: FVector, tol: f32) -> bool {
    (a.x - b.x).abs() <= tol && (a.y - b.y).abs() <= tol && (a.z - b.z).abs() <= tol
}





/// FMath::Clamp (X < Min ? Min : X < Max ? X : Max) as the exe's comiss/minss pairs compute it
#[inline]
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x < hi {
        x
    } else {
        hi
    }
}
/// maxss a, b (b when a <= b ... SSE: returns the second operand unless the first is greater)
#[inline]
pub fn maxss(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}
/// minss a, b
#[inline]
pub fn minss(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// FRandomStream (the CMC's +0x4d4) as UCharacterMovementComponent::PhysFalling rva=0x2f7ef00 inlines FRand for the
/// "virtual ditch" nudge (read off the disassembly, 0x142f800aa..0x142f800e5): Seed = Seed * 196314165 (0xbb38435) +
/// 907633515 (0x3619636b); the result is f32::from_bits(0x3f800000 | (Seed >> 9)) - 1.0 (logical shift)
#[derive(Clone, Debug, Default)]
pub struct RandomStream {
    pub seed: i32,
}

impl RandomStream {
    pub fn frand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(196_314_165).wrapping_add(907_633_515);
        f32::from_bits(0x3f80_0000u32 | ((self.seed as u32) >> 9)) - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inv_sqrt_close_to_exact() {
        for x in [1.0f32, 2.0, 3.0, 1650.0 * 1650.0, 1e-6, 12345.678] {
            let e = 1.0 / (x as f64).sqrt();
            assert!(((inv_sqrt(x) as f64) - e).abs() <= e * 2e-7, "{x}");
        }
    }
    #[test]
    fn safe_normal_rules() {
        assert_eq!(safe_normal(v(1.0, 0.0, 0.0)), v(1.0, 0.0, 0.0));
        assert_eq!(safe_normal(v(1e-5, 0.0, 0.0)), FVector::ZERO);
        let n = safe_normal(v(3.0, 4.0, 0.0));
        assert!((n.x - 0.6).abs() < 1e-6 && (n.y - 0.8).abs() < 1e-6);
        assert_eq!(clamped_to_max_size(v(3.0, 4.0, 0.0), 1e-5), FVector::ZERO);
        assert_eq!(clamped_to_max_size(v(0.3, 0.4, 0.0), 1.0), v(0.3, 0.4, 0.0));
    }
}

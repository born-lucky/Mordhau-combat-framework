//! `mordhau-core::ue` - shared UE math with the shipped exe's numeric semantics, used by every sim crate.
//! Owner: rust-combat (state/rust_brief.md). Others: request additions, or add a small clearly marked fn and report it.
//!
//! Float model: the GDScript reference (godot/game/**) computes in f64 (GDScript `float`) and rounds to binary32
//! exactly where the exe's single precision matters (`UeMath.f32`, PackedFloat32Array stores, Godot Vector2/Vector3,
//! which are f32 components). The Rust port mirrors that bit for bit: scalars are `f64`, `f32()` marks every place the
//! reference rounds, and `FVector` / `Vec2` hold `f32` components like their Godot/UE counterparts.
//! Ported from godot/game/combat/ue_math.gd, godot/game/ai/ue_rand.gd.

/// Round to IEEE-754 binary32 and back (GDScript `UeMath.f32`, a PackedFloat32Array store): the game computes in float
/// (e.g. 32767 * (1/32767f) is exactly 1.0f, while the same product in double is 0.999999999).
#[inline]
pub fn f32r(x: f64) -> f64 {
    x as f32 as f64
}

/// x86 cvtss2si (Ghidra's ROUND): round half to even under the default MXCSR (round-to-nearest-even).
/// Mirrors ue_math.gd `cvtss2si` (floor, then compare the fraction with 0.5) exactly.
pub fn cvtss2si(v: f64) -> i64 {
    let f = v.floor();
    let d = v - f;
    let fi = f as i64;
    if d > 0.5 {
        return fi + 1;
    }
    if d < 0.5 {
        return fi;
    }
    fi + (fi & 1)
}

/// x86 cvttss2si (Ghidra's (int) cast of a float): truncation toward zero. GDScript `int(float)`.
#[inline]
pub fn cvttss2si(v: f64) -> i64 {
    trunc_i(v)
}

/// GDScript `int(x)` of a float: truncation toward zero (saturating like Rust `as`).
#[inline]
pub fn trunc_i(v: f64) -> i64 {
    v as i64
}

/// FMath::RoundToInt as UE 4.26 compiles it on SSE: cvtss2si(2x + 0.5) >> 1 (seen in AMordhauWeapon::SampleTracers
/// rva=0x163c430 and UStatComponent::WriteReplicatedStat rva=0x1520b50).
pub fn round_to_int(v: f64) -> i64 {
    cvtss2si(v + v + 0.5) >> 1
}

/// from UMordhauUtilityLibrary::GetNormalizedTime rva=0x1624620:
///   Current < End: Current <= Start -> 0; Start < End -> (Current - Start) / (End - Start); otherwise 1
pub fn normalized_time(start: f64, end: f64, current: f64) -> f64 {
    if current < end {
        if current <= start {
            return 0.0;
        }
        if start < end {
            return (current - start) / (end - start);
        }
    }
    1.0
}

/// The range fraction UBlockedMotion::OnBegin_Implementation rva=0x165dc20 and OnTick_Implementation rva=0x16671b0
/// inline: x' = clamp(x, lo, hi) (lo first); |lo - hi| > SMALL_NUMBER and lo <= hi -> clamp((x' - lo) / (hi - lo), 0, 1),
/// otherwise 0. `small_number` = Constants::small_number (.rdata 0x144014a88).
pub fn range_fraction(lo: f64, hi: f64, x: f64, small_number: f64) -> f64 {
    let mut c = lo;
    if lo <= x {
        c = x;
        if hi <= x {
            c = hi;
        }
    }
    if (lo - hi).abs() <= small_number || hi < lo {
        return 0.0;
    }
    clampf((c - lo) / (hi - lo), 0.0, 1.0)
}

/// Godot `clampf` (CLAMP: a < min ? min : (a > max ? max : a)); never panics, unlike f64::clamp.
#[inline]
pub fn clampf(a: f64, lo: f64, hi: f64) -> f64 {
    if a < lo {
        lo
    } else if a > hi {
        hi
    } else {
        a
    }
}

/// Godot `maxf` (a > b ? a : b)
#[inline]
pub fn maxf(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

/// Godot `minf` (a < b ? a : b)
#[inline]
pub fn minf(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

/// Godot `lerpf` (from + (to - from) * weight), used by the FRichCurve Bezier port.
#[inline]
pub fn lerpf(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// A 2-component f32 vector (Godot Vector2 / UE FVector2D): the records' (X, Y) pairs (clamps, limits, ranges).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Vec2 { x, y }
    }
    /// components as the GDScript reads them (`v.x` is a float, i.e. f64 of the f32)
    #[inline]
    pub fn xf(self) -> f64 {
        self.x as f64
    }
    #[inline]
    pub fn yf(self) -> f64 {
        self.y as f64
    }
}

/// FVector with f32 components (UE single precision; also the Godot Vector3 the GDScript reference uses). Arithmetic
/// is per component in f32, like both engines.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FVector {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl FVector {
    pub const ZERO: FVector = FVector { x: 0.0, y: 0.0, z: 0.0 };
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        FVector { x, y, z }
    }
    pub fn dot(self, o: FVector) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn length_squared(self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }
    /// Godot Vector3::length (sqrt of the f32 sum)
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }
    /// Godot Vector3::normalized: zero vector stays zero, else divided by its length
    pub fn normalized(self) -> FVector {
        let l = self.length_squared();
        if l == 0.0 {
            return FVector::ZERO;
        }
        let l = l.sqrt();
        FVector::new(self.x / l, self.y / l, self.z / l)
    }
    /// scale by a GDScript float (converted to real_t = f32 first, as Godot's Vector3 * float does)
    pub fn scale(self, s: f64) -> FVector {
        let s = s as f32;
        FVector::new(self.x * s, self.y * s, self.z * s)
    }
    pub fn get(self, k: usize) -> f32 {
        match k {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }
}

impl std::ops::Add for FVector {
    type Output = FVector;
    fn add(self, o: FVector) -> FVector {
        FVector::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl std::ops::Sub for FVector {
    type Output = FVector;
    fn sub(self, o: FVector) -> FVector {
        FVector::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

/// FRotator (pitch, yaw, roll), degrees, f32 components.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FRotator {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

/// An affine transform with f32 components (Godot Transform3D: basis rows + origin). Only what the combat trace
/// needs: apply to a point, inverse for the segment-vs-box slab test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    /// basis rows (Godot Basis.rows)
    pub rows: [FVector; 3],
    pub origin: FVector,
}

impl Default for Xform {
    fn default() -> Self {
        Xform::IDENTITY
    }
}

impl Xform {
    pub const IDENTITY: Xform = Xform {
        rows: [FVector::new(1.0, 0.0, 0.0), FVector::new(0.0, 1.0, 0.0), FVector::new(0.0, 0.0, 1.0)],
        origin: FVector::ZERO,
    };
    /// Godot Transform3D::xform: basis * v + origin
    pub fn apply(&self, v: FVector) -> FVector {
        FVector::new(
            self.rows[0].dot(v) + self.origin.x,
            self.rows[1].dot(v) + self.origin.y,
            self.rows[2].dot(v) + self.origin.z,
        )
    }
    /// Godot Transform3D * Transform3D
    pub fn mul(&self, o: &Xform) -> Xform {
        let col = |m: &Xform, c: usize| FVector::new(m.rows[0].get(c), m.rows[1].get(c), m.rows[2].get(c));
        let mut rows = [FVector::ZERO; 3];
        for (r, row) in rows.iter_mut().enumerate() {
            *row = FVector::new(self.rows[r].dot(col(o, 0)), self.rows[r].dot(col(o, 1)), self.rows[r].dot(col(o, 2)));
        }
        Xform { rows, origin: self.apply(o.origin) }
    }
    /// Godot Transform3D::affine_inverse (Basis::inverse by cofactors, then -B^-1 * origin)
    pub fn affine_inverse(&self) -> Xform {
        let r = &self.rows;
        let (a, b, c) = (r[0], r[1], r[2]);
        let co = |r1: FVector, r2: FVector, c1: usize, c2: usize| r1.get(c1) * r2.get(c2) - r1.get(c2) * r2.get(c1);
        let cof00 = co(b, c, 1, 2);
        let cof01 = co(b, c, 2, 0);
        let cof02 = co(b, c, 0, 1);
        let det = a.x * cof00 + a.y * cof01 + a.z * cof02;
        let s = 1.0 / det;
        let inv = [
            FVector::new(cof00 * s, co(a, c, 2, 1) * s, co(a, b, 1, 2) * s),
            FVector::new(cof01 * s, co(a, c, 0, 2) * s, co(a, b, 2, 0) * s),
            FVector::new(cof02 * s, co(a, c, 1, 0) * s, co(a, b, 0, 1) * s),
        ];
        let m = Xform { rows: inv, origin: FVector::ZERO };
        let o = m.apply(self.origin);
        Xform { rows: inv, origin: FVector::new(-o.x, -o.y, -o.z) }
    }
}

/// The C runtime rand() every bot function draws from (godot/game/ai/ue_rand.gd): seed = seed * 214013 + 2531011,
/// return (seed >> 16) & 0x7fff, initial seed 1 (documented Microsoft CRT rand; UNCONFIRMED: the shipped UCRT's
/// constants were not disassembled, the import lives in ucrtbase.dll). Callers: UBotBehaviorProfile::
/// RerollRandomInstanceValues rva=0x149e910, UBTTask_MeleeAttack::PerformOffensiveEvaluation rva=0x1495d00.
/// `forced` holds raw rand() results to return first (tests: exact branch control).
#[derive(Clone, Debug)]
pub struct CrtRand {
    pub seed: u32,
    pub calls: u64,
    pub forced: std::collections::VecDeque<i64>,
}

impl Default for CrtRand {
    fn default() -> Self {
        CrtRand::new(1)
    }
}

impl CrtRand {
    pub fn new(seed: u32) -> Self {
        CrtRand { seed, calls: 0, forced: Default::default() }
    }
    pub fn rand(&mut self) -> i64 {
        self.calls += 1;
        if let Some(v) = self.forced.pop_front() {
            return v & 0x7fff;
        }
        self.seed = self.seed.wrapping_mul(214013).wrapping_add(2531011);
        ((self.seed >> 16) & 0x7fff) as i64
    }
    /// FMath::FRand as the game inlines it: f32((rand() & 0x7fff) * (1/32767)); `frand_scale` = BotConstants
    /// frand_scale (.rdata 0x1440dfae0).
    pub fn frand(&mut self, frand_scale: f64) -> f64 {
        f32r(self.rand() as f64 * frand_scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cvtss2si_half_even() {
        assert_eq!(cvtss2si(0.5), 0);
        assert_eq!(cvtss2si(1.5), 2);
        assert_eq!(cvtss2si(2.5), 2);
        assert_eq!(cvtss2si(-0.5), 0);
        assert_eq!(cvtss2si(-1.5), -2);
        assert_eq!(cvtss2si(2.6), 3);
        assert_eq!(round_to_int(2.5), 3);
    }
    #[test]
    fn crt_rand_sequence() {
        // MS CRT rand() with srand(1): 41, 18467, 6334, 26500 (the documented "well-known sequence")
        let mut r = CrtRand::new(1);
        assert_eq!([r.rand(), r.rand(), r.rand(), r.rand()], [41, 18467, 6334, 26500]);
    }
    #[test]
    fn f32_rounding() {
        assert_eq!(f32r(0.1), 0.10000000149011612);
        assert_eq!(normalized_time(1.0, 2.0, 1.5), 0.5);
        assert_eq!(range_fraction(0.0, 2.0, 3.0, 1e-8), 1.0);
    }
    #[test]
    fn xform_inverse_roundtrip() {
        let x = Xform { rows: [FVector::new(0.0, -1.0, 0.0), FVector::new(1.0, 0.0, 0.0), FVector::new(0.0, 0.0, 2.0)], origin: FVector::new(1.0, 2.0, 3.0) };
        let p = FVector::new(0.5, -0.25, 4.0);
        let q = x.affine_inverse().apply(x.apply(p));
        assert!((q - p).length() < 1e-5);
    }
}

// ==== UE-exact vector helpers (added by rust-character r3, agreed with rust-combat; append-only block) ==============
// UE 4.26 FVector operations as the shipped exe computes them (binary32, SSE), read off the disassembly of the functions
// that inline them (mh-character exe mode). These are free functions over FVector, separate from FVector's
// Godot-semantics methods above (normalized / length / scale), which reproduce the GDScript reference.
//  - FMath::InvSqrt = rsqrtss + two Newton-Raphson steps (UCharacterMovementComponent::CalcVelocity rva=0x2f70490,
//    0x142f70695..0x142f706d3). rsqrtss is a CPU table approximation: bit-exact on the CPU that runs it, as the game
//    is (UNCONFIRMED across CPU vendors); non-x86_64 builds fall back to 1/sqrt.
//  - SizeSquared = (X*X + Y*Y) + Z*Z (UMovementComponent::IsExceedingMaxSpeed rva=0x2fb3720).
//  - GetSafeNormal: SizeSquared == 1 -> itself; < tol (SMALL_NUMBER 1e-8, .rdata 0x144014a88) -> zero; else
//    * InvSqrt (CalcVelocity 0x142f70669..0x142f706e9). GetSafeNormal2D (HandleSlopeBoosting rva=0x2f7aac0).
//  - GetClampedToMaxSize: Max < KINDA_SMALL_NUMBER (1e-4, .rdata 0x144022350) -> zero; SizeSquared > Max^2 ->
//    * (InvSqrt * Max) (CalcVelocity 0x142f70d47..0x142f70dfe).
//  - VectorPlaneProject (UMovementComponent::ComputeSlideVector rva=0x2fa0b80), A | B, A ^ B.

/// SMALL_NUMBER (.rdata 0x144014a88)
pub const UE_SMALL_NUMBER: f32 = 1e-8;
/// KINDA_SMALL_NUMBER (.rdata 0x144022350)
pub const UE_KINDA_SMALL_NUMBER: f32 = 1e-4;

/// FMath::InvSqrt as inlined (rsqrtss + 2 Newton-Raphson iterations)
#[inline]
pub fn ue_inv_sqrt(x: f32) -> f32 {
    let y0 = ue_rsqrtss(x);
    let half = x * 0.5;
    let y1 = y0 + y0 * (0.5 - half * (y0 * y0));
    y1 + y1 * (0.5 - half * (y1 * y1))
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn ue_rsqrtss(x: f32) -> f32 {
    // SAFETY: SSE is part of the x86_64 baseline.
    unsafe {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(x)))
    }
}

/// UNCONFIRMED stand-in off x86_64: no rsqrtss table
#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn ue_rsqrtss(x: f32) -> f32 {
    1.0 / x.sqrt()
}

/// A | B
#[inline]
pub fn ue_dot(a: FVector, b: FVector) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
/// A ^ B: (Y*BZ - Z*BY, Z*BX - X*BZ, X*BY - Y*BX)
#[inline]
pub fn ue_cross(a: FVector, b: FVector) -> FVector {
    FVector::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}
/// SizeSquared
#[inline]
pub fn ue_size_sq(a: FVector) -> f32 {
    a.x * a.x + a.y * a.y + a.z * a.z
}
/// GetSafeNormal(SMALL_NUMBER)
pub fn ue_safe_normal(a: FVector) -> FVector {
    ue_safe_normal_tol(a, UE_SMALL_NUMBER)
}
/// GetSafeNormal(Tolerance)
pub fn ue_safe_normal_tol(a: FVector, tol: f32) -> FVector {
    let sq = ue_size_sq(a);
    if sq == 1.0 {
        return a;
    }
    if sq < tol {
        return FVector::ZERO;
    }
    let s = ue_inv_sqrt(sq);
    FVector::new(a.x * s, a.y * s, a.z * s)
}
/// GetSafeNormal2D: SizeSquared2D == 1 -> (X, Y, 0) (itself when Z == 0); < SMALL_NUMBER -> zero; else * InvSqrt, Z = 0
pub fn ue_safe_normal_2d(a: FVector) -> FVector {
    let sq = a.x * a.x + a.y * a.y;
    if sq == 1.0 {
        return if a.z == 0.0 { a } else { FVector::new(a.x, a.y, 0.0) };
    }
    if sq < UE_SMALL_NUMBER {
        return FVector::ZERO;
    }
    let s = ue_inv_sqrt(sq);
    FVector::new(a.x * s, a.y * s, 0.0)
}
/// GetClampedToMaxSize(MaxSize)
pub fn ue_clamped_to_max_size(a: FVector, max: f32) -> FVector {
    if max < UE_KINDA_SMALL_NUMBER {
        return FVector::ZERO;
    }
    let sq = ue_size_sq(a);
    if sq > max * max {
        let s = ue_inv_sqrt(sq) * max;
        return FVector::new(a.x * s, a.y * s, a.z * s);
    }
    a
}
/// FVector::VectorPlaneProject(V, N) = V - N * (V | N)
pub fn ue_plane_project(a: FVector, n: FVector) -> FVector {
    let d = ue_dot(a, n);
    FVector::new(a.x - n.x * d, a.y - n.y * d, a.z - n.z * d)
}
// ==== end of UE-exact vector helpers ==================================================================================

// ==== UE rotations and transforms (rust-combat r2; FQuat / FTransform as UE 4.26 defines them, f32) =================

/// FQuat (x, y, z, w), f32
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FQuat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for FQuat {
    fn default() -> Self {
        FQuat::IDENTITY
    }
}

impl FQuat {
    pub const IDENTITY: FQuat = FQuat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> FQuat {
        FQuat { x, y, z, w }
    }
    pub fn from_array(a: [f32; 4]) -> FQuat {
        FQuat::new(a[0], a[1], a[2], a[3])
    }
    /// FQuat::operator* (Hamilton product, this applied after `b`: (self * b).rotate(v) = self.rotate(b.rotate(v)))
    pub fn mul(self, b: FQuat) -> FQuat {
        let a = self;
        FQuat::new(
            a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
            a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        )
    }
    /// FQuat::RotateVector as the exe inlines it (UParryMotion::TestForwardParry rva=0x166dbd0, decomp: T = 2 (Q x V)
    /// as (a - b) + (a - b) per lane, result = (Q x T + W * T) + V, in that summation order)
    pub fn rotate(self, v: FVector) -> FVector {
        let q = FVector::new(self.x, self.y, self.z);
        let c = cross(q, v);
        let t = FVector::new(c.x + c.x, c.y + c.y, c.z + c.z);
        let qt = cross(q, t);
        FVector::new((qt.x + self.w * t.x) + v.x, (qt.y + self.w * t.y) + v.y, (qt.z + self.w * t.z) + v.z)
    }
    pub fn inverse(self) -> FQuat {
        FQuat::new(-self.x, -self.y, -self.z, self.w)
    }
    /// FRotator::Quaternion rva 0x18b8f90 (exe 0x1418b9f90, disassembled with scripts/ue_dis.py): the SIMD path, per lane
    /// on (Pitch, Yaw, Roll):
    ///   VectorMod(A, 360): T = cvttps2dq(A / 360) (kept as A/360 when |A/360| >= 2^23), R = clamp(A - T*360, -360, 360)
    ///   H = R * DEG_TO_RAD_HALF; VectorSinCos(H) (Q = cvtps2dq(H * 1/(2pi)) round-half-even, X = H - Q*2pi, reflect
    ///   |X| > pi/2 to (sign(X) pi) - X with cos sign -1, the 11 / 10-degree minimax polynomials from .rdata 0x144070290
    ///   (sin 1, -1/6, 0.008333331, -0.00019840874), 0x144070230 (2.7525562e-06, -2.3889859e-08), 0x1440702a0 (cos 1,
    ///   -0.5, 0.041666638, -0.0013888378), 0x144070240 (2.4760495e-05, -2.6051615e-07));
    ///   X = (SY*SP)*CR - (CY*CP)*SR, Y = -(CY*SP)*CR - (SY*CP)*SR, Z = (SY*CP)*CR - (CY*SP)*SR,
    ///   W = (CY*CP)*CR + (SY*SP)*SR (shuffle / sign-mask order of the disassembly, sign masks .rdata 0x14449f8f0 /
    ///   0x14449f910).
    /// UNCONFIRMED: the GlobalVectorConstants read from .data (360, 2^23, DEG_TO_RAD_HALF, 1/(2pi), 2pi, pi, pi/2,
    /// +-1) are filled at startup and not readable at rest; their float values are taken equal to the scalar
    /// FMath::SinCos literals in .rdata (0x1442713d4 360, 0x1442713cc 0.159155, 0x14402cb88 6.28319, 0x144034fb8
    /// 3.14159, 0x1440e9f90 1.5708) and DEG_TO_RAD_HALF = f32(pi / 360).
    pub fn from_rotator(pitch: f32, yaw: f32, roll: f32) -> FQuat {
        let (s, c) = (vector_sin_cos_lane(rot_half(pitch)), vector_sin_cos_lane(rot_half(yaw)));
        let r = vector_sin_cos_lane(rot_half(roll));
        let (sp, cp, sy, cy, sr, cr) = (s.0, s.1, c.0, c.1, r.0, r.1);
        FQuat::new(
            (sy * sp) * cr + -((cy * cp) * sr),
            -((cy * sp) * cr) + -((sy * cp) * sr),
            (sy * cp) * cr + -((cy * sp) * sr),
            (cy * cp) * cr + (sy * sp) * sr,
        )
    }
}

/// VectorMod(A, 360) then * DEG_TO_RAD_HALF, one lane (FRotator::Quaternion, see there)
fn rot_half(a: f32) -> f32 {
    let q = a / 360.0;
    let t = if q.abs() >= 8388608.0 { q } else { q.trunc() };
    let r = (a - t * 360.0).min(360.0).max(-360.0);
    r * (std::f32::consts::PI / 360.0)
}

/// VectorSinCos, one lane: (sin, cos) of x (FRotator::Quaternion's SIMD path, see there)
fn vector_sin_cos_lane(x: f32) -> (f32, f32) {
    const ONE_OVER_TWO_PI: f32 = 0.159154943; // 0x1442713cc
    const TWO_PI: f32 = 6.28318531; // 0x14402cb88
    const PI: f32 = 3.14159265; // 0x144034fb8
    const PI_BY_TWO: f32 = 1.57079633; // 0x1440e9f90
    let qn = (x * ONE_OVER_TWO_PI).round_ties_even();
    let mut y = x - qn * TWO_PI;
    let c = if y.is_sign_negative() { -PI } else { PI };
    let comp = PI_BY_TWO < y.abs();
    if comp {
        y = c - y;
    }
    let sign = if comp { -1.0f32 } else { 1.0 };
    let y2 = y * y;
    let mut s = -2.3889859e-08f32 * y2 + 2.7525562e-06;
    s = s * y2 + -0.00019840874;
    s = s * y2 + 0.008333331;
    s = s * y2 + -0.16666667;
    s = s * y2 + 1.0;
    let mut k = -2.6051615e-07f32 * y2 + 2.4760495e-05;
    k = k * y2 + -0.0013888378;
    k = k * y2 + 0.041666638;
    k = k * y2 + -0.5;
    k = k * y2 + 1.0;
    (s * y, k * sign)
}

/// FMath::SinCos as FRotator::Vector rva 0x18c3050 inlines it (exe 0x1418c40a5..: quotient = value * 1/(2pi)
/// (0x1442713cc) +-0.5 then cvttss2si, y = value - 2pi * quotient, reflect beyond +-pi/2 (0x1440e9f90 / 0x1442713dc)
/// to +-pi - y with cos sign -1, then the same polynomials as scalar ss ops: sin c0 = 2.75256e-06 (0x1442713b0) -
/// y2 * 2.38899e-08 (0x1442713a8), ..., cos 2.47605e-05 (0x1442713b8) - y2 * 2.60516e-07 (0x1442713ac), ...)
pub fn sin_cos_scalar(v: f32) -> (f32, f32) {
    let mut q = v * 0.159154943;
    q = if v >= 0.0 { q + 0.5 } else { q - 0.5 };
    let q = (q as i32) as f32;
    let mut y = v - q * 6.28318531;
    let sign;
    if y > 1.57079633 {
        y = 3.14159265 - y;
        sign = -1.0f32;
    } else if y < -1.57079633 {
        y = -3.14159265 - y;
        sign = -1.0;
    } else {
        sign = 1.0;
    }
    let y2 = y * y;
    let s = (((((2.7525562e-06f32 - y2 * 2.3889859e-08) * y2 - 0.00019840874) * y2 + 0.008333331) * y2 - 0.16666667) * y2 + 1.0) * y;
    let p = ((((2.4760495e-05f32 - y2 * 2.6051615e-07) * y2 - 0.0013888378) * y2 + 0.041666638) * y2 - 0.5) * y2 + 1.0;
    (s, sign * p)
}

/// FRotator::Vector rva 0x18c3050 (exe 0x1418c4050): fmodf(Pitch, 360), fmodf(Yaw, 360) (call 0x141815130),
/// * DEG2RAD (0.0174533, .rdata 0x144022360), SinCos each -> (CP*CY, CP*SY, SP)
pub fn rotator_vector(pitch: f32, yaw: f32) -> FVector {
    // FMath::Fmod rva=0x1815130 (ue_fmod; rust-mode-ai r4: was Rust `%`, exact fmodf, which differs for |A| >= 720)
    let (sp, cp) = sin_cos_scalar(ue_fmod(pitch, 360.0) * 0.0174532925);
    let (sy, cy) = sin_cos_scalar(ue_fmod(yaw, 360.0) * 0.0174532925);
    FVector::new(cp * cy, cp * sy, sp)
}

fn cross(a: FVector, b: FVector) -> FVector {
    FVector::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// FTransform without scale (rotation + translation), UE space (cm)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FTransform {
    pub rot: FQuat,
    pub loc: FVector,
}

impl FTransform {
    pub const IDENTITY: FTransform = FTransform { rot: FQuat::IDENTITY, loc: FVector::ZERO };
    pub fn new(rot: FQuat, loc: FVector) -> FTransform {
        FTransform { rot, loc }
    }
    /// FTransform::TransformPosition
    pub fn apply(&self, v: FVector) -> FVector {
        self.rot.rotate(v) + self.loc
    }
    /// UE `Local * Parent` (FTransform::Multiply): the child's transform in the parent's space put into world space
    pub fn then(&self, parent: &FTransform) -> FTransform {
        FTransform { rot: parent.rot.mul(self.rot), loc: parent.apply(self.loc) }
    }
    /// FTransform::Inverse
    pub fn inverse(&self) -> FTransform {
        let r = self.rot.inverse();
        let l = r.rotate(self.loc);
        FTransform { rot: r, loc: FVector::new(-l.x, -l.y, -l.z) }
    }
    /// FTransform::InverseTransformPosition
    pub fn inverse_apply(&self, v: FVector) -> FVector {
        self.rot.inverse().rotate(v - self.loc)
    }
}
// ==== end of UE rotations and transforms ==============================================================================

// ==== exe float math shared by every crate (rust-mode-ai r4; moved here from mh-mode's ue_math) =====================
// Disassembled from Mordhau-Win64-Shipping.exe with capstone (rva = 0x1000 + the PDB `0001:` offset). Every function
// is plain SSE single ops in the instruction order shown; no FMA. The scalar FMath::SinCos is `sin_cos_scalar` and the
// SIMD lane is `vector_sin_cos` (above); FMath::InvSqrt is `ue_inv_sqrt`. The sinf / cosf / acosf the exe imports come
// from api-ms-win-crt-math-l1-1-0.dll (ucrtbase): Rust's f32 sin / cos / acos on x86_64-pc-windows-msvc call the same
// import (mh-mode tests/trig_crt.rs checks it bit for bit against FFI calls).

/// VectorSinCos, one lane (pub name of `vector_sin_cos_lane`)
pub fn vector_sin_cos(x: f32) -> (f32, f32) {
    vector_sin_cos_lane(x)
}

/// 180 / pi as the exe stores it (.rdata 0x1442713d0)
pub const UE_RAD_TO_DEG: f32 = 57.295_776;
/// pi / 180 (.rdata 0x144022360)
pub const UE_DEG_TO_RAD: f32 = 0.017_453_292;

/// FMath::Fmod rva=0x1815130: |Y| <= SMALL_NUMBER (0x144014a88) -> 0; Div = X / Y; Quot = |Div| < 2^23 (0x14448e378)
/// ? truncf(Div) (import) : Div; IntPortion = Quot * Y, replaced by X when |IntPortion| > |X|; Result = X - IntPortion
/// clamped to [-|Y|, |Y|] (comiss -|Y|, minss |Y|). Not IEEE fmod: X - trunc(X / Y) * Y in single ops.
pub fn ue_fmod(x: f32, y: f32) -> f32 {
    let ay = y.abs();
    if ay <= UE_SMALL_NUMBER {
        return 0.0;
    }
    let div = x / y;
    let quot = if div.abs() < 8_388_608.0 { div.trunc() } else { div };
    let mut ip = quot * y;
    if ip.abs() > x.abs() {
        ip = x;
    }
    let r = x - ip;
    if r < -ay {
        -ay
    } else {
        r.min(ay)
    }
}

/// FGenericPlatformMath::Atan2 rva=0x180f930: UE 4.26's 7-term minimax on min/max of |X|, |Y| (coefficients .rdata
/// 0x14448e348..0x14448e35c), then pi/2 - t when |Y| > |X|, pi - t when X < 0, negated when Y < 0; 0 when both are 0
pub fn ue_atan2(y: f32, x: f32) -> f32 {
    const A: [f32; 6] = [0.007_212_885, 0.035_059_68, 0.081_675_88, 0.133_746_58, 0.198_565_63, 0.333_249_99];
    let (ax, ay) = (x.abs(), y.abs());
    let y_bigger = ay > ax;
    let (t0, t1) = if y_bigger { (ay, ax) } else { (ax, ay) };
    if t0 == 0.0 {
        return 0.0;
    }
    let mut t3 = t1 / t0;
    let t4 = t3 * t3;
    let mut p = A[0] * t4 - A[1];
    p = p * t4 + A[2];
    p = p * t4 - A[3];
    p = p * t4 + A[4];
    p = p * t4 - A[5];
    p = p * t4 + 1.0;
    t3 *= p;
    if y_bigger {
        t3 = std::f32::consts::FRAC_PI_2 - t3;
    }
    if x < 0.0 {
        t3 = std::f32::consts::PI - t3;
    }
    if y < 0.0 {
        t3 = -t3;
    }
    t3
}

/// FRotator::ClampAxis: Fmod(A, 360), + 360 when negative
pub fn ue_clamp_axis(a: f32) -> f32 {
    let a = ue_fmod(a, 360.0);
    if a < 0.0 {
        a + 360.0
    } else {
        a
    }
}

/// FRotator::NormalizeAxis: ClampAxis, - 360 when > 180
pub fn ue_normalize_axis(a: f32) -> f32 {
    let a = ue_clamp_axis(a);
    if a > 180.0 {
        a - 360.0
    } else {
        a
    }
}

/// FMath::FindDeltaAngleDegrees rva=0x1480350: D = A2 - A1; > 180 -> + -360; < -180 -> + 360
pub fn ue_find_delta_angle_degrees(a1: f32, a2: f32) -> f32 {
    let d = a2 - a1;
    if d > 180.0 {
        d + -360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}

/// FMath::ClampAngle rva=0x18a27c0: MaxDelta = ClampAxis(Max - Min) * 0.5; Center = ClampAxis(Min + MaxDelta);
/// Delta = NormalizeAxis(A - Center); > MaxDelta -> NormalizeAxis(Center + MaxDelta); < -MaxDelta ->
/// NormalizeAxis(Center - MaxDelta); else NormalizeAxis(A)
pub fn ue_clamp_angle(a: f32, min: f32, max: f32) -> f32 {
    let max_delta = ue_clamp_axis(max - min) * 0.5;
    let center = ue_clamp_axis(min + max_delta);
    let delta = ue_normalize_axis(a - center);
    if delta > max_delta {
        ue_normalize_axis(center + max_delta)
    } else if delta < -max_delta {
        ue_normalize_axis(center - max_delta)
    } else {
        ue_normalize_axis(a)
    }
}

/// the X and Y axes of FRotationTranslationMatrix(FRotator(0, Yaw, 0)) (rva=0xb7e6f0: VectorSinCos of Rot * DEG_TO_RAD,
/// no Fmod; with pitch = roll = 0 every other term is a product with an exact 0 / 1): X = (CY, SY, 0), Y = (-SY, CY, 0).
/// UNCONFIRMED: the .data DEG_TO_RAD vector is taken equal to .rdata 0x144022360.
pub fn ue_yaw_axes(yaw: f32) -> (FVector, FVector) {
    let (sy, cy) = vector_sin_cos(yaw * UE_DEG_TO_RAD);
    (FVector::new(cy, sy, 0.0), FVector::new(-sy, cy, 0.0))
}

/// FRotator(0, Yaw, 0).RotateVector(V) rva=0x18bd9b0: FRotationMatrix(R).TransformVector(V) =
/// (W*M3 + Z*M2) + (Y*M1 + X*M0) lane-wise with W = 0: (X*CY - Y*SY, X*SY + Y*CY, Z)
pub fn ue_rotate_yaw(v: FVector, yaw: f32) -> FVector {
    let (ax, ay) = ue_yaw_axes(yaw);
    // M2 = (-0, 0, 1) for pitch = roll = 0; 0 * M3 lane = 0
    let x = (0.0 + v.z * -0.0) + (v.y * ay.x + v.x * ax.x);
    let y = (0.0 + v.z * 0.0) + (v.y * ay.y + v.x * ax.y);
    let z = (0.0 + v.z * 1.0) + (v.y * 0.0 + v.x * 0.0);
    FVector::new(x, y, z)
}

/// VectorQuaternionRotateVector as USceneComponent::GetForwardVector rva=0x2fa7ca0 inlines it: T = 2 * (V.zxy * Q.yzx -
/// V.yzx * Q.zxy); Result = (T.zxy * Q.yzx - T.yzx * Q.zxy) + (Q.w * T + V). (Note: FQuat::rotate above associates
/// the sum as (cross + W*T) + V.)
pub fn ue_quat_rotate_vector(q: FQuat, v: FVector) -> FVector {
    let t0 = v.z * q.y - v.y * q.z;
    let t1 = v.x * q.z - v.z * q.x;
    let t2 = v.y * q.x - v.x * q.y;
    let t = FVector::new(t0 + t0, t1 + t1, t2 + t2);
    let c = FVector::new(t.z * q.y - t.y * q.z, t.x * q.z - t.z * q.x, t.y * q.x - t.x * q.y);
    FVector::new(c.x + (q.w * t.x + v.x), c.y + (q.w * t.y + v.y), c.z + (q.w * t.z + v.z))
}

/// AActor::GetActorForwardVector of an upright pawn: the root's rotation quaternion (FRotator::Quaternion of (0, Yaw, 0)
/// rva=0x18b9f90, UNCONFIRMED: the root's quaternion as the movement component sets it) rotating (1, 0, 0) by
/// GetForwardVector rva=0x2fa7ca0
pub fn ue_actor_forward_yaw(yaw: f32) -> FVector {
    ue_quat_rotate_vector(FQuat::from_rotator(0.0, yaw, 0.0), FVector::new(1.0, 0.0, 0.0))
}

/// UKismetMathLibrary::MakeRotFromZX(Z, X).Yaw: FRotationMatrix::MakeFromZX rva=0x18b3d80 (NewZ = Z.GetSafeNormal, Norm =
/// X.GetSafeNormal; |(Norm | NewZ)| within SMALL_NUMBER of 1 -> Norm = |NewZ.Z| < 0.9999 (0x1440e9f8c) ? (0, 0, 1) :
/// (1, 0, 0); NewY = (NewZ ^ Norm).GetSafeNormal; NewX = NewY ^ NewZ) then FMatrix::Rotator rva=0x18bdb00: Yaw =
/// Atan2(M[0][1], M[0][0]) * 57.29578
pub fn ue_make_rot_from_zx_yaw(z: FVector, x: FVector) -> f32 {
    let nz = ue_safe_normal(z);
    let mut n = ue_safe_normal(x);
    let d = ((n.x * nz.x + n.y * nz.y) + n.z * nz.z).abs();
    if (d - 1.0).abs() <= UE_SMALL_NUMBER {
        n = if nz.z.abs() < 0.9999 { FVector::new(0.0, 0.0, 1.0) } else { FVector::new(1.0, 0.0, 0.0) };
    }
    let y = ue_safe_normal(FVector::new(n.z * nz.y - n.y * nz.z, n.x * nz.z - nz.x * n.z, n.y * nz.x - n.x * nz.y));
    let newx = FVector::new(y.y * nz.z - y.z * nz.y, y.z * nz.x - y.x * nz.z, y.x * nz.y - y.y * nz.x);
    ue_atan2(newx.y, newx.x) * UE_RAD_TO_DEG
}
// ==== end of exe float math ===========================================================================================

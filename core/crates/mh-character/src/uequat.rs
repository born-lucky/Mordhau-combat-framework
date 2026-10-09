//! UE 4.26 rotation and angle math as the shipped exe computes it (binary32, SSE), for the exe-mode LODTick port:
//! VectorSinCos, FRotator::Quaternion, FQuat::RotateVector / inverse, FRotationMatrix axes, FMath::Atan2,
//! UnwindDegrees, FInterpConstantTo, UAnimInstance::CalculateDirection, FRichCurve::Eval for linear / constant keys.
//!
//! Constants: the polynomial coefficients are read from .rdata (SinCoeff 0x144070230 / 0x144070290, CosCoeff
//! 0x144070240 / 0x1440702a0; Atan2 0x14448e348..0x14448e35c). GlobalVectorConstants (OneOverTwoPi, TwoPi, Pi, PiByTwo,
//! DEG_TO_RAD_HALF, Float360, FloatNonFractional) live in .data, filled by static initialisers at startup (the file
//! bytes are not the values): their values are UE 4.26's definitions from PI = 3.1415926535897932f, UNCONFIRMED by bytes.

use crate::ue::FVector;
use crate::uemath::v;

const PI: f32 = 3.141_592_7;
const TWO_PI: f32 = 2.0 * PI;
const HALF_PI: f32 = 0.5 * PI;
/// GlobalVectorConstants::OneOverTwoPi = 1.0f / (2.0f * PI)
const ONE_OVER_TWO_PI: f32 = 1.0 / (2.0 * PI);
/// DEG_TO_RAD = PI / 180.f
const DEG_TO_RAD: f32 = PI / 180.0;
/// GlobalVectorConstants::DEG_TO_RAD_HALF = PI / 180.f * 0.5f
const DEG_TO_RAD_HALF: f32 = (PI / 180.0) * 0.5;
/// 180 / PI as the f32 the exe multiplies by (.rdata 0x1442713d0 = 57.2957764)
pub const RAD_TO_DEG: f32 = 57.295_776;

/// VectorSinCos (one lane), as inlined in FRotator::Quaternion rva=0x18b9f90 (0x1418ba010..0x1418ba12b) and
/// FRotationMatrix::FRotationMatrix rva=0xb7e6f0: Quotient = round-to-even(A * OneOverTwoPi) (cvtps2dq), X = A - TwoPi *
/// Quotient, reflect into [-pi/2, pi/2] (sign -1 when reflected), 11-degree sine / 10-degree cosine minimax polynomials
/// evaluated as x2 * c + k (mulps then addps)
pub fn vector_sin_cos(a: f32) -> (f32, f32) {
    let q = (a * ONE_OVER_TWO_PI).round_ties_even();
    let x = a - TWO_PI * q;
    let c = if x.is_sign_negative() { -PI } else { PI }; // Pi | sign(X)
    let (x, sign) = if x.abs() > HALF_PI { (c - x, -1.0f32) } else { (x, 1.0f32) };
    let x2 = x * x;
    // sine: SinCoeff1 (2.7525562e-06, -2.3889859e-08), SinCoeff0 (1, -0.16666667, 0.008333331, -0.00019840874)
    let mut s = x2 * f32::from_bits(0xb2cd_365b) + f32::from_bits(0x3638_b88e);
    s = x2 * s + f32::from_bits(0xb950_0bf1);
    s = x2 * s + f32::from_bits(0x3c08_8886);
    s = x2 * s + f32::from_bits(0xbe2a_aaab);
    s = x2 * s + 1.0;
    let sin = s * x;
    // cosine: CosCoeff1 (2.4760495e-05, -2.6051615e-07), CosCoeff0 (1, -0.5, 0.041666638, -0.0013888378)
    let mut k = x2 * f32::from_bits(0xb48b_dd11) + f32::from_bits(0x37cf_b4c2);
    k = x2 * k + f32::from_bits(0xbab6_09aa);
    k = x2 * k + f32::from_bits(0x3d2a_aaa3);
    k = x2 * k + -0.5;
    k = x2 * k + 1.0;
    (sin, k * sign)
}

/// FQuat (X, Y, Z, W)
pub type Quat = [f32; 4];

/// VectorMod(X, 360) (FRotator::Quaternion 0x1418b9fb8..0x1418ba00d): X - 360 * trunc(X / 360) (unless |X / 360| >=
/// 2^23), clamped to [-360, 360]
fn vector_mod_360(a: f32) -> f32 {
    let div = a / 360.0;
    let t = if div.abs() >= 8_388_608.0 { div } else { div.trunc() };
    let r = a - 360.0 * t;
    let r = if r < 360.0 { r } else { 360.0 };
    if -360.0 > r {
        -360.0
    } else {
        r
    }
}

/// FRotator::Quaternion rva=0x18b9f90 (vectorised: VectorMod 360, * DEG_TO_RAD_HALF, VectorSinCos, sign-masked terms)
pub fn rotator_quaternion(pitch: f32, yaw: f32, roll: f32) -> Quat {
    let (sp, cp) = vector_sin_cos(vector_mod_360(pitch) * DEG_TO_RAD_HALF);
    let (sy, cy) = vector_sin_cos(vector_mod_360(yaw) * DEG_TO_RAD_HALF);
    let (sr, cr) = vector_sin_cos(vector_mod_360(roll) * DEG_TO_RAD_HALF);
    // LeftTerm = SignBitsLeft (+, -, +, +) ^ ((SY_CY_SY_CY * SP_SP_CP_CP) * CR)
    // RightTerm = SignBitsRight (-, -, -, +) ^ ((CY_SY_CY_SY * CP_CP_SP_SP) * SR) (0x1418ba12e..0x1418ba16f: the
    // shuffles 0x88 / 0x22 of (SY, SY, CY, CY) and (CP, CP, SP, SP) from shufps 0 of cos / sin)
    let l = [(sy * sp) * cr, -((cy * sp) * cr), (sy * cp) * cr, (cy * cp) * cr];
    let r = [-((cy * cp) * sr), -((sy * cp) * sr), -((cy * sp) * sr), (sy * sp) * sr];
    [l[0] + r[0], l[1] + r[1], l[2] + r[2], l[3] + r[3]]
}

/// FQuat::Inverse as the exe applies it (multiply by GlobalVectorConstants::QINV_SIGN_MASK (-1, -1, -1, 1))
pub fn quat_inverse(q: Quat) -> Quat {
    [q[0] * -1.0, q[1] * -1.0, q[2] * -1.0, q[3] * 1.0]
}

/// FQuat::RotateVector (as inlined in UMordhauMovementComponent::LODTick, e.g. 0x1414c6c3e..0x1414c6d04):
/// T = 2 * (Q.xyz ^ V); V' = (V + T * W) + (Q.xyz ^ T), the sums in the order the disassembly adds them
pub fn quat_rotate(q: Quat, p: FVector) -> FVector {
    let (qx, qy, qz, w) = (q[0], q[1], q[2], q[3]);
    let mut tx = qy * p.z - qz * p.y;
    let mut ty = qz * p.x - qx * p.z;
    let mut tz = qx * p.y - qy * p.x;
    tx = tx + tx;
    ty = ty + ty;
    tz = tz + tz;
    v(
        (qy * tz - qz * ty) + tx * w + p.x,
        (qz * tx - qx * tz) + ty * w + p.y,
        (qx * ty - qy * tx) + tz * w + p.z,
    )
}

/// USceneComponent::GetForwardVector / GetRightVector of a root whose rotation is FRotator(0, Yaw, 0)
/// (SetActorRotation -> FRotator::Quaternion; GetUnitAxis = RotateVector of the unit axis)
pub fn actor_axes(yaw: f32) -> (Quat, FVector, FVector) {
    let q = rotator_quaternion(0.0, yaw, 0.0);
    (q, quat_rotate(q, v(1.0, 0.0, 0.0)), quat_rotate(q, v(0.0, 1.0, 0.0)))
}

/// FRotationMatrix::FRotationMatrix rva=0xb7e6f0, rows X (forward) and Y (right): angles * DEG_TO_RAD (no VectorMod),
/// VectorSinCos; M[0] = (CP*CY, CP*SY, SP), M[1] = (SR*SP*CY - CR*SY, SR*SP*SY + CR*CY, -(SR*CP))
pub fn rotation_matrix_axes(pitch: f32, yaw: f32, roll: f32) -> (FVector, FVector) {
    let (sp, cp) = vector_sin_cos(pitch * DEG_TO_RAD);
    let (sy, cy) = vector_sin_cos(yaw * DEG_TO_RAD);
    let (sr, cr) = vector_sin_cos(roll * DEG_TO_RAD);
    let srsp = sr * sp;
    (v(cy * cp, cp * sy, sp), v(srsp * cy - cr * sy, cr * cy + srsp * sy, -(cp * sr)))
}

/// FGenericPlatformMath::Atan2 rva=0x180f930 (UE's own minimax polynomial, not the CRT)
pub fn atan2(y: f32, x: f32) -> f32 {
    let (ax, ay) = (x.abs(), y.abs());
    let y_bigger = ay > ax;
    let (t0, t1) = if y_bigger { (ay, ax) } else { (ax, ay) };
    if t0 == 0.0 {
        return 0.0;
    }
    let t3 = t1 / t0;
    let t4 = t3 * t3;
    // .rdata 0x14448e348..0x14448e35c
    let mut p = t4 * f32::from_bits(0x3bec_5a11) - f32::from_bits(0x3d0f_9abd);
    p = p * t4 + f32::from_bits(0x3da7_45af);
    p = p * t4 - f32::from_bits(0x3e08_f4dd);
    p = p * t4 + f32::from_bits(0x3e4b_54ca);
    p = p * t4 - f32::from_bits(0x3eaa_9fbe);
    p = p * t4 + 1.0;
    let mut r = t3 * p;
    if y_bigger {
        r = f32::from_bits(0x3fc9_0fdb) - r; // 0.5 * PI (.rdata 0x1440e9f90)
    }
    if !(x >= 0.0) {
        r = f32::from_bits(0x4049_0fdb) - r; // PI (.rdata 0x144034fb8)
    }
    if !(y >= 0.0) {
        r = -r;
    }
    r
}

/// FMath::UnwindDegrees rva=0x14da4c0
pub fn unwind_degrees(mut a: f32) -> f32 {
    while a > 180.0 {
        a += -360.0;
    }
    while a < -180.0 {
        a += 360.0;
    }
    a
}

/// FMath::FInterpConstantTo rva=0x18aae00: Dist^2 < SMALL_NUMBER -> Target; else Current + clamp(Dist, -dt*Speed,
/// dt*Speed)
pub fn finterp_constant_to(current: f32, target: f32, dt: f32, speed: f32) -> f32 {
    let dist = target - current;
    if !(dist * dist >= 1e-8) {
        return target;
    }
    let step = dt * speed;
    let d = if dist >= -step {
        if step < dist {
            step
        } else {
            dist
        }
    } else {
        -step
    };
    d + current
}

/// UAnimInstance::CalculateDirection rva=0x2e63780: Velocity not nearly zero (1e-4) -> FRotationMatrix(BaseRotation)
/// forward / right; N = Velocity.GetSafeNormal2D(); acosf(clamp(Forward | N, -1, 1)) * 57.2957764, negated when
/// (Right | N) < 0
pub fn calculate_direction(vel: FVector, pitch: f32, yaw: f32, roll: f32) -> f32 {
    let k = 1e-4f32;
    if !(vel.x.abs() > k || vel.y.abs() > k || vel.z.abs() > k) {
        return 0.0;
    }
    let (f, r) = rotation_matrix_axes(pitch, yaw, roll);
    let n = crate::uemath::safe_normal_2d(vel);
    let d = f.y * n.y + f.x * n.x + f.z * n.z;
    let d = if d < -1.0 { -1.0 } else if d < 1.0 { d } else { 1.0 };
    let mut a = d.acos() * RAD_TO_DEG;
    if !(r.y * n.y + r.x * n.x + r.z * n.z >= 0.0) {
        a = -a;
    }
    a
}

/// FRichCurve::Eval rva=0x3024860 / EvalForTwoKeys rva=0x3024fa0 for constant / linear keys with constant
/// extrapolation (the turn-sprint prevention curves), binary32: alpha = (t - t1) / (t2 - t1), (v2 - v1) * alpha + v1.
/// The linear branch is EvalForTwoKeys' as `rich_eval_two` reads it off the disassembly ((v2 - v1) * alpha + v1).
pub fn curve_eval(keys: &[(f32, f32, bool)], t: f32) -> f32 {
    let n = keys.len();
    if n == 0 {
        return 0.0;
    }
    if n < 2 || t <= keys[0].0 {
        return keys[0].1;
    }
    if t >= keys[n - 1].0 {
        return keys[n - 1].1;
    }
    let mut lo = 1usize;
    let mut cnt = n - 2;
    while cnt > 0 {
        let half = cnt >> 1;
        if t >= keys[lo + half].0 {
            lo = lo + half + 1;
            cnt -= half + 1;
        } else {
            cnt = half;
        }
    }
    let (a, b) = (keys[lo - 1], keys[lo]);
    let dt = b.0 - a.0;
    if dt <= 0.0 || !a.2 {
        return a.1;
    }
    let alpha = (t - a.0) / dt;
    (b.1 - a.1) * alpha + a.1
}

/// FGenericPlatformMath::Fmod rva=0x1815130: |Y| <= 1e-8 -> 0; Quotient = |X / Y| < 2^23 ? truncf(X / Y) : X / Y;
/// IntPortion = Quotient * Y, replaced by X when |IntPortion| > |X|; clamp(X - IntPortion, -|Y|, |Y|)
pub fn fmod(x: f32, y: f32) -> f32 {
    let ay = y.abs();
    if !(ay > 1e-8) {
        return 0.0;
    }
    let div = x / y;
    let q = if div.abs() >= 8_388_608.0 { div } else { div.trunc() };
    let mut ip = q * y;
    if ip.abs() > x.abs() {
        ip = x;
    }
    let r = x - ip;
    if r < -ay {
        -ay
    } else {
        minss32(r, ay)
    }
}

fn minss32(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// FRotator::ClampAxis: Fmod(Angle, 360), + 360 when negative
pub fn clamp_axis(a: f32) -> f32 {
    let r = fmod(a, 360.0);
    if r < 0.0 {
        r + 360.0
    } else {
        r
    }
}

/// FMath::FixedTurn rva=0x18abb40
pub fn fixed_turn(current: f32, desired: f32, delta_rate: f32) -> f32 {
    if delta_rate == 0.0 {
        return clamp_axis(current);
    }
    if delta_rate >= 360.0 {
        return clamp_axis(desired);
    }
    let mut result = clamp_axis(current);
    let cur = result;
    let des = clamp_axis(desired);
    let rate = delta_rate.abs();
    if cur > des {
        let d = cur - des;
        if d < 180.0 {
            result = cur - minss32(d, rate);
        } else {
            result = minss32((des + 360.0) - cur, rate) + cur;
        }
    } else {
        let d = des - cur;
        if d < 180.0 {
            result = minss32(d, rate) + cur;
        } else {
            result = cur - minss32((cur + 360.0) - des, rate);
        }
    }
    clamp_axis(result)
}

/// FMath::SinCos (scalar), as inlined in FRotator::Vector rva=0x18c4050: Quotient = trunc(v * (0.5 / PI) +- 0.5),
/// y = v - 2 PI * Quotient, reflected into [-PI/2, PI/2] (sign -1), 11 / 10-degree polynomials written k - y2 * c
pub fn scalar_sin_cos(v: f32) -> (f32, f32) {
    let qf = v * f32::from_bits(0x3e22_f983);
    let q = if v >= 0.0 { qf + 0.5 } else { qf - 0.5 };
    let q = (q as i32) as f32; // cvttss2si
    let mut y = v - q * f32::from_bits(0x40c9_0fdb);
    let mut sign = 1.0f32;
    if y > f32::from_bits(0x3fc9_0fdb) {
        y = f32::from_bits(0x4049_0fdb) - y;
        sign = -1.0;
    } else if y < f32::from_bits(0xbfc9_0fdb) {
        y = f32::from_bits(0xc049_0fdb) - y;
        sign = -1.0;
    }
    let y2 = y * y;
    let mut s = f32::from_bits(0x3638_b88e) - y2 * f32::from_bits(0x32cd_365b);
    s = s * y2 - f32::from_bits(0x3950_0bf1);
    s = s * y2 + f32::from_bits(0x3c08_8886);
    s = s * y2 - f32::from_bits(0x3e2a_aaab);
    s = s * y2 + 1.0;
    let mut c = f32::from_bits(0x37cf_b4c2) - y2 * f32::from_bits(0x348b_dd11);
    c = c * y2 - f32::from_bits(0x3ab6_09aa);
    c = c * y2 + f32::from_bits(0x3d2a_aaa3);
    c = c * y2 - 0.5;
    c = c * y2 + 1.0;
    (s * y, c * sign)
}

/// FRotator::Vector rva=0x18c4050: (CP * CY, CP * SY, SP) with Fmod(angle, 360) * (PI / 180) and the scalar SinCos;
/// the yaw reflection sign is folded into CP for X (0x1418c4241)
pub fn rotator_vector(pitch: f32, yaw: f32) -> FVector {
    let p = fmod(pitch, 360.0) * f32::from_bits(0x3c8e_fa35);
    let yw = fmod(yaw, 360.0) * f32::from_bits(0x3c8e_fa35);
    let (sp, cp) = scalar_sin_cos(p);
    // yaw: the polynomials without the sign, the sign applied to CP
    let qf = yw * f32::from_bits(0x3e22_f983);
    let q = if yw >= 0.0 { qf + 0.5 } else { qf - 0.5 };
    let q = (q as i32) as f32;
    let mut y = yw - q * f32::from_bits(0x40c9_0fdb);
    let mut cps = cp;
    if y > f32::from_bits(0x3fc9_0fdb) {
        y = f32::from_bits(0x4049_0fdb) - y;
        cps = -cp;
    } else if y < f32::from_bits(0xbfc9_0fdb) {
        y = f32::from_bits(0xc049_0fdb) - y;
        cps = -cp;
    }
    let y2 = y * y;
    let mut c = f32::from_bits(0x37cf_b4c2) - y2 * f32::from_bits(0x348b_dd11);
    c = c * y2 - f32::from_bits(0x3ab6_09aa);
    c = c * y2 + f32::from_bits(0x3d2a_aaa3);
    c = c * y2 - 0.5;
    c = c * y2 + 1.0;
    let mut s = f32::from_bits(0x3638_b88e) - y2 * f32::from_bits(0x32cd_365b);
    s = s * y2 - f32::from_bits(0x3950_0bf1);
    s = s * y2 + f32::from_bits(0x3c08_8886);
    s = s * y2 - f32::from_bits(0x3e2a_aaab);
    s = s * y2 + 1.0;
    v(c * cps, (s * y) * cp, sp)
}

/// FRotationMatrix rows (FRotationMatrix::FRotationMatrix rva=0xb7e6f0)
pub fn rotation_matrix(pitch: f32, yaw: f32, roll: f32) -> [FVector; 3] {
    let (sp, cp) = vector_sin_cos(pitch * DEG_TO_RAD);
    let (sy, cy) = vector_sin_cos(yaw * DEG_TO_RAD);
    let (sr, cr) = vector_sin_cos(roll * DEG_TO_RAD);
    let srsp = sr * sp;
    let crsp = cr * sp;
    [
        v(cy * cp, cp * sy, sp),
        v(srsp * cy - cr * sy, cr * cy + srsp * sy, -(cp * sr)),
        v(-(crsp * cy + sr * sy), cy * sr - crsp * sy, cr * cp),
    ]
}

/// FRotator::RotateVector rva=0x18bd9b0: FRotationMatrix(R).TransformVector(V) as VectorTransformVector with W = 0:
/// (W * M3 + Z * M2) + (Y * M1 + X * M0)
pub fn rotator_rotate_vector(pitch: f32, yaw: f32, roll: f32, p: FVector) -> FVector {
    let m = rotation_matrix(pitch, yaw, roll);
    let lane = |a: f32, b: f32, c: f32| (0.0 * 0.0 + p.z * c) + (p.y * b + p.x * a);
    v(lane(m[0].x, m[1].x, m[2].x), lane(m[0].y, m[1].y, m[2].y), lane(m[0].z, m[1].z, m[2].z))
}

/// An FRichCurveKey: InterpMode (0 linear, 1 constant, 2 cubic), TangentWeightMode, Time, Value, tangents
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RichKey {
    pub interp: u8,
    pub weight_mode: u8,
    pub time: f32,
    pub value: f32,
    pub arrive: f32,
    pub leave: f32,
}

/// FRichCurve::EvalForTwoKeys rva=0x3024fa0 for linear / constant / unweighted cubic keys. The weighted Bezier solver
/// (0x143042930) is not ported: no curve in extract/json outside one animation uses RCTWM_Weighted* tangents.
pub fn rich_eval_two(k1: &RichKey, k2: &RichKey, t: f32) -> f32 {
    let diff = k2.time - k1.time;
    if !(diff > 0.0) || k1.interp == 1 {
        return k1.value;
    }
    let alpha = (t - k1.time) / diff;
    let p0 = k1.value;
    let mut p3 = k2.value;
    if k1.interp == 0 {
        return (p3 - p0) * alpha + p0;
    }
    let third = f32::from_bits(0x3eaa_aaab);
    let lt3 = (diff * k1.leave) * third;
    let at3 = (diff * k2.arrive) * third;
    let p01 = lt3 * alpha + p0;
    let p1 = lt3 + p0;
    p3 = p3 - at3; // P2
    let p2 = p3;
    let p23 = at3 * alpha + p2;
    let d21 = p2 - p1;
    let p12 = d21 * alpha + p1;
    let mut a = p23 - p12;
    let mut b = p12 - p01;
    a = a + d21;
    b = b + lt3;
    a = a * alpha;
    let p012 = b * alpha + p0;
    a = a + p1; // P123
    a = a - p012;
    a = a + b;
    a = a * alpha;
    a + p0
}

/// FRichCurve::Eval rva=0x3024860 with constant extrapolation on both sides
pub fn rich_curve_eval(keys: &[RichKey], t: f32) -> f32 {
    let n = keys.len();
    if n == 0 {
        return 0.0;
    }
    if n < 2 || t <= keys[0].time {
        return keys[0].value;
    }
    if t >= keys[n - 1].time {
        return keys[n - 1].value;
    }
    let mut lo = 1usize;
    let mut cnt = n - 2;
    while cnt > 0 {
        let half = cnt >> 1;
        if t >= keys[lo + half].time {
            lo = lo + half + 1;
            cnt -= half + 1;
        } else {
            cnt = half;
        }
    }
    rich_eval_two(&keys[lo - 1], &keys[lo], t)
}

/// the scalar SinCos core of FRotator::Vector / FVector::ToOrientationQuat: (sin, cos without the reflection sign,
/// reflection sign) of an angle in radians
fn sincos_parts(v: f32) -> (f32, f32, f32) {
    let qf = v * f32::from_bits(0x3e22_f983);
    let q = if v >= 0.0 { qf + 0.5 } else { qf - 0.5 };
    let q = (q as i32) as f32;
    let mut y = v - q * f32::from_bits(0x40c9_0fdb);
    let mut sign = 1.0f32;
    if y > f32::from_bits(0x3fc9_0fdb) {
        y = f32::from_bits(0x4049_0fdb) - y;
        sign = -1.0;
    } else if y < f32::from_bits(0xbfc9_0fdb) {
        y = f32::from_bits(0xc049_0fdb) - y;
        sign = -1.0;
    }
    let y2 = y * y;
    let mut s = f32::from_bits(0x3638_b88e) - y2 * f32::from_bits(0x32cd_365b);
    s = s * y2 - f32::from_bits(0x3950_0bf1);
    s = s * y2 + f32::from_bits(0x3c08_8886);
    s = s * y2 - f32::from_bits(0x3e2a_aaab);
    s = s * y2 + 1.0;
    let mut c = f32::from_bits(0x37cf_b4c2) - y2 * f32::from_bits(0x348b_dd11);
    c = c * y2 - f32::from_bits(0x3ab6_09aa);
    c = c * y2 + f32::from_bits(0x3d2a_aaa3);
    c = c * y2 - 0.5;
    c = c * y2 + 1.0;
    (s * y, c, sign)
}

/// FVector::ToOrientationQuat rva=0x18c10a0: Yaw = Atan2(Y, X), Pitch = Atan2(Z, Sqrt(X^2 + Y^2)) (radians), half
/// angles through the scalar SinCos; X = SY * SP, Y = -(CY * SP'), Z = SY * CP, W = CY * CP' (SP', CP' negated when the
/// half yaw was reflected)
pub fn to_orientation_quat(d: FVector) -> Quat {
    let yaw = atan2(d.y, d.x);
    let pitch = atan2(d.z, (d.x * d.x + d.y * d.y).sqrt());
    let (sp, cpu, psign) = sincos_parts(pitch * 0.5);
    let cp = cpu * psign;
    let (sy, cy, ysign) = sincos_parts(yaw * 0.5);
    let (sp2, cp2) = if ysign < 0.0 { (-sp, -cp) } else { (sp, cp) };
    [sy * sp, -(cy * sp2), sy * cp, cy * cp2]
}

/// the forward (X) axis of a rotation: FQuat::RotateVector of (1, 0, 0)
pub fn quat_forward(q: Quat) -> FVector {
    quat_rotate(q, v(1.0, 0.0, 0.0))
}

/// FQuat::Rotator rva=0x18bdc80 (read off the disassembly): SingularityTest = Z * X - W * Y, YawY = (W * Z + W * Z) +
/// (X * Y + X * Y), YawX = 1 - ((Z * Z + Z * Z) + (Y * Y + Y * Y)); below -0.4999995 (.rdata 0x14449f8e0) pitch -90,
/// roll NormalizeAxis(-Yaw - Atan2(X, W) * 114.59155); above 0.4999995 (0x14449f7f4) pitch 90, roll NormalizeAxis(Yaw -
/// Atan2(X, W) * 114.59155); else pitch FastAsin(2 * SingularityTest) * RAD_TO_DEG (the inlined polynomial, .rdata
/// 0x14449f790..0x14449f838), roll Atan2(-2 * (Y * Z) - (X * W + X * W), 1 - ((X * X + X * X) + (Y * Y + Y * Y))).
/// Returns (pitch, yaw, roll).
pub fn quat_rotator(q: Quat) -> (f32, f32, f32) {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let test = x * z - y * w;
    let wz = w * z;
    let yaw_y = (wz + wz) + {
        let xy = y * x;
        xy + xy
    };
    let zz = z * z;
    let yy = y * y;
    let yaw_x = 1.0 - ((zz + zz) + (yy + yy));
    let norm = |a: f32| {
        let mut r = fmod(a, 360.0);
        if !(r >= 0.0) {
            r = r + 360.0;
        }
        if r > 180.0 {
            r = r - 360.0;
        }
        r
    };
    if test < f32::from_bits(0xbeff_ffef) {
        let yaw = atan2(yaw_y, yaw_x) * RAD_TO_DEG;
        let roll = norm(-yaw - atan2(x, w) * f32::from_bits(0x42e5_2ee0));
        return (-90.0, yaw, roll);
    }
    if test > f32::from_bits(0x3eff_ffef) {
        let yaw = atan2(yaw_y, yaw_x) * RAD_TO_DEG;
        let roll = norm(yaw - atan2(x, w) * f32::from_bits(0x42e5_2ee0));
        return (90.0, yaw, roll);
    }
    // FMath::FastAsin(2 * test)
    let a = test + test;
    let ax = a.abs();
    let root = maxss32(1.0 - ax, 0.0).sqrt();
    let mut poly = f32::from_bits(0x3bda_90c5) - ax * f32::from_bits(0x3aa5_7a2c);
    poly = poly * ax - f32::from_bits(0x3c8b_fc66);
    poly = poly * ax + f32::from_bits(0x3cfd_10f8);
    poly = poly * ax - f32::from_bits(0x3d4d_8392);
    poly = poly * ax + f32::from_bits(0x3db6_3a9e);
    poly = poly * ax - f32::from_bits(0x3e5b_bfca);
    let hp = f32::from_bits(0x3fc9_0fda);
    poly = poly * ax + hp;
    let r = root * poly;
    let asin = if a >= 0.0 { hp - r } else { r - hp };
    let pitch = asin * RAD_TO_DEG;
    let yaw = atan2(yaw_y, yaw_x) * RAD_TO_DEG;
    let xx = x * x;
    let xw = x * w;
    let yy2 = y * y;
    let num = (y * z) * -2.0 - (xw + xw);
    let den = 1.0 - ((xx + xx) + (yy2 + yy2));
    (pitch, yaw, atan2(num, den) * RAD_TO_DEG)
}

fn maxss32(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// FVector::GetSafeNormal as the exe inlines it: SizeSquared ((X * X + Y * Y) + Z * Z) == 1 -> itself; < 1e-8 ->
/// zero; else times the rsqrtss + two Newton steps InvSqrt
fn safe_normal_inline(a: FVector) -> FVector {
    let ss = (a.x * a.x + a.y * a.y) + a.z * a.z;
    if ss == 1.0 {
        a
    } else if ss < 1e-8 {
        FVector::ZERO
    } else {
        let k = crate::uemath::inv_sqrt(ss);
        v(a.x * k, a.y * k, a.z * k)
    }
}

/// FRotationMatrix::MakeFromXZ rva=0x18b28e0 (read off the disassembly): NewX = X.GetSafeNormal, Norm =
/// Z.GetSafeNormal; |(Norm.X * NewX.X + Norm.Y * NewX.Y) + Norm.Z * NewX.Z| within 1e-8 of 1 -> Norm = |NewX.Z| <
/// 0.9999 (.rdata 0x1440e9f8c) ? (0, 0, 1) : (1, 0, 0); NewY = (Norm ^ NewX).GetSafeNormal (its SizeSquared summed
/// (Y.y^2 + Y.x^2) + Y.z^2), NewZ = NewX ^ NewY. Returns the three rows.
pub fn make_from_xz(xa: FVector, za: FVector) -> [FVector; 3] {
    let nx = safe_normal_inline(xa);
    let mut nz = safe_normal_inline(za);
    let d = ((nz.x * nx.x + nz.y * nx.y) + nz.z * nx.z).abs();
    if !((d - 1.0).abs() > 1e-8) {
        nz = if nx.z.abs() >= f32::from_bits(0x3f7f_f972) { v(1.0, 0.0, 0.0) } else { v(0.0, 0.0, 1.0) };
    }
    let yx = nz.y * nx.z - nz.z * nx.y;
    let yy = nx.x * nz.z - nz.x * nx.z;
    let yz = nz.x * nx.y - nz.y * nx.x;
    let ss = (yy * yy + yx * yx) + yz * yz;
    let ny = if ss == 1.0 {
        v(yx, yy, yz)
    } else if ss < 1e-8 {
        FVector::ZERO
    } else {
        let k = crate::uemath::inv_sqrt(ss);
        v(yx * k, yy * k, yz * k)
    };
    let zx = nx.y * ny.z - ny.y * nx.z;
    let zy = ny.x * nx.z - nx.x * ny.z;
    let zz = nx.x * ny.y - nx.y * ny.x;
    [nx, ny, v(zx, zy, zz)]
}

/// FRotationMatrix(Rotator) rva=0xb7e6f0, the Y row (the angles * DEG_TO_RAD, VectorSinCos): ((SR * SP) * CY - CR *
/// SY, CR * CY + (SR * SP) * SY, -(CP * SR))
fn rotation_matrix_y_row(pitch: f32, yaw: f32, roll: f32) -> FVector {
    let (sp, cp) = vector_sin_cos(pitch * DEG_TO_RAD);
    let (sy, cy) = vector_sin_cos(yaw * DEG_TO_RAD);
    let (sr, cr) = vector_sin_cos(roll * DEG_TO_RAD);
    let srsp = sr * sp;
    v(srsp * cy - cr * sy, cr * cy + srsp * sy, -(cp * sr))
}

/// FMatrix::Rotator rva=0x18bdb00 (read off the disassembly): Yaw = Atan2(X.Y, X.X) * RAD_TO_DEG, Pitch = Atan2(X.Z,
/// Sqrt(X.Y^2 + X.X^2)) * RAD_TO_DEG, then with SY = FRotationMatrix(Pitch, Yaw, 0)'s Y row, Roll = Atan2((SY.y * Z.y
/// + SY.x * Z.x) + SY.z * Z.z, (SY.x * Y.x + SY.y * Y.y) + SY.z * Y.z) * RAD_TO_DEG. Returns (pitch, yaw, roll).
pub fn matrix_rotator(m: &[FVector; 3]) -> (f32, f32, f32) {
    let (x, y, z) = (m[0], m[1], m[2]);
    let yaw = atan2(x.y, x.x) * RAD_TO_DEG;
    let pitch = atan2(x.z, (x.y * x.y + x.x * x.x).sqrt()) * RAD_TO_DEG;
    let s = rotation_matrix_y_row(pitch, yaw, 0.0);
    let den = (s.x * y.x + s.y * y.y) + s.z * y.z;
    let num = (s.y * z.y + s.x * z.x) + s.z * z.z;
    (pitch, yaw, atan2(num, den) * RAD_TO_DEG)
}

/// FMath::FindDeltaAngleDegrees: wrap (B - A) into (-180, 180]
pub fn find_delta_angle_degrees(a: f32, b: f32) -> f32 {
    let mut d = b - a;
    if d > 180.0 {
        d = d - 360.0;
    } else if d < -180.0 {
        d = d + 360.0;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sincos_close_to_libm() {
        for a in [-3.0f32, -1.0, 0.0, 0.3, 1.5, 2.9, 7.0] {
            let (s, c) = vector_sin_cos(a);
            assert!((s - a.sin()).abs() < 2e-6 && (c - a.cos()).abs() < 2e-6, "{a}: {s} {c}");
        }
        assert_eq!(vector_sin_cos(0.0), (0.0, 1.0));
    }
    #[test]
    fn yaw_zero_axes_are_unit() {
        let (_, f, r) = actor_axes(0.0);
        assert_eq!((f.x, f.y, f.z), (1.0, 0.0, 0.0));
        assert_eq!((r.x, r.y, r.z), (0.0, 1.0, 0.0));
        let (_, f, _) = actor_axes(90.0);
        assert!((f.x).abs() < 1e-6 && (f.y - 1.0).abs() < 1e-6);
    }
    #[test]
    fn atan2_close_to_libm() {
        for (y, x) in [(1.0f32, 1.0f32), (-2.0, 0.5), (0.3, -4.0), (-1.0, -1.0), (0.0, -1.0)] {
            assert!((atan2(y, x) - y.atan2(x)).abs() < 1e-5, "{y} {x}");
        }
        assert_eq!(atan2(0.0, 0.0), 0.0);
    }
}

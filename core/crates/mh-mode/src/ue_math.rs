//! The exe's own float math for the mode / AI rules (rust-mode-ai r3; r4: the shared pieces moved to
//! mordhau_core::ue so combat / character / net / mode use one copy, re-exported here).
//!
//! Findings (Mordhau-Win64-Shipping.exe, capstone + the PDB publics; rva = 0x1000 + the PDB `0001:` offset):
//! - `sinf` / `cosf` / `acosf` are import thunks (rva 0x3e235f7 / 0x3e23603 / 0x3e23609) into
//!   `api-ms-win-crt-math-l1-1-0.dll` (ucrt.lib, Windows Kits 10.0.22000; extract/native/modules.txt Mod 2615 / 2815):
//!   the system ucrtbase.dll, not a static libm. Rust's f32 sin / cos / acos on x86_64-pc-windows-msvc call the same
//!   import (tests/trig_crt.rs: bit-equal to FFI calls on 200k inputs). Off Windows the platform libm answers.
//!   (UBTTask_Experimental::PerformExperimentalTask calls a static `__libm_sse2_sincosf_`: an Intel libm copy, not
//!   on any ported path.)
//! - The gameplay paths use UE's own polynomials, ported in mordhau_core::ue: FMath::SinCos (`sin_cos_scalar`,
//!   FRotator::Vector rva=0x18c4050), VectorSinCos (`vector_sin_cos`, FRotationTranslationMatrix rva=0xb7e6f0),
//!   FGenericPlatformMath::Atan2 (`ue_atan2`, rva=0x180f930), FMath::Fmod (`ue_fmod`, rva=0x1815130), InvSqrt
//!   (`ue_inv_sqrt`: rsqrtss + 2 Newton steps, exact on the same CPU vendor), ClampAngle / FindDeltaAngleDegrees /
//!   RotateVector / GetForwardVector / MakeRotFromZX (`ue_clamp_angle` ...).
//! The GDScript reference evaluates these in f64: `Precision::Reference` keeps that; `Precision::Exe` uses these
//! (tasks.rs: CalculateAngle2D, the circle / side-step RotateVector, GetForwardVector, BackOff's MakeRotFromZX).

pub use mordhau_core::ue::{
    sin_cos_scalar as sin_cos, ue_atan2 as atan2, ue_clamp_angle as clamp_angle, ue_find_delta_angle_degrees as find_delta_angle,
    ue_fmod as fmod, ue_inv_sqrt as inv_sqrt, ue_yaw_axes, vector_sin_cos, UE_DEG_TO_RAD, UE_RAD_TO_DEG,
};

/// FRotator::Vector rva=0x18c4050 (mordhau_core::ue::rotator_vector) as an array
pub fn rotator_vector(pitch: f32, yaw: f32) -> [f32; 3] {
    let v = mordhau_core::ue::rotator_vector(pitch, yaw);
    [v.x, v.y, v.z]
}

/// the X and Y axes of FRotationMatrix(FRotator(0, yaw, 0)) as 2D pairs
pub fn yaw_axes(yaw: f32) -> ([f32; 2], [f32; 2]) {
    let (x, y) = ue_yaw_axes(yaw);
    ([x.x, x.y], [y.x, y.y])
}

/// FMath::Acos(float) = the acosf import (ucrtbase.dll)
pub fn acos(x: f32) -> f32 {
    x.acos()
}

/// UMordhauUtilityLibrary::CalculateAngle2D rva=0x1616170, the exe instruction sequence: every |component| <=
/// 1e-4 (.rdata 0x144022350) -> 0; forward / right = the X / Y axes of FRotationMatrix(FRotator(0, yaw, 0));
/// d.GetSafeNormal2D (X^2 + Y^2 == 1 -> (X, Y, 0); < 1e-8 (0x144014a88) -> 0; else * InvSqrt); dot clamped to
/// [-1, 1] (comiss -1 / minss 1); acosf * 57.29578; right . n < 0 -> negated
pub fn calculate_angle_2d(d: [f32; 3], yaw: f32) -> f32 {
    let e = 1e-4f32;
    if d[0].abs() <= e && d[1].abs() <= e && d[2].abs() <= e {
        return 0.0;
    }
    let (fwd, right) = yaw_axes(yaw);
    let (x, y) = (d[0], d[1]);
    let ss = x * x + y * y;
    let (nx, ny) = if ss == 1.0 {
        (x, y)
    } else if ss >= 1e-8 {
        let s = inv_sqrt(ss);
        (x * s, y * s)
    } else {
        (0.0, 0.0)
    };
    // (fy * ny + fx * nx) + fz * nz with fz = nz = 0
    let dot = fwd[1] * ny + fwd[0] * nx + 0.0 * 0.0;
    let dot = if dot < -1.0 { -1.0 } else { dot.min(1.0) };
    let a = acos(dot) * UE_RAD_TO_DEG;
    let side = right[1] * ny + right[0] * nx + 0.0 * 0.0;
    if side >= 0.0 {
        a
    } else {
        -a
    }
}

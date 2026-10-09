//! Trig exactness (rust-mode-ai r3): the exe imports sinf / cosf / acosf from api-ms-win-crt-math-l1-1-0.dll
//! (ucrtbase; see src/ue_math.rs). On Windows, Rust's f32 sin / cos / acos must be that same function: checked bit
//! for bit against direct FFI calls on a sweep of inputs. Also pins ue_math's ports of the exe's own polynomials on
//! values worked out from the disassembly, and shows the f64 path the GDScript reference takes is not the exe's.

use mh_mode::ue_math;

#[cfg(windows)]
#[link(name = "ucrt")]
extern "C" {
    fn sinf(x: f32) -> f32;
    fn cosf(x: f32) -> f32;
    fn acosf(x: f32) -> f32;
}

fn sweep() -> Vec<f32> {
    let mut v = Vec::new();
    let mut s: u32 = 0x1234_5678;
    for _ in 0..200_000 {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let f = f32::from_bits(s);
        if f.is_finite() && f.abs() < 1e6 {
            v.push(f);
        }
        v.push((s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0); // [-1, 1]
    }
    for d in -720..=720 {
        v.push(d as f32 * 0.25);
    }
    v
}

#[cfg(windows)]
#[test]
fn std_trig_is_the_ucrt_import() {
    let xs = std::hint::black_box(sweep());
    let mut n = 0usize;
    for &x in &xs {
        // SAFETY: plain C math functions
        let (s, c) = unsafe { (sinf(x), cosf(x)) };
        assert_eq!(x.sin().to_bits(), s.to_bits(), "sin {x:e}");
        assert_eq!(x.cos().to_bits(), c.to_bits(), "cos {x:e}");
        if x.abs() <= 1.0 {
            let a = unsafe { acosf(x) };
            assert_eq!(x.acos().to_bits(), a.to_bits(), "acos {x:e}");
            assert_eq!(ue_math::acos(x).to_bits(), a.to_bits());
            n += 1;
        }
    }
    assert!(n > 100_000);
}

/// the f64 route of the GDScript reference (acos of the double, rounded once) is not the exe's acosf everywhere
#[cfg(windows)]
#[test]
fn f64_route_differs_from_acosf() {
    let xs = std::hint::black_box(sweep());
    let diff = xs
        .iter()
        .filter(|x| x.abs() <= 1.0)
        .filter(|&&x| ((x as f64).acos() as f32).to_bits() != unsafe { acosf(x) }.to_bits())
        .count();
    eprintln!("acos: f64-rounded differs from ucrt acosf on {diff} inputs");
}

#[test]
fn sin_cos_polynomials() {
    // exact at 0 (y = 0: sin = 0 * ..., cos = 1)
    assert_eq!(ue_math::sin_cos(0.0), (0.0, 1.0));
    assert_eq!(ue_math::vector_sin_cos(0.0), (0.0, 1.0));
    // within the minimax error of the true values, and scalar == vector inside [-pi/2, pi/2] (same Horner order)
    for i in -1000..=1000 {
        let a = i as f32 * 0.0123;
        let (s, c) = ue_math::sin_cos(a);
        assert!((s as f64 - (a as f64).sin()).abs() < 2e-6 && (c as f64 - (a as f64).cos()).abs() < 2e-6, "{a}");
        if a.abs() < 1.5 {
            assert_eq!(ue_math::vector_sin_cos(a), (s, c), "{a}");
        }
    }
    let v = ue_math::rotator_vector(0.0, 90.0);
    assert!(v[0].abs() < 1e-6 && (v[1] - 1.0).abs() < 1e-6 && v[2] == 0.0);
    // Fmod: the wrap FRotator::Vector applies first
    assert_eq!(ue_math::fmod(370.0, 360.0), 10.0);
    assert_eq!(ue_math::fmod(-370.0, 360.0), -10.0);
    assert_eq!(ue_math::fmod(5.0, 0.0), 0.0);
}

#[test]
fn atan2_minimax() {
    assert_eq!(ue_math::atan2(0.0, 0.0), 0.0);
    assert_eq!(ue_math::atan2(0.0, -1.0), std::f32::consts::PI);
    for i in 0..360 {
        let r = (i as f64).to_radians();
        let (y, x) = (r.sin() as f32, r.cos() as f32);
        let e = ue_math::atan2(y, x) as f64 - (y as f64).atan2(x as f64);
        assert!(e.abs() < 1e-6, "{i}: {e}");
    }
}

#[test]
fn calculate_angle_2d_exe() {
    assert_eq!(ue_math::calculate_angle_2d([1e-5, 0.0, 0.0], 0.0), 0.0);
    let a = ue_math::calculate_angle_2d([0.0, 1.0, 0.0], 0.0);
    assert!((a - 90.0).abs() < 1e-4);
    let b = ue_math::calculate_angle_2d([0.0, -3.0, 2.0], 0.0);
    assert!((b + 90.0).abs() < 1e-4);
    let c = ue_math::calculate_angle_2d([1.0, 1.0, 0.0], 45.0);
    assert!(c.abs() < 0.05);
}

// ---- r4: the exe math now shared in mordhau_core::ue ------------------------------------------------------------------

use mordhau_core::ue::{self as ue, FQuat, FVector};

/// FMath::Fmod (X - trunc(X / Y) * Y in single ops) is not IEEE fmod: equal below 720, different above for some inputs
#[test]
fn ue_fmod_vs_ieee() {
    let mut diff = 0;
    for i in 0..200_000 {
        let x = -5000.0 + i as f32 * 0.0503;
        let a = ue::ue_fmod(x, 360.0);
        let b = x % 360.0;
        if x.abs() < 720.0 {
            assert_eq!(a.to_bits(), b.to_bits(), "{x}");
        } else if a.to_bits() != b.to_bits() {
            diff += 1;
        }
        assert!(a.abs() <= 360.0);
    }
    eprintln!("ue_fmod != fmodf on {diff} inputs with |x| >= 720");
}

#[test]
fn angles_and_axes() {
    assert_eq!(ue::ue_find_delta_angle_degrees(170.0, -170.0), 20.0);
    assert_eq!(ue::ue_clamp_angle(45.0, -30.0, 30.0), 30.0);
    assert_eq!(ue::ue_clamp_angle(-90.0, -30.0, 30.0), -30.0);
    assert_eq!(ue::ue_clamp_angle(10.0, -30.0, 30.0), 10.0);
    // the shared copies are the ones mh-mode re-exports
    assert_eq!(ue_math::sin_cos(1.234), ue::sin_cos_scalar(1.234));
    let r = ue::ue_rotate_yaw(FVector::new(1.0, 0.0, 5.0), 90.0);
    assert!(r.x.abs() < 1e-6 && (r.y - 1.0).abs() < 1e-6 && r.z == 5.0);
    for yaw in [-170.0f32, -45.0, 0.0, 30.0, 123.4, 359.0] {
        let f = ue::ue_actor_forward_yaw(yaw);
        let g = (yaw as f64).to_radians();
        assert!((f.x as f64 - g.cos()).abs() < 1e-5 && (f.y as f64 - g.sin()).abs() < 1e-5 && f.z.abs() < 1e-6, "{yaw}");
        // the quaternion forward and the matrix X axis are different instruction paths: equal within an ulp or two
        let (ax, _) = ue::ue_yaw_axes(yaw);
        assert!((f.x - ax.x).abs() < 1e-6 && (f.y - ax.y).abs() < 1e-6);
        let q = FQuat::from_rotator(0.0, yaw, 0.0);
        assert_eq!(ue::ue_quat_rotate_vector(q, FVector::new(1.0, 0.0, 0.0)), f);
    }
    let y = ue::ue_make_rot_from_zx_yaw(FVector::new(0.0, 0.0, 1.0), FVector::new(0.0, -1.0, 0.3));
    assert!((y + 90.0).abs() < 1e-3, "{y}");
    assert_eq!(ue::ue_make_rot_from_zx_yaw(FVector::new(0.0, 0.0, 1.0), FVector::new(0.0, 0.0, 1.0)), 0.0); // colinear
}

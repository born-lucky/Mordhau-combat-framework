//! Projectile flight and collision (projectile.rs) with the game's classes: the longbow / bow arrow
//! (BP_ArrowProjectile), the crossbow bolt (BP_BoltProjectile), the javelin (BP_ThrownJavelinProjectile). Data:
//! extract/json (the game's, ignored path); SKIP (pass) without it.

use mh_character::projectile::{Projectile, ProjectileCfg, ProjectileTarget, TargetBody};
use mh_character::uemath::{dot, size, v};
use mh_character::uequat::{quat_forward, rotator_quaternion, to_orientation_quat};
use mh_character::world::BoxWorld;
use std::path::PathBuf;

const G: f32 = -980.0;

fn read(rel: &str) -> Option<String> {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    std::fs::read_to_string(dir.join("../../../extract/json/Mordhau/Content/Mordhau/Blueprints").join(rel)).ok()
}

fn cfg(chain: &[&str]) -> Option<ProjectileCfg> {
    let j: Option<Vec<String>> = chain.iter().map(|c| read(c)).collect();
    let j = j?;
    let r: Vec<&str> = j.iter().map(|s| s.as_str()).collect();
    Some(ProjectileCfg::from_json_chain(&r).unwrap())
}

fn arrow() -> Option<ProjectileCfg> {
    cfg(&["Equipment/Ranged/BP_ArrowProjectile.json", "Equipment/Ranged/BP_MissileProjectile.json", "BP_MordhauProjectile.json"])
}
fn bolt() -> Option<ProjectileCfg> {
    cfg(&["Equipment/Ranged/BP_BoltProjectile.json", "Equipment/Ranged/BP_MissileProjectile.json", "BP_MordhauProjectile.json"])
}
fn javelin() -> Option<ProjectileCfg> {
    cfg(&["Equipment/Weapons/BP_ThrownJavelinProjectile.json", "Equipment/Weapons/BP_ThrownWeaponProjectile.json", "BP_MordhauProjectile.json"])
}

fn empty() -> BoxWorld {
    BoxWorld::new()
}

#[test]
fn class_data() {
    let (Some(a), Some(b), Some(j)) = (arrow(), bolt(), javelin()) else { return eprintln!("SKIP: no extract/json") };
    assert_eq!((a.initial_speed, a.max_speed, a.gravity_scale), (5000.0, 10000.0, 1.0));
    assert_eq!(a.box_extent, v(37.5, 1.0, 1.0));
    assert_eq!((b.initial_speed, b.max_speed, b.gravity_scale), (6700.0, 10000.0, 0.5));
    assert_eq!((j.initial_speed, j.max_speed, j.gravity_scale), (3000.0, 3000.0, 1.1));
    assert_eq!(j.box_extent, v(25.0, 3.5, 3.5));
    assert!(a.rotation_follows_velocity && !a.should_bounce);
    assert_eq!(a.will_sticky_on, vec![2, 6, 5, 9, 10, 8, 1]);
    assert_eq!(j.will_sticky_on, vec![1, 2]);
    assert_eq!(b.damage, vec![60.0, 50.0, 42.0, 40.0]);
}

/// FVector::ToOrientationQuat agrees with FRotator(pitch, yaw, 0).Quaternion() to rounding, and points along the vector
#[test]
fn orientation_quat() {
    for (p, y) in [(0.0f32, 0.0f32), (10.0, 30.0), (-45.0, 170.0), (60.0, -100.0)] {
        let r = rotator_quaternion(p, y, 0.0);
        let f = quat_forward(r);
        let q = to_orientation_quat(v(f.x * 300.0, f.y * 300.0, f.z * 300.0));
        for i in 0..4 {
            assert!((q[i] - r[i]).abs() < 2e-6, "{p} {y}: {q:?} vs {r:?}");
        }
    }
}

/// Velocity Verlet with constant gravity is exact: z(t) = v0z t + g t^2 / 2 (f32 rounding aside); the speed stays
/// under MaxSpeed; the box turns with the velocity
#[test]
fn ballistic_flight() {
    let (Some(a), Some(b)) = (arrow(), bolt()) else { return };
    let w = empty();
    for (c, scale) in [(a, 1.0f32), (b, 0.5)] {
        let mut p = Projectile::fire(c.clone(), v(0.0, 0.0, 0.0), 10.0, 0.0, 0.0, 0.0);
        let v0 = p.velocity;
        assert!((size(v0) - c.initial_speed).abs() < 0.01);
        let dt = 1.0f32 / 60.0;
        let mut t = 0.0f64;
        for _ in 0..90 {
            p.tick(&w, &[], dt, G);
            t += dt as f64;
            let g = (G * scale) as f64;
            let z = v0.z as f64 * t + 0.5 * g * t * t;
            let x = v0.x as f64 * t;
            assert!((p.location.z as f64 - z).abs() < 0.05 + 2e-6 * x.abs(), "{}: z {} vs {z}", c.class, p.location.z);
            assert!((p.location.x as f64 - x).abs() < 0.05 + 2e-6 * x.abs());
            let f = quat_forward(p.quat);
            let vn = size(p.velocity);
            // the rotation follows the velocity at the start of the last sub-step
            assert!(dot(f, p.velocity) / vn > 0.999, "{}: forward {f:?} vel {:?}", c.class, p.velocity);
        }
    }
}

/// MaxSpeed clamps: the javelin leaves at its MaxSpeed (3000) and gravity cannot speed it up
#[test]
fn javelin_speed_is_capped() {
    let Some(j) = javelin() else { return };
    let w = empty();
    let mut p = Projectile::fire(j, v(0.0, 0.0, 0.0), -30.0, 45.0, 0.0, 0.0);
    for _ in 0..120 {
        p.tick(&w, &[], 1.0 / 60.0, G);
        assert!(size(p.velocity) <= 3000.0 * (1.0 + 1e-6), "speed {}", size(p.velocity));
    }
}

/// a wall stops the flight (bShouldBounce clear -> StopSimulating) at its face; the hit is the sweep's last, blocking
/// one; a character body in front of the wall is hit first (overlap), in time order
#[test]
fn wall_and_body_hits() {
    let Some(a) = arrow() else { return };
    let mut w = BoxWorld::new();
    w.add_box(v(3000.0, -500.0, -500.0), v(3100.0, 500.0, 500.0));
    let body = ProjectileTarget { id: 7, bodies: vec![TargetBody { bone: "spine_03".into(), a: v(1500.0, 0.0, -40.0), b: v(1500.0, 0.0, 40.0), radius: 20.0, surface: 1 }] };
    let mut p = Projectile::fire(a, v(0.0, 0.0, 0.0), 0.0, 0.0, 0.0, 0.0);
    let mut all = Vec::new();
    for _ in 0..60 {
        all.extend(p.tick(&w, std::slice::from_ref(&body), 1.0 / 60.0, 0.0));
        if !p.simulating {
            break;
        }
    }
    assert!(!p.simulating, "never stopped");
    assert!(p.location.x <= 3000.0 && p.location.x > 2999.0, "x {}", p.location.x);
    let first = &all[0];
    assert_eq!(first.target, Some((7, "spine_03".to_string())));
    assert!(!first.blocking);
    // the box front (half length 37.5) reaches the capsule (radius 20) first: centre at 1500 - 20 - 37.5
    assert!((first.location.x - (1500.0 - 20.0 - 37.5)).abs() < 0.01, "body hit at {:?}", first.location);
    assert!((first.impact_point.x - 1480.0).abs() < 0.01, "impact {:?}", first.impact_point);
    let last = all.last().unwrap();
    assert!(last.target.is_none() && last.blocking);
    assert!(p.will_sticky(1) && !p.will_sticky(0));
    // stick into the wall: the projectile stays where it hit and stops
    let hit = last.clone();
    p.stick(&hit, v(0.0, 0.0, 0.0));
    assert!(p.terminated);
    assert_eq!(p.location, hit.impact_point);
    let before = p.location;
    p.tick(&w, &[], 1.0 / 60.0, G);
    assert_eq!(p.location, before);
}

/// the longbow arrow's range before landing on flat ground when fired 15 degrees up from 150 cm (a reference number
/// for the parity sheet): the ballistic range in vacuum, from the same integration
#[test]
fn arrow_range_on_flat_ground() {
    let Some(a) = arrow() else { return };
    let mut w = BoxWorld::new();
    w.add_box(v(-50000.0, -50000.0, -200.0), v(50000.0, 50000.0, 0.0));
    let mut p = Projectile::fire(a, v(0.0, 0.0, 150.0), 15.0, 0.0, 0.0, 0.0);
    let mut n = 0;
    while p.simulating && n < 60 * 10 {
        p.tick(&w, &[], 1.0 / 60.0, G);
        n += 1;
    }
    let v0 = 5000.0f64;
    let th = 15f64.to_radians();
    let (vx, vz) = (v0 * th.cos(), v0 * th.sin());
    let tl = (vz + (vz * vz + 2.0 * 980.0 * 150.0).sqrt()) / 980.0;
    let range = vx * tl;
    println!("arrow 15 deg from 150 cm: {:.0} cm (vacuum {range:.0})", p.location.x);
    assert!((p.location.x as f64 - range).abs() < 5.0, "{} vs {range}", p.location.x);
}

/// FQuat::Rotator (the exe's, with its FastAsin) inverts FRotator::Quaternion to rounding away from the poles, and
/// takes the singular branches at +-90 pitch; FMatrix::Rotator(MakeFromXZ) gives the rotator whose X / Z axes are the
/// given ones
#[test]
fn quat_and_matrix_rotators() {
    use mh_character::uequat::{make_from_xz, matrix_rotator, quat_rotator};
    for &(p, y, r) in &[(0.0f32, 0.0f32, 0.0f32), (10.0, 30.0, 0.0), (-45.0, 170.0, 20.0), (60.0, -100.0, -35.0), (89.0, 45.0, 10.0)] {
        let (p2, y2, r2) = quat_rotator(rotator_quaternion(p, y, r));
        assert!((p2 - p).abs() < 0.02 && (y2 - y).abs() < 0.02 && (r2 - r).abs() < 0.02, "{p} {y} {r} -> {p2} {y2} {r2}");
    }
    let (p, _, _) = quat_rotator(rotator_quaternion(90.0, 30.0, 0.0));
    assert_eq!(p, 90.0);
    // a wall facing -X: forward = +X, up = Z -> (0, 0, 0); facing down onto the floor: pitch -90
    let (p, y, r) = matrix_rotator(&make_from_xz(v(1.0, 0.0, 0.0), v(0.0, 0.0, 1.0)));
    assert_eq!((p, y, r), (0.0, 0.0, 0.0));
    let (p, _, _) = matrix_rotator(&make_from_xz(v(0.0, 0.0, -1.0), v(0.0, 0.0, 1.0)));
    assert!((p + 90.0).abs() < 1e-3, "{p}");
    let (p, y, _) = matrix_rotator(&make_from_xz(v(0.0, 1.0, 0.0), v(0.0, 0.0, 1.0)));
    assert!(p.abs() < 1e-4 && (y - 90.0).abs() < 1e-4, "{p} {y}");
}

/// AttachProjectile's surface blend: arrows keep their flight rotation (BP_MissileProjectile StickySurfacePitchBlend 0,
/// YawBlend 0 from the ctor); a class with pitch blend 1 (the AMordhauProjectile ctor value) takes the surface pitch
/// (-90 into a floor) and keeps its yaw and roll
#[test]
fn stick_blends() {
    let Some(a) = arrow() else { return };
    assert_eq!((a.sticky_surface_pitch_blend, a.sticky_surface_yaw_blend), (0.0, 0.0));
    let hit = mh_character::projectile::ProjectileHit { target: None, time: 1.0, location: v(0.0, 0.0, 0.0), impact_point: v(10.0, 10.0, 0.0), impact_normal: v(0.0, 0.0, 1.0), surface: 0, blocking: true };
    let mut p = Projectile::fire(a.clone(), v(0.0, 0.0, 0.0), -30.0, 40.0, 0.0, 0.0);
    p.stick(&hit, v(0.0, 0.0, 0.0));
    let (pp, yy, _) = mh_character::uequat::quat_rotator(p.quat);
    assert!((pp + 30.0).abs() < 0.05 && (yy - 40.0).abs() < 0.05, "arrow {pp} {yy}");
    let mut c = a;
    c.sticky_surface_pitch_blend = 1.0;
    let mut p = Projectile::fire(c, v(0.0, 0.0, 0.0), -30.0, 40.0, 0.0, 0.0);
    p.stick(&hit, v(0.0, 0.0, 0.0));
    let (pp, yy, _) = mh_character::uequat::quat_rotator(p.quat);
    assert!((pp + 90.0).abs() < 0.05, "blend 1 pitch {pp} yaw {yy}");
}

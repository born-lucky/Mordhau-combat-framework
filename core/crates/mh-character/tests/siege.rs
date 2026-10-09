//! Siege engines (siege.rs): the ballista's aim (turn / look limits and rate caps, AddTurnDegrees around the spawn
//! yaw, PhysicsRotation), the shooter stage machines (turret: spawn on fire, kickback, recovery / reload timers; arm:
//! adjust, release, FireSocket spawn with the arm value), the catapult launch speed, ClampAngle, the legacy
//! AVehicleBase net smoothing. Blueprint data: extract/json (the game's, ignored path); those parts SKIP without it.

use mh_character::projectile::{Projectile, ProjectileCfg};
use mh_character::siege::*;
use mh_character::uemath::{size, v};
use mh_character::uequat::{rotator_quaternion, Quat};
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;

fn bp(rel: &str) -> Option<String> {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    std::fs::read_to_string(dir.join("../../../extract/json/Mordhau/Content/Mordhau/Blueprints").join(rel)).ok()
}

fn ballista(yaw: f32) -> Option<SiegeEngine> {
    let j = bp("Interactables/SiegeEngines/BP_Ballista.json")?;
    let mut s = SiegeEngine::from_bp_json(&j, yaw).unwrap();
    s.driven = true;
    s.locally_controlled = true;
    Some(s)
}

fn catapult() -> Option<SiegeEngine> {
    let j = bp("Interactables/SiegeEngines/BP_Catapult.json")?;
    let mut s = SiegeEngine::from_bp_json(&j, 0.0).unwrap();
    s.driven = true;
    s.locally_controlled = true;
    Some(s)
}

#[test]
fn clamp_angle_matches_ue() {
    assert_eq!(clamp_angle(100.0, -45.0, 45.0), 45.0);
    assert_eq!(clamp_angle(-100.0, -45.0, 45.0), -45.0);
    assert_eq!(clamp_angle(350.0, -45.0, 45.0), -10.0);
    assert_eq!(clamp_angle(30.0, -45.0, 45.0), 30.0);
    // a range across 0 / 360 given in [0, 360)
    assert_eq!(clamp_angle(10.0, 315.0, 405.0), 10.0);
    assert_eq!(clamp_angle(90.0, 315.0, 405.0), 45.0);
    assert_eq!(clamp_angle(200.0, 315.0, 405.0), -45.0);
}

#[test]
fn ballista_data() {
    let Some(b) = ballista(0.0) else { return eprintln!("SKIP: no extract/json") };
    assert_eq!((b.aim.look_up_limit, b.aim.look_down_limit, b.aim.turn_limit), (20.0, 45.0, 45.0));
    assert_eq!((b.aim.turn_rate_cap, b.aim.look_up_rate_cap), (85.0, 60.0));
    let s = &b.shooter;
    assert!(!s.is_arm_shooter());
    assert_eq!((s.releasing_stage_duration, s.recovery_stage_duration, s.reloading_stage_duration), (0.0, 0.5, 4.5));
    assert_eq!(s.projectile_socket_name, "BoltSocket");
}

/// Turn input is capped at TurnRateCap 85 deg/s (TurnCapRemaining refilled to at most 85 * 0.0333 per LODTick), the
/// control yaw is held within SpawnTurnValue +/- TurnLimit 45, and the actor follows it
#[test]
fn ballista_turn_caps_and_limit() {
    let Some(mut b) = ballista(30.0) else { return eprintln!("SKIP: no extract/json") };
    let inp = SiegeInput { turn: 10.0, ..Default::default() };
    // a quarter second of hard turning: at most 85 * 0.25 deg + one frame's remaining (the limit is 45 away)
    let y0 = b.aim.yaw;
    for _ in 0..15 {
        b.frame(DT, &inp);
    }
    let turned = b.aim.yaw - y0;
    assert!(turned > 15.0 && turned <= 85.0 * 0.25 + 85.0 * 0.0333 + 1e-3, "{turned}");
    for _ in 0..120 {
        b.frame(DT, &inp);
    }
    assert_eq!(b.aim.control_yaw, 75.0, "spawn 30 + limit 45");
    assert!((b.aim.yaw - 75.0).abs() < 1e-3, "{}", b.aim.yaw);
    // the other way past the lower bound
    let inp = SiegeInput { turn: -10.0, ..Default::default() };
    for _ in 0..240 {
        b.frame(DT, &inp);
    }
    assert_eq!(b.aim.control_yaw, -15.0);
    assert!(normalize_axis_vm(b.aim.yaw + 15.0).abs() < 1e-3, "{}", b.aim.yaw);
}

#[test]
fn ballista_look_limits_and_cap() {
    let Some(mut b) = ballista(0.0) else { return eprintln!("SKIP: no extract/json") };
    b.frame(DT, &SiegeInput { look_up: 100.0, ..Default::default() });
    // the first frame's remaining cap is what one LODTick refilled before (0 at spawn): nothing moves yet
    assert_eq!(b.aim.look_up_value, 0.0);
    for _ in 0..120 {
        b.frame(DT, &SiegeInput { look_up: 100.0, ..Default::default() });
    }
    assert_eq!(b.aim.look_up_value, 20.0);
    for _ in 0..240 {
        b.frame(DT, &SiegeInput { look_up: -100.0, ..Default::default() });
    }
    assert_eq!(b.aim.look_up_value, -45.0);
}

/// Fire: the bolt spawns at BoltSocket in the same frame (releasing 0 -> recovery), the kickback raises the aim by
/// WeaponKickBackLookUp 2 (absolute: no cap), recovery 0.5 s, reload 4.5 s, then Loaded; fire is ignored meanwhile
#[test]
fn ballista_fire_cycle() {
    let Some(mut b) = ballista(0.0) else { return eprintln!("SKIP: no extract/json") };
    for _ in 0..30 {
        b.frame(DT, &SiegeInput::default());
    }
    let ev = b.frame(DT, &SiegeInput { fire: true, ..Default::default() });
    assert!(ev.contains(&ShooterEvent::Spawn { fire: false, socket: "BoltSocket".into(), arm: None }), "{ev:?}");
    assert!(ev.contains(&ShooterEvent::KickBackLookUp(2.0)));
    assert_eq!(b.aim.look_up_value, 2.0);
    assert_eq!(b.shooter.weapon_state, weapon_state::RECOVERY);
    let mut t = 0.0f32;
    let mut loading_at = None;
    let mut loaded_at = None;
    for _ in 0..400 {
        let ev = b.frame(DT, &SiegeInput { fire: true, ..Default::default() });
        t += DT;
        assert!(!ev.iter().any(|e| matches!(e, ShooterEvent::Spawn { .. })) || loaded_at.is_some());
        if ev.contains(&ShooterEvent::State(weapon_state::LOADING)) {
            loading_at = Some(t);
        }
        if ev.contains(&ShooterEvent::State(weapon_state::LOADED)) {
            loaded_at = Some(t);
            break;
        }
    }
    let (l, d) = (loading_at.unwrap(), loaded_at.unwrap());
    assert!((l - 0.5).abs() <= DT + 1e-4, "{l}");
    assert!((d - l - 4.5).abs() <= DT + 1e-4, "{d} {l}");
}

/// The ballista bolt: BP_BallistaProjectile 10000 cm/s straight from the socket, its box overlaps WorldDynamic
/// (only WorldStatic-type bodies stop it)
#[test]
fn ballista_bolt_cfg() {
    let chain: Option<Vec<String>> = ["Interactables/SiegeEngines/BP_BallistaProjectile.json", "BP_MordhauProjectile.json"].iter().map(|p| bp(p)).collect();
    let Some(chain) = chain else { return eprintln!("SKIP: no extract/json") };
    let r: Vec<&str> = chain.iter().map(|s| s.as_str()).collect();
    let c = ProjectileCfg::from_json_chain(&r).unwrap();
    assert_eq!((c.initial_speed, c.max_speed), (10000.0, 10000.0));
    assert_eq!(c.box_extent.x, 80.0);
    assert!(c.box_responses.contains(&("WorldDynamic".to_string(), 1)));
    assert!(c.box_responses.contains(&("Projectile".to_string(), 0)));
    let q: Quat = rotator_quaternion(10.0, 30.0, 0.0);
    let p = Projectile::fire_quat(c, v(0.0, 0.0, 0.0), q, 0.0);
    assert!((size(p.velocity) - 10000.0).abs() < 0.05);
}

#[test]
fn catapult_arm_and_fire() {
    let Some(mut c) = catapult() else { return eprintln!("SKIP: no extract/json") };
    assert!(c.shooter.is_arm_shooter());
    assert_eq!((c.aim.look_up_limit, c.aim.look_down_limit, c.aim.turn_limit), (40.0, 60.0, -1.0));
    assert_eq!((c.shooter.releasing_stage_duration, c.shooter.recovery_stage_duration, c.shooter.reloading_stage_duration), (0.2, 4.0, 6.0));
    // lower the arm (more power): +16 per press, presses 0.4 s apart, capped at ArmAdjustmentMax 80
    for _ in 0..30 {
        c.frame(DT, &SiegeInput::default());
    }
    c.frame(DT, &SiegeInput { lower_arm: true, ..Default::default() });
    assert_eq!(c.shooter.arm_value(), 16);
    c.frame(DT, &SiegeInput { lower_arm: true, ..Default::default() });
    assert_eq!(c.shooter.arm_value(), 16, "debounced");
    for _ in 0..10 {
        for _ in 0..30 {
            c.frame(DT, &SiegeInput::default());
        }
        c.frame(DT, &SiegeInput { lower_arm: true, ..Default::default() });
    }
    assert_eq!(c.shooter.arm_value(), 80);
    // the arm target creeps toward 80 * (1 - 0.5) * 0.01 + 0.5 = 0.9 at 0.125 / s
    let a0 = c.shooter.arm_target();
    c.frame(DT, &SiegeInput::default());
    assert!((c.shooter.arm_target() - a0 - 0.125 * DT).abs() < 1e-5 || c.shooter.arm_target() == 0.9);
    // fire: releasing 0.2 s, then the stone spawns at FireSocket with the arm value
    c.frame(DT, &SiegeInput { fire: true, ..Default::default() });
    assert_eq!(c.shooter.weapon_state, weapon_state::RELEASING);
    let mut spawned = None;
    let mut t = 0.0;
    for _ in 0..30 {
        t += DT;
        for e in c.frame(DT, &SiegeInput::default()) {
            if let ShooterEvent::Spawn { socket, arm, .. } = e {
                spawned = Some((t, socket, arm));
            }
        }
        if spawned.is_some() {
            break;
        }
    }
    let (t, socket, arm) = spawned.expect("spawned");
    assert!((t - 0.2).abs() <= DT + 1e-4, "{t}");
    assert_eq!((socket.as_str(), arm), ("FireSocket", Some(80)));
    assert_eq!(c.shooter.weapon_state, weapon_state::RECOVERY);
    c.frame(DT, &SiegeInput::default());
    assert_eq!(c.shooter.arm_value(), 0, "Recovery clears ReplicatedArm on the authority");
}

/// BP_CatapultProjectile: InitialSpeed 230 * (5 + arm / 100 * 5), launched along (0.65, 0, 0.65) normalized: 45 deg up
/// in the socket frame
#[test]
fn catapult_launch() {
    assert_eq!(catapult_initial_speed(230.0, 0), 1150.0);
    assert_eq!(catapult_initial_speed(230.0, 80), 2070.0);
    let chain: Option<Vec<String>> = ["Interactables/SiegeEngines/BP_CatapultProjectile.json", "BP_MordhauProjectile.json"].iter().map(|p| bp(p)).collect();
    let Some(chain) = chain else { return eprintln!("SKIP: no extract/json") };
    let r: Vec<&str> = chain.iter().map(|s| s.as_str()).collect();
    let mut c = ProjectileCfg::from_json_chain(&r).unwrap();
    assert_eq!(c.initial_velocity, v(0.65, 0.0, 0.65));
    assert_eq!((c.initial_speed, c.max_speed), (230.0, 0.0));
    c.initial_speed = catapult_initial_speed(c.initial_speed, 80);
    let p = Projectile::fire_quat(c, v(0.0, 0.0, 0.0), rotator_quaternion(0.0, 90.0, 0.0), 0.0);
    assert!((size(p.velocity) - 2070.0).abs() < 0.01, "{:?}", p.velocity);
    assert!((p.velocity.z - p.velocity.y).abs() < 0.01 && p.velocity.x.abs() < 0.01, "{:?}", p.velocity);
}

#[test]
fn vehicle_net_smoothing() {
    let mut n = VehicleNet { net_time_behind: 0.1, net_send_rate: 0.05, ..Default::default() };
    n.add_state(NetState::create(1.0, v(100.0, 0.0, 0.0), [0.0, 10.0, 0.0]), 2.0);
    n.add_state(NetState::create(1.05, v(105.0, 0.0, 0.0), [0.0, 20.0, 0.0]), 2.0);
    assert_eq!(n.queue[0].local_timestamp, 2.1);
    assert!((n.queue[1].local_timestamp - 2.15).abs() < 1e-5);
    assert!((n.lerp_begin_time - 2.05).abs() < 1e-6);
    assert_eq!(n.sync_physics(2.05, DT, (v(90.0, 0.0, 0.0), [0.0, 0.0, 0.0])), None);
    // halfway from the state now (start = the mesh at now - dt) to the first queued
    let (p, r) = n.sync_physics(2.08, 0.02, (v(90.0, 0.0, 0.0), [0.0, 0.0, 0.0])).unwrap();
    let a = (2.08f32 - (2.08 - 0.02)) / (2.1 - (2.08 - 0.02));
    assert!((p.x - (90.0 + 10.0 * a)).abs() < 1e-3, "{p:?} {a}");
    assert!((r[1] - 10.0 * a).abs() < 1e-3);
    // a jump over 100 cm snaps to the target
    let mut n2 = VehicleNet { net_time_behind: 0.1, net_send_rate: 0.05, ..Default::default() };
    n2.add_state(NetState::create(1.0, v(100.0, 0.0, 0.0), [0.0, 10.0, 0.0]), 2.0);
    let (p, r) = n2.sync_physics(2.06, 0.01, (v(-500.0, 0.0, 0.0), [0.0, 0.0, 0.0])).unwrap();
    assert_eq!((p.x, r[1]), (100.0, 10.0));
    // the compressed rotation
    let s = NetState::create(0.0, v(0.0, 0.0, 0.0), [90.0, 180.0, 45.0]);
    assert_eq!((s.pitch, s.yaw, s.roll), (16384, 32768, 32));
}

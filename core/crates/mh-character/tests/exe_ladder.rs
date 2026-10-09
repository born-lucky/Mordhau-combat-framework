//! Ladders (exe_ladder.rs): the BP_LadderMover's UOneDimensionalMovementComponent on the BP_Ladder default line
//! (LadderStart (-40, 0, 100), LadderEnd (-40, 0, 550), LadderExit (-40, 0, 705)), climbing in steps of 42.852 at
//! MaxWalkSpeed 200, leaving at the ends, the exit and the jump off. Base records: core/tests/golden (ignored path);
//! SKIP (pass) without them.

use mh_character::exe::ExeMovement;
use mh_character::exe_ladder::{closest_point_on_line, LadderInfo};
use mh_character::uemath::{size, v};
use mh_character::world::BoxWorld;
use mh_character::{CharacterRecords, CharacterSource, RecordsJson};
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;
const STEP: f32 = 42.852;

fn base() -> Option<CharacterRecords> {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let s = std::fs::read_to_string(dir.join("../../../core/tests/golden/character/records.json")).ok()?;
    Some(RecordsJson(&s).load().unwrap())
}

fn info() -> LadderInfo {
    LadderInfo { start: v(-40.0, 0.0, 100.0), end: v(-40.0, 0.0, 550.0), exit: v(-40.0, 0.0, 705.0), yaw: 0.0 }
}

fn mover(b: &CharacterRecords, at: f32) -> ExeMovement {
    let mut m = ExeMovement::new_ladder_mover(b, info(), v(10.0, 0.0, at));
    m.ladder.as_mut().unwrap().has_driver = true;
    m.world_time = 10.0;
    m
}

#[test]
fn line_and_steps() {
    let Some(b) = base() else { return eprintln!("SKIP: no records") };
    assert_eq!(closest_point_on_line(v(0.0, 0.0, 0.0), v(0.0, 0.0, 10.0), v(5.0, 0.0, 20.0)), v(0.0, 0.0, 10.0));
    assert_eq!(closest_point_on_line(v(0.0, 0.0, 0.0), v(0.0, 0.0, 10.0), v(5.0, 0.0, 4.0)), v(0.0, 0.0, 4.0));
    let m = mover(&b, 100.0);
    let l = m.ladder.as_ref().unwrap();
    assert_eq!(m.location, v(-40.0, 0.0, 100.0));
    assert!(l.direction.x == 0.0 && l.direction.y == 0.0 && (l.direction.z - 1.0).abs() < 1e-6); // the rsqrt GetSafeNormal
    // 450 / 42.852 = 10.5013 -> RoundToInt 10
    assert_eq!((l.total_steps, l.current_step, l.target_step), (10, 0, 0));
    let m = mover(&b, 100.0 + 3.0 * STEP + 1.0);
    assert_eq!(m.ladder.as_ref().unwrap().current_step, 3);
}

/// holding forward climbs at MaxWalkSpeed (200 cm/s) along the line; releasing settles on the next step
#[test]
fn climb_and_settle() {
    let Some(b) = base() else { return };
    let w = BoxWorld::new();
    let mut m = mover(&b, 100.0);
    let mut changes = 0;
    for _ in 0..60 {
        changes += m.ladder_frame(&w, DT, 1.0) as i32;
        assert_eq!((m.location.x, m.location.y), (-40.0, 0.0));
        assert!(size(m.velocity) <= 200.0 + 1e-3, "speed {}", size(m.velocity));
    }
    let z1 = m.location.z;
    assert!(z1 > 100.0 + 150.0 && z1 < 100.0 + 200.0 + 1e-3, "after 1 s at z {z1}");
    assert!(changes >= 3);
    for _ in 0..120 {
        m.ladder_frame(&w, DT, 0.0);
    }
    let l = m.ladder.as_ref().unwrap();
    assert_eq!(l.current_step, l.target_step);
    let want = 100.0 + l.target_step as f32 * STEP;
    assert!((m.location.z - want).abs() < 2.5, "settled at {} vs step {}", m.location.z, want);
    assert!(size(m.velocity) < 1.0);
}

/// the line clamps the top; pushing on at the top step requests a leave (every 0.25 s), at the bottom pushing back does
#[test]
fn ends_request_leave() {
    let Some(b) = base() else { return };
    let w = BoxWorld::new();
    let mut m = mover(&b, 100.0);
    for _ in 0..240 {
        m.ladder_frame(&w, DT, 1.0);
        assert!(m.location.z <= 550.0 + 1e-3);
    }
    let l = m.ladder.as_ref().unwrap();
    assert_eq!(l.current_step, 10);
    assert!(l.leave_requests >= 4, "{}", l.leave_requests);
    let mut m = mover(&b, 100.0);
    m.ladder_frame(&w, DT, -1.0);
    assert_eq!(m.ladder.as_ref().unwrap().leave_requests, 1);
    m.ladder_frame(&w, DT, -1.0);
    assert_eq!(m.ladder.as_ref().unwrap().leave_requests, 1);
    assert_eq!(m.location.z, 100.0);
}

/// someone on the rungs above blocks the climb
#[test]
fn blocked_by_character_above() {
    let Some(b) = base() else { return };
    let w = BoxWorld::new();
    let mut m = mover(&b, 100.0);
    m.ladder.as_mut().unwrap().others.push((v(-40.0, 0.0, 100.0 + 96.0 * 2.0 + 1.0), 30.0, 96.0));
    for _ in 0..120 {
        m.ladder_frame(&w, DT, 1.0);
    }
    assert!(m.location.z < 100.0 + 15.0, "climbed to {}", m.location.z);
}

/// the exit at the top is LadderExit; elsewhere the base exit 50 cm behind; the jump off is weaker at the top
#[test]
fn exit_and_jump_off() {
    let Some(b) = base() else { return };
    let w = BoxWorld::new();
    let mut m = mover(&b, 100.0);
    assert_eq!(m.ladder_exit(&w, v(-40.0, 0.0, 100.0), 30.0, 96.0), v(-90.0, 0.0, 100.0));
    let j = m.ladder_jump_off_velocity(300.0);
    assert!((j.x - 750.0).abs() < 1e-3 && j.y.abs() < 1e-3 && (j.z - 500.0).abs() < 1e-3, "{j:?}");
    for _ in 0..240 {
        m.ladder_frame(&w, DT, 1.0);
    }
    assert_eq!(m.ladder_exit(&w, m.location, 30.0, 96.0), v(-40.0, 0.0, 705.0));
    let j = m.ladder_jump_off_velocity(999.0);
    assert!((j.x - 0.333 * 750.0 * 0.34).abs() < 0.5 && (j.z - 500.0 * 0.34).abs() < 0.5, "{j:?}");
    assert_eq!(m.ladder_knock_off_impulse(), v(-1750.0, 0.0, 0.0));
}

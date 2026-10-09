//! Exe-mode movement against the in-crate BoxWorld: floors, walls, sliding, stairs, slopes, ledges, landing, crouch
//! under a ceiling, perching, plus the precision rules read off the disassembly. Expected values come from the records
//! (core/tests/golden/character/records.json, the reference's dump of the game's data) and the decomp constants, never
//! from running the code under test.

use mh_character::exe::{ExeInput, ExeMovement, Mode, Sprint};
use mh_character::exe_cmc::{AVG_FLOOR_DIST, MAX_FLOOR_DIST, MIN_FLOOR_DIST};
use mh_character::uemath::{inv_sqrt, size, size_2d, v};
use mh_character::ue::FVector;
use mh_character::world::{BoxWorld, World};
use mh_character::{restriction, CharacterRecords, CharacterSource, RecordsJson};
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;
const WORLD_T0: f32 = 10.0;

fn rec() -> CharacterRecords {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let p = dir.join("../../../core/tests/golden/character/records.json");
    let txt = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e} (run godot/tools/export_golden_character.gd)", p.display()));
    RecordsJson(&txt).load().unwrap()
}

/// a 200 m floor, top at z = 0
fn flat() -> BoxWorld {
    let mut w = BoxWorld::new();
    w.add_box(v(-20000.0, -20000.0, -200.0), v(20000.0, 20000.0, 0.0));
    w
}

fn spawn(w: &BoxWorld, x: f32, y: f32, floor_z: f32) -> ExeMovement {
    let r = rec();
    let mut m = ExeMovement::new(&r, v(x, y, floor_z + 96.0 + AVG_FLOOR_DIST));
    m.random.seed = 1;
    // a world that has run for a while (zero-filled RagdollFallingGetUpStartTime / LastLand / LastDodgeTime compare
    // against world TimeSeconds: a world younger than RagdollFallingGetUpDuration keeps every pawn "getting up")
    m.world_time = WORLD_T0;
    // settle one idle frame (first floor find)
    m.frame(w, DT, &idle());
    m
}

fn idle() -> ExeInput {
    ExeInput::default()
}
fn fwd() -> ExeInput {
    ExeInput { fwd: 1.0, ..idle() }
}

fn run(m: &mut ExeMovement, w: &dyn World, n: usize, inp: ExeInput) {
    for _ in 0..n {
        m.frame(w, DT, &inp);
    }
}

fn no_overlap(m: &ExeMovement, w: &dyn World) -> bool {
    !w.overlap_capsule(m.location, m.capsule_radius() - 0.01, m.half_height() - 0.01)
}

#[test]
fn stands_on_flat_floor_at_floor_distance() {
    let w = flat();
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    run(&mut m, &w, 30, idle());
    assert_eq!(m.mode, Mode::Walking);
    assert!(m.current_floor.is_walkable_floor());
    let fd = m.current_floor.floor_dist;
    assert!((MIN_FLOOR_DIST..=MAX_FLOOR_DIST).contains(&fd), "floor dist {fd}");
    assert!((m.location.z - (96.0 + fd)).abs() < 0.01, "z {} fd {fd}", m.location.z);
    assert_eq!(m.velocity, FVector::ZERO);
}

#[test]
fn walk_reaches_max_walk_speed_scaled_by_analog_modifier() {
    let w = flat();
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    run(&mut m, &w, 120, fwd());
    let top = m.c.max_walk_speed * m.analog_input_modifier;
    let s = size(m.velocity);
    // PhysWalking recomputes Velocity = (Location - OldLocation) * (1 / timeTick) (0x142f8203a..0x142f82085): binary32
    // positions make that exact only to a location ulp per tick
    let ulp = f32::from_bits(m.location.x.to_bits() + 1) - m.location.x;
    assert!((s - top).abs() <= 2.0 * ulp / DT, "speed {s} vs {top}");
    // UCharacterMovementComponent::ComputeAnalogInputModifier rva=0x2f74f50: Size * (1 / MaxAccel) clamped to 1
    let a = m.get_max_acceleration();
    let want = (size(m.acceleration) * (1.0 / a)).min(1.0);
    assert_eq!(m.analog_input_modifier, want);
    assert!(m.location.x > 300.0);
    assert_eq!(m.velocity.z, 0.0, "MaintainHorizontalGroundVelocity");
}

#[test]
fn wall_blocks_and_never_penetrates() {
    let mut w = flat();
    w.add_box(v(200.0, -1000.0, 0.0), v(260.0, 1000.0, 300.0));
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    run(&mut m, &w, 180, fwd());
    let r = m.capsule_radius();
    assert!(m.location.x <= 200.0 - r, "x {}", m.location.x);
    assert!(m.location.x > 200.0 - r - 1.0, "stopped too early: {}", m.location.x);
    assert!(size_2d(m.velocity) < 1.0, "still pushing into the wall: {:?}", m.velocity);
    assert!(no_overlap(&m, &w));
    assert_eq!(m.mode, Mode::Walking);
}

#[test]
fn slides_along_a_wall_when_pushing_diagonally() {
    let mut w = flat();
    w.add_box(v(200.0, -2000.0, 0.0), v(260.0, 2000.0, 300.0));
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    let inp = ExeInput { yaw: 45.0, fwd: 1.0, ..Default::default() };
    run(&mut m, &w, 240, inp);
    assert!(m.location.x <= 200.0 - m.capsule_radius());
    assert!(m.location.y > 300.0, "did not slide along the wall: y {}", m.location.y);
    assert!(no_overlap(&m, &w));
}

#[test]
fn climbs_steps_below_max_step_height() {
    let mut w = flat();
    // stairs: 30 cm rise, 40 cm tread (MaxStepHeight 60 from CharMoveComp)
    for i in 0..6 {
        let x0 = 150.0 + 40.0 * i as f32;
        w.add_box(v(x0, -500.0, 0.0), v(x0 + 40.0, 500.0, 30.0 * (i + 1) as f32));
    }
    w.add_box(v(390.0, -500.0, 0.0), v(2000.0, 500.0, 180.0));
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    run(&mut m, &w, 360, fwd());
    assert!(m.location.x > 500.0, "stuck on the stairs at x {}", m.location.x);
    assert!((m.location.z - (180.0 + 96.0)).abs() < MAX_FLOOR_DIST + 0.5, "z {}", m.location.z);
    assert_eq!(m.mode, Mode::Walking);
    assert!(no_overlap(&m, &w));
}

#[test]
fn step_higher_than_max_step_height_blocks() {
    let mut w = flat();
    let h = rec().movement.max_step_height as f32 + 20.0;
    w.add_box(v(200.0, -500.0, 0.0), v(1000.0, 500.0, h));
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    run(&mut m, &w, 180, fwd());
    assert!(m.location.x <= 200.0 - m.capsule_radius() + 0.01, "climbed a {h} cm step: {:?}", m.location);
    assert!(m.location.z < 96.0 + MAX_FLOOR_DIST + 0.5);
}

#[test]
fn walks_up_walkable_slope_but_not_steep_one() {
    // walkable: 30 degrees (normal z 0.866 > WalkableFloorZ 0.71); steep: 50 degrees (0.643)
    for (deg, walkable) in [(30.0f32, true), (50.0f32, false)] {
        let mut w = flat();
        let a = deg.to_radians();
        let n = v(-a.sin(), 0.0, a.cos());
        w.add_plane(n, v(200.0, 0.0, 0.0), (200.0, -1000.0), (3000.0, 1000.0));
        let mut m = spawn(&w, 0.0, 0.0, 0.0);
        run(&mut m, &w, 240, fwd());
        let climbed = m.location.z - (96.0 + AVG_FLOOR_DIST);
        if walkable {
            assert!(climbed > 100.0, "{deg} deg: only climbed {climbed}");
            assert_eq!(m.mode, Mode::Walking);
        } else {
            assert!(climbed < 10.0, "{deg} deg: climbed {climbed}");
        }
    }
}

#[test]
fn walks_off_a_ledge_falls_and_lands() {
    let mut w = BoxWorld::new();
    w.add_box(v(-2000.0, -2000.0, -200.0), v(300.0, 2000.0, 0.0)); // upper floor, edge at x = 300
    w.add_box(v(300.0, -2000.0, -1200.0), v(4000.0, 2000.0, -300.0)); // lower floor 300 below
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    let mut fell = false;
    for _ in 0..400 {
        m.frame(&w, DT, &fwd());
        fell |= m.mode == Mode::Falling;
        if fell && m.mode == Mode::Walking {
            break;
        }
    }
    assert!(fell, "never fell off the ledge");
    assert_eq!(m.mode, Mode::Walking, "never landed");
    assert!((m.location.z - (-300.0 + 96.0)).abs() < MAX_FLOOR_DIST + 0.5, "z {}", m.location.z);
    assert_eq!(m.landings, 1);
    assert!(!m.last_land_from_jump);
    assert!(no_overlap(&m, &w));
    // a 300 cm drop lands below MinVelocityForFallDamage (sqrt(2 g h) = 840 cm/s with GravityScale): no damage
    let g = -(m.c.gravity_z * m.c.gravity_scale);
    assert!((2.0 * g * 300.0).sqrt() < m.c.min_velocity_for_fall_damage);
    assert!(m.fall_damage.is_empty(), "{:?}", m.fall_damage);
}

#[test]
fn long_drop_deals_fall_damage_from_the_last_frame_velocity() {
    let mut w = BoxWorld::new();
    w.add_box(v(-2000.0, -2000.0, -3200.0), v(2000.0, 2000.0, -3000.0));
    let r = rec();
    let mut m = ExeMovement::new(&r, v(0.0, 0.0, 200.0));
    m.world_time = WORLD_T0;
    m.mode = Mode::Falling;
    let mut last_vz = 0.0;
    for _ in 0..400 {
        let before = m.last_falling_check_velocity_z;
        m.frame(&w, DT, &idle());
        if m.mode == Mode::Walking {
            last_vz = before;
            break;
        }
    }
    assert_eq!(m.mode, Mode::Walking);
    // the landing's CheckFallDamage(0) (OnMovementModeChanged rva=0x14901a0) uses the previous frame's Velocity.Z
    let c = &m.c;
    let delta = 0.0 - last_vz;
    let want = (delta + c.fall_damage_offset) * c.fall_damage_factor;
    assert!(want > 0.0 && delta > c.min_velocity_for_fall_damage);
    // a fall this long trips first (FallingTimeToRagdoll): the Ragdoll* terms apply then
    let d = &m.fall_damage;
    assert_eq!(d.len(), 1, "{d:?}");
    let want = if m.trips > 0 {
        (delta + c.ragdoll_fall_damage_offset) * c.ragdoll_fall_damage_factor
    } else {
        want
    };
    assert_eq!(d[0], want);
}

#[test]
fn jump_apex_and_landing_restriction() {
    let w = flat();
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    // a fresh world: no jump before LastDodgeTime (0) + DodgeDuration (AMordhauCharacter::CanJumpInternal_Implementation
    // 0x1415326bd) nor inside the zero-filled get-up window
    let mut fresh = ExeMovement::new(&rec(), m.location);
    fresh.frame(&w, DT, &idle());
    fresh.frame(&w, DT, &ExeInput { jump: true, ..idle() });
    assert_eq!(fresh.mode, Mode::Walking, "jumped at world start");
    m.frame(&w, DT, &idle());
    let z0 = m.location.z;
    m.frame(&w, DT, &ExeInput { jump: true, ..idle() });
    assert_eq!(m.mode, Mode::Falling);
    assert_eq!(m.jumps, 1);
    let mut apex = z0;
    for _ in 0..200 {
        m.frame(&w, DT, &idle());
        apex = apex.max(m.location.z);
        if m.mode == Mode::Walking {
            break;
        }
    }
    let g = -(m.c.gravity_z * m.c.gravity_scale);
    let want = m.c.jump_z_velocity * m.c.jump_z_velocity / (2.0 * g);
    assert!((apex - z0 - want).abs() < 1.0, "apex {} want {want}", apex - z0);
    assert_eq!(m.mode, Mode::Walking);
    assert!(m.last_land_from_jump);
    assert_eq!(m.get_movement_restriction(), restriction::PARTIAL_SPRINT);
}

#[test]
fn crouch_lowers_capsule_and_uncrouch_waits_for_headroom() {
    let mut w = flat();
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    let z0 = m.location.z;
    // CrouchPressed sets bWantsCrouch; AMordhauCharacter::LODTick (the actor tick, after the movement tick) calls
    // ACharacter::Crouch, which the next frame's movement applies
    m.frame(&w, DT, &ExeInput { crouch: true, ..idle() });
    assert!(!m.is_crouched && m.wants_to_crouch);
    m.frame(&w, DT, &ExeInput { crouch: true, ..idle() });
    assert!(m.is_crouched);
    let chh = m.c.crouched_half_height;
    assert_eq!(m.half_height(), chh);
    assert!((m.location.z - (z0 - (96.0 - chh))).abs() < 0.01, "base not kept: {} vs {}", m.location.z, z0);
    // a ceiling 20 cm above the crouched head: standing up is encroached (UCharacterMovementComponent::UnCrouch)
    let top = m.location.z + chh + 20.0;
    w.add_box(v(-500.0, -500.0, top), v(500.0, 500.0, top + 50.0));
    for _ in 0..60 {
        m.frame(&w, DT, &idle());
    }
    assert!(m.is_crouched, "stood up into the ceiling");
    // walk out from under it: stands up
    for _ in 0..240 {
        m.frame(&w, DT, &fwd());
        if !m.is_crouched {
            break;
        }
    }
    assert!(!m.is_crouched, "never stood up");
    assert_eq!(m.half_height(), 96.0);
    assert!(no_overlap(&m, &w));
}

#[test]
fn perches_on_an_edge_within_the_perch_radius() {
    // a 200 cm high block; the capsule centre stands 20 cm beyond its edge (radius 50, PerchRadiusThreshold 25 ->
    // valid perch radius 25): the bottom still overlaps the top by 30 cm -> stays on it
    let mut w = BoxWorld::new();
    w.add_box(v(-500.0, -500.0, -100.0), v(0.0, 500.0, 200.0));
    w.add_box(v(-5000.0, -5000.0, -300.0), v(5000.0, 5000.0, -200.0));
    let r = rec();
    let mut m = ExeMovement::new(&r, v(20.0, 0.0, 200.0 + 96.0 + AVG_FLOOR_DIST));
    m.world_time = WORLD_T0;
    run(&mut m, &w, 30, idle());
    assert_eq!(m.mode, Mode::Walking, "fell off a perchable edge");
    // 40 cm beyond the edge: 10 cm overlap, inside the 25 cm reject band of the perch radius -> falls
    let mut m = ExeMovement::new(&r, v(40.0, 0.0, 200.0 + 96.0 + AVG_FLOOR_DIST));
    m.world_time = WORLD_T0;
    run(&mut m, &w, 30, idle());
    assert!(m.mode == Mode::Falling || m.location.z < 150.0, "stood on a 10 cm lip: {:?}", m.location);
}

#[test]
fn sprint_states_from_input_direction() {
    let w = flat();
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    run(&mut m, &w, 60, ExeInput { sprint: true, ..fwd() });
    assert!(matches!(m.sprint_state, Sprint::Sprint | Sprint::Partial), "{:?}", m.sprint_state);
    run(&mut m, &w, 5, ExeInput { fwd: -1.0, ..idle() });
    assert_eq!(m.sprint_state, Sprint::Backpedal);
    run(&mut m, &w, 5, ExeInput { right: 1.0, ..idle() });
    assert_eq!(m.sprint_state, Sprint::Sideways);
    // GetMaxSpeed state 1: StrafeModifier (+0xcd4, the f32 0x3f733333) x speed factor x MaxWalkSpeed, all binary32
    assert_eq!(m.get_max_speed(), m.c.max_walk_speed * (f32::from_bits(0x3f73_3333) * m.get_speed_factor(0.0)));
}

#[test]
fn inv_sqrt_matches_the_inlined_sequence() {
    // FMath::InvSqrt as inlined at 0x142f70695: rsqrtss then two Newton-Raphson steps
    for x in [2.0f32, 1650.0 * 1650.0 * 2.0, 0.5, 308.0 * 308.0] {
        let y = inv_sqrt(x);
        assert!(((y as f64) - 1.0 / (x as f64).sqrt()).abs() < 1e-6 * (1.0 / (x as f64).sqrt()));
    }
}

#[test]
fn knockback_on_the_ground_and_frictions() {
    let w = flat();
    let mut m = spawn(&w, 0.0, 0.0, 0.0);
    let base = m.c.ground_friction;
    m.knockback(v(400.0, 0.0, 0.0));
    assert_eq!(m.c.ground_friction, m.c.knockback_ground_friction);
    m.frame(&w, DT, &idle());
    assert!(m.velocity.x > 0.0);
    let mut n = 1;
    while m.is_in_knockback() {
        m.frame(&w, DT, &idle());
        n += 1;
        assert!(n < 100);
    }
    assert_eq!(m.c.ground_friction, base);
    assert!(((n as f32) * DT - m.c.knockback_duration).abs() <= DT);
}

//! The climb's movement side (exe_climb.rs): UClimbingMotion::OnTick_Implementation rva=0x1667790's two phases and its
//! 2 cm / EndTime update rule, UMordhauUtilityLibrary::GetNormalizedTime rva=0x1624620.

use mh_character::exe::{ExeInput, ExeMovement, Mode};
use mh_character::exe_climb::{climb_begin, climb_step, normalized_time, ClimbState, ClimbTiming};
use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::uemath::v;
use mh_character::world::BoxWorld;
use mh_character::{CharacterSource, RecordsJson};
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;

fn pawn(w: &BoxWorld) -> ExeMovement {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let p = dir.join("../../../core/tests/golden/character/records.json");
    let r = RecordsJson(&std::fs::read_to_string(&p).unwrap()).load().unwrap();
    let mut m = ExeMovement::new(&r, v(0.0, 0.0, 96.0 + AVG_FLOOR_DIST));
    m.world_time = 10.0;
    m.frame(w, DT, &ExeInput::default());
    m
}

#[test]
fn normalized_time_branches() {
    assert_eq!(normalized_time(1.0, 2.0, 2.0), 1.0);
    assert_eq!(normalized_time(1.0, 2.0, 1.0), 0.0);
    assert_eq!(normalized_time(1.0, 2.0, 0.5), 0.0);
    assert_eq!(normalized_time(2.0, 2.0, 2.0), 1.0);
    assert_eq!(normalized_time(1.0, 3.0, 2.5), (2.5f32 - 1.0) / (3.0 - 1.0));
}

#[test]
fn climb_rises_then_moves_onto_the_ledge() {
    let mut w = BoxWorld::new();
    w.add_box(v(-2000.0, -2000.0, -200.0), v(2000.0, 2000.0, 0.0));
    let mut m = pawn(&w);
    climb_begin(&mut m, &w);
    assert_eq!(m.mode, Mode::Custom);
    let z0 = m.location.z;
    let target = v(80.0, 0.0, 150.0 + 96.0 + AVG_FLOOR_DIST);
    let t0 = m.world_time;
    let tm = ClimbTiming { start_time: t0, vertical_start: 0.1, horizontal_start: 0.6, horizontal_duration: 0.3, end_time: t0 + 1.0 };
    let mut s = ClimbState::default();
    // before VerticalStart nothing moves
    assert!(!climb_step(&mut m, &mut s, &tm, target));
    assert_eq!(s.start_z, None);
    // mid vertical phase: Z interpolated from the latched Z towards Target.Z + 1, X / Y untouched
    m.world_time = t0 + 0.35;
    assert!(climb_step(&mut m, &mut s, &tm, target));
    let a = normalized_time(t0 + 0.1, t0 + 0.6, t0 + 0.35);
    assert_eq!(m.location.z, ((target.z + 1.0) - z0) * a + z0);
    assert_eq!((m.location.x, m.location.y), (0.0, 0.0));
    // a tick that moves under 2 cm before EndTime leaves the actor
    let before = m.location;
    m.world_time = t0 + 0.351;
    assert!(!climb_step(&mut m, &mut s, &tm, target));
    assert_eq!(m.location, before);
    // horizontal phase: X / Y latched at its start, the vertical phase finished (alpha 1)
    m.world_time = t0 + 0.75;
    assert!(climb_step(&mut m, &mut s, &tm, target));
    assert_eq!(m.location.z, (target.z + 1.0 - z0) * 1.0 + z0);
    let a = normalized_time(0.6 + t0, 0.6 + t0 + 0.3, t0 + 0.75);
    assert_eq!(m.location.x, (target.x - 0.0) * a + 0.0);
    // at EndTime the actor sits on Target (Z + 1)
    m.world_time = t0 + 1.0;
    climb_step(&mut m, &mut s, &tm, target);
    assert_eq!((m.location.x, m.location.y), (target.x, target.y));
}

/// a 200 cm wide block of `h` cm in front of the pawn (x from 70 to 400), floor at z 0
fn ledge(h: f32) -> BoxWorld {
    let mut w = BoxWorld::new();
    w.add_box(v(-2000.0, -2000.0, -200.0), v(2000.0, 2000.0, 0.0));
    w.add_box(v(70.0, -100.0, 0.0), v(400.0, 100.0, h));
    w
}

/// BP_MordhauCharacter FindClimbSpot / AttemptClimb on a ledge: JumpPressed climbs a ledge between MinHeightToClimb
/// (max(GetMaxJumpHeight() - 12, MaxStepHeight)) and the reach of the 200 cm up cast; a low step or a tall wall is a
/// plain jump
#[test]
fn jump_pressed_climbs_a_ledge() {
    for (h, climbs) in [(120.0f32, true), (40.0, false), (500.0, false)] {
        let w = ledge(h);
        let mut m = pawn(&w);
        let min_h = (m.get_max_jump_height() - 12.0).max(m.c.max_step_height);
        assert_eq!(min_h, 450.0f32 * 450.0 * (-0.5 / m.gravity_z()) - 12.0);
        m.frame(&w, DT, &ExeInput { jump: true, ..Default::default() });
        match m.climb_request {
            Some((t, slow)) => {
                assert!(climbs, "climbed a {h} cm block");
                // the spot: the capsule centre resting on the block top, past its front edge
                assert!((t.z - (h + 96.0)).abs() < 0.5, "target {t:?}");
                assert!(t.x > 70.0 && t.x < 400.0, "target {t:?}");
                assert!(!slow, "standing climb with the floor 200 cm down");
                assert_eq!(m.climb_target_location, t);
                assert_eq!(m.mode, Mode::Walking, "a climb is not a jump");
            }
            None => {
                assert!(!climbs, "no climb onto a {h} cm block");
                assert_eq!(m.mode, Mode::Falling, "a failed climb jumps");
                // bWantsClimb was set by JumpPressed; the movement tick (bTickBeforeOwner: before the actor tick) made
                // the jump, so this frame's AMordhauCharacter::LODTick sees the pawn airborne and keeps retrying
                assert!(m.wants_climb);
            }
        }
    }
}

/// pressing jump again while airborne: TryClimbing fails, bWantsClimb stays set (airborne) and AMordhauCharacter::
/// LODTick retries it every tick until the pawn reaches the ledge; the climb motion then carries it onto the block
#[test]
fn airborne_retry_and_climb_onto_the_block() {
    let w = ledge(150.0);
    let mut m = pawn(&w);
    // run up, jump short of the block (no ledge in reach), release, press again in the air
    m.location = v(-600.0, 0.0, m.location.z);
    while m.location.x < -260.0 {
        m.frame(&w, DT, &ExeInput { fwd: 1.0, ..Default::default() });
    }
    m.frame(&w, DT, &ExeInput { jump: true, fwd: 1.0, ..Default::default() });
    assert!(m.climb_request.is_none() && m.mode == Mode::Falling, "x {}", m.location.x);
    m.frame(&w, DT, &ExeInput { fwd: 1.0, ..Default::default() });
    m.frame(&w, DT, &ExeInput { jump: true, fwd: 1.0, ..Default::default() });
    assert!(m.climb_request.is_none() && m.wants_climb, "press in the air arms the retry");
    let mut frames = 0;
    while m.climb_request.is_none() && frames < 120 && m.mode == Mode::Falling {
        m.frame(&w, DT, &ExeInput { jump: true, fwd: 1.0, ..Default::default() });
        frames += 1;
    }
    let (target, _) = m.climb_request.expect("no climb while airborne");
    assert!(!m.wants_climb);
    // the motion: begin, then steps until EndTime
    climb_begin(&mut m, &w);
    let t0 = m.world_time;
    let tm = ClimbTiming { start_time: t0, vertical_start: 0.0, horizontal_start: 0.5, horizontal_duration: 0.3, end_time: t0 + 0.9 };
    let mut s = ClimbState::default();
    while m.world_time <= tm.end_time + DT {
        m.world_time += DT;
        climb_step(&mut m, &mut s, &tm, target);
    }
    assert_eq!((m.location.x, m.location.y, m.location.z), (target.x, target.y, target.z + 1.0));
    // the motion's end hands the pawn back to walking (combat side); it stands on the block
    m.set_movement_mode(&w, Mode::Walking);
    for _ in 0..30 {
        m.frame(&w, DT, &ExeInput::default());
    }
    assert_eq!(m.mode, Mode::Walking);
    assert!((m.location.z - (150.0 + 96.0)).abs() < 3.0, "z {}", m.location.z);
}

/// CalculateLedgeOffsetAndNormal (BP): the capsule cast from +50 cm along the facing for 105 cm; OnBegin's timing
/// swap and EndTime, the net offset bytes
#[test]
fn climb_on_begin_offsets_and_timing() {
    use mh_character::exe_climb::ClimbMotionData;
    let w = ledge(150.0);
    let mut m = pawn(&w);
    // the block face is at x = 70: capsule radius 50 from x = 0 -> hit after 20 cm, normal -X
    let (o, n) = m.calculate_ledge_offset_and_normal(&w);
    assert!((o.x - 20.0).abs() < 0.1 && o.y == 0.0 && o.z == 0.0, "offset {o:?}");
    assert_eq!(n, v(1.0, 0.0, 0.0));
    let d = ClimbMotionData {
        end_extra: 0.2, vertical_start: 0.1, horizontal_start: 0.5, horizontal_duration: 0.3,
        slow_end_extra: 0.4, slow_vertical_start: 0.2, slow_horizontal_start: 0.9, slow_horizontal_duration: 0.5,
        turn_cap_yaw: 45.0, turn_cap_pitch: 30.0,
    };
    let t0 = m.world_time;
    let b = m.climb_on_begin(&w, &d, [255, 0, 51, 0], t0);
    assert_eq!(m.mode, Mode::Custom);
    assert_eq!(b.timing.end_time, (t0 + 0.5) + (0.2 + 0.3));
    let k = f32::from_bits(0x3b80_8081);
    assert_eq!(b.net_offset.x, 1.0 * 200.0 - 100.0 - o.x);
    assert_eq!(b.net_offset.y, (0.0 * k) * 200.0 - 100.0);
    assert_eq!(b.net_offset.z, (51.0 * k) * 255.0);
    let s = m.climb_on_begin(&w, &d, [0, 0, 0, 1], t0);
    assert_eq!((s.timing.vertical_start, s.timing.horizontal_start, s.timing.horizontal_duration), (0.2, 0.9, 0.5));
    assert_eq!(s.timing.end_time, (t0 + 0.9) + (0.4 + 0.5));
    assert_eq!(s.turn_caps, (45.0, 30.0));
}

//! UMordhauMovementComponent::LODTick rva=0x14c4d60 in exe mode: turn-sprint prevention, the avoidance bubble and its
//! input cancel, the chase timers, rush, the dodge. Expected values come from the records (the game's data: ctor
//! defaults, BP_MordhauCharacter, the FC_TurnSprintPrevention curves) and the disassembled formulas, in binary32.

use mh_character::exe::{ExeInput, ExeMovement, Mode, Sprint};
use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::uemath::{size, v};
use mh_character::uequat::{atan2, curve_eval, finterp_constant_to, RAD_TO_DEG};
use mh_character::world::{BoxWorld, World};
use mh_character::{CharacterRecords, CharacterSource, OtherPawn, RecordsJson};
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;

fn rec() -> CharacterRecords {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let p = dir.join("../../../core/tests/golden/character/records.json");
    RecordsJson(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).load().unwrap()
}

fn flat() -> BoxWorld {
    let mut w = BoxWorld::new();
    w.add_box(v(-20000.0, -20000.0, -200.0), v(20000.0, 20000.0, 0.0));
    w
}

fn pawn(w: &dyn World) -> ExeMovement {
    let mut m = ExeMovement::new(&rec(), v(0.0, 0.0, 96.0 + AVG_FLOOR_DIST));
    m.world_time = 10.0;
    m.frame(w, DT, &ExeInput::default());
    m
}

#[test]
fn records_carry_the_lodtick_data() {
    let r = rec();
    // BP_MordhauCharacter / CharMoveComp overrides of the native ctor values (ctor rva=0x14af4f0 / 0x1524d90)
    assert_eq!(r.move_extra.turn_sprint_prevention_max_accumulated_angle, 90.0);
    assert!(r.curves.contains_key(&r.move_extra.turn_sprint_prevention_decay_curve));
    assert!(r.curves.contains_key(&r.move_extra.turn_sprint_prevention_slowdown_curve));
    assert!(r.character.ellipse_bubble_radius > 0.0 && r.character.ellipse_bubble_length > 0.0);
    assert!(!r.character.b_can_dodge, "BP_MordhauCharacter does not set bCanDodge");
}

/// 0x1414c5560..: AngleCounter = FInterpConstantTo(clamp(dYaw + AngleCounter, +-Max), 0, dt, Decay(|c|)); input
/// clamped to Slowdown(|AngleCounter|) when that is below 1
#[test]
fn turn_sprint_prevention_slows_a_fast_turn() {
    let w = flat();
    let mut m = pawn(&w);
    let c = m.c.clone();
    let decay = c.turn_decay_curve.clone().unwrap();
    let slow = c.turn_slowdown_curve.clone().unwrap();
    m.frame(&w, DT, &ExeInput { fwd: 1.0, yaw: 0.0, ..Default::default() });
    assert_eq!(m.turn_angle_counter, 0.0);
    // a 120 degree turn in one frame: the counter clamps to the max, then decays
    m.frame(&w, DT, &ExeInput { fwd: 1.0, yaw: 120.0, ..Default::default() });
    let cl = c.turn_max_accumulated_angle;
    let want = finterp_constant_to(cl, 0.0, DT, curve_eval(&decay, cl));
    assert_eq!(m.turn_angle_counter, want);
    assert_eq!(m.turn_last_angle, 120.0);
    let s = curve_eval(&slow, want.abs());
    assert!(s < 1.0, "slowdown {s} at {want}");
    // the slowed input is what the movement accelerated with: |Acceleration| = MaxAccel * min(|input|, 1)
    assert!((size(m.control_input_vector) - 0.0).abs() < 1e-6, "input consumed");
    assert!((m.analog_input_modifier - s).abs() < 1e-5, "analog {} slowdown {s}", m.analog_input_modifier);
}

#[test]
fn turn_sprint_prevention_waits_five_seconds_after_creation() {
    let w = flat();
    let mut m = pawn(&w);
    m.creation_time = m.world_time; // just spawned: 5 < TimeSeconds - CreationTime fails
    m.frame(&w, DT, &ExeInput { fwd: 1.0, yaw: 120.0, ..Default::default() });
    assert_eq!(m.turn_angle_counter, 0.0);
    assert_eq!(m.turn_last_angle, 120.0, "LastAngle is written every tick");
}

/// walking straight into an enemy's bubble: the input heading is within 10 degrees of the direction to the enemy
/// (0x1414c68d8) -> input zeroed, bWasDodgeCanceled; the push alone cannot rotate a zero input (it is applied as a yaw)
#[test]
fn bubble_cancels_input_into_an_enemy() {
    let w = flat();
    let mut m = pawn(&w);
    let o = OtherPawn::enemy(&m, v(50.0, 0.0, m.location.z), 180.0);
    m.others = vec![o];
    m.frame(&w, DT, &ExeInput { fwd: 1.0, ..Default::default() });
    assert!(m.was_dodge_canceled);
    assert_eq!(m.velocity, v(0.0, 0.0, 0.0), "no acceleration toward the enemy");
}

/// an enemy off to the side: the push turns the input away from it, by at most 90 degrees (0x1414c6b9a)
#[test]
fn bubble_rotates_input_away_from_a_side_enemy() {
    let w = flat();
    let mut m = pawn(&w);
    let o = OtherPawn::enemy(&m, v(60.0, 60.0, m.location.z), 0.0);
    m.others = vec![o];
    m.frame(&w, DT, &ExeInput { fwd: 1.0, ..Default::default() });
    assert!(!m.was_dodge_canceled);
    let a = atan2(m.velocity.y, m.velocity.x) * RAD_TO_DEG;
    assert!(a < 0.0 && a >= -90.0, "velocity heading {a}");
}

/// a friendly pawn has no bubble (0x1414c614c: friendly and not in a vehicle -> skip)
#[test]
fn friendly_has_no_bubble() {
    let w = flat();
    let mut m = pawn(&w);
    let mut o = OtherPawn::enemy(&m, v(50.0, 0.0, m.location.z), 180.0);
    o.friendly = true;
    m.others = vec![o];
    m.frame(&w, DT, &ExeInput { fwd: 1.0, ..Default::default() });
    assert!(!m.was_dodge_canceled);
    assert!(m.velocity.x > 0.0);
}

/// chasing: an enemy ahead, facing away, inside ChasingMaxDistance of the camera, for MinTimeToStartChasing ->
/// bIsChasing; sprinting then is the Chase state (EMovementModifier 6)
#[test]
fn chase_starts_after_min_time() {
    let w = flat();
    let mut m = pawn(&w);
    for _ in 0..90 {
        m.frame(&w, DT, &ExeInput { fwd: 1.0, sprint: true, ..Default::default() });
    }
    assert_eq!(m.sprint_state, Sprint::Sprint);
    let ahead = v(m.location.x + 200.0, m.location.y, m.location.z);
    m.others = vec![OtherPawn::enemy(&m, ahead, 0.0)];
    let need = m.c.min_time_to_start_chasing;
    let mut t = 0.0f32;
    let mut frames = 0;
    while t < need {
        t += DT;
        frames += 1;
    }
    for i in 0..frames {
        // keep the target 200 cm ahead
        let o = &mut m.others[0];
        o.location = v(m.location.x + 200.0, m.location.y, m.location.z);
        o.mesh_location = o.location;
        o.lower_back = o.location;
        assert!(!m.is_chasing || i == frames, "chasing too early at {i}");
        m.frame(&w, DT, &ExeInput { fwd: 1.0, sprint: true, ..Default::default() });
    }
    assert_eq!(m.others[0].total_chased_time, t);
    assert!(m.is_chasing);
    assert_eq!(m.sprint_state, Sprint::Chase);
    // facing us instead (angle 180 >= MaxAngleToStopChasing) the timer breaks after TimeToBreakUsChasing
    m.others[0].forward = v(-1.0, 0.0, 0.0);
    let brk = m.c.time_to_break_us_chasing;
    let mut t = 0.0f32;
    while t <= brk + DT {
        m.frame(&w, DT, &ExeInput { fwd: 1.0, sprint: true, ..Default::default() });
        t += DT;
    }
    assert_eq!(m.others[0].total_chased_time, 0.0);
    assert!(!m.is_chasing);
}

/// Rush perk: inside RushMovementBoostDuration of the last kill the pawn sprints without the button (0x1414c6e80); at
/// walking speed that is the partial sprint (Speed + 5 < SpeedFactor * MaxWalkSpeed * PartialSprintModifier), and once
/// up to speed SprintTime restarts at RushSprintTimeStart (0x1414c7a0e)
#[test]
fn rush_sprints_and_starts_sprint_time() {
    let w = flat();
    let mut m = pawn(&w);
    for _ in 0..60 {
        m.frame(&w, DT, &ExeInput { fwd: 1.0, ..Default::default() });
    }
    assert_eq!(m.sprint_state, Sprint::Forward);
    m.rush_perk = Some((5.0, m.world_time));
    m.frame(&w, DT, &ExeInput { fwd: 1.0, ..Default::default() });
    assert_eq!(m.sprint_state, Sprint::Partial, "rush from walking speed is a partial sprint");
    for _ in 0..60 {
        m.frame(&w, DT, &ExeInput { fwd: 1.0, ..Default::default() });
        if m.sprint_state == Sprint::Rush {
            break;
        }
    }
    assert_eq!(m.sprint_state, Sprint::Rush);
    assert!(m.sprint_time >= m.c.rush_sprint_time_start + DT, "sprint time {}", m.sprint_time);
}

/// dodge (bCanDodge pawns, e.g. Horde): jump pressed with input not forward (local X <= 0.01) -> StopJumping, the
/// dodge starts (ServerRequestDodge byte = RoundToInt(angle * 256 / 360)), the pawn supersprints along the dodge
#[test]
fn dodge_backwards() {
    let w = flat();
    let mut m = pawn(&w);
    m.c.can_dodge = true;
    let stamina0 = m.stamina;
    m.frame(&w, DT, &ExeInput { fwd: -1.0, jump: true, ..Default::default() });
    assert_eq!(m.mode, Mode::Walking, "a dodge is not a jump");
    assert_eq!(m.last_dodge_time, m.world_time);
    // atan2(0, -1) = pi -> 180 deg -> 180 * 0.7083334 * 2 + 0.5 -> cvtss2si -> >> 1
    let a = 180.0f32 * f32::from_bits(0x3f35_5556);
    let want = ((((a + a + 0.5) as f64).round_ties_even() as i64) >> 1) as u8;
    assert_eq!(m.dodges, vec![want]);
    assert_eq!(want, 128);
    assert_eq!(m.sprint_state, Sprint::Super);
    assert_eq!(m.stamina, stamina0, "the stamina cost is the server's (ServerRequestDodge), not LODTick's");
    // cooldown: DodgeDuration + LastDodgeTime + DodgeCooldown < TimeSeconds
    m.frame(&w, DT, &ExeInput { fwd: -1.0, jump: true, ..Default::default() });
    assert_eq!(m.dodges.len(), 1);
}

/// BP_MordhauCharacter has no bCanDodge: jump with backward input is a jump
#[test]
fn no_dodge_without_bcandodge() {
    let w = flat();
    let mut m = pawn(&w);
    m.frame(&w, DT, &ExeInput { fwd: -1.0, jump: true, ..Default::default() });
    assert!(m.dodges.is_empty());
    assert_eq!(m.mode, Mode::Falling);
}

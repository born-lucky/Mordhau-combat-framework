//! Exe-mode rules checked against values computed from the disassembled formulas (binary32, the exe's operation order),
//! and the places where the exe differs from the GDScript reference (compared directly when reference_compat is on).

use mh_character::exe::{ExeInput, ExeMovement, Sprint};
#[cfg(feature = "reference_compat")]
use mh_character::exe::Mode;
use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::uemath::v;
use mh_character::world::{BoxWorld, World};
use mh_character::{restriction, CharacterRecords, CharacterSource, RecordsJson};
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
    m.frame(w, DT, &idle());
    m
}

fn idle() -> ExeInput {
    ExeInput::default()
}

/// UAdvancedCharacterMovement::CheckFallDamage rva=0x145a230 in binary32 (subss / addss / mulss)
#[test]
fn fall_damage_is_binary32() {
    let mut m = ExeMovement::new(&rec(), v(0.0, 0.0, 0.0));
    let vz = -1234.567f32;
    m.check_fall_damage(vz);
    let d = m.check_fall_damage(0.0);
    let c = &m.c;
    let want = ((0.0f32 - vz) + c.fall_damage_offset) * c.fall_damage_factor;
    assert_eq!(d, want);
    let f64_ref = ((0.0 - vz as f64) + c.fall_damage_offset as f64) * c.fall_damage_factor as f64;
    assert_ne!(d as f64, f64_ref, "binary32 and the reference's f64 agree here: pick another value");
}

/// UMordhauMovementComponent::GetMaxSpeed rva=0x14be1d0: the sprint ramp (m - Partial) * min(SprintTime / Reach, 1) +
/// Partial, times GetSpeedFactor(t) with t = (m - Partial) / (Sprint - Partial), times MaxWalkSpeed, all binary32
#[test]
fn sprint_ramp_is_binary32() {
    let w = flat();
    let mut m = pawn(&w);
    m.sprint_state = Sprint::Sprint;
    m.sprint_time = 0.3;
    let c = m.c.clone();
    let x = (0.3f32 / c.sprint_time_to_reach_max_sprint).min(1.0);
    let mm = (c.sprint_modifier - c.partial_sprint_modifier) * x + c.partial_sprint_modifier;
    let t = ((mm - c.partial_sprint_modifier) / (c.sprint_modifier - c.partial_sprint_modifier)).min(1.0);
    let k = ((m.equip_speed_add + m.armor_speed) + (1.0 - t) * m.equip_sprint_penalty) * m.motion_speed_factor;
    assert_eq!(m.get_max_speed(), c.max_walk_speed * (mm * k));
    // the Dwarf perk multiplies twice (GetSpeedFactor 0x1414bfa3c and GetMaxSpeed 0x1414be3be)
    m.dwarf_speed_modifier = Some(0.9);
    let k2 = (((m.equip_speed_add + m.armor_speed) + (1.0 - t) * m.equip_sprint_penalty) * 0.9) * m.motion_speed_factor;
    assert_eq!(m.get_max_speed(), c.max_walk_speed * ((mm * k2) * 0.9));
}

/// First walking frame from rest (CalcVelocity rva=0x2f70490, MoveAlongFloor rva=0x2f7bef0, PhysWalking's velocity
/// from the moved distance 0x142f8203a): V1 = A * dt (A = dir * WalkAcceleration * clamped input), location moves by V1 *
/// dt, Velocity = (Location - OldLocation) * (1 / dt)
#[test]
fn first_walking_frame_follows_the_disassembly() {
    let w = flat();
    let mut m = pawn(&w);
    let x0 = m.location.x;
    m.frame(&w, DT, &ExeInput { fwd: 1.0, ..idle() });
    let a = m.c.walk_acceleration; // sprint state Forward, not crouched
    let v1 = DT * a + 0.0;
    let x1 = (x0 + v1 * DT - x0) * 1.0 + x0; // MoveComponentImpl: TraceStart + (TraceEnd - TraceStart) * 1
    assert_eq!(m.location.x, x1);
    assert_eq!(m.velocity.x, (x1 - x0) * (1.0 / DT));
    assert_eq!(m.analog_input_modifier, (a * (1.0 / a)).min(1.0));
}

/// ComputeAnalogInputModifier rva=0x2f74f50 on a diagonal input: Size * (1 / MaxAccel), not 1
#[test]
fn diagonal_input_analog_modifier() {
    let w = flat();
    let mut m = pawn(&w);
    m.frame(&w, DT, &ExeInput { fwd: 1.0, right: 1.0, yaw: 45.0, ..Default::default() });
    // input = forward + right of FRotator(0, 45, 0).Quaternion() (size sqrt 2): clamped to size 1 (LODTick, then
    // ScaleInputAcceleration), so the analog modifier is 1 up to the rsqrtss rounding
    let want = 1.0f32;
    assert!((m.analog_input_modifier - want).abs() <= 2.0 * f32::EPSILON, "{}", m.analog_input_modifier);
}

/// Jump gates of the exe (ACharacter::CanJumpInternal_Implementation rva=0x2f2eb40: !bIsCrouched; CanAttemptJump
/// rva=0x2f71610: !bWantsToCrouch; AMordhauCharacter::CanJumpInternal_Implementation rva=0x1532630: the dodge window;
/// AAdvancedCharacter::CanJumpInternal_Implementation rva=0x1459fb0: getting-up window)
#[test]
fn jump_gates() {
    let w = flat();
    let mut m = pawn(&w);
    assert!(m.can_jump());
    m.is_crouched = true;
    assert!(!m.can_jump(), "jumped crouched");
    m.is_crouched = false;
    m.wants_to_crouch = true;
    assert!(!m.can_jump(), "jumped while wanting to crouch");
    m.wants_to_crouch = false;
    m.last_dodge_time = m.world_time - m.c.dodge_duration;
    assert!(!m.can_jump(), "jumped inside the dodge window (needs TimeSeconds > LastDodge + DodgeDuration)");
    m.last_dodge_time = 0.0;
    m.motion_restriction = restriction::NO_MOVEMENT;
    assert!(!m.can_jump());
    m.motion_restriction = 0;
    m.jump_current_count = 1;
    assert!(!m.can_jump(), "JumpMaxCount 1 (ACharacter ctor rva=0x2f27200)");
    // a world younger than RagdollFallingGetUpDuration: the zero-filled get-up start keeps the pawn getting up
    let mut fresh = ExeMovement::new(&rec(), v(0.0, 0.0, 98.15));
    fresh.world_time = 1.0;
    assert!(fresh.is_ragdoll_falling_or_getting_up());
    assert!(!fresh.can_jump());
}

/// UPrimitiveComponent::MoveComponentImpl's PullBackHit (0x142fb4884..0x142fb48d5): a blocked move stops
/// (Clamp(0.1, 0.1 / Dist, 1 / Dist) + 0.001) x Dist short of the contact: about 0.1 cm for sub-centimetre moves
#[test]
fn pull_back_leaves_a_tenth_of_a_centimetre() {
    let mut w = flat();
    w.add_box(v(100.0, -500.0, 0.0), v(200.0, 500.0, 300.0));
    let mut m = pawn(&w);
    for _ in 0..240 {
        m.frame(&w, DT, &ExeInput { fwd: 1.0, ..idle() });
    }
    let gap = 100.0 - m.capsule_radius() - m.location.x;
    assert!(gap > 0.09 && gap < 0.2, "gap {gap}");
}

/// The landing's CheckFallDamage(0) runs inside the move (SetMovementMode(Walking) -> OnMovementModeChanged
/// rva=0x14901a0) against the previous frame's Velocity.Z; the reference checks after a whole falling tick
#[cfg(feature = "reference_compat")]
#[test]
fn landing_damage_differs_from_the_reference() {
    use mh_character::compat::MordhauMovement as Ref;
    use mh_character::ue::FVector as F;
    let r = rec();
    // exe: drop 1500 cm onto a box floor
    let mut w = BoxWorld::new();
    w.add_box(v(-2000.0, -2000.0, -1700.0), v(2000.0, 2000.0, -1500.0));
    let mut m = ExeMovement::new(&r, v(0.0, 0.0, 96.0 + AVG_FLOOR_DIST));
    m.world_time = 10.0;
    m.mode = Mode::Falling;
    for _ in 0..600 {
        m.frame(&w, DT, &idle());
        if m.mode == Mode::Walking {
            break;
        }
    }
    let exe = m.fall_damage.iter().copied().fold(0.0f32, f32::max);
    // reference: same drop, flat-floor host
    let mut g = Ref::new(&r);
    g.mode = mh_character::compat::Mode::Falling;
    let mut y = 0.0f32;
    for _ in 0..600 {
        let d = g.tick(DT as f64, F::ZERO, F::new(0.0, 0.0, -1.0));
        y += d.y;
        if y <= -1500.0 {
            g.set_on_floor(true);
            break;
        }
    }
    let reference = g.fall_damage.iter().copied().fold(0.0f64, f64::max);
    assert!(exe > 0.0 && reference > 0.0);
    assert_ne!(exe as f64, reference, "exe and reference landing damage coincide");
}

#[test]
fn sprint_needs_pending_input_direction_within_the_cone() {
    let w = flat();
    let mut m = pawn(&w);
    // 50 degrees off the actor forward: inside the 56.25 cone (acosf * 57.2957764 at 0x1414c713c)
    let (s, c) = (50f32.to_radians().sin(), 50f32.to_radians().cos());
    for _ in 0..60 {
        m.frame(&w, DT, &ExeInput { sprint: true, fwd: 1.0, ..Default::default() });
    }
    assert!(m.sprint_state >= Sprint::Partial);
    m.control_input_vector = v(c, s, 0.0);
    m.lod_tick(DT);
    assert!(m.sprint_state >= Sprint::Partial, "{:?}", m.sprint_state);
    let (s, c) = (60f32.to_radians().sin(), 60f32.to_radians().cos());
    m.control_input_vector = v(c, s, 0.0);
    m.lod_tick(DT);
    assert_eq!(m.sprint_state, Sprint::Sideways);
}

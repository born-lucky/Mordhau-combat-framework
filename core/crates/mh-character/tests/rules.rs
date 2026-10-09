//! reference_compat mode. Rule tests, ported from godot/tests/test_character_rules.gd (and the sprint/crouch parts of test_movement.gd).
//! Every expected number is computed from the records the rule reads (the reference's dump,
//! core/tests/golden/character/records.json), never typed in. Boundary cases sit on the exact comparison each
//! decompiled function makes.

use mh_character::compat::character::{apply_movement_events, CharacterInput, CombatSide};
use mh_character::records::{hex_f64, ini_value};
use mh_character::ue::{FVector, V3Ext};
use mh_character::compat::{Mode, MordhauMovement, Sprint};
use mh_character::{restriction, CharacterRecords, CharacterSource, RecordsJson};
use std::path::PathBuf;

const DT: f64 = 1.0 / 60.0;
const FWD: FVector = FVector::new(0.0, 0.0, -1.0);

fn records() -> CharacterRecords {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let p = dir.join("../../../core/tests/golden/character/records.json");
    let txt = std::fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!("{}: {e}\nrun godot/tools/export_golden_character.gd (see tests/golden.rs)", p.display())
    });
    RecordsJson(&txt).load().unwrap()
}

fn m() -> MordhauMovement {
    MordhauMovement::new(&records())
}

/// flat floor at y = 0, the body's job done by hand (as test_character_rules.gd _step)
fn step(m: &mut MordhauMovement, pos: FVector, input: FVector) -> FVector {
    let mut pos = pos + m.tick(DT, input, FWD);
    if pos.y <= 0.0 {
        pos.y = 0.0;
        m.set_on_floor(true);
    }
    pos
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-5 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn records_dump_reads_every_field() {
    let r = records();
    assert!(r.move_extra.min_velocity_for_fall_damage > 0.0, "MinVelocityForFallDamage not read");
    assert!(r.move_extra.b_can_crouch, "NavAgentProps.bCanCrouch false");
    assert!(r.movement.max_walk_speed > 0.0 && r.physics.gravity_z < 0.0);
    assert!(RecordsJson("{}").load().is_err(), "an empty dump must fail loudly");
    assert_eq!(hex_f64("000000000000f03f"), Some(1.0));
    assert_eq!(ini_value("[A]\nk=1\n[B]\nk=2\nk=3", "B", "k").as_deref(), Some("3"));
}

/// UAdvancedCharacterMovement::CheckFallDamage rva=0x145a230: MinVelocity < Delta (strict), (Delta + Offset) * Factor > 0
#[test]
fn fall_damage_boundary() {
    let mut m = m();
    let mn = m.fx.min_velocity_for_fall_damage;
    m.check_fall_damage(-mn);
    assert_eq!(m.check_fall_damage(0.0), 0.0, "Delta == MinVelocity dealt damage (comparison is strict)");
    let extra = 300.0;
    m.check_fall_damage(-(mn + extra));
    let d = m.check_fall_damage(0.0);
    assert_eq!(d, (mn + extra + m.fx.fall_damage_offset) * m.fx.fall_damage_factor);
    assert_eq!(m.fall_damage.len(), 1);
    m.authority = false;
    m.check_fall_damage(-(mn + extra));
    assert_eq!(m.check_fall_damage(0.0), 0.0, "non-authority dealt fall damage");
}

/// bIsRagdollFalling selects the Ragdoll* terms (CheckFallDamage rva=0x145a230)
#[test]
fn fall_damage_ragdoll_terms() {
    let mut m = m();
    m.ragdoll_falling = true;
    let v = m.fx.ragdoll_min_velocity_for_fall_damage + 500.0;
    m.check_fall_damage(-v);
    let d = m.check_fall_damage(0.0);
    assert_eq!(d, (v + m.fx.ragdoll_fall_damage_offset) * m.fx.ragdoll_fall_damage_factor);
}

/// PerformMovement rva=0x1495cc0 (CheckFallDamage(Velocity.Z) each move) + OnMovementModeChanged rva=0x14901a0
/// (CheckFallDamage(0) on landing): a jump lands safely, a long drop hurts once.
#[test]
fn fall_damage_from_drop() {
    let mut m = m();
    let mut pos = FVector::ZERO;
    for _ in 0..30 {
        pos = step(&mut m, pos, FVector::ZERO);
    }
    assert!(m.do_jump(), "could not jump");
    for _ in 0..120 {
        pos = step(&mut m, pos, FVector::ZERO);
    }
    assert!(m.fall_damage.is_empty(), "a plain jump dealt fall damage {:?}", m.fall_damage);
    let g = -m.gravity();
    let v_land = m.fx.min_velocity_for_fall_damage + 400.0;
    let h = v_land * v_land / (2.0 * g) + 50.0;
    m.mode = Mode::Falling;
    pos = FVector::new(0.0, h as f32, 0.0);
    let mut vz = 0.0;
    for _ in 0..600 {
        pos = pos + m.tick(DT, FVector::ZERO, FWD);
        vz = m.velocity.yf();
        if pos.y <= 0.0 {
            m.set_on_floor(true);
            break;
        }
    }
    assert_eq!(m.mode, Mode::Walking, "never landed");
    assert_eq!(m.fall_damage.len(), 1);
    assert!(approx(m.fall_damage[0], (-vz + m.fx.fall_damage_offset) * m.fx.fall_damage_factor));
}

/// AAdvancedCharacter::Landed rva=0x1489820 + AMordhauCharacter::GetMovementRestriction rva=0x1540f50:
/// PartialSprint for 0.1 s after landing from a jump, None after.
#[test]
fn restriction_after_jump_landing() {
    let mut m = m();
    let mut pos = FVector::ZERO;
    for _ in 0..30 {
        pos = step(&mut m, pos, FVector::ZERO);
    }
    assert!(m.do_jump());
    assert!(m.jumps == 1 && m.b_is_airborne_from_jump, "OnJumped not recorded");
    while m.mode == Mode::Falling {
        pos = step(&mut m, pos, FVector::ZERO);
    }
    assert!(m.b_was_last_land_from_jump && !m.b_is_airborne_from_jump, "Landed flags wrong");
    assert_eq!(m.get_movement_restriction(), restriction::PARTIAL_SPRINT);
    let mut t = 0.0;
    while m.world_time < m.last_landed_time + 0.1 {
        pos = step(&mut m, pos, FVector::ZERO);
        t += DT;
        assert!(t <= 1.0, "clock stuck");
    }
    assert_eq!(m.get_movement_restriction(), 0);
    m.motion_restriction = restriction::WALK;
    m.last_landed_time = m.world_time;
    assert_eq!(m.get_movement_restriction(), restriction::WALK, "motion restriction not kept");
}

/// No jump in the first JumpCooldown s of world time (LastLand zero-filled), allowed from LastLand + JumpCooldown on
/// (AAdvancedCharacter::CanJumpInternal_Implementation rva=0x1459fb0)
#[test]
fn jump_cooldown_from_world_start() {
    let mut m = m();
    assert!(!m.can_jump());
    m.world_time = m.jump_cooldown;
    assert!(m.can_jump(), "boundary: TimeSeconds == LastLand + JumpCooldown jumps");
    m.mode = Mode::Falling;
    assert!(!m.can_jump(), "jump while falling");
}

/// GetMovementRestriction == NoMovement: IsMoveInputIgnored rva=0x154afe0, CanJumpInternal_Implementation
/// rva=0x1532630, CanCrouch rva=0x1532230
#[test]
fn no_movement_restriction() {
    let mut m = m();
    let mut pos = FVector::ZERO;
    for _ in 0..30 {
        pos = step(&mut m, pos, FVector::ZERO);
    }
    m.motion_restriction = restriction::NO_MOVEMENT;
    for _ in 0..30 {
        pos = step(&mut m, pos, FWD);
    }
    assert_eq!(m.velocity.length(), 0.0, "moved under NoMovement");
    assert!(!m.can_jump() && !m.can_crouch());
    m.motion_restriction = 0;
    for _ in 0..30 {
        pos = step(&mut m, pos, FWD);
    }
    assert!(m.velocity.length() > 0.0, "no movement after the restriction ended");
}

/// UMordhauMovementComponent::LODTick rva=0x14c4d60: restriction Walk -> bSprintIsAllowed false (walk speed cap)
#[test]
fn walk_restriction_blocks_sprint() {
    let mut m = m();
    let mut pos = FVector::ZERO;
    m.motion_restriction = restriction::WALK;
    m.wants_sprint = true;
    for _ in 0..240 {
        pos = step(&mut m, pos, FWD);
    }
    let top = m.cfg.max_walk_speed * m.get_speed_factor(0.0);
    assert!((m.velocity.length() as f64) <= top * 1.001, "sprinting under Walk");
    assert!(m.sprint_state < Sprint::Sprint);
}

/// UAdvancedCharacterMovement::ConstrainInputAcceleration rva=0x145aee0: bIgnoreMovementInput -> zero acceleration
#[test]
fn ignore_movement_input() {
    let mut m = m();
    m.b_ignore_movement_input = true;
    let mut pos = FVector::ZERO;
    for _ in 0..30 {
        pos = step(&mut m, pos, FWD);
    }
    assert_eq!(m.velocity.length(), 0.0);
}

/// Knockback rva=0x14c2820, ApplyAccumulatedForces rva=0x2f6e650, PostCharacterMovementTick rva=0x14d0710,
/// InitializeComponent rva=0x14c1300
#[test]
fn knockback() {
    let mut m = m();
    let mut pos = FVector::ZERO;
    for _ in 0..10 {
        pos = step(&mut m, pos, FVector::ZERO);
    }
    let (base_g, base_f) = (m.cfg.ground_friction, m.cfg.falling_lateral_friction);
    assert!(m.base_ground_friction == base_g && m.base_falling_lateral_friction == base_f);
    m.knockback(FVector::new(m.cfg.max_walk_speed as f32, 0.0, 0.0));
    assert!(m.is_in_knockback());
    assert_eq!(m.cfg.ground_friction, m.fx.knockback_ground_friction);
    assert_eq!(m.cfg.falling_lateral_friction, m.fx.knockback_falling_lateral_friction);
    assert_eq!(m.pending_impulse.yf(), m.fx.knockback_up_impulse as f32 as f64);
    pos = step(&mut m, pos, FVector::ZERO);
    assert!(m.velocity.x > 0.0, "knockback did not push");
    assert_eq!(m.pending_impulse, FVector::ZERO);
    let mut t = DT;
    while m.is_in_knockback() {
        pos = step(&mut m, pos, FVector::ZERO);
        t += DT;
        assert!(t <= 5.0);
    }
    assert!((t - m.fx.knockback_duration).abs() <= DT + 1e-4, "knockback lasted {t}");
    assert!(m.cfg.ground_friction == base_g && m.cfg.falling_lateral_friction == base_f, "frictions not restored");
    m.mode = Mode::Falling;
    m.knockback(FVector::ZERO);
    assert_eq!(m.pending_impulse.y, 0.0, "up impulse while falling");
    let mut w = MordhauMovement::new(&records());
    w.add_impulse(FVector::new(0.0, (50.0 - w.gravity() * DT) as f32, 0.0), true);
    w.apply_accumulated_forces(DT);
    assert_eq!(w.mode, Mode::Falling, "upward impulse kept walking");
}

/// CrouchPressed rva=0x1538040 / CrouchReleased rva=0x1538100 (m.ToggleCrouch) and the LODTick rva=0x154c390 cooldown
#[test]
fn crouch_input_and_cooldown() {
    let mut m = m();
    m.crouch_pressed();
    assert!(m.wants_crouch);
    assert_eq!(m.update_crouch(), None, "crouched at world time 0");
    m.world_time = m.ch.crouch_cooldown;
    assert_eq!(m.update_crouch(), None, "crouched at exactly CrouchCooldown (needs strictly more)");
    m.world_time += DT;
    assert_eq!(m.update_crouch(), Some(true));
    m.crouched = true;
    m.crouch_released();
    assert!(!m.wants_crouch);
    assert_eq!(m.update_crouch(), None, "uncrouched inside the cooldown");
    m.world_time += m.ch.crouch_cooldown + DT;
    assert_eq!(m.update_crouch(), Some(false));
    m.crouched = false;
    m.toggle_crouch = 1;
    m.crouch_pressed();
    m.crouch_released();
    assert!(m.wants_crouch, "toggle: release cleared crouch");
    m.crouch_pressed();
    assert!(!m.wants_crouch, "toggle: second press did not clear");
}

/// SprintPressed rva=0x156da50 / SprintReleased rva=0x156dba0 / MoveForward rva=0x1550170 (m.ToggleSprint)
#[test]
fn sprint_input_toggle() {
    let mut m = m();
    m.sprint_pressed();
    m.sprint_released();
    assert!(!m.wants_sprint);
    m.toggle_sprint = 1;
    m.sprint_pressed();
    m.sprint_released();
    assert!(m.wants_sprint, "toggle: release cleared sprint");
    m.move_forward_axis(1.0);
    assert!(m.wants_sprint);
    m.move_forward_axis(0.0);
    assert!(!m.wants_sprint, "toggle: zero forward axis kept sprint");
    m.sprint_pressed();
    m.sprint_pressed();
    assert!(!m.wants_sprint, "toggle: second press kept sprint");
}

/// MordhauCharacter.step_model: only edges reach SprintPressed / CrouchPressed (holding a button is one press)
#[test]
fn step_input_edges() {
    let mut m = m();
    m.toggle_sprint = 1;
    let mut inp = CharacterInput::default();
    for _ in 0..5 {
        inp.step(&mut m, DT, 1.0, 0.0, false, true, false);
    }
    assert!(m.wants_sprint, "held toggle-sprint key toggled more than once");
    inp.step(&mut m, DT, 1.0, 0.0, false, false, false);
    inp.step(&mut m, DT, 1.0, 0.0, false, true, false);
    assert!(!m.wants_sprint, "second press did not toggle off");
    while m.world_time <= m.ch.crouch_cooldown {
        inp.step(&mut m, DT, 0.0, 0.0, false, false, false);
    }
    let r = inp.step(&mut m, DT, 0.0, 0.0, false, false, true);
    assert_eq!(r.crouch, Some(true), "crouch edge after the cooldown");
    assert!(m.crouched);
}

/// AMordhauCharacter::LODTick rva=0x154c390: FallingTime past FallingTimeToRagdoll -> Trip()
#[test]
fn long_fall_trips() {
    let mut m = m();
    m.mode = Mode::Falling;
    let mut t = 0.0;
    while m.trips == 0 && t < 10.0 {
        m.tick(DT, FVector::ZERO, FWD);
        t += DT;
    }
    assert_eq!(m.trips, 1);
    assert!(t >= m.ch.falling_time_to_ragdoll && t <= m.ch.falling_time_to_ragdoll + 3.0 * DT, "tripped after {t}");
    assert_eq!(m.falling_time, 0.0, "FallingTime not reset after Trip");
}

struct Sink {
    restriction: i64,
    dead: bool,
    cost: f64,
    stamina: Vec<i64>,
    regen_stops: Vec<f64>,
    damage: Vec<f64>,
}
impl CombatSide for Sink {
    fn motion_movement_restriction(&self) -> i64 {
        self.restriction
    }
    fn dead(&self) -> bool {
        self.dead
    }
    fn jump_stamina_cost(&self) -> f64 {
        self.cost
    }
    fn offset_stamina(&mut self, amount: i64) {
        self.stamina.push(amount);
    }
    fn stop_stamina_regen(&mut self, delay: f64) {
        self.regen_stops.push(delay);
    }
    fn take_fall_damage(&mut self, amount: f64) {
        self.damage.push(amount);
    }
}

/// Fighter.apply_movement_events: OnJumped_Implementation rva=0x1554610 jump stamina (-(int)JumpStaminaCost, then
/// StopRegeneration(0)) and CheckFallDamage amounts as Fall damage reach the combat side; restriction and dead come back
#[test]
fn movement_events_reach_combat() {
    let mut m = m();
    let mut s = Sink { restriction: restriction::WALK, dead: false, cost: m.ch.jump_stamina_cost, stamina: vec![], regen_stops: vec![], damage: vec![] };
    m.jumps = 2;
    m.fall_damage.push(30.0);
    m.ragdoll_changes.push(true);
    apply_movement_events(&mut m, Some(&mut s));
    let c = -(m.ch.jump_stamina_cost as i64);
    assert_eq!(s.stamina, vec![c, c]);
    assert_eq!(s.regen_stops, vec![0.0, 0.0]);
    assert_eq!(s.damage, vec![30.0]);
    assert_eq!(m.motion_restriction, restriction::WALK);
    assert!(m.jumps == 0 && m.fall_damage.is_empty() && m.ragdoll_changes.is_empty());
    s.dead = true;
    apply_movement_events(&mut m, Some(&mut s));
    assert!(m.dead);
    m.jumps = 1;
    apply_movement_events::<Sink>(&mut m, None);
    assert_eq!(m.jumps, 0, "no combat side: events drained");
}

/// Trip rva=0x14a78c0 / SetIsRagdollFalling rva=0x14a0980, the get-up in UAdvancedCharacterMovement::TickComponent
/// rva=0x14a3e80 and IsRagdollFallingOrGettingUp rva=0x1487ca0
#[test]
fn ragdoll_trip_and_get_up() {
    let mut m = m();
    let mut pos = FVector::ZERO;
    for _ in 0..30 {
        pos = step(&mut m, pos, FVector::ZERO);
    }
    assert!(!m.is_ragdoll_falling_or_getting_up(), "getting up at spawn");
    assert!(m.trip());
    assert!(!m.trip(), "second trip while ragdoll falling");
    assert!(m.is_move_input_ignored());
    assert!(!m.can_jump() && !m.can_crouch());
    let t0 = m.world_time;
    let mut got_up = -1.0;
    for _ in 0..600 {
        pos = step(&mut m, pos, FWD);
        if !m.ragdoll_falling {
            got_up = m.world_time;
            break;
        }
    }
    assert!(got_up >= 0.0, "never got up");
    assert_eq!(m.velocity.length(), 0.0, "moved during the ragdoll");
    let earliest = t0 + m.ch.ragdoll_falling_min_time.max(m.ch.ragdoll_falling_time_at_min_velocity_to_get_up);
    assert!(got_up >= earliest && got_up <= earliest + 2.0 * DT, "got up at {got_up}, want just after {earliest}");
    assert!(m.is_ragdoll_falling_or_getting_up() && !m.can_jump());
    while m.world_time < got_up + m.ch.ragdoll_falling_get_up_duration {
        assert!(m.is_ragdoll_falling_or_getting_up(), "getting-up ended early");
        pos = step(&mut m, pos, FVector::ZERO);
    }
    assert!(!m.is_ragdoll_falling_or_getting_up());
    assert_eq!(m.ragdoll_changes, vec![true, false]);
    m.dead = true;
    assert!(!m.trip(), "dead character tripped");
}

/// Faster than RagdollFallingMinVelocityToGetUp resets StillTimeWhileRagdollFalling (TickComponent 0x1414a43ba)
#[test]
fn ragdoll_no_get_up_while_moving() {
    let mut m = m();
    let mut pos = FVector::ZERO;
    for _ in 0..10 {
        pos = step(&mut m, pos, FVector::ZERO);
    }
    let _ = pos;
    m.trip();
    let v = m.ch.ragdoll_falling_min_velocity_to_get_up;
    m.world_time += m.ch.ragdoll_falling_min_time + 1.0;
    m.velocity = FVector::new((v * 1.01) as f32, 0.0, 0.0);
    m.still_time_while_ragdoll_falling = 10.0;
    m.tick_ragdoll_get_up(DT);
    assert!(m.still_time_while_ragdoll_falling == 0.0 && m.ragdoll_falling);
    m.velocity = FVector::new(v as f32, 0.0, 0.0); // boundary: |V|^2 == Min^2 counts as still
    m.tick_ragdoll_get_up(m.ch.ragdoll_falling_time_at_min_velocity_to_get_up);
    assert!(!m.ragdoll_falling, "no get-up at exactly the min velocity");
}

/// GetMaxSpeed rva=0x14be1d0 / GetMaxAcceleration rva=0x14be0b0 / GetSpeedFactor rva=0x14bf9a0 per sprint state
#[test]
fn max_speed_and_acceleration_by_state() {
    let mut m = m();
    let c = m.cfg.clone();
    assert_eq!(m.get_max_speed(), c.max_walk_speed);
    m.sprint_state = Sprint::Backpedal;
    assert_eq!(m.get_speed_factor(0.0), 1.0, "no armor / equipment: factor 1 (ctor +0xd04 / +0xd08 = 1.0)");
    assert_eq!(m.get_max_speed(), c.backpedal_modifier * c.max_walk_speed);
    m.sprint_state = Sprint::Sideways;
    assert_eq!(m.get_max_speed(), mh_character::compat::movement::NATIVE_SIDEWAYS_MODIFIER * c.max_walk_speed);
    m.sprint_state = Sprint::Partial;
    assert_eq!(m.get_max_speed(), c.partial_sprint_modifier * c.max_walk_speed);
    assert_eq!(m.get_max_acceleration(), c.partial_sprint_acceleration);
    m.sprint_state = Sprint::Sprint;
    m.sprint_time = c.sprint_time_to_reach_max_sprint * 2.0;
    assert!(approx(m.get_max_speed(), c.sprint_modifier * c.max_walk_speed));
    assert_eq!(m.get_max_acceleration(), c.sprint_acceleration);
    m.armor_accel = 0.5;
    assert_eq!(m.get_max_acceleration(), 0.5 * c.sprint_acceleration);
    m.crouched = true;
    assert_eq!(m.get_max_acceleration(), c.walk_acceleration, "crouch acceleration is unscaled WalkAcceleration");
    m.sprint_state = Sprint::Forward;
    assert_eq!(m.get_max_speed(), c.max_walk_speed_crouched);
    m.mode = Mode::Falling;
    assert_eq!(m.get_max_speed(), c.max_speed_falling);
    m.knockback_time = 0.1;
    assert_eq!(m.get_max_speed(), c.max_walk_speed, "falling in knockback takes the UE max speed");
    assert_eq!(m.get_max_acceleration(), c.max_acceleration);
}

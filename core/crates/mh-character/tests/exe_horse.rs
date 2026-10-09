//! Horses in exe mode (exe_horse.rs): BP_Horse's data, the gears and AHorse::MoveForward's shifting, turning through
//! the speed curves and PhysicsRotation, the BP Jump bindings (jump fast, rear slow), head-on rearing, the soft
//! bubbles, mounting. Data: extract/json (BP_Horse, BP_VehicleHorse, the FC_ curves; the game's, ignored path); the
//! tests SKIP (pass) without it. Expected values come from that data and the disassembled formulas.

use mh_character::exe::{ExeMovement, Mode};
use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::exe_horse::{curve_keys_from_json, horse_character_records, other_horse, HorseCfg, HorseInput};
use mh_character::uemath::v;
use mh_character::uequat::{finterp_constant_to, fixed_turn, rich_curve_eval, RichKey};
use mh_character::world::{BoxWorld, World};
use mh_character::{CharacterRecords, CharacterSource, OtherPawn, RecordsJson};
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;

fn root() -> PathBuf {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    dir.join("../../..")
}

fn read(rel: &str) -> Option<String> {
    std::fs::read_to_string(root().join("extract/json").join(rel)).ok()
}

struct Data {
    base: CharacterRecords,
    bp: String,
    cfg: HorseCfg,
}

fn data() -> Option<Data> {
    let bp = read("Mordhau/Content/Mordhau/Blueprints/Interactables/Animals/BP_Horse.json")?;
    let veh = read("Mordhau/Content/Mordhau/Blueprints/VehicleComponents/BP_VehicleHorse.json");
    let curve = |p: &str| read(&format!("{p}.json"));
    let cfg = HorseCfg::from_json(&bp, veh.as_deref(), &curve).unwrap();
    let p = root().join("core/tests/golden/character/records.json");
    let base = RecordsJson(&std::fs::read_to_string(p).unwrap()).load().unwrap();
    Some(Data { base, bp, cfg })
}

fn flat() -> BoxWorld {
    let mut w = BoxWorld::new();
    w.add_box(v(-50000.0, -50000.0, -200.0), v(50000.0, 50000.0, 0.0));
    w
}

fn horse(d: &Data, w: &dyn World) -> ExeMovement {
    let rec = horse_character_records(&d.base, &d.bp).unwrap();
    let mut m = ExeMovement::new_horse(&rec, &d.bp, d.cfg.clone(), v(0.0, 0.0, 120.0 + AVG_FLOOR_DIST)).unwrap();
    m.world_time = 10.0;
    m.horse_frame(w, DT, &HorseInput::default());
    m
}

fn run(m: &mut ExeMovement, w: &dyn World, n: usize, inp: HorseInput) {
    for _ in 0..n {
        m.horse_frame(w, DT, &inp);
    }
}

fn speed(m: &ExeMovement) -> f32 {
    (m.velocity.x * m.velocity.x + m.velocity.y * m.velocity.y).sqrt()
}

#[test]
fn records_from_bp_horse() {
    let Some(d) = data() else { return eprintln!("SKIP: no extract/json") };
    let g: Vec<f32> = d.cfg.gears.iter().map(|g| g.max_speed).collect();
    assert_eq!(g, vec![103.0, 206.0, 512.0, 1000.0]);
    assert_eq!(d.cfg.gears.iter().map(|g| g.allow_jump).collect::<Vec<_>>(), vec![false, false, true, true]);
    assert!(d.cfg.turning_factor_curve.is_some() && d.cfg.turning_accel_curve.is_some() && d.cfg.turning_brake_curve.is_some());
    assert_eq!(d.cfg.head_on_min_speed_to_rear, 600.0);
    assert_eq!(d.cfg.rearing_duration, 1.5);
    assert_eq!(d.cfg.attach_offset, v(0.0, 0.0, 175.0));
    let w = flat();
    let m = horse(&d, &w);
    assert_eq!((m.capsule_radius(), m.half_height()), (55.0, 120.0));
    assert_eq!(m.c.max_walk_speed, 103.0, "InitializeComponent applies gear 0");
    assert_eq!(m.e.braking_deceleration_walking, 700.0);
    assert_eq!(m.c.ground_friction, 11.0);
    assert_eq!(m.mode, Mode::Walking);
}

/// FRichCurve::EvalForTwoKeys rva=0x3024fa0, the unweighted cubic: the Bezier through P0, P0 + Leave * d / 3,
/// P3 - Arrive * d / 3, P3 (de Casteljau in f64 for the reference)
#[test]
fn cubic_curve_matches_bezier() {
    let Some(j) = read("Mordhau/Content/Mordhau/Blueprints/Interactables/Animals/FC_HorseTurnFactorCurveLancer.json") else { return };
    let k = curve_keys_from_json(&j).unwrap();
    assert_eq!(k[0].interp, 2);
    for t in [0.0f32, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
        let got = rich_curve_eval(&k, t) as f64;
        let (p0, p3) = (k[0].value as f64, k[1].value as f64);
        let d = (k[1].time - k[0].time) as f64;
        let (p1, p2) = (p0 + k[0].leave as f64 * d / 3.0, p3 - k[1].arrive as f64 * d / 3.0);
        let a = ((t - k[0].time) as f64) / d;
        let l = |x: f64, y: f64| x + (y - x) * a;
        let want = l(l(l(p0, p1), l(p1, p2)), l(l(p1, p2), l(p2, p3)));
        assert!((got - want).abs() < 2e-6, "t {t}: {got} vs {want}");
    }
    // linear key segment and constant extrapolation
    let lin = [RichKey { interp: 0, weight_mode: 0, time: 415.0, value: 0.0, arrive: 0.0, leave: 0.0 }, RichKey { interp: 0, weight_mode: 0, time: 1000.0, value: 1.0, arrive: 0.0, leave: 0.0 }];
    assert_eq!(rich_curve_eval(&lin, 100.0), 0.0);
    assert_eq!(rich_curve_eval(&lin, 2000.0), 1.0);
    assert_eq!(rich_curve_eval(&lin, 707.5), (1.0 - 0.0) * ((707.5f32 - 415.0) / (1000.0 - 415.0)) + 0.0);
}

/// AHorse::MoveForward rva=0x14ffae0: holding forward from a standstill shifts up a gear each time the horse is within
/// 5 cm/s of the gear's MaxSpeed; the gear's MaxSpeed caps it; the horse keeps its gear without input; backward shifts
/// down when under MaxSpeed + 5, at a stop to reverse (DesiredGear -1, gear 0 backwards)
#[test]
fn gears_shift_up_and_down() {
    let Some(d) = data() else { return };
    let w = flat();
    let mut m = horse(&d, &w);
    let fwd = HorseInput { fwd: 1.0, ..Default::default() };
    let mut reached = Vec::new();
    for _ in 0..60 * 12 {
        m.horse_frame(&w, DT, &fwd);
        let g = m.horse.as_ref().unwrap().gear;
        if reached.last() != Some(&g) {
            reached.push(g);
        }
    }
    assert_eq!(reached, vec![0, 1, 2, 3], "gears seen {reached:?}");
    assert!((speed(&m) - 1000.0).abs() < 1.0, "top speed {}", speed(&m));
    // no input: the horse keeps going in its gear
    run(&mut m, &w, 60, HorseInput::default());
    assert!((speed(&m) - 1000.0).abs() < 1.0);
    // backward: down through the gears to a stop, then reverse
    run(&mut m, &w, 60 * 15, HorseInput { fwd: -1.0, ..Default::default() });
    let h = m.horse.as_ref().unwrap();
    assert_eq!(h.desired_gear, -1);
    assert_eq!(h.gear, 0);
    let (_, f, _) = mh_character::uequat::actor_axes(m.yaw);
    assert!(m.velocity.x * f.x + m.velocity.y * f.y < -100.0, "not reversing: {:?}", m.velocity);
}

/// MoveRight -> Turn -> PendingTurnValue; LODTick: factor curve(speed ratio), FInterpConstantTo(TurningVelocity,
/// PendingTurnValue, dt, braking curve (signs differ: TurningVelocity 0)); AddTurnDegrees(dt * TV * 90) on the control
/// yaw; PhysicsRotation FixedTurns the actor yaw to it at 360 deg/s
#[test]
fn first_turn_frame_is_exact() {
    let Some(d) = data() else { return };
    let w = flat();
    let mut m = horse(&d, &w);
    let yaw0 = m.yaw;
    m.horse_frame(&w, DT, &HorseInput { right: 1.0, ..Default::default() });
    let c = &d.cfg;
    let ratio = 0.0f32; // standing
    let ptv = rich_curve_eval(c.turning_factor_curve.as_ref().unwrap(), ratio) * 1.0;
    let acc = rich_curve_eval(c.turning_brake_curve.as_ref().unwrap(), ratio);
    let tv = finterp_constant_to(0.0, ptv, DT, acc);
    let delta = (DT * tv) * 90.0;
    let control = mh_character::uequat::fmod(yaw0 + delta, 360.0);
    let want = fixed_turn(yaw0, control, 360.0 * DT);
    assert_eq!(m.horse.as_ref().unwrap().turning_velocity, tv);
    assert_eq!(m.yaw, want);
    // holding the turn: the yaw keeps increasing, never faster than 90 deg/s times the factor
    let mut prev = m.yaw;
    for _ in 0..60 {
        m.horse_frame(&w, DT, &HorseInput { right: 1.0, ..Default::default() });
        let d = mh_character::uequat::unwind_degrees(m.yaw - prev);
        assert!(d > 0.0 && d <= 90.0 * 1.45 * DT + 1e-3, "yaw step {d}");
        prev = m.yaw;
    }
}

/// BP_Horse Jump (both bindings on press): fast (over half the gear's MaxSpeed) -> Jump(), which DoJump allows from gear
/// 2 (bAllowJump); slow (gear < 2) -> RequestRearing, during which move input is ignored
#[test]
fn jump_when_fast_rear_when_slow() {
    let Some(d) = data() else { return };
    let w = flat();
    let mut m = horse(&d, &w);
    m.horse_frame(&w, DT, &HorseInput { jump: true, ..Default::default() });
    assert_eq!(m.mode, Mode::Walking);
    assert_eq!(m.horse.as_ref().unwrap().rear_requests, 1);
    assert!(m.horse_is_rearing());
    assert_eq!(m.horse.as_ref().unwrap().desired_gear, -1);
    run(&mut m, &w, 10, HorseInput { fwd: 1.0, ..Default::default() });
    assert!(speed(&m) < 1.0, "moved while rearing: {}", speed(&m));
    // after the rearing: run up to gear 3, then jump
    run(&mut m, &w, 120, HorseInput::default());
    let mut n = 0;
    while m.horse.as_ref().unwrap().gear < 3 && n < 60 * 12 {
        m.horse_frame(&w, DT, &HorseInput { fwd: 1.0, ..Default::default() });
        n += 1;
    }
    m.horse_frame(&w, DT, &HorseInput { fwd: 1.0, jump: true, ..Default::default() });
    assert_eq!(m.mode, Mode::Falling, "no jump at gear 3");
    assert_eq!(m.horse.as_ref().unwrap().rear_requests, 1, "rearing at speed");
}

/// running into a wall at speed: the front capsule sweep hits, the avoidance direction is head-on (>= 150 deg) and the
/// speed is over HeadOnCollisionMinSpeedToRear -> RequestRearing
#[test]
fn head_on_wall_rears() {
    let Some(d) = data() else { return };
    let mut w = flat();
    w.add_box(v(6000.0, -2000.0, 0.0), v(6200.0, 2000.0, 600.0));
    let mut m = horse(&d, &w);
    let mut n = 0;
    while m.horse.as_ref().unwrap().rear_requests == 0 && n < 60 * 20 {
        m.horse_frame(&w, DT, &HorseInput { fwd: 1.0, ..Default::default() });
        n += 1;
    }
    let h = m.horse.as_ref().unwrap();
    assert_eq!(h.rear_requests, 1, "never reared; x {}", m.location.x);
    assert!(h.last_front_hit);
    assert!(m.location.x < 6000.0 - 55.0);
    assert!(!w.overlap_capsule(m.location, 54.9, 119.9));
}

/// the soft bubble of a horse standing ahead deflects a slow horse (gear 0, turning: the avoidance turn) and a
/// character ahead counts only while the bump damage curve gives 0 (under 415 cm/s)
#[test]
fn soft_bubbles() {
    let Some(d) = data() else { return };
    let w = flat();
    let mut m = horse(&d, &w);
    m.others = vec![other_horse(&d.cfg, v(250.0, 30.0, m.location.z), 180.0)];
    for _ in 0..30 {
        m.horse_frame(&w, DT, &HorseInput { fwd: 1.0, ..Default::default() });
    }
    let p = m.horse.as_ref().unwrap().last_push;
    assert!(p.x != 0.0 || p.y != 0.0, "no push from the horse ahead");
    // a character: in reach at a walk, ignored at a gallop (bump damage > 0 above 415 cm/s)
    let mut ch = OtherPawn::enemy(&m, v(m.location.x + 120.0, m.location.y, m.location.z), 0.0);
    ch.has_movement = true;
    m.others = vec![ch.clone()];
    m.horse_frame(&w, DT, &HorseInput::default());
    assert!(m.horse.as_ref().unwrap().last_push != v(0.0, 0.0, 0.0));
    assert!(m.horse_bump_damage(v(800.0, 0.0, 0.0)) > 0.0 && m.horse_bump_damage(v(400.0, 0.0, 0.0)) == 0.0);
    m.velocity = v(800.0, 0.0, 0.0);
    ch.location = v(m.location.x + 120.0, m.location.y, m.location.z);
    m.others = vec![ch];
    m.horse_frame(&w, DT, &HorseInput::default());
    assert_eq!(m.horse.as_ref().unwrap().last_push, v(0.0, 0.0, 0.0));
}

/// UMordhauVehicleComponent::CanInteract / CanDrive and the seat (BP_VehicleHorse: MinXYDistanceToEnter 200, MinZ
/// (-100, 100), MinimumInteractableVelocity 600, AttachSocketOffset (0, 0, 175))
#[test]
fn mounting() {
    let Some(d) = data() else { return };
    let w = flat();
    let mut m = horse(&d, &w);
    let l = m.location;
    assert!(m.horse_can_interact(v(l.x + 150.0, l.y, l.z - 30.0), true));
    assert!(!m.horse_can_interact(v(l.x + 200.0, l.y, l.z), true), "at MinXYDistanceToEnter");
    assert!(!m.horse_can_interact(v(l.x, l.y, l.z + 101.0), true), "over MinZ.Y");
    assert!(m.horse_can_interact(v(l.x, l.y, l.z + 99.0), true), "under MinZ.Y");
    assert!(!m.horse_can_interact(v(l.x + 10.0, l.y, l.z), false));
    let (seat, _) = m.horse_start_driving(7).unwrap();
    assert_eq!(seat, v(l.x + d.cfg.mesh_relative.x, l.y + d.cfg.mesh_relative.y, l.z + d.cfg.mesh_relative.z + 175.0));
    assert!(m.horse_start_driving(8).is_none(), "one driver");
    assert!(!m.horse_can_interact(v(l.x + 10.0, l.y, l.z), true), "has a driver");
    let (exit, _) = m.horse_stop_driving(&w, seat, 0.0, Some(v(l.x, l.y + 120.0, l.z)), 50.0, 96.0);
    assert_eq!(exit, v(l.x, l.y + 120.0, l.z));
    assert_eq!(m.horse.as_ref().unwrap().driver, None);
}

/// a character walking at a horse's flank is pushed by the horse's soft bubble (UMordhauMovementComponent::LODTick's
/// AHorse branch: the bubble at the horse's location + SoftBubbleEllipseRelativeLocation, forcing: no dodge cancel)
#[test]
fn character_bubble_includes_horses() {
    let Some(d) = data() else { return eprintln!("SKIP: no extract/json") };
    let w = flat();
    let mut m = mh_character::ExeMovement::new(&d.base, v(0.0, 0.0, 96.0 + mh_character::exe_cmc::AVG_FLOOR_DIST));
    m.world_time = 10.0;
    m.frame(&w, 1.0 / 60.0, &mh_character::ExeInput::default());
    // the horse broadside 60 cm ahead, facing +Y: walking +X runs into its bubble
    let c = d.cfg.soft_bubble_rel;
    let horse = mh_character::exe_horse::other_horse(&d.cfg, v(60.0 - c.y, -c.x, m.location.z), 90.0);
    m.others = vec![horse.clone()];
    m.frame(&w, 1.0 / 60.0, &mh_character::ExeInput { fwd: 1.0, ..Default::default() });
    let pushed = m.velocity;
    let mut free = mh_character::ExeMovement::new(&d.base, v(0.0, 0.0, 96.0 + mh_character::exe_cmc::AVG_FLOOR_DIST));
    free.world_time = 10.0;
    free.frame(&w, 1.0 / 60.0, &mh_character::ExeInput::default());
    free.frame(&w, 1.0 / 60.0, &mh_character::ExeInput { fwd: 1.0, ..Default::default() });
    println!("pushed {pushed:?} free {:?}", free.velocity);
    assert!(!m.was_dodge_canceled, "a horse forces correction: no dodge cancel");
    assert!(pushed != free.velocity, "the horse's bubble bends the input");
    // a horse without a UHorseMovementComponent is skipped
    let mut h2 = horse;
    h2.mordhau_movement = false;
    let mut m2 = mh_character::ExeMovement::new(&d.base, v(0.0, 0.0, 96.0 + mh_character::exe_cmc::AVG_FLOOR_DIST));
    m2.world_time = 10.0;
    m2.frame(&w, 1.0 / 60.0, &mh_character::ExeInput::default());
    m2.others = vec![h2];
    m2.frame(&w, 1.0 / 60.0, &mh_character::ExeInput { fwd: 1.0, ..Default::default() });
    assert_eq!(m2.velocity, free.velocity);
}

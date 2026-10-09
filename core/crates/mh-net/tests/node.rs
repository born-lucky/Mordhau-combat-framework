//! The host / join API (node.rs) in one process over real localhost UDP: a ServerNode and two ClientNodes polled in
//! turn on a shared clock (no simulated delay), a Duel (DuelMode) on real combat. Covers what r5 added to the nodes:
//!   - relevancy: a pawn beyond NetCullDistanceSquared stops being relevant, its movement channel closes after
//!     RelevantTimeout (the client destroys the proxy) and reopens when it is relevant again
//!   - horses: a server-spawned horse appears as a proxy everywhere; once a client drives it, that client predicts it
//!     with horse moves, the server follows, the other client's proxy tracks it
//!   - climbing: the owning client's jump at a ledge sends ServerSetClimbLocation + RequestClimb (FNetMotion::Climbing);
//!     server and owner run the climb, the other client sees the UClimbingMotion and the pawn ends on the ledge
//! Data: core/tests/golden/spec.json, character/records.json, mode/duel_rooms.jsonl, godot/data_gen/mode/mode_kismet.json
//! (horses: extract/json BP_Horse); absent -> SKIP.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::uemath::v;
use mh_character::{BoxWorld, CharacterRecords, CharacterSource, ExeInput, RecordsJson};
use mh_mode::{GameMode, Kismet, ModeData};
use mh_net::movement::{NetMoveCfg, SmoothCfg};
use mh_net::node::{ClientNode, HorseTemplate, MovementHost, ServerNode};
use mh_net::NetWorld;
use mordhau_core::combat::World;
use mordhau_core::data::{RecordsJsonExe, Spec, SpecSource};
use mordhau_core::ue::FVector;
use serde_json::Value;

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
const DT: f64 = 1.0 / 60.0;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

struct Data {
    spec: Rc<Spec>,
    rec: CharacterRecords,
    mode: (ModeData, String),
}

fn data() -> Option<Data> {
    let r = root();
    let spec = std::fs::read_to_string(r.join("core/tests/golden/spec.json")).ok()?;
    let rec = std::fs::read_to_string(r.join("core/tests/golden/character/records.json")).ok()?;
    let k = std::fs::read_to_string(r.join("godot/data_gen/mode/mode_kismet.json")).ok()?;
    let g = std::fs::read_to_string(r.join("core/tests/golden/mode/duel_rooms.jsonl")).ok()?;
    let mut header: Value = serde_json::from_str(g.lines().nth(1)?).ok()?;
    mordhau_core::data::decode_exact(&mut header);
    Some(Data {
        spec: Rc::new(RecordsJsonExe(&spec).load_spec().expect("spec")),
        rec: RecordsJson(&rec).load().expect("records"),
        mode: (serde_json::from_value(header["data"].clone()).expect("ModeData"), k),
    })
}

fn horse_template(base: &CharacterRecords) -> Option<HorseTemplate> {
    let r = root();
    let read = |rel: &str| std::fs::read_to_string(r.join("extract/json").join(rel)).ok();
    let bp = read("Mordhau/Content/Mordhau/Blueprints/Interactables/Animals/BP_Horse.json")?;
    let veh = read("Mordhau/Content/Mordhau/Blueprints/VehicleComponents/BP_VehicleHorse.json");
    let cfg = mh_character::exe_horse::HorseCfg::from_json(&bp, veh.as_deref(), &|p: &str| read(&format!("{p}.json"))).ok()?;
    let records = mh_character::exe_horse::horse_character_records(base, &bp).ok()?;
    Some(HorseTemplate { records, bp_horse: bp, cfg })
}

/// the floor, and a 120 cm block 70..400 cm in front of A (a ledge to climb); A at the origin facing +X, B 1000 cm
/// behind it facing +X
fn host(d: &Data, horse: bool) -> MovementHost {
    let mut level = BoxWorld::new();
    level.add_box(v(-50000.0, -50000.0, -200.0), v(50000.0, 50000.0, 0.0));
    level.add_box(v(70.0, -100.0, 0.0), v(400.0, 100.0, 120.0));
    MovementHost {
        level: Box::new(level),
        records: d.rec.clone(),
        spawn: Box::new(|n: &str| {
            let z = 96.0 + AVG_FLOOR_DIST;
            if n == "A" { (FVector { x: 0.0, y: 0.0, z }, 0.0) } else { (FVector { x: -1000.0, y: 0.0, z }, 0.0) }
        }),
        cfg: NetMoveCfg::default(),
        smooth: SmoothCfg::default(),
        horse: if horse { horse_template(&d.rec) } else { None },
    }
}

struct Net {
    s: ServerNode<World, GameMode>,
    a: ClientNode<World>,
    b: ClientNode<World>,
    now: f64,
}

fn net(d: &Data, horse: bool) -> Net {
    let mode = GameMode::new(d.mode.0.clone(), Arc::new(Kismet::from_json(&d.mode.1).unwrap()), mordhau_core::ue::CrtRand::new(1));
    let mut s = ServerNode::bind("127.0.0.1:0", World::new(d.spec.clone(), DT), mode, LS, "", DT, 2).expect("bind");
    s.movement = Some(host(d, horse));
    let addr = s.local_addr();
    let mut a = ClientNode::join(addr, 1, "A", "", World::new(d.spec.clone(), DT), DT).expect("join");
    let mut b = ClientNode::join(addr, 2, "B", "", World::new(d.spec.clone(), DT), DT).expect("join");
    a.movement = Some(host(d, horse));
    b.movement = Some(host(d, horse));
    Net { s, a, b, now: 0.0 }
}

impl Net {
    /// one frame on every machine (the sockets are pumped in between so localhost datagrams arrive)
    fn frame(&mut self) {
        self.now += DT;
        for _ in 0..3 {
            self.s.poll(self.now, &mut |_, _| {});
            self.a.poll(self.now);
            self.b.poll(self.now);
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
    }
    fn run(&mut self, secs: f64) {
        let end = self.now + secs;
        while self.now < end {
            self.frame();
        }
    }
    fn until(&mut self, limit: f64, f: &dyn Fn(&Net) -> bool) -> bool {
        let end = self.now + limit;
        while self.now < end {
            if f(self) {
                return true;
            }
            self.frame();
        }
        f(self)
    }
}

#[test]
fn relevancy_closes_and_reopens_the_proxy_channel() {
    let Some(d) = data() else { return eprintln!("SKIP: golden data missing") };
    let mut n = net(&d, false);
    assert!(n.until(5.0, &|n| n.a.proxies.contains_key("B") && n.b.proxies.contains_key("A")), "proxies never appeared");
    // B is 1000 cm from A: within the default 15000 cm, relevant. Cull at 500 cm with a 0.5 s timeout.
    n.s.relevancy.net_cull_distance_squared = 500.0 * 500.0;
    n.s.relevancy.relevant_timeout = 0.5;
    n.run(0.3);
    assert!(n.s.channel_open("B", 1), "closed before RelevantTimeout");
    assert!(n.until(2.0, &|n| !n.a.proxies.contains_key("B")), "B's proxy on A was not destroyed");
    assert!(!n.s.channel_open("B", 1) && !n.s.channel_open("A", 2));
    assert!(n.s.channel_open("A", 1), "the owner's own channel closed");
    assert!(n.s.channels_closed >= 2);
    // relevant again: the channel reopens with the spawn state
    n.s.relevancy.net_cull_distance_squared = mh_net::relevancy::ACTOR_NET_CULL_DISTANCE_SQUARED;
    assert!(n.until(2.0, &|n| n.a.proxies.contains_key("B") && n.b.proxies.contains_key("A")), "proxy not back");
    let truth = n.s.pawn_movement("B").unwrap().location;
    let seen = n.a.pawn_location("B").unwrap();
    assert!((truth.x - seen.x).abs() < 5.0 && (truth.y - seen.y).abs() < 5.0, "reopened proxy at {seen:?}, server {truth:?}");
}

#[test]
fn horse_over_the_nodes() {
    let Some(d) = data() else { return eprintln!("SKIP: golden data missing") };
    if horse_template(&d.rec).is_none() {
        return eprintln!("SKIP: extract/json BP_Horse missing");
    }
    let mut n = net(&d, true);
    assert!(n.until(5.0, &|n| n.a.own_move.is_some() && n.b.own_move.is_some()), "pawns never spawned");
    n.s.spawn_horse("H", FVector { x: 0.0, y: 600.0, z: 120.0 + AVG_FLOOR_DIST }, 0.0, None);
    assert!(n.until(2.0, &|n| n.a.proxies.contains_key("H") && n.b.proxies.contains_key("H")), "horse proxies missing");
    n.s.set_horse_driver("H", Some(1));
    assert!(n.until(2.0, &|n| n.a.vehicles.contains_key("H")), "client A does not drive the horse");
    assert!(n.b.proxies.contains_key("H") && !n.a.proxies.contains_key("H"));
    // the rider: attached to the horse's mesh (StartDriving AttachToComponent) on every machine via AttachmentReplication
    assert!(n.until(1.0, &|n| n.a.attachments.contains_key("A") && n.b.attachments.contains_key("A")), "rider attachment not replicated");
    n.a.horse_input = mh_character::exe_horse::HorseInput { fwd: 1.0, ..Default::default() };
    n.run(3.0);
    n.a.horse_input = Default::default();
    n.run(2.0);
    let srv = n.s.pawn_movement("H").unwrap().location;
    let own = n.a.pawn_location("H").unwrap();
    let prox = n.b.pawn_location("H").unwrap();
    let dist = |p: FVector, q: FVector| ((p.x - q.x).powi(2) + (p.y - q.y).powi(2) + (p.z - q.z).powi(2)).sqrt();
    let corr = n.s.pawn_prediction("H").unwrap().corrections_sent;
    eprintln!("horse: server {srv:?} driver {own:?} proxy {prox:?} corrections {corr}");
    assert!(srv.x > 500.0, "the horse did not move: {srv:?}");
    assert!(dist(srv, own) < 2.0, "driver / server gap {}", dist(srv, own));
    assert!(dist(srv, prox) < 5.0, "proxy / server gap {}", dist(srv, prox));
    let seat = n.s.pawn_movement("H").unwrap().horse_rider_seat().0;
    let rider_srv = n.s.pawn_movement("A").unwrap().location;
    let rider_b = n.b.pawn_location("A").unwrap();
    let rider_a = n.a.pawn_location("A").unwrap();
    eprintln!("rider: server {rider_srv:?} seat {seat:?} on B {rider_b:?} on A {rider_a:?}");
    assert!(dist(rider_srv, seat) < 0.01, "server rider off the seat");
    assert!(dist(rider_b, seat) < 5.0 && dist(rider_a, seat) < 5.0, "rider not on its horse on the clients");
    // the rider gets off: back to a proxy on A, the server simulates it
    n.s.set_horse_driver("H", None);
    assert!(n.until(2.0, &|n| n.a.proxies.contains_key("H") && !n.a.vehicles.contains_key("H")), "A still drives the horse");
    assert!(n.until(1.0, &|n| !n.a.attachments.contains_key("A") && !n.b.attachments.contains_key("A")), "rider still attached");
    // dismounted: A walks again, its moves agree with the server after the exit correction
    n.a.move_input = ExeInput { fwd: 1.0, ..Default::default() };
    n.run(1.0);
    n.a.move_input = ExeInput::default();
    n.run(1.0);
    let (srv_a, own_a) = (n.s.pawn_movement("A").unwrap().location, n.a.pawn_location("A").unwrap());
    assert!(dist(srv_a, own_a) < 2.0, "after dismounting: server {srv_a:?} owner {own_a:?}");
    n.s.destroy_horse("H");
    assert!(n.until(2.0, &|n| !n.a.proxies.contains_key("H") && !n.b.proxies.contains_key("H")), "horse not destroyed");
}

/// Climbing with and without move combining. Without it, server and owner move the capsule on the same timeline (RPCs
/// run in TickDispatch at the previous frame's clock, NetServer::dispatch; the owner saves its move after the climb's
/// actor-tick step, client_frame_with): no position error at all. With it (the exe's default: a pawn standing still
/// sends at ClientNetSendMoveDeltaTimeStationary 0.0333, so every second frame is combined), FSavedMove_Character::
/// CombineWith rva=0x2f74d30 puts the capsule back at the pending move's start before replaying both deltas, and a
/// MOVE_Custom replay does not move it: the move carries the position one frame old, so the server finds errors of one
/// frame of climb and corrects at most every NetworkMinTimeBetweenClientAdjustments - exe behaviour (no Mordhau class
/// skips ServerCheckClientError: vtable +0x9a8 = 0x2f86aa0 for all; no Blueprint sets
/// bIgnoreClientMovementErrorChecksAndCorrection).
#[test]
fn climbing_over_the_nodes() {
    climbing(false);
    climbing(true);
}

fn climbing(combine: bool) {
    let Some(d) = data() else { return eprintln!("SKIP: golden data missing") };
    let mut n = net(&d, false);
    assert!(n.until(5.0, &|n| n.a.own_move.is_some() && n.b.proxies.contains_key("A")), "pawns never spawned");
    // past the spawn's jump window (CanJump is false right after a spawn: the first landing's JumpCooldown)
    n.run(3.0);
    let m = &n.a.own_move.as_ref().unwrap().0;
    assert!(m.can_jump(), "A cannot jump yet (t {} mode {:?})", m.world_time, m.mode);
    // A presses jump facing the 120 cm block: AttemptClimb finds the ledge
    n.a.own_move.as_mut().unwrap().1.send.enable_move_combining = combine;
    // (held for a few frames: the client's frame clock may skip one of the shared clock's)
    n.a.move_input = ExeInput { jump: true, ..Default::default() };
    for _ in 0..3 {
        n.frame();
    }
    n.a.move_input = ExeInput::default();
    assert_eq!(n.a.climbs_requested, 1, "no climb requested");
    let climbing = |w: &World| w.net_climb_motion("A").is_some();
    assert!(n.until(1.0, &|n| climbing(&n.s.srv.world) && climbing(&n.b.c.world)), "server / client B never saw A climbing");
    let target = n.s.pawn_movement("A").unwrap().climb_target_location;
    assert!(target.z > 200.0 && target.x > 70.0, "server ClimbTargetLocation {target:?}");
    assert_eq!((target.x.fract(), target.y.fract(), target.z.fract()), (0.0, 0.0, 0.0), "not a Vector_NetQuantize (whole cm)");
    assert!(n.until(3.0, &|n| !climbing(&n.s.srv.world) && !climbing(&n.a.c.world)), "the climb never ended");
    n.run(1.0);
    let srv = n.s.pawn_movement("A").unwrap();
    let own = n.a.own_move.as_ref().unwrap().0.location;
    let prox = n.b.pawn_location("A").unwrap();
    let sp = n.s.pawn_prediction("A").unwrap();
    eprintln!("climb (combining {combine}): server {:?} mode {:?} owner {own:?} proxy {prox:?} corrections {} (throttled {}), errors {}", srv.location, srv.mode, sp.corrections_sent, sp.corrections_throttled, sp.errors.len());
    if combine {
        // every error is the one-frame lag of a combined move: at most one frame of the climb (475 cm/s vertical)
        for e in &sp.errors {
            let d = ((e.1[0] - e.2[0]).powi(2) + (e.1[1] - e.2[1]).powi(2) + (e.1[2] - e.2[2]).powi(2)).sqrt();
            assert!(d < 8.5 && e.3 == e.4, "error beyond one frame of climb: {e:?}");
        }
        assert!(sp.corrections_sent <= 12, "corrections {}", sp.corrections_sent);
    } else {
        assert!(sp.errors.is_empty(), "uncombined climb had errors: {:?}", sp.errors);
    }
    assert!((srv.location.z - (120.0 + 96.0)).abs() < 5.0, "the server's A is not on the block: {:?}", srv.location);
    assert_eq!(srv.mode, mh_character::Mode::Walking);
    assert!((own.z - srv.location.z).abs() < 2.0 && (own.x - srv.location.x).abs() < 2.0, "owner {own:?} vs server {:?}", srv.location);
    assert!((prox.z - srv.location.z).abs() < 5.0 && (prox.x - srv.location.x).abs() < 5.0, "proxy {prox:?} vs server {:?}", srv.location);
}

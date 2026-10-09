//! Character movement network prediction (src/movement.rs) on mh-character's exe-exact ExeMovement: the owning
//! client predicts with saved moves, the server replays its ServerMoves, acknowledges or corrects, the client replays
//! after a correction; time-discrepancy detection / resolution; the exe constants. Records:
//! core/tests/golden/character/records.json (ignored path) -> SKIP when absent.

use std::path::Path;

use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::uemath::{size_sq, sub, v};
use mh_character::{BoxWorld, CharacterRecords, CharacterSource, ExeInput, ExeMovement, RecordsJson};
use mh_net::movement::*;
use mh_net::transport::{JitterLoopback, Transport};
use mh_net::Msg;
use mordhau_core::ue::FVector;

const DT: f32 = 1.0 / 60.0;

fn records() -> Option<CharacterRecords> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../core/tests/golden/character/records.json");
    match std::fs::read_to_string(&p) {
        Ok(t) => Some(RecordsJson(&t).load().expect("records.json")),
        Err(_) => {
            eprintln!("SKIP: {} missing", p.display());
            None
        }
    }
}

fn floor() -> BoxWorld {
    let mut w = BoxWorld::new();
    w.add_box(v(-50000.0, -50000.0, -200.0), v(50000.0, 50000.0, 0.0));
    w
}

fn pawn(r: &CharacterRecords) -> ExeMovement {
    ExeMovement::new(r, v(0.0, 0.0, 96.0 + AVG_FLOOR_DIST))
}

/// the scripted player: walk, strafe while turning, jump, sprint, stop
fn input(t: f32) -> ExeInput {
    let mut i = ExeInput::default();
    if t < 2.0 {
        i.fwd = 1.0;
    } else if t < 3.5 {
        i.right = 1.0;
        i.yaw = (t - 2.0) * 40.0;
    } else if t < 5.5 {
        i.fwd = 1.0;
        i.yaw = 60.0;
        i.sprint = t > 4.0;
        i.jump = (3.6..3.62).contains(&t);
    } else if t < 6.0 {
        i.fwd = -1.0;
        i.yaw = 60.0;
    } else {
        i.yaw = 60.0;
    }
    i
}

struct Net {
    world: BoxWorld,
    client: ExeMovement,
    cp: ClientPrediction,
    server: ExeMovement,
    sp: ServerPrediction,
    cfg: NetMoveCfg,
    t: JitterLoopback,
    frame: u64,
}

impl Net {
    fn new(r: &CharacterRecords, latency: u64, jitter: u64) -> Self {
        let mut server = pawn(r);
        server.player_controlled = true;
        Net { world: floor(), client: pawn(r), cp: ClientPrediction::default(), server, sp: ServerPrediction::default(), cfg: NetMoveCfg::default(), t: JitterLoopback::new(latency, jitter, 7), frame: 0 }
    }

    /// one lockstep frame: the client's move out; the server's TickDispatch (received moves), world tick, the remote
    /// pawn's ticks, TickFlush (the answer); the client applies what arrived
    fn step(&mut self, inp: &ExeInput) {
        self.frame += 1;
        if let Some(d) = client_frame(&mut self.client, &mut self.cp, &self.world, DT, inp) {
            self.t.send(0, &Msg::ServerMove { who: "A".into(), gen: 1, data: d }, 1);
        }
        for (_, m) in self.t.receive(0) {
            if let Msg::ServerMove { data, .. } = m {
                let wt = self.server.world_time;
                server_move_packet(&mut self.server, &mut self.sp, &self.cfg, &self.world, &data, wt, wt);
            }
        }
        self.server.world_time += DT;
        server_remote_pawn_tick(&mut self.server, DT);
        if let Some(r) = send_client_adjustment(&mut self.sp, self.server.world_time, &self.cfg) {
            self.t.send(1, &Msg::MoveResponse { who: "A".into(), r }, 0);
        }
        for (_, m) in self.t.receive(1) {
            if let Msg::MoveResponse { r, .. } = m {
                client_handle_response(&mut self.client, &mut self.cp, &self.world, &r);
            }
        }
        self.t.advance();
    }

    fn run(&mut self, until: f32) {
        while (self.frame as f32) * DT < until {
            let t = self.frame as f32 * DT;
            self.step(&input(t));
        }
    }

    fn gap(&self) -> f32 {
        size_sq(sub(self.client.location, self.server.location)).sqrt()
    }
}

/// client_frame without a network is ExeMovement::frame up to RoundAcceleration (the move's acceleration is rounded
/// to 0.1 so client and server simulate the same): same path over the scripted input, within a millimetre
#[test]
fn client_frame_matches_exe_frame() {
    let Some(r) = records() else { return };
    let w = floor();
    let (mut a, mut b) = (pawn(&r), pawn(&r));
    let mut cp = ClientPrediction::default();
    for k in 0..300 {
        let i = input(k as f32 * DT);
        a.frame(&w, DT, &i);
        client_frame(&mut b, &mut cp, &w, DT, &i);
    }
    let d = size_sq(sub(a.location, b.location)).sqrt();
    assert!(d < 0.1, "client_frame drifted {d} cm from ExeMovement::frame ({:?} vs {:?})", a.location, b.location);
    assert_eq!(a.mode, b.mode);
}

#[test]
fn round_acceleration_is_the_exe_floor() {
    let r = round_acceleration(v(1.25, -1.25, 0.04));
    assert_eq!((r.x, r.y, r.z), (13.0f32 * 0.1, -12.0f32 * 0.1, 0.0));
    let r = round_acceleration(v(1530.27, -0.06, 99.95));
    assert_eq!((r.x, r.y, r.z), (15303.0f32 * 0.1, -1.0f32 * 0.1, 1000.0f32 * 0.1));
}

#[test]
fn compressed_flags_round_trip_mordhau_layout() {
    // FSavedMove_Custom::GetCompressedFlags 0x14bc4c0 / UMordhauMovementComponent::UpdateFromCompressedFlags 0x14dd5f0
    assert_eq!(compressed_flags(true, false, 0), 0x01);
    assert_eq!(compressed_flags(false, true, 0), 0x02);
    assert_eq!(compressed_flags(false, false, 7), 0x70);
    assert_eq!(compressed_flags(false, false, 4), 0x40);
    assert_eq!(compressed_flags(true, true, 5), 0x53);
    let Some(r) = records() else { return };
    let mut m = pawn(&r);
    update_from_compressed_flags(&mut m, 0x53);
    assert!(m.pressed_jump && m.wants_to_crouch && m.sprint_state as u8 == 5);
    assert_eq!(decompress_axis(compress_axis(90.0)), 90.0);
    assert_eq!(compress_axis(-90.0), 0xc000);
}

#[test]
fn server_move_delta_time_is_capped() {
    // GetServerMoveDeltaTime 0x2f7a800: min(TimeStamp - CurrentClientTimeStamp, MaxMoveDeltaTime 0.125)
    let cfg = NetMoveCfg::default();
    let mut sp = ServerPrediction::default();
    sp.current_client_time_stamp = 1.0;
    assert_eq!(sp.get_server_move_delta_time(&cfg, 1.01, 1.0), 1.01 - 1.0);
    assert_eq!(sp.get_server_move_delta_time(&cfg, 2.0, 1.0), 0.125);
    sp.resolving = true;
    sp.resolution_move_delta_override = 0.004;
    assert_eq!(sp.get_server_move_delta_time(&cfg, 2.0, 1.0), 0.004);
}

#[test]
fn position_error_threshold() {
    let Some(r) = records() else { return };
    let cfg = NetMoveCfg::default();
    let m = pawn(&r);
    let mode = pack_mode(m.mode);
    // LocDiff.SizeSquared() > 3.0 (MAXPOSITIONERRORSQUARED, strictly greater: `seta`)
    let l = m.location;
    assert!(!server_exceeds_allowable_position_error(&cfg, &m, FVector { x: l.x + 1.0, y: l.y + 1.0, z: l.z + 1.0 }, mode));
    assert!(server_exceeds_allowable_position_error(&cfg, &m, FVector { x: l.x + 1.8, y: l.y, z: l.z }, mode));
    // a different movement mode is always an error
    assert!(server_exceeds_allowable_position_error(&cfg, &m, l, mode + 2));
}

/// honest client: the discrepancy stays inside the margins; a client whose clock runs 30% fast is detected after
/// MaxTimeMargin and its moves are then bounded by server time and pay the stolen time back
#[test]
fn time_discrepancy_detection_and_resolution() {
    let cfg = NetMoveCfg::default();
    // (ServerPrediction, simulated time, world time, frames resolving with a server-bounded delta)
    let run = |speed: f32| -> (ServerPrediction, f32, f32, u32) {
        let mut sp = ServerPrediction::default();
        sp.last_possession_time = -10.0;
        let (mut client_ts, mut w, mut sim, mut bounded) = (0.0f32, 0.0f32, 0.0f32, 0u32);
        for _ in 0..600 {
            w += DT;
            client_ts += DT * speed;
            sp.process_client_time_stamp_for_time_discrepancy(&cfg, client_ts, w, w);
            let dt = sp.get_server_move_delta_time(&cfg, client_ts, 1.0);
            if sp.resolving && dt <= DT + 1e-6 {
                bounded += 1;
            }
            if dt > 0.0 {
                sim += dt;
                sp.current_client_time_stamp = client_ts;
                sp.server_time_stamp = w;
                sp.server_time_stamp_last_server_move = w;
            }
        }
        (sp, sim, w, bounded)
    };
    let (honest, sim_h, w_h, _) = run(1.0);
    assert!(honest.discrepancies_detected.is_empty() && !honest.resolving, "honest client flagged: {:?}", honest.discrepancies_detected);
    assert!((sim_h - w_h).abs() < 0.05, "honest client simulated {sim_h} s in {w_h} s");
    let (fast, sim_f, w_f, bounded) = run(1.3);
    assert!(!fast.discrepancies_detected.is_empty(), "speed hack not detected");
    assert!(fast.discrepancies_detected[0].0 > cfg.discrepancy_max_margin);
    assert!(bounded > 100, "moves bounded by server time on {bounded} frames");
    // the client claimed 1.3 x the time; the server let it move little more than the real time plus the margin
    // DriftAllowance 0.05 forgives 5 % of the server time; the rest beyond MaxTimeMargin is paid back
    assert!(sim_f < w_f * (1.0 + cfg.discrepancy_drift_allowance) + cfg.discrepancy_max_margin + 0.1 && sim_f < 1.3 * w_f - 1.0, "simulated {sim_f} s in {w_f} s");
    // no tracking during the 5 s after possession (UAdvancedCharacterMovement override 0x149b1f0)
    let mut sp = ServerPrediction::default();
    sp.server_time_stamp_last_server_move = 1.0;
    sp.process_client_time_stamp_for_time_discrepancy(&cfg, 100.0, 1.0, 1.0);
    assert_eq!(sp.lifetime_raw_time_discrepancy, 0.0);
}

/// zero latency, no jitter: every move acknowledged, never corrected, client and server in the same place
#[test]
fn client_and_server_agree_without_latency() {
    let Some(r) = records() else { return };
    let mut n = Net::new(&r, 0, 0);
    n.run(3.9);
    // walking, strafing, turning and jumping replay exactly (sprinting may not: see movement.rs SprintTime note)
    assert_eq!(n.sp.corrections_sent, 0, "corrections without latency before the sprint");
    assert!(n.gap() < 0.01, "gap before the sprint {}", n.gap());
    n.run(7.0);
    assert!(n.sp.moves > 200, "moves {}", n.sp.moves);
    // the engine's own tolerance: a client within sqrt(MAXPOSITIONERRORSQUARED) of the server is not corrected
    assert!(n.gap() <= n.cfg.max_position_error_squared.sqrt(), "client / server gap {}", n.gap());
    assert!(size_sq(n.client.location) > 300.0 * 300.0, "the pawn did not walk: {:?}", n.client.location);
}

/// 100 ms one way plus up to 50 ms jitter (messages can overtake each other): the client predicts ahead, the server
/// follows with the moves' timestamps; stale moves are dropped; any divergence is corrected; at rest they agree
#[test]
fn client_and_server_agree_under_latency_and_jitter() {
    let Some(r) = records() else { return };
    let mut n = Net::new(&r, 6, 3);
    n.run(7.0);
    for _ in 0..30 {
        n.step(&input(7.0));
    }
    assert!(n.gap() <= n.cfg.max_position_error_squared.sqrt(), "client / server gap {} (client {:?}, server {:?})", n.gap(), n.client.location, n.server.location);
    // acknowledgements at most every NetworkMinTimeBetweenClientAckGoodMoves 0.1 s (SendClientAdjustment 0x2f85d00)
    assert!(n.cp.acks > 40, "acks {}", n.cp.acks);
    assert!(n.cp.saved_moves.len() <= 2 * (6 + 3) + 6 + 2, "unacknowledged moves {} (at most a round trip plus the ack interval)", n.cp.saved_moves.len());
    eprintln!("latency+jitter: moves {} stale {} corrections {} replays {}", n.sp.moves, n.sp.stale_moves, n.sp.corrections_sent, n.cp.replays);
}

/// the server's pawn is pushed 100 cm (a divergence the client cannot predict): the next ServerMove's location is
/// off by more than sqrt(MAXPOSITIONERRORSQUARED), the server sends ClientAdjustPosition, the client takes the
/// server's position, replays its unacknowledged moves, and both agree again
#[test]
fn divergence_is_corrected() {
    let Some(r) = records() else { return };
    let mut n = Net::new(&r, 4, 0);
    n.run(1.0);
    assert_eq!(n.sp.corrections_sent, 0);
    n.server.location.y += 100.0;
    n.run(1.5);
    assert!(n.sp.corrections_sent >= 1, "no ClientAdjustPosition sent");
    assert!(n.cp.corrections >= 1 && n.cp.replays >= 1, "client corrections {} replays {}", n.cp.corrections, n.cp.replays);
    for _ in 0..60 {
        n.step(&ExeInput::default()); // come to rest so both sides are compared at the same state
    }
    assert!(n.gap() <= n.cfg.max_position_error_squared.sqrt(), "after the correction the gap is {}", n.gap());
    assert!((n.client.location.y - 100.0).abs() < 5.0, "client did not take the server's position: {:?}", n.client.location);
}

#[test]
#[ignore]
fn debug_first_correction() {
    let Some(r) = records() else { return };
    let mut n = Net::new(&r, 0, 0);
    let mut last = (n.client.location, n.server.location);
    while (n.frame as f32) * DT < 7.0 {
        let t = n.frame as f32 * DT;
        let c0 = n.sp.corrections_sent;
        let (cs, ct, sst, sstt) = (n.client.sprint_state as u8, n.client.sprint_time, n.server.sprint_state as u8, n.server.sprint_time);
        n.step(&input(t));
        if n.sp.corrections_sent > c0 {
            eprintln!("first correction at frame {} t={t}: client {:?} server {:?} prev {:?}; client vel {:?} server vel {:?}; sprint c {cs}/{ct} s {sst}/{sstt}; modes {:?} {:?}; maxspeed c {} s {}",
                n.frame, n.client.location, n.server.location, last, n.client.velocity, n.server.velocity, n.client.mode, n.server.mode, n.client.get_max_speed(), n.server.get_max_speed());
            return;
        }
        last = (n.client.location, n.server.location);
    }
}

/// GetClientNetSendDeltaTime 0x2f792a0: 0.0166 s at full net speed with few players; throttled 0.0333 over 48
/// players; standing still with an acknowledged move at the same rotation: Stationary 0.0333; clamped to [1/120, 1/5]
#[test]
fn client_send_delta_time_rules() {
    let Some(r) = records() else { return };
    let m = pawn(&r);
    let mut cp = ClientPrediction::default();
    assert_eq!(cp.client_net_send_delta_time(&m), 0.0166);
    cp.send.player_count = 49;
    assert_eq!(cp.client_net_send_delta_time(&m), 0.0333);
    cp.send.player_count = 2;
    cp.send.net_speed = 2000;
    assert_eq!(cp.client_net_send_delta_time(&m), 2.0 * 42.0 / 2000.0);
    cp.send.net_speed = 100000;
    let mut sm = ClientPrediction::default();
    let mut mm = pawn(&r);
    let w = floor();
    // one acknowledged move at rest -> stationary rate
    let p = replicate_move_to_server(&mut mm, &mut sm, &w, DT, FVector::ZERO, &NetMoveCfg::default());
    let ts = p.map(|p| p.new.time_stamp).unwrap_or(sm.saved_moves[0].time_stamp);
    client_handle_response(&mut mm, &mut sm, &w, &MoveResponse::AckGoodMove { time_stamp: ts });
    assert_eq!(sm.client_net_send_delta_time(&mm), 0.0333);
}

/// moves are combined (CanCombineWith) and held back between sends: a straight run sends far fewer packets than
/// frames, every frame's time still reaches the server (its timestamps cover the run), and the result is the same
#[test]
fn move_combining_and_throttling() {
    let Some(r) = records() else { return };
    let mut n = Net::new(&r, 0, 0);
    n.run(1.9);
    for _ in 0..30 {
        n.step(&ExeInput::default());
    }
    assert!(n.cp.combined > 10, "combined {}", n.cp.combined);
    assert!(n.cp.packets < n.frame as u32, "packets {} frames {}", n.cp.packets, n.frame);
    assert!((n.sp.current_client_time_stamp - n.cp.current_time_stamp).abs() < 0.05, "server time stamp {} client {}", n.sp.current_client_time_stamp, n.cp.current_time_stamp);
    assert!(n.gap() <= n.cfg.max_position_error_squared.sqrt(), "gap {}", n.gap());
}

/// 5 % of the unreliable ServerMoves / answers lost: ServerMoveOld re-sends the oldest important unacknowledged move,
/// so turns and stops survive a lost datagram; the pawns still agree at rest
#[test]
fn lost_moves_are_covered_by_old_moves() {
    let Some(r) = records() else { return };
    let mut n = Net::new(&r, 3, 1);
    n.t.loss = 0.05;
    n.run(7.0);
    for _ in 0..60 {
        n.step(&input(7.0));
    }
    assert!(n.t.lost > 5, "nothing lost ({})", n.t.lost);
    assert!(n.cp.old_moves_sent > 0, "no ServerMoveOld sent");
    assert!(n.gap() <= n.cfg.max_position_error_squared.sqrt(), "gap {} after losses", n.gap());
    eprintln!("loss: lost {} old sent {} server old {} corrections {}", n.t.lost, n.cp.old_moves_sent, n.sp.old_moves, n.sp.corrections_sent);
}


// ---- simulated proxies -----------------------------------------------------------------------------------------------

/// a simulated proxy: replicated at the server's rate, it extrapolates by its velocity along its floor and hides the
/// jumps with the mesh offset (SmoothCorrection), which decays to zero (SmoothClientPosition)
#[test]
fn proxy_smoothing() {
    let Some(r) = records() else { return };
    let w = floor();
    let c = SmoothCfg::default();
    let z = 96.0 + AVG_FLOOR_DIST;
    let r0 = RepMovement { location: [0.0, 0.0, z], velocity: [400.0, 0.0, 0.0], yaw_byte: 0, mode: 1 };
    let mut p = ProxyMove::new(&r0, pawn(&r));
    for _ in 0..6 {
        p.tick(&w, DT, &c);
    }
    assert!((p.location().x - 40.0).abs() < 0.01, "extrapolated {:?}", p.location());
    assert!((p.location().z - z).abs() < 0.01, "left its floor {:?}", p.location());
    // the next update says 30: the capsule jumps back 10, the mesh does not
    let before = p.visual();
    p.on_rep(&RepMovement { location: [30.0, 0.0, z], ..r0.clone() }, &c);
    assert!((p.visual().x - before.x).abs() < 1e-3, "visual jumped");
    for _ in 0..30 {
        p.tick(&w, DT, &c);
    }
    assert!(size_sq(p.offset) < 0.1 * 0.1, "offset not decayed {:?}", p.offset);
    // a far update (> NetworkNoSmoothUpdateDistance 384) teleports
    p.on_rep(&RepMovement { location: [5000.0, 0.0, z], ..r0.clone() }, &c);
    assert_eq!(p.visual().x, 5000.0);
}

/// floor, 8 steps of 15 cm up to a 120 cm plateau (to x = 840), then a ledge back down to the floor
fn stairs() -> BoxWorld {
    let mut w = floor();
    for i in 0..8 {
        let x0 = 200.0 + 40.0 * i as f32;
        w.add_box(v(x0, -500.0, 0.0), v(840.0, 500.0, 15.0 * (i + 1) as f32));
    }
    w
}

/// SimulateMovement on the proxy's own collision: the proxy of a pawn that walks up stairs and off a ledge, updated
/// only every 6th server frame (10 Hz: a lossy or saturated connection), stays on the stairs between updates and falls
/// off the ledge with gravity; the flat-ground extrapolation it replaces (r4: velocity on the last floor height) is
/// worse on the same path
#[test]
fn proxy_follows_stairs_and_ledges() {
    let Some(r) = records() else { return };
    let w = stairs();
    let c = SmoothCfg::default();
    let mut server = pawn(&r);
    server.world_time = 10.0;
    let mut p = ProxyMove::new(&rep_movement_of(&server), pawn(&r));
    let (mut flat_loc, mut flat_vel) = (server.location, server.velocity);
    let (mut err, mut flat_err) = (0.0f32, 0.0f32);
    let (mut sum, mut flat_sum, mut n) = (0.0f32, 0.0f32, 0);
    let mut fell = false;
    for k in 0..300 {
        let inp = ExeInput { fwd: 1.0, ..Default::default() };
        server.frame(&w, DT, &inp);
        fell |= server.mode == mh_character::Mode::Falling && server.location.x > 840.0;
        if k % 6 == 0 {
            let rep = rep_movement_of(&server);
            p.on_rep(&rep, &c);
            flat_loc = v(rep.location[0], rep.location[1], rep.location[2]);
            flat_vel = v(rep.velocity[0], rep.velocity[1], 0.0);
        }
        // both models extrapolate on the update frame too (SimulateMovement runs on it:
        // bNetworkSkipProxyPredictionOnNetUpdate false), so both lead the server by a frame here (no latency)
        flat_loc = v(flat_loc.x + flat_vel.x * DT, flat_loc.y + flat_vel.y * DT, flat_loc.z);
        p.tick(&w, DT, &c);
        let e = size_sq(sub(p.location(), server.location)).sqrt();
        let fe = size_sq(sub(flat_loc, server.location)).sqrt();
        err = err.max(e);
        flat_err = flat_err.max(fe);
        sum += e;
        flat_sum += fe;
        n += 1;
    }
    eprintln!("proxy vs server capsule: max {err:.2} cm mean {:.2} cm; flat-ground model max {flat_err:.2} mean {:.2}; server at {:?}", sum / n as f32, flat_sum / n as f32, server.location);
    assert!(server.location.x > 900.0 && fell, "the pawn did not cross the stairs and the ledge: {:?}", server.location);
    // measured: max 9.4 cm, mean 5.3 (the one-frame lead at ~300 cm/s); flat: max 33.9 (the stair rises and the ledge)
    assert!(err < 15.0 && sum / (n as f32) < 7.0, "proxy error max {err} mean {}", sum / n as f32);
    assert!(2.0 * err < flat_err && sum < flat_sum, "SimulateMovement is not better than flat extrapolation (max {err} vs {flat_err})");
}

/// ReplicatedMovement is quantized as a pawn's FRepMovement: Location to 1/100 cm (APawn ctor RoundTwoDecimals),
/// LinearVelocity to whole cm/s, yaw to a byte
#[test]
fn rep_movement_quantization() {
    let Some(r) = records() else { return };
    let mut m = pawn(&r);
    m.location = v(1234.5678, -98.7654, 98.15);
    m.velocity = v(123.4, -0.6, 0.0);
    m.set_yaw(91.0);
    let rep = rep_movement_of(&m);
    for (a, b) in rep.location.iter().zip([1234.57f32, -98.77, 98.15]) {
        assert!((a - b).abs() < 2e-4, "location {:?}", rep.location);
    }
    assert_eq!(rep.velocity, [123.0, -1.0, 0.0]);
    assert_eq!(rep.yaw_byte, 65); // RoundToInt(91 * 0.7111111) = 65
    assert!((rep.yaw() - 91.40625).abs() < 1e-4);
    // wire: 2 flag bits + location (Bits 16 in 5 bits, 3 x 18) + velocity (Bits 6 in 5 bits, 3 x 8) + yaw (3 flags + 8)
    assert_eq!(rep.wire_bits(), 2 + (5 + 3 * 18) + (5 + 3 * 8) + 11);
}

// ---- horses ---------------------------------------------------------------------------------------------------------

struct HorseData {
    rec: CharacterRecords,
    bp: String,
    cfg: mh_character::exe_horse::HorseCfg,
}

fn horse_data() -> Option<HorseData> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let read = |rel: &str| std::fs::read_to_string(root.join("extract/json").join(rel)).ok();
    let bp = read("Mordhau/Content/Mordhau/Blueprints/Interactables/Animals/BP_Horse.json")?;
    let veh = read("Mordhau/Content/Mordhau/Blueprints/VehicleComponents/BP_VehicleHorse.json");
    let cfg = mh_character::exe_horse::HorseCfg::from_json(&bp, veh.as_deref(), &|p: &str| read(&format!("{p}.json"))).ok()?;
    let base = records()?;
    let rec = mh_character::exe_horse::horse_character_records(&base, &bp).ok()?;
    Some(HorseData { rec, bp, cfg })
}

fn horse(d: &HorseData, server: bool) -> ExeMovement {
    let mut m = ExeMovement::new_horse(&d.rec, &d.bp, d.cfg.clone(), v(0.0, 0.0, 120.0 + AVG_FLOOR_DIST)).unwrap();
    m.world_time = 10.0;
    m.authority = true;
    m.player_controlled = true;
    if let Some(h) = m.horse.as_mut() {
        // the owning client drives it locally; on the server the controller is remote (no local steering)
        h.controlled = !server;
    }
    m
}

/// the rider: gallop forward through the gears, turn, slow down, stop
fn horse_input(t: f32) -> mh_character::exe_horse::HorseInput {
    let mut i = mh_character::exe_horse::HorseInput::default();
    if t < 4.0 {
        i.fwd = 1.0;
    } else if t < 6.0 {
        i.fwd = 1.0;
        i.right = 1.0;
    } else if t < 7.0 {
        i.fwd = -1.0;
    }
    i
}

/// A driven horse over the net: FSavedMove_Horse moves (the gear in compressed-flag bits 4..5, set in SetMoveFor),
/// the server replays them (UHorseMovementComponent::UpdateFromCompressedFlags: gear + ValidateGear(5); the control
/// yaw from the move, the actor turning in PhysicsRotation) and both agree under latency and jitter; the gears went up
/// past the first
#[test]
fn horse_client_and_server_agree() {
    let Some(d) = horse_data() else { return eprintln!("SKIP: no extract/json BP_Horse") };
    for (lat, jit) in [(0u64, 0u64), (6, 3)] {
        let world = floor();
        let (mut client, mut server) = (horse(&d, false), horse(&d, true));
        let (mut cp, mut sp, cfg) = (ClientPrediction::default(), ServerPrediction::default(), NetMoveCfg::default());
        let mut t = JitterLoopback::new(lat, jit, 11);
        let mut max_gear = 0;
        for k in 0..(9.0 / DT) as u32 {
            let i = horse_input(k as f32 * DT);
            if let Some(pk) = client_frame_horse(&mut client, &mut cp, &world, DT, &i) {
                t.send(0, &Msg::ServerMove { who: "H".into(), gen: 1, data: pk }, 1);
            }
            for (_, m) in t.receive(0) {
                if let Msg::ServerMove { data, .. } = m {
                    let wt = server.world_time;
                    server_move_packet(&mut server, &mut sp, &cfg, &world, &data, wt, wt);
                }
            }
            server.world_time += DT;
            server_remote_pawn_tick(&mut server, DT);
            if let Some(r) = send_client_adjustment(&mut sp, server.world_time, &cfg) {
                t.send(1, &Msg::MoveResponse { who: "H".into(), r }, 0);
            }
            for (_, m) in t.receive(1) {
                if let Msg::MoveResponse { r, .. } = m {
                    client_handle_response(&mut client, &mut cp, &world, &r);
                }
            }
            t.advance();
            max_gear = max_gear.max(server.horse.as_ref().unwrap().gear);
        }
        let gap = size_sq(sub(client.location, server.location)).sqrt();
        eprintln!("horse lat {lat} jit {jit}: at {:?} gap {gap:.3} moves {} corrections {} max gear {max_gear}", server.location, sp.moves, sp.corrections_sent);
        assert!(size_sq(server.location) > 1000.0 * 1000.0, "the horse did not gallop: {:?}", server.location);
        assert!(max_gear >= 2, "gear never went up ({max_gear})");
        assert!(gap <= cfg.max_position_error_squared.sqrt(), "client / server gap {gap}");
        assert_eq!(client.horse.as_ref().unwrap().gear, server.horse.as_ref().unwrap().gear);
        if lat == 0 {
            assert!(sp.corrections_sent <= 2, "corrections without latency: {}", sp.corrections_sent);
        }
    }
}

/// the horse's saved-move flags: bits 0 / 1 the character's, bits 4..5 the gear (FSavedMove_Horse::GetCompressedFlags
/// 0x14bc510), and the server takes the gear back from them
#[test]
fn horse_gear_rides_in_the_move_flags() {
    let Some(d) = horse_data() else { return eprintln!("SKIP: no extract/json BP_Horse") };
    let mut m = horse(&d, true);
    for g in 0..4u8 {
        let flags = (((g & 1) << 4) | ((g >> 1 & 1) << 5)) | 1;
        m.velocity = v(d.cfg.gears[g as usize].max_speed, 0.0, 0.0); // fast enough that ValidateGear(5) keeps it
        update_from_compressed_flags(&mut m, flags);
        assert_eq!(m.horse.as_ref().unwrap().gear, g);
        assert!(m.pressed_jump);
        assert_eq!(m.c.max_walk_speed, d.cfg.gears[g as usize].max_speed);
    }
}

/// SendClientAdjustment's throttle (read off 0x2f85d00): an acknowledgement at most every
/// NetworkMinTimeBetweenClientAckGoodMoves (0.1 s), a correction at most every NetworkMinTimeBetweenClientAdjustments
/// (0.1 s) or, for a large one (mode mismatch / > NetworkLargeClientCorrectionDistance 15 cm),
/// NetworkMinTimeBetweenClientAdjustmentsLargeCorrection (0.05 s); a throttled answer is dropped
#[test]
fn client_adjustment_throttle() {
    let cfg = NetMoveCfg::default();
    let mut sp = ServerPrediction::default();
    let ack = |sp: &mut ServerPrediction, t: f32| {
        sp.pending = Some(MoveResponse::AckGoodMove { time_stamp: t });
        send_client_adjustment(sp, t, &cfg).is_some()
    };
    assert!(ack(&mut sp, 10.0));
    assert!(!ack(&mut sp, 10.05));
    assert!(sp.pending.is_none(), "a throttled answer is dropped");
    assert!(ack(&mut sp, 10.11));
    let adj = |sp: &mut ServerPrediction, t: f32, large: bool| {
        sp.large_correction = large;
        sp.pending = Some(MoveResponse::AdjustPosition { time_stamp: t, location: [0.0; 3], velocity: [0.0; 3], mode: 1 });
        send_client_adjustment(sp, t, &cfg).is_some()
    };
    assert!(adj(&mut sp, 20.0, false));
    assert!(!adj(&mut sp, 20.07, false));
    assert!(adj(&mut sp, 20.08, true), "a large correction waits only 0.05 s");
    assert!(!adj(&mut sp, 20.12, true));
    assert_eq!((sp.acks_throttled, sp.corrections_throttled, sp.corrections_sent), (1, 2, 2));
}

/// Mordhau's SmoothClientPosition (0x14a1f70): once every offset component is within 0.01 the smoothing completes and
/// the offset is exactly zero
#[test]
fn proxy_smoothing_completes_at_a_hundredth() {
    let Some(r) = records() else { return };
    let w = floor();
    let c = SmoothCfg::default();
    let z = 96.0 + AVG_FLOOR_DIST;
    let r0 = RepMovement { location: [0.0, 0.0, z], velocity: [0.0, 0.0, 0.0], yaw_byte: 0, mode: 1 };
    let mut p = ProxyMove::new(&r0, pawn(&r));
    p.on_rep(&RepMovement { location: [-10.0, 0.0, z], ..r0.clone() }, &c);
    assert!((p.offset.x - 10.0).abs() < 1e-4);
    let mut frames = 0;
    while p.offset != FVector::ZERO && frames < 100 {
        p.tick(&w, DT, &c);
        frames += 1;
    }
    assert_eq!(p.offset, FVector::ZERO, "offset never completed");
    // standing still: SmoothLocationTime x0.5 = 0.05 s; 10 cm x (1 - dt/0.05)^n <= 0.01 after a handful of frames
    assert!(frames < 30, "took {frames} frames");
}

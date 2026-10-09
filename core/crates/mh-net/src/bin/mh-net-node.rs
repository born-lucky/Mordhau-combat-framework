//! mh-net-node: one machine of a networked Duel over real UDP sockets, as its own process (tests/two_process.rs,
//! docs/RUST_NET.md). A host program over the library's free-running ServerNode / ClientNode (node.rs): it reads the
//! records, scripts the players like godot/tests/test_net_match.gd (in RoundPlay, as its own ReplicatedRoomGame
//! says, the round's scheduled winner attacks from its client whenever its pawn is idle; the server's hit detection
//! lands the winner's attack on the loser's head once per attack in Release), walks both pawns back and forth on a
//! flat test floor (BoxWorld; a host with a level passes its collision), and writes a JSON result with the outcome and
//! the server's / each client's view of every pawn over wall-clock time.
//!
//!   mh-net-node server --root R [--port 0] [--clients 2] [--rounds 3] [--winners ABAA] [--latency-ms L
//!                      --jitter-ms J --loss P] --out F
//!   mh-net-node client --root R --server 127.0.0.1:P --id N --name A [--winners ABAA] [--latency-ms ...] --out F
//!
//! Data: built with feature `host`, the records come from the spec matrix (data_gen/spec) + the user's paks through
//! mh-host (combat spec, character records, Duel ModeData, Kismet literals), each proved equal to its golden file
//! (mh-host tests/host.rs, mh-mode tests/spec_equiv.rs); `--data golden`, a build without `host`, or a failed spec /
//! pak load use the golden files (core/tests/golden, godot/data_gen/mode/mode_kismet.json).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mh_character::{BoxWorld, CharacterRecords, CharacterSource, ExeInput, RecordsJson as CharRecordsJson};
use mh_mode::data::ModeDataExt;
use mh_mode::{GameMode, Kismet, ModeData};
use mh_net::duel_match::{ModeEvent, ROOM_GAME};
use mh_net::movement::{NetMoveCfg, SmoothCfg};
use mh_net::node::{ClientNode, ClientPoll, MovementHost, ServerNode, ServerPoll, NET_SERVER_MAX_TICK_RATE};
use mh_net::transport::NetSim;
use mh_net::{NetWorld, Transport};
use mordhau_core::combat::{Input, World};
use mordhau_core::data::{RecordsJsonExe, Spec, SpecSource};
use mordhau_core::ue::FVector;
use serde_json::{json, Value};

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
const TIMEOUT: Duration = Duration::from_secs(60);

fn arg(args: &[String], k: &str, d: &str) -> String {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| d.to_string())
}

fn wall() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
}

/// the spec matrix + paks (feature `host`), loaded once per process; None = use the golden files
#[cfg(feature = "host")]
fn host() -> Option<&'static mh_host::Data> {
    thread_local! {
        static DATA: Option<&'static mh_host::Data> = {
            let golden = std::env::args().collect::<Vec<_>>().windows(2).any(|w| w[0] == "--data" && w[1] == "golden");
            if golden {
                None
            } else {
                match mh_host::Data::load() {
                    Ok(d) => Some(&*Box::leak(Box::new(d))),
                    Err(e) => {
                        eprintln!("mh-net-node: spec / paks unavailable ({e}); using the golden files");
                        None
                    }
                }
            }
        };
    }
    DATA.with(|d| *d)
}
#[cfg(not(feature = "host"))]
fn host() -> Option<&'static ()> {
    None
}

fn spec(root: &Path) -> Rc<Spec> {
    #[cfg(feature = "host")]
    if let Some(d) = host() {
        return Rc::new(d.combat_spec(&[LS]).expect("combat spec from the matrix"));
    }
    let t = std::fs::read_to_string(root.join("core/tests/golden/spec.json")).expect("core/tests/golden/spec.json");
    Rc::new(RecordsJsonExe(&t).load_spec().expect("spec"))
}

fn char_records(root: &Path) -> CharacterRecords {
    #[cfg(feature = "host")]
    if let Some(d) = host() {
        return d.character_records().expect("character records from the matrix");
    }
    let t = std::fs::read_to_string(root.join("core/tests/golden/character/records.json")).expect("records.json");
    CharRecordsJson(&t).load().expect("records")
}

/// the Duel ModeData + the Kismet literals: from the matrix (feature `host`) or the golden files
fn duel_data(root: &Path) -> (ModeData, String) {
    #[cfg(feature = "host")]
    if let Some(d) = host() {
        return (d.mode_data("DU").expect("DU ModeData from the matrix"), d.kismet_json().expect("Kismet from the matrix"));
    }
    let _ = host();
    let k = std::fs::read_to_string(root.join("godot/data_gen/mode/mode_kismet.json")).expect("mode_kismet.json");
    let g = std::fs::read_to_string(root.join("core/tests/golden/mode/duel_rooms.jsonl")).expect("duel_rooms.jsonl");
    let mut header: Value = serde_json::from_str(g.lines().nth(1).unwrap()).unwrap();
    mordhau_core::data::decode_exact(&mut header);
    (serde_json::from_value(header["data"].clone()).expect("ModeData"), k)
}

/// the test floor and the fixture starts: A at the origin facing +X, B 400 cm ahead facing -X (a host passes the
/// level's collision and its PlayerStarts)
fn movement_host(root: &Path) -> MovementHost {
    let mut level = BoxWorld::new();
    level.add_box(FVector { x: -50000.0, y: -50000.0, z: -200.0 }, FVector { x: 50000.0, y: 50000.0, z: 0.0 });
    MovementHost {
        level: Box::new(level),
        records: char_records(root),
        spawn: Box::new(|n: &str| {
            let z = 96.0 + mh_character::exe_cmc::AVG_FLOOR_DIST;
            if n == "A" { (FVector { x: 0.0, y: 0.0, z }, 0.0) } else { (FVector { x: 400.0, y: 0.0, z }, 180.0) }
        }),
        cfg: NetMoveCfg::default(),
        smooth: SmoothCfg::default(),
        horse: None,
    }
}

fn sim(args: &[String], seed: u64) -> NetSim {
    NetSim {
        latency_ms: arg(args, "--latency-ms", "0").parse().unwrap(),
        jitter_ms: arg(args, "--jitter-ms", "0").parse().unwrap(),
        loss: arg(args, "--loss", "0").parse().unwrap(),
        seed,
    }
}

fn duel(root: &Path, rounds: Option<i64>) -> GameMode {
    let (mut data, k) = duel_data(root);
    if let (Some(r), ModeDataExt::Duel(d)) = (rounds, &mut data.ext) {
        d.rounds_to_win = r; // smoke-test override of BP_DuelGameMode RoundsToWin (shipped value 5)
    }
    GameMode::new(data, Arc::new(Kismet::from_json(&k).unwrap()), mordhau_core::ue::CrtRand::new(1))
}

fn round_play(m: &GameMode) -> i64 {
    match &m.d.ext {
        ModeDataExt::Duel(d) => d.stage["RoundPlay"],
        _ => panic!("not a Duel mode"),
    }
}

fn server(args: &[String]) {
    let root = PathBuf::from(arg(args, "--root", "."));
    let n_clients: usize = arg(args, "--clients", "2").parse().unwrap();
    let rounds: i64 = arg(args, "--rounds", "3").parse().unwrap();
    let winners: Vec<String> = arg(args, "--winners", "ABAA").chars().map(|c| c.to_string()).collect();
    let out = arg(args, "--out", "server.json");
    let dt = 1.0 / NET_SERVER_MAX_TICK_RATE;
    let sp = spec(&root);
    let mut node = ServerNode::bind(&format!("127.0.0.1:{}", arg(args, "--port", "0")), World::new(sp.clone(), dt), duel(&root, Some(rounds)), LS, "", dt, n_clients).expect("bind");
    node.movement = Some(movement_host(&root));
    node.set_sim(sim(args, 11));
    println!("PORT {}", node.local_addr().port());
    let mut hit_attack: Option<(String, f64)> = None;
    let mut finish_at: Option<u64> = None;
    let mut truth: Vec<Value> = vec![];
    let t0 = Instant::now();
    let mut last_progress = Instant::now();
    loop {
        let now = t0.elapsed().as_secs_f64();
        let r = node.poll(now, &mut |srv, m| {
            // the server's hit detection for this tick (test_net_match.gd)
            let won = m.mode_events.iter().filter(|e| matches!(e, ModeEvent::Other { kind, .. } if kind == "round_won")).count();
            let w = winners[won.min(winners.len() - 1)].clone();
            let l = if w == "A" { "B" } else { "A" };
            if let (Some(wi), Some(_)) = (srv.world.fighter_index(&w), srv.world.fighter_index(l)) {
                let m0 = srv.world.cur_m(wi).unwrap();
                if let Some(a) = m0.attack() {
                    let key = (w.clone(), m0.start_time);
                    if a.stage == 1 && hit_attack.as_ref() != Some(&key) {
                        hit_attack = Some(key);
                        let at = srv.world.now + srv.world.dt;
                        srv.world.at(at, Input::Contact { who: w.clone(), target: l.into(), bone: "Head".into() });
                    }
                }
            }
        });
        match r {
            ServerPoll::Waiting => {
                if node.frame == 0 && last_progress.elapsed() > TIMEOUT {
                    eprintln!("mh-net-node: no clients");
                    std::process::exit(2);
                }
                std::thread::sleep(Duration::from_micros(300));
            }
            ServerPoll::Stepped(n) => {
                last_progress = Instant::now();
                let w = wall();
                for p in ["A", "B"] {
                    if let Some(m) = node.pawn_movement(p) {
                        truth.push(json!([w, p, m.location.x, m.location.y, m.location.z]));
                    }
                }
                if finish_at.is_none() && node.m.mode_events.iter().any(|e| matches!(e, ModeEvent::Other { kind, .. } if kind == "match_finished")) {
                    finish_at = Some(n + 30); // half a second for the last replication to arrive
                }
                if finish_at == Some(n) || now > 300.0 {
                    node.end();
                    break;
                }
            }
            ServerPoll::Ended => break,
        }
    }
    node.flush(Duration::from_secs(5));
    let m = &node.m;
    let wins: Vec<Value> = m
        .mode_events
        .iter()
        .filter_map(|e| match e {
            ModeEvent::Other { kind, data } if kind == "round_won" => Some(data["winner"].clone()),
            _ => None,
        })
        .collect();
    let finished = m.mode_events.iter().any(|e| matches!(e, ModeEvent::Other { kind, .. } if kind == "match_finished"));
    let r = json!({"frames": node.frame, "finished": finished, "round_winners": wins, "deaths": node.srv.world.deaths(),
        "rejected": node.srv.rejected, "not_owner": node.srv.not_owner, "sent": node.t.stats().0,
        "resends": node.t.resends, "lost_by_sim": node.t.dropped_by_sim, "moves_received": node.moves_received,
        "stale_gen_moves": node.stale_gen_moves, "stale_actor": node.srv.stale_actor, "truth": truth});
    std::fs::write(&out, serde_json::to_string(&r).unwrap()).unwrap();
}

fn client(args: &[String]) {
    let root = PathBuf::from(arg(args, "--root", "."));
    let id: u32 = arg(args, "--id", "1").parse().unwrap();
    let name = arg(args, "--name", "A");
    let other = if name == "A" { "B" } else { "A" };
    let winners: Vec<String> = arg(args, "--winners", "ABAA").chars().map(|c| c.to_string()).collect();
    let out = arg(args, "--out", "client.json");
    let saddr: std::net::SocketAddr = arg(args, "--server", "127.0.0.1:7777").parse().unwrap();
    let dt = 1.0 / 60.0;
    let sp = spec(&root);
    let play = round_play(&duel(&root, None));
    let mut node = ClientNode::join(saddr, id, &name, "", World::new(sp.clone(), dt), dt).expect("bind");
    node.movement = Some(movement_host(&root));
    node.set_sim(sim(args, 100 + id as u64));
    let mut pid = -1;
    let mut view: Vec<Value> = vec![];
    let mut own_frames = 0u64;
    let t0 = Instant::now();
    let mut last_progress = Instant::now();
    loop {
        let now = t0.elapsed().as_secs_f64();
        // the scripted player: walk back and forth (2 s each way); the round's winner (by its own room game) attacks
        // when idle in RoundPlay
        let yaw = if name == "A" { 0.0 } else { 180.0 };
        node.move_input = ExeInput { fwd: if (now % 4.0) < 2.0 { 1.0 } else { -1.0 }, yaw, ..Default::default() };
        let c = &mut node.c;
        if let Some(rg) = c.pc_props.get(ROOM_GAME) {
            let seen = (rg["team1_wins"].as_i64().unwrap_or(0) + rg["team2_wins"].as_i64().unwrap_or(0)) as usize;
            if rg["round"]["stage"].as_i64() == Some(play) && winners[seen.min(winners.len() - 1)] == name && c.own == name {
                if let Some(i) = c.world.fighter_index(&name) {
                    let idle = c.world.cur_m(i).map(|m| m.kind() == "Idle").unwrap_or(false);
                    let queued = c.world.schedule.contains_key(&(c.world.tick_n + 1));
                    if idle && !queued {
                        c.world.input(Input::Attack { who: name.clone(), mv: 0, angle: 0.0 });
                    }
                }
            }
        }
        match node.poll(now) {
            ClientPoll::Refused(e) => {
                eprintln!("mh-net-node: login refused: {e}");
                std::process::exit(3);
            }
            ClientPoll::Joined { player_id } => pid = player_id,
            ClientPoll::Ended => break,
            ClientPoll::Stepped(_) => {
                last_progress = Instant::now();
                if node.own_move.is_some() {
                    own_frames += 1;
                }
                if let Some(p) = node.pawn_location(other) {
                    // the drawn (mesh) position and the proxy capsule's simulated one
                    let raw = node.proxies.get(other).map(|q| q.location()).unwrap_or(p);
                    view.push(json!([wall(), other, p.x, p.y, p.z, raw.x, raw.y, raw.z]));
                }
            }
            ClientPoll::Waiting => {
                if last_progress.elapsed() > TIMEOUT {
                    eprintln!("mh-net-node: server silent");
                    std::process::exit(2);
                }
                std::thread::sleep(Duration::from_micros(300));
            }
        }
    }
    node.flush(Duration::from_secs(5));
    let c = &node.c;
    let hits = c.world.hits.iter().filter(|e| matches!(e.get("kind").and_then(|k| k.as_str()), Some("hit" | "parry" | "chamber"))).count();
    let all: Vec<&mh_net::movement::ClientPrediction> = node.past_moves.iter().chain(node.own_move.as_ref().map(|(_, cp)| cp)).collect();
    let sum = |f: &dyn Fn(&mh_net::movement::ClientPrediction) -> u32| all.iter().map(|cp| f(cp)).sum::<u32>();
    let cp = json!({"packets": sum(&|c| c.packets), "combined": sum(&|c| c.combined), "old": sum(&|c| c.old_moves_sent), "dual": sum(&|c| c.duals_sent),
        "corrections": sum(&|c| c.corrections), "acks": sum(&|c| c.acks), "replays": sum(&|c| c.replays)});
    let r = json!({"frames": node.frame, "room_game": c.pc_props.get(ROOM_GAME), "deaths": c.world.deaths(),
        "dropped_rpcs": c.dropped_rpcs, "processed_hits": hits, "player_id": pid, "ping": c.get_ping(), "moves": cp, "own_frames": own_frames, "spawns": node.past_moves.len() + 1,
        "resends": node.t.resends, "lost_by_sim": node.t.dropped_by_sim, "view": view});
    std::fs::write(&out, serde_json::to_string(&r).unwrap()).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("server") => server(&args),
        Some("client") => client(&args),
        _ => {
            eprintln!("usage: mh-net-node server|client ... (see the file header)");
            std::process::exit(1);
        }
    }
}

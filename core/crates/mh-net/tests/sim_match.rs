//! A real online match (rust-net r7): the server's world is mh-sim's Sim (sim_server::SimWorld) - exe-exact combat,
//! weapon traces through the posed physics bodies, mh-mode bots - behind ServerNode on real localhost UDP; headless
//! ClientNodes predict their own pawn and see everything else replicated.
//!   - traced_hits_over_the_net: client A's attack (predicted, ServerAssignNetMotion) is traced on the server against
//!     B's posed bodies; the hit, B's health and the motions reach every client; no client processes a hit
//!   - server_bots_are_visible: two server bots fight; a client sees their pawns move (ReplicatedMovement), their
//!     attack motions (ReplicatedNetMotion) and the damage (ReplicatedHealth) the server's traces dealt
//!   - ffa_scale_64 (ignored by default; run with --ignored, best in release): 4 headless clients + 60 bots on one
//!     arena; the server's step time against NetServerMaxTickRate 60 (16.7 ms) and each client's bandwidth against
//!     MaxClientRate 100000 B/s (DefaultEngine.ini [/Script/OnlineSubsystemUtils.IpNetDriver])
//! Data: the spec matrix + paks (mh-sim load), core/tests/golden/character/records.json, mode/bots_duel.jsonl (bot
//! profile / tree), godot/data_gen/mode/mode_kismet.json; absent -> SKIP.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::{BoxWorld, CharacterRecords, CharacterSource, RecordsJson};
use mh_net::duel_match::{ModeEvent, ServerMode};
use mh_net::movement::{NetMoveCfg, SmoothCfg};
use mh_net::node::{ClientNode, MovementHost, ServerNode};
use mh_net::sim_server::{BotCfg, SimWorld};

use mh_sim::sim::Sim;
use mordhau_core::combat::{Input, World};
use mordhau_core::data::Spec;
use mordhau_core::ue::FVector;
use serde_json::Value;

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
const DT: f64 = 1.0 / 60.0;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

struct Data {
    spec: Rc<Spec>,
    geo: Rc<mh_sim::trace::Geometry>,
    rec: CharacterRecords,
    vfs: Arc<mh_pak::Vfs>,
    bots: BotCfg,
}

fn data() -> Option<Data> {
    let matrix = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).ok()?;
    let vfs = Arc::new(mh_pak::Vfs::mount_default().ok()?);
    let rec = RecordsJson(&std::fs::read_to_string(root().join("core/tests/golden/character/records.json")).ok()?).load().ok()?;
    let ld = mh_sim::load::load(&matrix, vfs.clone(), &[LS]).ok()?;
    let kt = std::fs::read_to_string(root().join("godot/data_gen/mode/mode_kismet.json")).ok()?;
    let ht = std::fs::read_to_string(root().join("core/tests/golden/mode/bots_duel.jsonl")).ok()?;
    let mut h: Value = serde_json::from_str(ht.lines().nth(1)?).ok()?;
    mordhau_core::data::decode_exact(&mut h);
    let bots = BotCfg {
        kismet: Arc::new(mh_mode::kismet::Kismet::from_json(&kt).ok()?),
        profile: serde_json::from_value(h["bots"][0]["profile"].clone()).ok()?,
        tree: serde_json::from_value(h["bots"][0]["tree"].clone()).ok()?,
        max_walk_speed: h["bodies"][0]["max_walk_speed"].as_f64()?,
    };
    Some(Data { spec: Rc::new(ld.spec), geo: Rc::new(ld.geo), rec, vfs, bots })
}

/// Arena: every login (player or bot) gets a pawn at once and keeps it; no rounds (the test's stand-in for a mode)
#[derive(Default)]
struct Arena {
    ev: Vec<ModeEvent>,
}
impl ServerMode for Arena {
    fn post_login(&mut self, ctrl: &str, _player_id: i64) {
        self.ev.push(ModeEvent::SpawnPawn { who: ctrl.into() });
    }
    fn logout(&mut self, _ctrl: &str) {}
    fn tick(&mut self, _dt: f64) {}
    fn drain(&mut self) -> Vec<ModeEvent> {
        std::mem::take(&mut self.ev)
    }
    fn set_alive(&mut self, _ctrl: &str, _alive: bool) {}
    fn team_of(&self, _ctrl: &str) -> i64 {
        -1
    }
    fn replicated_room_game(&self, _ctrl: &str) -> Option<Value> {
        None
    }
    fn login_bot(&mut self, ctrl: &str) -> bool {
        self.ev.push(ModeEvent::SpawnPawn { who: ctrl.into() });
        true
    }
}

fn floor() -> BoxWorld {
    let mut w = BoxWorld::new();
    w.add_box(FVector::new(-50000.0, -50000.0, -200.0), FVector::new(50000.0, 50000.0, 0.0));
    w
}

type Spawn = Rc<dyn Fn(&str) -> (FVector, f32)>;

fn host(d: &Data, spawn: Spawn) -> MovementHost {
    MovementHost { level: Box::new(floor()), records: d.rec.clone(), spawn: Box::new(move |n| spawn(n)), cfg: NetMoveCfg::default(), smooth: SmoothCfg::default(), horse: None }
}

struct Net {
    s: ServerNode<SimWorld, Arena>,
    c: Vec<ClientNode<World>>,
    now: f64,
}

fn net(d: &Data, clients: &[&str], spawn: Spawn) -> Net {
    let mut sim = Sim::new(d.spec.clone(), d.geo.clone(), d.rec.clone(), Box::new(floor()), DT as f32);
    sim.dedicated_server = true;
    let sp = spawn.clone();
    let mut w = SimWorld::new(sim, Box::new(move |n| sp(n)));
    w.bots = Some(d.bots.clone());
    w.poser = Some(mh_sim::anim::ClipPoser::new(mh_assets::pak_source::PakSource::new(d.vfs.clone())));
    let mut s = ServerNode::bind("127.0.0.1:0", w, Arena::default(), LS, "", DT, clients.len()).expect("bind");
    s.movement = Some(host(d, spawn.clone()));
    let addr = s.local_addr();
    let mut c = vec![];
    for (i, n) in clients.iter().enumerate() {
        let mut cl = ClientNode::join(addr, i as u32 + 1, n, "", World::new(d.spec.clone(), DT), DT).expect("join");
        cl.movement = Some(host(d, spawn.clone()));
        c.push(cl);
    }
    Net { s, c, now: 0.0 }
}

impl Net {
    fn frame(&mut self) {
        self.now += DT;
        for _ in 0..3 {
            self.s.poll(self.now, &mut |_, _| {});
            for c in self.c.iter_mut() {
                c.poll(self.now);
            }
            std::thread::sleep(std::time::Duration::from_micros(150));
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
    fn server(&self) -> &World {
        &self.s.srv.world.sim.combat
    }
}

fn health(w: &World, n: &str) -> Option<i64> {
    w.fighter_index(n).map(|i| w.fighters[i].health)
}

fn hits(w: &World) -> Vec<(String, String)> {
    w.hits.iter().filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("hit")).map(|e| {
        let g = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        (g("attacker"), g("victim"))
    }).collect()
}

#[test]
fn traced_hits_over_the_net() {
    let Some(d) = data() else { return eprintln!("SKIP: spec matrix / paks / golden data missing") };
    // A faces B 120 cm ahead (inside a longsword's reach), B faces A
    let spawn: Spawn = Rc::new(|n: &str| {
        let z = 96.0 + AVG_FLOOR_DIST;
        if n == "A" { (FVector::new(0.0, 0.0, z), 0.0) } else { (FVector::new(120.0, 0.0, z), 180.0) }
    });
    let mut n = net(&d, &["A", "B"], spawn);
    assert!(n.until(5.0, &|n| n.c.iter().all(|c| c.own_move.is_some()) && n.c[0].proxies.contains_key("B")), "pawns never spawned");
    n.until(1.0, &|_| false);
    let b0 = health(n.server(), "B").unwrap();
    // A attacks whenever it is idle (predicted on its client, ServerAssignNetMotion to the server)
    let t0 = n.now;
    while n.now < t0 + 8.0 && hits(n.server()).is_empty() {
        let w = &mut n.c[0].c.world;
        if let Some(i) = w.fighter_index("A") {
            let idle = w.cur_m(i).map(|m| m.kind() == "Idle").unwrap_or(false);
            let queued = w.schedule.contains_key(&(w.tick_n + 1));
            if idle && !queued {
                w.input(Input::Attack { who: "A".into(), mv: 0, angle: 0.0 });
            }
        }
        n.frame();
    }
    n.until(1.0, &|_| false);
    let sh = hits(n.server());
    let hb = health(n.server(), "B").unwrap();
    let (ha, hbb) = (health(&n.c[0].c.world, "B").unwrap(), health(&n.c[1].c.world, "B").unwrap());
    let client_hits = n.c.iter().map(|c| hits(&c.c.world).len()).sum::<usize>();
    eprintln!("server traced hits {sh:?}; B health server {b0} -> {hb}, on A {ha}, on B {hbb}; client-processed hits {client_hits}; rejected {}", n.s.srv.rejected);
    assert!(sh.iter().any(|(a, _)| a == "A"), "no traced hit by A on the server");
    assert!(hb < b0, "B took no damage");
    assert_eq!((ha, hbb), (hb, hb), "B's health did not replicate");
    assert_eq!(client_hits, 0, "a client processed a hit");
    assert_eq!(n.s.srv.rejected, 0);
}

#[test]
fn server_bots_are_visible() {
    let Some(d) = data() else { return eprintln!("SKIP: spec matrix / paks / golden data missing") };
    let spawn: Spawn = Rc::new(|n: &str| {
        let z = 96.0 + AVG_FLOOR_DIST;
        match n {
            "C" => (FVector::new(0.0, -600.0, z), 90.0),
            "Bot1" => (FVector::new(0.0, 0.0, z), 0.0),
            _ => (FVector::new(150.0, 0.0, z), 180.0),
        }
    });
    let mut n = net(&d, &["C"], spawn);
    assert!(n.until(5.0, &|n| n.c[0].own_move.is_some()), "client never spawned");
    assert!(n.s.m.add_bot("Bot1") && n.s.m.add_bot("Bot2"));
    assert!(n.until(3.0, &|n| n.c[0].proxies.contains_key("Bot1") && n.c[0].proxies.contains_key("Bot2")), "bot pawns not replicated");
    let mut attacks_seen = 0;
    let mut moved = false;
    let p0 = n.c[0].pawn_location("Bot1").unwrap();
    let t0 = n.now;
    let mut seen_motion = BTreeMap::new();
    while n.now < t0 + 40.0 {
        n.frame();
        let w = &n.c[0].c.world;
        for b in ["Bot1", "Bot2"] {
            if let Some(i) = w.fighter_index(b) {
                let k = w.cur_m(i).map(|m| (m.kind(), m.start_time.to_bits()));
                if let Some((kind, st)) = k {
                    if kind == "Attack" && seen_motion.insert((b, st), ()).is_none() {
                        attacks_seen += 1;
                    }
                }
            }
        }
        let p = n.c[0].pawn_location("Bot1").unwrap();
        moved |= ((p.x - p0.x).powi(2) + (p.y - p0.y).powi(2)).sqrt() > 20.0;
        let dead = ["Bot1", "Bot2"].iter().any(|b| health(n.server(), b).map(|h| h <= 0).unwrap_or(true));
        if dead && attacks_seen > 0 {
            break;
        }
    }
    n.until(1.0, &|_| false);
    let sh = hits(n.server());
    let (s1, s2) = (health(n.server(), "Bot1").unwrap_or(0), health(n.server(), "Bot2").unwrap_or(0));
    let (c1, c2) = (health(&n.c[0].c.world, "Bot1").unwrap_or(0), health(&n.c[0].c.world, "Bot2").unwrap_or(0));
    eprintln!("bots: server traced hits {}, health server {s1}/{s2} client {c1}/{c2}; client saw {attacks_seen} bot attacks; Bot1 moved {moved}", sh.len());
    assert!(moved, "the client never saw Bot1 move");
    assert!(attacks_seen > 0, "the client never saw a bot attack");
    assert!(!sh.is_empty(), "the bots never hit each other on the server");
    assert_eq!((c1, c2), (s1, s2), "bot health differs on the client");
}

/// 64 pawns: 4 headless clients + 60 server bots on one arena (bots in facing pairs). Reports the server's world step
/// and whole-frame time against NetServerMaxTickRate 60's 16.7 ms and every client's received bytes per second against
/// MaxClientRate 100000 (DefaultEngine.ini), plus the engine-equivalent bits of the movement updates (FRepMovement's
/// packed size, RepMovement::wire_bits) - this transport's JSON payloads are larger than the engine's bunches.
#[test]
#[ignore]
fn ffa_scale_64() {
    let Some(d) = data() else { return eprintln!("SKIP: spec matrix / paks / golden data missing") };
    let names: Vec<String> = (0..60).map(|i| format!("Bot{i}")).collect();
    let spawn: Spawn = Rc::new(|n: &str| {
        let z = 96.0 + AVG_FLOOR_DIST;
        if let Some(i) = n.strip_prefix("Bot").and_then(|x| x.parse::<i32>().ok()) {
            // pairs 140 cm apart on a 6 x 5 grid, 600 cm between pairs
            let (p, side) = (i / 2, i % 2);
            let (gx, gy) = ((p % 6) as f32 * 600.0, (p / 6) as f32 * 600.0);
            return if side == 0 { (FVector::new(gx, gy, z), 0.0) } else { (FVector::new(gx + 140.0, gy, z), 180.0) };
        }
        let k = n.strip_prefix('P').and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
        (FVector::new(-800.0, k * 400.0, z), 0.0)
    });
    let clients = ["P0", "P1", "P2", "P3"];
    let mut n = net(&d, &clients, spawn);
    assert!(n.until(5.0, &|n| n.c.iter().all(|c| c.own_move.is_some())), "clients never spawned");
    for b in &names {
        assert!(n.s.m.add_bot(b));
    }
    assert!(n.until(5.0, &|n| n.c.iter().all(|c| c.proxies.len() >= 63)), "not every client sees 63 others");
    let bytes0: BTreeMap<u32, u64> = n.s.t.bytes_to.iter().map(|(k, v)| (*k, *v)).collect();
    let steps0 = n.s.srv.world.step_times.len();
    let mut frame_times = vec![];
    let secs = 10.0;
    let t0 = n.now;
    let wall0 = Instant::now();
    while n.now < t0 + secs {
        // client inputs: walk in circles
        for (i, c) in n.c.iter_mut().enumerate() {
            c.move_input = mh_character::ExeInput { fwd: 1.0, yaw: ((n.now * 40.0) as f32 + i as f32 * 90.0) % 360.0, ..Default::default() };
        }
        n.now += DT;
        let f0 = Instant::now();
        n.s.poll(n.now, &mut |_, _| {});
        frame_times.push(f0.elapsed().as_secs_f64());
        for _ in 0..2 {
            for c in n.c.iter_mut() {
                c.poll(n.now);
            }
            n.s.poll(n.now, &mut |_, _| {});
        }
    }
    let wall = wall0.elapsed().as_secs_f64();
    let st: Vec<f64> = n.s.srv.world.step_times[steps0..].to_vec();
    let pct = |v: &[f64], q: f64| {
        let mut x = v.to_vec();
        x.sort_by(|a, b| a.partial_cmp(b).unwrap());
        x[((x.len() - 1) as f64 * q) as usize]
    };
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let rates: Vec<f64> = (1..=clients.len() as u32).map(|c| (n.s.t.bytes_to.get(&c).copied().unwrap_or(0) - bytes0.get(&c).copied().unwrap_or(0)) as f64 / secs).collect();
    // engine-equivalent movement bits per client per second: 63 proxies' FRepMovement at the server rate when moving
    let rep_bits = mh_net::movement::rep_movement_of(n.s.pawn_movement("Bot0").unwrap()).wire_bits() as f64;
    let engine_move = 63.0 * rep_bits / 8.0 * 60.0;
    let hits_n = hits(n.server()).len();
    eprintln!(
        "64 pawns ({} clients + {} bots), {secs} s simulated in {wall:.1} s wall: world step mean {:.2} ms p95 {:.2} ms max {:.2} ms; server frame mean {:.2} ms p95 {:.2} ms (budget 16.67 ms at NetServerMaxTickRate 60); per-client bytes/s {:?} (MaxClientRate 100000; JSON payloads); engine-equivalent movement {:.0} B/s per client ({} bits per FRepMovement); traced hits {hits_n}; budget-deferred updates {}",
        clients.len(), names.len(), mean(&st) * 1e3, pct(&st, 0.95) * 1e3, pct(&st, 1.0) * 1e3, mean(&frame_times) * 1e3, pct(&frame_times, 0.95) * 1e3, rates.iter().map(|r| r.round()).collect::<Vec<_>>(), engine_move, rep_bits, n.s.budget_deferred
    );
    assert!(n.server().fighters.len() == 64);
    assert!(rates.iter().all(|r| *r > 0.0));
}

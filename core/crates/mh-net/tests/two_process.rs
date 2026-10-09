//! Three-process UDP duel on the free-running model (node.rs): a server process (NetServerMaxTickRate 60) and two
//! client processes (the mh-net-node binary, built by cargo for this test) on 127.0.0.1, every datagram delayed
//! 100 ms + up to 20 ms jitter and 2 % of them lost (each process's simulated outgoing network), play a Duel to 3 wins
//! (DuelMode, RoundsToWin overridden to 3; winners A B A A) while walking back and forth. Checks:
//!   - zero desyncs in the outcome: the server finishes A 3-1; both clients' room games 3-1; every machine saw the
//!     deaths B A B B; no rejected / non-owner / dropped RPC; no hit processed on a client
//!   - the proxy view: each client's drawn position of the other pawn (simulated proxy: ReplicatedMovement,
//!     extrapolation, smoothing) stays within the stated bounds of the server's truth at the same wall-clock time
//! Needs the ignored test data (core/tests/golden/spec.json, character/records.json, mode/duel_rooms.jsonl,
//! godot/data_gen/mode/mode_kismet.json); absent -> SKIP.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap()
}

fn wait_all(children: &mut [Child], limit: Duration) -> Vec<bool> {
    let t0 = Instant::now();
    loop {
        let st: Vec<Option<bool>> = children.iter_mut().map(|c| c.try_wait().unwrap().map(|s| s.success())).collect();
        if st.iter().all(|s| s.is_some()) {
            return st.into_iter().map(|s| s.unwrap()).collect();
        }
        if t0.elapsed() > limit {
            for c in children.iter_mut() {
                let _ = c.kill(); // only the processes this test started
            }
            panic!("mh-net-node processes did not finish within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// the server's truth of `pawn` at wall time t (linear between its frames), and whether a respawn teleport is near
fn truth_at(truth: &[(f64, [f64; 3])], t: f64) -> Option<([f64; 3], bool)> {
    let i = truth.partition_point(|s| s.0 < t);
    if i == 0 || i >= truth.len() {
        return None;
    }
    let (a, b) = (&truth[i - 1], &truth[i]);
    let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
    let p = [a.1[0] + (b.1[0] - a.1[0]) * k, a.1[1] + (b.1[1] - a.1[1]) * k, a.1[2] + (b.1[2] - a.1[2]) * k];
    // a jump of more than 50 cm between server frames within +-1 s is a respawn: the proxy may show the old place
    let lo = truth.partition_point(|s| s.0 < t - 1.0);
    let hi = truth.partition_point(|s| s.0 < t + 0.3).min(truth.len() - 1);
    let teleport = (lo.max(1)..=hi).any(|j| {
        let d = (0..3).map(|c| (truth[j].1[c] - truth[j - 1].1[c]).powi(2)).sum::<f64>().sqrt();
        d > 50.0
    });
    Some((p, teleport))
}

#[test]
fn three_process_udp_duel_lossy() {
    let r = root();
    for p in ["core/tests/golden/spec.json", "core/tests/golden/character/records.json", "core/tests/golden/mode/duel_rooms.jsonl", "godot/data_gen/mode/mode_kismet.json"] {
        if !r.join(p).exists() {
            eprintln!("SKIP: {p} missing");
            return;
        }
    }
    let exe = env!("CARGO_BIN_EXE_mh-net-node");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mh-net-three-process");
    std::fs::create_dir_all(&dir).unwrap();
    let out = |n: &str| dir.join(n).to_string_lossy().to_string();
    let rs = r.to_string_lossy().to_string();
    let net = ["--latency-ms", "100", "--jitter-ms", "20", "--loss", "0.02"];
    let mut server = Command::new(exe)
        .args(["server", "--root", &rs, "--port", "0", "--clients", "2", "--rounds", "3", "--winners", "ABAA", "--out", &out("server.json")])
        .args(net)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn server");
    let mut line = String::new();
    BufReader::new(server.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let port = line.trim().strip_prefix("PORT ").expect("server printed its port").to_string();
    let saddr = format!("127.0.0.1:{port}");
    let a = Command::new(exe).args(["client", "--root", &rs, "--server", &saddr, "--id", "1", "--name", "A", "--winners", "ABAA", "--out", &out("a.json")]).args(net).spawn().unwrap();
    let b = Command::new(exe).args(["client", "--root", &rs, "--server", &saddr, "--id", "2", "--name", "B", "--winners", "ABAA", "--out", &out("b.json")]).args(net).spawn().unwrap();
    let mut ps = vec![server, a, b];
    let ok = wait_all(&mut ps, Duration::from_secs(400));
    assert!(ok.iter().all(|x| *x), "process exit codes {ok:?}");
    let read = |n: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(out(n)).unwrap()).unwrap() };
    let (s, ca, cb) = (read("server.json"), read("a.json"), read("b.json"));
    let brief = |v: &Value| {
        let mut v = v.clone();
        v.as_object_mut().unwrap().remove("truth");
        v.as_object_mut().unwrap().remove("view");
        v
    };
    eprintln!("server {}\nA {}\nB {}", brief(&s), brief(&ca), brief(&cb));
    // ---- outcome: zero desyncs ----
    assert_eq!(s["finished"], true, "match did not finish");
    assert_eq!(s["round_winners"], serde_json::json!([0, 1, 0, 0]));
    assert_eq!(s["deaths"], serde_json::json!(["B", "A", "B", "B"]));
    assert_eq!((s["rejected"].as_i64(), s["not_owner"].as_i64()), (Some(0), Some(0)));
    assert!(s["lost_by_sim"].as_i64().unwrap() > 0 && s["resends"].as_i64().unwrap() > 0, "the simulated loss did not happen");
    for c in [&ca, &cb] {
        assert_eq!(c["deaths"], s["deaths"], "client deaths");
        assert_eq!((c["room_game"]["team1_wins"].as_i64(), c["room_game"]["team2_wins"].as_i64()), (Some(3), Some(1)));
        assert_eq!((c["dropped_rpcs"].as_i64(), c["processed_hits"].as_i64()), (Some(0), Some(0)));
        assert!(c["ping"].as_f64().unwrap() > 0.15, "ping median {} below the simulated round trip", c["ping"]);
    }
    assert_ne!(ca["player_id"], cb["player_id"]);
    // ---- the proxy view against the server's truth ----
    let truth_of = |p: &str| -> Vec<(f64, [f64; 3])> {
        s["truth"].as_array().unwrap().iter().filter(|e| e[1] == p).map(|e| (e[0].as_f64().unwrap(), [e[2].as_f64().unwrap(), e[3].as_f64().unwrap(), e[4].as_f64().unwrap()])).collect()
    };
    // errs: the drawn (visual, smoothed mesh) position; raw: the proxy capsule (SimulateMovement only)
    let (mut errs, mut raw) = (vec![], vec![]);
    for c in [&ca, &cb] {
        for e in c["view"].as_array().unwrap() {
            let p = e[1].as_str().unwrap();
            let tr = truth_of(p);
            let t = e[0].as_f64().unwrap();
            if let Some((q, teleport)) = truth_at(&tr, t) {
                if teleport {
                    continue;
                }
                let at = |i: usize| ((e[i].as_f64().unwrap() - q[0]).powi(2) + (e[i + 1].as_f64().unwrap() - q[1]).powi(2) + (e[i + 2].as_f64().unwrap() - q[2]).powi(2)).sqrt();
                errs.push(at(2));
                raw.push(if e.as_array().unwrap().len() >= 8 { at(5) } else { at(2) });
            }
        }
    }
    errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    raw.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |q: f64| errs[((errs.len() as f64 - 1.0) * q) as usize];
    let rpct = |q: f64| raw[((raw.len() as f64 - 1.0) * q) as usize];
    eprintln!("proxy error vs server truth over {} samples: VISUAL median {:.1} cm, p95 {:.1} cm, max {:.1} cm; RAW capsule median {:.1}, p95 {:.1}, max {:.1}", errs.len(), pct(0.5), pct(0.95), pct(1.0), rpct(0.5), rpct(0.95), rpct(1.0));
    assert!(errs.len() > 1000, "too few proxy samples {}", errs.len());
    // a proxy shows the server's state of one trip earlier (120 ms one way plus frame phases), extrapolated; at walking
    // speed that is a few tens of cm, more right after a direction change
    assert!(pct(0.5) < 60.0, "median proxy error {:.1} cm", pct(0.5));
    assert!(pct(0.95) < 150.0, "p95 proxy error {:.1} cm", pct(0.95));
    // the owning clients kept predicting: most frames sent a move or were combined into one, and the server
    // acknowledged most of them
    for c in [&ca, &cb] {
        let mv = &c["moves"];
        assert!(mv["packets"].as_i64().unwrap() > 300 && mv["acks"].as_i64().unwrap() > 200, "client moves {mv}");
    }
}

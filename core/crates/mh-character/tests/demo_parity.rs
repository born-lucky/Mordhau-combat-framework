//! Movement parity against the real game: the decoded demos (state/parity/movement/*.jsonl, docs/PARITY.md section 7),
//! summarised by scripts/parity/movement_metrics.py into state/parity/movement_metrics.json (Triternion / user data:
//! ignored path, never committed). The same metrics are measured on exe mode and, with `--features reference_compat`,
//! on the GDScript reference, and printed side by side (`-- --nocapture`). Without the metrics file the tests only
//! print the model side.
//!
//! What the replicated stream supports (and the tolerances used):
//!  - speed classes: only ratios between classes are testable (equipment / armor speed factors are not in the stream).
//!    Velocity is SerializePackedVector<1,24> (whole cm/s), so a ratio of two plateaus carries +-1 cm/s on each.
//!  - jump: a least-squares parabola through the falling samples (positions 0.01 cm) gives gravity, takeoff Vz, apex
//!    and air time independent of the sampling phase; the model trace is fitted the same way. Tolerance: the demo's
//!    p10..p90 band.
//!  - acceleration, braking and landing depend on the player's input, which the stream does not carry: reported, not
//!    asserted.

use mh_character::exe::{ExeInput, ExeMovement, Mode};
use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::uemath::v;
use mh_character::world::BoxWorld;
use mh_character::{CharacterRecords, CharacterSource, RecordsJson};
use serde_json::Value;
use std::path::PathBuf;

const DT: f32 = 1.0 / 60.0;

fn root() -> PathBuf {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    dir.join("../../..")
}

fn rec() -> CharacterRecords {
    let p = root().join("core/tests/golden/character/records.json");
    let txt = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e} (run godot/tools/export_golden_character.gd)", p.display()));
    RecordsJson(&txt).load().unwrap()
}

fn demo() -> Option<Value> {
    let p = root().join("state/parity/movement_metrics.json");
    let t = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&t).ok()
}

/// one model sample: time, position (z up), velocity, walking
#[derive(Clone, Copy)]
struct S {
    t: f64,
    x: f64,
    y: f64,
    z: f64,
    vx: f64,
    vy: f64,
    #[allow(dead_code)]
    vz: f64,
    walking: bool,
}

impl S {
    fn speed(&self) -> f64 {
        self.vx.hypot(self.vy)
    }
}

/// a scripted input: per frame (fwd, right, jump, sprint, crouch)
type Script = dyn Fn(usize) -> (f32, f32, bool, bool, bool);

trait Model {
    fn name(&self) -> &'static str;
    fn run(&self, frames: usize, script: &Script) -> Vec<S>;
}

struct Exe;
impl Model for Exe {
    fn name(&self) -> &'static str {
        "exe"
    }
    fn run(&self, frames: usize, script: &Script) -> Vec<S> {
        let mut w = BoxWorld::new();
        w.add_box(v(-50000.0, -50000.0, -200.0), v(50000.0, 50000.0, 0.0));
        let mut m = ExeMovement::new(&rec(), v(0.0, 0.0, 96.0 + AVG_FLOOR_DIST));
        m.world_time = 10.0;
        m.frame(&w, DT, &ExeInput::default());
        let mut out = Vec::new();
        for i in 0..frames {
            let (fwd, right, jump, sprint, crouch) = script(i);
            m.frame(&w, DT, &ExeInput { fwd, right, jump, sprint, crouch, ..Default::default() });
            out.push(S {
                t: (i + 1) as f64 * DT as f64,
                x: m.location.x as f64,
                y: m.location.y as f64,
                z: m.location.z as f64,
                vx: m.velocity.x as f64,
                vy: m.velocity.y as f64,
                vz: m.velocity.z as f64,
                walking: m.mode == Mode::Walking,
            });
        }
        out
    }
}

#[cfg(feature = "reference_compat")]
struct Reference;
#[cfg(feature = "reference_compat")]
impl Model for Reference {
    fn name(&self) -> &'static str {
        "reference"
    }
    fn run(&self, frames: usize, script: &Script) -> Vec<S> {
        use mh_character::compat::character::CharacterInput;
        use mh_character::compat::{Mode as RMode, MordhauMovement};
        use mh_character::ue::FVector;
        let mut m = MordhauMovement::new(&rec());
        m.set_on_floor(true);
        let mut ci = CharacterInput::default();
        let mut pos = FVector::new(0.0, 0.0, 0.0);
        let dt = 1.0 / 60.0;
        let mut out = Vec::new();
        for i in 0..frames {
            let (fwd, right, jump, sprint, crouch) = script(i);
            let o = ci.step(&mut m, dt, fwd as f64, right as f64, jump, sprint, crouch);
            pos = pos + o.delta;
            if pos.y <= 0.0 {
                pos.y = 0.0;
                m.set_on_floor(true);
            }
            // Godot axes (Y up, forward -Z) -> UE-like (x = -Z, y = X, z = Y): only horizontal size and height are used
            out.push(S {
                t: (i + 1) as f64 * dt,
                x: -pos.z as f64,
                y: pos.x as f64,
                z: pos.y as f64,
                vx: -m.velocity.z as f64,
                vy: m.velocity.x as f64,
                vz: m.velocity.y as f64,
                walking: m.mode == RMode::Walking,
            });
        }
        out
    }
}

fn models() -> Vec<Box<dyn Model>> {
    #[allow(unused_mut)]
    let mut v: Vec<Box<dyn Model>> = vec![Box::new(Exe)];
    #[cfg(feature = "reference_compat")]
    v.push(Box::new(Reference));
    v
}

/// least squares z = a + b t + c t^2 (as movement_metrics.py quad_fit)
fn quad_fit(p: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    if p.len() < 5 {
        return None;
    }
    let t0 = p[0].0;
    let mut s = [[0.0f64; 3]; 3];
    let mut r = [0.0f64; 3];
    for &(t, z) in p {
        let t = t - t0;
        let b = [1.0, t, t * t];
        for i in 0..3 {
            r[i] += b[i] * z;
            for j in 0..3 {
                s[i][j] += b[i] * b[j];
            }
        }
    }
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&s);
    if d.abs() < 1e-12 {
        return None;
    }
    let mut o = [0.0; 3];
    for k in 0..3 {
        let mut m = s;
        for i in 0..3 {
            m[i][k] = r[i];
        }
        o[k] = det(&m) / d;
    }
    let (a, b, c) = (o[0], o[1], o[2]);
    Some((a - b * t0 + c * t0 * t0, b - 2.0 * c * t0, c))
}

/// (gravity, takeoff Vz, apex, air time) of the first jump of a trace, fitted as the demo script does
fn jump_fit(tr: &[S]) -> (f64, f64, f64, f64) {
    let i = tr.iter().position(|s| !s.walking).expect("no jump");
    let land = i + tr[i..].iter().position(|s| s.walking).expect("no landing");
    let z0 = tr[i - 1].z;
    let pts: Vec<(f64, f64)> = tr[i..land].iter().map(|s| (s.t, s.z)).collect();
    let (a, b, c) = quad_fit(&pts).unwrap();
    let tp = -b / (2.0 * c);
    let apex = a + b * tp + c * tp * tp - z0;
    let disc = b * b - 4.0 * c * (a - z0);
    let r = disc.sqrt();
    let (mut t1, mut t2) = ((-b + r) / (2.0 * c), (-b - r) / (2.0 * c));
    if t1 > t2 {
        std::mem::swap(&mut t1, &mut t2);
    }
    (2.0 * c, b + 2.0 * c * t1, apex, t2 - t1)
}

fn plateau(m: &dyn Model, fwd: f32, right: f32, sprint: bool, crouch: bool) -> f64 {
    let tr = m.run(240, &move |_| (fwd, right, false, sprint, crouch));
    tr.last().unwrap().speed()
}

fn dmed(d: &Option<Value>, path: &[&str]) -> Option<f64> {
    let mut v = d.as_ref()?;
    for k in path {
        v = v.get(*k)?;
    }
    v.as_f64()
}

/// the most common demo plateau of a class near `want` (whole cm/s), from the plateau histogram
fn demo_class(d: &Option<Value>, key: &str, lo: i64, hi: i64) -> Option<f64> {
    let a = d.as_ref()?.get("plateaus")?.get(key)?.as_array()?;
    a.iter()
        .filter_map(|e| Some((e[0].as_i64()?, e[1].as_i64()?)))
        .filter(|(s, _)| (lo..=hi).contains(s))
        .max_by_key(|(_, n)| *n)
        .map(|(s, _)| s as f64)
}

#[test]
fn speed_class_ratios_match_demos() {
    let d = demo();
    // the most common class in the vanilla demos: walk 251 / sprint 451 / strafe 238 / backpedal 200 / crouch 220
    let walk = demo_class(&d, "standing", 245, 255);
    let mut fails: Vec<String> = Vec::new();
    let rows: [(&str, Option<f64>, f32, f32, bool, bool); 5] = [
        ("sprint / walk", demo_class(&d, "standing", 445, 455), 1.0, 0.0, true, false),
        ("strafe / walk", demo_class(&d, "standing", 233, 243), 0.0, 1.0, false, false),
        ("backpedal / walk", demo_class(&d, "standing", 195, 205), -1.0, 0.0, false, false),
        ("crouch / walk", demo_class(&d, "crouched", 215, 225), 1.0, 0.0, false, true),
        ("diagonal sprint / walk", demo_class(&d, "standing", 445, 455), 1.0, 1.0, true, false),
    ];
    for m in models() {
        let w = plateau(m.as_ref(), 1.0, 0.0, false, false);
        println!("[{}] walk plateau {:.3} cm/s (speed factor 1)", m.name(), w);
        for (name, dv, f, r, s, c) in rows {
            let mv = plateau(m.as_ref(), f, r, s, c) / w;
            match (dv, walk) {
                (Some(dv), Some(wk)) => {
                    let ratio = dv / wk;
                    // +-1 cm/s on both plateaus (whole cm/s on the wire)
                    let tol = (dv + 1.0) / (wk - 1.0) - ratio;
                    println!("  {name:24} model {mv:.4}  demo {ratio:.4} ({dv}/{wk})  tol {tol:.4}  {}", if (mv - ratio).abs() <= tol { "ok" } else { "DIFF" });
                    if (mv - ratio).abs() > tol {
                        fails.push(format!("[{}] {name}: model {mv} demo {ratio}", m.name()));
                    }
                }
                _ => println!("  {name:24} model {mv:.4}  (no demo metrics)"),
            }
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[test]
fn jump_arc_matches_demos() {
    let d = demo();
    let mut fails: Vec<String> = Vec::new();
    for m in models() {
        let tr = m.run(200, &|i| (0.0, 0.0, i == 60, false, false));
        let (g, vz, apex, air) = jump_fit(&tr);
        println!("[{}] jump fit: gravity {g:.1} cm/s2, takeoff Vz {vz:.1} cm/s, apex {apex:.2} cm, air {air:.4} s", m.name());
        for (name, mv, key) in [("gravity", g, "gravity_fit"), ("takeoff vz", vz, "vz_fit"), ("apex", apex, "apex_fit"), ("air time", air, "air_fit")] {
            let (Some(p10), Some(med), Some(p90)) =
                (dmed(&d, &["jump", key, "p10"]), dmed(&d, &["jump", key, "median"]), dmed(&d, &["jump", key, "p90"]))
            else {
                println!("  {name:12} model {mv:.3}  (no demo metrics)");
                continue;
            };
            let ok = (p10.min(p90)..=p10.max(p90)).contains(&mv);
            println!("  {name:12} model {mv:9.3}  demo median {med:9.3}  band [{p10:.3}, {p90:.3}]  {}", if ok { "ok" } else { "OUTSIDE" });
            // r3 saw takeoff Vz 434 / apex 80 here: an artefact of the demo's ReplicatedMovementMode arriving after
            // the move it belongs to (the last "walking" samples are already airborne, median 4.5 cm up at Vz 438).
            // r4's metric takes the ground height from the grounded samples before the rising run: Vz 449.8, apex
            // 86.10, air 0.766 s, gravity -1175 (79 jumps), so every jump metric is asserted.
            if !ok {
                fails.push(format!("[{}] {name}: model {mv} outside the demo band [{p10}, {p90}]", m.name()));
            }
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// input-dependent metrics: printed for the report, not asserted (the stream has no input)
#[test]
fn accel_braking_landing_report() {
    let d = demo();
    for m in models() {
        // from rest to 250 cm/s walking forward
        let tr = m.run(120, &|_| (1.0, 0.0, false, false, false));
        let k = tr.iter().position(|s| s.speed() >= 250.0);
        let acc = k.map(|k| (tr[k].t, tr[k].x.hypot(tr[k].y)));
        // walk plateau, release: time and distance to a stop
        let tr = m.run(300, &|i| (if i < 180 { 1.0 } else { 0.0 }, 0.0, false, false, false));
        let s0 = tr[179];
        let k = (180..tr.len()).find(|&k| tr[k].speed() == 0.0);
        let brk = k.map(|k| (tr[k].t - s0.t, (tr[k].x - s0.x).hypot(tr[k].y - s0.y), s0.speed()));
        // reverse input from the walk plateau
        let tr = m.run(300, &|i| (if i < 180 { 1.0 } else { -1.0 }, 0.0, false, false, false));
        let k = (180..tr.len()).find(|&k| tr[k].vx <= 0.0);
        let rev = k.map(|k| (tr[k].t - tr[179].t, (tr[k].x - tr[179].x).abs()));
        // sprint jump: ground speed / takeoff speed after landing
        let tr = m.run(400, &|i| (1.0, 0.0, i == 200, true, false));
        let i = tr.iter().position(|s| !s.walking).unwrap();
        let land = i + tr[i..].iter().position(|s| s.walking).unwrap();
        let v0 = tr[i - 1].speed();
        let ratios: Vec<String> = (0..10)
            .map(|b| {
                let k = land + b * 6;
                format!("{:.3}", tr.get(k).map(|s| s.speed() / v0).unwrap_or(f64::NAN))
            })
            .collect();
        println!("[{}] accel 0->250: {:?} (t s, dist cm); demo median t {:?} dist {:?}", m.name(), acc,
            dmed(&d, &["accel", "time", "median"]), dmed(&d, &["accel", "dist", "median"]));
        println!("[{}] release stop: {:?} (t s, dist cm, from cm/s); reverse-input stop {:?}; demo median t {:?} dist {:?} decel {:?}",
            m.name(), brk, rev, dmed(&d, &["braking", "time", "median"]), dmed(&d, &["braking", "dist", "median"]),
            dmed(&d, &["braking", "decel", "median"]));
        let demo_land: Vec<String> = (0..10)
            .map(|b| dmed(&d, &["landing_ratio", &format!("{:.1}", b as f64 / 10.0), "median"]).map_or("-".into(), |x| format!("{x:.3}")))
            .collect();
        println!("[{}] landing speed / takeoff speed per 0.1 s: model {:?}; demo median {:?}", m.name(), ratios, demo_land);
    }
}

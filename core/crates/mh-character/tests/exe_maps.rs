//! Exe-mode movement on real maps: mh_level::collision::CollisionWorld (the game's placed collision, landscape and
//! blocking volumes from the install) through the `World` trait. DU_Arena: spawn, walk to the other spawn, run into the
//! arena wall and jump against it. Camp: walk the landscape from a spawn in several directions. FFA_Camp demo
//! (smilevod1, vanilla): the replicated walking positions of every player checked against the collision world
//! (free of overlap, floor under them at the CMC floor distance) and short stretches replayed through the model.
//! SKIP (pass) without the install; the demo part skips without state/parity/movement (Triternion / user data).

use mh_character::exe::{ExeInput, ExeMovement, Mode};
use mh_character::exe_cmc::{AVG_FLOOR_DIST, MAX_FLOOR_DIST, MIN_FLOOR_DIST};
use mh_character::uemath::v;
use mh_character::world::World;
use mh_character::{CharacterRecords, CharacterSource, RecordsJson};
use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::path::PathBuf;
use std::sync::Arc;

const DT: f32 = 1.0 / 60.0;

fn root() -> PathBuf {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    dir.join("../../..")
}

fn rec() -> CharacterRecords {
    let p = root().join("core/tests/golden/character/records.json");
    RecordsJson(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).load().unwrap()
}

struct Map {
    d: mh_level::LevelData,
    w: CollisionWorld,
}

fn map(pkg_suffix: &str, tris: bool) -> Option<Map> {
    let vfs = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let want = format!("/{pkg_suffix}.umap").to_lowercase();
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with(&want))?.to_string();
    let pkg = pkg.trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs.clone()));
    let d = read(&pk, &pkg);
    let src = mh_assets::pak_source::PakSource::new(vfs);
    let tri = |p: &str| -> Option<Vec<[f64; 3]>> {
        let m = mh_assets::static_mesh::lod0(&src, p).ok()?;
        Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
    };
    let w = if tris { CollisionWorld::build(&pk, &d, Some(&tri)) } else { CollisionWorld::build(&pk, &d, None) };
    println!("{pkg}: {} bodies, {} spawns", w.bodies.len(), d.spawns.len());
    Some(Map { d, w })
}

/// a pawn standing on the floor under `p` (UE cm), the world clock past the get-up window (exe_world.rs WORLD_T0)
fn spawn_at(w: &dyn World, p: [f64; 3]) -> Option<ExeMovement> {
    let top = v(p[0] as f32, p[1] as f32, p[2] as f32 + 50.0);
    let h = w.line_trace(top, v(p[0] as f32, p[1] as f32, p[2] as f32 - 2000.0))?;
    let mut m = ExeMovement::new(&rec(), v(p[0] as f32, p[1] as f32, h.impact_point.z + 96.0 + AVG_FLOOR_DIST));
    m.world_time = 10.0;
    m.frame(w, DT, &ExeInput::default());
    Some(m)
}

fn free(m: &ExeMovement, w: &dyn World) -> bool {
    !w.overlap_capsule(m.location, m.capsule_radius() - 0.1, m.half_height() - 0.1)
}

fn yaw_to(m: &ExeMovement, x: f32, y: f32) -> f32 {
    (y - m.location.y).atan2(x - m.location.x).to_degrees()
}

#[test]
fn arena_walk_spawn_to_spawn_and_wall() {
    let Some(Map { d, w }) = map("DU_Arena", true) else { return };
    let red = d.spawns.iter().find(|s| s.name == "SpawnRed").expect("SpawnRed").xf.translation();
    let blue = d.spawns.iter().find(|s| s.name == "SpawnBlue").expect("SpawnBlue").xf.translation();
    let mut m = spawn_at(&w, red).expect("no floor under SpawnRed");
    assert_eq!(m.mode, Mode::Walking);
    let (bx, by) = (blue[0] as f32, blue[1] as f32);
    let mut frames = 0;
    while ((m.location.x - bx).powi(2) + (m.location.y - by).powi(2)).sqrt() > 60.0 && frames < 60 * 30 {
        let yaw = yaw_to(&m, bx, by);
        m.frame(&w, DT, &ExeInput { fwd: 1.0, yaw, ..Default::default() });
        assert!(free(&m, &w), "overlap at {:?} frame {frames}", m.location);
        frames += 1;
    }
    let dist = ((m.location.x - bx).powi(2) + (m.location.y - by).powi(2)).sqrt();
    println!("Arena: SpawnRed -> SpawnBlue in {:.2} s, end {:?} mode {:?}", frames as f32 * DT, m.location, m.mode);
    assert!(dist <= 60.0, "stopped {dist} cm short at {:?}", m.location);
    assert_eq!(m.mode, Mode::Walking);
    assert!(m.current_floor.is_walkable_floor());
    let fd = m.current_floor.floor_dist;
    assert!((MIN_FLOOR_DIST - 0.01..=MAX_FLOOR_DIST + 0.01).contains(&fd), "floor dist {fd}");

    // the arena wall in +Y from SpawnRed (mh-level's collision test): walking into it stops the pawn, a jump does not
    // clear it, nothing penetrates
    let mut m = spawn_at(&w, red).unwrap();
    for _ in 0..60 * 40 {
        m.frame(&w, DT, &ExeInput { fwd: 1.0, yaw: 90.0, ..Default::default() });
        assert!(free(&m, &w));
    }
    let y_wall = m.location.y;
    for i in 0..120 {
        m.frame(&w, DT, &ExeInput { fwd: 1.0, yaw: 90.0, jump: i == 0, ..Default::default() });
        assert!(free(&m, &w));
    }
    println!("Arena: stopped by the wall at y {y_wall:.2}, after a jump y {:.2}, mode {:?}", m.location.y, m.mode);
    assert!(m.velocity.y.abs() < 1.0, "still moving into the wall: {:?}", m.velocity);
    assert!((m.location.y - y_wall).abs() < 1.0, "jumped past the wall");
}

#[test]
fn camp_landscape_walks() {
    let Some(Map { d, w }) = map("Camp", false) else { return };
    let Some(s) = d.spawns.first() else { return };
    let p = s.xf.translation();
    let mut checked = 0;
    for yaw in [0.0f32, 90.0, 180.0, 270.0, 45.0, 225.0] {
        let Some(mut m) = spawn_at(&w, p) else { continue };
        let z0 = m.location.z;
        let (mut zmin, mut zmax) = (z0, z0);
        let start = m.location;
        for _ in 0..60 * 6 {
            m.frame(&w, DT, &ExeInput { fwd: 1.0, sprint: true, yaw, ..Default::default() });
            assert!(free(&m, &w), "overlap at {:?} (yaw {yaw})", m.location);
            assert!(m.location.z as f64 > w.kill_z);
            zmin = zmin.min(m.location.z);
            zmax = zmax.max(m.location.z);
        }
        let moved = ((m.location.x - start.x).powi(2) + (m.location.y - start.y).powi(2)).sqrt();
        println!("Camp yaw {yaw}: moved {moved:.0} cm, z {zmin:.0}..{zmax:.0}, end mode {:?}", m.mode);
        assert!(matches!(m.mode, Mode::Walking | Mode::Falling));
        checked += 1;
    }
    assert!(checked > 0);
}

/// one replicated sample: time (server stamp), location, velocity, movement mode
#[derive(Clone, Copy, Debug)]
struct Rep {
    t: f64,
    p: [f64; 3],
    vel: [f64; 3],
    mode: u8,
}

/// the players' tracks of a demo, streamed (one line at a time)
fn demo_tracks(name: &str) -> Option<Vec<Vec<Rep>>> {
    use std::io::BufRead;
    let p = root().join("state/parity/movement").join(name);
    let f = std::fs::File::open(p).ok()?;
    let mut tracks: std::collections::BTreeMap<i64, Vec<Rep>> = Default::default();
    let mut mode: std::collections::HashMap<i64, u8> = Default::default();
    for line in std::io::BufReader::new(f).lines() {
        let line = line.ok()?;
        let o: serde_json::Value = serde_json::from_str(&line).ok()?;
        let c = o["c"].as_i64().unwrap_or(-1);
        match o["k"].as_str() {
            Some("char") if o["p"] == "ReplicatedMovementMode" => {
                mode.insert(c, (o["v"].as_i64().unwrap_or(1) & 0xff) as u8);
            }
            Some("char") if o["p"] == "ReplayLastTransformUpdateTimeStamp" => {
                // the stamp follows the move of the same update
                if let (Some(tr), Some(f)) = (tracks.get_mut(&c), o["f"].as_f64()) {
                    if let Some(last) = tr.last_mut() {
                        if (last.t - o["t"].as_f64().unwrap_or(-1.0)).abs() < 1e-9 {
                            last.t = -f; // negative = stamped (sign fixed below)
                        }
                    }
                }
            }
            Some("move") => {
                let g = |k: &str| o[k].as_f64().unwrap_or(0.0);
                tracks.entry(c).or_default().push(Rep {
                    t: g("t"),
                    p: [g("x"), g("y"), g("z")],
                    vel: [g("vx"), g("vy"), g("vz")],
                    mode: *mode.get(&c).unwrap_or(&1),
                });
            }
            _ => {}
        }
    }
    Some(
        tracks
            .into_values()
            .map(|tr| tr.into_iter().filter(|r| r.t < 0.0).map(|r| Rep { t: -r.t, ..r }).collect())
            .collect(),
    )
}

/// FFA_Camp (smilevod1, vanilla): every steady walking sample (Vz 0, mode Walking on it and its neighbours) stands
/// free of the collision world with a floor under it at the CMC floor distance; stretches of 0.5 s replayed through
/// the model (start at the sample's location and velocity, input towards the next samples) end near the real path.
#[test]
fn ffa_camp_demo_paths_fit_the_world() {
    let Some(tracks) = demo_tracks("smilevod1.replay.jsonl") else {
        eprintln!("SKIP: no decoded demo");
        return;
    };
    let Some(Map { w, .. }) = map("FFA_Camp", true) else { return };
    let (mut n, mut overlaps, mut floor_ok) = (0usize, 0usize, 0usize);
    let mut fds: Vec<f64> = Vec::new();
    let mut overlap_at: Vec<[f64; 3]> = Vec::new();
    let mut by_body: std::collections::BTreeMap<String, usize> = Default::default();
    let mut land_depth: Vec<f64> = Vec::new();
    let mut land_comps: std::collections::BTreeMap<String, usize> = Default::default();
    for tr in &tracks {
        for k in (1..tr.len().saturating_sub(1)).step_by(5) {
            let (a, r, b) = (tr[k - 1], tr[k], tr[k + 1]);
            if !(a.mode == 1 && r.mode == 1 && b.mode == 1 && r.vel[2] == 0.0 && a.vel[2] == 0.0 && b.vel[2] == 0.0) {
                continue;
            }
            n += 1;
            let c = v(r.p[0] as f32, r.p[1] as f32, r.p[2] as f32);
            if w.overlap_capsule(c, 50.0 - 0.5, 96.0 - 0.5) {
                overlaps += 1;
                if overlap_at.len() < 5 {
                    overlap_at.push(r.p);
                }
                // which bodies: one overlap query per candidate body near the capsule
                let (lo, hi) = ([r.p[0] - 60.0, r.p[1] - 60.0, r.p[2] - 110.0], [r.p[0] + 60.0, r.p[1] + 60.0, r.p[2] + 110.0]);
                let land = w.overlap(r.p, 49.5, 95.5, &|x| w.blocks_pawn(x) && w.bodies[x as usize].kind == "landscape");
                if land {
                    *by_body.entry("landscape".into()).or_insert(0usize) += 1;
                    // how far up the capsule must move to clear it (1 mm steps up to 50 cm)
                    let mut dz = 0.0;
                    while dz < 50.0 && w.overlap([r.p[0], r.p[1], r.p[2] + dz], 49.5, 95.5, &|x| w.blocks_pawn(x)) {
                        dz += 0.1;
                    }
                    land_depth.push(dz);
                    for (b, bd) in w.bodies.iter().enumerate() {
                        if bd.kind == "landscape" && w.overlap(r.p, 49.5, 95.5, &|x| x == b as u32) {
                            *land_comps.entry(bd.name.clone()).or_insert(0usize) += 1;
                        }
                    }
                }
                for b in w.candidates(lo, hi) {
                    if w.blocks_pawn(b) && w.overlap(r.p, 49.5, 95.5, &|x| x == b) {
                        let bd = &w.bodies[b as usize];
                        *by_body.entry(format!("{} ({}, {})", bd.name, bd.mesh, bd.profile)).or_insert(0usize) += 1;
                    }
                }
                continue;
            }
            // floor distance as ComputeFloorDist measures it: a capsule sweep down
            let hits = w.sweep_capsule(c, v(c.x, c.y, c.z - 50.0), 50.0, 96.0);
            if let Some(h) = hits.iter().find(|h| h.is_valid_blocking_hit()) {
                let fd = (c.z - h.location.z) as f64;
                fds.push(fd);
                // net quantization 0.01 cm on the location
                if (MIN_FLOOR_DIST as f64 - 0.02..=MAX_FLOOR_DIST as f64 + 0.02).contains(&fd) {
                    floor_ok += 1;
                }
            }
        }
    }
    fds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| fds.get(((fds.len().max(1) - 1) as f64 * p) as usize).copied().unwrap_or(f64::NAN);
    println!(
        "FFA_Camp demo: {n} steady walking samples, {overlaps} overlap the collision world (first {overlap_at:?}), floor dist p10 {:.3} median {:.3} p90 {:.3}, within [{MIN_FLOOR_DIST}, {MAX_FLOOR_DIST}] {floor_ok}/{}",
        q(0.1), q(0.5), q(0.9), fds.len()
    );
    land_depth.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let ql = |p: f64| land_depth.get(((land_depth.len().max(1) - 1) as f64 * p) as usize).copied().unwrap_or(f64::NAN);
    println!("overlapping bodies: {by_body:?}; landscape depth p50 {:.1} p90 {:.1} max {:.1} cm; components {land_comps:?}", ql(0.5), ql(0.9), ql(1.0));
    assert!(n > 100, "too few samples");
    // Known (r4/r5): ~2.3 % of the positions sit 28.4 cm inside four adjacent landscape components (Landscape_0
    // heightfield collision 211 / 212 / 223 / 224). rust-pak r6 verified mh-level against the cooked data (collision =
    // render heightmap, no holes, identity transforms): a terrain edit between the demo's recording and the installed
    // 702625635 paks (same NetworkChecksum; content-only patches keep it). Only overlaps with other bodies are asserted.
    let land = by_body.get("landscape").copied().unwrap_or(0);
    assert!(((overlaps - land) as f64) < 0.002 * n as f64, "{} of {n} demo positions overlap non-landscape bodies", overlaps - land);
    assert!((floor_ok as f64) > 0.9 * fds.len() as f64, "floor distance outside the CMC band too often");

    // replay: 0.5 s walking stretches (no mode change, steady speed > 100), input = heading to the sample 0.1 s ahead
    let (mut runs, mut errs) = (0usize, Vec::<f64>::new());
    for tr in &tracks {
        let mut k = 0;
        while k + 1 < tr.len() && runs < 200 {
            let s0 = tr[k];
            let end = tr[k..].iter().position(|r| r.t - s0.t >= 0.5).map(|e| k + e);
            let Some(e) = end else { break };
            let seg = &tr[k..=e];
            let ok = seg.iter().all(|r| r.mode == 1 && r.vel[2] == 0.0 && r.vel[0].hypot(r.vel[1]) > 100.0);
            if !ok {
                k += 1;
                continue;
            }
            let Some(mut m) = spawn_at(&w, s0.p) else {
                k = e;
                continue;
            };
            m.location = v(s0.p[0] as f32, s0.p[1] as f32, s0.p[2] as f32);
            m.velocity = v(s0.vel[0] as f32, s0.vel[1] as f32, 0.0);
            let mut t = s0.t;
            let mut j = k;
            while t < seg.last().unwrap().t {
                while j + 1 <= e && tr[j].t < t + 0.1 {
                    j += 1;
                }
                let yaw = yaw_to(&m, tr[j].p[0] as f32, tr[j].p[1] as f32);
                // the demo speed class is unknown: a sprint-class speed is sprint input
                let sprint = s0.vel[0].hypot(s0.vel[1]) > 400.0;
                m.frame(&w, DT, &ExeInput { fwd: 1.0, yaw, sprint, ..Default::default() });
                t += DT as f64;
            }
            let last = seg.last().unwrap().p;
            errs.push(((m.location.x as f64 - last[0]).powi(2) + (m.location.y as f64 - last[1]).powi(2)).sqrt());
            runs += 1;
            k = e;
        }
    }
    errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| errs.get(((errs.len().max(1) - 1) as f64 * p) as usize).copied().unwrap_or(f64::NAN);
    println!("FFA_Camp replay: {runs} stretches of 0.5 s, end error p50 {:.1} cm p90 {:.1} cm max {:.1} cm", q(0.5), q(0.9), q(1.0));
    // the model is driven by a heading, not the player's inputs or speed class: a loose bound that still catches a
    // pawn blocked by geometry the real player walked through (it would fall ~0.5 s * speed short)
    assert!(runs > 20);
    assert!(q(0.9) < 60.0, "replayed stretches end far from the demo path");
}

/// A horse (exe_horse.rs) galloping on Camp's landscape from the spawn: it shifts up, never overlaps the world, and
/// the avoidance keeps it from driving into blocking geometry without rearing or deflecting
#[test]
fn camp_horse_gallops() {
    use mh_character::exe_horse::{horse_character_records, HorseCfg, HorseInput};
    let read = |rel: &str| std::fs::read_to_string(root().join("extract/json").join(rel)).ok();
    let Some(bp) = read("Mordhau/Content/Mordhau/Blueprints/Interactables/Animals/BP_Horse.json") else { return };
    let veh = read("Mordhau/Content/Mordhau/Blueprints/VehicleComponents/BP_VehicleHorse.json");
    let curve = |p: &str| read(&format!("{p}.json"));
    let cfg = HorseCfg::from_json(&bp, veh.as_deref(), &curve).unwrap();
    let Some(Map { d, w }) = map("Camp", false) else { return };
    let Some(s) = d.spawns.first() else { return };
    let p = s.xf.translation();
    let Some(h) = w.line_trace(v(p[0] as f32, p[1] as f32, p[2] as f32 + 50.0), v(p[0] as f32, p[1] as f32, p[2] as f32 - 2000.0)) else { return };
    let hrec = horse_character_records(&rec(), &bp).unwrap();
    let mut best = 0.0f32;
    for yaw in [0.0f32, 90.0, 180.0, 270.0] {
        let mut m = ExeMovement::new_horse(&hrec, &bp, cfg.clone(), v(p[0] as f32, p[1] as f32, h.impact_point.z + 120.0 + AVG_FLOOR_DIST)).unwrap();
        m.world_time = 10.0;
        m.set_yaw(yaw);
        m.horse.as_mut().unwrap().control_yaw = yaw;
        let start = m.location;
        let mut top_gear = 0;
        for _ in 0..60 * 8 {
            m.horse_frame(&w, DT, &HorseInput { fwd: 1.0, ..Default::default() });
            assert!(!w.overlap_capsule(m.location, m.capsule_radius() - 0.1, m.half_height() - 0.1), "horse overlap at {:?} (yaw {yaw})", m.location);
            top_gear = top_gear.max(m.horse.as_ref().unwrap().gear);
        }
        let moved = ((m.location.x - start.x).powi(2) + (m.location.y - start.y).powi(2)).sqrt();
        println!(
            "Camp horse yaw {yaw}: moved {moved:.0} cm, top gear {top_gear}, rears {}, end yaw {:.1}, mode {:?}",
            m.horse.as_ref().unwrap().rear_requests, m.yaw, m.mode
        );
        best = best.max(moved);
    }
    // the spawn is hemmed in by fences on some sides (the character runs above stop there too)
    assert!(best > 1000.0, "the horse never got going: {best} cm");
}

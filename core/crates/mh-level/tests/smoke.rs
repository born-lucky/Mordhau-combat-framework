//! All-maps spawn smoke (MH_LEVEL_SMOKE=1): every map with a generated Godot scene (godot/data_gen/maps/*.tscn) plus
//! every FFA / TDM / SKM map of the spec sheet (data_gen/spec/entities/map.json): each PlayerStart's character capsule
//! (r 50, hh 96: AMordhauCharacter ctor) must not start overlapping blocking geometry and must rest on a floor
//! (walkable normal) within FLOOR_CM below it. Prints a per-map table and the failures with what lies under them.

use mh_character::world::World;
use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

const R: f32 = 50.0;
const HH: f32 = 96.0;
/// a PlayerStart sits at most this far above its floor (its own capsule is 92 half height; UE spawns then the
/// character falls / is adjusted)
const FLOOR_CM: f64 = 150.0;

fn v(x: f64, y: f64, z: f64) -> mh_character::ue::FVector {
    mh_character::uemath::v(x as f32, y as f32, z as f32)
}

#[test]
fn all_maps_spawns_rest_on_floors() {
    if std::env::var("MH_LEVEL_SMOKE").is_err() {
        println!("  (set MH_LEVEL_SMOKE=1 for the all-maps spawn smoke)");
        return;
    }
    let Ok(vfs) = Vfs::mount_default() else { return };
    let vfs = Arc::new(vfs);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut maps: Vec<String> = vec![];
    if let Ok(t) = std::fs::read_to_string(repo.join("data_gen/spec/entities/map.json")) {
        let j: Value = serde_json::from_str(&t).unwrap();
        for r in j.as_object().unwrap().values() {
            let pre = r["values"]["FLD_MAP_MODE_PREFIX"].as_str().unwrap_or("");
            if matches!(pre, "FFA" | "TDM" | "SKM") {
                if let Some(p) = r["values"]["FLD_MAP_PACKAGE"].as_str() {
                    maps.push(p.trim_end_matches(".umap").to_string());
                }
            }
        }
    }
    for f in std::fs::read_dir(repo.join("godot/data_gen/maps")).into_iter().flatten().flatten() {
        let n = f.file_name().to_string_lossy().into_owned();
        if let Some(stem) = n.strip_suffix(".tscn") {
            if let Some(p) = vfs.list().find(|p| p.ends_with(&format!("/{stem}.umap"))) {
                maps.push(p.trim_end_matches(".umap").to_string());
            }
        }
    }
    maps.sort();
    maps.dedup();
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let tris = |pkg: &str| -> Option<Vec<[f64; 3]>> {
        let m = mh_assets::static_mesh::lod0(&src, pkg).ok()?;
        Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
    };
    // per start: camera / spectator starts (LevelLoad*, Spectator*) are placed in the air on purpose and only reported;
    // player starts are ok (no overlap, floor within FLOOR_CM), high (floor within HIGH_CM: the pawn drops onto it),
    // shallow (overlap depth <= SHALLOW_CM: UE's spawn adjustment pushes the pawn out), or failing
    const HIGH_CM: f64 = 600.0;
    const SHALLOW_CM: f64 = 40.0;
    let mut tot = [0usize; 6]; // ok, high, shallow, fail, camera, all
    let mut fails = vec![];
    let mut adj = [0usize; 2]; // overlapping starts the spawn adjustment resolves: shallow, deep
    println!("{:<26} {:>6} {:>5} {:>5} {:>7} {:>5} {:>6} {:>7} {:>6}", "map", "starts", "ok", "high", "shallow", "FAIL", "camera", "bodies", "secs");
    for m in &maps {
        let t0 = std::time::Instant::now();
        let pk = Pkgs::new(Reader::new(vfs.clone()));
        let d = read(&pk, m);
        let w = CollisionWorld::build(&pk, &d, Some(&tris));
        let mut c = [0usize; 6];
        for s in &d.spawns {
            c[5] += 1;
            let p = s.xf.translation();
            let camera = s.name.contains("LevelLoad") || s.name.contains("Spectator");
            let pen = w.sweep(p, p, R as f64, HH as f64, &|b| w.blocks_pawn(b));
            let depth = pen.iter().filter(|h| h.start_penetrating).map(|h| h.penetration_depth).fold(0.0, f64::max);
            let down = w.sweep_capsule(v(p[0], p[1], p[2]), v(p[0], p[1], p[2] - HIGH_CM), R, HH);
            let floor = down.iter().find(|h| h.is_valid_blocking_hit() && h.impact_normal.z > 0.7).map(|h| p[2] - h.location.z as f64);
            let cat0 = if camera {
                4
            } else if depth == 0.0 && floor.is_some_and(|f| f <= FLOOR_CM) {
                0
            } else if depth == 0.0 && floor.is_some() {
                1
            } else if depth > 0.0 && depth <= SHALLOW_CM {
                2
            } else {
                3
            };
            let cat = cat0;
            // the engine's spawn adjustment (AdjustIfPossibleButAlwaysSpawn -> UWorld::FindTeleportSpot) for every start
            // that overlaps: resolved when the adjusted capsule is free and stands on a floor
            let mut note = String::new();
            let cat = if (cat == 2 || cat == 3) && depth > 0.0 {
                let (q, ok) = w.spawn_adjust(p, R as f64, HH as f64);
                let free = !w.overlap_capsule(v(q[0], q[1], q[2]), R, HH);
                let down = w.sweep_capsule(v(q[0], q[1], q[2]), v(q[0], q[1], q[2] - HIGH_CM), R, HH);
                let fl = down.iter().find(|h| h.is_valid_blocking_hit() && h.impact_normal.z > 0.7).map(|h| q[2] - h.location.z as f64);
                let moved = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt();
                note = format!(" | adjust ok {ok} moved {moved:.0} cm free {free} floor {:?}", fl.map(|f| f.round()));
                if ok && free && fl.is_some() {
                    adj[if cat == 2 { 0 } else { 1 }] += 1;
                    0
                } else {
                    cat
                }
            } else {
                cat
            };
            c[cat] += 1;
            if cat == 3 {
                let below = w.line_trace(v(p[0], p[1], p[2]), v(p[0], p[1], p[2] - 2000.0));
                let what = below.map(|h| format!("{:.0} cm down: {}", p[2] - h.impact_point.z as f64, w.bodies[h.component.unwrap() as usize].name)).unwrap_or("nothing within 20 m".into());
                let ovb: Vec<String> = pen.iter().filter(|h| h.start_penetrating).take(2).map(|h| format!("{} [{}] depth {:.0}", w.bodies[h.body as usize].name, w.bodies[h.body as usize].profile, h.penetration_depth)).collect();
                fails.push(format!("{} {} at {:?}: overlap {:?}; floor {:?}; below {what}{note}", m.rsplit('/').next().unwrap(), s.name, p.map(|x| x.round()), ovb, floor.map(|f| f.round())));
            }
        }
        for k in 0..6 {
            tot[k] += c[k];
        }
        println!("{:<26} {:>6} {:>5} {:>5} {:>7} {:>5} {:>6} {:>7} {:>6.2}", m.rsplit('/').next().unwrap(), c[5], c[0], c[1], c[2], c[3], c[4], w.bodies.len(), t0.elapsed().as_secs_f64());
    }
    println!("TOTAL {} maps, {} starts: {} ok, {} high (floor {FLOOR_CM}-{HIGH_CM} cm below), {} shallow overlaps (<= {SHALLOW_CM} cm), {} FAIL, {} camera/spectator starts", maps.len(), tot[5], tot[0], tot[1], tot[2], tot[3], tot[4]);
    println!("spawn adjustment (FindTeleportSpot) resolved {} shallow and {} deep overlaps (counted as ok above)", adj[0], adj[1]);
    for f in &fails {
        println!("  FAIL {f}");
    }
    // regression gate (rust-pak r7 baseline 12, after the engine's spawn adjustment), each start judged:
    // - designer placement, FindTeleportSpot finds no free spot and the pawn spawns overlapping (AlwaysSpawn), the
    //   character movement then depenetrates: SKM/TDM_Crossroads BlueSpawn12 (RubblePile12, 57 cm), TDM_Dungeon32/64
    //   MordhauPlayerStart5_90 (throne table) and MordhauPlayerStart68 (chair), FFA_Feitoria_64 MordhauPlayerStart52
    //   (tavern table);
    // - designer placement, no ground: SKM_Castello_64 MordhauPlayerStart_25 (z -22694, nothing below) and FFA_Grad
    //   MordhauPlayerStart93 (nothing within 20 m);
    // - designer placement, high: FFA/SKM/TDM_ThePit MordhauPlayerStart1 (team -1) 9.2 m above the rocks, nothing placed
    //   in between (every mesh within 6 m collides);
    // none is missing collision on our side. More than 12 means collision went missing or wrong
    assert!(tot[3] <= 12, "{} player starts fail (baseline 12)", tot[3]);
}

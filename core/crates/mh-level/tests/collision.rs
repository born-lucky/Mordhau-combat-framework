//! The reference collision world on real Arena geometry, through mh-character's `World` trait: the floor under the
//! DU_Arena spawn, a capsule walking from SpawnRed towards SpawnBlue staying on the floor, a known wall blocking a
//! sweep, and the world settings (KillZ, gravity, damage volumes). SKIP (pass) without the install.

use mh_character::world::World;
use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::sync::Arc;

// AMordhauCharacter ctor CapsuleComponent: half height 96 (+0x458), radius 50 (+0x45c) - mh-character exe.rs
const R: f32 = 50.0;
const HH: f32 = 96.0;

fn v(x: f64, y: f64, z: f64) -> mh_character::ue::FVector {
    mh_character::uemath::v(x as f32, y as f32, z as f32)
}

fn arena() -> Option<(Pkgs, mh_level::LevelData, CollisionWorld)> {
    let vfs = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let pk = Pkgs::new(Reader::new(vfs.clone()));
    let d = read(&pk, "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena");
    // complex-as-simple meshes: LOD0 triangles from mh-assets
    let src = mh_assets::pak_source::PakSource::new(vfs);
    let tris = |pkg: &str| -> Option<Vec<[f64; 3]>> {
        let m = mh_assets::static_mesh::lod0(&src, pkg).ok()?;
        Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
    };
    let t0 = std::time::Instant::now();
    let w = CollisionWorld::build(&pk, &d, Some(&tris));
    println!(
        "DU_Arena collision: {} bodies ({} block the Pawn), {} shapes, skipped {:?}, KillZ {}, gravity {}, {} damage volumes ({:.2} s)",
        w.bodies.len(),
        w.blocks.iter().filter(|b| **b).count(),
        w.shapes.len(),
        w.skipped,
        w.kill_z,
        w.gravity_z,
        w.damage_volumes.len(),
        t0.elapsed().as_secs_f64()
    );
    Some((pk, d, w))
}

#[test]
fn arena_floor_walk_and_wall() {
    let Some((_pk, d, w)) = arena() else { return };
    assert_eq!(w.kill_z, -1048575.0);
    assert_eq!(w.gravity_z, -980.0);
    assert!(!w.damage_volumes.is_empty(), "Arena's PainCausingVolumes");
    let red = d.spawns.iter().find(|s| s.name == "SpawnRed").expect("SpawnRed").xf.translation();
    let blue = d.spawns.iter().find(|s| s.name == "SpawnBlue").expect("SpawnBlue").xf.translation();

    // the floor under the spawn: a downward ray hits an upward-facing surface
    let floor = w.line_trace(v(red[0], red[1], red[2]), v(red[0], red[1], red[2] - 1000.0)).expect("no floor under SpawnRed");
    assert!(floor.impact_normal.z > 0.9, "floor normal {:?}", floor.impact_normal);
    let fz = floor.impact_point.z as f64;
    println!("SpawnRed {red:?}: floor z {fz:.1} ({})", w.bodies[floor.component.unwrap() as usize].name);
    // the capsule standing on it is not overlapping anything; 5 cm lower it is
    let stand = fz + HH as f64 + 1.0;
    assert!(!w.overlap_capsule(v(red[0], red[1], stand), R, HH));
    assert!(w.overlap_capsule(v(red[0], red[1], stand - 6.0), R, HH));

    // walk towards SpawnBlue in 25 cm steps: horizontal sweep, then a 60 cm downward sweep to find the floor
    let (mut x, mut y, mut z) = (red[0], red[1], stand);
    let dir = [blue[0] - red[0], blue[1] - red[1]];
    let dl = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
    let step = [dir[0] / dl * 25.0, dir[1] / dl * 25.0];
    let mut walked = 0.0;
    let mut blocked = None;
    for _ in 0..(dl / 25.0) as usize {
        let hits = w.sweep_capsule(v(x, y, z), v(x + step[0], y + step[1], z), R, HH);
        if let Some(h) = hits.iter().find(|h| h.is_valid_blocking_hit() && h.impact_normal.z < 0.7) {
            blocked = Some((walked, w.bodies[h.component.unwrap() as usize].name.clone()));
            break;
        }
        x += step[0];
        y += step[1];
        // snap down onto the floor (step-down as the movement's floor check does)
        let down = w.sweep_capsule(v(x, y, z + 30.0), v(x, y, z - 60.0), R, HH);
        let g = down.iter().find(|h| h.is_valid_blocking_hit()).expect("walked off the floor");
        assert!(g.impact_normal.z > 0.7, "landed on a wall at {x},{y}");
        z = g.location.z as f64 + 1.0;
        walked += 25.0;
    }
    println!("walked {walked:.0} cm of {dl:.0} towards SpawnBlue, z {stand:.1} -> {z:.1}; blocked: {blocked:?}");
    assert!(walked >= 1000.0, "walked only {walked} cm");
    assert!((z - stand).abs() < 120.0);

    // a known wall: sweeping from the spawn across the arena (+Y) is stopped by a near-vertical surface
    let far = 8000.0;
    let hits = w.sweep_capsule(v(red[0], red[1], stand + 50.0), v(red[0], red[1] + far, stand + 50.0), R, HH);
    let wall = hits.iter().find(|h| h.is_valid_blocking_hit() && h.impact_normal.z.abs() < 0.3).expect("no wall within 80 m");
    let body = &w.bodies[wall.component.unwrap() as usize];
    println!(
        "wall at y {:.0} (distance {:.0} cm), normal {:?}, body {} ({}, profile {}, mesh {})",
        wall.location.y, wall.distance, wall.impact_normal, body.name, body.kind, body.profile, body.mesh
    );
    // the arena wall is a ring of convex pieces: the capsule meets a piece's edge, so the normal opposes the motion
    // (+Y) without being axis-aligned
    assert!(wall.impact_normal.y < -0.3, "wall normal does not oppose the motion: {:?}", wall.impact_normal);
    assert!(body.mesh.contains("arena_wall"), "blocked by {} instead of the arena wall", body.name);
    // just short of the hit the capsule is free, just past it it overlaps
    let yh = wall.location.y as f64;
    assert!(!w.overlap_capsule(v(red[0], yh - 1.0, stand + 50.0), R, HH));
    assert!(w.overlap_capsule(v(red[0], yh + 5.0, stand + 50.0), R, HH));
}

/// Camp's landscape heightfield in the collision world: a downward ray at collision vertices lands at their height
#[test]
fn camp_landscape_ground() {
    let Ok(vfs) = Vfs::mount_default() else { return };
    let pk = Pkgs::new(Reader::new(Arc::new(vfs)));
    let d = read(&pk, "Mordhau/Content/Mordhau/Maps/DuelCamp/Camp");
    let w = CollisionWorld::build(&pk, &d, None);
    let land = w.bodies.iter().filter(|b| b.kind == "landscape").count();
    assert_eq!(land, 144);
    let mut checked = 0;
    for c in d.landscape_collision.iter().step_by(17) {
        for (x, y) in [(10usize, 20usize), (40, 5), (33, 33)] {
            let p = c.vertex_world(x, y);
            // only where the landscape is the top surface at this point
            let Some(h) = w.line_trace(v(p[0], p[1], p[2] + 5.0), v(p[0], p[1], p[2] - 5.0)) else { continue };
            if w.bodies[h.component.unwrap() as usize].kind != "landscape" {
                continue;
            }
            assert!((h.impact_point.z as f64 - p[2]).abs() < 0.05, "{} ({x},{y}): {} vs {}", c.name, h.impact_point.z, p[2]);
            assert!(h.impact_normal.z > 0.5);
            checked += 1;
        }
    }
    println!("Camp: {land} landscape bodies, {} shapes, {checked} vertices hit at their height", w.shapes.len());
    assert!(checked > 10);
}

/// The heightfield fast path (cells under the query box) gives the same hits as every landscape triangle in the BVH
#[test]
fn camp_heightfield_fast_path_equals_triangles() {
    let Ok(vfs) = Vfs::mount_default() else { return };
    let pk = Pkgs::new(Reader::new(Arc::new(vfs)));
    let d = read(&pk, "Mordhau/Content/Mordhau/Maps/DuelCamp/Camp");
    let t0 = std::time::Instant::now();
    let fast = CollisionWorld::build(&pk, &d, None);
    let tf = t0.elapsed().as_secs_f64();
    let t0 = std::time::Instant::now();
    let slow = CollisionWorld::build_with(&pk, &d, None, mh_level::collision::BuildOptions { landscape_as_triangles: true });
    let ts = t0.elapsed().as_secs_f64();
    assert_eq!(fast.heightfields.len(), 144);
    let filt_f = |b: u32| fast.blocks_pawn(b);
    let filt_s = |b: u32| slow.blocks_pawn(b);
    // a grid of downward sweeps, diagonal sweeps and overlaps over the landscape
    let c0 = &d.landscape_collision[0];
    let (lo, hi) = d.landscape_collision.iter().fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), c| {
        let a = c.vertex_world(0, 0);
        let b = c.vertex_world(c.verts - 1, c.verts - 1);
        ([lo[0].min(a[0]).min(b[0]), lo[1].min(a[1]).min(b[1])], [hi[0].max(a[0]).max(b[0]), hi[1].max(a[1]).max(b[1])])
    });
    let _ = c0;
    let (mut n, mut hits) = (0, 0);
    let (mut tq_f, mut tq_s) = (0.0, 0.0);
    for i in 0..12 {
        for j in 0..12 {
            let x = lo[0] + (hi[0] - lo[0]) * (i as f64 + 0.37) / 12.0;
            let y = lo[1] + (hi[1] - lo[1]) * (j as f64 + 0.61) / 12.0;
            let (a, b) = ([x, y, 20000.0], [x + 300.0, y - 200.0, -5000.0]);
            let t = std::time::Instant::now();
            let hf = fast.sweep(a, b, 50.0, 96.0, &filt_f);
            tq_f += t.elapsed().as_secs_f64();
            let t = std::time::Instant::now();
            let hs = slow.sweep(a, b, 50.0, 96.0, &filt_s);
            tq_s += t.elapsed().as_secs_f64();
            n += 1;
            match (hf.first(), hs.first()) {
                (Some(p), Some(q)) => {
                    hits += 1;
                    assert!((p.time - q.time).abs() * 25000.0 < 0.05, "sweep {i},{j}: {} vs {}", p.time, q.time);
                    // at an edge / vertex contact several triangles touch at the same time: the fast path's first hit
                    // must be one of the reference's equally early hits
                    let same = hs.iter().filter(|h| (h.time - p.time).abs() * 25000.0 < 0.05).any(|h| (0..3).all(|k| (p.normal[k] - h.normal[k]).abs() < 1e-3));
                    assert!(same, "sweep {i},{j}: normal {:?} not among the reference's earliest hits", p.normal);
                }
                (None, None) => {}
                (p, q) => panic!("sweep {i},{j}: fast {:?} slow {:?}", p.map(|h| h.time), q.map(|h| h.time)),
            }
            if let Some(q) = hs.first() {
                let c = q.location;
                for dz in [-3.0, 3.0] {
                    let pc = [c[0], c[1], c[2] + dz];
                    assert_eq!(fast.overlap(pc, 50.0, 96.0, &filt_f), slow.overlap(pc, 50.0, 96.0, &filt_s));
                }
            }
        }
    }
    println!(
        "Camp: fast {} shapes + 144 heightfields (build {tf:.2} s), triangles {} shapes (build {ts:.2} s); {n} sweeps, {hits} hits equal; query time fast {tq_f:.2} s vs triangles {tq_s:.2} s",
        fast.shapes.len(),
        slow.shapes.len()
    );
    assert!(hits > 50);
}

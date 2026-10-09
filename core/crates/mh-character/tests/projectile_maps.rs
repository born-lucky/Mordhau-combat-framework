//! Projectile box sweeps on a real map (mh-level CollisionWorld::sweep_box, the projectile BoxComp channels): an arrow
//! (BP_ArrowProjectile, half extent 37.5 x 1 x 1) shot level at DU_Arena's wall stops with its box front at the wall,
//! 37.5 cm before the centre ray's hit; a box that starts inside geometry is a start-penetrating hit; a pawn-only
//! channel list lets nothing through that the Projectile channel blocks. SKIP (pass) without the install / extract.

use mh_character::projectile::{Projectile, ProjectileCfg};
use mh_character::uemath::{size, sub, v};
use mh_character::uequat::{rotator_quaternion, to_orientation_quat};
use mh_character::world::World;
use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::path::PathBuf;
use std::sync::Arc;

fn bp(rel: &str) -> Option<String> {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    std::fs::read_to_string(dir.join("../../../extract/json/Mordhau/Content/Mordhau/Blueprints").join(rel)).ok()
}

fn arrow() -> Option<ProjectileCfg> {
    let j: Option<Vec<String>> = ["Equipment/Ranged/BP_ArrowProjectile.json", "Equipment/Ranged/BP_MissileProjectile.json", "BP_MordhauProjectile.json"].iter().map(|p| bp(p)).collect();
    let j = j?;
    let r: Vec<&str> = j.iter().map(|s| s.as_str()).collect();
    Some(ProjectileCfg::from_json_chain(&r).unwrap())
}

fn arena() -> Option<(mh_level::LevelData, CollisionWorld)> {
    let vfs = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/du_arena.umap"))?.trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs));
    let d = read(&pk, &pkg);
    let w = CollisionWorld::build(&pk, &d, None);
    Some((d, w))
}

#[test]
fn arrow_box_hits_wall_before_centre_ray() {
    let Some(cfg) = arrow() else { return eprintln!("SKIP: no extract/json") };
    let Some((d, w)) = arena() else { return };
    let red = d.spawns.iter().find(|s| s.name == "SpawnRed").expect("SpawnRed").xf.translation();
    let blue = d.spawns.iter().find(|s| s.name == "SpawnBlue").expect("SpawnBlue").xf.translation();
    // from red towards blue and beyond, 150 cm up: the far wall
    let start = v(red[0] as f32, red[1] as f32, red[2] as f32 + 150.0);
    let dir = v((blue[0] - red[0]) as f32, (blue[1] - red[1]) as f32, 0.0);
    let n = size(dir);
    let end = v(start.x + dir.x / n * 20000.0, start.y + dir.y / n * 20000.0, start.z);
    let ray = w.line_trace(start, end).expect("the arena wall");
    let q = to_orientation_quat(sub(end, start));
    let hits = w.sweep_box(start, end, cfg.box_extent, q, &cfg.box_responses);
    let first = hits.iter().find(|h| h.blocking_hit).expect("box hit (volumes are NoCollision: mh-world r1)");
    let gap = ray.distance - first.distance;
    println!("ray {:.2} box {:.2} gap {gap:.2} body {:?} start_pen {}", ray.distance, first.distance, first.component, first.start_penetrating);
    if let Some(b) = first.component.and_then(|c| w.bodies.get(c as usize)) {
        println!("  {} {} {} {} {} {:?}", b.name, b.kind, b.source, b.profile, b.object_type, b.collision_enabled);
    }
    // the box front (37.5 ahead of the centre) reaches the wall first; a slanted wall face or the 1 cm half width
    // moves it a little
    assert!((gap - 37.5).abs() < 3.0, "gap {gap}");
    // a box placed straddling the wall starts penetrating
    let at = v(start.x + dir.x / n * ray.distance, start.y + dir.y / n * ray.distance, start.z);
    let hits = w.sweep_box(at, v(at.x, at.y, at.z + 1.0), cfg.box_extent, q, &cfg.box_responses);
    assert!(hits.iter().any(|h| h.start_penetrating), "straddling box");
}

#[test]
fn arrow_flight_stops_at_wall() {
    let Some(cfg) = arrow() else { return eprintln!("SKIP: no extract/json") };
    let Some((d, w)) = arena() else { return };
    let red = d.spawns.iter().find(|s| s.name == "SpawnRed").expect("SpawnRed").xf.translation();
    let blue = d.spawns.iter().find(|s| s.name == "SpawnBlue").expect("SpawnBlue").xf.translation();
    let yaw = ((blue[1] - red[1]) as f32).atan2((blue[0] - red[0]) as f32).to_degrees();
    let start = v(red[0] as f32, red[1] as f32, red[2] as f32 + 150.0);
    let mut p = Projectile::fire_quat(cfg, start, rotator_quaternion(2.0, yaw, 0.0), 0.0);
    let mut world_hit = None;
    for i in 0..300 {
        let hits = p.tick(&w, &[], 1.0 / 60.0, w.gravity_z as f32);
        if let Some(h) = hits.into_iter().find(|h| h.blocking) {
            world_hit = Some((i, h));
            break;
        }
    }
    let (i, h) = world_hit.expect("the arrow hits the arena");
    println!("frame {i}: {:?}", h.location);
    assert!(i > 0, "not at the muzzle");
    assert!(!p.simulating);
    // the box ends outside geometry, at the wall
    assert!(!w.sweep_box(p.location, v(p.location.x, p.location.y, p.location.z + 0.01), p.cfg.box_extent, p.quat, &p.cfg.box_responses).iter().any(|h| h.start_penetrating && h.penetration_depth > 0.5));
}

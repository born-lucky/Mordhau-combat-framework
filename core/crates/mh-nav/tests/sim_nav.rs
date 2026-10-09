//! Bots on a real map (rust-mode-ai r4): mh-sim with DU_Arena's collision (mh_level CollisionWorld) and this crate's
//! navmesh attached (Sim::nav: the map's cooked RecastNavMesh, r5). Two Knight bots spawned at SpawnRed / SpawnBlue (records from the spec matrix: the
//! exe-mode data path) find each other, path over the navmesh and close to melee range. SKIP without the install /
//! spec / character records.

use mh_character::{CharacterSource, RecordsJson};
use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_nav::Bounds;
use mh_pak::{Reader, Vfs};
use mh_sim::{FighterDesc, Sim};
use mordhau_core::ue::FVector;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn arena_bots_path_and_meet() {
    let Ok(vfs) = Vfs::mount_default().map(Arc::new) else { return eprintln!("SKIP: no paks") };
    let Ok(matrix) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return eprintln!("SKIP: no spec") };
    let Ok(rec) = std::fs::read_to_string(repo().join("core/tests/golden/character/records.json")) else { return eprintln!("SKIP: no character records") };
    let records = RecordsJson(&rec).load().unwrap();
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/du_arena.umap")).unwrap().trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs.clone()));
    let d = read(&pk, &pkg);
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let tri = |p: &str| -> Option<Vec<[f64; 3]>> {
        let m = mh_assets::static_mesh::lod0(&src, p).ok()?;
        Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
    };
    let w = CollisionWorld::build(&pk, &d, Some(&tri));
    let bounds: Vec<Bounds> = d.volumes.iter().filter(|v| v.class.contains("NavMeshBoundsVolume")).map(|v| Bounds { min: v.min.map(|c| c as f32), max: v.max.map(|c| c as f32) }).collect();
    // the game's own navmesh (cooked RecastNavMesh) when the map has one, else the voxel stand-in
    let nav = mh_nav::cooked::nav_for_map(&pk, &pkg, &w, &bounds);
    eprintln!("DU_Arena: cooked navmesh {}", mh_nav::cooked::find(&pk, &pkg).is_some());
    let ld = mh_sim::load::load(&matrix, vfs.clone(), &[LS]).expect("load");
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), records, Box::new(w), 1.0 / 60.0);
    s.nav = Some(nav);
    let k = Arc::new(mh_mode::spec_bots::kismet(&matrix).unwrap());
    let profile = mh_mode::spec_bots::profile(&matrix, "BOTBEHAVIOR_Knight").unwrap();
    let tree = mh_mode::spec_bots::tree(&matrix, "BT_Deathmatch").unwrap();
    let sp = |n: &str| {
        let t = d.spawns.iter().find(|s| s.name == n).unwrap().xf.translation();
        FVector::new(t[0] as f32, t[1] as f32, t[2] as f32 + 10.0)
    };
    let (ra, rb) = (sp("SpawnRed"), sp("SpawnBlue"));
    let yaw_ab = (rb.y - ra.y).atan2(rb.x - ra.x).to_degrees();
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: ra, yaw: yaw_ab, ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: rb, yaw: yaw_ab + 180.0, ..Default::default() });
    let mut bots = mh_mode::ai::Bots { k: k.clone(), ..Default::default() };
    for fi in [a, b] {
        let mut body = mh_mode::ai::BotBody::new(&s.combat.fighters[fi].name);
        body.location = s.movers[fi].location;
        body.yaw = s.yaw[fi] as f64;
        body.weapon_length = s.combat.fighters[fi].tracer.length;
        body.max_walk_speed = 300.0;
        body.pawn = mh_mode::ai::combat_host::pawn_view(&s.combat, fi);
        body.weapon = mh_mode::ai::combat_host::weapon_view(&s.combat, fi);
        body.inventory = vec![mh_mode::ai::InventoryItem { weapon: body.weapon.clone(), is_fists: false, ranged: false }];
        bots.add_body(body);
        s.bot_fighter.push(fi);
    }
    for bi in 0..2 {
        let now = s.combat.now;
        let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut s.combat, fighter_of: s.bot_fighter.clone() };
        bots.add_bot(bi, profile.clone(), Some(&tree), now, &mut host);
    }
    s.bots = Some(bots);
    let d0 = (ra - rb).length();
    let mut closest = d0;
    let mut corners = 0;
    for f in 0..(60 * 60) {
        let inp = s.bot_inputs();
        s.step(&inp);
        s.drain();
        corners = corners.max(s.bot_follow.iter().map(|p| p.path.len()).max().unwrap_or(0));
        let dist = (s.movers[a].location - s.movers[b].location).length();
        closest = closest.min(dist);
        if dist < 250.0 {
            eprintln!("met after {:.1} s (start {d0:.0} cm apart), longest path {corners} corners", f as f32 / 60.0);
            break;
        }
    }
    assert!(s.bot_follow.len() == 2 && corners >= 1, "the bots followed navmesh paths");
    assert!(closest < 250.0, "the bots met (closest {closest:.0} cm of {d0:.0})");
}

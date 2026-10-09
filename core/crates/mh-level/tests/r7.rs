//! Round 7: gameplay actors as data, and the engine's spawn adjustment. SKIP (pass) without the install.

use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::collections::BTreeMap;
use std::sync::Arc;

fn pkgs() -> Option<Pkgs> {
    Vfs::mount_default().ok().map(|v| Pkgs::new(Reader::new(Arc::new(v))))
}

const MAPS: &str = "Mordhau/Content/Mordhau/Maps/";

/// Every Blueprint actor is listed with its class chain and native root; per category counts on maps that have each
/// kind (MountainPeak ladders, Feitoria siege / doors, Grad Frontline objectives, Horde chests)
#[test]
fn gameplay_actors() {
    let Some(pk) = pkgs() else { return };
    let mut natives: BTreeMap<String, BTreeMap<&str, usize>> = BTreeMap::new();
    for m in ["MaxMap/SKM_MountainPeak_64", "FeitoriaMap/FL_Feitoria", "Grad/FL_Grad", "Castello/HRD_Castello", "Arena_Map/DU_Arena"] {
        let pkg = format!("{MAPS}{m}");
        if !pk.rd.exists(&pkg) {
            println!("(no {m})");
            continue;
        }
        let d = read(&pk, &pkg);
        let mut cats: BTreeMap<&str, usize> = BTreeMap::new();
        for a in &d.gameplay {
            *cats.entry(a.category).or_default() += 1;
            let c = &d.gameplay_classes[&a.class];
            *natives.entry(c.native.clone()).or_default().entry(a.category).or_default() += 1;
            // every actor's class resolves to a package and a native root
            assert!(!c.pkg.is_empty(), "{} has no class package", a.class);
        }
        let ex: Vec<String> = ["door", "ladder", "siege", "destructible", "pickup", "objective", "horde_shop"]
            .iter()
            .filter_map(|k| d.gameplay.iter().find(|a| a.category == *k).map(|a| format!("{k}: {} ({} comps)", a.class, a.components.len())))
            .collect();
        println!("{m}: {} gameplay actors, {} classes, {cats:?}\n   e.g. {ex:?}", d.gameplay.len(), d.gameplay_classes.len());
    }
    println!("native roots -> categories:");
    for (n, c) in &natives {
        println!("   {n:<40} {c:?}");
    }
    let d = read(&pk, &format!("{MAPS}MaxMap/SKM_MountainPeak_64"));
    assert!(d.gameplay.iter().any(|a| a.category == "ladder"), "no ladders on MountainPeak");
    let lad = d.gameplay.iter().find(|a| a.category == "ladder").unwrap();
    assert!(lad.xf.is_some() && !lad.components.is_empty(), "{lad:?}");
}

/// The engine's spawn adjustment moves a capsule out of a box it overlaps (UWorld::FindTeleportSpot), and leaves a
/// free capsule alone
#[test]
fn spawn_adjustment_on_arena() {
    let Some(pk) = pkgs() else { return };
    let d = read(&pk, &format!("{MAPS}Arena_Map/DU_Arena"));
    let w = CollisionWorld::build(&pk, &d, None);
    let red = d.spawns.iter().find(|s| s.name == "SpawnRed").unwrap().xf.translation();
    // free: unchanged
    let (q, ok) = w.spawn_adjust(red, 50.0, 96.0);
    assert!(ok && q == red);
    // 5 cm into the floor (no other body near): the adjustment is the floor body's MTD, mostly +Z, and the first
    // attempt (Z only) frees the capsule
    let floor_z = 5.0; // arena_floor_16 under SpawnRed (tests/collision.rs)
    let sunk = [red[0], red[1], floor_z + 96.0 - 5.0];
    assert!(w.encroaching(sunk, 50.0, 96.0, None));
    let mut adj = [0.0; 3];
    w.encroaching(sunk, 50.0, 96.0, Some(&mut adj));
    let (q, ok) = w.spawn_adjust(sunk, 50.0, 96.0);
    println!("sunk {sunk:?}: adjustment {adj:?} -> {q:?} ok {ok}");
    assert!(adj[2] > 4.0, "{adj:?}");
    assert!(ok && !w.encroaching(q, 50.0, 96.0, None) && q[2] > sunk[2]);
    // deep (36 cm) into the multi-shape floor: the engine sums one MTD per body, each its first overlapping shape's
    // (FBodyInstance::OverlapTest), which is not enough here; FindTeleportSpot gives up and the pawn spawns where
    // it was (AdjustIfPossibleButAlwaysSpawn)
    let deep = [red[0], red[1], floor_z + 96.0 - 36.0];
    let (q, ok) = w.spawn_adjust(deep, 50.0, 96.0);
    println!("deep {deep:?} -> {q:?} ok {ok}");
    assert!(ok || q == deep);
}

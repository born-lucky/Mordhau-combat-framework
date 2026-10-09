use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use mh_world::{Kind, World};
use std::collections::BTreeMap;
use std::sync::Arc;

#[test]
#[ignore]
fn explore() {
    let Some(v) = Vfs::mount_default().ok() else { return };
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    for m in ["FeitoriaMap/FL_Feitoria", "DuelCamp/Camp", "MaxMap/SKM_MountainPeak_64"] {
        let pkg = format!("Mordhau/Content/Mordhau/Maps/{m}");
        if !pk.rd.exists(&pkg) { println!("no {m}"); continue; }
        let d = read(&pk, &pkg);
        let w = World::from_level(&pk, &d);
        let mut c: BTreeMap<String, usize> = BTreeMap::new();
        for a in &w.actors {
            let k = match &a.kind { Kind::Door(_) => "door", Kind::Destructible(_) => "destructible", Kind::Objective(_) => "objective", Kind::Ladder(_) => "ladder", Kind::EquipmentSpawner(_) => "equip", Kind::VehicleSpawner(_) => "vehicle", Kind::Pushable(_) => "pushable", Kind::ProgressDriver(_) => "driver", Kind::SlaveDriver { .. } => "slave", Kind::ProgressActor(_) => "progress", Kind::DoorLocker { .. } => "locker", Kind::KillObjective(_) => "killobj", Kind::Inert => "inert", _ => "other" };
            *c.entry(format!("{k} {}", a.class)).or_default() += 1;
        }
        println!("== {m}: {} actors", w.actors.len());
        for (k, n) in c { if !k.starts_with("inert") || n > 0 { println!("  {n:4} {k}"); } }
        for (cp, os) in &w.objectives { println!("  cp {} -> {:?} state {:?}", w.actors[*cp].name, os.iter().map(|o| &w.actors[*o].name).collect::<Vec<_>>(), w.cps.get(cp).map(|c| (c.owning_team, c.objective_progress))); }
        if let Some(a) = w.actors.iter().find(|a| matches!(a.kind, Kind::Door(_))) {
            if let Kind::Door(d) = &a.kind { println!("  door {} closed {} fwd {} back {} spd {} {} {} kick {} origin {:?} hp {} df {}", a.name, d.yaw_closed, d.yaw_fwd, d.yaw_back, d.open_speed, d.close_speed, d.fast_speed, d.can_kick, d.push_origin_rel, d.health.health, a.props.f("DamageFactor")); }
        }
        if let Some(a) = w.actors.iter().find(|a| matches!(a.kind, Kind::Ladder(_))) {
            if let Kind::Ladder(l) = &a.kind { println!("  ladder {} state {} start {:?} end {:?} exit {:?} curve keys {} {} dropped {:?}", a.name, l.state, l.start, l.end, l.exit, l.raise_curve.keys.len(), l.drop_curve.keys.len(), l.dropped_rel.m); }
        }
    }
}

/// every map: the pushables, with their spline and curves (cargo test -p mh-world --test explore pushables -- --ignored --nocapture)
#[test]
#[ignore]
fn pushables() {
    let Some(v) = Vfs::mount_default().ok() else { return };
    let maps: Vec<String> = v.list().filter(|p| p.contains("/Maps/") && p.ends_with(".umap")).map(|p| p.trim_end_matches(".umap").to_string()).collect();
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    for m in maps {
        let d = read(&pk, &m);
        let w = World::from_level(&pk, &d);
        for a in &w.actors {
            let p = match &a.kind {
                Kind::Pushable(p) => Some(p),
                Kind::Objective(o) => o.push.as_deref(),
                _ => None,
            };
            if let Some(p) = p {
                println!("{m}: {} {} progress {} spline {} len {:.0} curves {} {} auto {} contested-stop {} thresholds {:?}", a.name, a.class, p.progress, p.spline.is_some(), p.spline.as_ref().map(|s| s.length()).unwrap_or(0.0), p.team1_curve.is_some(), p.team2_curve.is_some(), p.auto_move_if_alone, p.stop_if_contested, p.non_pullable);
            }
        }
        pk.clear();
    }
}

/// every map: classes mh-level categorizes door / destructible / objective / pickup / siege and the Kind they got
#[test]
#[ignore]
fn categories() {
    let Some(v) = Vfs::mount_default().ok() else { return };
    let maps: Vec<String> = v.list().filter(|p| p.contains("/Maps/") && p.ends_with(".umap")).map(|p| p.trim_end_matches(".umap").to_string()).collect();
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    let mut c: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for m in maps {
        let d = read(&pk, &m);
        let w = World::from_level(&pk, &d);
        for (a, g) in w.actors.iter().zip(&d.gameplay) {
            if !["door", "destructible", "objective", "pickup", "siege", "ladder"].contains(&g.category) { continue; }
            let k = match &a.kind { Kind::Door(_) => "door", Kind::Destructible(_) => "destructible", Kind::Objective(_) => "objective", Kind::Ladder(_) => "ladder", Kind::EquipmentSpawner(_) => "equip", Kind::VehicleSpawner(_) => "vehicle", Kind::Pushable(_) => "pushable", Kind::ProgressDriver(_) => "driver", Kind::SlaveDriver { .. } => "slave", Kind::ProgressActor(_) => "progress", Kind::DoorLocker { .. } => "locker", Kind::KillObjective(_) => "killobj", Kind::Inert => "INERT", _ => "other" };
            let e = c.entry(format!("{} {} [{}]", g.category, a.class, a.chain.join(">"))).or_insert((0, k.into()));
            e.0 += 1;
        }
        pk.clear();
    }
    for (k, (n, kind)) in c { println!("{n:5} {kind:12} {k}"); }
}

/// every map: progress drivers with their targets / slaves, door lockers, kill objectives
#[test]
#[ignore]
fn drivers() {
    let Some(v) = Vfs::mount_default().ok() else { return };
    let maps: Vec<String> = v.list().filter(|p| p.contains("/Maps/") && p.ends_with(".umap")).map(|p| p.trim_end_matches(".umap").to_string()).collect();
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    for m in maps {
        let d = read(&pk, &m);
        let w = World::from_level(&pk, &d);
        for a in &w.actors {
            match &a.kind {
                Kind::ProgressDriver(dr) => println!("DRIVER {m} {} {} value {} sm {} targets {:?} slaves {}", a.name, a.class, dr.value, dr.smoothed, dr.targets.iter().map(|t| &w.actors[*t].class).collect::<Vec<_>>(), dr.slaves.len()),
                Kind::DoorLocker { doors, .. } => println!("LOCKER {m} {} doors {} raw {:?}", a.name, doors.len(), a.props.get("Doors To Lock").map(|v| v.to_string().chars().take(200).collect::<String>())),
                Kind::KillObjective(k) => println!("KILLOBJ {m} {} {} point {:?} health {}", a.name, a.class, k.point.map(|p| &w.actors[p].name), k.health),
                _ => {}
            }
        }
        pk.clear();
    }
}

/// Census of every placed Blueprint actor class over every map: maps, count, category, native root, chain, kind
/// (cargo test -p mh-world --test explore census -- --ignored --nocapture > state/world_census.tsv)
#[test]
#[ignore]
fn census() {
    let Some(v) = Vfs::mount_default().ok() else { return };
    let maps: Vec<String> = v.list().filter(|p| p.contains("/Maps/") && p.ends_with(".umap")).map(|p| p.trim_end_matches(".umap").to_string()).collect();
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    // class -> (count, maps, category, native, chain, kind)
    let mut c: BTreeMap<String, (usize, std::collections::BTreeSet<String>, String, String, String, String)> = BTreeMap::new();
    for m in maps {
        let d = read(&pk, &m);
        let w = World::from_level(&pk, &d);
        let short = m.rsplit('/').next().unwrap_or(&m).to_string();
        for (a, g) in w.actors.iter().zip(&d.gameplay) {
            let k = format!("{:?}", std::mem::discriminant(&a.kind));
            let kind = match &a.kind { Kind::Door(_) => "Door", Kind::Destructible(_) => "Destructible", Kind::Objective(_) => "Objective", Kind::Pushable(_) => "Pushable", Kind::Ladder(_) => "Ladder", Kind::ProgressDriver(_) => "ProgressDriver", Kind::SlaveDriver { .. } => "SlaveDriver", Kind::ProgressActor(_) => "ProgressActor", Kind::DoorLocker { .. } => "DoorLocker", Kind::KillObjective(_) => "KillObjective", Kind::EquipmentSpawner(_) => "EquipmentSpawner", Kind::VehicleSpawner(_) => "VehicleSpawner", Kind::Pickup(_) => "Pickup", Kind::Cauldron(_) => "Cauldron", Kind::DeliverySpawn { .. } => "DeliverySpawn", Kind::Inert if w.triggers.contains_key(&a.id) => "Trigger", Kind::Inert => "-" };
            let _ = k;
            let e = c.entry(a.class.clone()).or_insert((0, Default::default(), g.category.to_string(), d.gameplay_classes[&a.class].native.clone(), a.chain.join(">"), kind.to_string()));
            e.0 += 1;
            e.1.insert(short.clone());
        }
        pk.clear();
    }
    println!("CENSUS\tclass\tcount\tmaps\tcategory\tnative\tkind\tchain");
    for (k, (n, maps, cat, native, chain, kind)) in c {
        println!("CENSUS\t{k}\t{n}\t{}\t{cat}\t{native}\t{kind}\t{chain}", maps.len());
    }
}

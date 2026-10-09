//! mh-world r2 on real map data: door and destructible depth, gates / levers, the Feitoria door locker, the Frontline
//! kill objective, client replication. SKIP (pass) without the install.

use mh_level::{read, LevelData, Pkgs};
use mh_pak::{Reader, Vfs};
use mh_world::{ActorId, CharId, CharView, DamageInfo, Kind, Queries, SpawnedStatus, World, WorldEvent};
use std::collections::BTreeMap;
use std::sync::Arc;

fn pkgs() -> Option<Pkgs> {
    Vfs::mount_default().ok().map(|v| Pkgs::new(Reader::new(Arc::new(v))))
}
fn map(pk: &Pkgs, m: &str) -> Option<LevelData> {
    let p = format!("Mordhau/Content/Mordhau/Maps/{m}");
    pk.rd.exists(&p).then(|| read(pk, &p))
}

#[derive(Default)]
struct Q {
    chars: BTreeMap<ActorId, Vec<CharId>>,
    actors: BTreeMap<ActorId, Vec<ActorId>>,
}
impl Queries for Q {
    fn overlapping_chars(&self, a: ActorId) -> Vec<CharId> {
        self.chars.get(&a).cloned().unwrap_or_default()
    }
    fn overlapping_actors(&self, a: ActorId) -> Vec<ActorId> {
        self.actors.get(&a).cloned().unwrap_or_default()
    }
    fn spawned_status(&self, _: ActorId) -> SpawnedStatus {
        SpawnedStatus::default()
    }
}
fn run(w: &mut World, q: &Q, chars: &[CharView], secs: f64) -> Vec<WorldEvent> {
    let mut ev = vec![];
    for _ in 0..(secs * 60.0).round() as usize {
        ev.extend(w.tick(1.0 / 60.0, q, chars));
    }
    ev
}
fn player(team: u8) -> DamageInfo {
    DamageInfo { causer: Some(1), instigator: Some(1), instigator_player: true, instigator_team: Some(team), attack_move: Some(0), ..Default::default() }
}

/// Every door class on FL_Feitoria. Construction leaves ReplicatedHealth = floor(Health). The close timeline takes
/// |start - closed| / DoorCloseSpeed and ends on the closed yaw. A hit takes Damage x DamageFactor. At 0 the door
/// breaks (OnDeath), with DeleteWhenDestroyed (hidden, collision off, gone 1 s later) or DisableCollisionWhenDestroyed
/// (collision off); invulnerable classes (DamageFactor 0) never break
#[test]
fn feitoria_doors() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let mut w = World::from_level(&pk, &d);
    let q = Q::default();
    w.begin_play(&q);
    let mut classes: BTreeMap<String, ActorId> = BTreeMap::new();
    for a in &w.actors {
        if matches!(a.kind, Kind::Door(_)) {
            classes.entry(a.class.clone()).or_insert(a.id);
        }
    }
    assert!(classes.len() >= 2, "{classes:?}");
    for (class, id) in classes {
        let p = w.actors[id].props.clone();
        let (h, df, del, dis) = (p.f("Health"), p.f("DamageFactor"), p.b("DeleteWhenDestroyed"), p.b("DisableCollisionWhenDestroyed"));
        let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
        println!("{class}: health {h} df {df} delete {del} disable {dis} state {} speeds {}/{}/{} yaws {}/{}/{}", x.state, x.open_speed, x.close_speed, x.fast_speed, x.yaw_closed, x.yaw_fwd, x.yaw_back);
        assert_eq!(x.health.replicated as f64, h.floor().clamp(0.0, 255.0));
        let c = CharView { id: 2, forward: [1.0, 0.0, 0.0], ..Default::default() };
        let (start, closed, speed) = (x.yaw, x.yaw_closed, x.close_speed);
        if x.state != 0 {
            w.interact(id, &c);
            let ev = run(&mut w, &q, &[], ((start - closed).abs() / speed) + 0.5);
            let frames = ev.iter().filter(|e| matches!(e, WorldEvent::ComponentYaw { actor, .. } if *actor == id)).count();
            let expect = ((start - closed).abs() / speed * 60.0).ceil() as i64;
            assert!((frames as i64 - expect).abs() <= 1, "{class}: {frames} vs {expect}");
            let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
            assert_eq!((x.state, x.yaw), (0, closed));
        }
        let hit = DamageInfo { causer: Some(3), attack_move: Some(0), ..Default::default() };
        let mut ev = vec![];
        if df > 0.0 {
            ev.extend(w.apply_damage(id, 10.0, &hit, &q, &[]));
            let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
            assert!((x.health.health - (h - 10.0 * df)).abs() < 1e-4, "{class} {}", x.health.health);
            for _ in 0..((h / df / 50.0).ceil() as usize + 2) {
                ev.extend(w.apply_damage(id, 50.0, &hit, &q, &[]));
            }
            assert!(ev.iter().any(|e| *e == WorldEvent::Died { actor: id }), "{class}");
            if del || dis {
                assert!(ev.contains(&WorldEvent::CollisionEnabled { actor: id, on: false }), "{class}");
            }
            if del {
                assert!(ev.contains(&WorldEvent::Hidden { actor: id }));
                let ev = run(&mut w, &q, &[], 1.1);
                assert!(ev.contains(&WorldEvent::Destroyed { actor: id }));
            }
        } else {
            assert!(w.apply_damage(id, 1000.0, &hit, &q, &[]).iter().all(|e| !matches!(e, WorldEvent::Died { .. })));
        }
    }
}

/// Destructibles on FL_Feitoria (archer covers, planks, fortifications, ...). Damage-state meshes follow
/// DamageMeshesHealth as health drops. The owning team cannot damage an owned one. Breaking it disables collision per
/// its flags. A mesh change detaches attached projectiles 0.01 s later. Repairing an owned repairable one scores 15
/// for the owner, and with PerformsUnstuckProcess a repair mesh change unsticks the characters inside
#[test]
fn feitoria_destructibles() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let mut w = World::from_level(&pk, &d);
    let mut q = Q::default();
    w.begin_play(&q);
    let mut classes: BTreeMap<String, ActorId> = BTreeMap::new();
    for a in &w.actors {
        if matches!(a.kind, Kind::Destructible(_)) {
            classes.entry(a.class.clone()).or_insert(a.id);
        }
    }
    println!("{:?}", classes.keys().collect::<Vec<_>>());
    assert!(!classes.is_empty());
    let mut checked_mesh = 0;
    for (class, id) in classes {
        let p = w.actors[id].props.clone();
        let hs: Vec<i64> = p.arr("DamageMeshesHealth").iter().filter_map(|v| v.as_i64()).collect();
        let owner = p.i("OwningTeam");
        println!("{class}: health {} df {} owner {owner} meshes {hs:?} repairable {} segs {} regen {} unstuck {}", p.f("Health"), p.f("DamageFactor"), p.b("Repairable"), p.i("RepairableHealthSegments"), p.b("Regenerating"), p.b("PerformsUnstuckProcess"));
        let df = p.f("DamageFactor");
        if df <= 0.0 {
            continue;
        }
        let enemy = if owner == 0 { 1 } else { 0 };
        if (0..2).contains(&owner) {
            assert!(w.apply_damage(id, 10.0, &player(owner as u8), &q, &[]).is_empty(), "{class}: own team damage");
        }
        let mut ev = vec![];
        q.chars.insert(id, vec![7]);
        for _ in 0..400 {
            let e = w.apply_damage(id, (1.0 / df) as f32, &player(enemy), &q, &[]);
            let Kind::Destructible(x) = &w.actors[id].kind else { unreachable!() };
            if e.iter().any(|e| matches!(e, WorldEvent::MeshSwapped { .. })) {
                let rep = x.replicated as i64;
                let best = hs.iter().enumerate().filter(|(_, h)| rep <= **h).map(|(i, _)| i).last().unwrap_or(0);
                assert_eq!(x.mesh_index, Some(best), "{class} rep {rep}");
                checked_mesh += 1;
            }
            let done = x.replicated == 0;
            ev.extend(e);
            if done {
                break;
            }
        }
        let Kind::Destructible(x) = &w.actors[id].kind else { unreachable!() };
        assert_eq!(x.replicated, 0, "{class}");
        assert!(ev.iter().any(|e| *e == WorldEvent::Died { actor: id }));
        if p.b("DeleteWhenDestroyed") || p.b("DisableCollisionWhenDestroyed") {
            assert!(!x.collision);
        }
        if ev.iter().any(|e| matches!(e, WorldEvent::MeshSwapped { .. })) {
            let t = run(&mut w, &q, &[], 0.05);
            assert!(t.contains(&WorldEvent::DetachAttached { actor: id }), "{class}");
        }
        if p.b("Repairable") && (0..2).contains(&owner) && !p.b("DeleteWhenDestroyed") {
            let ev = w.apply_damage(id, -20.0, &player(owner as u8), &q, &[]);
            assert!(ev.iter().any(|e| matches!(e, WorldEvent::Score { kind: 15, .. })), "{class}: {ev:?}");
            if p.b("PerformsUnstuckProcess") && ev.iter().any(|e| matches!(e, WorldEvent::MeshSwapped { .. })) {
                assert!(ev.contains(&WorldEvent::Unstuck { char: 7, actor: id }), "{class}");
            }
        }
    }
    println!("mesh checks {checked_mesh}");
    assert!(checked_mesh > 0);
}

/// FL_Dungeon's levers / wall cranks. Use toggles Value and SmoothedValue eases to it at RaiseSpeed (LowerSpeed back).
/// Every target's moving component reaches its target transform at 1 and its start again at 0. A slave crank toggles
/// its master, and the switch is not interactable for MinDelayBetweenUses
#[test]
fn dungeon_drivers() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "Dungeon/FL_Dungeon") else { return };
    let mut w = World::from_level(&pk, &d);
    let q = Q::default();
    w.begin_play(&q);
    // a lever without AutoInteractRaised / AutoInteractLowered (those toggle themselves back at rest, @3279..@3545)
    let auto = |a: &mh_world::WorldActor| a.props.b("AutoInteractRaised") || a.props.b("AutoInteractLowered");
    let id = w.actors.iter().find(|a| !auto(a) && matches!(&a.kind, Kind::ProgressDriver(d) if !d.targets.is_empty() && !d.slaves.is_empty())).unwrap().id;
    // and one that does: it reaches 1, then its Value flips back and it lowers
    if let Some(ad) = w.actors.iter().find(|a| a.props.b("AutoInteractRaised") && matches!(&a.kind, Kind::ProgressDriver(d) if !d.value)).map(|a| a.id) {
        let rs = w.actors[ad].props.f("RaiseSpeed");
        w.interact(ad, &CharView { id: 1, ..Default::default() });
        let mut peak: f64 = 0.0;
        for _ in 0..((1.0 / rs + 0.5) * 60.0) as usize {
            w.tick(1.0 / 60.0, &q, &[]);
            if let Kind::ProgressDriver(d) = &w.actors[ad].kind {
                peak = peak.max(d.smoothed);
            }
        }
        let Kind::ProgressDriver(d) = &w.actors[ad].kind else { unreachable!() };
        println!("auto-return {}: peak {peak} now {} value {}", w.actors[ad].name, d.smoothed, d.value);
        assert!(peak == 1.0 && !d.value && d.smoothed < 1.0);
    }
    let (targets, slaves) = match &w.actors[id].kind {
        Kind::ProgressDriver(d) => (d.targets.clone(), d.slaves.clone()),
        _ => unreachable!(),
    };
    let p = w.actors[id].props.clone();
    let (rs, ls, delay) = (p.f("RaiseSpeed"), p.f("LowerSpeed"), p.f("MinDelayBetweenUses"));
    println!("{} raise {rs} lower {ls} delay {delay} targets {} slaves {}", w.actors[id].name, targets.len(), slaves.len());
    let c = CharView { id: 1, ..Default::default() };
    assert!(w.can_interact(id, &c).0);
    w.interact(id, &c);
    if delay.abs() > 1e-6 {
        assert!(!w.can_interact(id, &c).0, "PreventInteraction");
    }
    let ev = run(&mut w, &q, &[], 1.0 / rs + 0.2);
    let Kind::ProgressDriver(dr) = &w.actors[id].kind else { unreachable!() };
    assert!(dr.value && dr.smoothed == 1.0, "{}", dr.smoothed);
    for t in &targets {
        let Kind::ProgressActor(pa) = &w.actors[*t].kind else { panic!("{}", w.actors[*t].class) };
        let tgt = pa.target.xf();
        let r = pa.raise_curve.as_ref().map(|c| c.eval(1.0)).unwrap_or(1.0);
        println!("  {} progress {} curve(1) {r}", w.actors[*t].class, pa.progress);
        if r == 1.0 {
            assert!(pa.rel.xf().max_diff(&tgt) < 1e-6, "{} {:?} vs {:?}", w.actors[*t].class, pa.rel, pa.target);
        }
        assert!(ev.iter().any(|e| matches!(e, WorldEvent::ComponentTransform { actor, .. } if actor == t)));
    }
    run(&mut w, &q, &[], delay + 0.1);
    assert!(w.can_interact(slaves[0], &c).0);
    w.interact(slaves[0], &c);
    run(&mut w, &q, &[], 1.0 / ls + 0.2);
    let Kind::ProgressDriver(dr) = &w.actors[id].kind else { unreachable!() };
    assert!(!dr.value && dr.smoothed == 0.0);
    for t in &targets {
        let Kind::ProgressActor(pa) = &w.actors[*t].kind else { unreachable!() };
        let r = pa.lower_curve.as_ref().map(|c| c.eval(0.0)).unwrap_or(0.0);
        if r == 0.0 {
            assert!(pa.rel.xf().max_diff(&pa.start.xf()) < 1e-6);
        }
    }
}

/// DIH_Feitoria's BP_FeitoriaDoorLocker: 1 s after BeginPlay its 19 doors are invulnerable (bCanBeDamaged false,
/// DamageFactor 0), not interactable, and at health 255
#[test]
fn feitoria_door_locker() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/DIH_Feitoria") else { return };
    let mut w = World::from_level(&pk, &d);
    let q = Q::default();
    w.begin_play(&q);
    let doors = w.actors.iter().find_map(|a| if let Kind::DoorLocker { doors, .. } = &a.kind { Some(doors.clone()) } else { None }).unwrap();
    assert_eq!(doors.len(), 19);
    let c = CharView { id: 1, ..Default::default() };
    assert!(w.can_interact(doors[0], &c).0);
    run(&mut w, &q, &[], 1.05);
    for d in &doors {
        assert!(!w.can_interact(*d, &c).0);
        let Kind::Door(x) = &w.actors[*d].kind else { unreachable!() };
        assert_eq!(x.health.health, 255.0);
        assert!(w.apply_damage(*d, 1000.0, &player(0), &q, &[]).is_empty());
    }
}

/// FL_Feitoria's BP_FrontlineKillObjective (5Capture, through its wrapper). Progress is (1 - Health / 100) x Weight.
/// Enemy players score FTrunc(ScorePerDamageMultiplier x Damage) per hit and ScorePerKill once at death; the owning
/// team scores nothing. Its death completes the wrapper and runs the capture point
#[test]
fn feitoria_kill_objective() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let mut w = World::from_level(&pk, &d);
    let q = Q::default();
    w.begin_play(&q);
    let k = w.actors.iter().find(|a| matches!(a.kind, Kind::KillObjective(_))).unwrap().id;
    let Kind::KillObjective(ko) = &w.actors[k].kind else { unreachable!() };
    let cp = ko.point.expect("point");
    let owner = w.cps[&cp].owning_team;
    let enemy = owner.map(|t| 1 - t).unwrap_or(0);
    let p = w.actors[k].props.clone();
    println!("{} point {} owner {owner:?} per-damage {} per-kill {}", w.actors[k].class, w.actors[cp].name, p.f("ScorePerDamageMultiplier"), p.i("ScorePerKill"));
    let (p0, done0, n) = w.objective_progress(cp).unwrap();
    let ev = w.kill_objective_damaged(k, 40.0, &player(enemy), 60, false);
    let per = (p.f("ScorePerDamageMultiplier") * 40.0).trunc() as i64;
    assert_eq!(ev.iter().any(|e| matches!(e, WorldEvent::Score { amount, .. } if *amount == per)), per > 0);
    let (p1, _, _) = w.objective_progress(cp).unwrap();
    assert!(p1 > p0, "{p0} -> {p1} of {n}");
    if let Some(o) = owner {
        assert!(!w.kill_objective_damaged(k, 10.0, &player(o), 50, false).iter().any(|e| matches!(e, WorldEvent::Score { .. })));
    }
    let ev = w.kill_objective_damaged(k, 60.0, &player(enemy), 0, true);
    let kill = p.i("ScorePerKill");
    assert_eq!(ev.iter().filter(|e| matches!(e, WorldEvent::Score { amount, .. } if *amount == kill)).count(), (kill > 0) as usize);
    assert_eq!(w.objective_progress(cp).unwrap().1, done0 + 1);
    assert!(ev.iter().any(|e| matches!(e, WorldEvent::ObjectivesChanged { capture_point } if *capture_point == cp)));
}

/// Server -> client replication on FL_Camp. The server pushes the wagon and closes, then breaks, a door; the client
/// applies the changed rep_vars every frame and follows: the leaf turns, the wagon rides its spline by the native
/// interpolation, the broken door's collision goes off. The client runs no server logic itself
#[test]
fn fl_camp_replication() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "DuelCamp/FL_Camp") else { return };
    let mut sv = World::from_level(&pk, &d);
    let mut cl = World::from_level(&pk, &d);
    cl.set_authority(false);
    let q0 = Q::default();
    sv.begin_play(&q0);
    cl.begin_play(&q0);
    let wagon = sv.actors.iter().find(|a| a.class == "BP_FrontlineWagonTorch_C").unwrap().id;
    let door = sv.actors.iter().find(|a| matches!(&a.kind, Kind::Door(x) if x.state != 0) && a.props.f("DamageFactor") > 0.0).unwrap().id;
    if let Some(cp) = match &sv.actors[wagon].kind {
        Kind::Objective(o) => o.capture_point,
        _ => None,
    } {
        sv.capture_point_prerequisites(cp, true);
    }
    let mut q = Q::default();
    q.chars.insert(wagon, vec![1]);
    let team = match &sv.actors[wagon].kind {
        Kind::Objective(o) => {
            if o.push.as_ref().unwrap().team1_curve.is_some() {
                0
            } else {
                1
            }
        }
        _ => 0,
    };
    let pusher = CharView { id: 1, team: Some(team), ..Default::default() };
    // both start from the same construction / BeginPlay state; only changes after it are sent
    let mut last: BTreeMap<(ActorId, &str), i64> = sv.rep_vars().into_iter().map(|(a, k, v)| ((a, k), v)).collect();
    sv.interact(door, &CharView { id: 2, forward: [1.0, 0.0, 0.0], ..Default::default() });
    let mut cl_ev = vec![];
    for f in 0..180 {
        sv.tick(1.0 / 60.0, &q, &[pusher.clone()]);
        if f == 120 {
            for _ in 0..200 {
                sv.apply_damage(door, 50.0, &DamageInfo { attack_move: Some(0), ..Default::default() }, &q0, &[]);
            }
        }
        for (a, k, v) in sv.rep_vars() {
            if last.get(&(a, k)) != Some(&v) {
                last.insert((a, k), v);
                cl_ev.extend(cl.apply_rep(a, k, v, &q0));
            }
        }
        cl_ev.extend(cl.tick(1.0 / 60.0, &Q::default(), &[]));
    }
    assert!(!cl_ev.iter().any(|e| matches!(e, WorldEvent::Spawn { .. } | WorldEvent::SpawnEquipment { .. } | WorldEvent::Score { .. } | WorldEvent::CaptureProgress { .. })));
    let (Kind::Objective(so), Kind::Objective(co)) = (&sv.actors[wagon].kind, &cl.actors[wagon].kind) else { unreachable!() };
    let (sp, cp) = (so.push.as_ref().unwrap().progress, co.push.as_ref().unwrap().progress);
    println!("wagon server {sp:.5} client {cp:.5}");
    assert!(sp > 0.0 && (sp - cp).abs() < 0.002, "{sp} vs {cp}");
    assert!(cl_ev.iter().any(|e| matches!(e, WorldEvent::ActorTransform { actor, .. } if *actor == wagon)));
    let (Kind::Door(sd), Kind::Door(cd)) = (&sv.actors[door].kind, &cl.actors[door].kind) else { unreachable!() };
    assert_eq!((sd.state, sd.health.replicated), (cd.state, cd.health.replicated));
    assert!((sd.yaw - cd.yaw).abs() < 1e-6, "{} vs {}", sd.yaw, cd.yaw);
    assert_eq!(sd.health.collision, cd.health.collision);
}

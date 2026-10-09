//! mh-world on real map data (FL_Feitoria, Camp, MountainPeak). SKIP (pass) without the install.

use mh_level::{read, LevelData, Pkgs};
use mh_pak::{Reader, Vfs};
use mh_world::interaction::{sweeps, SweepHit};
use mh_world::{ActorId, CharId, CharView, DamageInfo, Kind, Queries, SpawnedStatus, World, WorldEvent};
use std::cell::RefCell;
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
    spawned: RefCell<BTreeMap<ActorId, SpawnedStatus>>,
}
impl Queries for Q {
    fn overlapping_chars(&self, a: ActorId) -> Vec<CharId> {
        self.chars.get(&a).cloned().unwrap_or_default()
    }
    fn overlapping_actors(&self, a: ActorId) -> Vec<ActorId> {
        self.actors.get(&a).cloned().unwrap_or_default()
    }
    fn spawned_status(&self, a: ActorId) -> SpawnedStatus {
        self.spawned.borrow().get(&a).copied().unwrap_or_default()
    }
}

fn find(w: &World, f: impl Fn(&mh_world::WorldActor) -> bool) -> ActorId {
    w.actors.iter().find(|a| f(a)).map(|a| a.id).expect("actor")
}
fn fwd(w: &World, a: ActorId) -> [f64; 3] {
    let m = &w.actors[a].xf.m;
    let v = [m[0][0], m[1][0], m[2][0]];
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / l)
}
fn run(w: &mut World, q: &Q, chars: &[CharView], secs: f64) -> Vec<WorldEvent> {
    let mut ev = vec![];
    for _ in 0..(secs * 60.0).round() as usize {
        ev.extend(w.tick(1.0 / 60.0, q, chars));
    }
    ev
}

/// Behaviours are attached by Blueprint chain, and the Frontline capture points' objectives are linked
#[test]
fn feitoria_world() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let w = World::from_level(&pk, &d);
    let mut n: BTreeMap<&str, usize> = BTreeMap::new();
    for a in &w.actors {
        let k = match &a.kind {
            Kind::Door(_) => "door",
            Kind::Destructible(_) => "destructible",
            Kind::Objective(_) => "objective",
            Kind::Ladder(_) => "ladder",
            Kind::EquipmentSpawner(_) => "equipment",
            Kind::VehicleSpawner(_) => "vehicle",
            Kind::Pushable(_) => "pushable",
            Kind::Inert => "inert",
            _ => "other",
        };
        *n.entry(k).or_default() += 1;
    }
    println!("{n:?}");
    assert_eq!(n["door"], 46);
    assert_eq!(n["ladder"], 48);
    assert_eq!(n["objective"], 13);
    assert_eq!(n["vehicle"], 8);
    assert_eq!(n["equipment"], 4);
    let by_name = |s: &str| w.actors.iter().find(|a| a.name == s).unwrap().id;
    assert_eq!(w.objectives[&by_name("1Capture")], vec![by_name("BP_DestroyableCogDeliverySpot_2")]);
    assert_eq!(w.objectives[&by_name("BP_HiddenCapturePointBlueSide")].len(), 9);
    // fresh objectives: no progress; the kill wrapper's KillObjective is placed (not completed)
    for (cp, os) in &w.objectives {
        let (p, done, total) = w.objective_progress(*cp).unwrap();
        println!("{}: progress {p} done {done}/{total} ({} objectives)", w.actors[*cp].name, os.len());
        assert_eq!((p, done), (0.0, 0));
    }
}

/// BP_Door: interact opens away from the character, the leaf turns at DoorOpenSpeed to the target, interact again
/// closes; a kick from the far side fast-opens it forward without damage; other damage goes to the destructible
#[test]
fn camp_door() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "DuelCamp/Camp") else { return };
    let mut w = World::from_level(&pk, &d);
    w.begin_play(&Q::default());
    // every Camp door starts open (StartingState 1 / 2 -> DoorState 1 / 3); interacting with an open door closes it
    let id = find(&w, |a| a.class == "BP_DestroyableWoodenDoor_C");
    {
        let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
        assert!(x.state == 1 || x.state == 3);
        assert_eq!(x.yaw, if x.state == 1 { x.yaw_fwd } else { x.yaw_back });
    }
    let c0 = CharView { id: 9, forward: [1.0, 0.0, 0.0], ..Default::default() };
    w.interact(id, &c0);
    run(&mut w, &Q::default(), &[], 3.0);
    let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
    assert_eq!((x.state, x.last_completed, x.yaw), (0, 0, x.yaw_closed));
    let q = Q::default();
    let f = fwd(&w, id);
    let (closed, back, fwd_yaw, open_speed, fast) = match &w.actors[id].kind {
        Kind::Door(x) => (x.yaw_closed, x.yaw_back, x.yaw_fwd, x.open_speed, x.fast_speed),
        _ => unreachable!(),
    };
    let same = CharView { id: 1, forward: f, allow_vehicles: true, ..Default::default() };
    assert_eq!(w.can_interact(id, &same), (true, false));
    // facing along the door's forward: angle 0 -> opens backward (state 3)
    w.interact(id, &same);
    let ev = run(&mut w, &q, &[], 2.0);
    let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
    assert_eq!((x.state, x.last_completed), (3, 3));
    assert!((x.yaw - back).abs() < 1e-9 && !x.tick_enabled);
    // time to open: |back - closed| / DoorOpenSpeed
    let moving = ev.iter().filter(|e| matches!(e, WorldEvent::ComponentYaw { .. })).count();
    let expect = ((back - closed).abs() / open_speed * 60.0).ceil() as usize;
    assert!((moving as i64 - expect as i64).abs() <= 1, "{moving} frames vs {expect}");
    // interact again: closes
    w.interact(id, &same);
    run(&mut w, &q, &[], 2.0);
    let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
    assert_eq!(x.state, 0);
    assert!((x.yaw - closed).abs() < 1e-9);
    // kick from the other side (facing against the door's forward, angle 180 >= 120): fast forward open (2)
    let hp0 = if let Kind::Door(x) = &w.actors[id].kind { x.health.health } else { 0.0 };
    let kick = DamageInfo { causer: Some(2), attack_move: Some(4), causer_forward: Some(f.map(|c| -c)), ..Default::default() };
    w.apply_damage(id, 25.0, &kick, &Q::default(), &[]);
    let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
    assert_eq!((x.state, x.speed, x.kicker), (2, fast, Some(2)));
    assert_eq!(x.health.health, hp0);
    run(&mut w, &q, &[], 1.0);
    let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
    assert!((x.yaw - fwd_yaw).abs() < 1e-9);
    // a slash: health -= damage * DamageFactor; enough of them break it (OnDeath)
    let hit = DamageInfo { causer: Some(2), attack_move: Some(0), ..Default::default() };
    w.apply_damage(id, 10.0, &hit, &Q::default(), &[]);
    let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
    assert!((x.health.health - (hp0 - 10.0 * w.actors[id].props.f("DamageFactor"))).abs() < 1e-6, "{}", x.health.health);
    let mut died = false;
    for _ in 0..400 {
        died |= w.apply_damage(id, 10.0, &hit, &Q::default(), &[]).iter().any(|e| matches!(e, WorldEvent::Died { .. }));
    }
    let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
    assert!(died && x.health.replicated == 0);
}

/// A closing leaf pushes characters it overlaps away; an opening one bounces back closed
#[test]
fn door_blocked() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "DuelCamp/Camp") else { return };
    let mut w = World::from_level(&pk, &d);
    w.begin_play(&Q::default());
    let id = find(&w, |a| a.class == "BP_DestroyableWoodenDoor_C");
    let f = fwd(&w, id);
    let c = CharView { id: 7, forward: f, location: w.actors[id].xf.translation(), ..Default::default() };
    let mut q = Q::default();
    w.interact(id, &c);
    run(&mut w, &q, &[], 0.1);
    q.chars.insert(id, vec![7]);
    run(&mut w, &q, &[c.clone()], 1.0 / 60.0);
    let Kind::Door(x) = &w.actors[id].kind else { unreachable!() };
    assert_eq!(x.state, 0, "opening into a character bounces closed (@1304)");
    let ev = run(&mut w, &q, &[c.clone()], 1.0 / 60.0);
    assert!(ev.iter().any(|e| matches!(e, WorldEvent::Knockback { char: 7, .. })), "{ev:?}");
}

/// BP_FrontlineDestroyable (BP_KillablePeasant): only damageable once the enemy has the prerequisites, the capture
/// point's owning team cannot damage it, a hit is capped at MaxDamagePerHit / DamageFactor, health scores 12
#[test]
fn feitoria_frontline() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let mut w = World::from_level(&pk, &d);
    let cp = w.actors.iter().find(|a| a.name == "BP_HiddenCapturePointBlueSide").unwrap().id;
    let id = w.objectives[&cp][0];
    let owner = w.cps[&cp].owning_team.expect("owning team");
    let enemy = 1 - owner;
    let p = w.actors[id].props.clone();
    println!("{} health {} df {} max/hit {} weight {}", w.actors[id].class, p.f("Health"), p.f("DamageFactor"), p.f("MaxDamagePerHit"), p.f("ObjectiveWeight"));
    let hit = |t: u8| DamageInfo { causer: Some(3), instigator: Some(3), instigator_player: true, instigator_team: Some(t), attack_move: Some(0), ..Default::default() };
    assert!(w.apply_damage(id, 30.0, &hit(enemy), &Q::default(), &[]).is_empty(), "no prerequisites yet");
    // the capture point hands the gained prerequisites to its objectives
    w.capture_point_prerequisites(cp, true);
    assert!(w.apply_damage(id, 30.0, &hit(owner), &Q::default(), &[]).is_empty(), "owning team");
    let none = DamageInfo { causer: Some(3), attack_move: Some(0), ..Default::default() };
    assert!(w.apply_damage(id, 30.0, &none, &Q::default(), &[]).is_empty(), "no instigator / player state (@2520)");
    let ev = w.apply_damage(id, 30.0, &hit(enemy), &Q::default(), &[]);
    let Kind::Objective(o) = &w.actors[id].kind else { unreachable!() };
    let df = p.f("DamageFactor");
    let lost = (30.0f64.min(p.f("MaxDamagePerHit") / df) * df).min(p.f("Health"));
    assert!((o.base.health - (p.f("Health") - lost)).abs() < 1e-6, "{}", o.base.health);
    assert!(ev.contains(&WorldEvent::ObjectivesChanged { capture_point: cp }), "{ev:?}");
    let pts = (lost.ceil() * p.f("ScoreDamageMultiplier")).trunc() as i64;
    println!("score multiplier {} -> {pts}; per destroy {}", p.f("ScoreDamageMultiplier"), p.i("ScoreAwardedPerDestroy"));
    assert_eq!(ev.iter().any(|e| matches!(e, WorldEvent::Score { kind: 12, char: Some(3), .. })), pts > 0);
    // BP_CapturePoint:ObjectivesChanged on a hidden point: SetCaptureProgress(ObjectiveProgress, enemy) (@1790)
    let (prog, _, n) = w.objective_progress(cp).unwrap();
    assert!(prog > 0.0 && (w.cps[&cp].objective_progress - prog).abs() < 1e-12, "{prog} of {n}");
    assert!(ev.contains(&WorldEvent::CaptureProgress { capture_point: cp, progress: prog, team: Some(enemy) }), "{ev:?}");
    // an AI instigator (player state, no MordhauPlayerController) damages but scores nothing
    let ai = DamageInfo { instigator_player: false, ..hit(enemy) };
    let ev = w.apply_damage(id, 1.0, &ai, &Q::default(), &[]);
    assert!(!ev.iter().any(|e| matches!(e, WorldEvent::Score { .. })), "{ev:?}");
    // kill it
    let mut destroy_score = false;
    for _ in 0..100 {
        destroy_score |= w.apply_damage(id, 1000.0, &hit(enemy), &Q::default(), &[]).iter().any(|e| matches!(e, WorldEvent::Score { amount, .. } if *amount == p.i("ScoreAwardedPerDestroy")));
    }
    assert_eq!(destroy_score, p.i("ScoreAwardedPerDestroy") > 0);
    let Kind::Objective(o) = &w.actors[id].kind else { unreachable!() };
    assert!(o.completed());
    assert_eq!(w.objective_progress(cp).unwrap().1, 1);
}

/// BP_CapturePoint objective layer on all Feitoria points: begin play sets each normal point's capture progress to
/// 1 - ObjectiveProgress for its owner; destroying every objective of a point completes it: a normal point drops to
/// 0.005 and, after ObjectiveWinDelay with the Frontline match in progress, zeroes its owner's score (TriggerWinDelayed)
#[test]
fn feitoria_capture_points() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let mut w = World::from_level(&pk, &d);
    w.fl_match_in_progress = true;
    let q = Q::default();
    let ev = w.begin_play(&q);
    for (cp, c) in &w.cps {
        let a = &w.actors[*cp];
        println!("{} hidden {} push {} owner {:?} win delay {} objectives {} spots {}", a.name, c.hidden, c.push, c.owning_team, c.win_delay, c.objectives.len(), c.delivery_spots.len());
        if !c.hidden && !c.push {
            assert!(ev.contains(&WorldEvent::CaptureProgress { capture_point: *cp, progress: 1.0 - c.objective_progress, team: c.owning_team }), "{ev:?}");
        }
    }
    // a normal (non-hidden) point whose objectives are all destroyables
    let pick = w.cps.iter().find(|(_, c)| !c.hidden && !c.push && c.objectives.iter().all(|o| matches!(o, Some(o) if matches!(&w.actors[*o].kind, Kind::Objective(x) if x.kind == mh_world::frontline::ObjectiveKind::Destroyable))));
    let Some((&cp, _)) = pick else { return println!("no all-destroyable normal point") };
    let owner = w.cps[&cp].owning_team;
    let enemy = owner.map(|t| 1 - t);
    w.capture_point_prerequisites(cp, true);
    let hit = DamageInfo { instigator: Some(1), instigator_player: true, instigator_team: enemy, ..Default::default() };
    let mut ev = vec![];
    for o in w.objectives[&cp].clone() {
        for _ in 0..200 {
            ev.extend(w.apply_damage(o, 1000.0, &hit, &Q::default(), &[]));
        }
    }
    assert!(w.cps[&cp].objectives_completed);
    assert!(ev.contains(&WorldEvent::ObjectivesCompleted { capture_point: cp }));
    assert!(ev.contains(&WorldEvent::CaptureProgress { capture_point: cp, progress: 0.004999999888241291, team: owner }), "{ev:?}");
    let delay = w.cps[&cp].win_delay;
    let ev = run(&mut w, &q, &[], delay + 0.1);
    if let Some(t) = owner {
        assert!(ev.contains(&WorldEvent::TeamScore { team: t, score: 0.0 }), "{ev:?}");
    }
}

/// BP_ItemDeliverySpot (the cog spot of 1Capture): inactive until the prerequisites, only its Type counts,
/// Progress = Deliverables / RequiredDeliveries, disabled when complete
#[test]
fn feitoria_delivery() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let mut w = World::from_level(&pk, &d);
    let id = w.actors.iter().find(|a| a.name == "BP_DestroyableCogDeliverySpot_2").unwrap().id;
    let p = w.actors[id].props.clone();
    let (req, ty) = (p.i("RequiredDeliveries"), p.i("Type"));
    println!("cog spot: required {req} type {ty} score {}", p.i("ScoreAwardPerDelivery"));
    assert!(req > 0);
    assert!(!w.deliver(id, ty, Some(5), Some(0)).0, "inactive");
    w.set_prerequisites(id, true);
    assert!(!w.deliver(id, ty + 1, Some(5), Some(0)).0, "wrong type");
    for i in 1..=req {
        let (ok, ev) = w.deliver(id, ty, Some(5), Some(0));
        assert!(ok);
        assert!(ev.iter().any(|e| matches!(e, WorldEvent::ObjectivesChanged { .. })));
        let Kind::Objective(o) = &w.actors[id].kind else { unreachable!() };
        assert!((o.progress - i as f64 / req as f64).abs() < 1e-12);
    }
    assert!(!w.deliver(id, ty, Some(5), Some(0)).0, "complete: disabled");
    let Kind::Objective(o) = &w.actors[id].kind else { unreachable!() };
    assert!(o.completed());
}

/// BP_Ladder: climbable from the front, the mount hands a BP_LadderMover_C on the climb line to the character;
/// a held use drops it over DropDuration, tripping and damaging characters under it once
#[test]
fn feitoria_ladder() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let mut w = World::from_level(&pk, &d);
    let id = find(&w, |a| a.class == "BP_Ladder_9m_C" && matches!(&a.kind, Kind::Ladder(l) if l.state == 0));
    let info = w.ladder(id).unwrap();
    let xf = w.actors[id].xf;
    // in front of the ladder (ladder-local X -100 at the bottom)
    let front = xf.apply([-100.0, 0.0, 100.0]);
    let ch = CharView { id: 4, location: front, allow_vehicles: true, ..Default::default() };
    assert!(w.can_interact(id, &ch).0);
    let behind = CharView { location: xf.apply([100.0, 0.0, 100.0]), ..ch.clone() };
    assert!(!w.can_interact(id, &behind).0, "behind and below the top");
    let ev = w.interact(id, &ch);
    let Some(WorldEvent::LadderMount { mover_xf, start, end, mover_class, info: ci, .. }) = ev.first() else { panic!("{ev:?}") };
    assert_eq!(*mover_class, "BP_LadderMover_C");
    assert_eq!((*start, *end), (info.start, info.end));
    // the mover sits at mh-character's ClosestPointOnLine (segment) of the character on LadderStart..LadderEnd
    let m = mover_xf.translation();
    let cp = mh_character::exe_ladder::closest_point_on_line(ci.start, ci.end, mh_character::uemath::v(front[0] as f32, front[1] as f32, front[2] as f32));
    assert_eq!([m[0] as f32, m[1] as f32, m[2] as f32], [cp.x, cp.y, cp.z]);
    assert!((ci.start.z as f64 - start[2]).abs() < 1e-3 && (ci.exit.x as f64 - info.exit[0]).abs() < 1e-3);
    // drop (if this one can)
    let can = matches!(&w.actors[id].kind, Kind::Ladder(l) if l.can_drop_and_raise);
    println!("ladder {} can drop {can}, info {info:?}", w.actors[id].name);
    if can {
        w.held_interact(id, &ch);
        assert!(!w.can_interact(id, &ch).0, "animating");
        let mut q = Q::default();
        q.chars.insert(id, vec![9]);
        let under = CharView { id: 9, location: xf.apply([-150.0, 0.0, 0.0]), ..Default::default() };
        let ev = run(&mut w, &q, &[under], 3.5);
        assert_eq!(ev.iter().filter(|e| matches!(e, WorldEvent::Trip { char: 9 })).count(), 1);
        assert!(ev.contains(&WorldEvent::DamageCharacter { char: 9, amount: 10.0 }));
        let Kind::Ladder(l) = &w.actors[id].kind else { unreachable!() };
        assert_eq!((l.state, l.animating, l.interactable), (1, false, true));
    }
}

/// BP_EquipmentSpawner: first spawn 0.5 s after BeginPlay, checked every 1 s; picked up -> respawn after
/// RespawnInterval
#[test]
fn camp_equipment_spawner() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "DuelCamp/Camp") else { return };
    let mut w = World::from_level(&pk, &d);
    let id = find(&w, |a| matches!(a.kind, Kind::EquipmentSpawner(_)));
    let ri = w.actors[id].props.f("RespawnInterval");
    println!("{} equipment {} respawn {ri}", w.actors[id].class, w.actors[id].props.obj("Equipment"));
    let q = Q::default();
    w.begin_play(&Q::default());
    let spawns = |ev: &[WorldEvent]| ev.iter().filter(|e| matches!(e, WorldEvent::SpawnEquipment { by, .. } if *by == id)).count();
    assert_eq!(spawns(&run(&mut w, &q, &[], 0.45)), 0);
    assert_eq!(spawns(&run(&mut w, &q, &[], 0.1)), 1);
    q.spawned.borrow_mut().insert(id, SpawnedStatus { exists: true, held: false, distance: 0.0, dead: false });
    assert_eq!(spawns(&run(&mut w, &q, &[], 5.0)), 0, "still on the rack");
    q.spawned.borrow_mut().insert(id, SpawnedStatus { exists: true, held: true, distance: 0.0, dead: false });
    assert_eq!(spawns(&run(&mut w, &q, &[], ri - 0.5)), 0);
    assert_eq!(spawns(&run(&mut w, &q, &[], 2.0)), 1);
}

/// Vehicle spawners spawn at BeginPlay (TrySpawnVehicle from ReceiveBeginPlay): a ballista at its Capsule, a horse of
/// one of its allowed classes under the map, teleported onto the spawner within 0.1 s; a dead vehicle respawns after
/// the game mode's respawn time (AMordhauGameMode ctor 30 s) once its OnCharacterDied was bound; a failed horse
/// teleport destroys it and retries after 1..3 s
#[test]
fn mountainpeak_vehicle_spawners() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "MaxMap/SKM_MountainPeak_64") else { return };
    let mut w = World::from_level(&pk, &d);
    w.game_mode_respawn = Some((mh_world::spawner::DEFAULT_VEHICLE_RESPAWN, mh_world::spawner::DEFAULT_VEHICLE_RESPAWN, mh_world::spawner::DEFAULT_VEHICLE_RESPAWN));
    let q = Q::default();
    let ev = w.begin_play(&q);
    let spawned: Vec<&WorldEvent> = ev.iter().filter(|e| matches!(e, WorldEvent::Spawn { .. })).collect();
    let n = w.actors.iter().filter(|a| matches!(&a.kind, Kind::VehicleSpawner(s) if s.kind != mh_world::spawner::VehicleKind::Other && s.active)).count();
    println!("{spawned:?}");
    assert_eq!(spawned.len(), n);
    let id = find(&w, |a| a.class == "BP_HorseSpawner_C");
    let Some(WorldEvent::Spawn { class, xf, .. }) = ev.iter().find(|e| matches!(e, WorldEvent::Spawn { by, .. } if *by == id)) else { panic!() };
    let Kind::VehicleSpawner(s) = &w.actors[id].kind else { unreachable!() };
    assert!(s.horses.contains(class) && xf.translation() == [0.0, 0.0, -3000.0], "{class} {:?}", s.horses);
    let ev = run(&mut w, &q, &[], 0.1);
    assert!(ev.contains(&WorldEvent::TeleportSpawned { by: id, xf: w.actors[id].xf }), "{ev:?}");
    w.spawn_teleport_result(id, true);
    q.spawned.borrow_mut().insert(id, SpawnedStatus { dead: true, ..Default::default() });
    run(&mut w, &q, &[], 1.0);
    q.spawned.borrow_mut().insert(id, SpawnedStatus::default());
    let ev = run(&mut w, &q, &[], 28.0);
    assert!(!ev.iter().any(|e| matches!(e, WorldEvent::Spawn { by, .. } if *by == id)));
    let ev = run(&mut w, &q, &[], 3.0);
    assert!(ev.iter().any(|e| matches!(e, WorldEvent::Spawn { by, .. } if *by == id)));
    // a failed teleport: destroyed within 0.5 s, another spawn within 3 s more
    run(&mut w, &q, &[], 0.1);
    w.spawn_teleport_result(id, false);
    let ev = run(&mut w, &q, &[], 3.6);
    assert!(ev.contains(&WorldEvent::DestroySpawned { by: id }));
    assert!(ev.iter().any(|e| matches!(e, WorldEvent::Spawn { by, .. } if *by == id)), "{ev:?}");
}

/// The use-key target: 8 sweeps of 180 cm; of two hit doors the one nearer the view line wins; out of reach (XY
/// > 130) or not interactable is skipped. And collision bodies map back to their gameplay actors
#[test]
fn camp_interaction_and_bodies() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "DuelCamp/Camp") else { return };
    let w = World::from_level(&pk, &d);
    let eye = [0.0, 0.0, 160.0];
    let sw = sweeps(eye, [1.0, 0.0, 0.0], [1.0; 3]);
    assert_eq!(sw.len(), 8);
    assert!((sw[0].end[0] - 180.0).abs() < 1e-9);
    for s in &sw[1..] {
        let o = [s.start[0] - eye[0], s.start[1] - eye[1], s.start[2] - eye[2]];
        assert!(((o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt() - 50.0).abs() < 1e-9 && o[0].abs() < 1e-9);
    }
    let doors: Vec<ActorId> = w.actors.iter().filter(|a| matches!(a.kind, Kind::Door(_))).map(|a| a.id).take(2).collect();
    let ch = CharView { id: 1, location: [0.0, 0.0, 90.0], forward: [1.0, 0.0, 0.0], ..Default::default() };
    let hits = [
        SweepHit { sweep: 3, actor: doors[0], point: [100.0, 40.0, 160.0] },
        SweepHit { sweep: 0, actor: doors[1], point: [110.0, 5.0, 160.0] },
    ];
    assert_eq!(w.interaction_target(&ch, eye, [1.0, 0.0, 0.0], &hits), Some(doors[1]));
    let far = [SweepHit { sweep: 0, actor: doors[0], point: [140.0, 0.0, 160.0] }];
    assert_eq!(w.interaction_target(&ch, eye, [1.0, 0.0, 0.0], &far), None);
    let inert = w.actors.iter().find(|a| matches!(a.kind, Kind::Inert)).unwrap().id;
    assert_eq!(w.interaction_target(&ch, eye, [1.0, 0.0, 0.0], &[SweepHit { sweep: 0, actor: inert, point: [50.0, 0.0, 160.0] }]), None);
    // bodies -> actors
    let cw = mh_level::collision::CollisionWorld::build(&pk, &d, None);
    let ba = cw.body_actors(&d);
    let mut per: BTreeMap<String, usize> = BTreeMap::new();
    for a in ba.iter().flatten() {
        *per.entry(d.gameplay[*a].class.clone()).or_default() += 1;
    }
    println!("bodies with an actor: {} of {}: {per:?}", ba.iter().flatten().count(), ba.len());
    let door_bodies = ba.iter().enumerate().filter(|(_, a)| a.map(|a| matches!(w.actors[a].kind, Kind::Door(_))).unwrap_or(false)).count();
    assert!(door_bodies >= 9, "{door_bodies}");
    let (b, a) = ba.iter().enumerate().find_map(|(b, a)| a.map(|a| (b, a))).unwrap();
    assert_eq!(cw.actor_of(b as u32, &d), Some(a));
}

/// The ladder mount wired to mh-character's ladder mover on real Feitoria data: the LadderMount event's info builds
/// ExeMovement::new_ladder_mover, which climbs (forward held) on the map's CollisionWorld to the top step, where the
/// exit is the ladder's LadderExit
#[test]
fn feitoria_ladder_climb() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "FeitoriaMap/FL_Feitoria") else { return };
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Ok(rec) = std::fs::read_to_string(dir.join("../../tests/golden/character/records.json")) else { return eprintln!("SKIP: no records") };
    use mh_character::CharacterSource;
    let base = mh_character::RecordsJson(&rec).load().unwrap();
    let mut w = World::from_level(&pk, &d);
    let cw = mh_level::collision::CollisionWorld::build(&pk, &d, None);
    let id = find(&w, |a| a.class == "BP_Ladder_9m_C" && matches!(&a.kind, Kind::Ladder(l) if l.state == 0));
    let xf = w.actors[id].xf;
    let ch = CharView { id: 4, location: xf.apply([-100.0, 0.0, 100.0]), allow_vehicles: true, ..Default::default() };
    let ev = w.interact(id, &ch);
    let Some(WorldEvent::LadderMount { info, .. }) = ev.first() else { panic!("{ev:?}") };
    let loc = mh_character::uemath::v(ch.location[0] as f32, ch.location[1] as f32, ch.location[2] as f32);
    let mut m = mh_character::ExeMovement::new_ladder_mover(&base, *info, loc);
    m.ladder.as_mut().unwrap().has_driver = true;
    m.world_time = 10.0;
    let total = m.ladder.as_ref().unwrap().total_steps;
    for _ in 0..(60 * 10) {
        m.ladder_frame(&cw, 1.0 / 60.0, 1.0);
    }
    let l = m.ladder.as_ref().unwrap();
    println!("steps {} / {total}, at {:?}, end {:?}", l.current_step, m.location, info.end);
    assert_eq!(l.current_step, total);
    let exit = m.ladder_exit(&cw, m.location, 30.0, 88.0);
    let e = info.exit;
    assert!(((exit.x - e.x).powi(2) + (exit.y - e.y).powi(2)).sqrt() < 60.0 && (exit.z - e.z).abs() < 120.0, "{exit:?} vs {e:?}");
}

/// BP_FrontlinePushable on FL_Camp (the wagons, native APushableActor on a TargetSplineActor spline): begin play puts
/// it on its spline; with the prerequisites one pusher of the curve's team moves it at curve(1) progress / s along the
/// spline at constant velocity, carrying an overlapping character with it; each move re-runs the capture point's
/// ObjectivesChanged (capture progress); an enemy alone pulls nothing (bIsPullingAllowed off)
#[test]
fn fl_camp_pushable() {
    let Some(pk) = pkgs() else { return };
    let Some(d) = map(&pk, "DuelCamp/FL_Camp") else { return };
    let mut w = World::from_level(&pk, &d);
    let q0 = Q::default();
    let ev = w.begin_play(&q0);
    let id = find(&w, |a| a.class == "BP_FrontlineWagonTorch_C");
    let cp = match &w.actors[id].kind {
        Kind::Objective(o) => o.capture_point,
        _ => panic!("not an objective"),
    };
    let (team, speed, len, pull) = {
        let Kind::Objective(o) = &w.actors[id].kind else { unreachable!() };
        let p = o.push.as_ref().unwrap();
        let (t, c) = if let Some(c) = &p.team1_curve { (0u8, c) } else { (1u8, p.team2_curve.as_ref().unwrap()) };
        (t, c.eval(1.0), p.spline.as_ref().unwrap().length(), p.pulling_allowed)
    };
    println!("wagon cp {:?} team {team} speed {speed}/s spline {len:.0} cm pull {pull}", cp.map(|c| &w.actors[c].name));
    let Some(WorldEvent::ActorTransform { xf: x0, .. }) = ev.iter().find(|e| matches!(e, WorldEvent::ActorTransform { actor, .. } if *actor == id)).cloned() else { panic!("not placed") };
    // ReceiveBeginPlay snaps it to GetTransformAlongSplineOffset at progress 0: the spline start (on FL_Camp the
    // placed wagon sits ~4.9 m off its spline; the game moves it there at BeginPlay)
    {
        let Kind::Objective(o) = &w.actors[id].kind else { unreachable!() };
        let sp = o.push.as_ref().unwrap().spline.as_ref().unwrap();
        let s0 = sp.location_at_distance(0.0);
        assert!(x0.translation().iter().zip(s0).all(|(a, b)| (a - b).abs() < 1e-6), "{:?} vs {s0:?}", x0.translation());
        println!("placed {:?} -> spline start {s0:?}", w.actors[id].xf.translation());
    }
    if let Some(cp) = cp {
        w.capture_point_prerequisites(cp, true);
    } else {
        w.set_prerequisites(id, true);
    }
    let mut q = Q::default();
    q.chars.insert(id, vec![1]);
    let pusher = CharView { id: 1, team: Some(team), has_player_controller: true, ..Default::default() };
    let ev = run(&mut w, &q, &[pusher.clone()], 2.0);
    let Kind::Objective(o) = &w.actors[id].kind else { unreachable!() };
    let prog = o.push.as_ref().unwrap().progress;
    assert!((prog - speed * 2.0).abs() < speed / 30.0, "{prog} vs {}", speed * 2.0);
    let x1 = o.push.as_ref().unwrap().xf.unwrap();
    let moved = {
        let (a, b) = (x0.translation(), x1.translation());
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    println!("progress {prog:.4} moved {moved:.1} cm (arc {:.1})", prog * len);
    assert!(moved <= prog * len + 1.0 && moved > prog * len * 0.8);
    let carried: f64 = ev.iter().filter_map(|e| if let WorldEvent::MoveCharacter { char: 1, offset } = e { Some((offset[0].powi(2) + offset[1].powi(2) + offset[2].powi(2)).sqrt()) } else { None }).sum();
    assert!((carried - prog * len).abs() < prog * len * 0.2, "carried {carried}");
    if let Some(cp) = cp {
        assert!(ev.iter().any(|e| matches!(e, WorldEvent::CaptureProgress { capture_point, .. } if *capture_point == cp)), "{:?}", &ev[..ev.len().min(8)]);
        assert!(w.cps[&cp].objective_progress > 0.0);
    }
    // the other team alone: no pulling
    let enemy = CharView { id: 1, team: Some(1 - team), ..Default::default() };
    run(&mut w, &q, &[enemy], 1.0);
    let Kind::Objective(o) = &w.actors[id].kind else { unreachable!() };
    if !pull {
        assert_eq!(o.push.as_ref().unwrap().progress, prog);
    }
}

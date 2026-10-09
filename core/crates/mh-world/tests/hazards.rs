//! mh-world r3 hazards, pickups and the cauldron on real map data (the first map of a list that places each class).
//! SKIP (pass) without the install.

use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use mh_world::hazards::TriggerKind;
use mh_world::pickups::Pickup;
use mh_world::{ActorId, CharId, CharView, DamageInfo, Kind, Queries, SpawnedStatus, World, WorldEvent};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Default)]
struct Q {
    comp: RefCell<BTreeMap<(ActorId, String), Vec<CharId>>>,
    actors: RefCell<BTreeMap<ActorId, Vec<ActorId>>>,
}
impl Queries for Q {
    fn overlapping_chars(&self, _: ActorId) -> Vec<CharId> {
        vec![]
    }
    fn overlapping_actors(&self, a: ActorId) -> Vec<ActorId> {
        self.actors.borrow().get(&a).cloned().unwrap_or_default()
    }
    fn spawned_status(&self, _: ActorId) -> SpawnedStatus {
        SpawnedStatus::default()
    }
    fn component_chars(&self, a: ActorId, c: &str) -> Vec<CharId> {
        self.comp.borrow().get(&(a, c.to_string())).cloned().unwrap_or_default()
    }
}

const MAPS: [&str; 8] = ["Dungeon/FL_Dungeon", "FeitoriaMap/FL_Feitoria", "DuelCamp/FL_Camp", "Grad/FL_Grad", "Castello/FL_Castello", "TaigaMap/FL_Taiga", "FeitoriaMap/DIH_Feitoria", "MaxMap/FL_MountainPeak"];

/// The first of MAPS whose World has an actor matching `f`
fn world_with(pk: &Pkgs, f: &dyn Fn(&World, ActorId) -> bool) -> Option<(World, ActorId, String)> {
    for m in MAPS {
        let p = format!("Mordhau/Content/Mordhau/Maps/{m}");
        if !pk.rd.exists(&p) {
            continue;
        }
        let d = read(pk, &p);
        let w = World::from_level(pk, &d);
        if let Some(a) = (0..w.actors.len()).find(|a| f(&w, *a)) {
            return Some((w, a, m.to_string()));
        }
    }
    None
}
fn pkgs() -> Option<Pkgs> {
    Vfs::mount_default().ok().map(|v| Pkgs::new(Reader::new(Arc::new(v))))
}
fn tick(w: &mut World, q: &Q, chars: &[CharView], n: usize) -> Vec<WorldEvent> {
    (0..n).flat_map(|_| w.tick(1.0 / 60.0, q, chars)).collect()
}
fn trig(w: &World, a: ActorId) -> Option<&TriggerKind> {
    w.triggers.get(&a).map(|t| &t.kind)
}

/// Spikes (FL_Dungeon): a character entering the Box gets the ragdoll flags and 1000 damage with the spike as Agent;
/// a dead one only the flags; each entry rolls the 13 dismember bones
#[test]
fn spikes() {
    let Some(pk) = pkgs() else { return };
    let Some((mut w, id, m)) = world_with(&pk, &|w, a| matches!(trig(w, a), Some(TriggerKind::Impaler { force_mult, .. }) if *force_mult == 10.0)) else { return eprintln!("SKIP: no spike") };
    println!("{m}: {}", w.actors[id].name);
    let q = Q::default();
    w.begin_play(&q);
    let c = CharView { id: 5, ..Default::default() };
    q.comp.borrow_mut().insert((id, "Box".into()), vec![5]);
    let ev = tick(&mut w, &q, std::slice::from_ref(&c), 1);
    assert!(ev.contains(&WorldEvent::ForceRagdollIfDmgAgent { char: 5, force_mult: 10.0 }));
    assert!(ev.contains(&WorldEvent::TakeDamage { char: 5, amount: 1000.0, damage_type: 0, sub_type: 0, source: Some(5), agent: Some(id) }));
    // staying inside: no second hit; leaving and re-entering dead: flags only
    assert!(!tick(&mut w, &q, std::slice::from_ref(&c), 10).iter().any(|e| matches!(e, WorldEvent::TakeDamage { .. })));
    q.comp.borrow_mut().clear();
    tick(&mut w, &q, &[], 1);
    let dead = CharView { dead: true, ..c.clone() };
    q.comp.borrow_mut().insert((id, "Box".into()), vec![5]);
    let ev = tick(&mut w, &q, std::slice::from_ref(&dead), 1);
    assert!(ev.iter().any(|e| matches!(e, WorldEvent::ForceRagdollIfDmgAgent { .. })) && !ev.iter().any(|e| matches!(e, WorldEvent::TakeDamage { .. })));
    // 13 RandomBoolWithWeight(0.4) draws per entry: the dismembers are a subset of the 13 bones
    let n = ev.iter().filter(|e| matches!(e, WorldEvent::QueueDismember { .. })).count();
    assert!(n <= 13);
}

/// Impalement volumes: a character hitting the Box faster than VelocityToKill against its X axis dies (1000), between
/// 320 and that takes 10, slower nothing; at most one hit per 0.5 s
#[test]
fn impalement() {
    let Some(pk) = pkgs() else { return };
    let Some((mut w, id, m)) = world_with(&pk, &|w, a| matches!(trig(w, a), Some(TriggerKind::Impalement { instant: false, .. }))) else { return eprintln!("SKIP") };
    let Some(TriggerKind::Impalement { velocity_to_kill, .. }) = trig(&w, id).cloned() else { unreachable!() };
    println!("{m}: {} VelocityToKill {velocity_to_kill}", w.actors[id].name);
    let x = w.triggers[&id].xf.m;
    let fx = [x[0][0], x[1][0], x[2][0]];
    let l = (fx[0] * fx[0] + fx[1] * fx[1] + fx[2] * fx[2]).sqrt();
    let into = |s: f64| CharView { id: 3, velocity: fx.map(|c| -c / l * s), ..Default::default() };
    let dmg = |ev: &[WorldEvent]| ev.iter().find_map(|e| if let WorldEvent::TakeDamage { amount, .. } = e { Some(*amount) } else { None });
    assert_eq!(dmg(&w.component_hit(id, &into(300.0))), None);
    assert_eq!(dmg(&w.component_hit(id, &into(400.0))), Some(10.0));
    assert_eq!(dmg(&w.component_hit(id, &into(velocity_to_kill + 1.0))), None, "within 0.5 s");
    tick(&mut w, &Q::default(), &[], 31);
    let ev = w.component_hit(id, &into(velocity_to_kill + 1.0));
    assert_eq!(dmg(&ev), Some(1000.0));
    assert!(ev.iter().any(|e| matches!(e, WorldEvent::KnockbackUnlessDead { .. })));
}

/// Out-of-bounds boxes and spawn protection boxes report entering and leaving; force-fall boxes set the fall
/// damage overrides
#[test]
fn volumes() {
    let Some(pk) = pkgs() else { return };
    for (name, want) in [("OutOfBounds", 0), ("SpawnProtection", 1), ("ForceFallDeath", 2)] {
        let Some((mut w, id, m)) = world_with(&pk, &|w, a| match trig(w, a) {
            Some(TriggerKind::OutOfBounds) => want == 0,
            Some(TriggerKind::SpawnProtection { .. }) => want == 1,
            Some(TriggerKind::ForceFallDeath) => want == 2,
            _ => false,
        }) else {
            println!("SKIP {name}");
            continue;
        };
        let q = Q::default();
        let c = CharView { id: 9, ..Default::default() };
        q.comp.borrow_mut().insert((id, "Box".into()), vec![9]);
        let ev1 = tick(&mut w, &q, std::slice::from_ref(&c), 2);
        q.comp.borrow_mut().clear();
        let ev2 = tick(&mut w, &q, std::slice::from_ref(&c), 1);
        println!("{m} {name} {}: {ev1:?} / {ev2:?}", w.actors[id].name);
        match want {
            0 => {
                assert_eq!(ev1, vec![WorldEvent::OutOfBounds { char: 9, volume: id, entered: true }]);
                assert_eq!(ev2, vec![WorldEvent::OutOfBounds { char: 9, volume: id, entered: false }]);
            }
            1 => {
                assert!(matches!(ev1[..], [WorldEvent::TeamArea { char: 9, entered: true, .. }]));
                assert!(matches!(ev2[..], [WorldEvent::TeamArea { char: 9, entered: false, .. }]));
            }
            _ => assert!(matches!(ev1[..], [WorldEvent::SetFallDamage { char: 9, fall_damage_offset, received_fall_damage_modifier: Some(m), .. }] if fall_damage_offset == 1e10 && m == 1.0)),
        }
    }
}

/// Ammo boxes ask the host to restock; rock piles hand out MaxAmmo rocks, then refill one per
/// AmmoReplenishInterval; food heals and disappears; the cauldron pours on use, spawns its oil fire after
/// SpawnFireTime and is usable again after ReloadTime
#[test]
fn pickups_and_cauldron() {
    let Some(pk) = pkgs() else { return };
    let c = CharView { id: 2, has_player_controller: true, ..Default::default() };
    let q = Q::default();
    if let Some((mut w, id, m)) = world_with(&pk, &|w, a| matches!(w.actors[a].kind, Kind::Pickup(Pickup::AmmoBox))) {
        println!("{m}: ammo box {}", w.actors[id].name);
        assert_eq!(w.interact(id, &c), vec![WorldEvent::RestockFromAmmoBox { char: 2, ammo_box: id }]);
    }
    if let Some((mut w, id, m)) = world_with(&pk, &|w, a| matches!(w.actors[a].kind, Kind::Pickup(Pickup::Restockable { .. }))) {
        w.begin_play(&q);
        let p = w.actors[id].props.clone();
        let (max, iv) = (p.i("MaxAmmo"), p.f("AmmoReplenishInterval"));
        println!("{m}: {} {} max {max} every {iv}", w.actors[id].class, p.obj("Equipment"));
        for _ in 0..max {
            assert!(w.can_interact(id, &c).0);
            assert!(w.interact(id, &c).iter().any(|e| matches!(e, WorldEvent::GiveEquipment { char: 2, .. })));
        }
        assert!(!w.can_interact(id, &c).0, "empty");
        tick(&mut w, &q, &[], (iv * 60.0) as usize + 2);
        assert!(w.can_interact(id, &c).0, "one back");
        let holding = CharView { right_hand_class: p.obj("Equipment"), ..c.clone() };
        assert!(!w.can_interact(id, &holding).0, "already holding one");
    }
    if let Some((mut w, id, m)) = world_with(&pk, &|w, a| matches!(w.actors[a].kind, Kind::Pickup(Pickup::Food { .. }))) {
        println!("{m}: food {}", w.actors[id].name);
        let ev = w.interact(id, &c);
        assert!(ev.contains(&WorldEvent::OffsetHealth { char: 2, amount: 20 }) && ev.contains(&WorldEvent::Destroyed { actor: id }));
        assert!(!w.can_interact(id, &c).0);
    }
    if let Some((mut w, id, m)) = world_with(&pk, &|w, a| matches!(w.actors[a].kind, Kind::Cauldron(_))) {
        let p = w.actors[id].props.clone();
        println!("{m}: cauldron {} fire {} reload {}", w.actors[id].name, p.f("SpawnFireTime"), p.f("ReloadTime"));
        w.begin_play(&q);
        // a kick (Move 4) activates it as well as use
        w.apply_damage(id, 10.0, &DamageInfo { causer: Some(2), attack_move: Some(4), ..Default::default() }, &q, std::slice::from_ref(&c));
        assert!(!w.can_interact(id, &c).0);
        let ev = tick(&mut w, &q, &[], (p.f("SpawnFireTime") * 60.0) as usize + 2);
        assert!(ev.iter().any(|e| matches!(e, WorldEvent::SpawnAt { class, instigator: Some(2), .. } if class == "BP_OilFire_C")), "{ev:?}");
        tick(&mut w, &q, &[], (p.f("ReloadTime") * 60.0) as usize + 2);
        assert!(w.can_interact(id, &c).0);
    }
}

/// BP_ItemDeliverySpawn (torches / barrels / chests for the delivery spots): inactive until its spot's capture point
/// hands the enemy the prerequisites; then usable by the point's enemies not already carrying a deliverable, giving
/// a deliverable of its class and Type usable by the enemy team; losing the prerequisites destroys what it handed out
#[test]
fn delivery_spawns() {
    let Some(pk) = pkgs() else { return };
    let Some((mut w, id, m)) = world_with(&pk, &|w, a| matches!(w.actors[a].kind, Kind::DeliverySpawn { capture_point: Some(_), .. })) else { return eprintln!("SKIP") };
    let Kind::DeliverySpawn { capture_point: Some(cp), .. } = w.actors[id].kind else { unreachable!() };
    let owner = w.cps[&cp].owning_team;
    let enemy = owner.map(|t| 1 - t).unwrap_or(0);
    println!("{m}: {} {} -> point {} (owner {owner:?})", w.actors[id].name, w.actors[id].props.obj("DeliverableClass"), w.actors[cp].name);
    let c = CharView { id: 4, team: Some(enemy), ..Default::default() };
    assert!(!w.can_interact(id, &c).0, "inactive");
    w.capture_point_prerequisites(cp, true);
    assert!(w.can_interact(id, &c).0);
    assert!(!w.can_interact(id, &CharView { team: owner, ..c.clone() }).0, "the owners");
    assert!(!w.can_interact(id, &CharView { holding_deliverable: true, ..c.clone() }).0, "already carrying one");
    let ev = w.interact(id, &c);
    assert!(matches!(&ev[..], [WorldEvent::GiveDeliverable { char: 4, usable_by_team, .. }] if *usable_by_team == Some(enemy)), "{ev:?}");
    w.capture_point_prerequisites(cp, false);
    let ev = tick(&mut w, &Q::default(), &[], 1);
    assert!(ev.contains(&WorldEvent::DestroySpawnedDeliverables { by: id }), "{ev:?}");
    assert!(!w.can_interact(id, &c).0);
}

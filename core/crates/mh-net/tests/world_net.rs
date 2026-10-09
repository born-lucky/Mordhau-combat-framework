//! Placed-actor replication through mh-net (world_rep.rs) on a real map: a server mh-world World on FL_Camp pushes
//! the Frontline wagon and opens then breaks a door; its replicated variables go through WorldRepServer (the 10 Hz
//! pushable / next-frame Blueprint actor rules), the wire encoding of Msg::WorldProp and a 100 ms delay to a client
//! World (set_authority(false)) that runs the OnReps through world_rep::MhWorld (mh-world r2's apply_rep). The client
//! ends with the server's replicated state, the wagon where the server has it and the door's leaf at the same yaw.
//! SKIP (pass) without the install.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use mh_level::{read, Pkgs};
use mh_net::msg::Msg;
use mh_net::world_rep::{MhWorld, RepWorld, WorldRepServer};
use mh_pak::{Reader, Vfs};
use mh_world::{ActorId, CharId, CharView, DamageInfo, Kind, Queries, SpawnedStatus, World};

#[derive(Default)]
struct Q {
    chars: BTreeMap<ActorId, Vec<CharId>>,
}
impl Queries for Q {
    fn overlapping_chars(&self, a: ActorId) -> Vec<CharId> {
        self.chars.get(&a).cloned().unwrap_or_default()
    }
    fn overlapping_actors(&self, _: ActorId) -> Vec<ActorId> {
        vec![]
    }
    fn spawned_status(&self, _: ActorId) -> SpawnedStatus {
        SpawnedStatus::default()
    }
}

#[test]
fn fl_camp_world_over_the_net() {
    let Ok(v) = Vfs::mount_default() else { return eprintln!("SKIP: no install") };
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    let p = "Mordhau/Content/Mordhau/Maps/DuelCamp/FL_Camp";
    if !pk.rd.exists(p) {
        return eprintln!("SKIP: no FL_Camp");
    }
    let d = read(&pk, p);
    let (mut sv, mut cl) = (World::from_level(&pk, &d), World::from_level(&pk, &d));
    cl.set_authority(false);
    let q0 = Q::default();
    sv.begin_play(&q0);
    cl.begin_play(&q0);
    let wagon = sv.actors.iter().find(|a| a.class == "BP_FrontlineWagonTorch_C").unwrap().id;
    let door = sv.actors.iter().find(|a| matches!(&a.kind, Kind::Door(x) if x.state != 0) && a.props.f("DamageFactor") > 0.0).unwrap().id;
    let (cpid, team) = match &sv.actors[wagon].kind {
        Kind::Objective(o) => (o.capture_point, if o.push.as_ref().unwrap().team1_curve.is_some() { 0 } else { 1 }),
        _ => (None, 0),
    };
    if let Some(cp) = cpid {
        sv.capture_point_prerequisites(cp, true);
    }
    let mut q = Q::default();
    q.chars.insert(wagon, vec![1]);
    let pusher = CharView { id: 1, team: Some(team), ..Default::default() };
    sv.interact(door, &CharView { id: 2, forward: [1.0, 0.0, 0.0], ..Default::default() });
    let mut rep = WorldRepServer::default();
    let mut wire: VecDeque<(usize, Vec<u8>)> = VecDeque::new();
    let delay = 6; // frames: 100 ms at 60 Hz
    for f in 0..240usize {
        sv.tick(1.0 / 60.0, &q, &[pusher.clone()]);
        if f == 120 {
            for _ in 0..200 {
                sv.apply_damage(door, 50.0, &DamageInfo { attack_move: Some(0), ..Default::default() }, &q0, &[]);
            }
        }
        let vars: Vec<(u32, String, i64)> = sv.rep_vars().into_iter().map(|(a, k, v)| (a as u32, k.to_string(), v)).collect();
        for m in rep.diff(1, f as f64 / 60.0, &vars) {
            wire.push_back((f + delay, m.encode()));
        }
        let mut a = MhWorld::new(&mut cl, &q0);
        while wire.front().is_some_and(|(t, _)| *t <= f) {
            let (_, b) = wire.pop_front().unwrap();
            if let Some(Msg::WorldProp { actor, var, value }) = Msg::decode(&b) {
                a.apply_rep(actor, &var, value);
            }
        }
        cl.tick(1.0 / 60.0, &Q::default(), &[]);
    }
    // drain the last delayed updates and let the client's interpolation settle
    for f in 240..300usize {
        let mut a = MhWorld::new(&mut cl, &q0);
        while wire.front().is_some_and(|(t, _)| *t <= f) {
            let (_, b) = wire.pop_front().unwrap();
            if let Some(Msg::WorldProp { actor, var, value }) = Msg::decode(&b) {
                a.apply_rep(actor, &var, value);
            }
        }
        cl.tick(1.0 / 60.0, &Q::default(), &[]);
        sv.tick(1.0 / 60.0, &Q::default(), &[]);
        let vars: Vec<(u32, String, i64)> = sv.rep_vars().into_iter().map(|(a, k, v)| (a as u32, k.to_string(), v)).collect();
        for m in rep.diff(1, f as f64 / 60.0, &vars) {
            wire.push_back((f + delay, m.encode()));
        }
    }
    let sv_vars: BTreeMap<(ActorId, &str), i64> = sv.rep_vars().into_iter().map(|(a, k, v)| ((a, k), v)).collect();
    let cl_vars: BTreeMap<(ActorId, &str), i64> = cl.rep_vars().into_iter().map(|(a, k, v)| ((a, k), v)).collect();
    let differ: Vec<_> = sv_vars.iter().filter(|(k, v)| cl_vars.get(*k) != Some(*v)).collect();
    println!("props sent {}; differing at the end {:?}", rep.props_sent, differ);
    assert!(differ.is_empty(), "client state differs: {differ:?}");
    let (Kind::Objective(so), Kind::Objective(co)) = (&sv.actors[wagon].kind, &cl.actors[wagon].kind) else { unreachable!() };
    let (sp, cp) = (so.push.as_ref().unwrap().progress, co.push.as_ref().unwrap().progress);
    println!("wagon server {sp:.5} client {cp:.5}");
    assert!(sp > 0.0 && (sp - cp).abs() < 0.002, "{sp} vs {cp}");
    let (Kind::Door(sd), Kind::Door(cd)) = (&sv.actors[door].kind, &cl.actors[door].kind) else { unreachable!() };
    assert_eq!((sd.state, sd.health.replicated, sd.health.collision), (cd.state, cd.health.replicated, cd.health.collision));
    assert!((sd.yaw - cd.yaw).abs() < 1e-6, "{} vs {}", sd.yaw, cd.yaw);
}

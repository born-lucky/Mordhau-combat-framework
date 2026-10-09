//! Port of godot/tests/test_net.gd: the game's replication rules with a server and two clients in one process, and
//! the golden property: client and server end tick-identical to a server-only run. Expected values follow from the
//! rules cited in src/ (UMotionSystemComponent::OnServerAssignNetMotion rva=0x14cf410, AssignNetMotion rva=0x14b34c0,
//! HandleNetMotionUpdate rva=0x14c01b0, AMordhauCharacter::GetLifetimeReplicatedProps rva=0x153fd60,
//! ServerDropParry_Implementation rva=0x1567ad0, UStatComponent::WriteReplicatedStat rva=0x1520b50 /
//! ReadReplicatedStat rva=0x1513260, UStaminaStatComponent::OffsetStamina rva=0x1507c20). The world is the test
//! fixture in tests/support (NOT combat: see its header).

mod support;

use mh_net::enums::{self, combat};
use mh_net::msg::Msg;
use mh_net::stat::{read_replicated_stat, write_replicated_stat};
use mh_net::transport::UdpLocal;
use mh_net::{FNetMotion, NetSession, NetWorld};
use support::{Input, Kind, ToyWorld};

const DT: f64 = 0.001;

fn session(latency: u64, dt: f64) -> NetSession<ToyWorld> {
    let mut s = NetSession::new(dt, latency, Box::new(move || ToyWorld::new(dt)));
    s.add_player("A", "LS", "");
    s.add_player("B", "LS", "");
    s
}

/// the bout: attack -> parry -> blocked, riposte-like attack -> hit + flinch, feint, attacks -> hits -> death.
/// (time, input); contacts are the server's hit detection
fn bout() -> (Vec<(f64, Input)>, f64) {
    let a = || "A".to_string();
    let b = || "B".to_string();
    let v = vec![
        (0.1, Input::Attack(a())),
        (0.3, Input::Parry(b())),
        (0.6, Input::Contact(a(), b())),
        (0.85, Input::Attack(b())),
        (1.3, Input::Contact(b(), a())),
        (1.8, Input::Attack(a())),
        (1.9, Input::Feint(a())),
        (2.3, Input::Attack(a())),
        (2.75, Input::Contact(a(), b())),
        (3.3, Input::Attack(a())),
        (3.75, Input::Contact(a(), b())),
        (4.3, Input::Attack(a())),
        (4.75, Input::Contact(a(), b())),
        (4.85, Input::Attack(b())),
    ];
    (v, 5.2)
}

fn reference(dt: f64) -> ToyWorld {
    let mut w = ToyWorld::new(dt);
    w.add_local("A", enums::ROLE_AUTHORITY, false);
    w.add_local("B", enums::ROLE_AUTHORITY, false);
    let (sched, t_end) = bout();
    for (t, i) in sched {
        w.at(t, i);
    }
    w.run_until(t_end);
    w
}

/// client inputs go to the owner's client world, contacts to the server
fn replay(s: &mut NetSession<ToyWorld>) {
    for (t, i) in bout().0 {
        match &i {
            Input::Contact(..) => s.server.world.at(t, i),
            Input::Attack(w) | Input::Parry(w) | Input::Feint(w) | Input::ReleaseBlock(w) | Input::Force(w, _) => {
                let w = w.clone();
                s.client(&w).unwrap().world.at(t, i)
            }
        }
    }
}

fn diff(a: &[String], b: &[String]) -> Option<String> {
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).map(|s| s.as_str()).unwrap_or("<end>");
        let y = b.get(i).map(|s| s.as_str()).unwrap_or("<end>");
        if x != y {
            return Some(format!("#{i}: {x} vs {y}"));
        }
    }
    None
}

fn check_all_machines(s: &NetSession<ToyWorld>, rw: &ToyWorld, how: &str) {
    let want = rw.timeline();
    let mut machines: Vec<(String, &ToyWorld)> = vec![("server".into(), &s.server.world)];
    for c in &s.clients {
        machines.push((format!("client {}", c.own), &c.world));
    }
    for (n, w) in machines {
        if let Some(d) = diff(&want, &w.timeline()) {
            panic!("{how}: {n} timeline differs from the server-only run {d}");
        }
        assert_eq!(w.outcome(), rw.outcome(), "{how}: {n} outcome");
    }
}

#[test]
fn net_duel_matches_server_only_run() {
    let rw = reference(DT);
    assert!(rw.hits.len() >= 4, "reference bout has too few outcomes: {:?}", rw.hits);
    assert!(rw.p("B").dead, "reference bout must end with a death");
    let mut s = session(0, DT);
    replay(&mut s);
    s.run_until(bout().1);
    let want: Vec<_> = rw.hits.iter().map(|h| format!("{:.3} {}>{}", h.0, h.1, h.2)).collect();
    let got: Vec<_> = s.server.world.hits.iter().map(|h| format!("{:.3} {}>{}", h.0, h.1, h.2)).collect();
    assert_eq!(got, want, "server hits differ from the server-only run");
    assert_eq!(s.server.rejected, 0);
    check_all_machines(&s, &rw, "loopback");
    for c in &s.clients {
        assert!(c.world.hits.is_empty(), "client {} processed a hit itself", c.own);
    }
    // what each client sees of the parry: A in Blocked(Parry) from 0.6, B's parry counting one block
    for c in &s.clients {
        let l = c.world.timeline();
        assert!(l.iter().any(|x| x == "0.600 A Blocked"), "client {} timeline {:?}", c.own, l);
    }
}

/// The same bout over real UDP sockets on 127.0.0.1 (UdpLocal): the rules unchanged, the transport swapped.
#[test]
fn net_duel_over_udp() {
    let dt = 0.005;
    let rw = reference(dt);
    let t = UdpLocal::start(2).expect("bind 127.0.0.1 UDP sockets");
    let mut s = NetSession::with_transport(dt, 0, Box::new(move || ToyWorld::new(dt)), Box::new(t));
    s.add_player("A", "LS", "");
    s.add_player("B", "LS", "");
    replay(&mut s);
    s.run_until(bout().1);
    check_all_machines(&s, &rw, "udp");
    let (sent, _) = s.transport.stats();
    assert!(sent >= 10, "only {sent} messages crossed the sockets");
}

/// With latency the clients still converge on the server's outcome (who was hit, who died, final health).
#[test]
fn net_duel_latency_clients_converge() {
    let mut s = session(20, DT);
    replay(&mut s);
    s.run_until(bout().1 + 0.1);
    let want = s.server.world.outcome();
    let sd = s.server.world.deaths();
    for c in &s.clients {
        assert_eq!(c.world.outcome(), want, "client {}", c.own);
        assert_eq!(c.world.deaths(), sd, "client {} deaths", c.own);
    }
}

/// OnServerAssignNetMotion: only the next Id is accepted, and only when the client had seen the server's latest motion
/// (LastAuthObserved == NetMotion.Id) or the server's latest motion was the client's own last accepted one.
#[test]
fn server_assign_net_motion_id_rules() {
    let mut s = session(0, DT);
    let p = s.server.world.pm("A");
    let mut rep = p.rep.take().unwrap();
    let mut nm = FNetMotion::new(combat::NET_PARRY, 0, 0, 0, 0);
    nm.id = 5; // NetMotion.Id is 0: not the next one
    assert!(!rep.on_server_assign_net_motion(p, nm, 0), "Id 5 accepted after Id 0");
    nm.id = 1;
    assert!(rep.on_server_assign_net_motion(p, nm, 0), "Id 1 with LastAuthObserved 0 refused");
    assert_eq!(rep.last_accepted_client_motion_id, 1);
    assert!(p.motion.kind == Kind::Parry && p.net.id == 1, "accepted motion not assigned");
    // two predictions in flight: LastAuthObserved still 0, but Id 1 was this client's own accepted motion -> Id 2
    let mut n2 = FNetMotion::new(combat::NET_FEINTED, 0, 0, 0, 0);
    n2.id = 2;
    assert!(rep.on_server_assign_net_motion(p, n2, 0), "Id 2 after the client's own accepted Id 1 refused");
    // the server forces a motion (Id 3, a hit's flinch); a prediction made before the client saw it is dropped
    p.rep = Some(rep);
    p.assign(FNetMotion::new(combat::NET_FLINCHED, 0, 0, 0, 0));
    assert_eq!(p.net.id, 3);
    let mut rep = p.rep.take().unwrap();
    let mut n4 = FNetMotion::new(combat::NET_PARRY, 0, 0, 0, 0);
    n4.id = 4;
    assert!(!rep.on_server_assign_net_motion(p, n4, 2), "prediction made without seeing the server's Id 3 accepted");
    assert!(rep.on_server_assign_net_motion(p, n4, 3), "Id 4 with LastAuthObserved 3 refused");
    p.rep = Some(rep);
}

/// AssignNetMotion on the authority: ReplicatedNetMotion (COND_SkipOwner) goes to the other client as a property and to
/// the owner only as ClientSetNetMotion.
#[test]
fn replicated_net_motion_skip_owner() {
    let mut s = session(0, DT);
    s.server.world.pm("A").assign(FNetMotion::new(combat::NET_PARRY, 0, 0, 0, 0));
    let t = s.transport.as_mut();
    s.server.replicate(t);
    let motion_msg = |m: &Msg| m.who() == "A" && (matches!(m, Msg::ClientSetNetMotion { .. }) || m.name() == enums::PROP_REPLICATED_NET_MOTION);
    let to_owner: Vec<Msg> = t.receive(1).into_iter().map(|e| e.1).filter(|m| motion_msg(m)).collect();
    let to_other: Vec<Msg> = t.receive(2).into_iter().map(|e| e.1).filter(|m| motion_msg(m)).collect();
    assert_eq!(to_owner.len(), 1, "owner got {to_owner:?}");
    assert!(matches!(to_owner[0], Msg::ClientSetNetMotion { .. }));
    assert_eq!(to_other.len(), 1, "other got {to_other:?}");
    let (Msg::ClientSetNetMotion { nm, .. }, Msg::Prop { v: mh_net::layout::PropValue::NetMotion(pv), .. }) = (&to_owner[0], &to_other[0]) else {
        panic!("kinds {to_owner:?} / {to_other:?}")
    };
    assert_eq!(nm, pv);
    assert_eq!(pv[1], combat::NET_PARRY);
    assert_eq!(pv[0], 1);
}

/// The owning client predicts its attack at once, sends ServerAssignNetMotion, and the server's ClientSetNetMotion of
/// the same motion only confirms it (no second motion object); the simulated proxy gets it at the same server tick.
#[test]
fn prediction_confirmed_not_restarted() {
    let mut s = session(0, DT);
    s.client("A").unwrap().world.at(0.1, Input::Attack("A".into()));
    s.run_until(0.1);
    let ca = s.client("A").unwrap();
    let p = ca.world.p("A");
    assert!(p.motion.kind == Kind::Attack && (p.motion.start - 0.1).abs() < 1e-4, "client A did not predict at 0.1");
    assert!(mh_net::pawn::is_initiated_locally(p) && mh_net::pawn::is_confirmed(p), "prediction not confirmed");
    let serial = p.motion.serial;
    s.run_until(0.2);
    let ca = s.client("A").unwrap();
    assert_eq!(ca.world.p("A").motion.serial, serial, "the server's confirmation restarted the predicted attack");
    assert_eq!(ca.world.timeline().iter().filter(|l| l.ends_with("A Attack")).count(), 1);
    let sm = s.server.world.p("A").motion.clone();
    assert!(sm.kind == Kind::Attack && (sm.start - 0.1).abs() < 1e-4);
    let cb = s.client("B").unwrap();
    let pm = cb.world.p("A");
    assert!(pm.motion.kind == Kind::Attack && (pm.motion.start - 0.1).abs() < 1e-4, "proxy start {}", pm.motion.start);
    assert!(!mh_net::pawn::is_initiated_locally(pm), "the simulated proxy's motion counts as locally initiated");
}

/// Two predictions in flight (attack 0.1, feint 0.19; 50 ms each way, so the attack's confirmation is back at 0.2 and
/// the feint's at 0.29): the feint pushes the unconfirmed attack into UnconfirmedMotionsBacklog; the server's
/// confirmation of the attack, arriving after the feint was predicted, is consumed there instead of restarting the
/// attack; the feint's own confirmation then confirms it.
#[test]
fn backlog_consumes_superseded_confirmation() {
    let mut s = session(50, DT);
    {
        let ca = s.client("A").unwrap();
        ca.world.at(0.1, Input::Attack("A".into()));
        ca.world.at(0.19, Input::Feint("A".into()));
    }
    s.run_until(0.19);
    let ca = s.client("A").unwrap();
    let feint = ca.world.p("A").motion.serial;
    assert_eq!(ca.world.p("A").motion.kind, Kind::Feinted);
    let types: Vec<u8> = ca.rep("A").unwrap().backlog.iter().map(|n| n.motion_type).collect();
    assert_eq!(types, vec![combat::NET_ATTACK, 0], "backlog newest first: the attack, then the spawn's NetMotion");
    s.run_until(0.21);
    let ca = s.client("A").unwrap();
    assert_eq!(ca.rep("A").unwrap().backlog.len(), 1, "attack confirmation not consumed at the front");
    assert_eq!(ca.world.p("A").motion.serial, feint, "attack confirmation replaced the feint");
    assert!(!mh_net::pawn::is_confirmed(ca.world.p("A")), "feint confirmed before its own confirmation arrived");
    s.run_until(0.3);
    let ca = s.client("A").unwrap();
    let p = ca.world.p("A");
    assert_eq!(p.motion.serial, feint);
    assert!(mh_net::pawn::is_confirmed(p) && ca.rep("A").unwrap().backlog.is_empty());
    assert!((p.slot.confirmed_by_authority_time - 0.29).abs() < DT * 0.5, "confirmed at {}", p.slot.confirmed_by_authority_time);
    assert_eq!(s.server.world.p("A").motion.kind, Kind::Feinted);
    assert_eq!(s.server.rejected, 0);
}

/// A motion the server forces (B parries A's attack -> A Blocked) replaces A's own predicted attack on A's client.
#[test]
fn server_motion_overrides_prediction() {
    let mut s = session(0, DT);
    s.client("A").unwrap().world.at(0.1, Input::Attack("A".into()));
    s.client("B").unwrap().world.at(0.3, Input::Parry("B".into()));
    s.server.world.at(0.6, Input::Contact("A".into(), "B".into()));
    s.run_until(0.65);
    let ca = s.client("A").unwrap();
    let p = ca.world.p("A");
    assert!(p.motion.kind == Kind::Blocked && p.motion.reason == combat::BLOCKED_PARRY && (p.motion.start - 0.6).abs() < DT * 0.5);
    assert!(mh_net::pawn::is_confirmed(p), "a server motion must arrive confirmed");
}

/// ServerDropParry: the client that releases block sends the RPC with its NetMotion Id; the server drops the parry at
/// the tick the client asked. A stale MotionID does nothing.
#[test]
fn server_drop_parry_rpc() {
    let mut s = session(0, DT);
    {
        let cf = s.client("A").unwrap();
        cf.world.at(0.1, Input::Parry("A".into()));
        cf.world.at(0.15, Input::ReleaseBlock("A".into()));
    }
    s.client("B").unwrap().world.at(0.1, Input::Parry("B".into()));
    s.run_until(0.3);
    assert!(s.server.world.timeline().contains(&"0.150 A Idle".to_string()), "server drop: {:?}", s.server.world.timeline());
    assert_eq!(s.server.world.p("B").motion.kind, Kind::Parry, "server dropped B's held parry by itself");
    let id = s.server.world.p("B").net.id.wrapping_add(1);
    s.server.world.server_drop_parry_impl("B", id);
    assert_eq!(s.server.world.p("B").motion.kind, Kind::Parry, "a stale MotionID dropped the parry");
}

/// OffsetStamina: only the authority applies it; the client gets the change through ReplicatedStamina.
#[test]
fn stamina_offsets_on_authority_only() {
    let mut s = session(0, DT);
    s.client("A").unwrap().world.pm("A").offset_stamina(-30);
    assert_eq!(s.client("A").unwrap().world.p("A").stamina, 100, "client applied a stamina offset itself");
    s.server.world.pm("A").offset_stamina(-30);
    s.step();
    for c in &s.clients {
        assert_eq!(c.world.p("A").stamina, 70, "client {} stamina from ReplicatedStamina", c.own);
    }
}

/// WriteReplicatedStat / ReadReplicatedStat: RoundToInt percent, 1 for a non-zero value that rounds to 0, the 0x80 flip
/// on an unchanged byte, low 7 bits read back.
#[test]
fn replicated_stat_byte() {
    assert_eq!(write_replicated_stat(28, 0, 100, 0), 28);
    assert_eq!(write_replicated_stat(28, 0, 100, 28), 28 | 0x80, "same byte not flipped");
    assert_eq!(write_replicated_stat(28, 0, 100, 28 | 0x80), 28, "flipped byte not flipped back");
    assert_eq!(write_replicated_stat(1, 0, 1000, 0), 1, "non-zero never reads 0");
    assert_eq!(write_replicated_stat(0, 0, 100, 5), 0);
    assert_eq!(write_replicated_stat(5, 5, 5, 0), 100, "degenerate range at Max");
    assert_eq!(write_replicated_stat(125, 0, 250, 0), 50);
    assert_eq!(read_replicated_stat(28 | 0x80, 0, 100), 28);
    assert_eq!(read_replicated_stat(50, 0, 250), 125);
    for v in 0..=100 {
        assert_eq!(read_replicated_stat(write_replicated_stat(v, 0, 100, 0), 0, 100), v, "round trip {v}");
    }
}

/// the FNetMotion wire form and compare (HandleNetMotionUpdate's inlined compare)
#[test]
fn net_motion_bytes_and_compare() {
    let mut a = FNetMotion::new(1, 2, 3, 4, 5);
    a.id = 9;
    assert_eq!(a.to_bytes(), [9, 1, 2, 3, 4, 5]);
    assert_eq!(FNetMotion::from_bytes(a.to_bytes()), a);
    let mut b = a;
    assert_eq!(mh_net::netmotion::compare(&a, &b), 0);
    b.dynamic_param = 7;
    assert_eq!(mh_net::netmotion::compare(&a, &b), 1);
    b.param2 = 0;
    assert_eq!(mh_net::netmotion::compare(&a, &b), 2);
    let m = Msg::ServerAssignNetMotion { who: "A".into(), nm: a.to_bytes(), last: 3 };
    assert_eq!(Msg::decode(&m.encode()), Some(m));
}

/// every .rdata constant's bits equal extract/native/rdata.tsv (when the extract is present on this machine), and
/// each citing function is in that row's funcs column
#[test]
fn net_constants_cited() {
    use mh_net::consts::{CITES, CITES64};
    assert_eq!(CITES.len(), 31);
    assert_eq!(mh_net::consts::STAT_UNIT_TO_BYTE_X2, 200.0);
    assert!((mh_net::consts::STAT_BYTE_TO_UNIT - 0.01).abs() < 1e-7);
    assert!((mh_net::consts::LAG_INDUCTION_MAX - 0.15).abs() < 1e-7);
    assert_eq!(mh_net::consts::PONG_S_TO_MS, 1000.0);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extract/native/rdata.tsv");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("rdata.tsv not present: bits checked by value only");
        return;
    };
    let mut rows = std::collections::HashMap::new();
    for line in text.lines().skip(1) {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() >= 9 {
            rows.insert(c[0].to_string(), (c[3].to_string(), c[8].to_string()));
        }
    }
    let le = |raw: &str| -> u64 {
        let b: Vec<u8> = (0..raw.len() / 2).map(|i| u8::from_str_radix(&raw[2 * i..2 * i + 2], 16).unwrap()).collect();
        b.iter().rev().fold(0u64, |v, x| (v << 8) | *x as u64)
    };
    for (name, va, bits, funcs) in CITES {
        let (raw, fl) = rows.get(&format!("{va:x}")).unwrap_or_else(|| panic!("{name}: va {va:#x} not in rdata.tsv"));
        assert_eq!(le(&raw[..8]) as u32, *bits, "{name}: bits");
        for f in *funcs {
            assert!(fl.split(';').any(|x| x == *f), "{name}: {va:#x} is not loaded by {f}");
        }
    }
    for (name, va, bits, funcs) in CITES64 {
        let (raw, fl) = rows.get(&format!("{va:x}")).unwrap();
        assert_eq!(le(&raw[..16]), *bits, "{name}: bits");
        for f in *funcs {
            assert!(fl.split(';').any(|x| x == *f), "{name}: not loaded by {f}");
        }
    }
}

//! The net tests on REAL combat: mordhau-core's World (the golden-parity combat port) driven by mh-net through the
//! NetHooks seam (mordhau-core combat/net.rs) and core_world.rs. These are the GDScript reference's own cases
//! (godot/tests/test_net.gd, test_net_ping.gd, test_net_match.gd) with their expected values, plus the golden
//! property over every combat scenario of core/tests/scenarios/combat.json: server + clients end tick-identical to a
//! server-only run. Data: core/tests/golden/spec.json (ignored path); absent -> SKIP (MORDHAU_GOLDEN_REQUIRED=1 makes
//! it a failure).

use std::path::Path;
use std::rc::Rc;

use mh_net::core_world::NetRules;
use mh_net::duel_match::{ModeEvent, NetDuelMatch, ROOM_GAME};
use mh_net::enums::{self, combat as ce};
use mh_net::msg::Msg;
use mh_net::pawn::Pawn;
use mh_net::rep::{BlockResult, NetMotionRep};
use mh_net::transport::UdpLocal;
use mh_net::{FNetMotion, NetSession, NetWorld};
use mordhau_core::combat::world::Call;
use mordhau_core::combat::{damage, Input, Motion, NetHooks, World};
#[cfg(feature = "reference_compat")]
use mordhau_core::data::RecordsJson;
use mordhau_core::data::{Spec, SpecSource};
use serde_json::Value;

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
const FISTS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/BP_FistsWeapon";
const DT: f64 = 0.001;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// the machine model under test: the exe (default; World::new on the exe's f32 records) or, with the
/// `reference_compat` feature, the GDScript reference bit for bit (World::new_reference)
fn new_world(sp: Rc<Spec>, dt: f64) -> World {
    #[cfg(feature = "reference_compat")]
    return World::new_reference(sp, dt);
    #[cfg(not(feature = "reference_compat"))]
    return World::new(sp, dt);
}

fn load(t: &str) -> Spec {
    #[cfg(feature = "reference_compat")]
    return RecordsJson(t).load_spec().expect("spec.json");
    #[cfg(not(feature = "reference_compat"))]
    return mordhau_core::data::RecordsJsonExe(t).load_spec().expect("spec.json");
}

fn spec() -> Option<Rc<Spec>> {
    match std::fs::read_to_string(root().join("core/tests/golden/spec.json")) {
        Ok(t) => Some(Rc::new(load(&t))),
        Err(_) => {
            assert!(std::env::var("MORDHAU_GOLDEN_REQUIRED").map(|v| v != "1").unwrap_or(true), "golden spec missing");
            eprintln!("SKIP: core/tests/golden/spec.json missing (godot/tools/export_golden.gd)");
            None
        }
    }
}

macro_rules! need_spec {
    () => {
        match spec() {
            Some(s) => s,
            None => return,
        }
    };
}

fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}
fn f32r(x: f64) -> f64 {
    x as f32 as f64
}
fn snapped(x: f64, step: f64) -> f64 {
    (x / step).round() * step
}

fn fi(w: &World, n: &str) -> usize {
    w.fighter_index(n).unwrap_or_else(|| panic!("no fighter {n}"))
}
fn cur<'a>(w: &'a World, n: &str) -> &'a Motion {
    w.cur_m(fi(w, n)).unwrap()
}
fn attack(t: f64, who: &str, mv: i64) -> (f64, Input) {
    (t, Input::Attack { who: who.into(), mv, angle: 0.0 })
}
fn parry(t: f64, who: &str) -> (f64, Input) {
    (t, Input::Parry { who: who.into(), bt: 0 })
}
fn contact(t: f64, a: &str, b: &str, bone: &str) -> (f64, Input) {
    (t, Input::Contact { who: a.into(), target: b.into(), bone: bone.into() })
}

fn session(sp: &Rc<Spec>, latency: u64, weapon: &str) -> NetSession<World> {
    let sp2 = sp.clone();
    let mut s = NetSession::new(DT, latency, Box::new(move || new_world(sp2.clone(), DT)));
    s.add_player("A", weapon, "");
    s.add_player("B", weapon, "");
    s
}

fn reference(sp: &Rc<Spec>, names: &[(&str, &str)]) -> World {
    let mut w = new_world(sp.clone(), DT);
    for (n, wp) in names {
        w.add_fighter(n, wp, "");
    }
    w
}

/// what a machine shows: every motion change (t who motion) and every death, in order (test_net.gd _timeline)
fn timeline(w: &World) -> Vec<String> {
    let mut out = vec![];
    for e in &w.hits {
        let t = e.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let who = e.get("who").and_then(|v| v.as_str()).unwrap_or("");
        match e.get("kind").and_then(|v| v.as_str()) {
            Some("motion") => out.push(format!("{t:.3} {who} {}", e["motion"].as_str().unwrap_or(""))),
            Some("died") => out.push(format!("{t:.3} {who} died")),
            _ => {}
        }
    }
    out
}

fn outcome(w: &World) -> String {
    w.fighters.iter().map(|f| format!("{} {}/{} dead {}", f.name, f.health, f.stamina, f.dead)).collect::<Vec<_>>().join(", ")
}

fn processed_hits(w: &World) -> usize {
    w.hits.iter().filter(|e| matches!(e.get("kind").and_then(|v| v.as_str()), Some("hit" | "parry" | "chamber"))).count()
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

/// run the session until the server's tick `t`, then run `f` on the server world in that frame (the reference's
/// `server.world.at(t, callable)`: here after the tick's motions, same start time t)
fn server_at(s: &mut NetSession<World>, t: f64, f: impl FnOnce(&mut World)) {
    s.run_until(t - DT);
    let mut f = Some(f);
    s.step_with(&mut |srv, _, _| {
        if let Some(f) = f.take() {
            f(&mut srv.world);
        }
    });
}

// ---- the duel bout (test_net.gd) ------------------------------------------------------------------------------------
/// The bout of test_combat (attack -> parry -> riposte -> hit + flinch -> feint -> attack -> head hit -> death), its
/// inputs recorded while it runs on one machine
fn reference_bout(sp: &Rc<Spec>) -> (World, Vec<(f64, Input)>, f64) {
    let mut w = reference(sp, &[("A", LS), ("B", LS)]);
    let mut sched = vec![];
    let mut put = |w: &mut World, ev: (f64, Input)| {
        w.at(ev.0, ev.1.clone());
        sched.push(ev);
    };
    put(&mut w, attack(0.1, "A", ce::MOVE_RIGHT_STRIKE as i64));
    put(&mut w, parry(0.5, "B"));
    put(&mut w, contact(0.7, "A", "B", "Spine1"));
    put(&mut w, attack(0.75, "B", ce::MOVE_LEFT_STRIKE as i64));
    w.run_until(0.76);
    let t_hit = cur(&w, "B").attack().unwrap().windup_end + 0.05;
    put(&mut w, contact(t_hit, "B", "A", "Spine1"));
    w.run_until(t_hit + DT);
    let t_a2 = snapped(cur(&w, "A").end_time, DT) + 0.05;
    put(&mut w, attack(t_a2, "A", 0));
    put(&mut w, (t_a2 + 0.1, Input::Feint { who: "A".into() }));
    w.run_until(t_a2 + 0.1 + DT);
    let t_a3 = snapped(cur(&w, "A").end_time, DT) + 0.05;
    put(&mut w, attack(t_a3, "A", 0));
    w.run_until(t_a3 + DT);
    let t_kill = cur(&w, "A").attack().unwrap().windup_end + 0.05;
    put(&mut w, contact(t_kill, "A", "B", "Head"));
    put(&mut w, attack(t_kill + 0.1, "B", 0));
    let t_end = t_kill + 1.0;
    w.run_until(t_end);
    (w, sched, t_end)
}

fn who_of(i: &Input) -> String {
    match i {
        Input::Attack { who, .. } | Input::Feint { who } | Input::Parry { who, .. } | Input::ReleaseBlock { who } | Input::SwitchMode { who } | Input::ToggleMode { who } | Input::Contact { who, .. } => who.clone(),
        Input::Call(_) => String::new(),
    }
}

/// client inputs go to the owner's client world, contacts to the server
fn replay(s: &mut NetSession<World>, sched: &[(f64, Input)]) {
    for (t, i) in sched {
        if matches!(i, Input::Contact { .. }) {
            s.server.world.at(*t, i.clone());
        } else {
            let w = who_of(i);
            s.client(&w).unwrap().world.at(*t, i.clone());
        }
    }
}

fn check_machines(s: &NetSession<World>, rw: &World, how: &str) {
    let want = timeline(rw);
    let mut ms: Vec<(String, &World)> = vec![("server".into(), &s.server.world)];
    for c in &s.clients {
        ms.push((format!("client {}", c.own), &c.world));
    }
    for (n, w) in ms {
        if let Some(d) = diff(&want, &timeline(w)) {
            panic!("{how}: {n} timeline differs from the server-only run {d}");
        }
        assert_eq!(outcome(w), outcome(rw), "{how}: {n} outcome");
    }
}

#[test]
fn net_duel_matches_server_only_run() {
    let sp = need_spec!();
    let (rw, sched, t_end) = reference_bout(&sp);
    let hits = |w: &World| -> Vec<String> {
        w.hits
            .iter()
            .filter(|e| matches!(e["kind"].as_str(), Some("hit" | "parry" | "died")))
            .map(|e| format!("{:.3} {} {}>{}", e["t"].as_f64().unwrap(), e["kind"].as_str().unwrap(), e.get("attacker").or(e.get("who")).and_then(|v| v.as_str()).unwrap_or(""), e.get("victim").and_then(|v| v.as_str()).unwrap_or("")))
            .collect()
    };
    let want_hits = hits(&rw);
    assert!(want_hits.len() >= 4, "reference bout has too few outcomes: {want_hits:?}");
    let mut s = session(&sp, 0, LS);
    replay(&mut s, &sched);
    s.run_until(t_end);
    if let Some(d) = diff(&want_hits, &hits(&s.server.world)) {
        panic!("server hits differ from the server-only run {d}");
    }
    assert_eq!(s.server.rejected, 0, "server rejected client motions");
    check_machines(&s, &rw, "loopback");
    for c in &s.clients {
        assert_eq!(processed_hits(&c.world), 0, "client {} processed a hit itself", c.own);
        // what each client sees of the parry: A in Blocked from 0.7 (the server's tick)
        assert!(timeline(&c.world).iter().any(|l| l == "0.700 A Blocked" || l == "0.701 A Blocked"), "client {} never saw A Blocked at 0.7", c.own);
    }
}

/// The same bout over real UDP sockets on 127.0.0.1: the rules unchanged, the transport swapped.
#[test]
fn net_duel_over_udp() {
    let sp = need_spec!();
    let (rw, sched, t_end) = reference_bout(&sp);
    let t = UdpLocal::start(2).expect("bind 127.0.0.1 UDP sockets");
    let sp2 = sp.clone();
    let mut s = NetSession::with_transport(DT, 0, Box::new(move || new_world(sp2.clone(), DT)), Box::new(t));
    s.add_player("A", LS, "");
    s.add_player("B", LS, "");
    replay(&mut s, &sched);
    s.run_until(t_end);
    check_machines(&s, &rw, "udp");
    assert!(s.transport.stats().0 >= 10);
}

/// With latency the clients still converge on the server's outcome (who was hit, who died, final health).
#[test]
fn net_duel_latency_clients_converge() {
    let sp = need_spec!();
    let (_, sched, t_end) = reference_bout(&sp);
    let mut s = session(&sp, 20, LS);
    replay(&mut s, &sched);
    s.run_until(t_end + 0.1);
    let want = outcome(&s.server.world);
    let sd = s.server.world.deaths();
    for c in &s.clients {
        assert_eq!(outcome(&c.world), want, "client {}", c.own);
        assert_eq!(c.world.deaths(), sd, "client {} deaths", c.own);
    }
}

// ---- the golden property over every combat scenario -----------------------------------------------------------------
/// Every scenario of core/tests/scenarios/combat.json (the golden combat traces' inputs), played networked with zero
/// latency: each fighter's own inputs on its owner's client (prediction), contacts and server-forced motions on the
/// server. Server and every client end tick-identical (motion timeline, deaths, health/stamina) to the server-only run.
/// Scenarios that poke state directly ("set", "equip_right", "weapon_no_drop": host test hooks with no replication)
/// are not network play and are listed as skipped.
/// scenarios that are tick-identical over the net (zero latency) to the server-only run: must stay so
const IDENTICAL: &[&str] = &["strike_hit_combo", "strike_hit_120hz", "morph_stab_and_kick", "parry_riposte_trade", "tdm_friendly_hit"];
/// scenarios with the same events at the same ticks whose order inside one tick differs between machines, as in the
/// exe: a client applies what the server replicated in UNetDriver::TickDispatch at the start of a frame and runs its
/// own pawn's input inside the tick groups, so a remote pawn's motion and its own pawn's motion of the same server
/// tick are ordered by arrival, not by the server's actor order (see session.rs, UWorld::Tick rva=0x31b0df0)
const SAME_TICK_ORDER: &[&str] = &["trade_order_ab", "trade_order_ba", "bastard_switch_vs_axe"];

#[test]
fn every_combat_scenario_tick_identical_over_net() {
    let sp = need_spec!();
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(root().join("core/tests/scenarios/combat.json")).unwrap()).unwrap();
    let alias = |x: &str| doc["weapons"][x].as_str().unwrap_or(x).to_string();
    let (mut ran, mut skipped, mut fails, mut report) = (vec![], vec![], vec![], vec![]);
    for sc in doc["scenarios"].as_array().unwrap() {
        let name = sc["name"].as_str().unwrap();
        let inputs = sc["inputs"].as_array().unwrap();
        if inputs.iter().any(|e| matches!(e["kind"].as_str(), Some("set" | "equip_right" | "weapon_no_drop"))) {
            skipped.push(name.to_string());
            continue;
        }
        let dt = sc["dt"].as_f64().unwrap_or(0.001);
        let until = sc["until"].as_f64().unwrap();
        let mode = sc.get("mode").map(|m| (m["game_mode"].as_str().unwrap().to_string(), m["game_state"].as_str().unwrap().to_string()));
        let fighters: Vec<(String, String, String, i64)> = sc["fighters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["name"].as_str().unwrap().into(), alias(f["weapon"].as_str().unwrap()), f["left"].as_str().map(|l| alias(l)).unwrap_or_default(), f["team"].as_i64().unwrap_or(255)))
            .collect();
        let mk = {
            let sp = sp.clone();
            let mode = mode.clone();
            move || {
                let mut w = new_world(sp.clone(), dt);
                if let Some((gm, gs)) = &mode {
                    w.set_game_mode(gm, gs);
                }
                w
            }
        };
        let mut rw = mk();
        for (n, wp, l, team) in &fighters {
            let i = rw.add_fighter(n, wp, l);
            rw.fighters[i].team = *team;
        }
        let mut s = NetSession::new(dt, 0, Box::new(mk.clone()));
        for (n, wp, l, _) in &fighters {
            s.add_player(n, wp, l);
        }
        for (n, _, _, team) in &fighters {
            s.server.world.set_team(n, *team);
            for c in s.clients.iter_mut() {
                c.world.set_team(n, *team);
            }
        }
        for e in inputs {
            let t = e["t"].as_f64().unwrap();
            let who = e["who"].as_str().unwrap_or("").to_string();
            let i = |k: &str, d: i64| e[k].as_i64().unwrap_or(d);
            let fv = |k: &str| e[k].as_f64().unwrap_or(0.0);
            let (input, on_server) = match e["kind"].as_str().unwrap() {
                "attack" => (Input::Attack { who: who.clone(), mv: i("move", 0), angle: fv("angle") }, false),
                "feint" => (Input::Feint { who: who.clone() }, false),
                "parry" => (Input::Parry { who: who.clone(), bt: i("bt", 0) }, false),
                "release_block" => (Input::ReleaseBlock { who: who.clone() }, false),
                "switch_mode" => (Input::SwitchMode { who: who.clone() }, false),
                "toggle_mode" => (Input::ToggleMode { who: who.clone() }, false),
                "request_parry" => (Input::Call(Call::RequestParry { who: who.clone(), bt: i("bt", 0), ftp: e["ftp"].as_bool().unwrap_or(true) }), false),
                "preset_attack" => (Input::Call(Call::PresetAttack { who: who.clone(), mv: i("move", 0), angle: fv("angle") }), false),
                "attack_now" => (Input::Call(Call::AttackNow { who: who.clone(), mv: i("move", 0), angle: fv("angle") }), false),
                "contact" => (Input::Contact { who: who.clone(), target: e["target"].as_str().unwrap().into(), bone: e["bone"].as_str().unwrap_or("Spine1").into() }, true),
                "blocked" => (Input::Call(Call::Blocked { who: who.clone(), reason: i("reason", 0), flags: i("flags", 0), time: fv("time") }), true),
                k => panic!("scenario {name}: input {k}"),
            };
            rw.at(t, input.clone());
            if on_server {
                s.server.world.at(t, input);
            } else {
                s.client(&who).unwrap().world.at(t, input);
            }
        }
        rw.run_until(until);
        s.run_until(until);
        let want = timeline(&rw);
        if std::env::var("MH_NET_DUMP").ok().as_deref() == Some(name) {
            // debugging aid: the trace log of the server-only run, the server and each client
            eprintln!("--- {name} server-only
{}", rw.events.join("
"));
            eprintln!("--- {name} server
{}", s.server.world.events.join("
"));
            for c in &s.clients {
                eprintln!("--- {name} client {}
{}", c.own, c.world.events.join("
"));
            }
        }
        let hd = |w: &World| w.fighters.iter().map(|f| format!("{} {} dead {}", f.name, f.health, f.dead)).collect::<Vec<_>>().join(", ");
        let per_tick = |v: &[String]| {
            let mut v = v.to_vec();
            v.sort(); // same events per timestamp, order inside one tick ignored (lines start with the time)
            v
        };
        let mut class = "identical";
        let mut notes = vec![];
        if s.server.rejected != 0 {
            notes.push(format!("server rejected {}", s.server.rejected));
        }
        let mut ms: Vec<(String, &World)> = vec![("server".into(), &s.server.world)];
        for c in &s.clients {
            ms.push((format!("client {}", c.own), &c.world));
        }
        for (n, w) in &ms {
            // health and deaths always converge on the server's (replicated every change)
            if hd(w) != hd(&s.server.world) {
                fails.push(format!("{name}: {n} health/deaths {} vs server {}", hd(w), hd(&s.server.world)));
            }
            let got = timeline(w);
            if got != want {
                if per_tick(&got) == per_tick(&want) {
                    if class == "identical" {
                        class = "same ticks, same-tick order differs";
                    }
                } else {
                    class = "differs";
                    notes.push(format!("{n} {}", diff(&want, &got).unwrap()));
                }
            }
            if outcome(w) != outcome(&rw) {
                notes.push(format!("{n} ends {} (server-only {})", outcome(w), outcome(&rw)));
                if class == "identical" {
                    class = "differs";
                }
            }
        }
        if IDENTICAL.contains(&name) && class != "identical" {
            fails.push(format!("{name}: was tick-identical over the net, now {class}: {notes:?}"));
        }
        if SAME_TICK_ORDER.contains(&name) && class == "differs" {
            fails.push(format!("{name}: was same-tick-order only, now {class}: {notes:?}"));
        }
        report.push(format!("{name}: {class} {notes:?}"));
        if class == "identical" {
            ran.push(name.to_string());
        }
    }
    eprintln!("net golden property over the combat scenarios ({} identical):
  {}
  skipped (state pokes): {:?}", ran.len(), report.join("
  "), skipped);
    assert!(fails.is_empty(), "net scenario failures:
{}", fails.join("
"));
}

// ---- the specific rules (test_net.gd) --------------------------------------------------------------------------------
/// OnServerAssignNetMotion: only the next Id is accepted, and only when the client had seen the server's latest motion
/// (LastAuthObserved == NetMotion.Id) or the server's latest motion was the client's own last accepted one.
#[test]
fn server_assign_net_motion_id_rules() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    let w = &mut s.server.world;
    let a = fi(w, "A");
    {
        let mut p = w.net_pawn("A").unwrap();
        let mut rep = p.take_rep().unwrap();
        let mut nm = FNetMotion::new(ce::NET_PARRY, 0, 0, 0, 0);
        nm.id = 5;
        assert!(!rep.on_server_assign_net_motion(&mut p, nm, 0), "Id 5 accepted after Id 0");
        nm.id = 1;
        assert!(rep.on_server_assign_net_motion(&mut p, nm, 0), "Id 1 with LastAuthObserved 0 refused");
        assert_eq!(rep.last_accepted_client_motion_id, 1);
        let mut n2 = FNetMotion::new(ce::NET_FEINTED, 0, 0, 0, 0);
        n2.id = 2;
        assert!(rep.on_server_assign_net_motion(&mut p, n2, 0), "Id 2 after the client's own accepted Id 1 refused");
        p.put_rep(rep);
    }
    assert_eq!(w.fighters[a].net.id, 2);
    w.assign_net_flinched(a, 0.0, false, 1.0, 1.0);
    assert_eq!(w.fighters[a].net.id, 3, "server motion Id");
    let mut p = w.net_pawn("A").unwrap();
    let mut rep = p.take_rep().unwrap();
    let mut n4 = FNetMotion::new(ce::NET_PARRY, 0, 0, 0, 0);
    n4.id = 4;
    assert!(!rep.on_server_assign_net_motion(&mut p, n4, 2), "prediction made without seeing the server's Id 3 accepted");
    assert!(rep.on_server_assign_net_motion(&mut p, n4, 3), "Id 4 with LastAuthObserved 3 refused");
    p.put_rep(rep);
}

/// AssignNetMotion on the authority: ReplicatedNetMotion (COND_SkipOwner) goes to the other client as a property and to
/// the owner only as ClientSetNetMotion.
#[test]
fn replicated_net_motion_skip_owner() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    s.run_until(0.05);
    let a = fi(&s.server.world, "A");
    s.server.world.assign_net_parry(a, 0);
    let t = s.transport.as_mut();
    s.server.replicate(t);
    let pick = |m: &Msg| m.who() == "A" && (matches!(m, Msg::ClientSetNetMotion { .. }) || m.name() == enums::PROP_REPLICATED_NET_MOTION);
    let to_owner: Vec<Msg> = t.receive(1).into_iter().map(|e| e.1).filter(|m| pick(m)).collect();
    let to_other: Vec<Msg> = t.receive(2).into_iter().map(|e| e.1).filter(|m| pick(m)).collect();
    assert!(to_owner.len() == 1 && matches!(to_owner[0], Msg::ClientSetNetMotion { .. }), "owner got {to_owner:?}");
    assert!(to_other.len() == 1 && to_other[0].name() == enums::PROP_REPLICATED_NET_MOTION, "other got {to_other:?}");
    let (Msg::ClientSetNetMotion { nm, .. }, Msg::Prop { v: mh_net::layout::PropValue::NetMotion(pv), .. }) = (&to_owner[0], &to_other[0]) else { panic!() };
    assert!(nm == pv && pv[1] == ce::NET_PARRY && pv[0] == 1);
}

/// The owning client predicts its attack at once, sends ServerAssignNetMotion, and the server's ClientSetNetMotion of
/// the same motion only confirms it (no second motion object); the simulated proxy gets it at the same server tick.
#[test]
fn prediction_confirmed_not_restarted() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    let (t, i) = attack(0.1, "A", 0);
    s.client("A").unwrap().world.at(t, i);
    s.run_until(0.1);
    let ca = &s.clients[0];
    let a = fi(&ca.world, "A");
    let m = ca.world.fighters[a].motion;
    assert!(cur(&ca.world, "A").attack().is_some() && near(cur(&ca.world, "A").start_time, 0.1, DT * 0.5), "client A did not predict at 0.1");
    let sl = &ca.world.fighters[a].net_slot;
    assert!(sl.is_initiated_locally(m) && sl.is_confirmed(m), "prediction not confirmed");
    s.run_until(0.2);
    let ca = &s.clients[0];
    assert_eq!(ca.world.fighters[a].motion, m, "the server's confirmation restarted the predicted attack");
    assert_eq!(timeline(&ca.world).iter().filter(|l| l.ends_with("A Attack")).count(), 1);
    assert!(near(cur(&s.server.world, "A").start_time, 0.1, DT * 0.5));
    let pb = &s.clients[1].world;
    assert!(cur(pb, "A").attack().is_some() && near(cur(pb, "A").start_time, 0.1, DT * 0.5));
    let pa = fi(pb, "A");
    assert!(!pb.fighters[pa].net_slot.is_initiated_locally(pb.fighters[pa].motion), "proxy motion counts as locally initiated");
}

/// Two predictions in flight (attack 0.1, feint 0.19; 50 ms each way): the feint pushes the unconfirmed attack into
/// UnconfirmedMotionsBacklog; the attack's confirmation (0.2) is consumed there; the feint's own (0.29) confirms it.
#[test]
fn backlog_consumes_superseded_confirmation() {
    let sp = need_spec!();
    let mut s = session(&sp, 50, LS);
    {
        let ca = s.client("A").unwrap();
        let (t, i) = attack(0.1, "A", 0);
        ca.world.at(t, i);
        ca.world.at(0.19, Input::Feint { who: "A".into() });
    }
    s.run_until(0.19);
    let ca = &s.clients[0];
    let a = fi(&ca.world, "A");
    assert!(cur(&ca.world, "A").feinted().is_some(), "client A did not predict the feint");
    let feint = ca.world.fighters[a].motion;
    let types: Vec<u8> = ca.rep("A").unwrap().backlog.iter().map(|n| n.motion_type).collect();
    assert_eq!(types, vec![ce::NET_ATTACK, 0], "backlog newest first");
    s.run_until(0.21);
    let ca = &s.clients[0];
    assert_eq!(ca.rep("A").unwrap().backlog.len(), 1);
    assert_eq!(ca.world.fighters[a].motion, feint, "attack confirmation replaced the feint");
    assert!(!ca.world.fighters[a].net_slot.is_confirmed(feint), "feint confirmed early");
    s.run_until(0.3);
    let ca = &s.clients[0];
    assert_eq!(ca.world.fighters[a].motion, feint, "feint ended before its confirmation: test timing");
    let sl = &ca.world.fighters[a].net_slot;
    assert!(sl.is_confirmed(feint) && ca.rep("A").unwrap().backlog.is_empty());
    assert!(near(sl.confirmed_by_authority_time, 0.29, DT * 0.5), "confirmed at {}", sl.confirmed_by_authority_time);
    assert!(cur(&s.server.world, "A").feinted().is_some());
    assert_eq!(s.server.rejected, 0);
}

/// A motion the server forces (B parries A's attack -> A Blocked) replaces A's own predicted attack on A's client.
#[test]
fn server_motion_overrides_prediction() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    let (t, i) = attack(0.1, "A", 0);
    s.client("A").unwrap().world.at(t, i);
    let (t, i) = parry(0.5, "B");
    s.client("B").unwrap().world.at(t, i);
    let (t, i) = contact(0.7, "A", "B", "Spine1");
    s.server.world.at(t, i);
    s.run_until(0.75);
    let w = &s.clients[0].world;
    let m = cur(w, "A");
    let b = m.blocked().expect("client A not Blocked");
    assert!(b.reason == ce::BLOCKED_PARRY as i64 && near(m.start_time, 0.7, DT * 0.5), "Blocked reason {} from {}", b.reason, m.start_time);
    let a = fi(w, "A");
    assert!(w.fighters[a].net_slot.is_confirmed(w.fighters[a].motion), "a server motion must arrive confirmed");
}

/// first tick at which `who` is in its parry's Recovery stage
fn parry_recovery_time(w: &World, who: &str) -> Option<f64> {
    let m = cur(w, who);
    m.parry().filter(|p| p.stage == 1).map(|_| w.now)
}

/// ServerDropParry (Fists, held parry): the client that releases block sends the RPC with its NetMotion Id; the server
/// drops the parry at the tick the client asked; it never drops a remote player's held parry on its own.
#[test]
fn server_drop_parry_rpc() {
    let sp = need_spec!();
    let mut rf = reference(&sp, &[("F", FISTS), ("H", FISTS)]);
    rf.at(0.1, Input::Parry { who: "F".into(), bt: 0 });
    rf.at(0.15, Input::ReleaseBlock { who: "F".into() });
    rf.at(0.1, Input::Parry { who: "H".into(), bt: 0 });
    let mut want = None;
    while rf.now < 1.5 - DT * 0.5 {
        rf.step();
        if want.is_none() {
            want = parry_recovery_time(&rf, "F");
        }
    }
    let want = want.expect("reference F never dropped its parry");
    let sp2 = sp.clone();
    let mut s = NetSession::new(DT, 0, Box::new(move || new_world(sp2.clone(), DT)));
    s.add_player("F", FISTS, "");
    s.add_player("H", FISTS, "");
    let f0 = fi(&s.server.world, "F");
    assert!(s.server.world.fighters[f0].weapon.as_ref().unwrap().b_is_parry_held, "Fists bIsParryHeld false");
    {
        let cf = s.client("F").unwrap();
        cf.world.at(0.1, Input::Parry { who: "F".into(), bt: 0 });
        cf.world.at(0.15, Input::ReleaseBlock { who: "F".into() });
    }
    s.client("H").unwrap().world.at(0.1, Input::Parry { who: "H".into(), bt: 0 });
    let (mut got, mut seen) = (None, None);
    while s.server.world.now < 1.5 - DT * 0.5 {
        s.step();
        if got.is_none() {
            got = parry_recovery_time(&s.server.world, "F");
        }
        if seen.is_none() {
            seen = parry_recovery_time(&s.clients[0].world, "F");
        }
    }
    assert!(got.is_some_and(|g| near(g, want, DT * 0.5)), "server drop at {got:?}, server-only run {want}");
    assert!(seen.is_some_and(|t| t <= want + DT + 1e-6), "client F saw its drop at {seen:?}");
    let h = cur(&s.server.world, "H");
    assert!(h.parry().is_some_and(|p| p.stage == 0), "server dropped H's held parry by itself");
    let hi = fi(&s.server.world, "H");
    let id = (s.server.world.fighters[hi].net.id + 1) & 0xff;
    s.server.world.server_drop_parry_implementation(hi, id);
    assert!(cur(&s.server.world, "H").parry().is_some_and(|p| p.stage == 0), "stale MotionID dropped the parry");
}

/// OffsetStamina: only the authority applies it; the client gets the change through ReplicatedStamina.
#[test]
fn stamina_offsets_on_authority_only() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    let st0 = s.server.world.fighters[fi(&s.server.world, "A")].stamina;
    let ca = &mut s.clients[0];
    let a = fi(&ca.world, "A");
    ca.world.offset_stamina(a, -30);
    assert_eq!(ca.world.fighters[a].stamina, st0, "client applied a stamina offset itself");
    let a = fi(&s.server.world, "A");
    s.server.world.offset_stamina(a, -30);
    s.step();
    for c in &s.clients {
        assert_eq!(c.world.fighters[fi(&c.world, "A")].stamina, st0 - 30, "client {} stamina from ReplicatedStamina", c.own);
    }
}

// ---- ping compensation and the remaining properties (test_net_ping.gd) -----------------------------------------------
const LAT: u64 = 50;

/// A motion the server forces on A (a flinch) reaches A's own client 50 ms late with ExpectedDelay = PingMedian x
/// TimeDilation = 0.1 there and ends that much earlier; the server and the simulated proxy keep ExpectedDelay 0.
#[test]
fn expected_delay_owner_only() {
    let sp = need_spec!();
    let mut s = session(&sp, LAT, LS);
    s.run_until(0.2);
    assert!(near(s.clients[0].get_ping(), 0.1, 1e-7), "client A PingMedian {}", s.clients[0].get_ping());
    server_at(&mut s, 0.3, |w| {
        let a = fi(w, "A");
        w.assign_net_flinched(a, 0.0, false, 1.0, 1.0);
    });
    s.run_until(0.45);
    let sm = cur(&s.server.world, "A");
    let om = cur(&s.clients[0].world, "A");
    let pm = cur(&s.clients[1].world, "A");
    for (n, m) in [("server", sm), ("owner", om), ("proxy", pm)] {
        assert!(m.is_flinch(), "{n} A is {}", m.kind());
    }
    assert!(near(sm.start_time, 0.3, DT * 0.5) && near(om.start_time, 0.35, DT * 0.5) && near(pm.start_time, 0.35, DT * 0.5), "starts {} {} {}", sm.start_time, om.start_time, pm.start_time);
    assert!(sm.expected_delay == 0.0 && pm.expected_delay == 0.0);
    assert!(near(om.expected_delay, 0.1, 1e-7), "owner ExpectedDelay {}", om.expected_delay);
    assert!(near((sm.end_time - sm.start_time) - (om.end_time - om.start_time), 0.1, 1e-5));
}

/// A attacks from Idle at 0.3 on its own client (ping 0.1): Param2 = 40 (f32: 0.08 x 500); server (starts 0.35)
/// Windup = W - 0.08, no LagInduction; owner (0.3) LagInduction = 0.1 x (1 - |AngleTarget| x 0.5), MissRecovery - it;
/// proxy on B (0.4) LagInduction = -0.05; the server's and B's windup ends coincide.
#[test]
fn attack_ping_compensation() {
    let sp = need_spec!();
    let mut rf = reference(&sp, &[("A", LS), ("B", LS)]);
    let (t, i) = attack(0.3, "A", 0);
    rf.at(t, i);
    rf.run_until(0.31);
    let ra = cur(&rf, "A").attack().unwrap();
    let (w, mr) = (ra.ai.windup, ra.ai.miss_recovery);
    let mut s = session(&sp, LAT, LS);
    let (t, i) = attack(0.3, "A", 0);
    s.client("A").unwrap().world.at(t, i);
    s.run_until(0.45);
    let (sw, ow, pw) = (&s.server.world, &s.clients[0].world, &s.clients[1].world);
    let (sm, om, pm) = (cur(sw, "A"), cur(ow, "A"), cur(pw, "A"));
    let (sa, oa, pa) = (sm.attack().expect("server"), om.attack().expect("owner"), pm.attack().expect("proxy"));
    assert_eq!(sw.fighters[fi(sw, "A")].net.param2, 40, "Param2");
    assert!(near(sm.start_time, 0.35, DT * 0.5) && near(pm.start_time, 0.4, DT * 0.5), "starts server {} proxy {}", sm.start_time, pm.start_time);
    assert!(sa.lag_induction == 0.0 && sa.lag_reduction == 0.0);
    assert!(near(sa.ai.windup, w - 0.08, 1e-5), "server windup {} want {}", sa.ai.windup, w - 0.08);
    let li = f32r(0.1 * (1.0 - oa.angle_target.abs() * 0.5));
    assert!(near(oa.lag_induction, li, 1e-6), "owner LagInduction {} want {li}", oa.lag_induction);
    assert_eq!(oa.lag_reduction, 0.0, "owner LagReduction for its own attack");
    assert!(near(oa.ai.windup, w - 0.08 + li, 1e-5), "owner windup {}", oa.ai.windup);
    assert!(near(oa.ai.miss_recovery, mr - li, 1e-5), "owner MissRecovery {}", oa.ai.miss_recovery);
    assert!(near(pa.lag_induction, -0.05, 1e-6), "proxy LagInduction {}", pa.lag_induction);
    assert!(near(sa.windup_end, pa.windup_end, 1e-5), "windup ends server {} proxy {}", sa.windup_end, pa.windup_end);
}

/// GetAttackCompensationStartTime: during a flinch (bCanAttack false) the compensation is 0; in the Idle after it the
/// Idle asks its ComingFromMotion: the time since the flinch's EndTime, capped by the ping.
#[test]
fn attack_compensation_start_time() {
    let sp = need_spec!();
    let mut s = session(&sp, LAT, LS);
    s.run_until(0.2);
    server_at(&mut s, 0.25, |w| {
        let a = fi(w, "A");
        w.assign_net_flinched(a, 0.0, false, 1.0, 1.0);
    });
    s.run_until(0.32);
    let ow = &s.clients[0].world;
    let a = fi(ow, "A");
    let f = cur(ow, "A");
    assert!(f.is_flinch() && !f.b_can_attack, "owner A is {}", f.kind());
    let fid = ow.fighters[a].motion;
    assert_eq!(NetRules.attack_ping_compensation(ow, a, 0), 0.0, "compensation during the flinch");
    let e = f.end_time;
    s.run_until(snapped(e, DT) + 0.03);
    let ow = &s.clients[0].world;
    let idle = cur(ow, "A");
    assert!(idle.kind() == "Idle" && idle.coming_from == fid, "after the flinch A is {}", idle.kind());
    let c = NetRules.attack_ping_compensation(ow, a, 0);
    assert!(near(c, ow.now - e, 1e-5), "compensation {c} want {}", ow.now - e);
    let t = s.server.world.now + 0.2;
    s.run_until(t);
    let ow = &s.clients[0].world;
    assert!(near(NetRules.attack_ping_compensation(ow, a, 0), 0.1, 1e-6), "not capped by the ping");
}

/// OnDynamicParamChanged on the owning client: a hit ends the attack at ReleaseEnd + HitRecovery - LagInduction; the
/// server keeps the full HitRecovery.
#[test]
fn hit_recovery_owner_minus_lag_induction() {
    let sp = need_spec!();
    let mut s = session(&sp, LAT, LS);
    let (t, i) = attack(0.3, "A", 0);
    s.client("A").unwrap().world.at(t, i);
    s.run_until(0.36);
    let t_hit = snapped(cur(&s.server.world, "A").attack().unwrap().windup_end, DT) + 0.03;
    let (t, i) = contact(t_hit, "A", "B", "Spine1");
    s.server.world.at(t, i);
    s.run_until(t_hit + 0.15);
    let om = cur(&s.clients[0].world, "A");
    let sm = cur(&s.server.world, "A");
    let (oa, sa) = (om.attack().expect("owner"), sm.attack().expect("server"));
    assert!(oa.b_has_hit && sa.b_has_hit);
    let hr = sm.def.attack().hit_recovery;
    assert!(near(sm.end_time - sa.release_end, hr, 1e-5), "server recovery {}", sm.end_time - sa.release_end);
    assert!(oa.lag_induction > 0.0);
    let want = (om.def.attack().hit_recovery - oa.lag_induction).max(0.0);
    assert!(near(om.end_time - oa.release_end, want, 1e-5), "owner recovery {} want {want}", om.end_time - oa.release_end);
}

/// With 50 ms each way and ping compensation on, the bout still ends the same on all three machines.
#[test]
fn latency_bout_all_machines_agree() {
    let sp = need_spec!();
    let mut s = session(&sp, LAT, LS);
    let (t, i) = attack(0.3, "A", 0);
    s.client("A").unwrap().world.at(t, i);
    let (t, i) = parry(0.75, "B");
    s.client("B").unwrap().world.at(t, i);
    s.run_until(0.76);
    let t_hit = snapped(cur(&s.server.world, "A").attack().unwrap().windup_end, DT) + 0.03;
    let (t, i) = contact(t_hit, "A", "B", "Head");
    s.server.world.at(t, i);
    s.run_until(t_hit + 0.5);
    let o = |w: &World| format!("A {} B {}", w.fighters[fi(w, "A")].health, w.fighters[fi(w, "B")].health);
    let want = o(&s.server.world);
    for c in &s.clients {
        assert_eq!(o(&c.world), want, "client {}", c.own);
    }
    assert_eq!(s.server.rejected, 0);
}

#[test]
fn suggest_hit_detection() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    let (t, i) = attack(0.1, "A", 0);
    s.client("A").unwrap().world.at(t, i);
    s.run_until(0.11);
    let we = cur(&s.server.world, "A").attack().unwrap().windup_end;
    s.run_until(snapped(we, DT) + 0.02);
    assert_eq!(cur(&s.server.world, "A").attack().unwrap().stage, 1, "server A not in Release");
    let t = s.transport.as_mut();
    assert!(!s.clients[0].cosmetic_hit(t, "B", "Spine1"), "suggested a hit on a standing victim");
    s.clients[0].rep_mut("B").unwrap().ragdoll_falling = true;
    s.server.rep_mut("B").unwrap().ragdoll_falling = true;
    let sw = &s.server.world;
    let (a, b) = (fi(sw, "A"), fi(sw, "B"));
    let h0 = sw.fighters[b].health;
    let q = sw.qf();
    let want = damage::compute(&sw.spec.constants, &sw.fighters[b], &cur(sw, "A").attack().unwrap().ai, "Spine1", q);
    assert!(s.clients[0].cosmetic_hit(s.transport.as_mut(), "B", "Spine1"), "no suggestion for a ragdolling victim");
    s.step();
    let sw = &s.server.world;
    let got: Vec<&serde_json::Map<String, Value>> = sw.hits.iter().filter(|e| e.get("suggested").is_some()).collect();
    assert!(got.len() == 1 && near(got[0]["damage"].as_f64().unwrap(), want, 1e-4), "server suggested hits {got:?}, want one of {want}");
    assert!(want <= 0.0 || sw.fighters[b].health < h0, "B not damaged");
    assert!(!cur(sw, "B").is_flinch(), "a suggested hit flinched B");
    assert!(sw.fighters[a].net.dynamic_param & 0x10 != 0 && !cur(sw, "A").attack().unwrap().b_has_hit, "dyn {}", sw.fighters[a].net.dynamic_param);
    assert!(!sw.fighters[b].dead, "test needs B alive after one Spine1 hit");
    s.clients[0].cosmetic_hit(s.transport.as_mut(), "B", "Spine1");
    s.step();
    assert_eq!(s.server.rep("A").unwrap().rejected_suggestions, 1, "repeat suggestion not rejected");
    s.clients[1].send_server_rpc(s.transport.as_mut(), &Msg::ServerSuggestHitDetection { who: "A".into(), other: "B".into(), bone: "Spine1".into() });
    assert_eq!(s.clients[1].dropped_rpcs, 1, "client B sent an RPC for A's pawn");
    s.step();
    assert_eq!(s.server.not_owner, 0);
}

/// ReplicatedLookUpValue (COND_SimulatedOnly), ReplicatedCharacterFlags bit 0, ReplicatedTeam
#[test]
fn look_up_flags_team_replicate() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    let a = fi(&s.server.world, "A");
    let (up, down) = (s.server.world.fighters[a].character.look_up_limit, s.server.world.fighters[a].character.look_down_limit);
    assert!(up + down > 0.0, "look limits {up} / {down}");
    let ca = fi(&s.clients[0].world, "A");
    s.clients[0].world.fighters[ca].look_up_value = 7.0;
    s.server.world.fighters[a].look_up_value = 30.0;
    s.server.world.fighters[a].airborne = true;
    let ep = s.server.owner_of["A"];
    s.server.player_states.get_mut(&ep).unwrap().set_team(1, true);
    s.step();
    let byte = (f32r(f32r(f32r(f32r(30.0 + down) / f32r(up + down)).clamp(0.0, 1.0)) * 255.0)) as i64;
    assert_eq!(s.server.rep("A").unwrap().replicated_look_up_value as i64, byte);
    let pw = &s.clients[1].world;
    let want = (up + down) * (byte as f64 / 255.0) - down;
    assert!(near(pw.fighters[fi(pw, "A")].look_up_value, want, 1e-3), "proxy look up {}", pw.fighters[fi(pw, "A")].look_up_value);
    assert_eq!(s.clients[0].world.fighters[ca].look_up_value, 7.0, "owner received its own look-up");
    for c in &s.clients {
        let f = &c.world.fighters[fi(&c.world, "A")];
        assert!(f.airborne && f.team == 1, "client {}: airborne {} team {}", c.own, f.airborne, f.team);
    }
}

/// NetBlock / ReplicatedKnockback (UParryMotion::ReceiveBlock rva=0x166bf90): B parries A's strike at 0.7 s and at
/// 7.7 s; OnRep_NetBlock does nothing while World TimeSeconds <= 7, so only the second block is applied (on every
/// machine); ReplicatedKnockback counts both; the server's Knockback amount is -Forward x KnockbackParry.
#[test]
fn net_block_and_knockback_replicate() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    let kp = s.server.world.fighters[fi(&s.server.world, "B")].character.knockback_parry;
    assert!(kp > 0.1, "KnockbackParry {kp}");
    for t0 in [0.0, 7.0] {
        let (t, i) = attack(t0 + 0.1, "A", 0);
        s.client("A").unwrap().world.at(t, i);
        let (t, i) = parry(t0 + 0.5, "B");
        s.client("B").unwrap().world.at(t, i);
        let (t, i) = contact(t0 + 0.7, "A", "B", "Spine1");
        s.server.world.at(t, i);
        s.run_until(t0 + 0.75);
        let n: u8 = if t0 == 0.0 { 1 } else { 2 };
        let sb = s.server.rep("B").unwrap();
        assert_eq!(sb.net_block.version, n, "server NetBlock Version at {t0}");
        assert!(sb.net_block.reason == ce::BLOCKED_PARRY && sb.net_block.mv == ce::MOVE_RIGHT_STRIKE, "{:?}", sb.net_block);
        assert_eq!(sb.replicated_knockback, n);
        let want_applied = if t0 == 0.0 { 0 } else { 1 };
        for r in [s.server.rep("B").unwrap(), s.clients[0].rep("B").unwrap(), s.clients[1].rep("B").unwrap()] {
            assert_eq!(r.net_blocks_applied.len(), want_applied, "applied NetBlocks at {t0}");
            assert_eq!(r.replicated_knockback, n);
        }
        s.run_until(t0 + 3.0);
    }
    let sb = s.server.rep("B").unwrap();
    assert_eq!(sb.knockbacks.len(), 2);
    assert!(near(sb.knockbacks[0][0] as f64, -kp, 1e-3), "server knockbacks {:?}", sb.knockbacks);
    for r in [s.server.rep("B").unwrap(), s.clients[0].rep("B").unwrap(), s.clients[1].rep("B").unwrap()] {
        let b = &r.net_blocks_applied[0];
        assert!(b.result.reason == ce::BLOCKED_PARRY && b.mv == ce::MOVE_RIGHT_STRIKE && !b.result.ranged && !b.result.stun);
        assert_eq!(r.last_net_block_version, 2);
    }
    let mut p = s.server.world.net_pawn("B").unwrap();
    let mut r = p.take_rep().unwrap();
    r.on_rep_net_block(&p);
    assert_eq!(r.net_blocks_applied.len(), 1, "a repeated Version was applied again");
    assert!(!r.knockback(&p, [0.1, 0.0, 0.0]), "Knockback of 0.1 accepted");
    p.put_rep(r);
    let mut cp = s.clients[1].world.net_pawn("B").unwrap();
    let mut cr = cp.take_rep().unwrap();
    assert!(!cr.knockback(&cp, [500.0, 0.0, 0.0]), "a client applied Knockback");
    cp.put_rep(cr);
}

/// AssignNetBlock packs FBlockResult into Flags bit by bit, the OnRep reads it back
#[test]
fn net_block_flags_round_trip() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    s.run_until(7.01);
    {
        let mut p = s.server.world.net_pawn("A").unwrap();
        let mut r = p.take_rep().unwrap();
        r.assign_net_block(&p, &BlockResult { reason: ce::BLOCKED_CLASH, clash_on_parry: true, party: true, surface: 3, ..Default::default() }, ce::MOVE_STAB, "B");
        assert_eq!(r.net_block.flags, 0x50);
        let b = r.net_blocks_applied.last().unwrap();
        assert!(b.result.clash_on_parry && b.result.party && !b.result.stun && b.result.surface == 3 && b.actor == "B");
        p.put_rep(r);
    }
    s.step();
    let pb = s.clients[1].rep("A").unwrap().net_blocks_applied.last().unwrap().clone();
    assert!(pb.result.reason == ce::BLOCKED_CLASH && pb.mv == ce::MOVE_STAB && pb.result.clash_on_parry);
}

/// A's client dodges along UE (-1, -1): PackedWorldYaw 159; the server stores 159 and pays DodgeStaminaCost; B's
/// proxy gets ReplicatedDodge (COND_SimulatedOnly); a repeat dodge stores 160.
#[test]
fn dodge_replicates() {
    let sp = need_spec!();
    let mut s = session(&sp, 0, LS);
    s.run_until(0.1);
    let a = fi(&s.server.world, "A");
    let cost = s.server.world.fighters[a].character.dodge_stamina_cost;
    let st0 = s.server.world.fighters[a].stamina;
    let packed = s.clients[0].request_dodge(s.transport.as_mut(), [-1.0, -1.0]).unwrap();
    assert_eq!(packed, 159);
    let d = s.clients[0].rep("A").unwrap().dodges.clone();
    assert!(d.len() == 1 && near(d[0].1, -135.0, 1e-3), "owner dodges {d:?}");
    s.step();
    let want_yaw = NetMotionRep::unwind_dodge(f32r(159.0 * mh_net::consts::DODGE_BYTE_TO_DEG as f64));
    let sa = s.server.rep("A").unwrap();
    assert!(sa.replicated_dodge == 159 && sa.dodges.len() == 1 && near(sa.dodges[0].1, want_yaw, 1e-4), "server {} {:?}", sa.replicated_dodge, sa.dodges);
    let sf = &s.server.world.fighters[a];
    if cost > 0 {
        assert_eq!(sf.stamina, (st0 - cost).max(sf.stamina_stat.min_value), "server stamina");
    }
    let pa = s.clients[1].rep("A").unwrap();
    assert!(pa.replicated_dodge == 159 && pa.dodges.len() == 1);
    let oa = s.clients[0].rep("A").unwrap();
    assert!(oa.replicated_dodge == 0 && oa.dodges.len() == 1, "owner got its own ReplicatedDodge");
    s.clients[0].request_dodge(s.transport.as_mut(), [-1.0, -1.0]);
    s.step();
    let pa = s.clients[1].rep("A").unwrap();
    assert!(s.server.rep("A").unwrap().replicated_dodge == 160 && pa.replicated_dodge == 160 && pa.dodges.len() == 2);
    if cost > 0 {
        assert_eq!(pa.replicated_stamina, s.server.rep("A").unwrap().replicated_stamina);
    }
}

// ---- the networked Duel match (test_net_match.gd) on real combat + mh-mode's DuelMode ----------------------------------
fn duel_mode() -> Option<mh_mode::GameMode> {
    let k = std::fs::read_to_string(root().join("godot/data_gen/mode/mode_kismet.json")).ok()?;
    let g = std::fs::read_to_string(root().join("core/tests/golden/mode/duel_rooms.jsonl")).ok()?;
    let mut header: Value = serde_json::from_str(g.lines().nth(1)?).ok()?;
    mordhau_core::data::decode_exact(&mut header);
    let data: mh_mode::ModeData = serde_json::from_value(header["data"].clone()).expect("ModeData");
    Some(mh_mode::GameMode::new(data, std::sync::Arc::new(mh_mode::Kismet::from_json(&k).expect("kismet")), mordhau_core::ue::CrtRand::new(1)))
}

fn play_match(sp: &Rc<Spec>, latency: u64) -> Option<(NetSession<World>, NetDuelMatch<mh_mode::GameMode>, bool)> {
    const MDT: f64 = 1.0 / 60.0;
    let mode = duel_mode()?;
    let play = match &mode.d.ext {
        mh_mode::data::ModeDataExt::Duel(d) => d.stage["RoundPlay"],
        _ => panic!("not DU"),
    };
    let sp2 = sp.clone();
    let mut s = NetSession::new(MDT, latency, Box::new(move || new_world(sp2.clone(), MDT)));
    let mut m = NetDuelMatch::new(mode, LS, "");
    let ca = m.join(&mut s, "A", "").unwrap();
    let cb = m.join(&mut s, "B", "").unwrap();
    let winners = ["A", "B", "A", "A", "B", "A", "A"];
    let mut hit_attack: Option<(String, f64)> = None;
    let mut finished = false;
    for _ in 0..(240.0 / MDT) as u32 {
        let won = m.mode_events.iter().filter(|e| matches!(e, ModeEvent::Other { kind, .. } if kind == "round_won")).count();
        let w = winners[won.min(winners.len() - 1)];
        let l = if w == "A" { "B" } else { "A" };
        // the round's winner plays from its own client: attack whenever its pawn is idle in RoundPlay
        let wc = s.client_by_id(if w == "A" { ca } else { cb }).unwrap();
        let in_play = wc.pc_props.get(ROOM_GAME).is_some_and(|r| r["round"]["stage"].as_i64() == Some(play));
        if in_play && wc.own == w {
            if let Some(i) = wc.world.fighter_index(w) {
                if wc.world.cur_m(i).map(|m| m.kind() == "Idle").unwrap_or(false) {
                    wc.world.input(Input::Attack { who: w.into(), mv: 0, angle: 0.0 });
                }
            }
        }
        // the server's hit detection: each of the winner's attacks lands once on the loser's head while in Release
        let sw = &s.server.world;
        if let (Some(wi), Some(_)) = (sw.fighter_index(w), sw.fighter_index(l)) {
            let m0 = sw.cur_m(wi).unwrap();
            if let Some(a) = m0.attack() {
                let key = (w.to_string(), m0.start_time);
                if a.stage == 1 && hit_attack.as_ref() != Some(&key) {
                    hit_attack = Some(key);
                    let t = sw.now + MDT;
                    s.server.world.at(t, Input::Contact { who: w.into(), target: l.into(), bone: "Head".into() });
                }
            }
        }
        m.step(&mut s);
        if m.mode_events.iter().any(|e| matches!(e, ModeEvent::Other { kind, .. } if kind == "match_finished")) {
            finished = true;
            for _ in 0..latency + 2 {
                m.step(&mut s);
            }
            break;
        }
    }
    Some((s, m, finished))
}

fn check_match(s: &NetSession<World>, m: &NetDuelMatch<mh_mode::GameMode>, finished: bool) {
    let wins: Vec<i64> = m
        .mode_events
        .iter()
        .filter_map(|e| match e {
            ModeEvent::Other { kind, data } if kind == "round_won" => data["winner"].as_i64(),
            _ => None,
        })
        .collect();
    assert!(finished, "match did not finish; round winners {wins:?}");
    assert_eq!(wins, vec![0, 1, 0, 0, 1, 0, 0], "round winners (teams)");
    for c in &s.clients {
        let rg = c.pc_props.get(ROOM_GAME).expect("room game");
        assert_eq!((rg["team1_wins"].as_i64(), rg["team2_wins"].as_i64()), (Some(5), Some(2)), "client {} room game", c.id);
    }
    let sd = s.server.world.deaths();
    assert_eq!(sd, vec!["B", "A", "B", "B", "A", "B", "B"], "server deaths");
    for c in &s.clients {
        assert_eq!(c.world.deaths(), sd, "client {} deaths", c.id);
        assert_eq!(processed_hits(&c.world), 0, "client {} processed a hit", c.id);
        assert_eq!(c.dropped_rpcs, 0);
    }
    assert_eq!((s.server.rejected, s.server.not_owner), (0, 0));
}

#[test]
fn net_duel_match_to_five_loopback() {
    let sp = need_spec!();
    let Some((s, m, f)) = play_match(&sp, 0) else {
        eprintln!("SKIP: duel mode data missing");
        return;
    };
    check_match(&s, &m, f);
}

#[test]
fn net_duel_match_to_five_latency() {
    let sp = need_spec!();
    let Some((mut s, m, f)) = play_match(&sp, 3) else { return };
    check_match(&s, &m, f);
    for c in s.clients.iter_mut() {
        assert!((c.get_ping() - 2.0 * 3.0 / 60.0).abs() < 1e-6, "client {} PingMedian {}", c.id, c.get_ping());
    }
}

/// Climbing over the net (rust-net r5): the owning client's RequestClimb is AssignNetMotion(FNetMotion::Climbing, type
/// 0x1b): it predicts the UClimbingMotion at once, the server creates it from ServerAssignNetMotion with the same
/// params (the climb offset bytes, bIsSlowClimb), the other client sees it through ReplicatedNetMotion; the movement
/// side reads it through NetWorld::net_climb_motion; when it ends every machine leaves it
#[test]
fn climbing_replicates_as_a_net_motion() {
    let sp = need_spec!();
    let mut s = session(&sp, 30, LS);
    s.run_until(0.1);
    assert!(s.client("A").unwrap().world.net_request_climb("A", [20.0, -10.0, 120.0], false));
    let ca = &s.clients[0];
    let own = ca.world.net_climb_motion("A").expect("client A did not predict the climb");
    assert_eq!(own.params[3], 0, "bIsSlowClimb byte");
    assert!(own.params[2] > 100 && own.params[0] > 128 && own.params[1] < 128, "offset bytes {:?}", own.params);
    assert!(ca.world.net_motion_blocks_climb("A"));
    s.run_until(0.2);
    let srv = s.server.world.net_climb_motion("A").expect("the server did not start the climb");
    assert_eq!(srv.params, own.params, "server params differ from the prediction");
    assert!(near(srv.start_time as f64, 0.1 + 0.03, 2.0 * DT), "server climb start {}", srv.start_time);
    assert_eq!(s.server.rejected, 0);
    s.run_until(0.3);
    let other = s.clients[1].world.net_climb_motion("A").expect("client B does not see A climbing");
    assert_eq!(other.params, own.params);
    let ca = &s.clients[0];
    let a = fi(&ca.world, "A");
    let m = ca.world.fighters[a].motion;
    assert!(ca.world.fighters[a].net_slot.is_initiated_locally(m) && ca.world.fighters[a].net_slot.is_confirmed(m), "prediction not confirmed");
    // EndTime = StartTime + AuthorityMoveLateralStartTime 0.25 + ClimbRecoveryDuration 0.5 + AuthorityMoveLateralDuration
    // 0.5 (UClimbingMotion ctor rva=0x16457d0)
    s.run_until(1.0);
    assert!(s.server.world.net_climb_motion("A").is_some(), "the climb ended early on the server");
    s.run_until(2.0);
    assert!(s.server.world.net_climb_motion("A").is_none() && s.clients[0].world.net_climb_motion("A").is_none() && s.clients[1].world.net_climb_motion("A").is_none(), "a machine is still climbing");
}

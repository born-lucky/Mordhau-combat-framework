//! Closed loop: the bots (mh-mode ai) driving mordhau-core's exe-exact combat World through `CombatHost`, and a Duel
//! match (mh-mode DU) run to completion by bots. Records: core/tests/golden/spec.json (combat), the bots_duel golden
//! header (BOTBEHAVIOR_Knight profile + BT_Deathmatch tree, as the package readers resolved them) and the duel_rooms
//! header (DU mode data). Triternion data, git-ignored: without them the tests say so and pass.
//!
//! Hit detection stand-in (TEST SCAFFOLDING, not a game rule): there is no level / weapon geometry here, so an attack
//! that enters Release reaches its target: the test schedules one Input::Contact (the combat World's scripted contact,
//! processed like a traced hit: ProcessHitForBlocking / ForDamage decide parry, chamber, damage) per attack motion.
//! The trace-backed version lives in rust-combat's mh-sim (core/crates/mh-sim/tests/sim.rs
//! `bots_fight_with_real_traces`: two Knight bots through the same CombatHost, attacks swept through the physics-asset
//! bodies, no scripted contacts, a kill at about 2 s). It can't move here: mh-sim depends on mh-mode, and these tests
//! stay the crate-local check of the bot <-> World seam without level / weapon geometry.

use mh_mode::ai::{Profile, TreeDef};
use mh_mode::data::ModeData;
use mh_mode::game_mode::{GameMode, MatchState};
use mh_mode::kismet::Kismet;
use mh_mode::sim::BotSim;
use mordhau_core::combat::world::Input;
use mordhau_core::combat::World;
use mordhau_core::data::{decode_exact, RecordsJsonExe, SpecSource};
use mordhau_core::ue::{CrtRand, FVector};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn header(name: &str) -> Option<Value> {
    let t = std::fs::read_to_string(root().join("core/tests/golden/mode").join(name)).ok()?;
    let mut v: Value = serde_json::from_str(t.lines().nth(1)?).ok()?;
    decode_exact(&mut v);
    Some(v)
}

struct Recs {
    world: World,
    k: Arc<Kismet>,
    profile: Profile,
    tree: TreeDef,
    walk: f64,
}


/// the exe-mode data path (r4): the spec matrix (data_gen/spec, mh-spec) when present; the golden dumps otherwise
fn spec() -> Option<&'static mh_spec::Spec> {
    static S: std::sync::OnceLock<Option<mh_spec::Spec>> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data_gen/spec");
        if d.join("index.json").exists() { mh_spec::Spec::load(&d, false).ok() } else { None }
    })
    .as_ref()
}

fn recs() -> Option<Recs> {
    let spec = std::fs::read_to_string(root().join("core/tests/golden/spec.json")).ok()?;
    let h = header("bots_duel.jsonl")?;
    let spec = Rc::new(RecordsJsonExe(&spec).load_spec().expect("spec.json"));
    // bots + literals: the spec matrix when present (exe-mode data path), else the dumps
    let (k, profile, tree) = match self::spec() {
        Some(s) => (
            mh_mode::spec_bots::kismet(s).ok()?,
            mh_mode::spec_bots::profile(s, "BOTBEHAVIOR_Knight").ok()?,
            mh_mode::spec_bots::tree(s, "BT_Deathmatch").ok()?,
        ),
        None => (
            Kismet::from_json(&std::fs::read_to_string(root().join("godot/data_gen/mode/mode_kismet.json")).ok()?).ok()?,
            serde_json::from_value(h["bots"][0]["profile"].clone()).unwrap(),
            serde_json::from_value(h["bots"][0]["tree"].clone()).unwrap(),
        ),
    };
    Some(Recs { world: World::new(spec, 1.0 / 60.0), k: Arc::new(k), profile, tree, walk: h["bodies"][0]["max_walk_speed"].as_f64().unwrap() })
}

fn skip(what: &str) {
    if std::env::var("MH_GOLDEN_REQUIRED").is_ok() {
        panic!("{what} missing");
    }
    eprintln!("SKIP: {what} missing");
}

/// the test's hit stand-in (header): one contact per attack motion, on its first Release frame
fn contacts(sim: &mut BotSim, done: &mut HashSet<(usize, i64)>) {
    let n = sim.fighter_of.len();
    for b in 0..n {
        let v = &sim.bots.bodies[b].pawn.motion;
        if !v.is_attack || v.stage != 1 || sim.bots.bodies[b].is_dead || !done.insert((b, v.id)) {
            continue;
        }
        let target = (0..n).find(|&o| o != b && !sim.bots.bodies[o].is_dead);
        if let Some(t) = target {
            let (who, tgt) = (sim.world.fighters[sim.fighter_of[b]].name.clone(), sim.world.fighters[sim.fighter_of[t]].name.clone());
            sim.world.input(Input::Contact { who, target: tgt, bone: "Spine1".into() });
        }
    }
}

// two BT_Deathmatch Knight bots, 150 cm apart, fight until one dies
#[test]
fn two_bots_fight_to_a_kill() {
    let Some(r) = recs() else { return skip("spec.json / bots_duel / mode_kismet") };
    let mut sim = BotSim::new(r.world, r.k.clone());
    let a = sim.world.add_fighter("A", LS, "");
    let b = sim.world.add_fighter("B", LS, "");
    let ba = sim.add_body(a, FVector::new(0.0, 0.0, 0.0), 0.0, 8.0, 0, r.walk);
    let bb = sim.add_body(b, FVector::new(150.0, 0.0, 0.0), 180.0, 8.0, 0, r.walk);
    sim.add_bot(ba, r.profile.clone(), Some(&r.tree));
    sim.add_bot(bb, r.profile.clone(), Some(&r.tree));
    let mut done = HashSet::new();
    let mut attacks = 0;
    for _ in 0..(120 * 60) {
        sim.step();
        contacts(&mut sim, &mut done);
        attacks = done.len();
        if sim.world.fighters.iter().any(|f| f.dead) {
            break;
        }
    }
    let dead: Vec<&str> = sim.world.fighters.iter().filter(|f| f.dead).map(|f| f.name.as_str()).collect();
    eprintln!("closed loop: {} attacks reached release, dead {:?} at t={:.2}", attacks, dead, sim.world.now);
    assert_eq!(dead.len(), 1, "one fighter dies");
    assert!(attacks >= 2);
    assert!(sim.bots.bodies.iter().any(|b| b.is_dead));
}

// a Duel (BP_DuelGameMode) match to completion: two bot controllers logged in like players, their pawns spawned into
// the combat World on the mode's possess events and removed on destroy_pawn (RestartRoom), deaths reported to
// OnKilled; RoundsToWin wins finish the room (FinishRoomGame, room destroyed)
#[test]
fn duel_match_with_bots_to_completion() {
    let Some(r) = recs() else { return skip("spec.json / bots_duel / mode_kismet") };
    let Some(dh) = header("duel_rooms.jsonl") else { return skip("duel_rooms golden") };
    let data: ModeData = match spec() {
        Some(s) => ModeData::from_spec(s, "DU").expect("DU from spec"),
        None => serde_json::from_value(dh["data"].clone()).unwrap(),
    };
    let rounds_to_win = match &data.ext {
        mh_mode::data::ModeDataExt::Duel(d) => d.rounds_to_win,
        _ => unreachable!(),
    };
    let mut mode = GameMode::new(data, r.k.clone(), CrtRand::new(1));
    let names = ["A", "B"];
    let ctrls: Vec<usize> = names.iter().map(|n| mode.login(n, true, -1)).collect();
    let mut sim = BotSim::new(r.world, r.k.clone());
    // one body per controller, bound to a fighter while the controller has a pawn
    let mut bodies: Vec<Option<usize>> = vec![None, None];
    let mut killed_reported: HashSet<String> = HashSet::new();
    let mut done = HashSet::new();
    let mut finished = false;
    let dt = sim.world.dt;
    for _ in 0..(900 * 60) {
        mode.tick(dt);
        for e in mode.drain() {
            let who = e.get("who").map(|a| a.as_s().to_string()).unwrap_or_default();
            match e.kind {
                "possess" => {
                    let i = names.iter().position(|n| *n == who).unwrap();
                    if let Some(fi) = sim.world.fighter_index(&who) {
                        let _ = fi;
                        sim.world.remove_fighter(&who);
                    }
                    let fi = sim.world.add_fighter(&who, LS, "");
                    killed_reported.remove(&who);
                    match bodies[i] {
                        None => {
                            let loc = if i == 0 { FVector::new(0.0, 0.0, 0.0) } else { FVector::new(150.0, 0.0, 0.0) };
                            let b = sim.add_body(fi, loc, if i == 0 { 0.0 } else { 180.0 }, 8.0, mode.ctrls[ctrls[i]].team, r.walk);
                            sim.add_bot(b, r.profile.clone(), Some(&r.tree));
                            bodies[i] = Some(b);
                        }
                        Some(_) => {}
                    }
                }
                "destroy_pawn" => {
                    if sim.world.fighter_index(&who).is_some() {
                        sim.world.remove_fighter(&who);
                    }
                }
                "match_finished" => finished = true,
                _ => {}
            }
        }
        // fighter indices shift on removal: rebind every body by name
        for (i, b) in bodies.iter().enumerate() {
            if let Some(b) = *b {
                match sim.world.fighter_index(names[i]) {
                    Some(fi) => sim.fighter_of[b] = fi,
                    None => sim.bots.bodies[b].is_dead = true,
                }
            }
        }
        if finished {
            break;
        }
        let all_bound = bodies.iter().all(|b| b.is_some()) && names.iter().all(|n| sim.world.fighter_index(n).is_some());
        if !all_bound {
            continue;
        }
        sim.world.step();
        sim.refresh();
        // input is blocked in the room's RoundStart stage (BP_DuelGameState ShouldBlockPawnInput): those bots skip
        // their AI frame
        let (dtw, now) = (sim.world.dt, sim.world.now);
        for i in 0..sim.bots.bots.len() {
            let ci = ctrls[i];
            if mode.should_block_input(ci) {
                continue;
            }
            let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut sim.world, fighter_of: sim.fighter_of.clone() };
            sim.bots.with_bot(i, now, &mut host, |c, cx| c.tick(dtw, cx));
        }
        contacts(&mut sim, &mut done);
        // deaths -> OnKilled (the other fighter as the killer)
        for (i, n) in names.iter().enumerate() {
            let dead = sim.world.fighter_index(n).map(|fi| sim.world.fighters[fi].dead).unwrap_or(false);
            if dead && killed_reported.insert(n.to_string()) {
                let killer = ctrls[1 - i];
                mode.on_killed(Some(killer), Some(ctrls[i]), 0, "Longsword", false);
            }
        }
    }
    assert!(finished, "the room's game finished");
    let won: Vec<f64> = ctrls.iter().map(|&c| mode.ctrls[c].score).collect();
    eprintln!("duel: scores {:?} at t={:.1}", won, mode.now);
    assert!(won.iter().any(|&s| s >= 100000.0)); // FinishRoomGame AddScore(100000)
    assert!(mode.duel().rooms.is_empty() || mode.duel().rooms.iter().all(|r| r.state == 2));
    assert_eq!(mode.match_state, MatchState::InProgress); // a Duel server keeps running rooms
    let _ = rounds_to_win;
}

// perfect parry (UBTTask_MeleeDefend rva=0x14945e0 region: bWillPerfectParry +0xdc -> parry delay 0 instead of
// ParryTimingRandom +0xe0): a Knight profile with PerfectParryProbability 1 and no footwork defends every threat with a
// zero delay
#[test]
fn perfect_parry_defends_without_delay() {
    let Some(r) = recs() else { return skip("spec.json / bots_duel / mode_kismet") };
    let mut sim = BotSim::new(r.world, r.k.clone());
    let a = sim.world.add_fighter("A", LS, "");
    let b = sim.world.add_fighter("B", LS, "");
    let ba = sim.add_body(a, FVector::new(0.0, 0.0, 0.0), 0.0, 8.0, 0, r.walk);
    let bb = sim.add_body(b, FVector::new(150.0, 0.0, 0.0), 180.0, 8.0, 0, r.walk);
    let mut perfect = r.profile.clone();
    perfect.perfect_parry_probability = 1.0;
    perfect.footwork_instead_of_parry_probability = 0.0;
    sim.add_bot(ba, r.profile.clone(), Some(&r.tree));
    sim.add_bot(bb, perfect, Some(&r.tree));
    for c in sim.bots.bots.iter_mut().flatten() {
        c.record_trace = true;
    }
    let mut done = HashSet::new();
    for _ in 0..(20 * 60) {
        sim.step();
        contacts(&mut sim, &mut done);
        if sim.world.fighters.iter().any(|f| f.dead) {
            break;
        }
    }
    let notes: Vec<String> = sim.bots.bot(1).trace.iter().filter(|n| n.task == "MeleeDefend" && n.step.contains("threat")).map(|n| n.step.clone()).collect();
    eprintln!("B defended {} threats; first: {:?}", notes.len(), notes.first());
    assert!(!notes.is_empty(), "B saw a threat");
    assert!(notes.iter().all(|s| s.contains("perfect=true delay=0.0000")), "{notes:?}");
}

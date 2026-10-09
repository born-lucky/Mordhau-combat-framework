//! Port of godot/tests/test_net_match.gd onto mh-mode's real GameMode (BP_DuelGameMode, ModeData + Blueprint bytecode
//! literals as the mode golden traces carry them): a server-authoritative Duel match with two clients. The mode runs on
//! the server only; each client sees pawns spawn and die, its team and its room's round state only through
//! replication. Rounds are scripted: in RoundPlay (as its own ReplicatedRoomGame says) the round's winner attacks from
//! its client and the server's hit detection lands every attack while it is in Release, until the loser dies.
//! Winners A B A A B A A: A wins 5-2. The combat world is the tests/support fixture until mordhau-core's World
//! implements NetWorld (then this file runs on it unchanged). Data is read from ignored paths
//! (core/tests/golden/mode/duel_rooms.jsonl header, godot/data_gen/mode/mode_kismet.json); missing -> SKIP.

mod support;

use std::path::Path;
use std::sync::Arc;

use mh_mode::data::{ModeData, ModeDataExt};
use mh_mode::game_mode::GameMode;
use mh_mode::kismet::Kismet;
use mh_net::duel_match::{ModeEvent, NetDuelMatch, ROOM_GAME};
use mh_net::{NetSession, NetWorld};
use mordhau_core::ue::CrtRand;
use support::{Input, Kind, ToyWorld};

const DT: f64 = 1.0 / 60.0;

fn duel_mode() -> Option<GameMode> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let k = std::fs::read_to_string(root.join("godot/data_gen/mode/mode_kismet.json")).ok()?;
    let g = std::fs::read_to_string(root.join("core/tests/golden/mode/duel_rooms.jsonl")).ok()?;
    // present data must load: only absent files skip
    let mut header: serde_json::Value = serde_json::from_str(g.lines().nth(1).expect("header")).expect("header json");
    mordhau_core::data::decode_exact(&mut header); // the exporter's exact "f64:<hex>" floats
    let data: ModeData = serde_json::from_value(header["data"].clone()).expect("ModeData");
    assert!(matches!(data.ext, ModeDataExt::Duel(_)) && data.ok, "duel_rooms data not a DU mode / not ok");
    Some(GameMode::new(data, Arc::new(Kismet::from_json(&k).expect("kismet")), CrtRand::new(1)))
}

fn round_play(m: &GameMode) -> i64 {
    match &m.d.ext {
        ModeDataExt::Duel(d) => d.stage["RoundPlay"],
        _ => unreachable!(),
    }
}

fn play(latency: u64) -> Option<(NetSession<ToyWorld>, NetDuelMatch<GameMode>, bool)> {
    let mode = duel_mode()?;
    let play_stage = round_play(&mode);
    let mut s: NetSession<ToyWorld> = NetSession::new(DT, latency, Box::new(|| ToyWorld::new(DT)));
    let mut m = NetDuelMatch::new(mode, "LS", "");
    let ca = m.join(&mut s, "A", "").unwrap();
    let cb = m.join(&mut s, "B", "").unwrap();
    let winners = ["A", "B", "A", "A", "B", "A", "A"];
    let mut hit_attack: Option<(String, u64)> = None;
    let mut finished = false;
    for _ in 0..(240.0 / DT) as u32 {
        // each client plays the round its own room game shows, while its stage is RoundPlay
        for (cid, me) in [(ca, "A"), (cb, "B")] {
            let c = s.client_by_id(cid).unwrap();
            let Some(rg) = c.pc_props.get(ROOM_GAME) else { continue };
            let seen = (rg["team1_wins"].as_i64().unwrap_or(0) + rg["team2_wins"].as_i64().unwrap_or(0)) as usize;
            if rg["round"]["stage"].as_i64() == Some(play_stage)
                && winners[seen.min(winners.len() - 1)] == me
                && c.world.pawn(me).is_some_and(|p| p.motion.kind == Kind::Idle && !p.dead)
            {
                let t = c.world.now + DT;
                c.world.at(t, Input::Attack(me.into()));
            }
        }
        // the server's hit detection: each of the winner's attacks lands once on the other pawn while in Release
        let now = s.server.world.now;
        for (w, l) in [("A", "B"), ("B", "A")] {
            if let Some(sw) = s.server.world.pawn(w) {
                let rel = sw.motion.kind == Kind::Attack && sw.motion.stage(now) == 1;
                let key = (w.to_string(), sw.motion.serial);
                if rel && s.server.world.pawn(l).is_some() && hit_attack.as_ref() != Some(&key) {
                    hit_attack = Some(key);
                    s.server.world.at(now + DT, Input::Contact(w.into(), l.into()));
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

fn check(s: &NetSession<ToyWorld>, m: &NetDuelMatch<GameMode>, finished: bool) {
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
        assert!(c.world.hits.is_empty(), "client {} processed a hit", c.id);
        assert_eq!(c.dropped_rpcs, 0);
    }
    assert_eq!((s.server.rejected, s.server.not_owner), (0, 0));
}

#[test]
fn net_duel_match_to_five_loopback() {
    let Some((s, m, f)) = play(0) else {
        eprintln!("SKIP: duel mode data missing (core/tests/golden/mode, godot/data_gen/mode)");
        return;
    };
    check(&s, &m, f);
}

#[test]
fn net_duel_match_to_five_latency() {
    let Some((mut s, m, f)) = play(3) else { return };
    check(&s, &m, f);
    for c in s.clients.iter_mut() {
        assert!((c.get_ping() - 2.0 * 3.0 * DT).abs() < 1e-6, "client {} PingMedian {}", c.id, c.get_ping());
    }
}

//! Port of godot/tests/test_net_session.gd (the server's login gate and session, the beacon a client uses to ping a
//! server or reserve its party's slots, the login / logout path through NetSession) and of test_net_match.gd's
//! networked match glue. Each test names the exe functions whose rules it checks (rva=...). The match tests drive a
//! FIXTURE mode (ToyMode below, NOT BP_DuelGameMode: mh-mode's DuelMode plugs into `ServerMode` once it lands).

mod support;

use std::collections::VecDeque;

use mh_net::beacon::{BeaconClient, BeaconHost};
use mh_net::duel_match::{ModeEvent, NetDuelMatch, ServerMode};
use mh_net::enums::*;
use mh_net::game_session::{entity_key, get_int_option, parse_option, NetGameSession};
use mh_net::msg::Entity;
use mh_net::transport::{Loopback, Transport};
use mh_net::{NetSession, NetWorld};
use support::{Input, Kind, ToyWorld};

const DT: f64 = 1.0 / 60.0;

fn ent(id: &str) -> Entity {
    Entity { id: id.into(), r#type: 1 }
}

/// UGameplayStatics::ParseOption rva=0x30e3ab0, GetIntOption rva=0x30d6f60
#[test]
fn url_options() {
    let o = "?Name=A?spectatoronly=1?MaxPlayers=24?Flag";
    assert_eq!(parse_option(o, "SpectatorOnly"), "1", "key compare must ignore case");
    assert_eq!(parse_option(o, "Flag"), "", "an option without '=' has no value");
    assert_eq!(get_int_option(o, "MaxPlayers", 3), 24);
    assert_eq!(get_int_option(o, "Missing", 7), 7);
    assert_eq!(get_int_option("?Flag", "Flag", 5), 5, "empty value -> default");
}

/// AMordhauGameSession::PostInitProperties rva=0x15a35b0, AGameSession::InitOptions rva=0x30df0e0, the ini config
#[test]
fn session_config() {
    let mut g = NetGameSession::new();
    assert_eq!((g.max_players, g.max_spectators, g.max_splitscreens_per_connection), (16, 0, 1), "DefaultGame.ini GameSession");
    g.post_init_properties(0);
    assert_eq!((g.max_slots, g.max_players), (16, 16));
    g.admin_slots = 2;
    g.post_init_properties(24);
    assert_eq!((g.max_slots, g.max_players), (24, 26));
    g.post_init_properties(-3);
    assert_eq!(g.max_slots, 16);
    g.init_options("?MaxPlayers=8?MaxSpectators=2");
    assert_eq!((g.max_players, g.max_spectators), (8, 2));
    assert!(g.allows_join() && g.admin_slots == 2);
}

/// AMordhauGameSession::ApproveLogin rva=0x1584680 -> AGameSession::ApproveLogin rva=0x30ca8e0, AtCapacity
/// rva=0x30cb0c0 (AGameMode::GetNumPlayers rva=0x30d7800, GetNumSpectators rva=0x30d78a0), AGameModeBase::PreLogin
/// rva=0x30e5ae0, AGameMode::PostLogin rva=0x30e4ff0 / Logout rva=0x30e2310
#[test]
fn approve_login_rules() {
    let mut g = NetGameSession::new();
    g.post_init_properties(0);
    g.max_players = 2;
    assert_eq!(g.pre_login("", true), "", "empty server refuses");
    g.post_login(false, false);
    g.post_login(false, true); // a travelling player counts too
    assert_eq!(g.get_num_players(), 2);
    assert_eq!(g.approve_login("?Name=C"), "Server full.");
    g.max_players_override = 3;
    assert_eq!(g.approve_login(""), "", "net.MaxPlayersOverride 3 > MaxPlayers");
    g.max_players_override = 0;
    g.logout(false, false);
    assert_eq!(g.approve_login(""), "", "after a logout there is room");
    g.max_players = 0;
    assert_eq!(g.approve_login(""), "", "MaxPlayers 0 = no limit");
    // spectators: MaxSpectators 0 (DefaultGame.ini) -> a spectator is refused on a dedicated server ...
    assert_eq!(g.approve_login("?SpectatorOnly=1"), "Server full.");
    // ... and on an empty listen server let in
    let mut l = NetGameSession::new();
    l.net_mode = NET_MODE_LISTEN_SERVER;
    assert_eq!(l.approve_login("?SpectatorOnly=1"), "");
    l.post_login(false, false);
    assert_eq!(l.approve_login("?SpectatorOnly=1"), "Server full.");
    let mut s = NetGameSession::new();
    s.net_mode = NET_MODE_STANDALONE;
    s.max_players = 0;
    s.num_players = 50;
    assert!(!s.at_capacity(false) && !s.at_capacity(true), "standalone is never at capacity");
    assert_eq!(g.approve_login("?SplitscreenCount=2"), "Maximum splitscreen players");
    assert_eq!(g.approve_login("?SplitscreenCount=1"), "");
    assert_eq!(g.pre_login("", false), "incompatible_unique_net_id");
}

/// AMordhauGameSession::RegisterPlayer rva=0x15a4c50 -> AGameSession::RegisterPlayer rva=0x30e7fe0 (NextPlayerID++),
/// through NetSession::connect_client / NetServer::login / logout
#[test]
fn login_gate_and_player_ids() {
    let mut s: NetSession<ToyWorld> = NetSession::new(DT, 0, Box::new(|| ToyWorld::new(DT)));
    s.server.game_session.max_players = 2;
    let a = s.connect_client("?Name=A").expect("A fits");
    let b = s.connect_client("?Name=B").expect("B fits");
    assert_eq!((s.server.player_states[&a].player_id, s.server.player_states[&b].player_id), (0, 1));
    assert_eq!(s.client_by_id(a).unwrap().player_state().player_id, 0);
    assert_eq!(s.client_by_id(b).unwrap().player_state().player_id, 1);
    assert!(s.connect_client("?Name=C").is_none());
    assert_eq!(s.last_login_error, "Server full.");
    assert_eq!(s.server.player_states.len(), 2, "a refused login made a player state");
    assert!(s.connect_client("?Name=S?SpectatorOnly=1").is_none(), "spectator with MaxSpectators 0");
    s.disconnect_client(a);
    assert_eq!(s.server.game_session.num_players, 1);
    let c = s.connect_client("?Name=C").expect("room after a logout");
    assert!(c != b && s.server.player_states[&c].player_id == 2, "new endpoint and PlayerId 2 (NextPlayerID never reused)");
}

/// AMordhauGameSession::ReserveSlot rva=0x15a6950, FreeSlot rva=0x15945a0, GetReservedSlots rva=0x1598cc0,
/// GetOpenSlots rva=0x1597ce0, AllowJoin rva=0x1582560, AllowsJoin rva=0x15825c0, HandleMatchHasEnded rva=0x1599e20,
/// IsPlayFabPlayerPresent rva=0x159c090, FPlayFabEntity::IsValid rva=0x1487dc0
#[test]
fn slot_reservations() {
    let mut g = NetGameSession::new();
    g.post_init_properties(2);
    g.playfab_time_offset = 5;
    assert!(!g.reserve_slot(&ent(""), 100) && !g.reserve_slot(&Entity { id: "x".into(), r#type: 0 }, 100), "invalid entity reserved");
    assert!(g.reserve_slot(&ent("a"), 100) && g.reserved_slots[&entity_key(&ent("a"))] == 95, "reserve a at unix - offset");
    assert!(g.reserve_slot(&ent("b"), 100));
    assert_eq!((g.get_reserved_slots(), g.get_open_slots()), (2, 0));
    assert!(!g.reserve_slot(&ent("c"), 100), "third slot of 2");
    assert!(g.reserve_slot(&ent("a"), 200) && g.reserved_slots[&entity_key(&ent("a"))] == 195, "re-reserve refreshes");
    g.free_slot(&ent("b"));
    assert_eq!(g.get_open_slots(), 1);
    g.max_slots = 0;
    assert_eq!(g.get_open_slots(), 0, "open slots never negative");
    g.max_slots = 2;
    g.set_allow_join(false);
    assert!(!g.allows_join() && !g.reserve_slot(&ent("c"), 100), "joining not allowed");
    assert!(g.reserve_slot(&ent("a"), 300), "an existing reservation is refreshed even when joining is closed");
    g.set_allow_join(true);
    g.handle_match_has_ended();
    assert!(!g.allows_join() && g.match_has_ended);
    g.present_players.insert("p".into());
    assert!(g.is_play_fab_player_present("p") && !g.is_play_fab_player_present("") && !g.is_play_fab_player_present("q"));
}

fn beacon_run(t: &mut Loopback, cl: &mut BeaconClient, host: &mut BeaconHost, frames: u32, now_unix: i64) {
    for _ in 0..frames {
        host.receive(t, now_unix);
        let now = t.frame() as f64 * DT as f64;
        cl.receive(t, now);
        t.advance();
    }
}

/// AMordhauBeaconClient::ReserveSlots rva=0x1514ec0, OnConnected rva=0x15084c0, ServerReserveSlots_Implementation
/// rva=0x15155f0, ClientNotifyReservationStatus_Implementation rva=0x14f4b80, OnFailure rva=0x1508730
#[test]
fn beacon_reserves_party() {
    let mut t = Loopback::new(2);
    let mut g = NetGameSession::new();
    g.post_init_properties(3);
    let mut c1 = BeaconClient::new(1);
    assert!(!c1.reserve_slots(&mut t, &[]), "an empty party does not connect");
    assert!(c1.reserve_slots(&mut t, &[ent("a"), ent("b")]) && c1.request == BEACON_REQUEST_RESERVE_SLOTS);
    {
        let mut host = BeaconHost::new(Some(&mut g));
        beacon_run(&mut t, &mut c1, &mut host, 12, 1000);
    }
    assert_eq!(c1.reservation_responses, vec![(1, RESERVATION_SUCCESS)]);
    // a party of 3 with 1 slot left: Full, and the ones that got in are freed again (unless already on the server)
    g.present_players.insert("d".into());
    let mut c2 = BeaconClient::new(2);
    c2.reserve_slots(&mut t, &[ent("c"), ent("d"), ent("e")]);
    {
        let mut host = BeaconHost::new(Some(&mut g));
        beacon_run(&mut t, &mut c2, &mut host, 12, 1000);
    }
    assert_eq!(c2.reservation_responses, vec![(1, RESERVATION_FULL)]);
    assert!(!g.reserved_slots.contains_key(&entity_key(&ent("c"))), "c not rolled back");
    // no game session -> Failure; a host that refuses -> OnFailure (0, Failure)
    let mut h2 = BeaconHost::new(None);
    let mut c3 = BeaconClient::new(3);
    c3.reserve_slots(&mut t, &[ent("z")]);
    beacon_run(&mut t, &mut c3, &mut h2, 12, 1000);
    assert_eq!(c3.reservation_responses, vec![(0, RESERVATION_FAILURE)]);
    let mut host = BeaconHost::new(Some(&mut g));
    host.accepting = false;
    let mut c4 = BeaconClient::new(4);
    c4.ping(&mut t);
    beacon_run(&mut t, &mut c4, &mut host, 12, 1000);
    assert!(c4.reservation_responses == vec![(0, RESERVATION_FAILURE)] && !c4.connected);
}

/// AMordhauBeaconClient::Ping rva=0x15128b0, ServerPing_Implementation rva=0x15155a0, ClientPong_Implementation
/// rva=0x14f4ba0: the ping is the ServerPing -> ClientPong round trip in whole milliseconds
#[test]
fn beacon_ping() {
    let mut t = Loopback::new(3); // 50 ms each way
    let mut host = BeaconHost::new(None);
    let mut c = BeaconClient::new(1);
    assert!(c.ping(&mut t) && c.request == BEACON_REQUEST_PING);
    beacon_run(&mut t, &mut c, &mut host, 20, 1000);
    assert_eq!(c.ping_responses, vec![100]);
    c.ping_start_time = 0.0;
    assert_eq!(c.client_pong(0.0124), 12, "RoundToInt of ms");
    assert_eq!(c.client_pong(0.0126), 13);
}

/// AMordhauGameSession::IsPlayerBanned rva=0x159c0d0, IsPlayerMuted rva=0x159c140, GetPlayerBanEndTime rva=0x1597f70,
/// GetPlayerBanDuration rva=0x1597e00, KickPlayer rva=0x159c280, BanPlayer rva=0x15846a0, UnbanPlayer rva=0x15ac950
#[test]
fn bans_and_mutes() {
    let mut g = NetGameSession::new();
    let now = 1_000_000i64;
    g.banned_players.insert("perm".into(), 0);
    g.banned_players.insert("90s".into(), now + 90);
    g.banned_players.insert("60s".into(), now + 60);
    g.official_banned_players.insert("61s".into(), now + 61);
    g.banned_players.insert("old".into(), now - 5);
    g.official_muted_players.insert("m".into(), 0);
    assert!(!g.is_player_banned("") && g.is_player_banned("61s") && !g.is_player_banned("x"));
    assert!(g.is_player_muted("m") && !g.is_player_muted("perm"));
    assert_eq!(g.get_player_ban_end_time("x"), -1);
    assert_eq!(g.get_player_ban_end_time("61s"), now + 61);
    for (id, want) in [("perm", 0), ("90s", 2), ("60s", 1), ("61s", 2), ("old", -1), ("x", -1)] {
        assert_eq!(g.get_player_ban_duration(id, now), want, "{id}");
    }
    g.playfab_time_offset = 30; // the server clock 30 s behind: 90 s ban -> 120 s -> 2 min
    assert_eq!(g.get_player_ban_duration("90s", now), 2);
    assert_eq!(g.get_player_ban_duration("60s", now), 2);
    assert!(!g.kick_player("p", "r") && !g.ban_player("p", 10, "r") && !g.unban_player("p"));
    assert_eq!(g.kick_requests, vec![("p".to_string(), "r".to_string())]);
    assert_eq!(g.ban_requests, vec![("p".to_string(), 10, "r".to_string())]);
    assert_eq!(g.unban_requests, vec!["p".to_string()]);
}

// ---- the networked match glue (fixture mode) ------------------------------------------------------------------------

/// FIXTURE game mode, NOT BP_DuelGameMode: two controllers, both pawns spawn once two have logged in (teams 0 / 1), a
/// death wins the round for the other team, 1 s later both pawns are destroyed and respawned, first to `to_win`.
struct ToyMode {
    ctrls: Vec<(String, i64, bool)>,
    events: VecDeque<ModeEvent>,
    wins: [i64; 2],
    to_win: i64,
    round_end_timer: f64,
    finished: bool,
    replicated: bool,
}

impl ToyMode {
    fn new(to_win: i64) -> Self {
        ToyMode { ctrls: vec![], events: VecDeque::new(), wins: [0, 0], to_win, round_end_timer: 0.0, finished: false, replicated: false }
    }
    fn spawn_all(&mut self) {
        for (n, team, alive) in self.ctrls.iter_mut() {
            *alive = true;
            self.events.push_back(ModeEvent::Team { who: n.clone(), team: *team });
            self.events.push_back(ModeEvent::SpawnPawn { who: n.clone() });
        }
    }
}

impl ServerMode for ToyMode {
    fn post_login(&mut self, ctrl: &str, _player_id: i64) {
        let team = self.ctrls.len() as i64;
        self.ctrls.push((ctrl.into(), team, false));
        if self.ctrls.len() == 2 {
            self.spawn_all();
            self.replicated = true;
        }
    }
    fn logout(&mut self, ctrl: &str) {
        self.ctrls.retain(|c| c.0 != ctrl);
    }
    fn tick(&mut self, dt: f64) {
        if self.round_end_timer > 0.0 {
            self.round_end_timer -= dt;
            if self.round_end_timer <= 0.0 {
                for (n, _, _) in &self.ctrls {
                    self.events.push_back(ModeEvent::DestroyPawn { who: n.clone() });
                }
                if !self.finished {
                    self.spawn_all();
                }
            }
        }
    }
    fn drain(&mut self) -> Vec<ModeEvent> {
        self.events.drain(..).collect()
    }
    fn set_alive(&mut self, ctrl: &str, alive: bool) {
        let Some(i) = self.ctrls.iter().position(|c| c.0 == ctrl) else { return };
        self.ctrls[i].2 = alive;
        if alive || self.finished {
            return;
        }
        let winner = 1 - self.ctrls[i].1;
        self.wins[winner as usize] += 1;
        self.events.push_back(ModeEvent::Other { kind: "round_won".into(), data: serde_json::json!({ "winner": winner }) });
        self.round_end_timer = 1.0;
        if self.wins[winner as usize] >= self.to_win {
            self.finished = true;
            self.events.push_back(ModeEvent::Other { kind: "match_finished".into(), data: serde_json::json!({ "winner_team": winner }) });
        }
    }
    fn team_of(&self, ctrl: &str) -> i64 {
        self.ctrls.iter().find(|c| c.0 == ctrl).map(|c| c.1).unwrap_or(-1)
    }
    fn replicated_room_game(&self, _ctrl: &str) -> Option<serde_json::Value> {
        self.replicated.then(|| serde_json::json!({ "team1_wins": self.wins[0], "team2_wins": self.wins[1], "playing": self.round_end_timer <= 0.0 && !self.finished }))
    }
}

fn play(latency: u64) -> (NetSession<ToyWorld>, NetDuelMatch<ToyMode>, bool) {
    let mut s: NetSession<ToyWorld> = NetSession::new(DT, latency, Box::new(|| ToyWorld::new(DT)));
    let mut m = NetDuelMatch::new(ToyMode::new(3), "LS", "");
    let ca = m.join(&mut s, "A", "").unwrap();
    let cb = m.join(&mut s, "B", "").unwrap();
    let winners = ["A", "B", "A", "B", "B"];
    let mut finished = false;
    let mut hit_attack: Option<(String, u64)> = None;
    for _ in 0..(120.0 / DT) as u32 {
        let won = m.mode_events.iter().filter(|e| matches!(e, ModeEvent::Other { kind, .. } if kind == "round_won")).count();
        let w = winners[won.min(winners.len() - 1)];
        let l = if w == "A" { "B" } else { "A" };
        // the round's winner plays from its own client: attack whenever its pawn is idle while its own
        // ReplicatedRoomGame says the round is on (as test_net_match.gd waits for RoundPlay); each client picks the
        // round from the wins its room game shows, so it never acts on a round it has not seen yet
        for (cid, me) in [(ca, "A"), (cb, "B")] {
            let c = s.client_by_id(cid).unwrap();
            let Some(rg) = c.pc_props.get(mh_net::duel_match::ROOM_GAME) else { continue };
            let seen = (rg["team1_wins"].as_i64().unwrap_or(0) + rg["team2_wins"].as_i64().unwrap_or(0)) as usize;
            let playing = rg["playing"].as_bool() == Some(true);
            if playing && winners[seen.min(winners.len() - 1)] == me && c.own == me && c.world.pawn(me).is_some_and(|p| p.motion.kind == Kind::Idle && !p.dead) {
                let t = c.world.now + DT;
                c.world.at(t, Input::Attack(me.into()));
            }
        }
        // the server's hit detection: each of the winner's attacks lands once on the loser while in Release
        let now = s.server.world.now;
        if let Some(sw) = s.server.world.pawn(w) {
            let rel = sw.motion.kind == Kind::Attack && sw.motion.stage(now) == 1;
            let key = (w.to_string(), sw.motion.serial);
            if rel && s.server.world.pawn(l).is_some() && hit_attack.as_ref() != Some(&key) {
                hit_attack = Some(key);
                s.server.world.at(now + DT, Input::Contact(w.into(), l.into()));
            }
        }
        m.step(&mut s);
        if m.mode_events.iter().any(|e| matches!(e, ModeEvent::Other { kind, .. } if kind == "match_finished")) {
            finished = true;
            for _ in 0..latency + 2 {
                m.step(&mut s); // the clients get the final room game
            }
            break;
        }
    }
    (s, m, finished)
}

fn check(s: &NetSession<ToyWorld>, m: &NetDuelMatch<ToyMode>, finished: bool) {
    assert!(finished, "match did not finish: {:?}", m.mode_events);
    let wins: Vec<i64> = m
        .mode_events
        .iter()
        .filter_map(|e| match e {
            ModeEvent::Other { kind, data } if kind == "round_won" => data["winner"].as_i64(),
            _ => None,
        })
        .collect();
    assert_eq!(wins, vec![0, 1, 0, 1, 1], "round winners (teams)");
    for c in &s.clients {
        let rg = c.pc_props.get(mh_net::duel_match::ROOM_GAME).expect("room game");
        assert_eq!((rg["team1_wins"].as_i64(), rg["team2_wins"].as_i64()), (Some(2), Some(3)), "client {} room game", c.id);
    }
    let sd = s.server.world.deaths();
    assert_eq!(sd, vec!["B", "A", "B", "A", "A"], "server deaths");
    for c in &s.clients {
        assert_eq!(c.world.deaths(), sd, "client {} saw deaths", c.id);
        assert!(c.world.hits.is_empty(), "client {} processed a hit", c.id);
        assert_eq!(c.dropped_rpcs, 0);
    }
    assert_eq!((s.server.rejected, s.server.not_owner), (0, 0));
}

#[test]
fn net_match_to_three_loopback() {
    let (s, m, f) = play(0);
    check(&s, &m, f);
    // teams from ReplicatedTeam on every client
    for c in &s.clients {
        for (n, team) in [("A", 0), ("B", 1)] {
            if let Some(p) = c.world.pawn(n) {
                assert_eq!(p.team, team, "client {} pawn {n}", c.id);
            }
        }
    }
}

#[test]
fn net_match_to_three_latency() {
    let (mut s, m, f) = play(3);
    check(&s, &m, f);
    // with 50 ms each way both clients measured a 0.1 s ping
    for c in s.clients.iter_mut() {
        assert!((c.get_ping() - 2.0 * 3.0 * DT).abs() < 1e-6, "client {} PingMedian {}", c.id, c.get_ping());
    }
}

/// the networked match behind the gate: a full server refuses, a leave frees the place
#[test]
fn net_match_full_server() {
    let mut s: NetSession<ToyWorld> = NetSession::new(DT, 0, Box::new(|| ToyWorld::new(DT)));
    s.server.game_session.max_players = 2;
    let mut m = NetDuelMatch::new(ToyMode::new(5), "LS", "");
    assert!(m.join(&mut s, "A", "").is_some() && m.join(&mut s, "B", "").is_some());
    assert!(m.join(&mut s, "C", "").is_none());
    assert_eq!(s.last_login_error, "Server full.");
    for _ in 0..(2.0 / DT) as u32 {
        m.step(&mut s);
    }
    assert!(s.server.rep("A").is_some(), "A's pawn spawned");
    assert!(s.clients.iter().all(|c| c.world.pawn("A").is_some() && c.world.pawn("B").is_some()), "pawns spawned on the clients");
    m.leave(&mut s, "A");
    for _ in 0..3 {
        m.step(&mut s);
    }
    assert!(s.server.rep("A").is_none(), "A's pawn stays after A left");
    assert!(s.clients.iter().all(|c| c.world.pawn("A").is_none()), "A's pawn stays on a client");
    assert!(m.join(&mut s, "C", "").is_some(), "C after A left ({})", s.last_login_error);
    let _ = Transport::stats(s.transport.as_ref());
}

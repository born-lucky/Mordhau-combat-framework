//! Horde and Battle Royale rules (bytecode ports, no GDScript reference) on the game's own records: the
//! data_hrd_br golden header (godot/tools/golden/mode.gd _run_data, Triternion data, git-ignored). Each test names the
//! bytecode statement it pins; the table values double as the oracle check of docs/HORDE_SPEC.md. Without the data the
//! tests say so and pass (MH_GOLDEN_REQUIRED=1 fails instead).

use mh_mode::data::{BrData, HordeData, ModeData, ModeDataExt};
use mh_mode::game_mode::{GameMode, MatchState};
use mh_mode::kismet::Kismet;
use mordhau_core::ue::CrtRand;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn header() -> Option<Value> {
    let t = std::fs::read_to_string(root().join("core/tests/golden/mode/data_hrd_br.jsonl")).ok()?;
    let mut v: Value = serde_json::from_str(t.lines().nth(1)?).ok()?;
    mordhau_core::data::decode_exact(&mut v);
    Some(v)
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

fn kismet() -> Option<Arc<Kismet>> {
    if let Some(s) = spec() {
        return mh_mode::spec_bots::kismet(s).ok().map(Arc::new);
    }
    let t = std::fs::read_to_string(root().join("godot/data_gen/mode/mode_kismet.json")).ok()?;
    Some(Arc::new(Kismet::from_json(&t).unwrap()))
}

fn skip() {
    if std::env::var("MH_GOLDEN_REQUIRED").is_ok() {
        panic!("data_hrd_br missing");
    }
    eprintln!("SKIP: core/tests/golden/mode/data_hrd_br.jsonl missing");
}

fn mode_data(h: &Value, mode: &str, ext: &str) -> ModeData {
    if let Some(s) = spec() {
        return ModeData::from_spec_class(s, if ext == "horde" { "HRD" } else { "BR" }).expect("HRD / BR from spec");
    }
    let mut m = h[mode].clone();
    let mut e = h[ext].clone();
    e["id"] = Value::String(if ext == "horde" { "HRD".into() } else { "BR".into() });
    m["ext"] = e;
    serde_json::from_value(m).expect("mode data")
}

fn horde(h: &Value) -> HordeData {
    if let Some(s) = spec() {
        return mh_mode::spec_horde::horde_data(s).expect("horde from spec");
    }
    serde_json::from_value(h["horde"].clone()).unwrap()
}

// docs/HORDE_SPEC.md tables vs the packages: SquadWaves (GM.json), SquadInfo assets with the USquadInfo ctor
// (rva=0x166fb50: Difficulty 1.0, SpawnMaxWave 21 when the asset omits them), KillReward class defaults
#[test]
fn horde_tables_match_spec() {
    let Some(h) = header() else { return skip() };
    let d = horde(&h);
    assert_eq!(d.squad_waves.len(), 25);
    let rw: Vec<i64> = d.squad_waves.iter().map(|w| w.completion_reward).collect();
    assert_eq!(&rw[..6], &[325, 325, 325, 325, 325, 350]);
    assert_eq!(rw[24], 1); // the last row's reward is 1 (spec)
    assert_eq!(d.squad_waves[9].difficulty_pool, 150.0);
    assert_eq!(d.squad_waves[24].mandatory_squads, vec!["DA_SquadSeymour", "DA_SquadTestBoss_Hard", "DA_SquadTestBoss_Hard"]);
    let sq = |n: &str| d.squads.iter().find(|s| s.name == n).unwrap().clone();
    let s = sq("DA_SquadSeymour");
    assert_eq!((s.spawn_min_wave, s.spawn_max_wave, s.difficulty, s.delay_before_spawn), (25, 25, 60.0, 35));
    assert_eq!(s.members, vec![("Seymour".to_string(), 1), ("Axeman".to_string(), 7)]);
    assert_eq!(sq("DA_SquadRockThrower").difficulty, 1.0);
    assert_eq!(sq("DA_SquadTestBoss").spawn_max_wave, 21);
    assert_eq!(sq("DA_SquadBossSupport01").spawn_min_wave, 26);
    let kr = |k: &str| d.enemies.iter().find(|e| e.key == k).unwrap().kill_reward;
    assert_eq!((kr("Peasant"), kr("Footman"), kr("Guard"), kr("Elite Knight"), kr("Ogre"), kr("Ogre Hard"), kr("Scrub")), (1, 4, 6, 7, 50, 75, 2));
    let c = &d.config;
    assert_eq!((c.wave, c.coins, c.skill_points, c.team_kill_coin_punishment, c.max_enemies_in_wave), (-1, 200, 5, -50, 200));
    assert_eq!((c.min_difficulty, c.max_difficulty, c.min_delay, c.max_delay), (0.45, 1.5, 1.0, 0.15));
    assert!((d.damage_by_player_count[0] - 0.25).abs() < 1e-6 && d.damage_by_player_count[2] == 0.3875 && d.damage_by_player_count[6] == 0.45);
}

fn horde_mode(h: &Value) -> GameMode {
    let mut m = GameMode::new(mode_data(h, "hrd_mode", "horde"), kismet().unwrap(), CrtRand::new(1));
    m.set_horde_spawners(4);
    m
}

// GetDifficultyRatio / GetSpawnDelayRatio@286-@753: endpoints n = 0, 1, 6 and the n = 7 extrapolation
#[test]
fn horde_ratios() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let mut m = horde_mode(&h);
    assert_eq!((m.get_difficulty_ratio(), m.get_spawn_delay_ratio()), (0.45, 0.15)); // no players
    m.login("p0", false, -1);
    assert_eq!((m.get_difficulty_ratio(), m.get_spawn_delay_ratio()), (0.45, 1.0));
    for i in 1..6 {
        m.login(&format!("p{i}"), false, -1);
    }
    assert_eq!(m.get_difficulty_ratio(), (1.5f32 - 0.45f32) * (5.0f32 / 5.0) + 0.45f32);
    assert!((m.get_spawn_delay_ratio() - 0.15).abs() < 1e-6);
    m.login("p6", false, -1);
    assert!(m.get_difficulty_ratio() > 1.5); // no clamp: n = 7 extrapolates
}

fn run(m: &mut GameMode, secs: f64) {
    let n = (secs / 0.5) as usize;
    for _ in 0..n {
        m.tick(0.5);
    }
}

// a full Horde match: StartHordeMatch -> ProgressWave (Wave 0, 25 s intermission), SpawnSquadWave (mandatory squads
// first, Find Squad window), kills pay KillReward (@8866), clearing a wave pays CompletionReward (@3643) + 1 skill
// point, a later join gets Round(clamp(total * 1.1, 0, 6000) / 10) * 10 (HandlePostLogin@648-@866); clearing the
// last row ends the match with WinnerTeam 0 (@4619)
#[test]
fn horde_waves_to_victory() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let d = horde(&h);
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    let q = m.login("p1", false, -1);
    run(&mut m, 5.0);
    assert!(m.horde().has_started && m.horde().wave == 0 && m.allow_spawning);
    assert_eq!(m.horde().match_info.next_wave_start_time, m.horde().wave_start_time + 25.0);
    assert!(m.ctrls[p].has_pawn && m.ctrls[q].has_pawn);
    let mut wave_seen = 0;
    let mut coins_from_kills = 0;
    for _ in 0..40000 {
        m.tick(0.5);
        // the players kill every live enemy at once, alternating killers
        let live: Vec<usize> = (0..m.horde().enemies.len()).filter(|&e| !m.horde().enemies[e].dead).collect();
        for (i, e) in live.into_iter().enumerate() {
            coins_from_kills += m.horde().enemies[e].kill_reward;
            m.horde_enemy_killed(e, Some(if i % 2 == 0 { p } else { q }));
        }
        if m.horde().wave == 1 && wave_seen == 0 {
            wave_seen = 1;
            assert_eq!(m.horde().total_awarded_gold_per_player, d.squad_waves[0].completion_reward);
            let late = m.login("late", false, -1);
            let lp = m.horde().players[&late].clone();
            assert_eq!(lp.coins, 200 + 360); // Round(325 * 1.1 / 10) * 10
            assert_eq!(lp.skill_points, 5 + 1);
        }
        if m.match_state != MatchState::InProgress {
            break;
        }
    }
    assert_eq!(m.match_state, MatchState::WaitingPostMatch);
    let e = m.match_end_info.clone().unwrap();
    assert_eq!((e.winner_team, e.winner_score, e.draw), (0, 1.0, false));
    assert_eq!(m.horde().wave, 25);
    let total: i64 = d.squad_waves.iter().map(|w| w.completion_reward).sum();
    assert_eq!(m.horde().total_awarded_gold_per_player, total);
    let pp = &m.horde().players[&p];
    assert!(pp.coins > 200 + total); // + kill rewards
    assert_eq!(pp.skill_points, (5 + 25) & 0xff);
    assert!(coins_from_kills > 0);
    // Seymour came with the last row's mandatory squads
    assert!(m.horde().enemies.iter().any(|e| e.key == "Seymour"));
}

// TriggerDefeat (@7703-@8232): during a wave (bAllowSpawning false) every team-0 player dead, no graves, nothing in
// the spawn queue -> MatchEndInfo WinnerTeam 1
#[test]
fn horde_defeat_when_all_dead_in_wave() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    run(&mut m, 27.0);
    assert!(!m.allow_spawning && m.horde().wave_has_spawned);
    m.on_killed(None, Some(p), 0, "", false);
    m.ctrls[p].has_pawn = false;
    m.tick(0.5);
    let e = m.match_end_info.clone().unwrap();
    assert_eq!((e.winner_team, m.match_state), (1, MatchState::WaitingPostMatch));
    let r = m.match_result_for(p, &e).unwrap();
    assert_eq!((r.text.as_str(), r.subtext.as_str()), ("defeat", "Reached Wave 1")); // Format("Reached Wave {a}", Wave + 1)
}

// a player killed during the intermission respawns by NextWaveStartTime - 1 at the latest (@7075-@7355); a team kill
// costs the killer 50 coins, clamped at 0 (@9190-@9300)
#[test]
fn horde_intermission_respawn_and_team_kill() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    let q = m.login("p1", false, -1);
    run(&mut m, 3.0);
    assert!(m.allow_spawning);
    m.on_killed(Some(p), Some(q), 0, "", false);
    assert_eq!(m.horde().players[&p].coins, 150);
    let nw = m.horde().match_info.next_wave_start_time - 1.0;
    assert!(m.ctrls[q].next_respawn_time as f32 <= nw);
    for _ in 0..5 {
        m.on_killed(Some(p), Some(q), 0, "", false);
    }
    assert_eq!(m.horde().players[&p].coins, 0);
}

// Battle Royale: the round starts once (StartRoundStart + StartRound), everyone spawns, the countdown follows the
// living, each death sets Placement / PlacementPosition (@2638-@3265), the last one alive wins (EndRound@625) and a
// human winner gets AddScore(10000)
#[test]
fn br_last_alive_wins() {
    let Some(h) = header() else { return skip() };
    let Some(k) = kismet() else { return skip() };
    let br: BrData = serde_json::from_value(h["br"].clone()).unwrap();
    assert_eq!(br.round_start_duration, 5.0);
    let d = mode_data(&h, "br_mode", "br");
    assert!(matches!(d.ext, ModeDataExt::Br(_)));
    let mut m = GameMode::new(d, k, CrtRand::new(1));
    let ps: Vec<usize> = ["p0", "p1", "p2"].iter().map(|n| m.login(n, false, -1)).collect();
    let b = m.login("bot", true, -1);
    for _ in 0..30 {
        m.tick(0.25);
    }
    assert!(m.br().initialized_match && !m.allow_spawning);
    assert!(ps.iter().chain([&b]).all(|&c| m.ctrls[c].has_pawn && m.ctrls[c].alive));
    assert_eq!(m.br().countdown, 4);
    assert!(!m.should_block_input(ps[0])); // WarmupEnd -1 + 5 already passed
    m.on_killed(Some(ps[0]), Some(b), 0, "", false);
    assert_eq!(m.br().countdown, 3);
    assert_eq!(m.br().placement[&b], 1);
    assert!(!m.br().placement_position.contains_key(&b)); // a bot has no BP_BattleRoyalePlayerController
    m.on_killed(Some(ps[0]), Some(ps[1]), 0, "", false);
    assert_eq!((m.br().placement[&ps[1]], m.br().placement_position[&ps[1]]), (2, 3));
    assert!(!m.controller_can_restart(ps[1])); // AlreadyDiedArray while in progress
    m.on_killed(Some(ps[0]), Some(ps[2]), 0, "", false);
    m.tick(0.25);
    assert_eq!(m.match_state, MatchState::WaitingPostMatch);
    let e = m.match_end_info.clone().unwrap();
    assert_eq!((e.winner.as_str(), e.draw), ("p0", false));
    assert!(m.ctrls[ps[0]].score >= 10000.0);
    let late = m.login("late", false, -1);
    assert!(m.br().late_join.contains(&late));
}

// ---- r3: the Horde extras, Demon Invasion and BR's result ------------------------------------------------------------

use mh_mode::horde_extras::{self as hx, BuyOutcome, BuyerView, ChanceItem, Purchase};
use mordhau_core::ue::FVector;

// BR HandleMatchEndInfo (ubergraph @15-@602): only the winner's local player gets ShowMatchResult(true, "victory")
#[test]
fn br_match_result_winner_only() {
    let Some(h) = header() else { return skip() };
    let Some(k) = kismet() else { return skip() };
    let mut m = GameMode::new(mode_data(&h, "br_mode", "br"), k, CrtRand::new(1));
    let a = m.login("a", false, -1);
    let b = m.login("b", false, -1);
    for _ in 0..30 {
        m.tick(0.25);
    }
    m.on_killed(Some(a), Some(b), 0, "", false);
    m.tick(0.25);
    let e = m.match_end_info.clone().unwrap();
    let r = m.match_result_for(a, &e).unwrap();
    assert_eq!((r.victory, r.text.as_str(), r.subtext.as_str()), (true, "victory", ""));
    assert!(m.match_result_for(b, &e).is_none());
}

// the exported records: BP_HordePlayerGrave defaults, the chest tiers (RespawnTime 5 inherited from
// BP_HordeChestBase, Second / Third lists empty), the buy menus, 52 skills with 8 ultimates, E_HordeSkill values
#[test]
fn horde_extras_tables() {
    let Some(h) = header() else { return skip() };
    let x = horde(&h).extras;
    assert_eq!((x.grave.revive_period, x.grave.auto_revive_time, x.grave.max_interaction_hold_time), (60.0, 30.0, 1.5));
    let costs: Vec<(i64, f32, usize)> = x.chests.iter().map(|c| (c.cost, c.respawn_time, c.item_list.len())).collect();
    assert_eq!(&costs[1..], &[(300, 5.0, 36), (1000, 5.0, 24), (1750, 5.0, 18)]);
    assert!(x.chests.iter().all(|c| c.second_item_list.is_empty() && c.original_second_item_list.is_empty()));
    assert_eq!((x.buy_menu.len(), x.di_buy_menu.len()), (82, 75));
    assert_eq!(x.skills.len(), 52);
    assert_eq!(x.skills.iter().filter(|s| s.ultimate).count(), 8);
    assert_eq!(x.skill_prerequisites.len(), 52);
    assert_eq!((hx::skill_value("NewEnumerator2"), hx::skill_value("NewEnumerator1"), hx::skill_value("NewEnumerator4")), (Some(1), Some(2), Some(53)));
    let lc = x.skills.iter().find(|s| hx::skill_value(&s.skill) == Some(hx::SKILL_LAST_CHANCE)).unwrap();
    assert_eq!(lc.name, "Last Chance");
    let restock = x.buy_menu.iter().find(|e| e.is_ammo_restock).unwrap();
    assert_eq!((restock.base_cost, restock.ammo_restock_amount), (25, 50));
    assert!(x.purchasables.len() > 50);
    assert!(x.demon.difficulty_scaling.len() > 6 && x.demon.jip_coins.len() == 16);
}

// graves: a player dying while bAllowSpawning is false leaves a grave; the defeat check waits for it (Graves.Num);
// it expires after RevivePeriod 60 (not while held), then AutoRevive restarts the player 30 s later
#[test]
fn horde_grave_holds_defeat_and_auto_revives() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    run(&mut m, 27.0);
    assert!(!m.allow_spawning);
    m.on_killed(None, Some(p), 0, "", false);
    m.ctrls[p].has_pawn = false;
    let g = m.horde_character_killed(p, Some(FVector::new(1.0, 2.0, 3.0))).unwrap();
    m.tick(0.5);
    assert!(m.match_end_info.is_none() && m.horde().graves.len() == 1);
    // held for 10 s: the revive period does not run down
    for _ in 0..20 {
        m.grave_interaction_maintained(g);
        m.tick(0.5);
    }
    assert_eq!(m.horde().graves[0].revive_period, 59.5);
    run(&mut m, 60.0);
    assert!(m.horde().graves[0].expired && m.match_end_info.is_none());
    run(&mut m, 30.5);
    assert!(m.horde().graves.is_empty());
}

// a revive: the reviver's GiveClientScoreBP(2, 250), the dead player restarted at the grave + 100 z, grave gone
#[test]
fn horde_grave_revive() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    let q = m.login("p1", false, -1);
    run(&mut m, 27.0);
    m.on_killed(None, Some(q), 0, "", false);
    m.ctrls[q].has_pawn = false;
    let g = m.horde_character_killed(q, Some(FVector::new(0.0, 0.0, 10.0))).unwrap();
    m.drain();
    m.grave_revive(g, Some(p));
    assert!(m.ctrls[q].has_pawn && m.ctrls[q].location.z == 110.0 && m.horde().graves.is_empty());
    let evs = m.drain();
    assert!(evs.iter().any(|e| e.kind == "client_score" && format!("{:?}", e.args).contains("250")));
    // during the intermission (bAllowSpawning) a classic death leaves no grave
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    run(&mut m, 3.0);
    assert!(m.allow_spawning && m.horde_character_killed(p, Some(FVector::ZERO)).is_none());
}

// GetRandomItem / RenormalizeChances on a forced rand() stream; the restock byte math
#[test]
fn chest_rolls() {
    let it = |c: &str, w: f32, k: &str| ChanceItem { class: c.into(), chance: w, kind: k.into() };
    let mut l = vec![it("a", 1.0, "equipment"), it("b", 3.0, "other")];
    let mut r = CrtRand::new(1);
    r.forced = [8192].into_iter().collect(); // FRand ~0.25 -> 0.25 >= r? no -> "b" unless exactly; check both lists
    let k0 = hx::get_random_item(&mut r, &mut l);
    assert_eq!((l[0].chance, l[1].chance), (0.25, 0.75));
    assert_eq!(k0, Some(1));
    r.forced = [0].into_iter().collect(); // 0.0: the first window
    assert_eq!(hx::get_random_item(&mut r, &mut l), Some(0));
    r.forced = [32767].into_iter().collect(); // 1.0: the last index
    assert_eq!(hx::get_random_item(&mut r, &mut l), Some(1));
    let mut e: Vec<ChanceItem> = vec![];
    assert_eq!(hx::get_random_item(&mut r, &mut e), None);
    assert_eq!(hx::restock_ammo(250, 10, 255), 4); // Add_ByteByte wraps before BMin
    assert_eq!(hx::restock_ammo(5, 50, 20), 20);
}

// a Tier1 chest: Cost 300 from the 200 starting coins fails; with coins it breaks, rolls one item (Second / Third
// empty), can't be opened while destroyed, and respawns after RespawnTime 5
#[test]
fn horde_chest_buy_and_respawn() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    let c = m.add_chest("BP_HordeChestTier1").unwrap();
    assert!(m.chest_interact(c, p).is_empty());
    assert_eq!(m.horde().players[&p].coins, 200);
    m.horde_mut().players.get_mut(&p).unwrap().coins = 1000;
    let s = m.chest_interact(c, p);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].impulse_yaw.is_some(), s[0].kind != "other");
    assert_eq!(m.horde().players[&p].coins, 700);
    // destroyed: the coins are still taken (OnInteractionStart has no destroyed check), BreakChest does nothing
    assert!(m.chest_interact(c, p).is_empty());
    assert_eq!(m.horde().players[&p].coins, 400);
    run(&mut m, 5.5);
    assert!(!m.horde().chests[c].destroyed);
}

// the buy menu: Clamp(Coins - |BaseCost|, 0, INT_MAX); an ammo restock bumps PurchaseTrigger; a placed purchasable
// debits Cost without a clamp and only dispenses to a BP_BattleRoyaleCharacter
#[test]
fn horde_shop() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    let v = BuyerView { has_character: true, can_restock: true, wearable_ok: true };
    let sword = m.buy_menu().iter().find(|e| e.purchased_actor == "BP_ArmingSword").unwrap().clone();
    assert_eq!(m.buy_menu_price(sword.id), 400);
    assert!(m.purchase_buy_menu_entry(p, sword.id, v).is_none()); // 200 coins
    m.horde_mut().players.get_mut(&p).unwrap().coins = 1000;
    assert_eq!(m.purchase_buy_menu_entry(p, sword.id, v), Some(BuyOutcome::Spawn { class: "BP_ArmingSword".into(), count: 1 }));
    assert_eq!(m.horde().players[&p].coins, 600);
    let ammo = m.buy_menu().iter().find(|e| e.is_ammo_restock).unwrap().clone();
    assert!(!m.is_horde_purchase_allowed(p, ammo.id, BuyerView { can_restock: false, ..v }));
    assert!(matches!(m.purchase_buy_menu_entry(p, ammo.id, v), Some(BuyOutcome::AmmoRestock { amount: 50, .. })));
    assert_eq!((m.horde().players[&p].coins, m.horde().players[&p].purchase_trigger), (575, 1));
    let def = m.horde_extras().purchasables.iter().find(|d| !d.purchasable_class.is_empty() && !d.is_ammo_restock && d.cost <= 500).unwrap().clone();
    m.horde_mut().players.get_mut(&p).unwrap().coins = 5000;
    let before = m.horde().players[&p].coins;
    assert!(m.purchasable_interact(&def, p, false).is_none());
    assert_eq!(m.horde().players[&p].coins, before - def.cost);
    assert_eq!(m.purchasable_interact(&def, p, true), Some(Purchase::SpawnItem { class: def.purchasable_class.clone() }));
}

// UpgradeSkill: points, prerequisites (missing entry or prerequisite 0 -> free), level cap 5, one ultimate
#[test]
fn horde_skill_tree() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    let mut m = horde_mode(&h);
    let p = m.login("p0", false, -1);
    m.horde_mut().players.get_mut(&p).unwrap().skill_points = 20;
    let x = m.horde_extras().clone();
    // NewEnumerator2 (Marathon, value 1) requires NewEnumerator0 (value 0): free
    let marathon = hx::skill_value("NewEnumerator2").unwrap();
    for _ in 0..5 {
        assert!(m.upgrade_skill(p, marathon));
    }
    assert!(!m.upgrade_skill(p, marathon)); // level < 5
    assert_eq!((m.skill_level(p, marathon), m.horde().players[&p].skill_points), (5, 15));
    // a skill whose prerequisite has level 0 is refused
    let (s, pre) = x
        .skill_prerequisites
        .iter()
        .map(|(a, b)| (hx::skill_value(a).unwrap(), hx::skill_value(b).unwrap()))
        .find(|&(_, b)| b != 0 && b != marathon)
        .unwrap();
    assert!(m.skill_level(p, pre) == 0 && !m.upgrade_skill(p, s));
    let (pa, _, _, _) = m.scaled_skill_level_params(p, marathon, 0);
    let info = x.skills.iter().find(|i| hx::skill_value(&i.skill) == Some(marathon)).unwrap();
    assert_eq!(pa, info.percent_a * 5.0);
    // ultimates: with every skill at level 1 except two ultimates, SpecialSkill is another ultimate and the two are
    // refused
    let ults: Vec<u8> = x.skills.iter().filter(|i| i.ultimate).map(|i| hx::skill_value(&i.skill).unwrap()).collect();
    {
        let pl = m.horde_mut().players.get_mut(&p).unwrap();
        for v in pl.skills.iter_mut() {
            *v = 1;
        }
        pl.skills[ults[0] as usize] = 0;
        pl.skills[ults[1] as usize] = 0;
    }
    m.skills_updated(p);
    let sp = m.horde().players[&p].special_skill;
    assert!(ults.contains(&sp) && sp != ults[0] && sp != ults[1]);
    assert!(!m.upgrade_skill(p, ults[0]));
}

// Demon Invasion: StartHordeMatch override (no ProgressWave), the burst, the kill reward remap, FailStage's end info
// and config vars
#[test]
fn demon_invasion_rules() {
    let Some(h) = header() else { return skip() };
    if kismet().is_none() {
        return skip();
    }
    use mh_mode::demon_horde::{map_range_clamped, DemonConfig};
    use mh_mode::horde_extras::DemonStage;
    let mut m = horde_mode(&h);
    let st = DemonStage {
        stage: 1,
        max_total_enemies: 10,
        wave_spawn_interval: 10.0,
        stage_spawn_delay: 10.0,
        horde_enemies: vec![("Footman".into(), 1.0), ("Guard".into(), 3.0)],
        ..Default::default()
    };
    m.set_demon_invasion(vec![st], DemonConfig::default());
    let p = m.login("p0", false, -1);
    run(&mut m, 2.0);
    assert!(m.horde().has_started && m.horde().wave == -1); // no ProgressWave
    assert_eq!(m.demon().prestige_tier, 1.0);
    m.demon_set_current_stage(1);
    run(&mut m, 25.0);
    let evs = m.drain();
    let spawns = evs.iter().filter(|e| e.kind == "demon_spawn_enemy").count();
    assert!(spawns > 0, "the burst spawns once the stage is > 0");
    assert_eq!(GameMode::demon_kill_reward(4), 30); // FTrunc(MapRangeClamped(4, 0, 50, 20, 150)) = 30.4
    assert_eq!(map_range_clamped(3000.0, 600.0, 2400.0, 1.5, 1.0), 1.0);
    m.demon_fail_stage();
    let e = m.match_end_info.clone().unwrap();
    assert_eq!(e.winner_team, 1);
    let r = m.match_result_for(p, &e).unwrap();
    assert_eq!((r.victory, r.text.as_str()), (false, "defeat"));
    assert_eq!((m.demon().config.fails, m.demon().config.difficulty), (Some(1), Some(1.0)));
}

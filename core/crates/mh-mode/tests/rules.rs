//! Rule tests for mh-mode on synthetic records (no game data needed): each names the reference rule it checks. The
//! golden traces (golden_mode.rs) hold the full per-tick parity with the GDScript reference on the game's own data.

use mh_mode::ai::tasks::{calculate_angle_2d, find_delta_angle, rotate_yaw, safe_normal, segment_hits_capsule};
use mh_mode::control_point::{ControlPoint, NO_TEAM};
use mh_mode::data::*;
use mh_mode::game_mode::{GameMode, MatchState};
use mh_mode::kismet::Kismet;
use mordhau_core::ue::{CrtRand, FVector};
use std::sync::Arc;

/// the bytecode literals these rules read (values as scripts/mode_kismet.py extracts them; srcs omitted)
fn kismet() -> Arc<Kismet> {
    let j = r#"{
      "kf_flag_fall_damage_type": {"value": 3}, "kf_flag_fall": {"value": 2}, "kf_flag_kick": {"value": 1},
      "dm_end_winner_team": {"value": 0}, "dm_spectator_team": {"value": -1}, "dm_victory": {"value": "Victory"},
      "dm_defeat": {"value": "Defeat"}, "dm_winner_format": {"value": "{a} wins the match!"},
      "tdm_tie_winner_team": {"value": 0}, "sb_time_no_limit_days": {"value": 200}
    }"#;
    Arc::new(Kismet::from_json(j).unwrap())
}

fn data(ext: ModeDataExt, team_mode: bool, team_count: i64) -> ModeData {
    ModeData {
        game_mode: "GM".into(),
        game_state: "GS".into(),
        map_json: "MAP".into(),
        scoring: ScoringDef {
            kill_score_change: 100.0,
            team_kill_score_change: -100.0,
            kill_team_score_change: 10.0,
            team_kill_team_score_change: -10.0,
            player_respawn_time: 5.0,
            assist_score_factor: 1.0,
            assist_damage_to_count_as_kill: 100,
            b_is_scoring_disabled: true,
            b_team_kills_decrement_killer_kills: true,
            ..Default::default()
        },
        state: MatchStateDef {
            match_duration_max: 1200,
            team_count,
            b_is_team_mode: team_mode,
            warmup_end: -1.0,
            team_names: vec!["Red".into(), "Blue".into()],
            b_allow_spawning: true,
            ..Default::default()
        },
        meta: Metadata::default(),
        starts: (0..4)
            .map(|i| PlayerStartDef { name: format!("S{i}"), team: -2, b_is_spawn_disabled: false, origin: [i as f32 * 10.0, 0.0, 0.0] })
            .collect(),
        bot_auto_respawn: true,
        bot_default_team: -2,
        ok: true,
        ext,
    }
}

fn run(m: &mut GameMode, ticks: usize) {
    for _ in 0..ticks {
        m.tick(0.1);
    }
}

// AMordhauGameMode::OnKilled_Implementation 0x159d6f0 + BP_MordhauGameMode AddKillNotify flags; BP_DeathmatchGameMode
// OnScoreChanged ends the match at ScoreToWin
#[test]
fn ffa_kill_scoring_respawn_and_win() {
    let mut m = GameMode::new(data(ModeDataExt::Ffa { score_to_win: 200.0 }, false, 1), kismet(), CrtRand::new(1));
    let p = m.login("player", false, -1);
    let b = m.login("bot", true, -1);
    run(&mut m, 5); // match starts, both spawn through the queue (3 steps each)
    assert_eq!(m.match_state, MatchState::InProgress);
    assert!(m.ctrls[p].has_pawn && m.ctrls[b].has_pawn);
    m.drain();
    m.on_killed(Some(p), Some(b), 3, "Longsword", false);
    assert_eq!((m.ctrls[p].kills, m.ctrls[p].score, m.ctrls[b].deaths), (1, 100.0, 1));
    assert_eq!(m.ctrls[b].next_respawn_time, m.now + 5.0); // not in waves: now + PlayerRespawnTime
    let kn = m.events.iter().find(|e| e.kind == "kill_notify").unwrap();
    assert_eq!(kn.get("flags").unwrap().as_i(), 2); // DamageType 3 -> Flags 2
    m.on_killed(Some(b), Some(b), 0, "", true); // suicide: TeamKillScoreChange -100, kick flag
    assert_eq!(m.ctrls[b].score, -100.0);
    m.on_killed(Some(p), Some(b), 0, "", false);
    assert_eq!(m.match_state, MatchState::WaitingPostMatch);
    let e = m.match_end_info.clone().unwrap();
    assert_eq!((e.winner.as_str(), e.winner_score, e.draw), ("player", 200.0, false));
    assert_eq!(m.match_result_for(b, &e).unwrap().subtext, "player wins the match!");
}

// UMordhauUtilityLibrary::MordhauPlayerStateSortPredicate 0x162e7d0: score desc, kills desc, deaths asc, name, PlayerId
#[test]
fn sort_players_predicate() {
    let mut m = GameMode::new(data(ModeDataExt::Ffa { score_to_win: 3000.0 }, false, 1), kismet(), CrtRand::new(1));
    let a = m.login("b", false, -1);
    let b = m.login("a", false, -1);
    let c = m.login("c", false, -1);
    m.ctrls[c].score = 10.0;
    assert_eq!(m.sort_players(&[a, b, c]), vec![c, b, a]);
    m.ctrls[a].kills = 1;
    assert_eq!(m.sort_players(&[a, b, c]), vec![c, a, b]);
}

// OnKilled assist loop: RoundToInt(min(damage * 0.01, 1) * KillScoreChange) * AssistScoreFactor, team mode only;
// AMordhauCharacter::TakeDamage 0x156e980 DamageHistory freshness (20 s)
#[test]
fn tdm_assists_and_team_scores() {
    let mut m = GameMode::new(data(ModeDataExt::Tdm { team_score_to_win: 1000.0 }, true, 2), kismet(), CrtRand::new(1));
    let x = m.login("x", false, -1);
    let y = m.login("y", false, -1);
    let z = m.login("z", false, -1);
    m.set_team(x, 0);
    m.set_team(y, 0);
    m.set_team(z, 1);
    run(&mut m, 12);
    assert!(m.ctrls[z].alive);
    // exe (binary32): f32(50.5 * 0.0099999998f (.rdata 0x144014a94)) = 0.505, (f + f) * 100 = 101.0f, + 0.5 = 101.5 ->
    // cvtss2si half-even 102 >> 1 = 51. The GDScript reference (doubles) gets 100.99999 + 0.5 -> 50: an exe/reference difference.
    m.on_damage(z, y, 50.5);
    m.on_killed(Some(x), Some(z), 0, "", false);
    assert_eq!(m.ctrls[y].score, 51.0);
    assert_eq!(m.ctrls[y].assists, 1);
    assert_eq!(m.team_scores, vec![10.0, 0.0]);
    m.on_killed(Some(x), Some(y), 0, "", false); // team kill: -10 team score, -100 score, kills - 1
    assert_eq!(m.team_scores[0], 0.0);
    assert_eq!((m.ctrls[x].kills, m.ctrls[x].score), (0, 0.0));
}

// UMordhauUtilityLibrary::GetMaxIndexWithDraw 0x1623f00
#[test]
fn max_index_with_draw() {
    assert_eq!(GameMode::max_index_with_draw(&[]), (-1, false));
    assert_eq!(GameMode::max_index_with_draw(&[1, 3, 2]), (1, false));
    assert_eq!(GameMode::max_index_with_draw(&[2, 2]), (0, true));
    assert_eq!(GameMode::max_index_with_draw(&[2, 2, 3]), (2, false));
}

fn cp_def() -> ControlPointDef {
    ControlPointDef {
        name: "CP".into(),
        owning_team: NO_TEAM,
        capturing_team: NO_TEAM,
        award_score_interval: 1.0,
        award_score_capturing: 10,
        award_score_captured: 50,
        award_score_neutralizing: 10,
        award_score_neutralized: 50,
        uncapture_speed: 0.5,
        network_smooth_time: 0.1,
        b_prevent_spawning_if_contested: true,
        b_is_capturable: true,
        capture_speed: vec![0.0, 0.25, 0.5],
        neutralize_speed: vec![0.0, 0.5, 1.0],
        time_to_unlock: -1.0,
        ..Default::default()
    }
}

// AControlPoint::UpdateCaptureProgress 0x151b3f0 / SetCaptureProgress 0x1515740 / UpdatePresenceNumbers 0x151e920:
// a lone team-0 pawn captures at CaptureSpeedCurve(1) per second, owns it at 1, scores per AwardScoreInterval and 50
// on capture; a stronger team-1 presence neutralizes it back to 255
#[test]
fn control_point_capture_and_neutralize() {
    let mut m = GameMode::new(data(ModeDataExt::Tdm { team_score_to_win: 1e9 }, true, 2), kismet(), CrtRand::new(1));
    let a = m.login("a", false, -1);
    let b = m.login("b", false, -1);
    let c = m.login("c", false, -1);
    m.set_team(a, 0);
    m.set_team(b, 1);
    m.set_team(c, 1);
    run(&mut m, 12);
    let cp = m.add_control_point(ControlPoint::new(cp_def()));
    m.cp_begin_overlap(cp, a);
    run(&mut m, 30); // first tick sets the prerequisites (empty = owned); then 0.25 / s
    assert_eq!(m.control_points[cp].capturing_team, 0);
    run(&mut m, 20);
    assert_eq!(m.control_points[cp].owning_team, 0);
    assert_eq!(m.control_points[cp].capture_progress, 1.0);
    assert!(m.ctrls[a].score >= 50.0 + 30.0);
    assert_eq!(m.team1_capture_points, 1);
    m.cp_begin_overlap(cp, b);
    m.cp_begin_overlap(cp, c);
    run(&mut m, 40); // lead 1 -> NeutralizeSpeedCurve(1) 0.5 / s
    assert_eq!(m.control_points[cp].owning_team, NO_TEAM);
    assert_eq!(m.team1_capture_points, 0);
}

// FVector::GetSafeNormal (exact 1 kept, < 1e-8 zero), CalculateAngle2D 0x1616170, FindDeltaAngleDegrees 0x1480350
#[test]
fn task_math() {
    assert_eq!(safe_normal(FVector::new(3.0, 4.0, 0.0)), FVector::new(0.6, 0.8, 0.0));
    assert_eq!(safe_normal(FVector::new(1e-5, 0.0, 0.0)), FVector::ZERO);
    assert!((calculate_angle_2d(FVector::new(0.0, 1.0, 0.0), 0.0) - 90.0).abs() < 1e-4);
    assert!((calculate_angle_2d(FVector::new(0.0, -1.0, 0.0), 0.0) + 90.0).abs() < 1e-4);
    assert_eq!(calculate_angle_2d(FVector::new(1e-5, 0.0, 0.0), 0.0), 0.0);
    assert_eq!(find_delta_angle(170.0, -170.0), 20.0);
    assert_eq!(find_delta_angle(-170.0, 170.0), -20.0);
    let r = rotate_yaw(FVector::new(1.0, 0.0, 5.0), 90.0);
    assert!(r.x.abs() < 1e-6 && (r.y - 1.0).abs() < 1e-6 && r.z == 5.0);
    assert!(segment_hits_capsule(FVector::new(-100.0, 0.0, 0.0), FVector::new(100.0, 0.0, 0.0), FVector::ZERO, 96.0, 50.0));
    assert!(!segment_hits_capsule(FVector::new(-100.0, 80.0, 0.0), FVector::new(100.0, 80.0, 0.0), FVector::ZERO, 96.0, 50.0));
}

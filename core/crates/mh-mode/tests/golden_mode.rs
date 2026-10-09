//! Golden-trace parity for mh-mode: replays core/tests/golden/mode/*.jsonl (written by godot/tools/export_golden.gd
//! through the runner godot/tools/golden/mode.gd, scenarios in core/tests/scenarios/mode.json) and compares every
//! tick with the GDScript reference: mode scenarios (ops -> events / control point events / full state) and bot
//! scenarios (fighter views -> every bot's decisions, requests, blackboard, BT state, perception, profile, rand()).
//! The golden files are Triternion data (git-ignored); without them the tests say so and pass (set
//! MH_GOLDEN_REQUIRED=1 to fail instead).

use mh_mode::ai::{BbVal, BodyId, BotBody, BotHost, Bots, InventoryItem, PawnView, Profile, Request, TreeDef, WeaponView};
use mh_mode::control_point::ControlPoint;
use mh_mode::data::{ControlPointDef, ModeData};
use mh_mode::game_mode::{GameMode, ModeExt};
use mh_mode::kismet::Kismet;
use mordhau_core::ue::{CrtRand, FVector};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// a golden row: floats are written as "f64:<hex>" (export_golden.gd exact(): lossless); decoded bit for bit
fn parse(line: &str) -> Result<Value, String> {
    let mut v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    mordhau_core::data::decode_exact(&mut v);
    Ok(v)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn golden_files(prefix: &str) -> Vec<PathBuf> {
    let dir = root().join("core/tests/golden/mode");
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    v.retain(|p| p.extension().map(|e| e == "jsonl").unwrap_or(false) && p.file_name().unwrap().to_string_lossy().starts_with(prefix));
    v.sort();
    v
}

fn kismet() -> Option<Arc<Kismet>> {
    let t = std::fs::read_to_string(root().join("godot/data_gen/mode/mode_kismet.json")).ok()?;
    Some(Arc::new(Kismet::from_json(&t).expect("mode_kismet.json")))
}

fn missing(what: &str) {
    if std::env::var("MH_GOLDEN_REQUIRED").is_ok() {
        panic!("golden data missing: {what}");
    }
    eprintln!("SKIP: {what} missing (run the exporter; see godot/tools/golden/mode.gd)");
}

/// exact comparison (numbers as f64), first difference as "path: expected vs got"
fn diff(path: &str, want: &Value, got: &Value) -> Option<String> {
    match (want, got) {
        (Value::Number(a), Value::Number(b)) => {
            let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            if x == y {
                None
            } else {
                Some(format!("{path}: want {x:?} got {y:?}"))
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: want len {} got {} ({want} vs {got})", a.len(), b.len()));
            }
            a.iter().zip(b).enumerate().find_map(|(i, (x, y))| diff(&format!("{path}[{i}]"), x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            for k in a.keys() {
                if !b.contains_key(k) {
                    return Some(format!("{path}.{k}: missing in port (want {})", a[k]));
                }
            }
            for k in b.keys() {
                if !a.contains_key(k) {
                    return Some(format!("{path}.{k}: extra in port ({})", b[k]));
                }
            }
            a.iter().find_map(|(k, x)| diff(&format!("{path}.{k}"), x, &b[k]))
        }
        _ => {
            if want == got {
                None
            } else {
                Some(format!("{path}: want {want} got {got}"))
            }
        }
    }
}

fn v3(v: FVector) -> Value {
    json!([v.x as f64, v.y as f64, v.z as f64])
}

fn f3(v: &Value) -> FVector {
    let a = v.as_array().unwrap();
    FVector::new(a[0].as_f64().unwrap() as f32, a[1].as_f64().unwrap() as f32, a[2].as_f64().unwrap() as f32)
}

fn check_consts(h: &Value) -> Result<(), String> {
    use mh_mode::consts::{bot as B, mode as M};
    // a group the reference had not initialized when it wrote this header is absent (checked in the other files)
    let want = |g: &str, k: &str| h["consts"][g][k].as_f64();
    let ours: &[(&str, &str, f64)] = &[
        ("mode", "spawn_proximity_range", M::SPAWN_PROXIMITY_RANGE),
        ("mode", "spawn_proximity_scale", M::SPAWN_PROXIMITY_SCALE),
        ("mode", "spawn_proximity_min", M::SPAWN_PROXIMITY_MIN),
        ("mode", "spawn_proximity_max", M::SPAWN_PROXIMITY_MAX),
        ("mode", "spawn_crowd_weight", M::SPAWN_CROWD_WEIGHT),
        ("mode", "spawn_blocked_penalty", M::SPAWN_BLOCKED_PENALTY),
        ("mode", "spawn_invalid", M::SPAWN_INVALID),
        ("mode", "spawn_rand_scale", M::SPAWN_RAND_SCALE),
        ("mode", "assist_window", M::ASSIST_WINDOW),
        ("mode", "assist_damage_scale", M::ASSIST_DAMAGE_SCALE),
        ("mode", "assist_fraction_max", M::ASSIST_FRACTION_MAX),
        ("mode", "assist_thanks_fraction", M::ASSIST_THANKS_FRACTION),
        ("mode", "team_rand_scale", M::TEAM_RAND_SCALE),
        ("bot", "frand_scale", B::FRAND_SCALE),
        ("bot", "bt_wait_rand_scale", B::BT_WAIT_RAND_SCALE),
        ("bot", "bt_service_rand_scale", B::BT_SERVICE_RAND_SCALE),
        ("bot", "voice_pick_rand_scale", B::VOICE_PICK_RAND_SCALE),
        ("bot", "profile_one", B::PROFILE_ONE),
        ("bot", "profile_unit_vector_min_sq", B::PROFILE_UNIT_VECTOR_MIN_SQ),
        ("bot", "randomize_ally_count_scale", B::RANDOMIZE_ALLY_COUNT_SCALE),
        ("bot", "randomize_cube_scale", B::RANDOMIZE_CUBE_SCALE),
        ("bot", "ally_clearance_rand_scale", B::ALLY_CLEARANCE_RAND_SCALE),
        ("bot", "safe_normal_min_sq", B::SAFE_NORMAL_MIN_SQ),
        ("bot", "weapon_length_to_cm", B::WEAPON_LENGTH_TO_CM),
        ("bot", "default_acceptance", B::DEFAULT_ACCEPTANCE),
        ("bot", "angle2d_zero_component", B::ANGLE2D_ZERO_COMPONENT),
        ("bot", "angle2d_min_sq", B::ANGLE2D_MIN_SQ),
        ("bot", "angle2d_rad_to_deg", B::ANGLE2D_RAD_TO_DEG),
        ("bot", "defend_in_front_dot", B::DEFEND_IN_FRONT_DOT),
        ("bot", "defend_enemy_facing_dot", B::DEFEND_ENEMY_FACING_DOT),
        ("bot", "defend_move_max_zdiff", B::DEFEND_MOVE_MAX_ZDIFF),
        ("bot", "defend_crouch_delay", B::DEFEND_CROUCH_DELAY),
        ("bot", "defend_footwork_facing", B::DEFEND_FOOTWORK_FACING),
        ("bot", "defend_back_off_margin", B::DEFEND_BACK_OFF_MARGIN),
        ("bot", "defend_facing_height", B::DEFEND_FACING_HEIGHT),
        ("bot", "defend_back_off_min", B::DEFEND_BACK_OFF_MIN),
        ("bot", "defend_parry_lead", B::DEFEND_PARRY_LEAD),
        ("bot", "defend_sweep_max", B::DEFEND_SWEEP_MAX),
        ("bot", "defend_sweep_step", B::DEFEND_SWEEP_STEP),
        ("bot", "attack_default_time_left", B::ATTACK_DEFAULT_TIME_LEFT),
        ("bot", "attack_enemy_speed_factor", B::ATTACK_ENEMY_SPEED_FACTOR),
        ("bot", "attack_jitter_length_offset", B::ATTACK_JITTER_LENGTH_OFFSET),
        ("bot", "attack_jitter_scale", B::ATTACK_JITTER_SCALE),
        ("bot", "attack_circle_base", B::ATTACK_CIRCLE_BASE),
        ("bot", "attack_out_of_range_margin", B::ATTACK_OUT_OF_RANGE_MARGIN),
        ("bot", "attack_engage_margin", B::ATTACK_ENGAGE_MARGIN),
        ("bot", "attack_move_max_zdiff", B::ATTACK_MOVE_MAX_ZDIFF),
        ("bot", "attack_side_step_dist", B::ATTACK_SIDE_STEP_DIST),
        ("bot", "attack_side_step_yaw", B::ATTACK_SIDE_STEP_YAW),
        ("bot", "attack_side_step_yaw_range", B::ATTACK_SIDE_STEP_YAW_RANGE),
        ("bot", "attack_feint_lead", B::ATTACK_FEINT_LEAD),
        ("bot", "attack_drag_pitch_scale", B::ATTACK_DRAG_PITCH_SCALE),
        ("bot", "attack_drag_yaw_base", B::ATTACK_DRAG_YAW_BASE),
        ("bot", "attack_drag_yaw_per_angle", B::ATTACK_DRAG_YAW_PER_ANGLE),
        ("bot", "attack_flip", B::ATTACK_FLIP),
        ("bot", "attack_brawl_range", B::ATTACK_BRAWL_RANGE),
        ("bot", "attack_choice_min_weight", B::ATTACK_CHOICE_MIN_WEIGHT),
        ("bot", "attack_flip_rand_scale", B::ATTACK_FLIP_RAND_SCALE),
        ("bot", "attack_angle_rand_scale", B::ATTACK_ANGLE_RAND_SCALE),
        ("bot", "attack_angle_offset", B::ATTACK_ANGLE_OFFSET),
        ("bot", "circle_no_ally_dist", B::CIRCLE_NO_ALLY_DIST),
        ("bot", "circle_full_turn", B::CIRCLE_FULL_TURN),
        ("bot", "circle_half_turn", B::CIRCLE_HALF_TURN),
        ("bot", "circle_ally_avoid_dist", B::CIRCLE_ALLY_AVOID_DIST),
        ("bot", "circle_step_min", B::CIRCLE_STEP_MIN),
        ("bot", "circle_step_max", B::CIRCLE_STEP_MAX),
        ("bot", "feint_enemy_facing_dot", B::FEINT_ENEMY_FACING_DOT),
        ("bot", "feint_react_delay", B::FEINT_REACT_DELAY),
        ("bot", "feint_watch_lead", B::FEINT_WATCH_LEAD),
        ("bot", "feint_range_sq", B::FEINT_RANGE_SQ),
        ("bot", "voice_min_interval", B::VOICE_MIN_INTERVAL),
        ("bot", "closest_enemy_recheck_s", B::CLOSEST_ENEMY_RECHECK_S),
        ("bot", "closest_enemy_saturation_sq", B::CLOSEST_ENEMY_SATURATION_SQ),
        ("bot", "random_float_next_scale", B::RANDOM_FLOAT_NEXT_SCALE),
        ("bot", "random_float_first_delay", B::RANDOM_FLOAT_FIRST_DELAY),
        ("bot", "random_float_delay", B::RANDOM_FLOAT_DELAY),
        ("bot", "move_midpoint_half", B::MOVE_MIDPOINT_HALF),
        ("bot", "can_see_raise", B::CAN_SEE_RAISE),
    ];
    let mut n_checked = 0;
    for (g, k, v) in ours {
        let Some(w) = want(g, k) else { continue };
        n_checked += 1;
        if w != *v {
            return Err(format!("const {g}.{k}: reference {w:?}, port {v:?}"));
        }
    }
    let n = h["consts"]["mode"].as_object().map(|o| o.len()).unwrap_or(0) + h["consts"]["bot"].as_object().map(|o| o.len()).unwrap_or(0);
    if n != n_checked {
        return Err(format!("reference has {n} constants, port checks {n_checked}"));
    }
    Ok(())
}

// ==== modes ========================================================================================================
/// HUD commands compared without "t" (the reference passes the mode's announce events through with their time) and
/// "src" (a diagnostic)
fn norm_hud(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|c| {
                    let mut c = c.clone();
                    if let Some(o) = c.as_object_mut() {
                        o.remove("t");
                        o.remove("src");
                    }
                    c
                })
                .collect(),
        ),
        _ => Value::Array(Vec::new()),
    }
}

fn mode_state(m: &GameMode) -> Value {
    let names = |v: &[usize]| Value::Array(v.iter().map(|&c| json!(m.ctrls[c].name)).collect());
    let rg = |g: &Option<mh_mode::game_mode::RoomGame>| match g {
        Some(g) => json!([g.team1_wins, g.team2_wins, g.round.stage, g.round.winner, g.round.start_time]),
        None => json!([]),
    };
    let mut st = Map::new();
    st.insert("now".into(), json!(m.now));
    st.insert("ms".into(), json!(m.match_state.name()));
    st.insert("el".into(), json!(m.elapsed_time));
    st.insert("sd".into(), json!(m.scoring_disabled));
    st.insert("as".into(), json!(m.allow_spawning));
    st.insert("tro".into(), json!(m.match_time_ran_out));
    st.insert("ts".into(), json!(m.team_scores));
    st.insert("cp1".into(), json!(m.team1_capture_points));
    st.insert("cp2".into(), json!(m.team2_capture_points));
    st.insert("rng".into(), json!([m.rng.seed, m.rng.calls]));
    st.insert("q".into(), names(&m.spawns.queue));
    st.insert("sp".into(), json!(m.spawns.spawning.map(|c| m.ctrls[c].name.clone()).unwrap_or_default()));
    st.insert("step".into(), json!(m.spawns.step));
    let end = match &m.match_end_info {
        Some(e) => json!({"winner": e.winner, "winner_team": e.winner_team, "winner_score": e.winner_score,
            "other_score": e.other_score, "draw": e.draw}),
        None => json!({}),
    };
    st.insert("end".into(), end);
    st.insert("sb".into(), json!(m.scoreboard_seconds()));
    let mut cs = Vec::new();
    let mut mr = Map::new();
    for &c in &m.controllers {
        let cc = &m.ctrls[c];
        let dh: Vec<Value> = cc.damage_history.iter().map(|e| json!([m.ctrls[e.who].name, e.t, e.damage])).collect();
        cs.push(json!({"name": cc.name, "team": cc.team, "alive": cc.alive, "score": cc.score, "k": cc.kills,
            "d": cc.deaths, "a": cc.assists, "pawn": cc.has_pawn, "nrt": cc.next_respawn_time, "wr": cc.wants_respawn,
            "laf": cc.last_asked_for_spawn, "pid": cc.player_id, "cpt": cc.capture_point_time,
            "cpi": cc.capture_point.map(|i| i as i64).unwrap_or(-1), "loc": v3(cc.location),
            "blk": m.should_block_input(c), "dh": dh, "mid": cc.match_id, "iiw": names(&cc.in_instance_with),
            "rrg": rg(&cc.replicated_room_game)}));
        if let Some(e) = &m.match_end_info {
            let r = match m.match_result_for(c, e) {
                Some(r) => json!({"kind": "match_result", "victory": r.victory, "text": r.text, "subtext": r.subtext}),
                None => json!({}),
            };
            mr.insert(cc.name.clone(), r);
        }
    }
    st.insert("c".into(), Value::Array(cs));
    st.insert("mr".into(), Value::Object(mr));
    match &m.ext {
        ModeExt::Skm(k) => {
            let ct: Vec<Value> = m.client_tick(0).iter().map(|e| e.to_json()).collect();
            st.insert("skm".into(), json!({"ri": [k.round_info.stage, k.round_info.winner, k.round_info.start_time],
                "rpp": k.round_points_per_team, "srp": k.state_round_points, "ad": names(&k.already_died),
                "lo": k.last_observed_round_stage, "lw": k.last_winner, "ls": k.loss_streak, "sw": k.has_swapped_teams,
                "cp": k.capture_point.map(|i| i as i64).unwrap_or(-1), "ct": ct}));
        }
        ModeExt::Duel(d) | ModeExt::Tf(d) => {
            let rooms: Vec<Value> = d
                .rooms
                .iter()
                .map(|r| json!({"c": names(&r.controllers), "g": rg(&Some(r.game)), "m1": r.team1_mmr, "m2": r.team2_mmr,
                    "st": r.state, "mid": r.match_id}))
                .collect();
            let mut mt = Map::new();
            for (k, mem) in &d.matches {
                mt.insert(k.clone(), Value::Array(mem.iter().map(|x| json!([x.entity, x.team_id])).collect()));
            }
            st.insert("du".into(), json!({"rooms": rooms, "un": names(&d.unhandled), "tumc": d.time_until_map_change,
                "ja": d.join_allowed, "matches": mt}));
        }
        ModeExt::Fl(f) => {
            st.insert("fl".into(), json!({"dc": f.ticket_drain_counter}));
        }
        _ => {}
    }
    let cps: Vec<Value> = m
        .control_points
        .iter()
        .map(|cp| json!({"name": cp.name, "p": cp.capture_progress, "rp": cp.replicated_capture_progress, "ow": cp.owning_team,
            "ca": cp.capturing_team, "u": cp.unchanged_capture_progress_time, "nuf": cp.net_update_frequency,
            "cap": cp.b_is_capturable, "t1": cp.b_team1_owns_prerequisites, "t2": cp.b_team2_owns_prerequisites,
            "sdis": cp.b_spawns_disabled, "stm": cp.spawns_team, "p1": cp.team1_presence, "p2": cp.team2_presence,
            "fl": cp.b_is_flashing, "lk": cp.locked, "ov": names(&cp.overlapping), "oc": names(&cp.overlaps_cache)}))
        .collect();
    st.insert("cps".into(), Value::Array(cps));
    if !m.control_points.is_empty() {
        st.insert("sdis".into(), Value::Array(m.starts.iter().map(|s| json!(s.b_is_spawn_disabled)).collect()));
    }
    Value::Object(st)
}

fn apply_op(m: &mut GameMode, op: &Value, header: &Value) -> Result<(), String> {
    let s = |k: &str| op[k].as_str().unwrap_or("").to_string();
    let find = |m: &GameMode, n: &str| m.by_name(n).ok_or(format!("op {op}: no controller {n}"));
    match op["op"].as_str().unwrap() {
        "login" => {
            m.login(&s("name"), op["bot"].as_bool().unwrap(), op["sid"].as_i64().unwrap());
        }
        "logout" => {
            let c = find(m, &s("name"))?;
            m.logout(c);
        }
        "kill" => {
            let killer = if s("killer").is_empty() { None } else { Some(find(m, &s("killer"))?) };
            let killed = find(m, &s("killed"))?;
            m.on_killed(killer, Some(killed), op["dt"].as_i64().unwrap(), &s("weapon"), op["kick"].as_bool().unwrap());
        }
        "despawn" => {
            let c = find(m, &s("name"))?;
            m.ctrls[c].has_pawn = false;
        }
        "damage" => {
            let (v, a) = (find(m, &s("victim"))?, find(m, &s("attacker"))?);
            m.on_damage(v, a, op["amount"].as_f64().unwrap());
        }
        "loc" => {
            let c = find(m, &s("name"))?;
            m.ctrls[c].location = f3(&op["p"]);
        }
        "overlap" => {
            let c = find(m, &s("name"))?;
            m.cp_begin_overlap(op["cp"].as_u64().unwrap() as usize, c);
        }
        "unoverlap" => {
            let c = find(m, &s("name"))?;
            m.cp_end_overlap(op["cp"].as_u64().unwrap() as usize, c);
        }
        "skm_cp" => {
            let def: ControlPointDef = serde_json::from_value(header["skm_cp_def"].clone()).map_err(|e| e.to_string())?;
            m.set_capture_point(ControlPoint::new(def));
        }
        o => return Err(format!("unknown op {o}")),
    }
    Ok(())
}

fn run_mode_file(path: &Path, k: &Arc<Kismet>) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut lines = text.lines();
    lines.next(); // exporter header {scenario, file}
    let header: Value = parse(lines.next().ok_or("no header")?)?;
    check_consts(&header)?;
    let data: ModeData = serde_json::from_value(header["data"].clone()).map_err(|e| format!("ModeData: {e}"))?;
    if !data.ok {
        return Err("reference data not ok".into());
    }
    let dt = header["dt"].as_f64().unwrap();
    let mut m = GameMode::new_reference(data, k.clone(), CrtRand::new(1));
    let mut n_rows = 0;
    let mut pv: Option<mh_mode::views::PlayerView> = None;
    let mut dv: Option<mh_mode::views::DuelView> = None;
    for line in lines {
        let row: Value = parse(line)?;
        let n = row["n"].as_i64().unwrap();
        if n >= 0 {
            for op in row["ops"].as_array().unwrap() {
                apply_op(&mut m, op, &header).map_err(|e| format!("tick {n}: {e}"))?;
            }
            m.tick(dt);
        }
        let drained = m.drain();
        let evs = Value::Array(drained.iter().map(|e| e.to_json()).collect());
        if let Some(d) = diff("events", &row["events"], &evs) {
            return Err(format!("tick {n}: {d}"));
        }
        if n >= 0 {
            // the local player's views (HUD side), as the runner drives MordhauPlayerView / DuelPlayerView
            let mut hud = Vec::new();
            if pv.is_none() {
                if let Some(me) = m.by_name("player") {
                    pv = Some(mh_mode::views::PlayerView::new(me));
                    if matches!(m.ext, ModeExt::Duel(_) | ModeExt::Tf(_)) {
                        dv = Some(mh_mode::views::DuelView::new(me));
                    }
                }
            }
            if let Some(v) = pv.as_mut() {
                v.on_events(&m, &drained);
                v.tick(&m);
                hud.extend(v.drain());
            }
            if let Some(v) = dv.as_mut() {
                let now = m.now;
                v.tick(&m, now, now);
                hud.extend(v.drain());
            }
            let got = Value::Array(hud.iter().map(|h| h.to_json()).collect());
            if let Some(d) = diff("hud", &norm_hud(&row["hud"]), &norm_hud(&got)) {
                return Err(format!("tick {n}: {d}"));
            }
        }
        let mut cpe = Vec::new();
        for cp in m.control_points.iter_mut() {
            cpe.extend(cp.drain().iter().map(|e| e.to_json()));
        }
        if let Some(d) = diff("cp_events", &row["cp_events"], &Value::Array(cpe)) {
            return Err(format!("tick {n}: {d}"));
        }
        if let Some(d) = diff("state", &row["state"], &mode_state(&m)) {
            return Err(format!("tick {n}: {d}"));
        }
        n_rows += 1;
    }
    Ok(n_rows)
}

fn summarize(path: &Path) -> String {
    // the scenario's outcome, for the log: final match state / end info
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let last = text.lines().last().unwrap_or("{}");
    let v: Value = parse(last).unwrap_or(json!({}));
    format!("{} {}", v["state"]["ms"], v["state"]["end"])
}

#[test]
fn golden_modes() {
    let files: Vec<PathBuf> = golden_files("").into_iter().filter(|p| {
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        !n.starts_with("bots_") && !n.starts_with("data_")
    }).collect();
    let Some(k) = kismet() else { return missing("godot/data_gen/mode/mode_kismet.json") };
    if files.is_empty() {
        return missing("core/tests/golden/mode/*.jsonl");
    }
    let mut fails = Vec::new();
    for f in &files {
        match run_mode_file(f, &k) {
            Ok(n) => eprintln!("golden mode {}: {} ticks match ({})", f.file_name().unwrap().to_string_lossy(), n, summarize(f)),
            Err(e) => fails.push(format!("{}: {e}", f.file_name().unwrap().to_string_lossy())),
        }
    }
    assert!(fails.is_empty(), "mode golden mismatches:\n{}", fails.join("\n"));
}

// ==== bots =========================================================================================================
struct ReplayHost {
    reqs: VecDeque<Value>,
    names: Vec<String>,
    err: Option<String>,
}

impl ReplayHost {
    fn take(&mut self, body: BodyId, kind: &str, check: impl Fn(&Value) -> bool) -> PawnView {
        let Some(r) = self.reqs.pop_front() else {
            self.err.get_or_insert(format!("{} requested {kind}, the reference did not", self.names[body]));
            return PawnView::default();
        };
        if r["bot"].as_str() != Some(&self.names[body]) || r["kind"].as_str() != Some(kind) || !check(&r) {
            self.err.get_or_insert(format!("{} requested {kind}, the reference {}", self.names[body], r));
        }
        serde_json::from_value(r["view"].clone()).unwrap_or_default()
    }
}

impl BotHost for ReplayHost {
    fn request_attack(&mut self, body: BodyId, mv: i64, angle: f64) -> PawnView {
        self.take(body, "attack", |r| r["mv"].as_i64() == Some(mv) && r["angle"].as_f64() == Some(angle))
    }
    fn request_parry(&mut self, body: BodyId, bt: i64) -> PawnView {
        self.take(body, "parry", |r| r["bt"].as_i64() == Some(bt))
    }
    fn request_feint(&mut self, body: BodyId) -> PawnView {
        self.take(body, "feint", |_| true)
    }
}

fn pawn_view(v: &Value) -> PawnView {
    serde_json::from_value(v.clone()).expect("pawn view")
}

fn weapon(v: &Value) -> Option<WeaponView> {
    if v.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        None
    } else {
        Some(serde_json::from_value(v.clone()).expect("weapon"))
    }
}

fn bot_out(bots: &Bots, i: usize) -> Value {
    let c = bots.bot(i);
    let b = &bots.bodies;
    let name = |o: Option<usize>| o.map(|x| b[x].name.clone()).unwrap_or_default();
    let mut bb = Map::new();
    for (k, v) in &c.blackboard {
        let j = match v {
            BbVal::Bool(x) => json!({"bool": x}),
            BbVal::Float(x) => json!({"float": x}),
            BbVal::Body(x) => json!({"body": b[*x].name}),
            BbVal::Vec(x) => json!({"vec": v3(*x)}),
        };
        bb.insert(k.clone(), j);
    }
    let lr = match &c.last_request {
        None => json!({}),
        Some(Request::Attack { t, mv, angle }) => json!({"kind": "attack", "t": t, "move": mv, "angle": angle}),
        Some(Request::Parry { t, bt }) => json!({"kind": "parry", "t": t, "bt": bt}),
        Some(Request::Feint { t }) => json!({"kind": "feint", "t": t}),
    };
    let ev: Vec<Value> = c.events.iter().map(|e| json!([e.kind, e.id, e.forced, e.t])).collect();
    let bt = match &c.tree {
        Some(t) => {
            let stack: Vec<Value> = t.stack.iter().map(|f| json!([t.nodes[f.node].name, f.idx])).collect();
            json!([t.active_name(), stack, t.pending, t.pending_result])
        }
        None => json!(["", [], false, -1]),
    };
    let per: Vec<Value> = c
        .perceived
        .iter()
        .map(|(x, i)| json!([b[*x].name, i.sight, i.hearing, i.damage, i.team, i.update_time]))
        .collect();
    let pev: Vec<Value> = c.perception_events.iter().map(|(st, x)| json!([st, b[*x].name])).collect();
    let mv = match &c.move_request {
        Some(r) => json!([v3(r.dest), r.acceptance, r.t]),
        None => json!([]),
    };
    json!({"mr": c.motion_random, "fm": c.facing_mode as i64, "fa": name(c.facing_actor), "fl": v3(c.facing_location),
        "fo": [c.facing_offset[0] as f64, c.facing_offset[1] as f64], "fp": c.facing_param, "fs": c.facing_since,
        "mv": mv, "ps": c.path_status as i64, "mp": c.move_pending, "lr": lr, "ev": ev, "bb": bb, "bt": bt,
        "lce": name(c.last_closest_enemy), "lcet": c.last_closest_enemy_changed_time, "rce": name(c.really_close_enemy),
        "sat": c.closest_enemy_saturated, "rf": c.random_float, "nrf": c.next_random_float_assignment, "per": per,
        "pev": pev, "prof": serde_json::to_value(&c.profile.p).unwrap(),
        "lfm": c.profile.last_footworking_enemy_motion.unwrap_or(0),
        "wc": b[c.body].wants_crouch, "ws": b[c.body].wants_sprint, "lvt": c.last_voice_time})
}

fn ws_json(bots: &Bots) -> Value {
    let f = |x: f64| if x.is_infinite() { Value::Null } else { json!(x) };
    json!([f(bots.ws.last_voice), f(bots.ws.last_emote)])
}

fn run_bots_file(path: &Path, k: &Arc<Kismet>) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut lines = text.lines();
    lines.next();
    let h: Value = parse(lines.next().ok_or("no header")?)?;
    check_consts(&h)?;
    let dt = h["dt"].as_f64().unwrap();
    let mut bots = Bots { team_mode: h["team_mode"].as_bool().unwrap(), k: k.clone(), precision: mh_mode::game_mode::Precision::Reference, ..Default::default() };
    let mut names = Vec::new();
    for b in h["bodies"].as_array().unwrap() {
        let mut body = BotBody::new(b["name"].as_str().unwrap());
        body.team = b["team"].as_i64().unwrap();
        body.location = f3(&b["location"]);
        body.yaw = b["yaw"].as_f64().unwrap();
        body.velocity = f3(&b["velocity"]);
        body.capsule_radius = b["capsule_radius"].as_f64().unwrap();
        body.capsule_half_height = b["capsule_half_height"].as_f64().unwrap();
        body.capsule_scale_min = b["capsule_scale_min"].as_f64().unwrap();
        body.max_walk_speed = b["max_walk_speed"].as_f64().unwrap();
        body.mesh_scale_x = b["mesh_scale_x"].as_f64().unwrap();
        body.weapon_length = b["weapon_length"].as_f64().unwrap();
        body.facing_bone_height = b["facing_bone_height"].as_f64().unwrap();
        body.eye_height = b["eye_height"].as_f64().unwrap();
        body.right_hand = b["right_hand"].as_u64().unwrap() as usize;
        body.weapon = weapon(&b["weapon"]);
        body.inventory = b["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| InventoryItem { weapon: weapon(&it["weapon"]), is_fists: it["is_fists"].as_bool().unwrap(), ranged: it["ranged"].as_bool().unwrap() })
            .collect();
        body.pawn = pawn_view(&b["pawn"]);
        names.push(body.name.clone());
        bots.add_body(body);
    }
    let mut host = ReplayHost { reqs: VecDeque::new(), names: names.clone(), err: None };
    let mut order = Vec::new();
    for bh in h["bots"].as_array().unwrap() {
        let body = names.iter().position(|n| n == bh["body"].as_str().unwrap()).unwrap();
        let prof: Profile = serde_json::from_value(bh["profile"].clone()).map_err(|e| format!("profile: {e}"))?;
        let tree: Option<TreeDef> = if bh["tree"].is_null() { None } else { Some(serde_json::from_value(bh["tree"].clone()).map_err(|e| format!("tree: {e}"))?) };
        let i = bots.add_bot(body, prof, tree.as_ref(), 0.0, &mut host);
        order.push((i, names[body].clone()));
    }
    let check_outs = |bots: &mut Bots, outs: &Value, n: i64| -> Result<(), String> {
        for (i, nm) in &order {
            let got = bot_out(bots, *i);
            if let Some(d) = diff(&format!("out.{nm}"), &outs[nm], &got) {
                return Err(format!("tick {n}: {d}"));
            }
        }
        for i in 0..bots.bots.len() {
            let c = bots.bots[i].as_mut().unwrap();
            c.events.clear();
            c.perception_events.clear();
        }
        Ok(())
    };
    check_outs(&mut bots, &h["out0"], -1)?;
    if let Some(d) = diff("rng", &h["rng"], &json!([bots.rng.seed, bots.rng.calls])) {
        return Err(format!("setup: {d}"));
    }
    let mut n_rows = 0;
    for line in lines {
        let row: Value = parse(line)?;
        let n = row["n"].as_i64().unwrap();
        let now = row["now"].as_f64().unwrap();
        for (i, nm) in names.iter().enumerate() {
            bots.bodies[i].pawn = pawn_view(&row["pre"][nm]);
        }
        host.reqs = row["req"].as_array().unwrap().iter().cloned().collect();
        bots.tick(dt, now, &mut host);
        if let Some(e) = host.err.take() {
            return Err(format!("tick {n}: {e}"));
        }
        if let Some(r) = host.reqs.front() {
            return Err(format!("tick {n}: the reference also requested {r}"));
        }
        check_outs(&mut bots, &row["out"], n)?;
        if let Some(d) = diff("rng", &row["rng"], &json!([bots.rng.seed, bots.rng.calls])) {
            return Err(format!("tick {n}: {d}"));
        }
        if let Some(d) = diff("ws", &row["ws"], &ws_json(&bots)) {
            return Err(format!("tick {n}: {d}"));
        }
        n_rows += 1;
    }
    Ok(n_rows)
}

#[test]
fn golden_bots() {
    let files = golden_files("bots_");
    let Some(k) = kismet() else { return missing("godot/data_gen/mode/mode_kismet.json") };
    if files.is_empty() {
        return missing("core/tests/golden/mode/bots_*.jsonl");
    }
    let mut fails = Vec::new();
    for f in &files {
        match run_bots_file(f, &k) {
            Ok(n) => eprintln!("golden bots {}: {} ticks match", f.file_name().unwrap().to_string_lossy(), n),
            Err(e) => fails.push(format!("{}: {e}", f.file_name().unwrap().to_string_lossy())),
        }
    }
    assert!(fails.is_empty(), "bot golden mismatches:\n{}", fails.join("\n"));
}

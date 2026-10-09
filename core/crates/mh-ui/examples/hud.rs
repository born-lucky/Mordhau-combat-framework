//! Offscreen evidence of the in-match HUD fed with match data through the HUD Blueprints' own entry points, on a duel
//! map (BP_DuelGameMode -> BP_DuelHUD / BP_DuelGameState via GameModeMapPrefixes) and a deathmatch map: players
//! (PlayerStates), kill feed (SendMessageToKillFeed), chat (SendMessageToChatbox), announcement, hit marker and damage
//! indicator (BP_Crosshair), the scoreboard on the real Tab key (the "Show Scoreboard" input action), match result.
//!   sh scripts/cargo.sh run -p mh-ui --example hud [-- MAP]
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::ButtonState;
use bevy::prelude::*;
use mh_ui::evidence::{offscreen_app_sized, run_dir, screenshot};
use mh_ui::game::PlayerInfo;
use mh_ui::real::Rt;
use mh_ui::*;

#[derive(Resource)]
struct Run {
    frame: u32,
    dir: std::path::PathBuf,
    window: Entity,
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let map = a.get(1).cloned().unwrap_or_else(|| "FFA_Arena".into());
    let mut app = offscreen_app_sized(false, (1920, 1080));
    let dir = run_dir(&format!("mh-ui-hud-{map}"));
    let cfg = dir.join("config");
    let _ = std::fs::create_dir_all(&cfg);
    for f in ["GameUserSettings.ini", "Input.ini"] {
        let _ = std::fs::copy(mh_ui::settings::config_dir().join(f), cfg.join(f));
    }
    std::env::set_var("MH_CONFIG_DIR", &cfg);
    let window = app.world_mut().spawn_empty().id();
    app.add_plugins(UiPlugin).insert_resource(MatchMap(map)).insert_resource(Screen::Match).insert_resource(Run { frame: 0, dir, window }).add_systems(Update, script);
    app.run();
}

fn key(world: &mut World, code: KeyCode, state: ButtonState) {
    let window = world.resource::<Run>().window;
    world.write_message(KeyboardInput { key_code: code, logical_key: Key::Unidentified(NativeKey::Unidentified), state, text: None, repeat: false, window });
}

fn players(t: f64) -> Vec<PlayerInfo> {
    vec![
        PlayerInfo { id: 1, name: "Player".into(), team: 0, score: 120.0, kills: 2, deaths: 1, assists: 0, ping_ms: 28, alive: true, local: true },
        PlayerInfo { id: 2, name: "Bot Aldric".into(), team: 1, score: 60.0 + t, kills: 1, deaths: 2, assists: 1, ping_ms: 0, alive: true, local: false },
        PlayerInfo { id: 3, name: "Bot Brannoc".into(), team: 1, score: 0.0, kills: 0, deaths: 1, assists: 0, ping_ms: 0, alive: false, local: false },
    ]
}

fn shot(world: &mut World, name: &str) {
    let dir = world.resource::<Run>().dir.clone();
    screenshot(world, dir.join(format!("{name}.png")));
    if let Some(rt) = world.get_non_send_resource::<Rt>() {
        let mut rows = vec![];
        let mut ids: Vec<_> = rt.ui.out.rects.keys().copied().collect();
        ids.sort();
        for w in ids {
            let o = rt.ui.vm.o(w);
            rows.push(serde_json::json!({"id": w, "name": o.name, "class": o.class.name, "rect": rt.ui.out.rects[&w].map(|x| (x * 10.0).round() / 10.0)}));
        }
        let _ = std::fs::write(dir.join(format!("{name}.layout.json")), serde_json::to_string_pretty(&rows).unwrap());
        println!("shot {name}: mode {:?}, hud {}, items {}", rt.ui.mode, rt.ui.vm.o(rt.ui.hud).class.name, rt.ui.out.items.len());
    }
}

fn with_rt(world: &mut World, f: impl FnOnce(&mut mh_ui::host::UiRuntime)) {
    if let Some(mut rt) = world.get_non_send_resource_mut::<Rt>() {
        f(&mut rt.ui);
    }
}

fn script(world: &mut World) {
    let f = {
        let mut r = world.resource_mut::<Run>();
        r.frame += 1;
        r.frame
    };
    match f {
        5 => with_rt(world, |ui| {
            ui.set_players(&players(0.0));
            ui.set_vitals(64, 82, true);
            ui.set_match_time(431.0);
        }),
        10 => with_rt(world, |ui| {
            ui.kill_feed(Some(1), "Longsword", 3);
            ui.kill_feed(Some(2), "Messer", 1);
            ui.player_chat(2, "Good fight!", false);
            ui.player_chat(1, "gg", true);
            ui.announce("Fight!", "First to 5", 4.0, 0);
        }),
        20 => with_rt(world, |ui| {
            ui.hit_marker(0);
            ui.damage_taken(45.0);
        }),
        24 => shot(world, "01_hud_feed_chat_hit"),
        40 => key(world, KeyCode::Tab, ButtonState::Pressed),
        60 => shot(world, "02_scoreboard_tab_held"),
        70 => key(world, KeyCode::Tab, ButtonState::Released),
        80 => with_rt(world, |ui| ui.match_result(true, "Victory", "Player wins")),
        95 => shot(world, "03_match_result"),
        110 => key(world, KeyCode::Escape, ButtonState::Pressed),
        111 => key(world, KeyCode::Escape, ButtonState::Released),
        140 => shot(world, "04_escape_menu"),
        160 => {
            let missing = world.get_non_send_resource::<Rt>().map(|rt| rt.ui.vm.missing.clone()).unwrap_or_default();
            let dir = world.resource::<Run>().dir.clone();
            let _ = std::fs::write(dir.join("summary.json"), serde_json::to_string_pretty(&serde_json::json!({"missing_natives": missing})).unwrap());
            println!("hud evidence: {}", dir.display());
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

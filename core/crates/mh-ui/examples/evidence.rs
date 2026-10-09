//! mh-ui offscreen evidence: the HUD (status bar after a hit, kill feed, announcement, scoreboard, duel pips) and the
//! main menu, rendered by the GPU into an image and saved under state/runtime_evidence/<date>/<time>-mh-ui/.
//!   sh scripts/cargo.sh run -p mh-ui --example evidence

use bevy::prelude::*;
use mh_mode::views::HudCmd;
use mh_ui::evidence::{offscreen_app, run_dir, screenshot};
use mh_ui::*;

#[derive(Resource)]
struct Run {
    frame: u32,
    dir: std::path::PathBuf,
    log: Vec<serde_json::Value>,
}

fn main() {
    let mut app = offscreen_app(false);
    let dir = run_dir("mh-ui");
    app.add_plugins(UiPlugin).insert_resource(Run { frame: 0, dir, log: vec![] }).add_systems(Update, script);
    app.run();
}

fn script(world: &mut World) {
    let f = {
        let mut r = world.resource_mut::<Run>();
        r.frame += 1;
        r.frame
    };
    match f {
        2 => {
            *world.resource_mut::<HudVitals>() = HudVitals {
                target: Some(1),
                health: 100,
                stamina: 100,
                visible: true,
                wins: [2, 1],
                show_pips: true,
                team_colors: [[0.7, 0.0, 0.0, 1.0], [0.0, 0.1, 0.8, 1.0]],
            };
            world.write_message(HudMsg::Cmd(HudCmd::Announce { text: "Fight!".into(), subtext: "-3-".into(), duration: 5.0, src: "evidence".into() }));
            world.write_message(HudMsg::Cmd(HudCmd::KillFeed {
                killer: "Bot1".into(),
                with: "Longsword".into(),
                victim: "Player".into(),
                killer_color: [1.0, 0.27, 0.22, 1.0],
                victim_color: [0.22, 0.49, 1.0, 1.0],
            }));
            world.write_message(HudMsg::Cmd(HudCmd::KillFeed {
                killer: "Player".into(),
                with: "Kick".into(),
                victim: "Bot2".into(),
                killer_color: [1.0, 0.87, 0.26, 1.0],
                victim_color: [0.36, 0.36, 0.36, 1.0],
            }));
        }
        10 => {
            let mut v = world.resource_mut::<HudVitals>();
            v.health = 55; // a hit: the delayed bar holds the old 100 for the wait, then follows
            v.stamina = 70;
        }
        40 => {
            let mut s = world.resource_mut::<Scoreboard>();
            s.visible = true;
            s.title = "Deathmatch".into();
            s.seconds_left = 431;
            s.rows = vec![
                ScoreRow { name: "Player".into(), score: 120, kills: 2, deaths: 1, local: true },
                ScoreRow { name: "Bot1".into(), score: 60, kills: 1, deaths: 1, local: false },
                ScoreRow { name: "Bot2".into(), score: 0, kills: 0, deaths: 1, local: false },
            ];
        }
        60 => {
            let d = world.resource::<Run>().dir.join("hud.png");
            screenshot(world, d);
            record(world, "hud");
        }
        90 => {
            world.resource_mut::<HudVitals>().visible = false;
            world.resource_mut::<Scoreboard>().visible = false;
            let mut m = world.resource_mut::<MainMenu>();
            m.visible = true;
            m.modes = mh_mode::mode_table::rows().iter().map(|r| r.id.to_string()).collect();
            m.map_label = "Arena".into();
        }
        110 => {
            let d = world.resource::<Run>().dir.join("menu.png");
            screenshot(world, d);
            record(world, "menu");
        }
        170 => {
            let r = world.resource::<Run>();
            let dir = r.dir.clone();
            let ok = ["hud.png", "menu.png"].iter().all(|p| dir.join(p).exists());
            let summary = serde_json::json!({"plugin": "mh-ui", "frames": f, "screenshots_written": ok, "records": r.log});
            let _ = std::fs::write(dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap());
            println!("mh-ui evidence: {} (screenshots {})", dir.display(), ok);
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

fn record(world: &mut World, what: &str) {
    let s = world.resource::<UiStats>().clone();
    let v = serde_json::json!({"shot": what, "items": s.items, "textures": s.textures, "missing_textures": s.missing, "fonts": s.fonts});
    world.resource_mut::<Run>().log.push(v);
}

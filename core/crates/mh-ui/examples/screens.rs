//! Offscreen evidence of the real UMG screens (no window): the menu map's front end driven by real input events
//! through the widgets' own Blueprint logic, then a match HUD, the scoreboard and the Escape menu. For each shot it
//! writes <name>.png (GPU render) and <name>.layout.json (every painted widget's rect as laid out from the asset:
//! name, class, x, y, w, h in pixels) under state/runtime_evidence/<date>/<time>-mh-ui-screens/.
//!   sh scripts/cargo.sh run -p mh-ui --example screens
use bevy::prelude::*;
use mh_ui::evidence::{offscreen_app, run_dir, screenshot};
use mh_ui::real::Rt;
use mh_ui::*;

#[derive(Resource)]
struct Run {
    frame: u32,
    dir: std::path::PathBuf,
    log: Vec<serde_json::Value>,
    actions: Vec<String>,
}

fn main() {
    let mut app = offscreen_app(false);
    let dir = run_dir("mh-ui-screens");
    // the user's real GameUserSettings.ini, copied: the settings screen reads it, the evidence run never writes the
    // live file
    let cfg = dir.join("config");
    let _ = std::fs::create_dir_all(&cfg);
    let real = mh_ui::settings::config_dir();
    for f in ["GameUserSettings.ini", "Input.ini"] {
        let _ = std::fs::copy(real.join(f), cfg.join(f));
    }
    std::env::set_var("MH_CONFIG_DIR", &cfg);
    app.add_plugins(UiPlugin).insert_resource(Run { frame: 0, dir, log: vec![], actions: vec![] }).insert_resource(Screen::MainMenu).add_systems(Update, script);
    app.run();
}

fn centre(world: &World, name: &str) -> Option<Vec2> {
    let rt = world.get_non_send_resource::<Rt>()?;
    let w = rt.ui.find_any(name)?;
    rt.ui.centre(w).map(|c| Vec2::new(c[0] as f32, c[1] as f32))
}

fn click(world: &mut World, name: &str) -> bool {
    let Some(c) = centre(world, name) else {
        println!("click {name}: not painted");
        return false;
    };
    world.write_message(UiInput::Move(c));
    world.write_message(UiInput::Down("LeftMouseButton".into()));
    world.write_message(UiInput::Up("LeftMouseButton".into()));
    true
}

fn shot(world: &mut World, name: &str) {
    let dir = world.resource::<Run>().dir.clone();
    screenshot(world, dir.join(format!("{name}.png")));
    let Some(rt) = world.get_non_send_resource::<Rt>() else { return };
    let mut rows = vec![];
    let mut ids: Vec<_> = rt.ui.out.rects.keys().copied().collect();
    ids.sort();
    for w in ids {
        let r = rt.ui.out.rects[&w];
        let o = rt.ui.vm.o(w);
        rows.push(serde_json::json!({"id": w, "name": o.name, "class": o.class.name, "native": o.class.native(), "rect": r.map(|x| (x * 10.0).round() / 10.0)}));
    }
    let _ = std::fs::write(dir.join(format!("{name}.layout.json")), serde_json::to_string_pretty(&rows).unwrap());
    let rec = serde_json::json!({"shot": name, "items": rt.ui.out.items.len(), "painted_widgets": rows.len(), "scale": rt.ui.out.scale,
        "main_menu_visible": rt.ui.main_menu_visible(), "missing_natives": rt.ui.vm.missing.len()});
    world.resource_mut::<Run>().log.push(rec);
}

fn script(world: &mut World) {
    let f = {
        let mut r = world.resource_mut::<Run>();
        r.frame += 1;
        r.frame
    };
    let acts: Vec<String> = world.resource_mut::<Messages<UiAction>>().drain().map(|a| format!("{a:?}")).collect();
    world.resource_mut::<Run>().actions.extend(acts);
    match f {
        30 => shot(world, "01_launch"),
        70 => shot(world, "02_home"),
        80 => {
            click(world, "PlayButton");
        }
        110 => shot(world, "03_play"),
        120 => {
            click(world, "LocalMatchButton");
        }
        150 => shot(world, "04_local_play"),
        152 => {
            // pick a map tile, then Start Match: BP_LocalPlay -> GameInstance.ClientTravel -> UiAction::OpenLevel
            let c = world.get_non_send_resource::<Rt>().and_then(|rt| {
                let lp = rt.ui.find_any("BP_LocalPlay")?;
                let ml = rt.ui.vm.child(lp, "BP_MapList")?;
                let g = rt.ui.vm.child(ml, "EntryGrid")?;
                let first = rt.ui.vm.o(*rt.ui.vm.o(g).slots.first()?).content?;
                rt.ui.centre(first)
            });
            println!("map tile centre {c:?}");
            if let Some(c) = c {
                world.write_message(UiInput::Move(Vec2::new(c[0] as f32, c[1] as f32)));
                world.write_message(UiInput::Down("LeftMouseButton".into()));
                world.write_message(UiInput::Up("LeftMouseButton".into()));
            }
        }
        155 => {
            if let Some(rt) = world.get_non_send_resource::<Rt>() {
                let lp = rt.ui.find_any("BP_LocalPlay").unwrap();
                let ml = rt.ui.vm.child(lp, "BP_MapList").unwrap();
                println!("selected entry before Start: {:?}", rt.ui.vm.prop(ml, "SelectedEntry"));
            }
            click(world, "StartButton");
        }
        160 => {
            click(world, "SettingsButton");
        }
        190 => shot(world, "05_settings"),
        192 => {
            click(world, "VideoButton");
        }
        205 => shot(world, "05b_settings_video"),
        207 => {
            click(world, "AudioButton");
        }
        220 => shot(world, "05c_settings_audio"),
        222 => {
            click(world, "ControlsButton");
        }
        235 => shot(world, "05d_settings_controls"),
        237 => {
            click(world, "KeyBindingsButton");
        }
        250 => shot(world, "05e_settings_bindings"),
        255 => {
            *world.resource_mut::<MatchMap>() = MatchMap("Contraband".into());
            *world.resource_mut::<Screen>() = Screen::Match;
        }
        295 => {
            if let Some(mut rt) = world.get_non_send_resource_mut::<Rt>() {
                rt.ui.set_vitals(72, 55, true);
            }
        }
        325 => shot(world, "06_hud"),
        335 => {
            if let Some(mut rt) = world.get_non_send_resource_mut::<Rt>() {
                let h = rt.ui.hud;
                rt.ui.vm.event(h, "ShowScoreboard", vec![]);
            }
        }
        355 => shot(world, "07_scoreboard"),
        365 => {
            if let Some(mut rt) = world.get_non_send_resource_mut::<Rt>() {
                let h = rt.ui.hud;
                rt.ui.vm.event(h, "HideScoreboard", vec![]);
            }
            world.write_message(UiInput::Key("Escape".into()));
        }
        395 => shot(world, "08_escape_menu"),
        455 => {
            let r = world.resource::<Run>();
            let dir = r.dir.clone();
            let missing = world.get_non_send_resource::<Rt>().map(|rt| rt.ui.vm.missing.clone()).unwrap_or_default();
            let r = world.resource::<Run>();
            let summary = serde_json::json!({"plugin": "mh-ui real screens", "frames": f, "records": r.log, "actions": r.actions, "missing_natives": missing});
            let _ = std::fs::write(dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap());
            println!("mh-ui screens evidence: {}", dir.display());
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

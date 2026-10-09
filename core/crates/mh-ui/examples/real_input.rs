//! The front end driven by REAL Bevy input messages (MouseButtonInput / KeyboardInput) and a moving cursor, with human
//! timing (the cursor glides to each target over ~20 frames, ~1 s between clicks), at an emulated window size: title ->
//! fight -> local match -> a map tile -> Start Match must emit UiAction::OpenLevel. Offscreen (no window): the cursor
//! comes in through UiCursor, as the window's cursor_position would.
//!   sh scripts/cargo.sh run -p mh-ui --example real_input [-- WIDTH HEIGHT]
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::ButtonState;
use bevy::prelude::*;
use mh_ui::evidence::{offscreen_app_sized, run_dir, screenshot};
use mh_ui::real::Rt;
use mh_ui::*;

#[derive(Resource)]
struct Run {
    frame: u32,
    dir: std::path::PathBuf,
    steps: Vec<Step>,
    at: usize,
    wait: u32,
    cursor: Vec2,
    glide: Option<(Vec2, Vec2, u32)>,
    log: Vec<String>,
    opened: Option<String>,
    no_click: bool,
    window: Entity,
}

#[derive(Clone, Debug)]
enum Step {
    Wait(u32),
    Key(KeyCode),
    /// glide to a widget (path "A/B/C#n": nested variable names, #n = n-th panel child) and click it
    Click(&'static str),
    Shot(&'static str),
    /// mouse wheel notches (MouseWheel, Line units) at the current cursor; negative = scroll down
    Wheel(f32),
    /// glide to a point given as fractions of the viewport (no click)
    MoveTo(f32, f32),
    /// click the open combo menu's row showing this text
    ClickMenuRow(&'static str),
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let w: u32 = a.get(1).and_then(|x| x.parse().ok()).unwrap_or(1920);
    let h: u32 = a.get(2).and_then(|x| x.parse().ok()).unwrap_or(1080);
    let mut app = offscreen_app_sized(false, (w, h));
    let dir = run_dir(&format!("mh-ui-real-input-{w}x{h}"));
    let cfg = dir.join("config");
    let _ = std::fs::create_dir_all(&cfg);
    for f in ["GameUserSettings.ini", "Input.ini"] {
        let _ = std::fs::copy(mh_ui::settings::config_dir().join(f), cfg.join(f));
    }
    std::env::set_var("MH_CONFIG_DIR", &cfg);
    let window = app.world_mut().spawn_empty().id();
    use Step::*;
    let steps = vec![
        Wait(40),
        // a PC launch lands on Home (no title screen: UMordhauGameInstance ctor 0x1528830 pairs the controller)
        Shot("01_launch_home"),
        Click("PlayButton"),
        Wait(30),
        Click("LocalMatchButton"),
        Wait(40),
        Shot("02_local_play"),
        // the Game Mode dropdown: open it, pick Team Deathmatch from its menu (map list refreshes for TDM maps)
        Click("BP_LocalPlay/GameModeComboBox"),
        Wait(20),
        Shot("02b_mode_menu_open"),
        ClickMenuRow("Team Deathmatch"),
        Wait(30),
        // scroll the map grid with the wheel, then pick a map that was below the fold
        MoveTo(0.3, 0.6),
        Wait(25),
        Wheel(-3.0),
        Wait(10),
        Wheel(-3.0),
        Wait(20),
        Shot("02c_scrolled"),
        Click("BP_LocalPlay/BP_MapList/EntryGrid#7"),
        Wait(40),
        Shot("03_tile_selected"),
        Click("BP_LocalPlay/StartButton"),
        Wait(30),
        Shot("04_after_start"),
    ];
    app.add_plugins(UiPlugin)
        .insert_resource(Screen::MainMenu)
        .insert_resource(UiCursor(Some(Vec2::new(w as f32 * 0.5, h as f32 * 0.5))))
        .insert_resource(Run { frame: 0, dir, steps, at: 0, wait: 0, cursor: Vec2::new(w as f32 * 0.5, h as f32 * 0.5), glide: None, log: vec![], opened: None, no_click: false, window })
        .add_systems(Update, drive);
    app.run();
}

fn widget_centre(world: &World, path: &str) -> Option<Vec2> {
    let rt = world.get_non_send_resource::<Rt>()?;
    let mut parts = path.split('/');
    let mut w = rt.ui.find_any(parts.next()?)?;
    for p in parts {
        let (name, idx) = match p.split_once('#') {
            Some((n, i)) => (n, i.parse::<usize>().ok()),
            None => (p, None),
        };
        w = rt.ui.vm.child(w, name)?;
        if let Some(i) = idx {
            let s = *rt.ui.vm.o(w).slots.get(i)?;
            w = rt.ui.vm.o(s).content?;
        }
    }
    rt.ui.centre(w).map(|c| Vec2::new(c[0] as f32, c[1] as f32))
}

fn mouse(world: &mut World, state: ButtonState) {
    let window = world.resource::<Run>().window;
    world.write_message(MouseButtonInput { button: MouseButton::Left, state, window });
}

fn key(world: &mut World, code: KeyCode, state: ButtonState) {
    let window = world.resource::<Run>().window;
    world.write_message(KeyboardInput { key_code: code, logical_key: Key::Unidentified(NativeKey::Unidentified), state, text: None, repeat: false, window });
}

fn drive(world: &mut World) {
    let f = {
        let mut r = world.resource_mut::<Run>();
        r.frame += 1;
        r.frame
    };
    for a in world.resource_mut::<Messages<UiAction>>().drain().collect::<Vec<_>>() {
        let s = format!("frame {f}: {a:?}");
        println!("{s}");
        let mut r = world.resource_mut::<Run>();
        if let UiAction::OpenLevel { map, options } = &a {
            r.opened = Some(format!("{map}?{options}"));
        }
        r.log.push(s);
    }
    // cursor glide in progress: move toward the target, then press (one frame) and release (next frames)
    let glide = world.resource::<Run>().glide;
    if let Some((from, to, t)) = glide {
        let n = 20;
        let p = from.lerp(to, (t as f32 / n as f32).min(1.0));
        world.resource_mut::<UiCursor>().0 = Some(p);
        world.resource_mut::<Run>().cursor = p;
        if t >= n && world.resource::<Run>().no_click {
            let mut r = world.resource_mut::<Run>();
            r.glide = None;
            r.no_click = false;
            return;
        }
        match t {
            x if x < n => world.resource_mut::<Run>().glide = Some((from, to, t + 1)),
            x if x == n + 2 => mouse(world, ButtonState::Pressed),
            x if x == n + 8 => {
                mouse(world, ButtonState::Released);
                world.resource_mut::<Run>().glide = None;
                return;
            }
            _ => {}
        }
        if let Some(g) = world.resource_mut::<Run>().glide.as_mut() {
            if g.2 >= n {
                g.2 += 1;
            }
        }
        return;
    }
    {
        let mut r = world.resource_mut::<Run>();
        if r.wait > 0 {
            r.wait -= 1;
            return;
        }
    }
    let (at, step) = {
        let r = world.resource::<Run>();
        (r.at, r.steps.get(r.at).cloned())
    };
    let Some(step) = step else {
        if f > 2000 || world.resource::<Run>().wait == 0 {
            let r = world.resource::<Run>();
            let summary = serde_json::json!({"ok": r.opened.is_some(), "open_level": r.opened, "log": r.log});
            let _ = std::fs::write(r.dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap());
            println!("real input: OpenLevel {:?} -> {}", r.opened, r.dir.display());
            world.write_message(AppExit::Success);
        }
        return;
    };
    world.resource_mut::<Run>().at = at + 1;
    match step {
        Step::Wait(n) => world.resource_mut::<Run>().wait = n,
        Step::Wheel(d) => {
            let window = world.resource::<Run>().window;
            world.write_message(MouseWheel { unit: MouseScrollUnit::Line, x: 0.0, y: d, window, phase: bevy::input::touch::TouchPhase::Moved });
            world.resource_mut::<Run>().wait = 2;
        }
        Step::MoveTo(fx, fy) => {
            let vp = world.resource::<UiViewport>().0;
            let from = world.resource::<Run>().cursor;
            let to = Vec2::new(vp.x * fx, vp.y * fy);
            // a glide without the click: stop it at the end
            world.resource_mut::<Run>().glide = Some((from, to, 0));
            world.resource_mut::<Run>().no_click = true;
        }
        Step::ClickMenuRow(text) => {
            let c = world.get_non_send_resource::<Rt>().and_then(|rt| {
                rt.ui.out.popup_rows.iter().find(|(_, c, i)| rt.ui.vm.prop(*c, "DefaultOptions").arr().get(*i).map(|v| v.s()) == Some(text.to_string())).map(|(r, _, _)| Vec2::new((r[0] + r[2] * 0.5) as f32, (r[1] + r[3] * 0.5) as f32))
            });
            match c {
                Some(c) => {
                    println!("frame {f}: menu row {text} at {c:?}");
                    let from = world.resource::<Run>().cursor;
                    world.resource_mut::<Run>().glide = Some((from, c, 0));
                }
                None => println!("frame {f}: menu row {text}: NOT OPEN"),
            }
        }
        Step::Key(k) => {
            key(world, k, ButtonState::Pressed);
            key(world, k, ButtonState::Released);
            world.resource_mut::<Run>().wait = 2;
        }
        Step::Click(path) => match widget_centre(world, path) {
            Some(c) => {
                let from = world.resource::<Run>().cursor;
                println!("frame {f}: click {path} at {c:?}");
                world.resource_mut::<Run>().glide = Some((from, c, 0));
            }
            None => {
                println!("frame {f}: click {path}: NOT PAINTED");
                world.resource_mut::<Run>().log.push(format!("{path} not painted"));
            }
        },
        Step::Shot(n) => {
            let d = world.resource::<Run>().dir.join(format!("{n}.png"));
            screenshot(world, d);
            if let Some(rt) = world.get_non_send_resource::<Rt>() {
                let sel = rt.ui.find_any("BP_LocalPlay").and_then(|lp| rt.ui.vm.child(lp, "BP_MapList")).map(|ml| rt.ui.vm.prop(ml, "SelectedEntry"));
                println!("frame {f}: shot {n}; hovered {:?} selected entry {:?}", rt.ui.hovered.map(|h| rt.ui.vm.o(h).name.clone()), sel);
            }
            world.resource_mut::<Run>().wait = 5;
        }
    }
}

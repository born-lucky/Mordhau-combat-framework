//! The Armory -> Mercenaries screens driven by REAL Bevy input (cursor glide + MouseButtonInput / KeyboardInput) at
//! 1920x1080 offscreen, on a fresh config dir (settings::config_dir() via MH_CONFIG_DIR): open Mercenaries, pick a
//! different mercenary, change its weapon, save; then in a match (Escape -> Mercenaries) pick a mercenary and close,
//! which must emit mh_ui::armory::SpawnProfile. Screenshots + summary.json (ArmoryPreview per shot, SpawnProfile
//! messages, the saved Game.ini) go to state/runtime_evidence/<date>/<time>-mh-ui-armory/.
//!   sh scripts/cargo.sh run -p mh-ui --example armory [-- steps.txt | match]
//! A steps file overrides the built-in steps: one per line, `click <widget path>`, `text <visible text>`,
//! `entry <list widget> <n>`, `nth <widget class prefix> <n>`, `key <KeyCode>`, `wait <frames>`, `shot <name>`, `match <map name>`.
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::mouse::MouseButtonInput;
use bevy::input::ButtonState;
use bevy::prelude::*;
use mh_ui::armory::{ArmoryPreview, SpawnProfile};
use mh_ui::evidence::{offscreen_app_sized, run_dir, screenshot};
use mh_ui::real::Rt;
use mh_ui::*;

#[derive(Resource)]
struct Run {
    frame: u32,
    dir: std::path::PathBuf,
    steps: Vec<(String, String)>,
    at: usize,
    wait: u32,
    cursor: Vec2,
    glide: Option<(Vec2, Vec2, u32)>,
    log: Vec<serde_json::Value>,
    spawns: Vec<serde_json::Value>,
    window: Entity,
}

const DEFAULT_STEPS: &str = "wait 30
click ArmoryButton
wait 30
click BP_PromptButton_Mercenaries
wait 50
shot 01_mercenaries
nth BP_LoadoutEntry 2
wait 30
shot 02_picked_veteran
nth BP_LoadoutEntry 9
wait 30
shot 03_picked_custom
nth BP_BreakdownEquipmentEntry 0
wait 40
shot 04_equipment_list
nth BP_EquipmentItemEntry 3
wait 30
shot 05_weapon_picked
text Back
wait 30
nth BP_BreakdownArmorEntry 0
wait 40
shot 06_armor
nth BP_WearableItemEntry 2
wait 40
shot 07_wearable_list
nth BP_WearableItemEntry 4
wait 30
shot 08_wearable_picked
";

/// the in-match pick (run with a steps file): Escape -> Mercenaries -> pick -> Play emits SpawnProfile
#[allow(dead_code)]
const MATCH_STEPS: &str = "match FFA_Arena
wait 60
key Escape
wait 30
click ArmoryButton
wait 30
click BP_PromptButton_Mercenaries
wait 50
nth BP_LoadoutEntry 1
wait 30
shot 10_match_picked
text Play
wait 30
shot 11_after_play
";

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut app = offscreen_app_sized(false, (1920, 1080));
    let dir = run_dir("mh-ui-armory");
    let cfg = dir.join("config");
    let _ = std::fs::create_dir_all(&cfg);
    std::env::set_var("MH_CONFIG_DIR", &cfg);
    let window = app.world_mut().spawn_empty().id();
    let text = match a.get(1).map(String::as_str) {
        Some("match") => MATCH_STEPS.to_string(),
        p => p.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_else(|| DEFAULT_STEPS.to_string()),
    };
    let steps = text.lines().filter_map(|l| l.trim().split_once(' ').map(|(k, v)| (k.to_string(), v.trim().to_string()))).collect();
    app.add_plugins(UiPlugin)
        .insert_resource(Screen::MainMenu)
        .insert_resource(UiCursor(Some(Vec2::new(960.0, 540.0))))
        .insert_resource(Run { frame: 0, dir, steps, at: 0, wait: 0, cursor: Vec2::new(960.0, 540.0), glide: None, log: vec![], spawns: vec![], window })
        .add_systems(Update, drive);
    app.run();
}

/// a painted widget's centre: by nested path ("A/B"), searched from the newest viewport root down
fn by_path(rt: &Rt, path: &str) -> Option<Vec2> {
    let ui = &rt.ui;
    let mut parts = path.split('/');
    let first = parts.next()?;
    let mut w = ui.vm.viewport.iter().rev().find_map(|&(r, _)| ui.find(r, first).filter(|&w| ui.painted(w)))?;
    for p in parts {
        w = ui.vm.child(w, p)?;
    }
    ui.centre(w).map(|c| Vec2::new(c[0] as f32, c[1] as f32))
}

/// the painted widget showing `text` (TextBlock Text, case-insensitive), topmost root first
fn by_text(rt: &Rt, text: &str) -> Option<Vec2> {
    let ui = &rt.ui;
    for &(r, _) in ui.vm.viewport.iter().rev() {
        let mut d = vec![];
        ui.vm.descendants(r, &mut d);
        for w in d.into_iter().rev() {
            if ui.painted(w) && ui.vm.prop(w, "Text").s().eq_ignore_ascii_case(text) {
                return ui.centre(w).map(|c| Vec2::new(c[0] as f32, c[1] as f32));
            }
        }
    }
    None
}

/// the n-th entry widget of a painted list / panel (its slot children in order)
fn entry(rt: &Rt, list: &str, n: usize) -> Option<Vec2> {
    let ui = &rt.ui;
    let l = ui.vm.viewport.iter().rev().find_map(|&(r, _)| ui.find(r, list).filter(|&w| ui.painted(w)))?;
    // a ListView's entry widgets (natives.rs list_add keeps them in __entries), else a panel's slot children
    let c = match ui.vm.prop(l, "__entries").arr().get(n).and_then(|v| v.obj()) {
        Some(e) => e,
        None => ui.vm.o(*ui.vm.o(l).slots.get(n)?).content?,
    };
    ui.centre(c).map(|c| Vec2::new(c[0] as f32, c[1] as f32))
}

/// the n-th painted widget whose class name starts with `cls` (in paint order), e.g. BP_LoadoutEntry 4
fn nth(rt: &Rt, cls: &str, n: usize) -> Option<Vec2> {
    let ui = &rt.ui;
    // every live painted widget of that class, in reading order (top to bottom, then left to right)
    let mut found: Vec<(i64, i64, u32)> = (1..ui.vm.objs.len() as u32)
        .filter(|&w| ui.vm.alive(w) && ui.vm.o(w).class.name.starts_with(cls) && ui.painted(w))
        .filter_map(|w| ui.out.rects.get(&w).map(|r| (r[1].round() as i64, r[0].round() as i64, w)))
        .collect();
    found.sort();
    let w = found.get(n)?.2;
    ui.centre(w).map(|c| Vec2::new(c[0] as f32, c[1] as f32))
}

fn state(world: &World) -> serde_json::Value {
    let p = world.resource::<ArmoryPreview>().clone();
    let Some(rt) = world.get_non_send_resource::<Rt>() else { return serde_json::json!({}) };
    let ui = &rt.ui;
    let picker = ui.find_any("BP_LoadoutPicker");
    let sel = picker.map(|pk| ui.vm.prop(pk, "SelectedId").i());
    let wrapper = ui.find_any("BP_MordhauProfileCustomization").and_then(|pc| ui.vm.prop(pc, "ProfileWrapper").obj());
    let prof = wrapper.map(|w| ui.vm.prop(w, "Profile")).unwrap_or_default();
    let active = |sw: &str| -> String {
        ui.find_any(sw)
            .and_then(|s| {
                let i = ui.vm.prop(s, "ActiveWidgetIndex").i().max(0) as usize;
                let slot = *ui.vm.o(s).slots.get(i)?;
                ui.vm.o(slot).content.map(|c| ui.vm.o(c).name.clone())
            })
            .unwrap_or_default()
    };
    serde_json::json!({
        "main": active("WidgetSwitcher_Main"),
        "panel": active("CustomizationPanelSwitcher"),
        "selected_id": sel,
        "editing": prof.field("Name").s(),
        "editing_equipment": prof.field("GearCustomization").field("Equipment").arr().iter().map(|e| e.field("Id").i()).collect::<Vec<_>>(),
        "preview": {"active": p.active, "doll": p.doll_location, "doll_rot": p.doll_rotation, "camera": p.camera_location, "camera_rot": p.camera_rotation, "fov": p.fov, "profile": p.profile.as_ref().and_then(|j| j.pointer("/Name/SourceString")).cloned()},
        "missing_natives": ui.vm.missing.keys().cloned().collect::<Vec<_>>(),
    })
}


fn drive(world: &mut World) {
    let f = {
        let mut r = world.resource_mut::<Run>();
        r.frame += 1;
        r.frame
    };
    let sp: Vec<SpawnProfile> = world.resource_mut::<Messages<SpawnProfile>>().drain().collect();
    for s in sp {
        println!("frame {f}: SpawnProfile {} {}", s.index, s.name);
        world.resource_mut::<Run>().spawns.push(serde_json::json!({"frame": f, "index": s.index, "name": s.name, "equipment": s.profile.pointer("/GearCustomization/Equipment")}));
    }
    let acts: Vec<String> = world.resource_mut::<Messages<UiAction>>().drain().map(|a| format!("{a:?}")).collect();
    for a in acts {
        println!("frame {f}: {a}");
        world.resource_mut::<Run>().log.push(serde_json::json!({"frame": f, "action": a}));
    }
    if let Some((from, to, t)) = world.resource::<Run>().glide {
        let n = 20;
        let p = from.lerp(to, (t as f32 / n as f32).min(1.0));
        world.resource_mut::<UiCursor>().0 = Some(p);
        world.resource_mut::<Run>().cursor = p;
        let window = world.resource::<Run>().window;
        if t == n + 2 {
            world.write_message(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window });
        }
        if t == n + 8 {
            world.write_message(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Released, window });
            world.resource_mut::<Run>().glide = None;
            return;
        }
        world.resource_mut::<Run>().glide = Some((from, to, t + 1));
        return;
    }
    {
        let mut r = world.resource_mut::<Run>();
        if r.wait > 0 {
            r.wait -= 1;
            return;
        }
    }
    let step = {
        let r = world.resource::<Run>();
        r.steps.get(r.at).cloned()
    };
    let Some((kind, arg)) = step else {
        let ini = std::fs::read_to_string(mh_ui::armory::game_ini()).unwrap_or_default();
        let r = world.resource::<Run>();
        let summary = serde_json::json!({"spawns": r.spawns, "log": r.log, "game_ini": ini});
        let _ = std::fs::write(r.dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap());
        println!("armory: {} spawn(s) -> {}", r.spawns.len(), r.dir.display());
        world.write_message(AppExit::Success);
        return;
    };
    world.resource_mut::<Run>().at += 1;
    let target = |world: &World| -> Option<Vec2> {
        let rt = world.get_non_send_resource::<Rt>()?;
        match kind.as_str() {
            "click" => by_path(rt, &arg),
            "text" => by_text(rt, &arg),
            "entry" => {
                let (l, n) = arg.rsplit_once(' ')?;
                entry(rt, l, n.parse().ok()?)
            }
            "nth" => {
                let (c, n) = arg.rsplit_once(' ')?;
                nth(rt, c, n.parse().ok()?)
            }
            _ => None,
        }
    };
    match kind.as_str() {
        "wait" => world.resource_mut::<Run>().wait = arg.parse().unwrap_or(10),
        "key" => {
            let window = world.resource::<Run>().window;
            let code = match arg.as_str() {
                "Escape" => KeyCode::Escape,
                "Enter" => KeyCode::Enter,
                _ => KeyCode::Space,
            };
            for st in [ButtonState::Pressed, ButtonState::Released] {
                world.write_message(KeyboardInput { key_code: code, logical_key: Key::Unidentified(NativeKey::Unidentified), state: st, text: None, repeat: false, window });
            }
            world.resource_mut::<Run>().wait = 2;
        }
        "match" => {
            // the front end's OpenLevel lands in a match: the UI remounts for that map (mh-runtime menu.rs does this)
            world.insert_resource(MatchMap(arg.clone()));
            world.insert_resource(Screen::Match);
            world.resource_mut::<Run>().log.push(serde_json::json!({"frame": f, "match": arg}));
        }
        "shot" => {
            let d = world.resource::<Run>().dir.join(format!("{arg}.png"));
            screenshot(world, d);
            let s = state(world);
            println!("frame {f}: shot {arg} {s}");
            world.resource_mut::<Run>().log.push(serde_json::json!({"frame": f, "shot": arg, "state": s}));
            world.resource_mut::<Run>().wait = 5;
        }
        _ => match target(world) {
            Some(c) => {
                let from = world.resource::<Run>().cursor;
                println!("frame {f}: {kind} {arg} at {c:?}");
                world.resource_mut::<Run>().glide = Some((from, c, 0));
            }
            None => {
                println!("frame {f}: {kind} {arg}: NOT PAINTED");
                world.resource_mut::<Run>().log.push(serde_json::json!({"frame": f, "not_painted": format!("{kind} {arg}")}));
            }
        },
    }
}

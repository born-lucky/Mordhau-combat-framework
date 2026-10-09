//! Real-input QA pass over every reachable front-end screen: each top-bar nav button and each of its sub-nav tabs is
//! clicked with real Bevy input (cursor glide + MouseButtonInput) at 1920x1080 on a fresh config dir; per screen a
//! screenshot, the active content widget, the sub-nav shown, natives first missed on it, VM step-budget overruns and
//! UI actions go to qa.json under state/runtime_evidence/<date>/<time>-mh-ui-qa/.
//!   sh scripts/cargo.sh run -p mh-ui --example qa
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::mouse::MouseButtonInput;
use bevy::input::ButtonState;
use bevy::prelude::*;
use mh_ui::evidence::{offscreen_app_sized, run_dir, screenshot};
use mh_ui::real::Rt;
use mh_ui::*;

/// (top nav button, sub-nav buttons to visit under it)
const NAV: &[(&str, &[&str])] = &[
    ("HomeButton", &[]),
    ("PlayButton", &["TrainingButton", "LocalMatchButton", "MatchmakingButton", "ServerBrowserButton"]),
    ("ArmoryButton", &[]),
    ("PlayerButton", &["PlayerSubButton"]),
    ("FriendsButton", &[]),
    ("MiscButton", &["SocialButton", "CreditsButton"]),
    ("SettingsButton", &["GameButton", "VideoButton", "AudioButton", "ControlsButton", "KeyBindingsButton", "ControlsLayoutButton", "ModsButton"]),
    ("QuitButton", &["QuitSubNavButton"]),
];

#[derive(Resource)]
struct Run {
    frame: u32,
    dir: std::path::PathBuf,
    window: Entity,
    steps: Vec<(String, String)>,
    at: usize,
    wait: u32,
    cursor: Vec2,
    glide: Option<(Vec2, Vec2, u32)>,
    records: Vec<serde_json::Value>,
    seen_missing: std::collections::BTreeSet<String>,
    seen_budget: usize,
}

fn main() {
    // QA_SIZE=1280x720: the emulated window size (default 1920x1080)
    let size = std::env::var("QA_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).unwrap_or((1920u32, 1080u32));
    let mut app = offscreen_app_sized(false, size);
    let dir = run_dir("mh-ui-qa");
    let cfg = dir.join("config");
    let _ = std::fs::create_dir_all(&cfg);
    std::env::set_var("MH_CONFIG_DIR", &cfg);
    let window = app.world_mut().spawn_empty().id();
    // ("key", name) | ("click", path) | ("shot", label) | ("wait", n)
    let mut steps: Vec<(String, String)> = vec![("wait".into(), "40".into()), ("shot".into(), "launch".into())];
    // QA_ONLY=SettingsButton[,PlayButton]: only those top-nav screens (short offscreen runs)
    // QA_WAIT=<frames>: frames waited after each click (default 30)
    let wait = std::env::var("QA_WAIT").unwrap_or_else(|_| "30".into());
    let only: Vec<String> = std::env::var("QA_ONLY").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_default();
    for (top, subs) in NAV {
        if !only.is_empty() && !only.iter().any(|o| o == top) {
            continue;
        }
        steps.push(("click".into(), top.to_string()));
        steps.push(("wait".into(), wait.clone()));
        steps.push(("shot".into(), top.to_string()));
        for s in *subs {
            steps.push(("click".into(), s.to_string()));
            steps.push(("wait".into(), wait.clone()));
            steps.push(("shot".into(), format!("{top}.{s}")));
            // online-only screens answer offline with the game's information dialog: dismiss it with its Ok
            if matches!(*s, "MatchmakingButton" | "ServerBrowserButton") {
                steps.push(("click".into(), "BP_PromptButton_OneButtonDialog".into()));
                steps.push(("wait".into(), "20".into()));
                steps.push(("shot".into(), format!("{top}.{s}.dismissed")));
            }
        }
    }
    // QA_HOVER=PlayButton,GameButton,...: afterwards, move the cursor onto each (no click) and shoot the hovered state
    for h in std::env::var("QA_HOVER").map(|v| v.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default() {
        steps.push(("hover".into(), h.clone()));
        steps.push(("wait".into(), "12".into()));
        steps.push(("shot".into(), format!("hover.{h}")));
    }
    app.add_plugins(UiPlugin)
        .insert_resource(Screen::MainMenu)
        .insert_resource(UiCursor(Some(Vec2::new(960.0, 540.0))))
        .insert_resource(Run { frame: 0, dir, window, steps, at: 0, wait: 0, cursor: Vec2::new(960.0, 540.0), glide: None, records: vec![], seen_missing: Default::default(), seen_budget: 0 })
        .add_systems(Update, drive);
    app.run();
}

fn centre(world: &World, name: &str) -> Option<Vec2> {
    let rt = world.get_non_send_resource::<Rt>()?;
    let mm = rt.ui.main_menu()?;
    // the menu's own widget first, else any viewport root's (dialogs are their own roots)
    let w = rt.ui.find(mm, name).filter(|&w| rt.ui.painted(w)).or_else(|| rt.ui.vm.viewport.iter().rev().find_map(|&(r, _)| rt.ui.find(r, name).filter(|&w| rt.ui.painted(w))))?;
    rt.ui.centre(w).map(|c| Vec2::new(c[0] as f32, c[1] as f32))
}

fn record(world: &mut World, label: &str) {
    let dir = world.resource::<Run>().dir.clone();
    screenshot(world, dir.join(format!("{}.png", label.replace('.', "_"))));
    let Some(rt) = world.get_non_send_resource::<Rt>() else { return };
    let ui = &rt.ui;
    let vm = &ui.vm;
    let mm = ui.main_menu().unwrap();
    let active = |sw: &str| -> String {
        ui.find(mm, sw)
            .and_then(|s| {
                let i = vm.prop(s, "ActiveWidgetIndex").i().max(0) as usize;
                let slot = *vm.o(s).slots.get(i)?;
                vm.o(slot).content.map(|c| vm.o(c).name.clone())
            })
            .unwrap_or_default()
    };
    let content = active("ContentSwitcher");
    let subnav = active("SubNavSwitcher");
    let content_painted = ui.find(mm, &content).map(|w| ui.painted(w)).unwrap_or(false);
    let missing: Vec<String> = vm.missing.keys().cloned().collect();
    let budget = vm.budget_hits.clone();
    let mut r = world.resource_mut::<Run>();
    let new_missing: Vec<String> = missing.into_iter().filter(|m| r.seen_missing.insert(m.clone())).collect();
    let new_budget: Vec<String> = budget[r.seen_budget.min(budget.len())..].to_vec();
    r.seen_budget = budget.len();
    let rec = serde_json::json!({"screen": label, "content": content, "content_painted": content_painted, "subnav": subnav, "new_missing_natives": new_missing, "step_budget_overruns": new_budget});
    println!("{}", rec);
    r.records.push(rec);
}

fn drive(world: &mut World) {
    let f = {
        let mut r = world.resource_mut::<Run>();
        r.frame += 1;
        r.frame
    };
    let acts: Vec<String> = world.resource_mut::<Messages<UiAction>>().drain().map(|a| format!("{a:?}")).collect();
    for a in acts {
        println!("frame {f}: action {a}");
        world.resource_mut::<Run>().records.push(serde_json::json!({"action": a}));
    }
    if let Some((from, to, t)) = world.resource::<Run>().glide {
        let n = 12;
        let p = from.lerp(to, (t as f32 / n as f32).min(1.0));
        world.resource_mut::<UiCursor>().0 = Some(p);
        world.resource_mut::<Run>().cursor = p;
        let window = world.resource::<Run>().window;
        if t == n + 2 {
            world.write_message(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window });
        }
        if t == n + 6 {
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
        let r = world.resource::<Run>();
        let _ = std::fs::write(r.dir.join("qa.json"), serde_json::to_string_pretty(&r.records).unwrap());
        println!("qa: {}", r.dir.display());
        world.write_message(AppExit::Success);
        return;
    };
    world.resource_mut::<Run>().at += 1;
    match kind.as_str() {
        "wait" => world.resource_mut::<Run>().wait = arg.parse().unwrap_or(10),
        "key" => {
            let window = world.resource::<Run>().window;
            for st in [ButtonState::Pressed, ButtonState::Released] {
                world.write_message(KeyboardInput { key_code: KeyCode::Space, logical_key: Key::Unidentified(NativeKey::Unidentified), state: st, text: None, repeat: false, window });
            }
        }
        "click" => match centre(world, &arg) {
            Some(c) => {
                let from = world.resource::<Run>().cursor;
                world.resource_mut::<Run>().glide = Some((from, c, 0));
            }
            None => {
                println!("frame {f}: {arg} NOT PAINTED");
                world.resource_mut::<Run>().records.push(serde_json::json!({"not_painted": arg}));
            }
        },
        "hover" => match centre(world, &arg) {
            Some(c) => {
                world.resource_mut::<UiCursor>().0 = Some(c);
                world.resource_mut::<Run>().cursor = c;
            }
            None => println!("frame {f}: {arg} NOT PAINTED"),
        },
        "shot" => {
            record(world, &arg);
            world.resource_mut::<Run>().wait = 4;
        }
        _ => {}
    }
}

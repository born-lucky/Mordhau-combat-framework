//! mordhau - the Bevy runtime of the Rust rewrite (docs/RUST_RUNTIME.md). Plugin layout mirrors the Godot port:
//! level (ue_level.gd), material (ue_material.gd / ue_tint), lighting (ue_light.gd), fighter (fighter.gd /
//! ue_anim.gd), sim bridge (fixed step; stub or mordhau-core), camera (level_walker.gd), hud, and the Pattern 4
//! script harness.
//!
//!   mordhau [--headless | --offscreen] [--script FILE] [--map PKG] [--frames N] [--fighters N] [--cam=x,y,z,pitch,yaw]
//!           [--level-source pak|extract] [--assets pak|gltf] [--materials uetint|standard] [--sim mh|core|stub]
//!
//! Defaults (R3): level from the paks, assets decoded from the paks, ue_tint materials, the mh-sim facade (combat +
//! movement on the map's collision); each falls back when its data is missing (evidence: summary.json "sources").
//!
//! --headless: no window, no GPU (wgpu backends None, no winit); assets still load, so the level is placed exactly as
//! in a windowed run and the run's counts are checked. --offscreen: the GPU renders the camera into an image (no
//! window): pipelines compile for real and `screenshot` writes PNGs. Without --script a built-in script runs:
//! load_map, spawn N, wait, dump_state (+ quit when not windowed).

mod armory_host;
mod bridge;
mod camera;
mod equiv;
mod fighter;
mod gameworld;
mod hud;
mod devmenu;
mod combat_timings;
mod input;
mod onhit;
mod shake;
mod usersettings;
// rust-ui r3: Video / Audio settings applied to the window, frame pacing and sound classes
mod settings_apply;
mod level;
mod loadout;
mod lighting;
mod material;
mod memwatch;
mod menu;
mod pak_fighter;
mod paksrc;
mod paths;
mod plan;
mod reflection;
mod script;
mod sim;
mod sim_core;
mod sim_mh;
mod sim_net;
mod specdata;
mod source;
mod ue;
mod weapon;
mod uetint;
mod uepost;
mod uepost_render;
mod uesky;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::render::settings::WgpuSettings;
use bevy::render::RenderPlugin;
use bevy::window::{ExitCondition, WindowPlugin};
use source::{AssetSrc, LevelSrc, MatSrc, Source};
use std::path::PathBuf;
use std::time::Duration;

#[derive(PartialEq)]
enum Mode {
    Windowed,
    Headless,
    Offscreen,
}

struct Args {
    mode: Mode,
    script: Option<PathBuf>,
    map: String,
    frames: u64,
    fighters: usize,
    cam: Option<[f32; 5]>,
    level: Option<LevelSrc>,
    assets: AssetSrc,
    materials: MatSrc,
    sim: String,
    golden: bool,
    bots: usize,
    profile: String,
    bot_profile: String,
    menu: bool,
    /// the stand-in menu panel (menu.rs keys / mouse) instead of the game's own front end (`--menu`: rust-ui's
    /// BP_MainMenu; Start Match -> UiAction::OpenLevel, scripts/runtime/real_menu.txt)
    legacy_menu: bool,
    /// offscreen / headless frame rate (ScheduleRunner loop), default 120
    fps: f64,
    /// --dt <secs>: every frame advances exactly this much game time (Bevy TimeUpdateStrategy::ManualDuration), for
    /// 1:1 replays of a reader record whatever the render speed
    dt: Option<f64>,
    /// the runtime's debug overlay (map, counts, sim tick, fly-cam keys): not part of the game
    debug_hud: bool,
    /// sim stepping: "frame" (per rendered frame, variable dt; default when the backend supports it) or "fixed"
    step: String,
    /// fixed rand() seed (else the session seed from the cycle counter, sim.rs session_seed)
    seed: Option<u32>,
    /// network phase (sim_net.rs): --host PORT / --join ADDR, --net-clients N, --net-name NAME
    host: Option<String>,
    join: Option<String>,
    net_clients: usize,
    net_name: Option<String>,
}

const USAGE: &str = "mordhau [--headless | --offscreen] [--script FILE] [--map PKG] [--frames N] [--fighters N] [--cam=x,y,z,pitch,yaw] [--level-source pak|extract] [--assets pak|gltf] [--materials uetint|standard] [--sim mh|core|stub] [--bots N] [--profile I|NAME|@file.json] [--bot-profile I|NAME|all] [--menu | --legacy-menu] [--fps N] [--debug-hud] [--step frame|fixed] [--seed N] [--host PORT|ADDR:PORT [--net-clients N] | --join ADDR [--net-name NAME]]";

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        mode: Mode::Windowed,
        script: None,
        map: level::DEFAULT_MAP.into(),
        frames: 0,
        fighters: 2,
        cam: None,
        level: None,
        assets: AssetSrc::Pak,
        materials: MatSrc::UeTint,
        sim: "mh".into(),
        golden: false,
        bots: 0,
        profile: "0".into(),
        bot_profile: "0".into(),
        menu: false,
        legacy_menu: false,
        fps: 120.0,
        dt: None,
        debug_hud: false,
        step: "frame".into(),
        seed: None,
        host: None,
        join: None,
        net_clients: 2,
        net_name: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        let (k, inline) = match s.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (s.clone(), None),
        };
        let mut val = |name: &str| inline.clone().or_else(|| it.next()).ok_or(format!("{name} needs a value"));
        match k.as_str() {
            "--headless" => a.mode = Mode::Headless,
            "--offscreen" => a.mode = Mode::Offscreen,
            "--script" => a.script = Some(PathBuf::from(val("--script")?)),
            "--map" => a.map = val("--map")?,
            "--frames" => a.frames = val("--frames")?.parse().map_err(|e| format!("--frames: {e}"))?,
            "--fighters" => a.fighters = val("--fighters")?.parse().map_err(|e| format!("--fighters: {e}"))?,
            "--cam" => {
                let v: Vec<f32> = val("--cam")?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() != 5 {
                    return Err("--cam needs x,y,z,pitch,yaw".into());
                }
                a.cam = Some([v[0], v[1], v[2], v[3], v[4]]);
            }
            "--level-source" => {
                a.level = Some(match val("--level-source")?.as_str() {
                    "pak" => LevelSrc::Pak,
                    "extract" => LevelSrc::Extract,
                    x => return Err(format!("--level-source {x}")),
                })
            }
            "--assets" => {
                a.assets = match val("--assets")?.as_str() {
                    "gltf" => AssetSrc::Gltf,
                    "pak" => AssetSrc::Pak,
                    x => return Err(format!("--assets {x}")),
                }
            }
            "--materials" => {
                a.materials = match val("--materials")?.as_str() {
                    "standard" => MatSrc::Standard,
                    "uetint" => MatSrc::UeTint,
                    x => return Err(format!("--materials {x}")),
                }
            }
            "--sim" => {
                a.sim = val("--sim")?;
                if !["mh", "core", "stub"].contains(&a.sim.as_str()) {
                    return Err(format!("--sim {}", a.sim));
                }
            }
            "--golden" => a.golden = true,
            "--profile" => a.profile = val("--profile")?,
            "--bot-profile" => a.bot_profile = val("--bot-profile")?,
            "--menu" => a.menu = true,
            "--legacy-menu" => a.legacy_menu = true,
            "--seed" => a.seed = Some(val("--seed")?.parse().map_err(|e| format!("--seed: {e}"))?),
            "--step" => {
                a.step = val("--step")?;
                if !["frame", "fixed"].contains(&a.step.as_str()) {
                    return Err(format!("--step {}", a.step));
                }
            }
            "--debug-hud" => a.debug_hud = true,
            "--fps" => a.fps = val("--fps")?.parse().map_err(|e| format!("--fps: {e}"))?,
            "--dt" => a.dt = Some(val("--dt")?.parse().map_err(|e| format!("--dt: {e}"))?),
            "--host" => a.host = Some(val("--host")?),
            "--join" => a.join = Some(val("--join")?),
            "--net-clients" => a.net_clients = val("--net-clients")?.parse().map_err(|e| format!("--net-clients: {e}"))?,
            "--net-name" => a.net_name = Some(val("--net-name")?),
            "--bots" => a.bots = val("--bots")?.parse().map_err(|e| format!("--bots: {e}"))?,
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument {s}\n{USAGE}")),
        }
    }
    Ok(a)
}

fn main() -> AppExit {
    // Separate private v1 gate: even direct/headless entry requires original install and local imports.
    let local_inputs = match mh_install::RuntimeInputs::discover() {
        Ok(inputs) => inputs,
        Err(e) => { eprintln!("mordhau v1: {e}"); return AppExit::from_code(2); }
    };
    local_inputs.apply_environment();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("mordhau: {e}");
            return AppExit::from_code(2);
        }
    };
    // This private distribution snapshot exposes only the installed-data combat backend.
    if args.sim != "mh" || args.golden || args.host.is_some() || args.join.is_some() {
        eprintln!("mordhau v1: only --sim mh standalone combat is supported; golden/stub/core/network fallback is disabled");
        return AppExit::from_code(2);
    }
    let spec = specdata::SpecData::load();
    if let Err(e) = &spec {
        eprintln!("mordhau v1: local matrix cannot initialize: {e}; launch blocked");
        return AppExit::from_code(2);
    }
    let paths = paths::Paths::discover();
    let (vfs, pak, mount_error) = Source::mount();
    let level = args.level.unwrap_or(if vfs.is_some() { LevelSrc::Pak } else { LevelSrc::Extract });
    let src = Source { level, assets: args.assets, materials: args.materials, vfs, pak, mount_error };
    let needs_extract = level == LevelSrc::Extract || src.assets == AssetSrc::Gltf;
    if needs_extract && (!paths.json.is_dir() || !paths.data.is_dir()) {
        eprintln!("mordhau: extract/ not found under {} (set MORDHAU_EXTRACT or MORDHAU_REPO)", paths.repo.display());
        return AppExit::from_code(2);
    }
    let needs_pak = level == LevelSrc::Pak || src.assets == AssetSrc::Pak || src.materials == MatSrc::UeTint;
    if needs_pak && src.vfs.is_none() {
        eprintln!("mordhau: the Mordhau install did not mount ({:?}); set MORDHAU_DIR", src.mount_error);
        return AppExit::from_code(2);
    }
    let unattended = args.mode != Mode::Windowed;
    // script: a file, or the built-in default
    let (name, cmds) = match &args.script {
        Some(p) => {
            let text = match std::fs::read_to_string(p) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("mordhau: cannot read {}: {e}", p.display());
                    return AppExit::from_code(2);
                }
            };
            match script::parse(&text) {
                Ok(c) => (p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("script".into()), c),
                Err(e) => {
                    eprintln!("mordhau: {}: {e}", p.display());
                    return AppExit::from_code(2);
                }
            }
        }
        None => {
            // --bots N: one player-driven fighter + N bots (a duel vs a bot = --bots 1); else N plain fighters
            let mut s = if args.menu || args.legacy_menu {
                String::new()
            } else if args.bots > 0 {
                format!("load_map {}\nspawn 1\nspawn_bot {}\nwait 30\ndump_state final\n", args.map, args.bots)
            } else {
                format!("load_map {}\nspawn {}\nwait 30\ndump_state final\n", args.map, args.fighters)
            };
            if unattended {
                s.push_str("quit\n");
            }
            let n = match args.mode {
                Mode::Headless => "headless",
                Mode::Offscreen => "offscreen",
                Mode::Windowed => "default",
            };
            (n.into(), script::parse(&s).expect("built-in script"))
        }
    };
    // frame cap: --frames, else generous when unattended (asset loading of ~800 meshes), unbounded windowed
    let max_frames = if args.frames > 0 { args.frames } else if unattended { 20_000 } else { u64::MAX };

    // sim backend: mh-sim (default) / mordhau-core / stub; a missing input falls back to the stub (recorded)

    // loadouts: BP_MordhauSingleton DefaultProfiles by index or name (Knight, Protector, ...)
    let mut sel = loadout::Selected::default();
    if let Some(v) = &src.vfs {
        let pk = mh_level::Pkgs::new(mh_pak::Reader::new(v.clone()));
        let n = loadout::profile_count(&pk);
        sel.names = (0..n).filter_map(|i| loadout::profile_gear(&pk, i).ok().map(|g| g.name)).collect();
        // `@file.json`: an FCharacterProfile in DefaultProfiles' shape (scripts/ini_profile.py converts a Game.ini
        // CharacterProfiles line), registered under a custom key like a Mercenaries-screen profile
        let pick = |s: &str| -> usize {
            if let Some(path) = s.strip_prefix('@') {
                let key = loadout::CUSTOM_KEY + 500 + (path.len() % 100);
                match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string())) {
                    Ok(v) => {
                        loadout::register_profile(key, v);
                        return key;
                    }
                    Err(e) => eprintln!("mordhau: --profile {s}: {e}"),
                }
            }
            s.parse::<usize>().ok().filter(|i| *i < n.max(1)).or_else(|| sel.names.iter().position(|x| x.eq_ignore_ascii_case(s))).unwrap_or(0)
        };
        sel.count = n;
        sel.player = pick(&args.profile);
        sel.bot_cycle = args.bot_profile.eq_ignore_ascii_case("all");
        sel.bot = if sel.bot_cycle { 0 } else { pick(&args.bot_profile) };
        for i in sel.list() {
            if let Ok(mut g) = loadout::profile_gear(&pk, i) {
                // the wearable classes the profile wears (the movement's armor factors, rust-character r9)
                if let Ok(sd) = &spec {
                    if let Ok(lo) = loadout::resolve(&pk, sd, i) {
                        g.wearables = lo.classes.clone();
                    }
                }
                sel.gear.insert(i, g);
            }
        }
    }
    let weapons: Vec<String> = {
        let mut w: Vec<String> = Vec::new();
        // only weapons with a spec record (ENT_WPN_<class>): mh-sim's melee spec has no ranged / tool records yet
        // (Huntsman's BP_Longbow, Engineer's BP_ToolBox); those profiles' fighters hold the default weapon (sim_mh
        // set_next_weapon ignores a weapon outside this list), recorded in dump_state profiles.unsupported
        let has_rec = |wp: &str| spec.as_ref().is_ok_and(|sd| sd.m.entities.contains_key(&specdata::SpecData::ent_of("ENT_WPN_", wp)));
        for g in sel.list().iter().filter_map(|i| sel.gear.get(i)) {
            if !has_rec(&g.weapon) {
                sel.unsupported.push(g.weapon.clone());
                continue;
            }
            if !w.contains(&g.weapon) {
                w.push(g.weapon.clone());
            }
            // the left-hand item (a shield) needs its records + geometry too (rust-combat r9: left hands in `weapons`)
            if !g.left.is_empty() && has_rec(&g.left) && !w.contains(&g.left) {
                w.push(g.left.clone());
            }
        }
        // Both direct matches and the front end can choose saved/new custom profiles after startup. Load the
        // supported original Equipment registry, preserving the initial weapon first (proof: mercenary_respawn_reset).
        if let (Some(v), Ok(sd)) = (&src.vfs, &spec) {
            let pk = mh_level::Pkgs::new(mh_pak::Reader::new(v.clone()));
            for wp in loadout::extend_equipment_preload(&pk, sd, &mut w) {
                if !sel.unsupported.contains(&wp) {
                    sel.unsupported.push(wp);
                }
            }
        }
        w
    };
    let sim_note = serde_json::Value::Null;
    let backend: Box<dyn sim::SimBackend> = match (&spec, &src.vfs) {
        (Ok(sd), Some(_)) => match sim_mh::MhSim::new(sd.m.clone(), &paths.repo, false, weapons.clone()) {
            Ok(m) => Box::new(m),
            Err(e) => { eprintln!("mordhau v1: combat backend initialization failed: {e}"); return AppExit::from_code(2); }
        },
        (Err(e), _) => { eprintln!("mordhau v1: local matrix unavailable: {e}"); return AppExit::from_code(2); }
        (_, None) => { eprintln!("mordhau v1: original paks are unavailable; combat launch blocked"); return AppExit::from_code(2); }
    };
    let mut app = App::new();
    let assets = AssetPlugin { file_path: paths.data.to_string_lossy().to_string(), ..default() };
    // CUE4Parse glbs carry TEXCOORD_2..7 (lightmap / custom UVs) Bevy does not map: one warning per primitive otherwise
    let log = bevy::log::LogPlugin { filter: "wgpu=error,naga=warn,bevy_gltf::loader=error".into(), ..default() };
    match args.mode {
        Mode::Headless | Mode::Offscreen => {
            let render = if args.mode == Mode::Headless {
                RenderPlugin { render_creation: WgpuSettings { backends: None, ..default() }.into(), ..default() }
            } else {
                RenderPlugin::default()
            };
            app.add_plugins(
                DefaultPlugins
                    .set(assets)
                    .set(log)
                    .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                    .set(render)
                    .disable::<bevy::winit::WinitPlugin>(),
            )
            .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / args.fps.max(1.0))));
        }
        Mode::Windowed => {
            app.add_plugins(DefaultPlugins.set(assets).set(log).set(WindowPlugin {
                primary_window: Some(Window { title: "Mordhau (mh-runtime)".into(), ..default() }),
                ..default()
            }));
        }
    }
    if args.mode == Mode::Offscreen {
        let h = camera::offscreen_image(&mut app.world_mut().resource_mut::<Assets<Image>>());
        app.insert_resource(camera::Offscreen(h));
    }
    // per-frame client stepping (sim.rs StepMode): the backend steps with each frame's dt when it can
    let mut backend = backend;
    let per_frame = args.step == "frame" && backend.set_variable_dt(true);
    let seed = sim::session_seed(args.seed);
    backend.set_rand_seed(seed);
    app.insert_resource(sim::SessionSeed(seed));
    app.insert_resource(sim::StepMode { per_frame });
    if let Some(dt) = args.dt {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(dt)));
    }
    if !sim_note.is_null() {
        eprintln!("mordhau: {sim_note}");
    }
    if let Ok(sd) = spec {
        app.insert_resource(sd);
    }
    let evidence = paths.repo.join("state").join("runtime_evidence");
    let mut sc = script::Script::new(&name, cmds, evidence, args.mode == Mode::Headless, max_frames, unattended);
    sc.mode = match args.mode {
        Mode::Headless => "headless",
        Mode::Offscreen => "offscreen",
        Mode::Windowed => "windowed",
    };
    sc.sim_note = sim_note;
    if let Some(v) = &src.vfs {
        app.insert_resource(mh_ui::Paks(v.clone()));
    }
    // the camera entity exists before Startup so the HUD can target it (mh_ui::UiTarget is read once at setup)
    let cam = app.world_mut().spawn_empty().id();
    app.insert_resource(camera::CamEntity(cam)).insert_resource(mh_ui::UiTarget(cam)).insert_resource(mh_fx::FxCamera(cam));
    if args.mode == Mode::Offscreen {
        app.insert_resource(mh_ui::UiViewport(Vec2::new(camera::OFFSCREEN_SIZE.0 as f32, camera::OFFSCREEN_SIZE.1 as f32)));
    }
    app.insert_resource(sel.clone());
    app.insert_resource(paths)
        .insert_resource(src)
        .insert_non_send(sim::Sim(backend))
        .insert_resource(ClearColor(Color::srgb(0.52, 0.66, 0.85))) // replaced by the map's sky (lighting.rs)
        .add_plugins(uetint::UeTintPlugin)
        .add_plugins(uepost_render::UePostPlugin)
        // rust-assets' UMG HUD / Cascade particles / SoundCue plugins, fed by bridge.rs; they share this mount
        .add_plugins((mh_ui::UiPlugin, mh_fx::FxPlugin, mh_audio::AudioPlugin, bridge::BridgePlugin, menu::MenuPlugin { open: args.legacy_menu, real: args.menu }, gameworld::GameWorldPlugin))
        .add_plugins((level::LevelPlugin, sim::SimPlugin, fighter::FighterPlugin, camera::CameraPlugin, hud::HudPlugin { show: args.debug_hud }, script::ScriptPlugin, input::InputPlugin, memwatch::MemWatchPlugin, armory_host::ArmoryHostPlugin))
        .add_plugins(settings_apply::SettingsApplyPlugin)
        .add_plugins(devmenu::DevMenuPlugin)
        .add_plugins(combat_timings::CombatTimingsPlugin)
        .insert_resource(camera::CamRequest(args.cam))
        .insert_resource(sc);
    // a match started directly (play.bat: --map / -TestLevel, no --menu) runs the game's own in-match UI as a match
    // opened from the front end does (BP_MordhauHUD; Escape -> the player controller's "Show Main Menu" action,
    // DefaultInput.ini, opens BP_MainMenu as the escape menu: mh-ui host.rs key_down). Windowed only: offscreen
    // evidence scripts opt in with `ui match`.
    if args.mode == Mode::Windowed && !args.menu && !args.legacy_menu {
        app.insert_resource(mh_ui::Screen::Match).insert_resource(mh_ui::MatchMap(level::match_map_name(&args.map)));
    }
    app.run()
}

//! script.rs - ScriptPlugin: the Pattern 4 oracle harness (universal-modder mashup-mods SKILL.md, "Pattern 4":
//! "a scriptable runtime as the oracle: spawn; wait 2s; screenshot; dump, with blocking verbs and evidence files per
//! run"). Verbs and evidence layout: docs/RUST_RUNTIME.md section 3. Runs as an exclusive system so a verb can read
//! the whole world.

use crate::camera::{CamRequest, FlyCam};
use crate::fighter::{Fighter, FighterReady};
use crate::level::{LevelState, UeMesh};
use crate::sim::{Sim, SpawnQueue};
use bevy::prelude::*;
use bevy::render::view::window::screenshot::{save_to_disk, Screenshot};
use serde_json::json;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// the pending press / release of a `ui_click` (popped one per frame, last first)
#[derive(Resource, Default)]
pub struct UiClicks(pub Vec<mh_ui::UiInput>);

fn ui_clicks(mut c: ResMut<UiClicks>, mut out: MessageWriter<mh_ui::UiInput>) {
    if let Some(i) = c.0.pop() {
        out.write(i);
    }
}

/// keys pressed by the `key` verb, released on the next frame
#[derive(Resource, Default)]
pub struct KeyTaps(pub Vec<KeyCode>);

fn key_msg(k: KeyCode, state: bevy::input::ButtonState) -> bevy::input::keyboard::KeyboardInput {
    use bevy::input::keyboard::{Key, NativeKey};
    bevy::input::keyboard::KeyboardInput { key_code: k, logical_key: Key::Unidentified(NativeKey::Unidentified), state, text: None, repeat: false, window: Entity::PLACEHOLDER }
}

fn release_taps(mut taps: ResMut<KeyTaps>, mut out: MessageWriter<bevy::input::keyboard::KeyboardInput>, mut age: Local<u8>) {
    if taps.0.is_empty() {
        *age = 0;
        return;
    }
    // the press is read in this frame's PreUpdate; release one frame later
    *age += 1;
    if *age >= 2 {
        for k in taps.0.drain(..) {
            out.write(key_msg(k, bevy::input::ButtonState::Released));
        }
        *age = 0;
    }
}

fn key_code(s: &str) -> Option<KeyCode> {
    Some(match s {
        "ArrowUp" => KeyCode::ArrowUp,
        "ArrowDown" => KeyCode::ArrowDown,
        "ArrowLeft" => KeyCode::ArrowLeft,
        "ArrowRight" => KeyCode::ArrowRight,
        "Enter" => KeyCode::Enter,
        "Escape" => KeyCode::Escape,
        "Space" => KeyCode::Space,
        "KeyA" => KeyCode::KeyA,
        "KeyB" => KeyCode::KeyB,
        "KeyC" => KeyCode::KeyC,
        "KeyD" => KeyCode::KeyD,
        "KeyE" => KeyCode::KeyE,
        "KeyF" => KeyCode::KeyF,
        "KeyG" => KeyCode::KeyG,
        "KeyH" => KeyCode::KeyH,
        "KeyI" => KeyCode::KeyI,
        "KeyJ" => KeyCode::KeyJ,
        "KeyK" => KeyCode::KeyK,
        "KeyL" => KeyCode::KeyL,
        "KeyM" => KeyCode::KeyM,
        "KeyN" => KeyCode::KeyN,
        "KeyO" => KeyCode::KeyO,
        "KeyP" => KeyCode::KeyP,
        "KeyQ" => KeyCode::KeyQ,
        "KeyR" => KeyCode::KeyR,
        "KeyS" => KeyCode::KeyS,
        "KeyT" => KeyCode::KeyT,
        "KeyU" => KeyCode::KeyU,
        "KeyV" => KeyCode::KeyV,
        "KeyW" => KeyCode::KeyW,
        "KeyX" => KeyCode::KeyX,
        "KeyY" => KeyCode::KeyY,
        "KeyZ" => KeyCode::KeyZ,
        "Digit0" => KeyCode::Digit0,
        "Digit1" => KeyCode::Digit1,
        "Digit2" => KeyCode::Digit2,
        "Digit3" => KeyCode::Digit3,
        "Digit4" => KeyCode::Digit4,
        "Digit5" => KeyCode::Digit5,
        "Digit6" => KeyCode::Digit6,
        "Digit7" => KeyCode::Digit7,
        "Digit8" => KeyCode::Digit8,
        "Digit9" => KeyCode::Digit9,
        "ShiftLeft" => KeyCode::ShiftLeft,
        "ControlLeft" => KeyCode::ControlLeft,
        "AltLeft" => KeyCode::AltLeft,
        "Tab" => KeyCode::Tab,
        "F1" => KeyCode::F1,
        "F7" => KeyCode::F7,
        "F8" => KeyCode::F8,
        "F9" => KeyCode::F9,
        "Backquote" => KeyCode::Backquote,
        _ => return None,
    })
}

/// A named widget/path, optionally indexed by its real generated list entry or panel slot. This only locates a
/// painted widget; the script still clicks through the UI's normal event path. ListView rows are __entries,
/// not panel slots (mh-ui natives::list_add and host::mouse_down_route).
fn script_widget(ui: &mh_ui::host::UiRuntime, path: &str) -> Option<mh_ui::model::Id> {
    let find = |path: &str| {
        let mut parts = path.split('/');
        let first = parts.next()?;
        ui.vm.viewport.iter().rev().find_map(|&(root, _)| {
            let mut w = ui.find(root, first)?;
            for part in parts.clone() {
                w = ui.vm.child(w, part)?;
            }
            ui.painted(w).then_some(w)
        })
    };
    let w = match path.strip_suffix(']').and_then(|p| p.split_once('[')) {
        Some((name, index)) => {
            let parent = find(name)?;
            let index = index.parse::<usize>().ok()?;
            if matches!(ui.vm.o(parent).class.native(), "ListView" | "TileView" | "TreeView" | "ListViewBase") {
                ui.vm.prop(parent, "__entries").arr().get(index)?.obj()?
            } else {
                ui.vm.o(*ui.vm.o(parent).slots.get(index)?).content?
            }
        }
        None => find(path)?,
    };
    ui.painted(w).then_some(w)
}

/// a reflection probe switched off by `debug probes_off`
#[derive(Component)]
pub struct ParkedProbe(pub bevy::light::LightProbe);

#[derive(Debug, Clone)]
pub struct Cmd {
    pub line: usize,
    pub verb: String,
    pub arg: String,
}

#[derive(Debug)]
enum Wait {
    Frames(u64),
    Until(Instant),
    Level(String),
    Fighters(usize),
    File(PathBuf, u64),
}

#[derive(Resource)]
pub struct Script {
    pub name: String,
    pub cmds: Vec<Cmd>,
    pub pc: usize,
    wait: Option<Wait>,
    pub dir: PathBuf,
    pub log: Vec<String>,
    pub frame: u64,
    pub max_frames: u64,
    pub headless: bool,
    pub started: Instant,
    pub finished: bool,
    pub failures: Vec<String>,
    pub quit_at_end: bool,
    /// "headless" | "offscreen" | "windowed"
    pub mode: &'static str,
    /// why --sim core fell back to the stub, if it did
    pub sim_note: serde_json::Value,
}

/// Parse a script: one verb per line, `#` comments, blank lines ignored.
pub fn parse(src: &str) -> Result<Vec<Cmd>, String> {
    let mut out = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let l = raw.split('#').next().unwrap_or("").trim();
        if l.is_empty() {
            continue;
        }
        let (verb, arg) = l.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((l, ""));
        if !["load_map", "spawn", "wait", "cam", "screenshot", "screenshot_nowait", "dump_state", "quit", "equiv_assets", "input", "move", "view", "probe", "debug", "spawn_bot", "hud", "menu", "menu_select", "menu_bots", "menu_start", "key", "world_use", "world_goto", "world_prereq", "cam_bone", "ui", "ui_key", "ui_click", "mouse_move", "mouse_to", "mouse_down", "mouse_up", "look", "drive", "post", "memcheck", "key_down", "key_up", "mouse_motion", "wheel", "real_input"].contains(&verb) {
            return Err(format!("line {}: unknown verb '{verb}'", i + 1));
        }
        out.push(Cmd { line: i + 1, verb: verb.into(), arg: arg.into() });
    }
    Ok(out)
}

/// UTC date / time stamp from the system clock (civil-from-days, H. Hinnant's algorithm).
pub fn utc_stamp() -> (String, String) {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, sod) = (s.div_euclid(86400), s.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}{:02}{:02}", sod / 3600, sod / 60 % 60, sod % 60))
}

impl Script {
    pub fn new(name: &str, cmds: Vec<Cmd>, evidence_root: PathBuf, headless: bool, max_frames: u64, quit_at_end: bool) -> Script {
        let (date, time) = utc_stamp();
        let dir = evidence_root.join(date).join(format!("{time}-{name}"));
        Script {
            name: name.into(),
            cmds,
            pc: 0,
            wait: None,
            dir,
            log: Vec::new(),
            frame: 0,
            max_frames,
            headless,
            started: Instant::now(),
            finished: false,
            failures: Vec::new(),
            quit_at_end,
            mode: if headless { "headless" } else { "windowed" },
            sim_note: serde_json::Value::Null,
        }
    }

    fn note(&mut self, s: String) {
        info!("script: {s}");
        self.log.push(format!("[f{} {:.2}s] {s}", self.frame, self.started.elapsed().as_secs_f32()));
    }
}

pub struct ScriptPlugin;

impl Plugin for ScriptPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyTaps>().init_resource::<UiClicks>().add_systems(Last, run_script).add_systems(First, (release_taps, ui_clicks));
    }
}

fn counts(world: &mut World) -> serde_json::Value {
    let meshes = world.query_filtered::<(), With<Mesh3d>>().iter(world).count();
    let ue = world.query_filtered::<(), With<UeMesh>>().iter(world).count();
    let mats = world.query_filtered::<(), With<MeshMaterial3d<StandardMaterial>>>().iter(world).count();
    let tint = world.query_filtered::<(), With<MeshMaterial3d<crate::uetint::UeTintMaterial>>>().iter(world).count();
    let fighters = world.query_filtered::<(), With<Fighter>>().iter(world).count();
    let ready = world.query_filtered::<(), With<FighterReady>>().iter(world).count();
    let players = world.query_filtered::<(), With<AnimationPlayer>>().iter(world).count();
    let suns = world.query_filtered::<(), With<DirectionalLight>>().iter(world).count();
    let entities = world.entities().count_spawned();
    json!({"entities": entities, "mesh3d": meshes, "ue_mesh_placements": ue, "with_material": mats + tint, "with_ue_tint": tint,
           "fighters": fighters, "fighters_animated": ready, "animation_players": players, "directional_lights": suns})
}

fn state_json(world: &mut World) -> serde_json::Value {
    let c = counts(world);
    // camera1p gauntlet (2026-10-07): the drawn first-person geometry at this frame, for a pixel comparison against the
    // sim's own projection and the real record: the render camera, the local player's Spine1 / Position / RightHand /
    // RightWeapon joints (world), the drawn actor and the held weapon's entity transform
    let probe = {
        let pid = world.get_resource::<crate::input::PlayerControl>().map(|p| p.id).unwrap_or(0);
        let rcam = world
            .query_filtered::<&GlobalTransform, With<crate::camera::FlyCam>>()
            .iter(world)
            .next()
            .map(|g| { let t = g.compute_transform(); json!({"translation": t.translation.to_array(), "rotation": t.rotation.to_array()}) });
        let mut qf = world.query::<(&crate::fighter::Fighter, &Transform, &Children)>();
        let (actor, roots): (Option<serde_json::Value>, Vec<Entity>) = qf
            .iter(world)
            .find(|(f, _, _)| f.id == pid)
            .map(|(_, t, c)| (Some(json!({"translation": t.translation.to_array(), "rotation": t.rotation.to_array()})), c.iter().collect::<Vec<_>>()))
            .unwrap_or((None, vec![]));
        let mut joints = serde_json::Map::new();
        let mut qb = world.query::<&crate::fighter::BodyJoints>();
        let mut ents: Vec<(String, Entity)> = vec![];
        for r in &roots {
            if let Ok(b) = qb.get(world, *r) {
                for name in ["Position", "Spine1", "RightHand", "RightWeapon", "LeftHand"] {
                    if let Some(j) = b.names.iter().position(|n| n == name) {
                        ents.push((name.to_string(), b.joints[j]));
                    }
                }
            }
        }
        for (name, e) in ents {
            if let Some(g) = world.get::<GlobalTransform>(e) {
                let t = g.compute_transform();
                joints.insert(name, json!({"translation": t.translation.to_array(), "rotation": t.rotation.to_array()}));
            }
        }
        let mut qw = world.query::<(&crate::weapon::HeldWeapon, &GlobalTransform)>();
        let weapon = qw.iter(world).find(|(h, _)| h.fighter == pid && !h.left).map(|(_, g)| { let t = g.compute_transform(); json!({"translation": t.translation.to_array(), "rotation": t.rotation.to_array()}) });
        json!({"player": pid, "render_camera": rcam, "actor": actor, "joints": joints, "weapon": weapon})
    };
    let cam = world
        .query_filtered::<&Transform, With<FlyCam>>()
        .iter(world)
        .next()
        .map(|t| json!({"translation": t.translation.to_array(), "rotation": t.rotation.to_array()}));
    let lvl = world.resource::<LevelState>();
    let level = json!({"map": lvl.map, "loading": lvl.loading, "loaded": lvl.loaded, "load_secs": lvl.load_secs, "stats": lvl.stats});
    let sim = world.non_send::<Sim>();
    // camera shakes started (evidence: shake.rs)
    let shakes = world.get_resource::<crate::shake::CameraShakes>().map(|c| json!({"started": c.started, "active": c.active.len()}));
    let clocks = json!({
        "elapsed_real_secs": world.get_resource::<Time<Real>>().map(|t|t.elapsed_secs_f64()),
        "elapsed_world_secs": world.get_resource::<Time<Virtual>>().map(|t|t.elapsed_secs_f64()),
        "relative_speed": world.get_resource::<Time<Virtual>>().map(|t|t.relative_speed()),
        "world_generation": sim.0.world_generation(),
    });
    let sim = json!({"backend": sim.0.name(), "ticks": sim.0.ticks(), "fighters": sim.0.fighters(), "debug": sim.0.debug()});
    let dev = world.get_resource::<crate::devmenu::DevMenu>().map(|d| json!({"open": d.open, "pawn_debug": d.pawn_debug,
        "visualize_block_collider": d.visualize_block_collider, "parry_boxes_enabled": d.pawn_debug || d.visualize_block_collider,
        "parry_boxes": world.get_resource::<crate::devmenu::ParryBoxDebug>().map(|p| p.snapshot()),
        "combat_timings": crate::combat_timings::snapshot(world)}));
    let tracers = world.get_resource::<crate::weapon::Tracers>().map(|t| {
        let history: Vec<_> = t.lines.iter().rev().take(128).map(|l| json!({
            "fighter": l.fighter, "tick": l.tick, "emitted_real_secs": l.emitted_at,
            "emitted_world_secs": l.emitted_world_at, "expires_world_secs": l.expires_at,
            "environment_only": l.environment_only,
            "debug_category": if l.environment_only { "local_environment_blue" } else { "local_authoritative_blade_green" },
            "start_bevy_m": l.start.to_array(), "end_bevy_m": l.end.to_array(),
        })).collect();
        json!({"sampled": t.sampled, "visible_lines": t.lines.len(), "last": t.last,
            "display_scope": "combined local authoritative blade and environment sweeps; network/cosmetic blade prediction not modeled",
            "visible_environment_lines": t.lines.iter().filter(|l|l.environment_only).count(),
            "visible_blade_lines": t.lines.iter().filter(|l|!l.environment_only).count(),
            "history_newest_first": history, "history_limit": 128, "history_truncated": t.lines.len() > 128})
    });
    let look = serde_json::to_value(world.resource::<crate::lighting::Look>().clone()).unwrap_or_default();
    let (hl_shown, hl_hidden) = crate::level::hlod_shown(world);
    let pak_body = world.get_resource::<crate::fighter::PakBody>().map(|p| p.info.clone());
    let weapons = world.get_resource::<crate::weapon::WeaponCache>().map(|w| w.info.clone());
    let bridge = world.get_resource::<crate::bridge::BridgeState>().map(|b| json!({"events": b.events, "fx_sent": b.fx_sent,
        "cues_sent": b.cues_sent, "hit_markers": b.hit_markers, "triggers": b.triggers, "map_ambience": b.map_ambience, "map_sounds": b.map_sounds, "map_effects": b.map_effects, "map_spawners": b.map_spawners, "map_volumes": b.map_volumes, "voices": b.voices.keys().collect::<Vec<_>>(), "last_events": b.last_events,
        "parry_fx_notes": b.parry_fx_notes}));
    let fx = world.get_resource::<mh_fx::FxStats>().map(|f| json!({"started": f.started.len(), "live_systems": f.live_systems,
        "live_particles":f.live_particles,"started_systems":f.started,
        "peak_particles": f.peak_particles, "missing": f.missing, "unsupported": f.unsupported_modules.len()}));
    let fx_diagnostics = world.get_non_send_resource::<mh_fx::FxState>().map(|f|
        f.diagnostics(world.get_resource::<Assets<mh_fx::ue_material::UeParticleMaterial>>()));
    let mut presentation = {
        let sim = world.non_send::<Sim>();
        let flags:Vec<_> = sim.0.fighters().iter().map(|f|json!({"fighter":f.id,
            "first_person":sim.0.first_person(f.id),"raw_camera_1p":sim.0.raw_camera_1p(f.id)})).collect();
        json!({"custom_enabled":world.get_resource::<crate::custom_visuals::CustomVisuals>().map(|c|c.enabled),
            "framework_menu":world.get_resource::<crate::framework_ui::LabMenu>(),
            "pose_bones":sim.0.pose_bones(),
            "camera_fields":flags})
    };
    presentation["authored_visuals"] = crate::custom_visuals::diagnostics(world);
    let audio = world.get_resource::<mh_audio::AudioLog>().map(|a| json!({"plays": a.rows.len(), "missing": a.missing, "sample_counts": a.sample_counts(),
        "rows": a.rows.iter().rev().take(10).map(|r| r.json()).collect::<Vec<_>>()}));
    let ui = world.get_resource::<mh_ui::HudVitals>().map(|v| json!({"visible": v.visible, "health": v.health, "stamina": v.stamina}));
    // the in-match UI's keyboard focus path and viewport roots (first-person r3: Escape routing)
    let ui_focus = world.get_non_send_resource::<mh_ui::real::Rt>().map(|r| {
        let vm = &r.ui.vm;
        json!({"focus": vm.focus.map(|f| vm.o(f).name.clone()), "main_menu_visible": r.ui.main_menu_visible(), "mode": r.ui.mode,
            "viewport": vm.viewport.iter().map(|(w, z)| (vm.o(*w).name.clone(), *z, vm.prop(*w, "Visibility").i())).collect::<Vec<_>>()})
    });
    let sim_level = world.get_resource::<crate::sim::SimLevel>().map(|s| {
        let mut retired: Vec<_> = s.retired.iter().copied().collect();
        retired.sort_unstable();
        json!({"map": s.map, "error": s.error, "attach_secs": s.secs, "profiles": s.profiles, "retired": retired})
    });
    let spec_missing = world.get_resource::<crate::specdata::SpecData>().map(|s| s.missing_list());
    let player = world.get_resource::<crate::input::PlayerControl>().map(|p| serde_json::to_value(p).unwrap_or_default());
    let cam_mode = world.get_resource::<crate::camera::CamMode>().map(|m| serde_json::to_value(m).unwrap_or_default());
    let rig = world.get_resource::<crate::camera::RigState>().map(|r| serde_json::to_value(r).unwrap_or_default());
    let profiles = world.get_resource::<crate::loadout::Selected>().map(|r| serde_json::to_value(r).unwrap_or_default());
    let motion = world.get_resource::<crate::sim::FrameMotion>().map(|m| m.stats());
    let smear = world.get_resource::<crate::weapon::SmearStats>().map(|m| serde_json::to_value(m).unwrap_or_default());
    let weapon_blood = world.get_resource::<crate::weapon::blood::WeaponBlood>()
        .map(|b| json!({"hits": b.hits, "last_hit": b.last_hit, "materials": b.materials}));
    let trails = world.get_resource::<crate::bridge::TrailState>().map(|t| json!({"starts": t.starts, "blood": t.blood, "stops": t.stops, "active": t.active.len()}));
    let seed = world.get_resource::<crate::sim::SessionSeed>().map(|s| s.0);
    let step_mode = world.get_resource::<crate::sim::StepMode>().map(|s| s.per_frame);
    let gameworld = world.get_resource::<crate::gameworld::GameWorldStats>().map(|r| serde_json::to_value(r).unwrap_or_default());
    let backdrop = world.get_resource::<crate::menu::MenuBackdrop>().map(|r| serde_json::to_value(r).unwrap_or_default());
    let menu = world.get_resource::<crate::menu::MenuState>().map(|r| serde_json::to_value(r).unwrap_or_default());
    let menu_ui = world.get_resource::<mh_ui::MainMenu>().map(|m| json!({"visible": m.visible, "modes": m.modes, "selected": m.selected, "map_label": m.map_label}));
    let armory = world.get_resource::<crate::armory_host::ArmoryHost>().map(|r| serde_json::to_value(r).unwrap_or_default());
    let armory_preview = world.get_resource::<mh_ui::armory::ArmoryPreview>().map(|p| json!({"active": p.active, "doll_location": p.doll_location, "doll_rotation": p.doll_rotation,
        "camera_location": p.camera_location, "camera_rotation": p.camera_rotation, "fov": p.fov, "doll_yaw": p.doll_yaw, "zoom": p.zoom, "focused_socket": p.focused_socket,
        "profile_name": p.profile.as_ref().and_then(|j| j.pointer("/Name/SourceString").cloned())}));
    let memwatch = world.get_resource::<crate::memwatch::MemWatch>().map(|r| serde_json::to_value(r).unwrap_or_default());
    let sc = world.resource::<Script>();
    let mut state = json!({"clocks": clocks, "memwatch": memwatch, "armory": armory, "armory_preview": armory_preview, "frame": sc.frame, "secs": sc.started.elapsed().as_secs_f32(), "mode": sc.mode, "counts": c, "camera": cam, "camera1p_probe": probe,
           "hlod": {"proxies_shown": hl_shown, "meshes_hidden_by_hlod": hl_hidden}, "look": look, "pak_body": pak_body, "weapons": weapons, "bridge": bridge, "fx": fx, "audio": audio, "hud": ui, "ui_focus": ui_focus, "sim_level": sim_level,
           "spec_missing": spec_missing, "player": player, "cam_mode": cam_mode, "rig": rig, "profiles": profiles, "menu": menu, "backdrop": backdrop, "menu_ui": menu_ui, "gameworld": gameworld, "motion": motion, "trails": trails, "smear": smear, "weapon_blood": weapon_blood, "shakes": shakes, "rand_seed": seed, "step_per_frame": step_mode, "level": level, "sim": sim, "development": dev, "tracers": tracers});
    state["fx_diagnostics"] = fx_diagnostics.into();
    state["presentation"] = presentation.into();
    state
}

fn write(path: &std::path::Path, s: &str) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = std::fs::write(path, s) {
        warn!("script: cannot write {}: {e}", path.display());
    }
}

fn finish(world: &mut World, ok: bool) {
    let st = state_json(world);
    // the headless check: a run that asked for a map must end with it loaded and something placed
    let asked = world.resource::<Script>().cmds.iter().any(|c| c.verb == "load_map");
    let (loaded, prims, test) = { let l = world.resource::<LevelState>(); (l.loaded, l.stats.primitives, l.map == crate::level::TEST_LEVEL) };
    // the combat test level places no map meshes (level.rs TEST_LEVEL)
    if asked && (!loaded || (prims == 0 && !test)) {
        world.resource_mut::<Script>().failures.push(format!("level not loaded or empty (loaded {loaded}, primitives {prims})"));
    }
    let mut sc = world.resource_mut::<Script>();
    sc.finished = true;
    let ok = ok && sc.failures.is_empty();
    let summary = json!({"script": sc.name, "ok": ok, "failures": sc.failures, "frames": sc.frame,
                         "headless": sc.headless, "mode": sc.mode,
                         "sim_note": sc.sim_note, "final_state": st});
    let dir = sc.dir.clone();
    let log = sc.log.join("\n");
    write(&dir.join("summary.json"), &serde_json::to_string_pretty(&summary).unwrap_or_default());
    write(&dir.join("log.txt"), &log);
    println!("mh-runtime: {} -> {}", if ok { "OK" } else { "FAIL" }, dir.display());
    println!("{}", serde_json::to_string(&summary["final_state"]["counts"]).unwrap_or_default());
    if sc.quit_at_end || !ok {
        world.write_message(if ok { AppExit::Success } else { AppExit::from_code(1) });
    }
}

fn run_script(world: &mut World) {
    if !world.contains_resource::<Script>() || world.resource::<Script>().finished {
        return;
    }
    {
        let mut sc = world.resource_mut::<Script>();
        sc.frame += 1;
        if sc.frame > sc.max_frames {
            let msg = format!("frame cap {} reached at line {:?} (waiting on {:?})", sc.max_frames, sc.cmds.get(sc.pc).map(|c| c.line), sc.wait);
            sc.note(msg.clone());
            sc.failures.push(msg);
            finish(world, false);
            return;
        }
    }
    // blocking wait in progress?
    let wait_done = {
        let frame = world.resource::<Script>().frame;
        match world.resource::<Script>().wait.as_ref() {
            None => true,
            Some(Wait::Frames(n)) => frame >= *n,
            Some(Wait::Until(t)) => Instant::now() >= *t,
            Some(Wait::Level(m)) => {
                let l = world.resource::<LevelState>();
                l.loaded && &l.map == m && l.request.is_none()
            }
            Some(Wait::Fighters(n)) => {
                let n = *n;
                world.query_filtered::<(), With<FighterReady>>().iter(world).count() >= n
            }
            Some(Wait::File(p, deadline)) => {
                if p.is_file() {
                    true
                } else if frame >= *deadline {
                    let msg = format!("screenshot not written: {}", p.display());
                    world.resource_mut::<Script>().failures.push(msg);
                    true
                } else {
                    false
                }
            }
        }
    };
    if !wait_done {
        return;
    }
    if let Some(w) = world.resource_mut::<Script>().wait.take() {
        let s = format!("done waiting {w:?}");
        world.resource_mut::<Script>().note(s);
        if let Wait::Level(_) = w {
            let lvl = world.resource::<LevelState>();
            let ev = json!({"map": lvl.map, "load_secs": lvl.load_secs, "stats": lvl.stats,
                            "starts": lvl.starts.iter().map(|s| json!({"name": s.name, "team": s.team, "translation": Vec3::from(s.xf.translation).to_array()})).collect::<Vec<_>>()});
            let dir = world.resource::<Script>().dir.clone();
            write(&dir.join("load_map.json"), &serde_json::to_string_pretty(&ev).unwrap_or_default());
        }
    }
    // run commands until one blocks
    loop {
        let (cmd, frame, headless, dir) = {
            let sc = world.resource::<Script>();
            match sc.cmds.get(sc.pc) {
                Some(c) => (c.clone(), sc.frame, sc.headless, sc.dir.clone()),
                None => {
                    finish(world, true);
                    return;
                }
            }
        };
        world.resource_mut::<Script>().pc += 1;
        world.resource_mut::<Script>().note(format!("line {}: {} {}", cmd.line, cmd.verb, cmd.arg));
        let wait = match cmd.verb.as_str() {
            "load_map" => {
                let map = if cmd.arg.is_empty() { crate::level::DEFAULT_MAP.to_string() } else { cmd.arg.clone() };
                world.resource_mut::<LevelState>().request = Some(map.clone());
                Some(Wait::Level(map))
            }
            "spawn" => {
                let n: usize = cmd.arg.parse().unwrap_or(1);
                world.resource_mut::<SpawnQueue>().0 += n;
                let have = world.non_send::<Sim>().0.fighters().len();
                Some(Wait::Fighters(have + n))
            }
            "menu" => {
                world.resource_mut::<mh_ui::MainMenu>().visible = cmd.arg.trim() != "off";
                Some(Wait::Frames(frame + 2))
            }
            "menu_select" => {
                let i: usize = cmd.arg.trim().parse().unwrap_or(0);
                world.resource_mut::<mh_ui::MainMenu>().selected = i;
                Some(Wait::Frames(frame + 2))
            }
            "menu_bots" => {
                let n: usize = cmd.arg.trim().parse().unwrap_or(0);
                let mut st = world.resource_mut::<crate::menu::MenuState>();
                let max = st.bot_max;
                st.bot_count = n.min(max);
                Some(Wait::Frames(frame + 2))
            }
            "key" => {
                // key <KeyCode name>: a real key tap through Bevy's input path (a KeyboardInput Pressed message now,
                // Released next frame; keyboard_input_system turns them into ButtonInput<KeyCode>, as from the window)
                match key_code(cmd.arg.trim()) {
                    Some(k) => {
                        world.write_message(key_msg(k, bevy::input::ButtonState::Pressed));
                        world.resource_mut::<KeyTaps>().0.push(k);
                    }
                    None => world.resource_mut::<Script>().failures.push(format!("line {}: unknown key {}", cmd.line, cmd.arg)),
                }
                Some(Wait::Frames(frame + 3))
            }
            // first-person r2 (real input path tests): key_down / key_up <KeyCode> hold a key (KeyboardInput Pressed /
            // Released), mouse_motion <dx> <dy> one raw mouse delta (MouseMotion -> AccumulatedMouseMotion, as the OS
            // raw device), wheel up | down one notch (MouseWheel, Line unit)
            "real_input" => {
                world.insert_resource(crate::input::HeadlessRealInput);
                Some(Wait::Frames(frame + 1))
            }
            "key_down" | "key_up" => {
                match key_code(cmd.arg.trim()) {
                    Some(k) => {
                        let st = if cmd.verb == "key_down" { bevy::input::ButtonState::Pressed } else { bevy::input::ButtonState::Released };
                        world.write_message(key_msg(k, st));
                    }
                    None => world.resource_mut::<Script>().failures.push(format!("line {}: unknown key {}", cmd.line, cmd.arg)),
                }
                Some(Wait::Frames(frame + 2))
            }
            "mouse_motion" => {
                let a: Vec<f32> = cmd.arg.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                let d = Vec2::new(a.first().copied().unwrap_or(0.0), a.get(1).copied().unwrap_or(0.0));
                world.write_message(bevy::input::mouse::MouseMotion { delta: d });
                Some(Wait::Frames(frame + 1))
            }
            "wheel" => {
                let y = if cmd.arg.trim() == "down" { -1.0 } else { 1.0 };
                world.write_message(bevy::input::mouse::MouseWheel { unit: bevy::input::mouse::MouseScrollUnit::Line, x: 0.0, y, window: Entity::PLACEHOLDER, phase: bevy::input::touch::TouchPhase::Moved });
                Some(Wait::Frames(frame + 2))
            }
            "ui" => {
                // ui main_menu | match | off: the game's front end screen (mh-ui Screen)
                let s = match cmd.arg.trim() {
                    "main_menu" => mh_ui::Screen::MainMenu,
                    "match" => {
                        if world.resource::<mh_ui::MatchMap>().0.is_empty() {
                            let m = crate::level::match_map_name(&world.resource::<crate::level::LevelState>().map);
                            world.insert_resource(mh_ui::MatchMap(m));
                        }
                        mh_ui::Screen::Match
                    }
                    _ => mh_ui::Screen::Off,
                };
                world.insert_resource(s);
                Some(Wait::Frames(frame + 5))
            }
            "ui_key" => {
                // ui_key <UE key name>: a key press + release into the front end (mh-ui UiInput)
                let k = cmd.arg.trim().to_string();
                world.write_message(mh_ui::UiInput::Key(k.clone()));
                world.write_message(mh_ui::UiInput::KeyUp(k));
                Some(Wait::Frames(frame + 3))
            }
            // real mouse input, as a window delivers it: the cursor (mh_ui::UiCursor, what real.rs reads when there is
            // no window) and MouseButtonInput messages (Bevy's mouse_button_input_system -> ButtonInput<MouseButton>,
            // mh-ui's own reader). `mouse_move X Y` (logical px), `mouse_to <widget path>` (its centre, from the UI
            // layout), `mouse_down` / `mouse_up` [Left|Right]
            "mouse_move" | "mouse_to" => {
                let at = if cmd.verb == "mouse_move" {
                    let a: Vec<f32> = cmd.arg.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    (a.len() == 2).then(|| Vec2::new(a[0], a[1]))
                } else {
                    let arg = cmd.arg.trim().to_string();
                    world.get_non_send_resource::<mh_ui::real::Rt>().and_then(|r| {
                        let w = script_widget(&r.ui, &arg)?;
                        r.ui.centre(w).map(|c| Vec2::new(c[0] as f32, c[1] as f32))
                    })
                };
                match at {
                    Some(p) => {
                        world.resource_mut::<mh_ui::UiCursor>().0 = Some(p);
                        world.resource_mut::<Script>().note(format!("mouse at {p}"));
                    }
                    None => world.resource_mut::<Script>().failures.push(format!("line {}: no position for {}", cmd.line, cmd.arg)),
                }
                Some(Wait::Frames(frame + 3))
            }
            "mouse_down" | "mouse_up" => {
                let button = if cmd.arg.trim().eq_ignore_ascii_case("right") { MouseButton::Right } else { MouseButton::Left };
                let state = if cmd.verb == "mouse_down" { bevy::input::ButtonState::Pressed } else { bevy::input::ButtonState::Released };
                world.write_message(bevy::input::mouse::MouseButtonInput { button, state, window: Entity::PLACEHOLDER });
                Some(Wait::Frames(frame + 3))
            }
            "ui_click" => {
                // ui_click <widget name>: a left click at the centre of the first painted widget of that name
                // "<name>[<i>]": generated entry i of a ListView, otherwise content of panel slot i
                let arg = cmd.arg.trim().to_string();
                let at = world.get_non_send_resource::<mh_ui::real::Rt>().and_then(|r| {
                    let w = script_widget(&r.ui, &arg)?;
                    r.ui.centre(w)
                });
                match at {
                    Some(c) => {
                        // the runtime's own click (move + press + release, mh-ui host.rs click), as its tests drive it
                        if let Some(mut r) = world.get_non_send_resource_mut::<mh_ui::real::Rt>() {
                            r.ui.click(c);
                        }
                    }
                    None => world.resource_mut::<Script>().failures.push(format!("line {}: no painted widget {}", cmd.line, cmd.arg)),
                }
                Some(Wait::Frames(frame + 6))
            }
            "world_goto" => {
                // world_goto <actor name substring> [dx dy dz]: move the player fighter to that actor's root (+ offset,
                // UE cm) through SimBackend::move_by (evidence placement, not gameplay)
                let a: Vec<&str> = cmd.arg.split_whitespace().collect();
                let target = world.get_non_send_resource::<crate::gameworld::GameWorld>().and_then(|g| {
                    let n = a.first().copied().unwrap_or("");
                    g.w.actors.iter().find(|x| x.name.contains(n)).and_then(|x| g.actor_xf.get(&x.id).map(|t| t.1.translation()))
                        // Native PlayerStarts are read as spawns, not Blueprint gameplay actors.
                        // Exact-name fallback is evidence placement only; original world transform is retained.
                        .or_else(|| (!n.is_empty()).then(|| g.level.spawns.iter().find(|s| s.name == n)
                            .map(|s| s.xf.translation())).flatten())
                });
                let off = |i: usize| a.get(i).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0);
                let id = world.get_resource::<crate::input::PlayerControl>().map(|p| p.id).unwrap_or(0);
                match target {
                    Some(t) => {
                        let mut sim = world.non_send_resource_mut::<Sim>();
                        if let Some(v) = sim.0.fighters().iter().find(|v| v.id == id) {
                            let d = [t[0] + off(1) - v.loc[0] as f64, t[1] + off(2) - v.loc[1] as f64, t[2] + off(3) - v.loc[2] as f64];
                            sim.0.move_by(id, d.map(|x| x as f32));
                        }
                    }
                    None => world.resource_mut::<Script>().failures.push(format!("line {}: no actor {}", cmd.line, cmd.arg)),
                }
                Some(Wait::Frames(frame + 2))
            }
            "world_prereq" => {
                // world_prereq: every capture point's enemy gained its prerequisites, as mh-mode's
                // enemy_gained_prerequisites would report (the Frontline mode is not run by the runtime yet; test hook)
                if let Some(mut g) = world.get_non_send_resource_mut::<crate::gameworld::GameWorld>() {
                    let cps: Vec<usize> = g.w.cps.keys().copied().collect();
                    for cp in cps {
                        g.w.capture_point_prerequisites(cp, true);
                    }
                }
                Some(Wait::Frames(frame + 2))
            }
            "world_use" => {
                // world_use <actor name substring>: mh-world interact() on that actor as the player (the Use key's
                // target picked by name, for offscreen evidence)
                world.resource_mut::<crate::gameworld::UseRequest>().0 = Some(cmd.arg.trim().to_string());
                Some(Wait::Frames(frame + 2))
            }
            "menu_start" => {
                // the Enter key's MenuChoice, then wait for the map and the fighters
                let i = world.resource::<mh_ui::MainMenu>().selected;
                world.write_message(mh_ui::MenuChoice(i));
                Some(Wait::Frames(frame + 2))
            }
            "hud" => {
                world.resource_mut::<mh_ui::HudVitals>().visible = cmd.arg.trim() != "off";
                Some(Wait::Frames(frame + 2))
            }
            "spawn_bot" => {
                let n: usize = cmd.arg.parse().unwrap_or(1);
                world.resource_mut::<crate::sim::BotQueue>().0 += n;
                let have = world.non_send::<Sim>().0.fighters().len();
                Some(Wait::Fighters(have + n))
            }
            "wait" => {
                if let Some(s) = cmd.arg.strip_suffix('s') {
                    let secs: f64 = s.parse().unwrap_or(1.0);
                    Some(Wait::Until(Instant::now() + std::time::Duration::from_secs_f64(secs)))
                } else {
                    Some(Wait::Frames(frame + cmd.arg.parse::<u64>().unwrap_or(1)))
                }
            }
            "cam_bone" => {
                // cam_bone <fighter> <bone> <dist_cm> <pitch> <yaw>: the fly camera looking at a fighter's bone from
                // dist along the UE rotator (pitch, yaw) (evidence close-ups, fidelity-audit r4; not game behaviour)
                let a: Vec<&str> = cmd.arg.split_whitespace().collect();
                let fid: u32 = a.first().and_then(|x| x.parse().ok()).unwrap_or(0);
                let bone = a.get(1).copied().unwrap_or("RightHand").to_lowercase();
                let g = |i: usize, d: f32| a.get(i).and_then(|x| x.parse::<f32>().ok()).unwrap_or(d);
                let (dist, pitch, yaw) = (g(2, 60.0), g(3, 0.0), g(4, 0.0));
                let mut q = world.query::<(&crate::fighter::Fighter, &Children)>();
                let roots: Vec<Entity> = q.iter(world).filter(|(f, _)| f.id == fid).flat_map(|(_, c)| c.iter().collect::<Vec<_>>()).collect();
                let mut found = None;
                let mut qb = world.query::<&crate::fighter::BodyJoints>();
                for r in roots {
                    if let Ok(b) = qb.get(world, r) {
                        if let Some(j) = b.names.iter().position(|n| n.to_lowercase() == bone) {
                            found = Some(b.joints[j]);
                        }
                    }
                }
                match found.and_then(|e| world.get::<GlobalTransform>(e).map(|g| g.translation())) {
                    Some(p) => {
                        // bone (Bevy m) -> UE cm, back off along the view direction
                        let ue = [p.x * 100.0, p.z * 100.0, p.y * 100.0];
                        let (pr, yr) = (pitch.to_radians(), yaw.to_radians());
                        let f = [pr.cos() * yr.cos(), pr.cos() * yr.sin(), pr.sin()];
                        world.resource_mut::<CamRequest>().0 = Some([ue[0] - f[0] * dist, ue[1] - f[1] * dist, ue[2] - f[2] * dist, pitch, yaw]);
                    }
                    None => world.resource_mut::<Script>().failures.push(format!("line {}: no bone {}", cmd.line, cmd.arg)),
                }
                Some(Wait::Frames(frame + 2))
            }
            // rust-render: `post all | none | -bloom -film -tint -vignette -sharpen -fringe -ea | exposure <e> | exposure auto`
            // toggles UE post stages for the per-stage evidence (uepost_render.rs PostDebug)
            "post" => {
                let mut d = *world.resource::<crate::uepost_render::PostDebug>();
                let words: Vec<&str> = cmd.arg.split_whitespace().collect();
                let mut i = 0;
                while i < words.len() {
                    let st = &mut d.stages;
                    match words[i] {
                        "all" => *st = Default::default(),
                        "none" => *st = crate::uepost_render::Stages { eye_adaptation: true, bloom: false, film: true, tint: false, vignette: false, sharpen: false, fringe: false },
                        "-bloom" => st.bloom = false,
                        "-film" => st.film = false,
                        "-tint" => st.tint = false,
                        "-vignette" => st.vignette = false,
                        "-sharpen" => st.sharpen = false,
                        "-fringe" => st.fringe = false,
                        "-ea" => st.eye_adaptation = false,
                        "exposure" => {
                            i += 1;
                            d.fixed_exposure = words.get(i).and_then(|w| w.parse().ok());
                        }
                        w => world.resource_mut::<Script>().failures.push(format!("line {}: post: unknown '{w}'", cmd.line)),
                    }
                    i += 1;
                }
                *world.resource_mut::<crate::uepost_render::PostDebug>() = d;
                Some(Wait::Frames(frame + 2))
            }
            "cam" => {
                let v: Vec<f32> = cmd.arg.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() == 5 {
                    world.resource_mut::<CamRequest>().0 = Some([v[0], v[1], v[2], v[3], v[4]]);
                } else {
                    world.resource_mut::<Script>().failures.push(format!("line {}: cam needs x,y,z,pitch,yaw", cmd.line));
                }
                Some(Wait::Frames(frame + 2))
            }
            // camera1p gauntlet: `screenshot_nowait NAME` queues the capture and continues on the next frame, so a 1:1
            // replay script (`drive` per frame + `--dt`) keeps its timeline (a blocking `screenshot` lets the sim step
            // while the file is written: 128 captures drifted the motion1 replay by seconds)
            "screenshot_nowait" => {
                let name = if cmd.arg.is_empty() { format!("shot{}", frame) } else { cmd.arg.clone() };
                let p = dir.join(format!("{name}.png"));
                let _ = std::fs::create_dir_all(&dir);
                let target = world.get_resource::<crate::camera::Offscreen>().map(|o| o.0.clone());
                let shot = match target {
                    Some(img) => Screenshot(bevy::camera::RenderTarget::Image(img.into())),
                    None => Screenshot::primary_window(),
                };
                world.spawn(shot).observe(save_to_disk(p));
                None
            }
            "screenshot" => {
                let name = if cmd.arg.is_empty() { format!("shot{}", frame) } else { cmd.arg.clone() };
                let _ = headless;
                let mode = world.resource::<Script>().mode;
                if mode == "headless" {
                    write(&dir.join(format!("{name}.skipped")), "headless run: no GPU, no window\n");
                    None
                } else {
                    let p = dir.join(format!("{name}.png"));
                    let _ = std::fs::create_dir_all(&dir);
                    // offscreen runs render the camera into an image (camera.rs Offscreen); windowed: the window
                    let target = world.get_resource::<crate::camera::Offscreen>().map(|o| o.0.clone());
                    let shot = match target {
                        Some(img) => Screenshot(bevy::camera::RenderTarget::Image(img.into())),
                        None => Screenshot::primary_window(),
                    };
                    world.spawn(shot).observe(save_to_disk(p.clone()));
                    Some(Wait::File(p, frame + 600))
                }
            }
            "dump_state" => {
                let name = if cmd.arg.is_empty() { format!("state{}", frame) } else { cmd.arg.clone() };
                let st = state_json(world);
                write(&dir.join(format!("{name}.json")), &serde_json::to_string_pretty(&st).unwrap_or_default());
                None
            }
            // memcheck <window_secs> <max_mb_per_s> [<max_asset_growth>]: the leak regression check over memwatch's trailing rows
            "memcheck" => {
                let a: Vec<f64> = cmd.arg.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                let (warm, max_mb, max_assets) = (a.first().copied().unwrap_or(5.0) as f32, a.get(1).copied().unwrap_or(2.0), a.get(2).copied().unwrap_or(50.0) as i64);
                let rows = world.resource::<crate::memwatch::MemWatch>().rows.clone();
                let msg = match crate::memwatch::growth(&rows, warm) {
                    Some((per_s, d)) => {
                        let grew: Vec<String> = d.iter().filter(|x| x.1 > max_assets).map(|x| format!("{} +{}", x.0, x.1)).collect();
                        let m = format!("memcheck: private {per_s:.2} MB/s over the last {warm}s, assets {d:?}");
                        info!("{m}");
                        world.resource_mut::<Script>().log.push(m.clone());
                        (per_s > max_mb || !grew.is_empty()).then(|| format!("{m}: LEAK (limit {max_mb} MB/s, {max_assets} assets; grew {grew:?})"))
                    }
                    None => Some(format!("memcheck: fewer than 2 s of rows in the last {warm}s")),
                };
                if let Some(m) = msg {
                    world.resource_mut::<Script>().failures.push(m);
                }
                None
            }
            "equiv_assets" => {
                let n: usize = cmd.arg.parse().unwrap_or(usize::MAX);
                let r = crate::equiv::equiv_assets(world, n);
                write(&dir.join("assets_equiv.json"), &serde_json::to_string_pretty(&r).unwrap_or_default());
                None
            }
            "input" => {
                // input <fighter id> attack <move> [angle] | feint | parry | release
                let a: Vec<&str> = cmd.arg.split_whitespace().collect();
                let id: u32 = a.first().and_then(|x| x.parse().ok()).unwrap_or(0);
                // input <id> jump: the Jump action's press for the next step (fidelity-audit r6; PlayerControl sets the
                // same FighterInputs.jump from the bound key)
                if a.get(1).copied() == Some("jump") {
                    world.resource_mut::<crate::sim::Inputs>().0.entry(id).or_default().jump = true;
                }
                // input <id> mode: the weapon-mode switch request for the next step (grip r2; the R binding's
                // FrameInput.switch_mode)
                if a.get(1).copied() == Some("mode") {
                    world.resource_mut::<crate::sim::Inputs>().0.entry(id).or_default().switch_mode = true;
                }
                let ev = match a.get(1).copied() {
                    Some("attack") => Some(crate::sim::SimInput::Attack {
                        mv: a.get(2).and_then(|x| x.parse().ok()).unwrap_or(0),
                        angle: a.get(3).and_then(|x| x.parse().ok()).unwrap_or(0.0),
                    }),
                    Some("feint") => Some(crate::sim::SimInput::Feint),
                    Some("parry") => Some(crate::sim::SimInput::Parry),
                    Some("release") => Some(crate::sim::SimInput::ReleaseBlock),
                    _ => None,
                };
                let ok = ev.is_some();
                if let Some(e) = ev {
                    world.resource_mut::<crate::sim::Inputs>().0.entry(id).or_default().add_event(&e);
                }
                world.resource_mut::<Script>().note(format!("input queued: {ok}"));
                Some(Wait::Frames(frame + 1))
            }
            "move" => {
                // move <fighter id> <fwd> <right> [yaw deg]: held movement axes (persist until the next move)
                let a: Vec<&str> = cmd.arg.split_whitespace().collect();
                let id: u32 = a.first().and_then(|x| x.parse().ok()).unwrap_or(0);
                let g = |i: usize| a.get(i).and_then(|x| x.parse::<f32>().ok());
                let mut inputs = world.resource_mut::<crate::sim::Inputs>();
                let fi = inputs.0.entry(id).or_default();
                fi.fwd = g(1).unwrap_or(0.0);
                fi.right = g(2).unwrap_or(0.0);
                if let Some(y) = g(3) {
                    fi.yaw = Some(y);
                }
                Some(Wait::Frames(frame + 1))
            }
            "probe" => {
                // probe <px> <py> [name]: what the camera sees through pixel (px, py) of a 1280x720 frame: placed
                // primitives whose world box the ray crosses, nearest first (mesh, component, material)
                let a: Vec<f32> = cmd.arg.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                let r = crate::script::probe(world, a.first().copied().unwrap_or(640.0), a.get(1).copied().unwrap_or(360.0));
                write(&dir.join(format!("probe_{}_{}.json", a.first().copied().unwrap_or(640.0), a.get(1).copied().unwrap_or(360.0))),
                    &serde_json::to_string_pretty(&r).unwrap_or_default());
                None
            }
            "debug" => {
                // debug albedo | lit: the unlit base-colour view of every ue_tint material
                // debug probes_off | probes_on: hide / show the per-capture reflection probes (level.rs; before/after
                // evidence: off = only the camera's map-wide capture)
                match cmd.arg.trim() {
                    // debug interp_off | interp_on: draw the latest fixed step only / interpolated (sim.rs SimFrames);
                    // debug motion_reset: clear the per-frame motion record
                    "interp_off" | "interp_on" => world.resource_mut::<crate::sim::SimFrames>().enabled = cmd.arg.trim() == "interp_on",
                    "motion_reset" => world.resource_mut::<crate::sim::FrameMotion>().frames.clear(),
                    "probes_off" => {
                        // the LightProbe component is parked (Bevy gathers probes by that component)
                        let mut q = world.query::<(Entity, &bevy::light::LightProbe)>();
                        let v: Vec<(Entity, bevy::light::LightProbe)> = q.iter(world).map(|(e, p)| (e, p.clone())).collect();
                        for (e, p) in v {
                            world.entity_mut(e).remove::<bevy::light::LightProbe>().insert(ParkedProbe(p));
                        }
                    }
                    "probes_on" => {
                        let mut q = world.query::<(Entity, &ParkedProbe)>();
                        let v: Vec<(Entity, bevy::light::LightProbe)> = q.iter(world).map(|(e, p)| (e, p.0.clone())).collect();
                        for (e, p) in v {
                            world.entity_mut(e).remove::<ParkedProbe>().insert(p);
                        }
                    }
                    a if crate::devmenu::command(world, a) => {},
                    a => world.resource_mut::<crate::uetint::DebugAlbedo>().0 = a == "albedo",
                }
                Some(Wait::Frames(frame + 3))
            }
            "look" => {
                // look <pitch deg, + up> | look off: fighter 0's look-up held at a value (rust-combat r10; offscreen the
                // PlayerControl reads no input, so the held value goes straight into the frame input like `move`'s yaw)
                let v = cmd.arg.trim();
                let pitch: Option<f64> = if v == "off" { None } else { v.parse().ok() };
                if let Some(mut pc) = world.get_resource_mut::<crate::input::PlayerControl>() {
                    pc.script_pitch = pitch;
                }
                world.resource_mut::<crate::sim::Inputs>().0.entry(0).or_default().look_up = pitch;
                Some(Wait::Frames(frame + 1))
            }
            "drive" => {
                // drive <yaw deg> <look-up deg> [fwd] [right]: one recorded frame of fighter 0's input (yaw, look, held
                // movement axes) in a single command, so a reader record (state/live_rec/*.jsonl) replays 1:1 per frame
                let a: Vec<f32> = cmd.arg.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                let pitch = a.get(1).map(|&p| p as f64);
                if let Some(mut pc) = world.get_resource_mut::<crate::input::PlayerControl>() {
                    pc.script_pitch = pitch;
                }
                let mut inputs = world.resource_mut::<crate::sim::Inputs>();
                let fi = inputs.0.entry(0).or_default();
                fi.yaw = a.first().copied();
                fi.look_up = pitch;
                fi.fwd = a.get(2).copied().unwrap_or(0.0);
                fi.right = a.get(3).copied().unwrap_or(0.0);
                Some(Wait::Frames(frame + 1))
            }
            "view" => {
                // view 3p | 1p | fly: the camera mode (the player rig follows fighter 0)
                let m = cmd.arg.trim().to_string();
                // `fly1p` (first-person r1, evidence): the fly camera, fighter 0 kept in first person
                world.insert_resource(crate::camera::FpInFly(m == "fly1p"));
                if m == "fly1p" {
                    *world.resource_mut::<crate::camera::CamMode>() = crate::camera::CamMode::Fly;
                    if let Some(mut pc) = world.get_resource_mut::<crate::input::PlayerControl>() {
                        pc.third_person = false;
                    }
                } else if m == "fly" {
                    *world.resource_mut::<crate::camera::CamMode>() = crate::camera::CamMode::Fly;
                } else {
                    *world.resource_mut::<crate::camera::CamMode>() = crate::camera::CamMode::Player;
                    if let Some(mut pc) = world.get_resource_mut::<crate::input::PlayerControl>() {
                        pc.third_person = m != "1p";
                    }
                }
                Some(Wait::Frames(frame + 2))
            }
            "quit" => {
                world.resource_mut::<Script>().quit_at_end = true;
                finish(world, true);
                return;
            }
            _ => None,
        };
        if let Some(w) = wait {
            world.resource_mut::<Script>().wait = Some(w);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercenary_focus_rejects_hidden_paths_without_rejecting_offscreen_widgets() {
        use mh_ui::model::V;
        let vfs = match mh_pak::Vfs::mount_default() {
            Ok(vfs) => std::sync::Arc::new(vfs),
            Err(e) => {
                assert!(!std::env::var("MORDHAU_GOLDEN_REQUIRED").is_ok_and(|v| v == "1"), "required UI paks unavailable: {e:?}");
                eprintln!("SKIP mercenary focus paths: {e:?}");
                return;
            }
        };
        let mut vm = mh_ui::vm::Vm::new(mh_pak::Reader::new(vfs));
        let cls = vm.native_class("WidgetSwitcher");
        let switcher = vm.new_obj(cls, "FocusPathSwitcher");
        let cls = vm.native_class("Button");
        let first = vm.new_obj(cls.clone(), "VisibleOffscreenButton");
        let second = vm.new_obj(cls, "InactiveButton");
        vm.set(first, "RenderTranslation", V::st(&[("X", V::Float(1_000_000.0)), ("Y", V::Float(1_000_000.0))]));
        for child in [first, second] {
            let cls = vm.native_class("WidgetSwitcherSlot");
            let slot = vm.new_obj(cls, "FocusPathSlot");
            vm.om(slot).content = Some(child);
            vm.om(slot).panel = Some(switcher);
            vm.om(child).parent_slot = Some(slot);
            vm.om(switcher).slots.push(slot);
        }
        let set_focus = |vm: &mut mh_ui::vm::Vm, target| {
            mh_ui::natives::call(vm, mh_ui::vm::Ctx::Obj(target), "Widget", "SetKeyboardFocus", &[]);
        };
        set_focus(&mut vm, first);
        assert_eq!(vm.focus, None, "detached widget cannot take keyboard focus");
        vm.viewport.push((switcher, 0));
        vm.set(switcher, "ActiveWidgetIndex", V::Int(0));
        // Visible but outside the viewport still has a structural path; hit-test invisible remains focusable.
        for visible in [0, 3, 4] {
            vm.set(switcher, "Visibility", V::Int(visible));
            vm.focus = None;
            set_focus(&mut vm, first);
            assert_eq!(vm.focus, Some(first));
            assert_eq!(vm.visible_widget_path(first), Some(vec![switcher, first]));
            set_focus(&mut vm, second);
            assert_eq!(vm.focus, Some(first), "inactive switcher child cannot replace focus");
        }
        for hidden in [1, 2] {
            vm.set(switcher, "Visibility", V::Int(hidden));
            vm.focus = None;
            set_focus(&mut vm, first);
            assert_eq!(vm.focus, None, "hidden/collapsed ancestor prevents refocusing");
            assert!(vm.visible_widget_path(first).is_none());
        }
        vm.set(switcher, "Visibility", V::Int(0));
        vm.set(switcher, "ActiveWidgetIndex", V::Int(1));
        set_focus(&mut vm, second);
        assert_eq!(vm.focus, Some(second), "newly active child becomes focusable");
        vm.focus = None;
        vm.set(switcher, "ActiveWidgetIndex", V::Int(-1));
        set_focus(&mut vm, first);
        assert_eq!(vm.focus, None, "no active index must not focus the first child");
        let first_slot = vm.o(switcher).slots[0];
        vm.om(first_slot).content = None;
        vm.set(switcher, "ActiveWidgetIndex", V::Int(0));
        set_focus(&mut vm, second);
        assert_eq!(vm.focus, None, "empty slot must not shift the selected child");
        vm.set(switcher, "ActiveWidgetIndex", V::Int(1));
        set_focus(&mut vm, second);
        assert_eq!(vm.focus, Some(second), "exact second slot remains selectable after empty first slot");
        vm.viewport.clear();
        assert!(vm.visible_widget_path(second).is_none(), "removed viewport root invalidates focus route");
    }

    #[test]
    fn mercenary_list_entry_click_sends_the_real_selected_profile() {
        let vfs = match mh_pak::Vfs::mount_default() {
            Ok(vfs) => std::sync::Arc::new(vfs),
            Err(e) => {
                assert!(!std::env::var("MORDHAU_GOLDEN_REQUIRED").is_ok_and(|v| v == "1"), "required UI paks unavailable: {e:?}");
                eprintln!("SKIP mercenary UI picker: {e:?}");
                return;
            }
        };
        let mut ui = mh_ui::host::UiRuntime::new(vfs, "FFA_Arena");
        let frames = |ui: &mut mh_ui::host::UiRuntime, n: usize| {
            for _ in 0..n {
                ui.update(1.0 / 60.0, [1920.0, 1080.0]);
                mh_ui::armory_natives::tick(&mut ui.vm, 1.0 / 60.0);
            }
        };
        ui.set_vitals(100, 100, true);
        frames(&mut ui, 30);
        ui.key_down("B");
        ui.key_up("B");
        frames(&mut ui, 60);
        assert!(ui.main_menu_visible(), "actual B action opens the mercenary picker");
        let picker = ui.find_any("BP_LoadoutPicker").expect("picker");
        let list = ui.find_any("ListView_Loadouts").expect("loadout list");
        let target = script_widget(&ui, "ListView_Loadouts[2]").expect("painted Veteran row");
        assert_eq!(Some(target), ui.vm.prop(list, "__entries").arr()[2].obj());
        assert!(script_widget(&ui, "ListView_Loadouts[99999]").is_none());
        assert!(script_widget(&ui, "ListView_Loadouts[invalid]").is_none());
        // The existing panel-slot syntax must keep resolving the active widget's content.
        let panel = ui.find_any("WidgetSwitcher_Main").expect("picker switcher");
        let active = ui.vm.prop(panel, "ActiveWidgetIndex").i() as usize;
        let content = ui.vm.o(ui.vm.o(panel).slots[active]).content.expect("active panel content");
        assert_eq!(script_widget(&ui, &format!("WidgetSwitcher_Main[{active}]")), Some(content));
        let centre = ui.centre(target).unwrap();
        ui.mouse_move(centre);
        frames(&mut ui, 2);
        ui.click(centre);
        frames(&mut ui, 30);
        assert_eq!(ui.vm.prop(picker, "SelectedId").i(), 2);
        let _ = mh_ui::armory::take_spawns(&mut ui.vm);
        let play = script_widget(&ui, "BP_PromptButton_Play").expect("painted Play");
        let centre = ui.centre(play).unwrap();
        ui.mouse_move(centre);
        frames(&mut ui, 2);
        ui.click(centre);
        frames(&mut ui, 30);
        let sent = mh_ui::armory::take_spawns(&mut ui.vm);
        assert_eq!(sent.len(), 1, "exactly one actual UI spawn-profile request");
        assert_eq!(sent[0].0, 2);
        assert_eq!(sent[0].1.field("Name").s(), "Veteran");
        assert!(!ui.main_menu_visible(), "Play closes the actual picker");
        // A stale last-painted preview Tick must not reclaim focus after Hide. One Escape reaches the game action.
        ui.key_down("Escape");
        ui.key_up("Escape");
        frames(&mut ui, 30);
        assert!(ui.main_menu_visible(), "one Escape opens the pause menu after Play");
        let suicide = script_widget(&ui, "SuicideButton").expect("painted suicide action");
        ui.click(ui.centre(suicide).unwrap());
        frames(&mut ui, 3);
        assert!(ui.take_actions().iter().any(|a| matches!(a, mh_ui::vm::Action::Pawn(p) if p == "RequestSuicide")),
            "actual suicide click must emit the pawn action");
    }

    #[test]
    fn parses_verbs_and_comments() {
        let c = parse("# smoke\nload_map\nspawn 2 # two\n\nwait 2s\nscreenshot a\ndump_state end\nquit\n").unwrap();
        let v: Vec<_> = c.iter().map(|c| (c.verb.as_str(), c.arg.as_str())).collect();
        assert_eq!(v, [("load_map", ""), ("spawn", "2"), ("wait", "2s"), ("screenshot", "a"), ("dump_state", "end"), ("quit", "")]);
        assert!(parse("explode now").is_err());
    }

    #[test]
    fn stamp_shape() {
        let (d, t) = utc_stamp();
        assert_eq!(d.len(), 10);
        assert_eq!(t.len(), 6);
    }
}

/// Ray through pixel (px, py) of the offscreen frame against every placed primitive's world box (Bevy Aabb in mesh
/// space through its GlobalTransform: an oriented box test), nearest entry first.
pub fn probe(world: &mut World, px: f32, py: f32) -> serde_json::Value {
    let (w, h) = (crate::camera::OFFSCREEN_SIZE.0 as f32, crate::camera::OFFSCREEN_SIZE.1 as f32);
    let Some((ct, proj)) = world
        .query_filtered::<(&GlobalTransform, &Projection), With<crate::camera::FlyCam>>()
        .iter(world)
        .next()
        .map(|(g, p)| (*g, p.clone()))
    else {
        return json!({"error": "no camera"});
    };
    let fov = if let Projection::Perspective(p) = &proj { p.fov } else { 1.0 };
    let t = (fov * 0.5).tan();
    let x = (px / w * 2.0 - 1.0) * t * (w / h);
    let y = (1.0 - py / h * 2.0) * t;
    let o = ct.translation();
    let d = (ct.rotation() * Vec3::new(x, y, -1.0)).normalize();
    let mut hits: Vec<(f32, String, String, String)> = Vec::new();
    let mut q = world.query::<(&bevy::camera::primitives::Aabb, &GlobalTransform, &crate::level::PrimInfo, &ChildOf, &Mesh3d)>();
    let mut names: Vec<(f32, Entity, String, Entity)> = Vec::new();
    let mut cands: Vec<(bevy::math::Affine3A, Vec3, Vec3, AssetId<Mesh>, Entity, String)> = Vec::new();
    for (bb, g, pi, parent, _m) in q.iter(world) {
        let inv = g.affine().inverse();
        let lo = inv.transform_point3(o);
        let ld = inv.transform_vector3(d);
        let (mn, mx) = (Vec3::from(bb.center - bb.half_extents), Vec3::from(bb.center + bb.half_extents));
        let (mut t0, mut t1) = (0.0f32, f32::MAX);
        let mut ok = true;
        for i in 0..3 {
            if ld[i].abs() < 1e-9 {
                if lo[i] < mn[i] || lo[i] > mx[i] {
                    ok = false;
                }
                continue;
            }
            let (a, b) = ((mn[i] - lo[i]) / ld[i], (mx[i] - lo[i]) / ld[i]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
        if ok && t0 <= t1 {
            cands.push((g.affine(), lo, ld, _m.0.id(), parent.parent(), pi.material.clone()));
        }
    }
    // exact: ray vs the primitive's triangles (Moller-Trumbore in mesh space), distance in world metres
    let meshes = world.resource::<Assets<Mesh>>();
    for (aff, lo, ld, mid, ent, mat) in cands {
        let Some(m) = meshes.get(mid) else { continue };
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else { continue };
        let idx: Vec<usize> = match m.indices() {
            Some(i) => i.iter().collect(),
            None => (0..pos.len()).collect(),
        };
        let mut best = f32::MAX;
        for t in idx.chunks_exact(3) {
            let (a, b, c) = (Vec3::from(pos[t[0]]), Vec3::from(pos[t[1]]), Vec3::from(pos[t[2]]));
            let (e1, e2) = (b - a, c - a);
            let pv = ld.cross(e2);
            let det = e1.dot(pv);
            if det.abs() < 1e-12 {
                continue;
            }
            let inv = 1.0 / det;
            let tv = lo - a;
            let u = tv.dot(pv) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let qv = tv.cross(e1);
            let v = ld.dot(qv) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let tt = e2.dot(qv) * inv;
            if tt > 0.0 && tt < best {
                best = tt;
            }
        }
        if best < f32::MAX {
            let wpt = aff.transform_point3(lo + ld * best);
            names.push(((wpt - o).length(), ent, mat, ent));
        }
    }
    names.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    for (dist, e, mat, _) in names.into_iter().take(8) {
        let (n, m) = world.get::<crate::level::UeMesh>(e).map(|u| (u.name.clone(), u.mesh.clone())).unwrap_or_default();
        hits.push((dist, n, m, mat));
    }
    json!({"pixel": [px, py], "origin": o.to_array(), "dir": d.to_array(),
           "hits": hits.iter().map(|h| json!({"dist_m": h.0, "component": h.1, "mesh": h.2, "material": h.3})).collect::<Vec<_>>()})
}

//! menu.rs - the main menu (r5 item 4): mh-ui draws BP_LocalPlay's panel (MainMenu resource); this module gives it
//! input and starts the match (port of godot/game/ui/main_menu.gd + mordhau_match.gd _default_bots).
//! Rows: mh-mode's ModeTable (FFA / TDM / SKM / DU / TF, game/mode/mode_table.gd). The map follows the mode: the map
//! metadata's map whose prefix is the mode's (BP_ArenaMapMetadata Maps, main_menu.gd map_name: <prefix>_Arena), or the
//! row's fallback while that map is not in the paks.
//! Keys: Up / Down (or W / S) = Game Mode combo, Left / Right (or A / D) = Bot Count slider, Enter = Start Match.
//! Script: `menu on|off`, `menu_select <i>`, `menu_bots <n>`, `menu_start`.
//! Bots: the Bot Count slider (BP_MordhauSlider_BotCount: default ENT_CONST_KISMET_lp_bot_count_default = 0,
//! BP_LocalPlay ubergraph 4939; range 0..SelectFloat(32, 64, IsConsolePlatform) = 64 on PC, ENT_CONST_KISMET_lp_bot_range,
//! ubergraph 4850) for the bot modes; room modes hide it and fill: Duel 1 bot, Teamfight MaxPeoplePerRoom - 1
//! (ENT_MODE_TF FLD_MODE_MAX_PEOPLE_PER_ROOM = 6 -> 5; mordhau_match.gd _default_bots).
//! UNCONFIRMED: the mode rules (mh-mode GameMode: rounds, rooms, score) are not run by the runtime yet; starting a
//! row loads its map and spawns the player plus the bots (their DefaultProfiles per --bot-profile).

use bevy::prelude::*;

pub const MAP_DIR: &str = "Mordhau/Content/Mordhau/Maps/Arena_Map/";

#[derive(Resource, Default, Clone, Debug, serde::Serialize)]
pub struct MenuState {
    /// Bot Count slider value (bot modes)
    pub bot_count: usize,
    pub bot_max: usize,
    /// Teamfight's MaxPeoplePerRoom
    pub tf_room: usize,
    pub started: Option<String>,
    pub map: Option<String>,
    pub bots: usize,
}

pub struct MenuPlugin {
    /// the legacy stand-in panel (mh-ui draw list) open at start (`--legacy-menu`)
    pub open: bool,
    /// the game's own front end (rust-ui: BP_MordhauHUD + BP_MainMenu run by mh-ui's Kismet VM) at start (`--menu`)
    pub real: bool,
}

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        let open = self.open;
        let real = self.real;
        if real {
            app.insert_resource(mh_ui::Screen::MainMenu);
        }
        app.init_resource::<MenuState>()
            .init_resource::<MenuBackdrop>()
            .add_systems(
                PostStartup,
                move |screen: Option<Res<mh_ui::Screen>>, bd: ResMut<MenuBackdrop>, lvl: ResMut<crate::level::LevelState>, src: Res<crate::source::Source>| start_backdrop(real, screen, bd, lvl, src),
            )
            .add_systems(Update, (place_backdrop, spawn_doll))
            .add_systems(
                Startup,
                move |mut m: ResMut<mh_ui::MainMenu>, mut st: ResMut<MenuState>, spec: Option<Res<crate::specdata::SpecData>>, src: Res<crate::source::Source>| {
                    m.modes = mh_mode::mode_table::rows().iter().map(|r| r.id.to_string()).collect();
                    m.selected = mh_mode::mode_table::rows().iter().position(|r| r.id == "DU").unwrap_or(0);
                    m.visible = open;
                    let s = spec.as_deref();
                    st.bot_count = s.map(|s| s.f("ENT_CONST_KISMET_lp_bot_count_default", "FLD_CONST_LITERAL", 0.0)).unwrap_or(0.0) as usize;
                    st.bot_max = s.map(|s| s.v2("ENT_CONST_KISMET_lp_bot_range", "FLD_CONST_LITERAL", [32.0, 64.0])[1]).unwrap_or(64.0) as usize;
                    st.tf_room = s.map(|s| s.f("ENT_MODE_TF", "FLD_MODE_MAX_PEOPLE_PER_ROOM", 6.0)).unwrap_or(6.0) as usize;
                    m.map_label = map_label(&m, &src);
                },
            )
            .add_systems(Startup, spawn_hint)
            .add_systems(Update, (menu_keys, menu_mouse, start_match, update_hint, open_level, real_hud).chain());
    }
}

/// the on-screen key help + the Bot Count value (mh-ui draws BP_LocalPlay without the slider)
#[derive(Component)]
pub struct MenuHint;

fn spawn_hint(mut commands: Commands, target: Option<Res<mh_ui::UiTarget>>) {
    let mut e = commands.spawn((
        MenuHint,
        Text::new(""),
        TextFont { font_size: bevy::text::FontSize::Px(18.0), ..default() },
        TextColor(Color::srgb(0.85, 0.78, 0.6)),
        Node { position_type: PositionType::Absolute, left: Val::Px(24.0), bottom: Val::Px(24.0), ..default() },
        Visibility::Hidden,
    ));
    if let Some(t) = target {
        e.insert(bevy::ui::UiTargetCamera(t.0));
    }
}

fn update_hint(m: Res<mh_ui::MainMenu>, st: Res<MenuState>, mut q: Query<(&mut Text, &mut Visibility), With<MenuHint>>) {
    for (mut t, mut v) in q.iter_mut() {
        let want = if m.visible { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
        if !m.visible {
            continue;
        }
        let row = mh_mode::mode_table::rows().get(m.selected);
        let bots = row.map(|r| if r.rooms { format!("{} (room fill)", row_bots(m.selected, &st)) } else { format!("{} (Left / Right)", st.bot_count) }).unwrap_or_default();
        let s = format!("Up / Down or click the Game Mode box: mode    Bots: {bots}    Enter or click Start Match: play");
        if t.0 != s {
            t.0 = s;
        }
    }
}

/// a rect of mh-ui's draw list (logical px, x y w h)
fn rect_of(d: &mh_ui::Draw) -> [f64; 4] {
    match d {
        mh_ui::Draw::Rect { rect, .. } | mh_ui::Draw::Image { rect, .. } | mh_ui::Draw::Text { rect, .. } => *rect,
    }
}

/// mouse: a left click on Start Match starts the row, on the Game Mode box cycles the mode (main_menu.gd: the combo
/// box "click to pick the next mode", the StartButton pressed -> _start); hit rects = mh-ui's own draw list
#[allow(clippy::too_many_arguments)]
fn menu_mouse(
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    windows: Query<&Window>,
    ui: Option<NonSend<mh_ui::UiState>>,
    vit: Option<Res<mh_ui::HudVitals>>,
    sb: Option<Res<mh_ui::Scoreboard>>,
    vp: Option<Res<mh_ui::UiViewport>>,
    mut m: ResMut<mh_ui::MainMenu>,
    mut out: MessageWriter<mh_ui::MenuChoice>,
) {
    let (Some(mouse), Some(ui), Some(vit), Some(sb), Some(vp)) = (mouse, ui, vit, sb, vp) else { return };
    if !m.visible || m.modes.is_empty() || !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(c) = windows.iter().find_map(|w| w.cursor_position()) else { return };
    let list = mh_ui::draw_list(&ui, &vit, &sb, &m, [vp.0.x as f64, vp.0.y as f64]);
    let hit = |id: &str| {
        list.iter().any(|(n, d)| {
            let r = rect_of(d);
            n == id && (c.x as f64) >= r[0] && (c.x as f64) <= r[0] + r[2] && (c.y as f64) >= r[1] && (c.y as f64) <= r[1] + r[3]
        })
    };
    if hit("menu.start") {
        out.write(mh_ui::MenuChoice(m.selected));
    } else if hit("menu.mode_box") || hit("menu.mode") {
        let n = m.modes.len();
        m.selected = (m.selected + 1) % n;
    }
}

/// The real front end's Start Match: BP_LocalPlay's StartButton ends in UMordhauGameInstance::ClientTravel rva
/// 0x1535ce0 with "<ModePrefix>_<Map>" and "PlayerCount=<bot slider + 1>" (rust-ui UiAction::OpenLevel). The prefix
/// picks the ModeTable row (UGameMapsSettings GameModeMapPrefixes), the map package is found by name in the paks, the
/// bots are PlayerCount - 1 for bot modes (the room modes fill their room: row_bots); then the in-match HUD
/// (Screen::Match + MatchMap). UiAction::Quit exits.
#[allow(clippy::too_many_arguments)]
fn open_level(
    mut ev: MessageReader<mh_ui::UiAction>,
    mut st: ResMut<MenuState>,
    mut lvl: ResMut<crate::level::LevelState>,
    mut q: ResMut<crate::sim::SpawnQueue>,
    mut bq: ResMut<crate::sim::BotQueue>,
    src: Res<crate::source::Source>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    for a in ev.read() {
        match a {
            mh_ui::UiAction::OpenLevel { map, options } => {
                // (rust-ui) REWRITE-ONLY: Local Match's "Combat Test" tile travels to "<prefix>_TestLevel" (mh-ui
                // menu_data::combat_test_metadata): the combat test level (level.rs TEST_LEVEL, duel HUD)
                let test = map.ends_with("_TestLevel");
                let Some(path) = (if test { Some(crate::level::TEST_LEVEL.to_string()) } else { find_map(&src, map) }) else {
                    warn!("menu: OpenLevel {map}: no such map in the paks");
                    continue;
                };
                let prefix = map.split('_').next().unwrap_or("");
                let row = mh_mode::mode_table::rows().iter().position(|r| r.id == prefix);
                let players = options
                    .split(['?', '&', ' '])
                    .find_map(|kv| kv.strip_prefix("PlayerCount="))
                    .and_then(|n| n.trim().parse::<usize>().ok())
                    .unwrap_or(1);
                let bots = match row.and_then(|i| mh_mode::mode_table::rows().get(i)) {
                    Some(r) if r.rooms => row_bots(row.unwrap(), &st),
                    _ => players.saturating_sub(1),
                };
                lvl.request = Some(path.clone());
                q.0 += 1;
                bq.0 += bots;
                st.started = Some(prefix.to_string());
                st.map = Some(path.clone());
                st.bots = bots;
                commands.insert_resource(mh_ui::Screen::Match);
                commands.insert_resource(mh_ui::MatchMap(if test { crate::level::match_map_name(crate::level::TEST_LEVEL) } else { map.clone() }));
                info!("menu: OpenLevel {map} ({options}) -> {path}, {bots} bots");
            }
            mh_ui::UiAction::Quit => {
                exit.write(AppExit::Success);
            }
            // (first-person r3) BP_MainMenu:QuitMatch -> UDestroyMordhauServerSession OnFailure -> ExecuteConsoleCommand
            // ("disconnect") (ubergraph @3191): UEngine's "disconnect" travels the client to the default map, i.e.
            // GameDefaultMap (DefaultEngine.ini GameMapsSettings), the front end
            mh_ui::UiAction::Console(c) if c.trim().eq_ignore_ascii_case("disconnect") => {
                let mut nb = MenuBackdrop::default();
                if let Some(map) = fill_backdrop(&mut nb, &src) {
                    lvl.request = Some(map);
                    commands.insert_resource(nb);
                    commands.insert_resource(mh_ui::Screen::MainMenu);
                    commands.insert_resource(mh_ui::MatchMap(String::new()));
                    st.started = None;
                    st.map = None;
                    info!("menu: disconnect -> the front end");
                }
            }
            mh_ui::UiAction::Console(c) => info!("menu: console command {c} (not handled)"),
            // (first-person r3) the escape menu's pawn calls: level.rs ui_pawn_actions
            mh_ui::UiAction::Pawn(_) => {}
        }
    }
}

/// the in-match HUD's pawn data (rust-ui proxies: GetOwningPlayerPawn -> Health / Stamina, RightHandEquipment
/// bCanAttack for BP_Crosshair) from the player's fighter every frame
fn real_hud(rt: Option<NonSendMut<mh_ui::real::Rt>>, sim: Option<NonSend<crate::sim::Sim>>, pc: Option<Res<crate::input::PlayerControl>>) {
    let (Some(mut rt), Some(sim)) = (rt, sim) else { return };
    let id = pc.map(|p| p.id).unwrap_or(0);
    let views = sim.0.fighters();
    match views.iter().find(|v| v.id == id) {
        Some(v) => {
            rt.ui.set_vitals(v.health.round() as i64, v.stamina, v.health > 0.0);
            // a held melee weapon can attack (AMordhauEquipment bCanAttack; no ranged / tool state in the sim view yet)
            // the held equipment's Blueprint: its Ammo / MaxAmmo defaults drive BP_EquipmentInfoDisplay (shown only for
            // GetAmmo() != 255: ranged / throwables), rust-ui r2
            rt.ui.set_equipment(sim.0.weapon_path(id).as_deref(), true, None);
        }
        None => rt.ui.set_vitals(0, 0, false),
    }
}

/// a map package by its name ("FFA_Arena" -> "Mordhau/Content/Mordhau/Maps/Arena_Map/FFA_Arena")
pub fn find_map(src: &crate::source::Source, map: &str) -> Option<String> {
    let v = src.vfs.as_ref()?;
    let want = format!("/{map}.umap");
    v.list().find(|p| p.ends_with(&want) || p.to_lowercase().ends_with(&want.to_lowercase())).map(|p| p.trim_end_matches(".umap").to_string())
}

/// the selected row's map (or its fallback when the map is not in the paks)
pub fn row_map(i: usize, src: &crate::source::Source) -> Option<&'static str> {
    let row = mh_mode::mode_table::rows().get(i)?;
    let has = |name: &str| src.vfs.as_ref().is_some_and(|v| v.has(&format!("{MAP_DIR}{name}.umap")));
    Some(if has(row.map) || row.fallback.is_empty() { row.map } else { row.fallback })
}

/// "Arena  (DU_Arena)" (main_menu.gd entry text: the map metadata's Name "Arena" + map_name())
pub fn map_label(m: &mh_ui::MainMenu, src: &crate::source::Source) -> String {
    format!("Arena  ({})", row_map(m.selected, src).unwrap_or(""))
}

/// the bots a row starts with: the slider for bot modes, the room fill for room modes
pub fn row_bots(i: usize, st: &MenuState) -> usize {
    match mh_mode::mode_table::rows().get(i) {
        Some(r) if r.rooms => {
            if r.id == "DU" {
                1
            } else {
                st.tf_room.saturating_sub(1)
            }
        }
        _ => st.bot_count,
    }
}

fn menu_keys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut m: ResMut<mh_ui::MainMenu>,
    mut st: ResMut<MenuState>,
    src: Res<crate::source::Source>,
    mut out: MessageWriter<mh_ui::MenuChoice>,
) {
    if !m.visible || m.modes.is_empty() {
        return;
    }
    if let Some(k) = keys {
        let n = m.modes.len();
        if k.just_pressed(KeyCode::ArrowDown) || k.just_pressed(KeyCode::KeyS) {
            m.selected = (m.selected + 1) % n;
        }
        if k.just_pressed(KeyCode::ArrowUp) || k.just_pressed(KeyCode::KeyW) {
            m.selected = (m.selected + n - 1) % n;
        }
        // SnapToDiscreteValues: step 1
        if k.just_pressed(KeyCode::ArrowRight) || k.just_pressed(KeyCode::KeyD) {
            st.bot_count = (st.bot_count + 1).min(st.bot_max);
        }
        if k.just_pressed(KeyCode::ArrowLeft) || k.just_pressed(KeyCode::KeyA) {
            st.bot_count = st.bot_count.saturating_sub(1);
        }
        if k.just_pressed(KeyCode::Enter) || k.just_pressed(KeyCode::NumpadEnter) {
            out.write(mh_ui::MenuChoice(m.selected));
        }
    }
    // the map entry follows the mode (script menu_select too)
    let label = map_label(&m, &src);
    if m.map_label != label {
        m.map_label = label;
    }
}

/// MenuChoice -> the row's map (or its fallback) loads, the player and the row's bots spawn, the menu closes.
#[allow(clippy::too_many_arguments)]
fn start_match(
    mut ev: MessageReader<mh_ui::MenuChoice>,
    mut m: ResMut<mh_ui::MainMenu>,
    mut st: ResMut<MenuState>,
    mut lvl: ResMut<crate::level::LevelState>,
    mut q: ResMut<crate::sim::SpawnQueue>,
    mut bq: ResMut<crate::sim::BotQueue>,
    src: Res<crate::source::Source>,
) {
    for c in ev.read() {
        let Some(row) = mh_mode::mode_table::rows().get(c.0) else { continue };
        let Some(map) = row_map(c.0, &src) else { continue };
        let path = format!("{MAP_DIR}{map}");
        let bots = row_bots(c.0, &st);
        lvl.request = Some(path.clone());
        q.0 += 1;
        bq.0 += bots;
        m.visible = false;
        st.started = Some(row.id.to_string());
        st.map = Some(path);
        st.bots = bots;
        info!("menu: {} on {} with {} bots", row.id, map, bots);
    }
}

/// The front end's 3D backdrop: the game starts on GameDefaultMap (DefaultEngine.ini
/// [/Script/EngineSettings.GameMapsSettings] GameDefaultMap=/Game/Mordhau/Maps/MainMenu/MainMenu) whose
/// BP_MordhauMainMenuPawn (AutoPossessPlayer Player0) is the view target: its "Camera" CameraComponent (FieldOfView
/// 33 in the map) is the view the menu draws over. The doll characters (BP_CharacterDoll / BP_RandomProfileDoll) and
/// the party-size CameraActors (2Players..6Players) are not run here (UNCONFIRMED: the menu map's Blueprint logic).
#[derive(Resource, Default, Clone, Debug, serde::Serialize)]
pub struct MenuBackdrop {
    pub map: String,
    /// UE [x, y, z, pitch, yaw] of the view camera
    pub cam: Option<[f32; 5]>,
    /// horizontal FOV (UE FieldOfView)
    pub fov_h: Option<f32>,
    pub placed: bool,
    pub note: Option<String>,
    /// the BP_CharacterDoll the player's character stands as: (actor name, its root's world transform, UE)
    #[serde(skip)]
    pub doll: Option<(String, mh_level::xf::Xf, Option<mh_level::xf::Xf>)>,
    pub doll_name: Option<String>,
    pub doll_spawned: bool,
    /// the Armory's observer camera has the view (place_backdrop)
    pub armory_view: bool,
}

fn ue_pkg(path: &str) -> String {
    // "/Game/Mordhau/Maps/MainMenu/MainMenu.MainMenu" -> "Mordhau/Content/Mordhau/Maps/MainMenu/MainMenu"
    let p = path.split('.').next().unwrap_or(path);
    match p.strip_prefix("/Game/") {
        Some(r) => format!("Mordhau/Content/{r}"),
        None => p.trim_start_matches('/').to_string(),
    }
}

/// the menu map's view: the auto-possessed pawn's camera component, its world transform and FieldOfView
fn menu_view(pk: &mh_level::Pkgs, map: &str) -> Option<([f32; 5], f32)> {
    let ex = pk.load_pkg(map);
    let pawn = ex.iter().find(|e| {
        let p = pk.props(e);
        p.get("AutoPossessPlayer").and_then(|v| v.as_str()) == Some("EAutoReceiveInput::Player0")
    })?;
    let cam = pk.obj(pk.props(pawn).get("Camera"))?;
    let xf = pk.world_xf(&cam);
    let t = xf.translation();
    let m = &xf.m;
    // the component's +X axis (column 0): yaw / pitch
    let (fx, fy, fz) = (m[0][0], m[1][0], m[2][0]);
    let yaw = fy.atan2(fx).to_degrees() as f32;
    let pitch = fz.atan2((fx * fx + fy * fy).sqrt()).to_degrees() as f32;
    // UCameraComponent FieldOfView default 90 (UE ctor) when the instance does not set it
    let fov = pk.props(&cam).get("FieldOfView").and_then(|v| v.as_f64()).unwrap_or(90.0) as f32;
    Some(([t[0] as f32, t[1] as f32, t[2] as f32, pitch, yaw], fov))
}

/// the doll the solo player stands as. The MainMenu level Blueprint (MainMenu_C ExecuteUbergraph, decoded in
/// state/world_kismet/MainMenu.txt) fills Characters = [BP_RandomProfileDoll_507, BP_CharacterDoll_2, BP_CharacterDoll2,
/// BP_CharacterDoll3, BP_CharacterDoll4, BP_CharacterDoll5] (@3470..@4240) and shows Characters[PartySize - 1]'s
/// predecessors (@8556 Array_Get(Characters, PartySizeOverride) .Mesh.SetVisibility(true)); party member i takes
/// Characters[i] (OnPartyUpdated @496..@1029). Party of one: Characters[0] = the BP_RandomProfileDoll, and the pawn
/// stays on PawnTransforms[0] = its own Camera transform (@6238, @8379).
fn menu_doll(pk: &mh_level::Pkgs, map: &str, _cam: [f32; 5]) -> Option<(String, mh_level::xf::Xf, Option<mh_level::xf::Xf>)> {
    let ex = pk.load_pkg(map);
    let mut best: Option<(f64, String, mh_level::xf::Xf, Option<mh_level::xf::Xf>)> = None;
    for e in ex.iter() {
        let ty = e.get("Type").and_then(|t| t.as_str()).unwrap_or("");
        if ty != "BP_RandomProfileDoll_C" {
            continue;
        }
        let Some(root) = pk.obj(pk.props(e).get("RootComponent")) else { continue };
        let xf = pk.world_xf(&root);
        let name = e.get("Name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let perp = 0.0;
        if best.is_none() {
            // the doll's character mesh (its "CharacterMesh0" HumanMeshComponent over the BP template chain) relative to
            // the capsule: where the body stands
            let mesh = pk.props(e).get("Mesh").and_then(|m| pk.obj(Some(m))).map(|m| {
                let p = pk.props(&m);
                let v = |k: &str| p.get(k).map(|x| ["X", "Y", "Z"].map(|c| x.get(c).and_then(|y| y.as_f64()).unwrap_or(0.0)));
                let r = p.get("RelativeRotation");
                let rr = |k: &str| r.and_then(|x| x.get(k)).and_then(|y| y.as_f64()).unwrap_or(0.0);
                let q = crate::ue::rot_quat(Some(&serde_json::json!({"Pitch": rr("Pitch"), "Yaw": rr("Yaw"), "Roll": rr("Roll")})));
                mh_level::xf::Xf::trs(v("RelativeLocation").unwrap_or([0.0; 3]), q.map(|x| x as f64), v("RelativeScale3D").unwrap_or([1.0; 3]))
            });
            best = Some((perp, name, xf, mesh));
        }
    }
    best.map(|b| (b.1, b.2, b.3))
}

/// the player's character on the backdrop's doll: the run's player loadout (Selected.player), idle clip, at the doll's
/// capsule with the character mesh's relative transform (the sim's mesh_xf, as fighters)
#[allow(clippy::too_many_arguments)]
fn spawn_doll(
    mut commands: Commands,
    mut bd: ResMut<MenuBackdrop>,
    lvl: Res<crate::level::LevelState>,
    pb: Option<Res<crate::fighter::PakBody>>,
    sel: Option<Res<crate::loadout::Selected>>,
    sim: Option<NonSend<crate::sim::Sim>>,
) {
    let (Some(sel), Some(sim)) = (sel, sim) else { return };
    if bd.doll_spawned || bd.map.is_empty() || lvl.map != bd.map || !lvl.loaded {
        return;
    }
    let (Some(pb), Some((name, xf, mesh))) = (pb, bd.doll.clone()) else { return };
    bd.doll_spawned = true;
    let mut e = commands.spawn((Transform::from_matrix(Mat4::from(crate::plan::xf_gltf(&xf))), Visibility::default(), Name::new(format!("MenuDoll {name}"))));
    if let Some(r) = lvl.root {
        e.insert(ChildOf(r));
    }
    let root = e.id();
    let mesh_tr = match mesh {
        Some(m) => Transform::from_matrix(Mat4::from(crate::plan::xf_gltf(&m))),
        None => sim.0.mesh_xf().as_ref().map(crate::fighter::ue_to_bevy).unwrap_or_else(|| Transform::from_rotation(crate::fighter::yaw_quat(crate::fighter::MESH_YAW))),
    };
    // posed by the character Anim Blueprint's idle for what it holds (armory_host DollPose: BP_RandomProfileDoll has no
    // AnimClass of its own, it inherits AB_MordhauCharacterAnimation)
    let g = sel.gear.get(&sel.player);
    commands.entity(root).insert(crate::armory_host::DollPose { right: g.map(|g| g.weapon.clone()).unwrap_or_default(), left: g.map(|g| g.left.clone()).unwrap_or_default() });
    crate::fighter::spawn_pak_body(&mut commands, &pb, root, mesh_tr, false, sel.player, u32::MAX);
}

/// Screen::MainMenu at start: load GameDefaultMap as the backdrop
fn start_backdrop(real: bool, screen: Option<Res<mh_ui::Screen>>, mut bd: ResMut<MenuBackdrop>, mut lvl: ResMut<crate::level::LevelState>, src: Res<crate::source::Source>) {
    if !real || screen.as_deref().is_none_or(|s| *s != mh_ui::Screen::MainMenu) {
        return;
    }
    if let Some(map) = fill_backdrop(&mut bd, &src) {
        if lvl.request.is_none() && !lvl.loaded {
            lvl.request = Some(map);
        }
    }
}

/// (first-person r3) GameDefaultMap's view camera / FOV / doll into the backdrop; the map to load (start_backdrop at
/// start, and the escape menu's "disconnect" back to the front end)
fn fill_backdrop(bd: &mut MenuBackdrop, src: &crate::source::Source) -> Option<String> {
    let v = src.vfs.as_ref()?;
    let gdm = mh_level::config::value(v, "DefaultEngine.ini", "/Script/EngineSettings.GameMapsSettings", "GameDefaultMap");
    if gdm.is_empty() {
        bd.note = Some("no GameDefaultMap in DefaultEngine.ini".into());
        return None;
    }
    let map = ue_pkg(&gdm);
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(v.clone()));
    match menu_view(&pk, &map) {
        Some((c, f)) => {
            bd.cam = Some(c);
            bd.fov_h = Some(f);
            bd.doll = menu_doll(&pk, &map, c);
            bd.doll_name = bd.doll.as_ref().map(|d| format!("{} mesh {:?}", d.0, d.2.map(|m| m.translation())));
        }
        None => bd.note = Some(format!("{map}: no auto-possessed pawn camera")),
    }
    bd.map = map.clone();
    Some(map)
}

/// once the backdrop map is in: the view camera there, its FOV while the front end shows
#[allow(clippy::too_many_arguments)]
fn place_backdrop(
    mut bd: ResMut<MenuBackdrop>,
    lvl: Res<crate::level::LevelState>,
    screen: Option<Res<mh_ui::Screen>>,
    req: Option<ResMut<crate::camera::CamRequest>>,
    prev: Option<Res<mh_ui::armory::ArmoryPreview>>,
    windows: Query<&Window>,
    mut cams: Query<(&mut Projection, &mut Transform), With<crate::camera::FlyCam>>,
) {
    if bd.map.is_empty() || lvl.map != bd.map || !lvl.loaded {
        return;
    }
    let Some(mut req) = req else { return };
    if !bd.placed {
        bd.placed = true;
        req.0 = bd.cam;
    }
    if !screen.as_deref().is_some_and(|s| *s == mh_ui::Screen::MainMenu) {
        return;
    }
    // the Armory: the view goes through the customization platform's CustomizationObserver camera
    // (AMordhauCameraManager::EnterCustomization; FOV 26 = BP_MordhauCustomizationPlatform:UpdateCamera@1701/@4379,
    // rust-armory's mh_ui::armory::ArmoryPreview); leaving it the view returns to the menu pawn's camera
    let armory = prev.as_deref().filter(|p| p.active && p.fov > 0.0);
    match armory {
        Some(p) => {
            bd.armory_view = true;
            let [x, y, z] = p.camera_location;
            let [pitch, yaw, roll] = p.camera_rotation;
            let xf = crate::ue::xf(Some(&serde_json::json!({"X": x, "Y": y, "Z": z})), crate::ue::rot_quat(Some(&serde_json::json!({"Pitch": pitch, "Yaw": yaw, "Roll": roll}))), None);
            let t = Transform::from_translation(xf.translation.into()).with_rotation(crate::level::ue_look_basis(&xf));
            for (_, mut ct) in cams.iter_mut() {
                *ct = t;
            }
        }
        None if bd.armory_view => {
            bd.armory_view = false;
            req.0 = bd.cam;
        }
        None => {}
    }
    // the view camera's FieldOfView is the VERTICAL field of view: AMordhauPlayerController::BeginPlay rva 0x15c7ed0
    // sets GetLocalPlayer()->AspectRatioAxisConstraint (ULocalPlayer +0x94) = 0 = AspectRatio_MaintainYFOV, over
    // BaseEngine.ini's MaintainXFOV; ULocalPlayer::GetProjectionData rva 0x31bd200 passes that byte (movzx edx,
    // [r14+0x94]) to FMinimalViewInfo::CalculateProjectionMatrixGivenView rva 0x2f2e570, whose axis test (2 = major
    // axis on a wide viewport, 1 = X) falls to the Y-axis branch. Checked against the real game: the Mercenaries doll
    // (p26_002: x 0.72, 63 % height) and the main-menu doll (state/reference/real/00_start.png, head 7.2 px/cm at
    // 1080p; vertical 33 deg at the pawn camera's 243 cm gives 7.5, horizontal would give 13.3). Bevy's fov is
    // vertical too
    let fov = armory.map(|p| p.fov).or(bd.fov_h);
    if let Some(fv) = fov {
        let _ = &windows;
        for (mut p, _) in cams.iter_mut() {
            if let Projection::Perspective(pp) = &mut *p {
                pp.fov = fv.to_radians();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::keyboard::{Key, KeyboardInput};
    use bevy::input::ButtonState;

    /// the menu app as main.rs builds it, minus rendering: Bevy's InputPlugin (keyboard_input_system turns
    /// KeyboardInput messages into ButtonInput<KeyCode>, as bevy_winit feeds them from the window) + mh-ui's
    /// resources + MenuPlugin
    fn menu_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin))
            .add_message::<mh_ui::MenuChoice>()
            .add_message::<mh_ui::UiAction>()
            .init_resource::<mh_ui::MainMenu>()
            .init_resource::<crate::level::LevelState>()
            .init_resource::<crate::sim::SpawnQueue>()
            .init_resource::<crate::sim::BotQueue>()
            .insert_resource(crate::source::Source {
                level: crate::source::LevelSrc::Pak,
                assets: crate::source::AssetSrc::Pak,
                materials: crate::source::MatSrc::UeTint,
                vfs: None,
                pak: None,
                mount_error: None,
            })
            .add_plugins(MenuPlugin { open: true, real: false });
        app.update();
        app
    }

    /// one key tap the way the window delivers it: a Pressed message one frame, Released the next
    fn tap(app: &mut App, key: KeyCode) {
        let window = Entity::PLACEHOLDER;
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput { key_code: key, logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified), state, text: None, repeat: false, window });
            app.update();
        }
    }

    #[test]
    fn real_keyboard_path_selects_and_starts() {
        let mut app = menu_app();
        let du = mh_mode::mode_table::rows().iter().position(|r| r.id == "DU").unwrap();
        assert!(app.world().resource::<mh_ui::MainMenu>().visible);
        assert_eq!(app.world().resource::<mh_ui::MainMenu>().selected, du);
        tap(&mut app, KeyCode::ArrowUp);
        assert_eq!(app.world().resource::<mh_ui::MainMenu>().selected, du - 1, "Up moves the mode");
        tap(&mut app, KeyCode::ArrowDown);
        tap(&mut app, KeyCode::ArrowDown);
        assert_eq!(app.world().resource::<mh_ui::MainMenu>().selected, du + 1, "Down moves the mode");
        // back to FFA (row 0), two bots, start
        for _ in 0..(du + 1) {
            tap(&mut app, KeyCode::ArrowUp);
        }
        assert_eq!(app.world().resource::<mh_ui::MainMenu>().selected, 0);
        tap(&mut app, KeyCode::ArrowRight);
        tap(&mut app, KeyCode::ArrowRight);
        tap(&mut app, KeyCode::ArrowLeft);
        tap(&mut app, KeyCode::ArrowRight);
        assert_eq!(app.world().resource::<MenuState>().bot_count, 2, "Left / Right set the bot count");
        tap(&mut app, KeyCode::Enter);
        let st = app.world().resource::<MenuState>();
        assert_eq!(st.started.as_deref(), Some("FFA"));
        assert_eq!(st.bots, 2);
        assert!(!app.world().resource::<mh_ui::MainMenu>().visible, "the menu closes on start");
        assert_eq!(app.world().resource::<crate::level::LevelState>().request.as_deref(), Some("Mordhau/Content/Mordhau/Maps/Arena_Map/FFA_Arena"));
        assert_eq!(app.world().resource::<crate::sim::BotQueue>().0, 2);
        assert_eq!(app.world().resource::<crate::sim::SpawnQueue>().0, 1);
    }

    /// the mouse path: a window with the cursor over mh-ui's Game Mode box / Start Match (rects from mh-ui's own draw
    /// list), MouseButtonInput messages as from the window. Skipped without the install (the UI needs the paks).
    #[test]
    fn real_mouse_path_cycles_and_starts() {
        let Ok(vfs) = mh_pak::vfs::Vfs::mount_default() else {
            eprintln!("skip: no Mordhau install");
            return;
        };
        let ui = mh_ui::load_state(&mh_ui::Paks(std::sync::Arc::new(vfs)));
        let mut app = menu_app();
        let mut w = Window::default();
        w.resolution.set(1280.0, 720.0);
        let win = app.world_mut().spawn(w).id();
        app.insert_resource(mh_ui::UiViewport(Vec2::new(1280.0, 720.0)))
            .init_resource::<mh_ui::HudVitals>()
            .init_resource::<mh_ui::Scoreboard>();
        let centre = |app: &App, ui: &mh_ui::UiState, id: &str| -> Vec2 {
            let w = app.world();
            let l = mh_ui::draw_list(ui, w.resource::<mh_ui::HudVitals>(), w.resource::<mh_ui::Scoreboard>(), w.resource::<mh_ui::MainMenu>(), [1280.0, 720.0]);
            let r = l.iter().find(|(n, _)| n == id).map(|(_, d)| rect_of(d)).unwrap_or_else(|| panic!("no {id} in the draw list"));
            Vec2::new((r[0] + r[2] / 2.0) as f32, (r[1] + r[3] / 2.0) as f32)
        };
        let mode_box = centre(&app, &ui, "menu.mode_box");
        let start = centre(&app, &ui, "menu.start");
        app.insert_non_send(ui);
        let click = |app: &mut App, at: Vec2| {
            app.world_mut().get_mut::<Window>(win).unwrap().set_cursor_position(Some(at));
            for state in [ButtonState::Pressed, ButtonState::Released] {
                app.world_mut().write_message(bevy::input::mouse::MouseButtonInput { button: MouseButton::Left, state, window: win });
                app.update();
            }
        };
        let du = mh_mode::mode_table::rows().iter().position(|r| r.id == "DU").unwrap();
        click(&mut app, mode_box);
        assert_eq!(app.world().resource::<mh_ui::MainMenu>().selected, du + 1, "a click on the Game Mode box picks the next mode");
        click(&mut app, start);
        assert_eq!(app.world().resource::<MenuState>().started.as_deref(), Some("TF"), "a click on Start Match starts");
        assert!(!app.world().resource::<mh_ui::MainMenu>().visible);
    }

    #[test]
    fn row_bots_follow_main_menu_gd() {
        let st = MenuState { bot_count: 3, bot_max: 64, tf_room: 6, ..Default::default() };
        let idx = |id: &str| mh_mode::mode_table::rows().iter().position(|r| r.id == id).unwrap();
        assert_eq!(row_bots(idx("DU"), &st), 1);
        assert_eq!(row_bots(idx("TF"), &st), 5);
        assert_eq!(row_bots(idx("FFA"), &st), 3);
    }
}

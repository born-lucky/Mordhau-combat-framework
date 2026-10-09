//! Original framework presentation. Reads combat state and never drives its clocks.
use bevy::prelude::*;
use crate::sim::Sim;
use crate::input::PlayerControl;
mod controls;

#[derive(Component)]
struct LabStatus;

#[derive(Component)]
struct ControlLegend;

#[derive(Component)]
struct VitalsPanel;

#[derive(Resource, Default, serde::Serialize)]
pub struct LabMenu { pub open: bool }

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LabInput;

/// Independent writable settings. Fresh installs use native defaults (including kick on F).
/// Importing personal vanilla bindings is an explicit action in the controls editor.
/// Call before Bevy creates threads; neither the game nor the main rewrite's settings are modified.
pub fn initialize_settings() -> std::io::Result<()> {
    if std::env::var_os("MH_CONFIG_DIR").is_none() {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            std::env::set_var("MH_CONFIG_DIR", std::path::PathBuf::from(base)
                .join("MordhauCombatFramework/Saved/Config/WindowsClient"));
        }
    }
    std::fs::create_dir_all(mh_ui::settings::config_dir())
}

#[cfg(test)]
fn copy_input_if_missing(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;
    if target.exists() { return Ok(()); }
    let bytes = std::fs::read(source)?;
    if let Some(parent) = target.parent() { std::fs::create_dir_all(parent)?; }
    match std::fs::OpenOptions::new().write(true).create_new(true).open(target) {
        Ok(mut file) => file.write_all(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

#[derive(Component)]
struct MenuRoot;

#[derive(Component, Clone, Copy)]
enum MenuAction { Resume, Perspective, Controls, Tools, Quit }

#[derive(Component, Clone, Copy)]
enum Vital { Health, Stamina }

#[derive(Component)]
struct VitalFill(Vital);

#[derive(Component)]
struct VitalLabel(Vital);

pub struct FrameworkUiPlugin;
impl Plugin for FrameworkUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(controls::ControlsPlugin).init_resource::<LabMenu>()
            .add_systems(Startup, setup)
            .add_systems(Update, menu_input.in_set(LabInput).before(crate::input::player_input))
            .add_systems(Update, update.after(crate::sim::tick_sim_frame))
            .add_systems(PostUpdate, original_hud_visibility
                .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate));
    }
}

fn label(text: &str, size: f32, node: Node, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont { font_size: bevy::text::FontSize::Px(size), ..default() },
        TextColor(color),
        node,
    )
}

fn setup(mut commands: Commands, camera: Res<crate::camera::CamEntity>) {
    commands.spawn((MenuRoot, UiTargetCamera(camera.0), GlobalZIndex(100), Visibility::Hidden,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0),
            padding: UiRect::all(Val::Px(48.0)), column_gap: Val::Px(48.0), align_items: AlignItems::Center, ..default() },
        BackgroundColor(Color::srgba(0.022, 0.055, 0.074, 0.97)),
    )).with_children(|root| {
        root.spawn((Node { width: Val::Percent(48.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(20.0), ..default() },))
            .with_children(|left| {
                left.spawn(label("01 / TEST RANGE", 15.0, Node::default(), Color::srgb(0.72, 0.64, 0.40)));
                left.spawn(label("STEEL\nLAB", 76.0, Node::default(), Color::srgb(0.91, 0.94, 0.91)));
                left.spawn(label("A local workshop for first-person melee combat.", 18.0, Node::default(), Color::srgb(0.51, 0.70, 0.73)));
                left.spawn(label("Mordhau Combat Framework\nIndependent interface and training figures\nCombat animation and simulation use your local game data.",
                    14.0, Node::default(), Color::srgb(0.61, 0.71, 0.72)));
                left.spawn(label("Built on Triternion's remarkable combat system.\nSupport the original game.", 13.0, Node::default(), Color::srgb(0.65, 0.62, 0.47)));
            });
        root.spawn((Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, row_gap: Val::Px(12.0), ..default() },))
            .with_children(|right| {
                right.spawn(label("YOUR SESSION", 14.0, Node::default(), Color::srgb(0.51, 0.70, 0.73)));
                for (action, title) in [(MenuAction::Resume, "RETURN TO RANGE"), (MenuAction::Perspective, "CHANGE PERSPECTIVE"),
                    (MenuAction::Controls, "CONTROLS / BINDINGS"), (MenuAction::Tools, "COMBAT TOOLS"), (MenuAction::Quit, "EXIT FRAMEWORK")] {
                    right.spawn((Button, action,
                        Node { width: Val::Percent(100.0), min_height: Val::Px(58.0), padding: UiRect::all(Val::Px(18.0)), ..default() },
                        BackgroundColor(Color::srgb(0.09, 0.17, 0.21)),
                    )).with_children(|button| { button.spawn(label(title, 17.0, Node::default(), Color::srgb(0.91, 0.94, 0.91))); });
                }
                right.spawn(label("ESC / ENTER to return\nF7 tracers   /   F8 parry geometry   /   F9 tools", 13.0, Node::default(), Color::srgb(0.61, 0.71, 0.72)));
            });
    });

    // Explicit roots also render when the gameplay camera targets an offscreen image.
    // Bevy's automatic UI camera selection only considers primary-window cameras.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0), left: Val::Px(0.0),
            width: Val::Percent(100.0), height: Val::Px(64.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.078, 0.165, 0.224, 0.85)),
        UiTargetCamera(camera.0),
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute, left: Val::Percent(50.0), top: Val::Percent(50.0),
            width: Val::Px(3.0), height: Val::Px(3.0), ..default()
        },
        BackgroundColor(Color::srgb(0.92, 0.91, 0.86)),
        UiTargetCamera(camera.0),
    ));
    commands.spawn(label(
        "STEEL LAB / COMBAT FRAMEWORK",
        22.0,
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(24.0), ..default() },
        Color::srgb(0.827, 0.737, 0.471),
    )).insert(UiTargetCamera(camera.0));
    commands.spawn((
        LabStatus,
        UiTargetCamera(camera.0),
        label(
            "TEST RANGE  /  LOCAL SESSION",
            14.0,
            Node { position_type: PositionType::Absolute, top: Val::Px(40.0), left: Val::Px(24.0), ..default() },
            Color::srgb(0.655, 0.765, 0.804),
        ),
    ));
    commands.spawn((ControlLegend, UiTargetCamera(camera.0), label(
        "Loading saved controls\nTracers: F7    Parry geometry: F8    Combat tools: F9",
        13.0,
        Node { position_type: PositionType::Absolute, bottom: Val::Px(16.0), left: Val::Px(24.0), ..default() },
        Color::srgb(0.922, 0.914, 0.859),
    )));
    commands.spawn((
        VitalsPanel,
        UiTargetCamera(camera.0),
        Node {
            position_type: PositionType::Absolute, bottom: Val::Px(92.0), right: Val::Px(24.0),
            width: Val::Px(310.0), padding: UiRect::all(Val::Px(14.0)),
            flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), ..default()
        },
        BackgroundColor(Color::srgba(0.035, 0.065, 0.085, 0.94)),
        Visibility::Hidden,
    )).with_children(|panel| {
        for (kind, name, color) in [
            (Vital::Health, "HEALTH", Color::srgb(0.46, 0.78, 0.79)),
            (Vital::Stamina, "STAMINA", Color::srgb(0.78, 0.66, 0.34)),
        ] {
            panel.spawn((VitalLabel(kind), label(name, 13.0, Node::default(), Color::srgb(0.92, 0.91, 0.86))));
            panel.spawn((
                Node { width: Val::Percent(100.0), height: Val::Px(12.0), ..default() },
                BackgroundColor(Color::srgb(0.14, 0.20, 0.23)),
            )).with_children(|track| {
                track.spawn((
                    VitalFill(kind),
                    Node { width: Val::Percent(0.0), height: Val::Percent(100.0), ..default() },
                    BackgroundColor(color),
                ));
            });
        }
    });
}

/// Original widgets keep their gameplay event models, but never draw or consume input in this host.
fn original_hud_visibility(
    mut roots: Query<&mut Visibility, Or<(With<mh_ui::real::RtRoot>, With<mh_ui::UiRoot>)>>,
) {
    for mut root in &mut roots { *root = Visibility::Hidden; }
}

fn menu_input(
    mut menu: ResMut<LabMenu>, keys: Option<Res<ButtonInput<KeyCode>>>,
    mut buttons: Query<(&Interaction, &MenuAction, &mut BackgroundColor), (With<Button>, Changed<Interaction>)>,
    mut roots: Query<&mut Visibility, With<MenuRoot>>,
    mut player: ResMut<PlayerControl>, mut dev: ResMut<crate::devmenu::DevMenu>,
    mut exit: MessageWriter<bevy::app::AppExit>,
    mut editor: Option<ResMut<controls::Controls>>,
) {
    // The debug panel and controls editor own Escape while open.
    let editor_open = editor.as_deref().is_some_and(|c| c.open);
    if keys.as_ref().is_some_and(|k| k.just_pressed(KeyCode::Escape)) && !dev.open && !editor_open {
        menu.open = !menu.open;
    }
    if menu.open && !editor_open && keys.as_ref().is_some_and(|k| k.just_pressed(KeyCode::Enter)) { menu.open = false; }
    for (interaction, action, mut color) in &mut buttons {
        *color = BackgroundColor(match interaction {
            Interaction::Hovered | Interaction::Pressed => Color::srgb(0.20, 0.34, 0.38),
            Interaction::None => Color::srgb(0.09, 0.17, 0.21),
        });
        if !menu.open || editor_open || *interaction != Interaction::Pressed { continue; }
        match action {
            MenuAction::Resume => menu.open = false,
            MenuAction::Perspective => { player.third_person = !player.third_person; menu.open = false; }
            MenuAction::Controls => { if let Some(c) = editor.as_mut() { c.show(); } }
            MenuAction::Tools => { dev.open = true; menu.open = false; }
            MenuAction::Quit => { exit.write(bevy::app::AppExit::Success); }
        }
    }
    let editor_open = editor.as_deref().is_some_and(|c| c.open);
    for mut v in &mut roots { *v = if menu.open && !editor_open { Visibility::Inherited } else { Visibility::Hidden }; }
}

fn controls(player: Option<&PlayerControl>) -> String {
    let Some(player) = player else { return "Loading saved controls".into() };
    let key = |key: &crate::input::Key| match key {
        crate::input::Key::K(k) => format!("{k:?}").trim_start_matches("Key").to_string(),
        crate::input::Key::M(m) => format!("Mouse {m:?}"),
        crate::input::Key::WheelUp => "Wheel up".into(),
        crate::input::Key::WheelDown => "Wheel down".into(),
    };
    let bindings: Vec<_> = [("Strike", &["Strike", "Right Strike", "Left Strike"][..]),
        ("Stab", &["Stab", "Right Stab", "Left Stab"][..]), ("Parry", &["Parry"][..]),
        ("Kick", &["Kick"][..]), ("Feint", &["Feint"][..]), ("Grip", &["Weapon Mode / Reload"][..])].into_iter().map(|(label, actions)| {
            let mut keys: Vec<_> = actions.iter().flat_map(|action| player.actions.get(*action)).flatten().map(key).collect();
            keys.sort(); keys.dedup();
            format!("{label}: {}", if keys.is_empty() { "unbound".into() } else { keys.join(" / ") })
        }).collect();
    format!("{}\nTracers: F7    Parry geometry: F8    Combat tools: F9", bindings.join("    "))
}

fn update(
    sim: NonSend<Sim>, player: Option<Res<PlayerControl>>,
    menu: Res<LabMenu>, level: Res<crate::level::LevelState>,
    mut text: Query<(&mut Text, Has<LabStatus>), (Or<(With<LabStatus>, With<ControlLegend>)>, Without<VitalLabel>)>,
    mut labels: Query<(&VitalLabel, &mut Text), (Without<LabStatus>, Without<ControlLegend>)>,
    mut fills: Query<(&VitalFill, &mut Node)>,
    mut panels: Query<&mut Visibility, With<VitalsPanel>>,
) {
    let fighters = sim.0.fighters();
    let local_id = player.as_ref().map(|p| p.id);
    let status = match local_id.and_then(|id| fighters.iter().find(|f| f.id == id)) {
        Some(p) => format!(
            "{}  /  {}",
            if level.map == "TestLevel" { "TEST RANGE" } else { level.map.rsplit('/').next().unwrap_or("CUSTOM RANGE") },
            if p.health <= 0.0 { "DOWN" } else if p.state.contains("Attack") { "ATTACK" } else if p.state.contains("Parry") { "GUARD" } else { "READY" }
        ),
        None => "Waiting for the combat world".into(),
    };
    let legend = controls(player.as_deref());
    for (mut t, is_status) in &mut text {
        let next = if is_status { &status } else { &legend };
        if t.0 != *next { t.0.clone_from(next); }
    }
    // Read each fighter's live stat bounds, including modifications. The UI never
    // changes regeneration, costs, damage, or timings and never assumes a 100 cap.
    let local = local_id.and_then(|id| sim.0.combat().zip(sim.0.fighter_index(id)))
        .and_then(|(world, fi)| world.fighters.get(fi));
    let show = local.is_some() && !menu.open;
    for mut visible in &mut panels { *visible = if show { Visibility::Inherited } else { Visibility::Hidden }; }
    let Some(fighter) = local else { return };
    let values = |kind| match kind {
        Vital::Health => ("HEALTH", fighter.health, &fighter.health_stat),
        Vital::Stamina => ("STAMINA", fighter.stamina, &fighter.stamina_stat),
    };
    for (label, mut text) in &mut labels {
        let (name, value, stat) = values(label.0);
        let next = format!("{name}   {value} / {}", stat.max_value);
        if text.0 != next { text.0 = next; }
    }
    for (fill, mut node) in &mut fills {
        let (_, value, stat) = values(fill.0);
        let span = stat.max_value as f64 - stat.min_value as f64;
        let fraction = if span > 0.0 {
            ((value as f64 - stat.min_value as f64) / span).clamp(0.0, 1.0)
        } else { 0.0 };
        node.width = Val::Percent((fraction * 100.0) as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn menu_app() -> App {
        let mut app = App::new();
        app.init_resource::<LabMenu>().init_resource::<crate::devmenu::DevMenu>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(PlayerControl::new(crate::input::AngK::from_spec(None)))
            .add_message::<bevy::app::AppExit>().add_systems(Update, menu_input);
        app.world_mut().spawn((MenuRoot, Visibility::Hidden));
        app
    }
    #[test]
    fn escape_and_return_restore_framework_input_without_original_menu() {
        let mut app = menu_app();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
        app.update();
        assert!(app.world().resource::<LabMenu>().open);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Enter);
        app.update();
        assert!(!app.world().resource::<LabMenu>().open);
        let w = app.world_mut();
        let mut roots = w.query_filtered::<&Visibility, With<MenuRoot>>();
        assert!(roots.iter(w).all(|v| *v == Visibility::Hidden));
    }
    #[test]
    fn development_panel_keeps_ownership_of_escape() {
        let mut app = menu_app();
        app.world_mut().resource_mut::<crate::devmenu::DevMenu>().open = true;
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
        app.update();
        assert!(!app.world().resource::<LabMenu>().open);
    }
    #[test]
    fn input_import_never_changes_existing_framework_or_original_preferences() {
        let folder = std::env::temp_dir().join(format!("steel-input-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let source = folder.join("original.ini"); let target = folder.join("framework.ini");
        std::fs::write(&source, b"MouseXSensitivity=0.000350").unwrap();
        copy_input_if_missing(&source, &target).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), std::fs::read(&target).unwrap());
        std::fs::write(&target, b"my existing preferences").unwrap();
        copy_input_if_missing(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"my existing preferences");
        assert_eq!(std::fs::read(&source).unwrap(), b"MouseXSensitivity=0.000350");
        for path in [source, target] { std::fs::remove_file(path).unwrap(); }
        std::fs::remove_dir(folder).unwrap();
    }
}

//! Original framework presentation. Reads combat state and never drives its clocks.
use bevy::prelude::*;
use crate::sim::Sim;
use crate::input::PlayerControl;

#[derive(Component)]
struct LabStatus;

#[derive(Component)]
struct ControlLegend;

#[derive(Component)]
struct VitalsPanel;

#[derive(Component, Clone, Copy)]
enum Vital { Health, Stamina }

#[derive(Component)]
struct VitalFill(Vital);

#[derive(Component)]
struct VitalLabel(Vital);

pub struct FrameworkUiPlugin;
impl Plugin for FrameworkUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
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
        "Mordhau Combat Framework",
        22.0,
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(24.0), ..default() },
        Color::srgb(0.827, 0.737, 0.471),
    )).insert(UiTargetCamera(camera.0));
    commands.spawn((
        LabStatus,
        UiTargetCamera(camera.0),
        label(
            "Loading combat lab",
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
            position_type: PositionType::Absolute, bottom: Val::Px(80.0), left: Val::Px(24.0),
            width: Val::Px(310.0), padding: UiRect::all(Val::Px(14.0)),
            flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), ..default()
        },
        BackgroundColor(Color::srgba(0.035, 0.065, 0.085, 0.94)),
        Visibility::Hidden,
    )).with_children(|panel| {
        for (kind, name, color) in [
            (Vital::Health, "HEALTH", Color::srgb(0.72, 0.24, 0.21)),
            (Vital::Stamina, "STAMINA", Color::srgb(0.78, 0.66, 0.34)),
        ] {
            panel.spawn((VitalLabel(kind), label(name, 13.0, Node::default(), Color::srgb(0.92, 0.91, 0.86))));
            panel.spawn((
                Node { width: Val::Percent(100.0), height: Val::Px(9.0), ..default() },
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

/// Keep original menu behavior available while substituting our match presentation.
/// Hiding the draw roots does not remove the VM or alter HUD/gameplay event processing.
fn original_hud_visibility(
    player: Option<Res<PlayerControl>>, screen: Res<mh_ui::Screen>,
    real: Option<NonSend<mh_ui::real::Rt>>,
    mut roots: Query<&mut Visibility, Or<(With<mh_ui::real::RtRoot>, With<mh_ui::UiRoot>)>>,
) {
    let menu = menu_visible(player.as_deref(), &screen, real.as_deref());
    for mut root in &mut roots {
        *root = if menu { Visibility::Inherited } else { Visibility::Hidden };
    }
}

fn menu_visible(player: Option<&PlayerControl>, screen: &mh_ui::Screen, real: Option<&mh_ui::real::Rt>) -> bool {
    *screen == mh_ui::Screen::MainMenu
        || player.is_some_and(|p| p.menu_open)
        || real.is_some_and(|r| r.ui.main_menu_visible())
}

fn controls(player: Option<&PlayerControl>) -> String {
    let Some(player) = player else { return "Loading saved controls".into() };
    let key = |key: &crate::input::Key| match key {
        crate::input::Key::K(k) => format!("{k:?}").trim_start_matches("Key").to_string(),
        crate::input::Key::M(m) => format!("Mouse {m:?}"),
        crate::input::Key::WheelUp => "Wheel up".into(),
        crate::input::Key::WheelDown => "Wheel down".into(),
    };
    let bindings: Vec<_> = [("Strike", "Strike"), ("Stab", "Stab"), ("Parry", "Parry"),
        ("Feint", "Feint"), ("Grip", "Weapon Mode / Reload")].into_iter().map(|(label, action)| {
            let keys: Vec<_> = player.actions.get(action).into_iter().flatten().map(key).collect();
            format!("{label}: {}", if keys.is_empty() { "unbound".into() } else { keys.join(" / ") })
        }).collect();
    format!("{}\nTracers: F7    Parry geometry: F8    Combat tools: F9", bindings.join("    "))
}

fn update(
    sim: NonSend<Sim>, player: Option<Res<PlayerControl>>,
    screen: Res<mh_ui::Screen>, real: Option<NonSend<mh_ui::real::Rt>>,
    mut text: Query<(&mut Text, Has<LabStatus>), (Or<(With<LabStatus>, With<ControlLegend>)>, Without<VitalLabel>)>,
    mut labels: Query<(&VitalLabel, &mut Text), (Without<LabStatus>, Without<ControlLegend>)>,
    mut fills: Query<(&VitalFill, &mut Node)>,
    mut panels: Query<&mut Visibility, With<VitalsPanel>>,
) {
    let fighters = sim.0.fighters();
    let local_id = player.as_ref().map(|p| p.id);
    let status = match local_id.and_then(|id| fighters.iter().find(|f| f.id == id)) {
        Some(p) => format!(
            "Pawn {} | {} | Health {:.0} | Stamina {} | {} simulation",
            p.id, p.state, p.health, p.stamina, sim.0.name()
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
    let show = local.is_some() && !menu_visible(player.as_deref(), &screen, real.as_deref());
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

//! Original framework presentation. Reads combat state and never drives its clocks.
use bevy::prelude::*;
use crate::sim::Sim;
use crate::input::PlayerControl;

#[derive(Component)]
struct LabStatus;

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
        app.add_systems(Startup, setup).add_systems(Update, update);
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

fn setup(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0), left: Val::Px(0.0),
            width: Val::Percent(100.0), height: Val::Px(64.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.078, 0.165, 0.224, 0.85)),
    ));
    commands.spawn(label(
        "Mordhau Combat Framework",
        22.0,
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(24.0), ..default() },
        Color::srgb(0.827, 0.737, 0.471),
    ));
    commands.spawn((
        LabStatus,
        label(
            "Loading combat lab",
            14.0,
            Node { position_type: PositionType::Absolute, top: Val::Px(40.0), left: Val::Px(24.0), ..default() },
            Color::srgb(0.655, 0.765, 0.804),
        ),
    ));
    commands.spawn(label(
        "Strike: left click    Stab: wheel up    Parry: right click    Feint: Q    Grip: R\nView: P    Tracers: F7    Parry geometry: F8    Combat tools: F9",
        13.0,
        Node { position_type: PositionType::Absolute, bottom: Val::Px(16.0), left: Val::Px(24.0), ..default() },
        Color::srgb(0.922, 0.914, 0.859),
    ));
    commands.spawn((
        VitalsPanel,
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

fn update(
    sim: NonSend<Sim>, player: Option<Res<PlayerControl>>,
    mut text: Query<&mut Text, (With<LabStatus>, Without<VitalLabel>)>,
    mut labels: Query<(&VitalLabel, &mut Text), Without<LabStatus>>,
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
    for mut t in &mut text {
        if t.0 != status { t.0.clone_from(&status); }
    }
    // Read each fighter's live stat bounds, including modifications. The UI never
    // changes regeneration, costs, damage, or timings and never assumes a 100 cap.
    let local = local_id.and_then(|id| sim.0.combat().zip(sim.0.fighter_index(id)))
        .and_then(|(world, fi)| world.fighters.get(fi));
    let show = local.is_some() && !player.as_ref().is_some_and(|p| p.menu_open);
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

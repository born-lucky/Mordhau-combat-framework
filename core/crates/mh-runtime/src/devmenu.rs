//! devmenu.rs - the Development menu (user 2026-10-07 11:30 / 14:09: "a dev menu in the settings for enabling the
//! console commands in mordhau so we can detect bugs"). The shipped game has no such menu; its console variables and
//! exec functions (docs/CVARS.tsv, docs/EXECS.tsv) are what the rows switch:
//!   m.DrawTracers / m.DrawTracersStayTime (UMordhauGameUserSettings DrawTracers; weapon.rs tracers),
//!   m.Gore (UMordhauGameUserSettings::ShouldShowBlood rva=0x15a8510), Slomo (UCheatManager::Slomo: the world's time
//!   dilation), AddBots (AMordhauPlayerController::AddBots exec, docs/EXECS.tsv), Suicide (AMordhauCharacter::Suicide
//!   rva=0x14a3860, sim_mh.rs), ToggleThirdPerson (the view), and two runtime-only views: the fly camera (F1) and the
//!   capsule-shadow proxies (fighter.rs SHADOW_LAYER) drawn as solid capsules.
//! Runtime glue: F9 opens / closes it (Escape closes); it is not a port of a game widget. The settings it changes are
//! written to the rewrite's own GameUserSettings.ini (mh_ui::settings::config_dir), the same keys the real Settings
//! screens write, so the game's own Settings > Draw Tracers toggle and this menu agree.

use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct DevMenu {
    pub open: bool,
    pub slomo: bool,
    pub capsules: bool,
    pub pawn_debug: bool,
    pub visualize_block_collider: bool,
    pending: Vec<Action>,
    built: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Tracers,
    PawnDebug,
    BlockCollider,
    TimingExport,
    TimingReload,
    TimingReset,
    Gore,
    Slomo,
    AddBot,
    Kill,
    ThirdPerson,
    FlyCamera,
    Capsules,
    Interp,
    Close,
}

const ROWS: &[(Action, &str)] = &[
    (Action::Tracers, "Draw tracers (m.DrawTracers)"),
    (Action::PawnDebug, "Pawn debug (m.ShowPawnDebug)"),
    (Action::BlockCollider, "Parry boxes (m.VisualizeBlockCollider)"),
    (Action::TimingExport, "Export original combat timing catalog"),
    (Action::TimingReload, "Reload combat timing edits"),
    (Action::TimingReset, "Reset combat timings to stock (session)"),
    (Action::Gore, "Gore (m.Gore)"),
    (Action::Slomo, "Slow motion 0.25 (Slomo)"),
    (Action::AddBot, "Add a bot (AddBots 1)"),
    (Action::Kill, "Kill self (Suicide)"),
    (Action::ThirdPerson, "Third person (ToggleThirdPerson)"),
    (Action::FlyCamera, "Fly camera (F1)"),
    (Action::Capsules, "Show body capsules"),
    (Action::Interp, "Draw the last fixed step only"),
    (Action::Close, "Close (` / F9 / Esc)"),
];

#[derive(Component)]
struct Root;
#[derive(Component)]
struct Row(Action);
#[derive(Component)]
struct Label(Action);
#[derive(Component)]
struct PawnText;

pub struct DevMenuPlugin;

impl Plugin for DevMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DevMenu>()
            .init_resource::<ParryBoxDebug>()
            .add_systems(Update, toggle.before(crate::input::player_input).after(crate::framework_ui::LabInput))
            .add_systems(Update, (build, clicks, labels).chain().after(toggle))
            .add_systems(Update, apply.after(labels))
            .add_systems(Update, pawn_debug.after(crate::sim::tick_sim_frame).after(apply))
            .add_systems(Update, draw_parry_boxes.after(crate::sim::tick_sim_frame).after(apply));
    }
}

fn toggle(keys: Option<Res<ButtonInput<KeyCode>>>, mut dm: ResMut<DevMenu>, mut roots: Query<&mut Visibility, With<Root>>) {
    let Some(k) = keys else { return };
    if k.just_pressed(KeyCode::F9) || k.just_pressed(KeyCode::Backquote) || (dm.open && k.just_pressed(KeyCode::Escape)) {
        dm.open = !dm.open;
    }
    if k.just_pressed(KeyCode::F7) { dm.pending.push(Action::Tracers); }
    if k.just_pressed(KeyCode::F8) { dm.pending.push(Action::PawnDebug); }
    for mut v in roots.iter_mut() {
        let want = if dm.open { Visibility::Visible } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
}

fn build(mut commands: Commands, mut dm: ResMut<DevMenu>, windows: Query<(), With<Window>>) {
    if dm.built || windows.is_empty() {
        return;
    }
    dm.built = true;
    commands.spawn((Text::new("` / F9: Development   F7: Tracers   F8: Pawn debug"),
        TextFont { font_size: bevy::text::FontSize::Px(14.0), ..default() }, TextColor(Color::WHITE),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(8.0), left: Val::Px(12.0), ..default() }, GlobalZIndex(51)));
    commands.spawn((PawnText, Text::new(""), TextFont { font_size: bevy::text::FontSize::Px(14.0), ..default() },
        TextColor(Color::srgb(1.0, 0.95, 0.5)), Visibility::Hidden, GlobalZIndex(51),
        Node { position_type: PositionType::Absolute, top: Val::Px(8.0), left: Val::Px(12.0), ..default() }));
    commands
        .spawn((
            Root,
            Name::new("DevMenu"),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(24.0),
                top: Val::Px(80.0),
                width: Val::Px(360.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.92)),
            Visibility::Hidden,
            GlobalZIndex(50),
        ))
        .with_children(|p| {
            p.spawn((Text::new("Development"), TextFont { font_size: bevy::text::FontSize::Px(18.0), ..default() }, TextColor(Color::srgb(0.95, 0.85, 0.5))));
            for (a, _) in ROWS {
                p.spawn((
                    Button,
                    Row(*a),
                    Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), ..default() },
                    BackgroundColor(Color::srgba(0.18, 0.18, 0.2, 0.95)),
                ))
                .with_children(|b| {
                    b.spawn((Text::new(""), Label(*a), TextFont { font_size: bevy::text::FontSize::Px(14.0), ..default() }, TextColor(Color::WHITE)));
                });
            }
        });
}

fn clicks(mut dm: ResMut<DevMenu>, mut q: Query<(&Interaction, &Row, &mut BackgroundColor), Changed<Interaction>>) {
    for (i, r, mut bg) in q.iter_mut() {
        match i {
            Interaction::Pressed => {
                dm.pending.push(r.0);
                bg.0 = Color::srgba(0.4, 0.35, 0.15, 0.95);
            }
            Interaction::Hovered => bg.0 = Color::srgba(0.28, 0.28, 0.32, 0.95),
            Interaction::None => bg.0 = Color::srgba(0.18, 0.18, 0.2, 0.95),
        }
    }
}

fn labels(
    dm: Res<DevMenu>,
    us: Option<Res<crate::usersettings::UserSettings>>,
    pc: Option<Res<crate::input::PlayerControl>>,
    mode: Option<Res<crate::camera::CamMode>>,
    frames: Option<Res<crate::sim::SimFrames>>,
    timings: Option<Res<crate::combat_timings::Controls>>,
    mut q: Query<(&Label, &mut Text)>,
) {
    if !dm.open {
        return;
    }
    let on = |b: bool| if b { "ON" } else { "off" };
    for (l, mut t) in q.iter_mut() {
        let name = ROWS.iter().find(|r| r.0 == l.0).map(|r| r.1).unwrap_or("");
        let state = match l.0 {
            Action::Tracers => format!("{} (stay {:.1} s)", on(us.as_ref().is_some_and(|u| u.draw_tracers != 0)), us.as_ref().map(|u| u.draw_tracers_stay_time).unwrap_or(2.0)),
            Action::PawnDebug => on(dm.pawn_debug).into(),
            Action::BlockCollider => on(dm.visualize_block_collider).into(),
            Action::TimingReload => timings.as_ref().map(|t|t.message.clone()).unwrap_or_default(),
            Action::Gore => us.as_ref().map(|u| u.gore.to_string()).unwrap_or_default(),
            Action::Slomo => on(dm.slomo).into(),
            Action::ThirdPerson => on(pc.as_ref().is_some_and(|p| p.third_person)).into(),
            Action::FlyCamera => on(mode.as_ref().is_some_and(|m| **m == crate::camera::CamMode::Fly)).into(),
            Action::Capsules => on(dm.capsules).into(),
            Action::Interp => on(frames.as_ref().is_some_and(|f| !f.enabled)).into(),
            _ => String::new(),
        };
        let s = if state.is_empty() { name.to_string() } else { format!("{name}: {state}") };
        if t.0 != s {
            t.0 = s;
        }
    }
}

/// the queued clicks, applied with full world access (the sim is a non-send resource)
pub(crate) fn apply(world: &mut World) {
    let pending: Vec<Action> = std::mem::take(&mut world.resource_mut::<DevMenu>().pending);
    for a in pending {
        match a {
            Action::PawnDebug => {
                let mut dm = world.resource_mut::<DevMenu>();
                dm.pawn_debug = !dm.pawn_debug;
            }
            Action::BlockCollider => {
                let mut dm = world.resource_mut::<DevMenu>();
                dm.visualize_block_collider = !dm.visualize_block_collider;
            }
            Action::TimingExport => { crate::combat_timings::command(world,"export"); }
            Action::TimingReload => { crate::combat_timings::command(world,"reload"); }
            Action::TimingReset => { crate::combat_timings::command(world,"reset"); }
            Action::Tracers => {
                let v = world.get_resource::<crate::usersettings::UserSettings>().map(|u| if u.draw_tracers != 0 { 0 } else { 1 }).unwrap_or(1);
                if let Some(mut u) = world.get_resource_mut::<crate::usersettings::UserSettings>() {
                    u.draw_tracers = v;
                }
                set_ini_key("DrawTracers", &v.to_string());
            }
            Action::Gore => {
                let v = world.get_resource::<crate::usersettings::UserSettings>().map(|u| if u.gore != 0 { 0 } else { 2 }).unwrap_or(2);
                if let Some(mut u) = world.get_resource_mut::<crate::usersettings::UserSettings>() {
                    u.gore = v;
                }
                set_ini_key("Gore", &v.to_string());
            }
            Action::Slomo => {
                let on = {
                    let mut dm = world.resource_mut::<DevMenu>();
                    dm.slomo = !dm.slomo;
                    dm.slomo
                };
                if let Some(mut t) = world.get_resource_mut::<Time<Virtual>>() {
                    t.set_relative_speed(if on { 0.25 } else { 1.0 });
                }
            }
            Action::AddBot => {
                if let Some(mut q) = world.get_resource_mut::<crate::sim::BotQueue>() {
                    q.0 += 1;
                }
            }
            Action::Kill => {
                let id = world.get_resource::<crate::input::PlayerControl>().map(|p| p.id);
                if let (Some(id), Some(mut sim)) = (id, world.get_non_send_resource_mut::<crate::sim::Sim>()) {
                    sim.0.suicide(id);
                }
            }
            Action::ThirdPerson => {
                if let Some(mut pc) = world.get_resource_mut::<crate::input::PlayerControl>() {
                    pc.third_person = !pc.third_person;
                }
                if let Some(mut m) = world.get_resource_mut::<crate::camera::CamMode>() {
                    *m = crate::camera::CamMode::Player;
                }
            }
            Action::FlyCamera => {
                if let Some(mut m) = world.get_resource_mut::<crate::camera::CamMode>() {
                    *m = if *m == crate::camera::CamMode::Fly { crate::camera::CamMode::Player } else { crate::camera::CamMode::Fly };
                }
            }
            Action::Capsules => {
                let on = {
                    let mut dm = world.resource_mut::<DevMenu>();
                    dm.capsules = !dm.capsules;
                    dm.capsules
                };
                let cams: Vec<Entity> = world.query_filtered::<Entity, With<Camera3d>>().iter(world).collect();
                for e in cams {
                    if on {
                        world.entity_mut(e).insert(bevy::camera::visibility::RenderLayers::from_layers(&[0, crate::fighter::SHADOW_LAYER]));
                    } else {
                        world.entity_mut(e).insert(bevy::camera::visibility::RenderLayers::layer(0));
                    }
                }
            }
            Action::Interp => {
                if let Some(mut f) = world.get_resource_mut::<crate::sim::SimFrames>() {
                    f.enabled = !f.enabled;
                }
            }
            Action::Close => world.resource_mut::<DevMenu>().open = false,
        }
    }
}

/// Host display of current pawn movement/motion values (m.ShowPawnDebug, PlayerTick 0x15e8840).
/// Uses current simulation values, so the diagnostics don't depend on an Unreal console.
fn pawn_debug(dm: Res<DevMenu>, sim: NonSend<crate::sim::Sim>, pc: Option<Res<crate::input::PlayerControl>>,
    tracers: Res<crate::weapon::Tracers>, mut text: Query<(&mut Text, &mut Visibility), With<PawnText>>) {
    let pawn = pc.as_ref().and_then(|p| sim.0.fighters().into_iter().find(|v| v.id == p.id));
    for (mut t, mut vis) in &mut text {
        *vis = if dm.pawn_debug { Visibility::Visible } else { Visibility::Hidden };
        if !dm.pawn_debug { continue; }
        t.0 = match &pawn {
            Some(v) => format!("m.ShowPawnDebug | {} tick {} | pawn {}\nMotion: {} | health {:.0} | stamina {}\nVelocity: {:.1}, {:.1}, {:.1} cm/s\nPosition: {:.1}, {:.1}, {:.1} cm | yaw {:.1} | pitch {:.1}\nFalling {} | crouched {} | {}\nActual tracer segments: {} total / {} this frame",
                sim.0.name(), sim.0.ticks(), v.id, v.state, v.health, v.stamina,
                v.vel[0], v.vel[1], v.vel[2], v.loc[0], v.loc[1], v.loc[2], v.yaw, v.look_up,
                v.falling, v.crouched, if pc.as_ref().is_some_and(|p| p.third_person) { "third person" } else { "first person" },
                tracers.sampled, tracers.last.len()),
            None => "m.ShowPawnDebug | No possessed pawn".into(),
        };
    }
}

/// Original LODTick (0x154c390) draws the enabled actual BlockCollider in green and its forward test volume in red.
/// PawnDebug also exposes these boxes as a rewrite convenience; native m.VisualizeBlockCollider remains separate.
#[derive(Resource, Default)]
pub(crate) struct ParryBoxDebug {
    enabled: bool,
    tick: u64,
    boxes: Vec<ParryDebugBox>,
    green: Vec<[f32; 3]>,
    red: Vec<[f32; 3]>,
    mesh_updated: bool,
}

#[derive(serde::Serialize)]
struct ParryDebugBox {
    fighter: u32,
    center_ue_cm: [f32; 3],
    rotation_ue_xyzw: [f32; 4],
    half_extent_ue_cm: [f32; 3],
    forward_distance_ue_cm: [f32; 2],
    forward_center_ue_cm: Option<[f32; 3]>,
    forward_half_extent_ue_cm: Option<[f32; 3]>,
}

impl ParryBoxDebug {
    pub(crate) fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"enabled": self.enabled, "tick": self.tick, "mesh_updated": self.mesh_updated,
            "boxes": self.boxes, "green_vertices_bevy_m": self.green, "red_vertices_bevy_m": self.red})
    }
}

fn parry_debug_enabled(dm: &DevMenu) -> bool {
    dm.pawn_debug || dm.visualize_block_collider
}

fn box_edges(points: &mut Vec<[f32; 3]>, transform: mordhau_core::ue::FTransform, half: mordhau_core::ue::FVector) {
    use mordhau_core::ue::FVector;
    let corners: [[f32; 3]; 8] = std::array::from_fn(|i| {
        let sign = |axis: usize| if i & (1usize << axis) == 0 { -1.0 } else { 1.0 };
        let p = transform.apply(FVector::new(sign(0) * half.x, sign(1) * half.y, sign(2) * half.z));
        // The runtime's existing UE cm -> Bevy metre basis; do not rebuild a camera offset.
        [p.x * 0.01, p.z * 0.01, p.y * 0.01]
    });
    for i in 0..8 {
        for axis in 0..3 {
            if i & (1usize << axis) == 0 {
                points.extend([corners[i], corners[i | (1usize << axis)]]);
            }
        }
    }
}

fn collect_parry_boxes<'a>(enabled: bool, fighters: impl IntoIterator<Item = (u32, bool, Option<&'a mordhau_core::combat::world::FighterGeom>, (f32, f32))>) -> ParryBoxDebug {
    use mordhau_core::ue::{FTransform, FVector};
    let mut out = ParryBoxDebug { enabled, ..Default::default() };
    if !enabled { return out; }
    for (fighter, active, geometry, forward) in fighters {
        let Some(g) = geometry.filter(|_| active) else { continue };
        let (actual, half) = (g.block_collider, g.block_extent);
        box_edges(&mut out.green, actual, half);
        let red = if forward.0 > 0.0 && forward.1 > 0.0 {
            // Same local bounds as core TestForwardParry (0x166dbd0) and original RED debug box.
            let min = FVector::new(-half.x, -forward.1, -half.z);
            let max = FVector::new(forward.0 - half.x, forward.1, half.z);
            let center = actual.apply((min + max).scale(0.5));
            let extent = (max - min).scale(0.5);
            box_edges(&mut out.red, FTransform::new(actual.rot, center), extent);
            Some((center, extent))
        } else { None };
        let v = |p: FVector| [p.x, p.y, p.z];
        out.boxes.push(ParryDebugBox { fighter, center_ue_cm: v(actual.loc),
            rotation_ue_xyzw: [actual.rot.x, actual.rot.y, actual.rot.z, actual.rot.w], half_extent_ue_cm: v(half),
            forward_distance_ue_cm: [forward.0, forward.1], forward_center_ue_cm: red.map(|r| v(r.0)),
            forward_half_extent_ue_cm: red.map(|r| v(r.1)) });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_parry_boxes(mut commands: Commands, dm: Res<DevMenu>, sim: NonSend<crate::sim::Sim>,
    mut debug: ResMut<ParryBoxDebug>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>,
    mut entities: Local<Option<[(Entity, Handle<Mesh>); 2]>>) {
    *debug = if let Some(w) = sim.0.combat() {
        let mut geometry = collect_parry_boxes(parry_debug_enabled(&dm), w.fighters.iter().map(|f| {
            let d = f.character.block_collider_forward_parry_distance;
            (f.id, f.block_collider_enabled, f.geom.as_ref(), (d.xf() as f32, d.yf() as f32))
        }));
        geometry.tick = w.tick_n as u64;
        geometry
    } else { ParryBoxDebug { enabled: parry_debug_enabled(&dm), ..Default::default() } };
    if entities.is_none() {
        // Prepare the ordinary LineList/material pipelines before the first collider activation.
        // The native geometry/gates remain unchanged (LODTick154c390); this readiness adaptation
        // is judged by the identical cold-on/off capture, not by populated CPU vertices alone.
        let spawned = [("ParryBlockColliderDebug", Color::srgb(0.0, 1.0, 0.0)), ("ForwardParryDebug", Color::srgb(1.0, 0.0, 0.0))]
            .map(|(name, color)| {
                let mesh = meshes.add(Mesh::new(bevy::mesh::PrimitiveTopology::LineList, bevy::asset::RenderAssetUsages::default())
                    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, -1.0e4, 0.0]; 2]));
                let material = materials.add(StandardMaterial { base_color: color, unlit: true, ..default() });
                let entity = commands.spawn((Name::new(name), Mesh3d(mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY,
                    bevy::light::NotShadowCaster, bevy::light::NotShadowReceiver,
                    bevy::camera::visibility::NoFrustumCulling, Visibility::Visible)).id();
                (entity, mesh)
            });
        *entities = Some(spawned);
    }
    let vertices = [&debug.green, &debug.red];
    let mut updated = true;
    for ((entity, handle), points) in entities.as_ref().unwrap().iter().zip(vertices) {
        // Keep the prepared pipeline queued while off; the degenerate line below the level has
        // no visible volume. Hiding the entity here would defeat first-activation preparation.
        commands.entity(*entity).insert(Visibility::Visible);
        if let Some(mut mesh) = meshes.get_mut(handle) {
            // Replace all positions when either gate closes so no previous box can remain.
            let positions = if points.is_empty() { vec![[0.0, -1.0e4, 0.0]; 2] } else { points.clone() };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        } else { updated = false; }
    }
    debug.mesh_updated = updated;
}

/// The script harness and development UI use the same live toggles. CVar names match docs/CVARS.tsv.
pub fn command(world: &mut World, input: &str) -> bool {
    let mut args = input.split_whitespace();
    let Some(name) = args.next() else { return false };
    match name.to_ascii_lowercase().as_str() {
        "rw.combattimings" => return crate::combat_timings::command(world,args.next().unwrap_or("reload")),
        // Harness access to the same queued F9 action; deliberately a rewrite command,
        // not an assertion about UCheatManager's unported argument/clamping behavior.
        "rw.slomotoggle" => world.resource_mut::<DevMenu>().pending.push(Action::Slomo),
        "m.showpawndebug" => {
            let mut dm = world.resource_mut::<DevMenu>();
            dm.pawn_debug = args.next().and_then(|v| v.parse::<i32>().ok()).map(|v| v != 0).unwrap_or(!dm.pawn_debug);
        }
        "m.visualizeblockcollider" => {
            let mut dm = world.resource_mut::<DevMenu>();
            dm.visualize_block_collider = args.next().and_then(|v| v.parse::<i32>().ok()).map(|v| v != 0).unwrap_or(!dm.visualize_block_collider);
        }
        "m.drawtracers" | "drawtracers" => {
            if let Some(mut u) = world.get_resource_mut::<crate::usersettings::UserSettings>() {
                u.draw_tracers = args.next().and_then(|v| v.parse::<i32>().ok()).unwrap_or(if u.draw_tracers == 0 { 1 } else { 0 });
            }
        }
        "m.drawtracersstaytime" => {
            if let Some(v) = args.next().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite() && *v >= 0.0) {
                if let Some(mut u) = world.get_resource_mut::<crate::usersettings::UserSettings>() { u.draw_tracers_stay_time = v; }
            }
        }
        _ => return false,
    }
    true
}

/// rewrite one `Key=Value` line of the rewrite's GameUserSettings.ini section (the real Settings screens write the same
/// file through mh_ui::settings; a missing file or section gets created with just this key)
fn set_ini_key(key: &str, value: &str) {
    let path = mh_ui::settings::config_dir().join("GameUserSettings.ini");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let section = mh_ui::settings::SECTION;
    let mut out: Vec<String> = Vec::new();
    let (mut in_section, mut done, mut seen_section) = (false, false, false);
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            if in_section && !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            in_section = l == section;
            seen_section |= in_section;
        } else if in_section && l.split('=').next().map(|k| k.trim()) == Some(key) {
            out.push(format!("{key}={value}"));
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !seen_section {
        out.push(section.to_string());
    }
    if !done {
        out.push(format!("{key}={value}"));
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&path, out.join("\n") + "\n");
}

#[cfg(test)]
mod parry_box_tests {
    use super::*;
    use mordhau_core::{combat::world::FighterGeom, ue::{FQuat, FTransform, FVector}};

    #[test]
    fn slomo_harness_command_uses_the_queued_menu_action() {
        let mut world = World::new();
        world.init_resource::<DevMenu>();
        world.insert_resource(Time::<Virtual>::default());
        assert!(command(&mut world,"rw.SlomoToggle"));
        assert_eq!(world.resource::<Time<Virtual>>().relative_speed(),1.0);
        apply(&mut world);
        assert_eq!(world.resource::<Time<Virtual>>().relative_speed(),0.25);
        assert!(world.resource::<DevMenu>().slomo);
        assert!(command(&mut world,"rw.SlomoToggle"));
        apply(&mut world);
        assert_eq!(world.resource::<Time<Virtual>>().relative_speed(),1.0);
        assert!(!world.resource::<DevMenu>().slomo);
    }

    fn actual() -> FighterGeom {
        // The shipped character half extents, with an intentionally rotated/translated actual collider.
        // Different camera values prove that drawing consumes collision geometry instead of rebuilding placement.
        FighterGeom { block_collider: FTransform::new(FQuat::from_rotator(0.0, 90.0, 0.0), FVector::new(100.0, 200.0, 300.0)),
            block_extent: FVector::new(65.0, 65.0, 105.0), camera_loc: FVector::new(-999.0, 888.0, 777.0),
            camera_rot: (-67.0, 32.0), ..Default::default() }
    }

    #[test]
    fn actual_rotated_collider_and_native_forward_bounds_produce_twelve_edges() {
        let g = actual();
        let drawn = collect_parry_boxes(true, [(7, true, Some(&g), (50.0, 10.0))]);
        assert_eq!(drawn.green.len(), 24);
        assert_eq!(drawn.red.len(), 24);
        assert_eq!(drawn.boxes.len(), 1);
        let b = &drawn.boxes[0];
        assert_eq!(b.fighter, 7);
        assert_eq!(b.center_ue_cm, [100.0, 200.0, 300.0]);
        assert_eq!(b.half_extent_ue_cm, [65.0, 65.0, 105.0]);
        for (value, expected) in drawn.green[0].iter().zip([1.65, 1.95, 1.35]) {
            assert!((*value - expected).abs() < 1e-6);
        }
        let center = b.forward_center_ue_cm.unwrap();
        for (value, expected) in center.iter().zip([100.0, 160.0, 300.0]) {
            assert!((*value - expected).abs() < 1e-4);
        }
        assert_eq!(b.forward_half_extent_ue_cm, Some([25.0, 10.0, 105.0]));
        let mut edges = std::collections::BTreeSet::new();
        for edge in drawn.green.chunks_exact(2) {
            let mut pair = [edge[0].map(f32::to_bits), edge[1].map(f32::to_bits)];
            pair.sort();
            assert!(edges.insert(pair), "each of the twelve edges must occur once");
            let length = Vec3::from_array(edge[0]).distance(Vec3::from_array(edge[1]));
            assert!((length - 1.3).abs() < 1e-6 || (length - 2.1).abs() < 1e-6);
        }
        assert_eq!(edges.len(), 12);
    }

    #[test]
    fn native_enable_and_forward_distance_gates_clear_stale_geometry() {
        let g = actual();
        let fighters = [(1, true, Some(&g), (50.0, 10.0)), (2, false, Some(&g), (50.0, 10.0)),
            (3, true, None, (50.0, 10.0)), (4, true, Some(&g), (0.0, 10.0)),
            (5, true, Some(&g), (50.0, -1.0)), (6, true, Some(&g), (50.0, 0.0))];
        let drawn = collect_parry_boxes(true, fighters);
        assert_eq!(drawn.boxes.iter().map(|b| b.fighter).collect::<Vec<_>>(), [1, 4, 5, 6]);
        assert_eq!(drawn.green.len(), 4 * 24);
        assert_eq!(drawn.red.len(), 24, "both original forward distances must be strictly positive");
        let off = collect_parry_boxes(false, fighters);
        assert!(!off.enabled && off.boxes.is_empty() && off.green.is_empty() && off.red.is_empty());
        let inactive = collect_parry_boxes(true, [(1, false, Some(&g), (50.0, 10.0))]);
        assert!(inactive.boxes.is_empty() && inactive.green.is_empty() && inactive.red.is_empty());
    }

    #[test]
    fn independent_commands_and_pawn_debug_convenience_preserve_the_or_gate() {
        let mut world = World::new();
        world.init_resource::<DevMenu>();
        assert!(command(&mut world, "m.VisualizeBlockCollider 1"));
        assert!(parry_debug_enabled(world.resource::<DevMenu>()));
        assert!(!world.resource::<DevMenu>().pawn_debug);
        assert!(command(&mut world, "m.ShowPawnDebug 1"));
        assert!(command(&mut world, "m.VisualizeBlockCollider 0"));
        assert!(parry_debug_enabled(world.resource::<DevMenu>()));
        assert!(command(&mut world, "m.ShowPawnDebug 0"));
        assert!(!parry_debug_enabled(world.resource::<DevMenu>()));
        world.resource_mut::<DevMenu>().pending.extend([Action::PawnDebug, Action::BlockCollider]);
        apply(&mut world);
        assert!(world.resource::<DevMenu>().pawn_debug && world.resource::<DevMenu>().visualize_block_collider);
        world.resource_mut::<DevMenu>().pending.push(Action::PawnDebug);
        apply(&mut world);
        assert!(parry_debug_enabled(world.resource::<DevMenu>()));
        assert!(command(&mut world, "m.VisualizeBlockCollider"));
        assert!(!parry_debug_enabled(world.resource::<DevMenu>()));
    }
}

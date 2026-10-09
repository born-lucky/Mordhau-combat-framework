//! Owned controls editor. Persist native input settings and action names, then use the existing keymap rebuild.
use bevy::prelude::*;
use std::collections::HashMap;

const SECTION: &str = "/Script/Mordhau.MordhauInput";
const ACTIONS: &[&str] = &["Strike", "Stab", "Parry", "Kick", "Feint", "Weapon Mode / Reload",
    "Right Strike", "Left Strike", "Right Upper Strike", "Left Upper Strike", "Right Lower Strike", "Left Lower Strike",
    "Right Stab", "Left Stab", "Flip Attack Side", "Shove", "Jump", "Sprint", "Crouch", "Use", "Drop"];
const FLAGS: &[(&str, &str)] = &[("MouseXIsFlipAttackSide", "Mouse selects attack side"),
    ("AngleAttackAfterPress", "Select angle after pressing attack"), ("AngleAttacksWithMovement", "Movement selects attack angle"),
    ("InverseAttackDirectionX", "Invert attack direction X"), ("InverseAttackDirectionY", "Invert attack direction Y")];

#[derive(Resource, Default)]
pub(super) struct Controls { pub open: bool, capture: Option<&'static str>, quiet: u8, status: String, bindings: Option<crate::input::Bindings> }
impl Controls {
    pub(super) fn show(&mut self) { self.open = true; self.capture = None; self.bindings = None; self.status.clear(); }
}
#[derive(Component)] struct Root;
#[derive(Component, Clone, Copy)] enum Action { Close, Import, Defaults, Mode, Flag(&'static str), Bind(&'static str) }
#[derive(Component, Clone, Copy)] enum Label { Status, Mode, Binding(&'static str), Flag(&'static str, &'static str) }

pub(super) struct ControlsPlugin;
impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Controls>().add_systems(Startup, setup)
            .add_systems(Update, input.in_set(super::LabInput).before(super::menu_input))
            .add_systems(Update, refresh.after(super::menu_input));
    }
}

fn button(parent: &mut ChildSpawnerCommands, action: Action, text: &str, width: f32, label: Option<Label>) {
    parent.spawn((Button, action, Node { width: Val::Percent(width), min_height: Val::Px(34.), padding: UiRect::all(Val::Px(7.)), ..default() },
        BackgroundColor(Color::srgb(0.09,0.17,0.21)))).with_children(|p| {
        let mut e = p.spawn(super::label(text, 13., Node::default(), Color::srgb(0.91,0.94,0.91)));
        if let Some(label) = label { e.insert(label); }
    });
}
fn setup(mut commands: Commands, camera: Res<crate::camera::CamEntity>) {
    commands.spawn((Root, UiTargetCamera(camera.0), GlobalZIndex(120), Visibility::Hidden,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.), height: Val::Percent(100.),
            padding: UiRect::all(Val::Px(24.)), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgba(0.022,0.055,0.074,0.99)))).with_children(|root| {
        root.spawn(Node { width: Val::Percent(94.), max_width: Val::Px(1150.), flex_direction: FlexDirection::Column, row_gap: Val::Px(8.), ..default() })
            .with_children(|p| {
                p.spawn(super::label("CONTROLS / INPUT", 26., Node::default(), Color::srgb(0.827,0.737,0.471)));
                p.spawn((Label::Status, super::label("Select an action to bind a key, mouse button or wheel direction. Escape cancels capture.", 13., Node::default(), Color::srgb(0.65,0.76,0.80))));
                p.spawn(Node { column_gap: Val::Percent(1.), ..default() }).with_children(|row| {
                    button(row, Action::Import, "IMPORT MORDHAU SAVED CONTROLS", 49., None);
                    button(row, Action::Defaults, "RESTORE GAME DEFAULTS", 49., None);
                });
                button(p, Action::Mode, "Mouse direction (240)", 100., Some(Label::Mode));
                p.spawn(Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Percent(1.), row_gap: Val::Px(5.), ..default() }).with_children(|row| {
                    for &(key, title) in FLAGS { button(row, Action::Flag(key), title, 49., Some(Label::Flag(key,title))); }
                });
                p.spawn(Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Percent(1.), row_gap: Val::Px(5.), ..default() }).with_children(|row| {
                    for &name in ACTIONS { button(row, Action::Bind(name), name, 32., Some(Label::Binding(name))); }
                });
                button(p, Action::Close, "BACK / ESC", 100., None);
            });
    });
}

fn flag(settings: &crate::input::MordhauInputSettings, key: &str) -> i32 {
    match key {
        "MouseXIsFlipAttackSide" => settings.mouse_x_is_flip_attack_side,
        "AngleAttackAfterPress" => settings.angle_attack_after_press,
        "AngleAttacksWithMovement" => settings.angle_attacks_with_movement,
        "InverseAttackDirectionX" => settings.inverse_attack_direction_x,
        "InverseAttackDirectionY" => settings.inverse_attack_direction_y,
        _ => 0,
    }
}

fn ini_path() -> std::path::PathBuf { mh_ui::settings::config_dir().join("Input.ini") }

/// Replace only edited rows. Keep axis config, sensitivity, unrelated actions and other sections verbatim.
fn edit_ini(text: &str, actions: &HashMap<String, Vec<String>>, settings: &HashMap<String,String>) -> String {
    let mut result = Vec::new(); let mut active = false; let mut found = false; let mut inserted = false;
    let append = |out: &mut Vec<String>| {
        let mut names: Vec<_> = actions.keys().collect(); names.sort();
        for name in names {
            let keys = &actions[name];
            let none = vec!["None".to_string()];
            for key in if keys.is_empty() { &none } else { keys } {
                out.push(format!("ActionMappings=(ActionName=\"{name}\",bShift=False,bCtrl=False,bAlt=False,bCmd=False,Key={key})"));
            }
        }
        let mut names: Vec<_> = settings.keys().collect(); names.sort();
        for name in names { out.push(format!("{name}={}",settings[name])); }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if active && !inserted { append(&mut result); inserted = true; }
            active = trimmed == format!("[{SECTION}]"); found |= active;
        }
        if active {
            let parsed = crate::input::parse_section(&format!("[{SECTION}]\n{trimmed}"), SECTION);
            if parsed.actions.iter().any(|(name,_)| actions.contains_key(name)) { continue; }
            if trimmed.split_once('=').is_some_and(|(name,_)| settings.contains_key(name.trim())) { continue; }
        }
        result.push(line.to_string());
    }
    if !found { result.push(format!("[{SECTION}]")); }
    if !inserted { append(&mut result); }
    result.join("\n") + "\n"
}

fn install(world: &mut World, text: &str) -> Result<(),String> {
    // An explicit CLI override is a read-only source; editing it could alter another game's preferences.
    if std::env::var_os("MORDHAU_INPUT_INI").is_some() { return Err("An external input override is active; controls editing is disabled for this session.".into()); }
    let path = ini_path();
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e|e.to_string())?; }
    if path.exists() {
        let backup = path.with_extension("ini.before-controls");
        if !backup.exists() { std::fs::copy(&path,backup).map_err(|e|e.to_string())?; }
    }
    let vfs = world.resource::<crate::source::Source>().vfs.clone().ok_or("Local game data is not mounted")?;
    let default = mh_level::config::text(&vfs,"DefaultInput.ini").ok_or("Game defaults are unavailable")?;
    let saved = crate::input::parse_section(text, SECTION);
    let defaults = crate::input::parse_section(&default,"/Script/Engine.InputSettings");
    let mut b = crate::input::merge_bindings(Some(&saved),&defaults);
    b.source = format!("framework controls: {}",path.display());
    // Save first. A failed write must not leave a transient binding advertised as persistent.
    std::fs::write(&path,text).map_err(|e|e.to_string())?;
    let mut pc = world.resource_mut::<crate::input::PlayerControl>();
    pc.apply_bindings(&b);
    pc.wants_strike = false; pc.wants_stab = false;
    drop(pc);
    world.resource_mut::<Controls>().bindings = Some(b);
    Ok(())
}

fn current(world: &World) -> Result<crate::input::Bindings,String> {
    let vfs = world.resource::<crate::source::Source>().vfs.as_ref().ok_or("Local game data is not mounted")?;
    let default = mh_level::config::text(vfs,"DefaultInput.ini").ok_or("Game defaults are unavailable")?;
    Ok(crate::input::load_bindings(&default))
}
fn mode240(b: &crate::input::Bindings) -> bool { ["Strike","Stab"].iter().any(|n| b.actions.get(*n).is_some_and(|keys| !keys.is_empty())) }

fn choose(world: &mut World, action: Action) -> Result<(),String> {
    if matches!(action,Action::Close) { world.resource_mut::<Controls>().open = false; return Ok(()); }
    if let Action::Bind(name) = action {
        let mut panel = world.resource_mut::<Controls>(); panel.capture = Some(name); panel.quiet = 2;
        panel.status = format!("Bind {name}: press a key, mouse button or wheel direction. Escape cancels."); return Ok(());
    }
    let mut text = std::fs::read_to_string(ini_path()).unwrap_or_default();
    let mut actions = HashMap::new(); let mut settings = HashMap::new();
    match action {
        Action::Import => {
            let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
            let path = std::path::PathBuf::from(base).join("Mordhau/Saved/Config/WindowsClient/Input.ini");
            text = std::fs::read_to_string(&path).map_err(|e|format!("Could not import {}: {e}",path.display()))?;
            if crate::input::parse_section(&text,SECTION).actions.is_empty() { return Err("The game's saved controls contain no action mappings.".into()); }
        }
        Action::Defaults => {
            let vfs = world.resource::<crate::source::Source>().vfs.as_ref().ok_or("Local game data is not mounted")?;
            text = mh_level::config::text(vfs,"DefaultInput.ini").ok_or("Game defaults are unavailable")?
                .replace("[/Script/Engine.InputSettings]",&format!("[{SECTION}]"));
            let d = crate::input::MordhauInputSettings::default();
            for &(key,_) in FLAGS { settings.insert(key.to_string(),flag(&d,key).to_string()); }
        }
        Action::Flag(key) => { settings.insert(key.to_string(),(1-flag(&world.resource::<crate::input::PlayerControl>().settings,key).clamp(0,1)).to_string()); }
        Action::Mode => {
            let b = current(world)?; let enabled = !mode240(&b);
            let vfs = world.resource::<crate::source::Source>().vfs.as_ref().ok_or("Local game data is not mounted")?;
            let defaults = mh_level::config::text(vfs,"DefaultInput.ini").ok_or("Game defaults are unavailable")?;
            let defaults = crate::input::parse_section(&defaults,"/Script/Engine.InputSettings");
            let defaults = crate::input::merge_bindings(None,&defaults);
            // Generic Strike/Stab invoke the native direction selector; directional actions invoke preset requests.
            // Turning AngleAttackAfterPress off only changes sampling time; it does not disable 240 input.
            for (generic,directional) in [("Strike","Right Strike"),("Stab","Right Stab")] {
                let keys = b.actions.get(if enabled { directional } else { generic }).cloned().unwrap_or_default();
                let mut target = b.actions.get(if enabled { generic } else { directional }).cloned().unwrap_or_default();
                for key in keys { if !target.contains(&key) { target.push(key); } }
                if target.is_empty() { target = defaults.actions.get(generic).cloned().unwrap_or_default(); }
                actions.insert(if enabled { directional } else { generic }.to_string(),Vec::new());
                actions.insert(if enabled { generic } else { directional }.to_string(),target);
            }
            settings.insert("MouseXIsFlipAttackSide".into(),i32::from(enabled).to_string());
        }
        _ => {}
    }
    text = edit_ini(&text,&actions,&settings);
    install(world,&text)?;
    world.resource_mut::<Controls>().status = "Saved and applied. Movement, sensitivity and unedited bindings are retained.".into();
    Ok(())
}

fn key_name(key: KeyCode) -> String {
    match key { KeyCode::Space => "SpaceBar".into(), KeyCode::ShiftLeft => "LeftShift".into(), KeyCode::ShiftRight => "RightShift".into(),
        KeyCode::ControlLeft => "LeftControl".into(), KeyCode::ControlRight => "RightControl".into(), KeyCode::AltLeft => "LeftAlt".into(),
        KeyCode::AltRight => "RightAlt".into(), KeyCode::Digit0 => "Zero".into(),KeyCode::Digit1 => "One".into(),KeyCode::Digit2 => "Two".into(),
        KeyCode::Digit3 => "Three".into(),KeyCode::Digit4 => "Four".into(),KeyCode::Digit5 => "Five".into(),KeyCode::Digit6 => "Six".into(),
        KeyCode::Digit7 => "Seven".into(),KeyCode::Digit8 => "Eight".into(),KeyCode::Digit9 => "Nine".into(),
        other => format!("{other:?}").trim_start_matches("Key").to_string() }
}
fn input(world: &mut World) {
    if !world.resource::<Controls>().open { return; }
    let keys: Vec<_> = world.resource::<ButtonInput<KeyCode>>().get_just_pressed().copied().collect();
    if keys.contains(&KeyCode::Escape) {
        let mut c = world.resource_mut::<Controls>();
        if c.capture.take().is_some() { c.status = "Binding capture cancelled.".into(); } else { c.open = false; }
        world.resource_mut::<ButtonInput<KeyCode>>().clear_just_pressed(KeyCode::Escape);
        return;
    }
    // Enter must not close the underlying range menu while this editor owns input.
    world.resource_mut::<ButtonInput<KeyCode>>().clear_just_pressed(KeyCode::Enter);
    if let Some(name) = world.resource::<Controls>().capture {
        if world.resource::<Controls>().quiet > 0 { world.resource_mut::<Controls>().quiet -= 1; return; }
        let mut key = keys.first().copied().map(key_name);
        if key.is_none() {
            key = world.resource::<ButtonInput<MouseButton>>().get_just_pressed().find_map(|m| match m {
                MouseButton::Left => Some("LeftMouseButton"),MouseButton::Right => Some("RightMouseButton"),MouseButton::Middle => Some("MiddleMouseButton"),
                MouseButton::Back => Some("ThumbMouseButton"),MouseButton::Forward => Some("ThumbMouseButton2"),_ => None }).map(str::to_string);
        }
        if key.is_none() {
            if let Some(w) = world.get_resource::<bevy::input::mouse::AccumulatedMouseScroll>() {
                if w.delta.y != 0. { key = Some(if w.delta.y > 0. {"MouseScrollUp"} else {"MouseScrollDown"}.into()); }
            }
        }
        if let Some(key) = key {
            let result = if crate::input::ue_key(&key).is_none() { Err(format!("{key} is not supported by this host.")) } else {
                let text = std::fs::read_to_string(ini_path()).unwrap_or_default();
                let mut actions = HashMap::new(); actions.insert(name.to_string(),vec![key.clone()]);
                install(world,&edit_ini(&text,&actions,&HashMap::new()))
            };
            let mut c = world.resource_mut::<Controls>();
            c.status = match result { Ok(()) => { c.capture = None; format!("Saved {name}: {key}") },Err(e) => e };
        }
        return;
    }
    let actions: Vec<_> = world.query::<(Ref<Interaction>,&Action)>().iter(world)
        .filter(|(i,_)| i.is_changed() && **i == Interaction::Pressed).map(|(_,a)| *a).collect();
    for action in actions { if let Err(e) = choose(world,action) { world.resource_mut::<Controls>().status = e; } }
}
fn refresh(world: &mut World) {
    let open = world.resource::<Controls>().open;
    for mut v in world.query_filtered::<&mut Visibility,With<Root>>().iter_mut(world) { *v = if open {Visibility::Inherited} else {Visibility::Hidden}; }
    if !open { return; }
    // Read configuration once per editor session, never every rendered frame.
    if world.resource::<Controls>().bindings.is_none() {
        let b = match current(world) { Ok(b) => b,Err(_) => return };
        world.resource_mut::<Controls>().bindings = Some(b);
    }
    let b = world.resource::<Controls>().bindings.as_ref().unwrap().clone();
    let status = world.resource::<Controls>().status.clone();
    for (label,mut text) in world.query::<(&Label,&mut Text)>().iter_mut(world) {
        let next = match label {
            Label::Status => if status.is_empty() { "Select an action to bind a key, mouse button or wheel. Changes save immediately.".into() } else {status.clone()},
            Label::Mode => format!("MOUSE DIRECTION / 240: {}  — click to switch input mode",if mode240(&b) {"ON"} else {"OFF / DIRECTIONAL BINDINGS"}),
            Label::Binding(name) => format!("{}: {}",name,b.actions.get(*name).filter(|v|!v.is_empty()).map(|v|v.join(" / ")).unwrap_or("Unbound".into())),
            Label::Flag(key,title) => format!("{}: {}",title,if flag(&b.settings,key)!=0 {"ON"} else {"OFF"}),
        };
        if text.0 != next { text.0 = next; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_preserve_other_controls_and_explicit_unbinding_suppresses_native_default_append() {
        let original = format!("[Console]\nHistoryBuffer=local\n[{SECTION}]\nActionMappings=(ActionName=\"Strike\",Key=LeftMouseButton)\nActionMappings=(ActionName=\"Kick\",Key=B)\nAxisMappings=(AxisName=\"MoveForward\",Key=W,Scale=1.0)\nMouseXSensitivity=0.035\n[Other]\nUnrelated=retained\n");
        let changes = HashMap::from([("Strike".into(),vec![]),("Kick".into(),vec!["F".into()])]);
        let text = edit_ini(&original,&changes,&HashMap::new());
        for preserved in ["HistoryBuffer=local","MouseXSensitivity=0.035","AxisMappings=(AxisName=\"MoveForward\",Key=W,Scale=1.0)","[Other]\nUnrelated=retained"] { assert!(text.contains(preserved)); }
        let saved = crate::input::parse_section(&text,SECTION);
        let defaults = crate::input::parse_section(&format!("[/Script/Engine.InputSettings]\nActionMappings=(ActionName=\"Strike\",Key=LeftMouseButton)"),"/Script/Engine.InputSettings");
        let bindings = crate::input::merge_bindings(Some(&saved),&defaults);
        assert!(bindings.actions["Strike"].is_empty()); assert_eq!(bindings.actions["Kick"],vec!["F"]);
        assert_eq!(bindings.axes["MoveForward"],vec![("W".into(),1.)]);
    }
}

//! UMordhauInput (GetMordhauInput()) for the controls / action-binding screens: a config object whose
//! [/Script/Mordhau.MordhauInput] section in the user's Input.ini holds its UPROPERTYs (ControlScheme, mouse / gamepad
//! sensitivity, inversion and deadzones, toggle options) plus the binding arrays ActionMappings / AxisMappings /
//! ConsoleKeys / AxisConfig, one `Key=Value` line per element (struct values in ExportText form
//! `(ActionName="X",bShift=False,...,Key=W)`).
//!   accessors: plain loads / stores (`GetMouseXSensitivity` 0x13da300, `GetMouseXInverted` 0x984c70,
//!   `GetControlScheme` 0x984840, `SetMouseXSensitivity` 0x15a7460, ... each one mov)
//!   `GetActionKeyBindings` 0x1594a80 / `GetAxisKeyBindings` 0x1594e00 / `GetConsoleKeyBindings` 0x1595320: copy the
//!   array into the out parameter; `GetActionName` 0x1594a90 / `GetActionKey` 0x1594a50: the mapping's fields
//!   limits: `GetMouseSensitivityLimits` 0x1596ce0 (0.01, 0.2), `GetGamepadSensitivityLimits` 0x15961a0 (0.1, 5.0),
//!   `GetGamepadDeadzoneLimits` 0x15960e0 (0.0, 0.75)
//!   `SaveSettings` 0x15a7080 -> SaveConfig: the section rewritten from the object (other sections kept)
//! Config dir as settings.rs ($MH_CONFIG_DIR or the game's Saved/Config/WindowsClient).

use crate::model::*;
use crate::vm::Vm;

pub const SECTION: &str = "[/Script/Mordhau.MordhauInput]";
const ARRAYS: &[&str] = &["ActionMappings", "AxisMappings", "ConsoleKeys", "AxisConfig", "SpeechMappings"];

/// ExportText struct / scalar -> value
pub fn parse_text(s: &str) -> V {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        let mut m = std::collections::BTreeMap::new();
        let mut depth = 0;
        let mut quote = false;
        let mut start = 0;
        let b: Vec<char> = inner.chars().collect();
        let mut fields = vec![];
        for (i, &c) in b.iter().enumerate() {
            match c {
                '"' => quote = !quote,
                '(' if !quote => depth += 1,
                ')' if !quote => depth -= 1,
                ',' if !quote && depth == 0 => {
                    fields.push(b[start..i].iter().collect::<String>());
                    start = i + 1;
                }
                _ => {}
            }
        }
        fields.push(b[start..].iter().collect::<String>());
        for f in fields {
            if let Some((k, v)) = f.split_once('=') {
                m.insert(k.trim().to_string(), parse_text(v));
            }
        }
        // FKey fields are FName-like: Key=W -> {KeyName: W}
        if let Some(V::Name(k)) = m.get("Key").cloned() {
            m.insert("Key".into(), V::st(&[("KeyName", V::Name(k))]));
        }
        return V::Struct(Box::new(m));
    }
    if let Some(q) = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return V::Name(q.to_string());
    }
    match s {
        "True" => V::Bool(true),
        "False" => V::Bool(false),
        _ => s
            .parse::<i64>()
            .map(V::Int)
            .or_else(|_| s.parse::<f64>().map(V::Float))
            .or_else(|_| s.trim_end_matches('f').parse::<f64>().map(V::Float))
            .unwrap_or_else(|_| V::Name(s.to_string())),
    }
}

/// value -> ExportText (the inverse of parse_text for what the section holds)
pub fn export_text(v: &V) -> String {
    match v {
        V::Bool(b) => if *b { "True" } else { "False" }.into(),
        V::Int(i) => i.to_string(),
        V::Float(f) => format!("{f:.6}"),
        V::Struct(m) => {
            if m.len() == 1 && m.contains_key("KeyName") {
                return m["KeyName"].s();
            }
            // UScriptStruct::ExportText writes members in declaration order: FInputActionKeyMapping (ActionName,
            // bShift, bCtrl, bAlt, bCmd, Key), FInputAxisKeyMapping (AxisName, Scale, Key), as the real Input.ini
            let order: &[&str] = if m.contains_key("ActionName") {
                &["ActionName", "bShift", "bCtrl", "bAlt", "bCmd", "Key"]
            } else if m.contains_key("AxisName") {
                &["AxisName", "Scale", "Key"]
            } else {
                &[]
            };
            let mut keys: Vec<&String> = order.iter().filter_map(|k| m.get_key_value(*k).map(|x| x.0)).collect();
            keys.extend(m.keys().filter(|k| !order.contains(&k.as_str())));
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| (k, &m[k]))
                .map(|(k, x)| match x {
                    V::Name(s) if k == "ActionName" || k == "AxisName" || k == "AxisKeyName" => format!("{k}=\"{s}\""),
                    _ => format!("{k}={}", export_text(x)),
                })
                .collect();
            format!("({})", parts.join(","))
        }
        v => v.s(),
    }
}

/// the UMordhauInput constructor (`UMordhauInput::UMordhauInput` 0x157f400): scalar defaults, then the UInputSettings
/// CDO's AxisConfig / ActionMappings / AxisMappings / ConsoleKeys (Engine/Config/BaseInput.ini then
/// Mordhau/Config/DefaultInput.ini, [/Script/Engine.InputSettings], with the config array operators: "+" add unique,
/// "-" remove, "." add, bare = reset then add), then `UMordhauInput::LoadAxisConfig` 0x159caa0 (MouseX / MouseY /
/// Gamepad axes -> b<Axis>Inverted, <Axis>Sensitivity from the AxisConfig entry; gamepad <Axis>Deadzone from its
/// DeadZone: UNCONFIRMED for the deadzone)
pub fn vanilla(vm: &mut Vm, o: Id) {
    for (k, v) in [("ControlScheme", 0), ("AngleAttacksWithMovement", 0), ("MouseXIsFlipAttackSide", 1), ("InverseAttackDirectionX", 0), ("InverseAttackDirectionY", 0), ("AngleAttackAfterPress", 1), ("ToggleSprint", 0), ("ToggleCrouch", 0)] {
        vm.set(o, k, V::Int(v));
    }
    let mut arrays: std::collections::HashMap<&str, Vec<String>> = ARRAYS.iter().map(|a| (*a, vec![])).collect();
    for f in ["Engine/Config/BaseInput.ini", "Mordhau/Config/DefaultInput.ini"] {
        let Some(b) = vm.rd.file(f) else { continue };
        let t = String::from_utf8_lossy(&b).into_owned();
        let mut in_sec = false;
        for l in t.lines() {
            let l = l.trim_end_matches('\r').trim();
            if l.starts_with('[') {
                in_sec = l == "[/Script/Engine.InputSettings]";
                continue;
            }
            if !in_sec {
                continue;
            }
            let Some((k, v)) = l.split_once('=') else { continue };
            let (op, key) = match k.chars().next() {
                Some(c @ ('+' | '-' | '.' | '!')) => (c, &k[1..]),
                _ => (' ', k),
            };
            let Some(a) = arrays.get_mut(key) else { continue };
            match op {
                '+' => {
                    if !a.iter().any(|x| x == v) {
                        a.push(v.to_string());
                    }
                }
                '.' => a.push(v.to_string()),
                '-' => a.retain(|x| x != v),
                '!' => a.clear(),
                _ => {
                    a.clear();
                    a.push(v.to_string());
                }
            }
        }
    }
    let mut order = vec![];
    for (k, a) in &arrays {
        let vals: Vec<V> = a.iter().map(|x| if *k == "ConsoleKeys" { V::st(&[("KeyName", V::Name(x.clone()))]) } else { parse_text(x) }).collect();
        vm.set(o, k, V::Array(vals));
        order.push(V::Str(k.to_string()));
    }
    // LoadAxisConfig
    let cfg = vm.prop(o, "AxisConfig").arr().to_vec();
    for (axis, field) in [("MouseX", "MouseX"), ("MouseY", "MouseY"), ("Gamepad_LeftX", "GamepadLeftX"), ("Gamepad_LeftY", "GamepadLeftY"), ("Gamepad_RightX", "GamepadRightX"), ("Gamepad_RightY", "GamepadRightY")] {
        if let Some(e) = cfg.iter().find(|e| e.field("AxisKeyName").s() == axis) {
            let p = e.field("AxisProperties");
            vm.set(o, &format!("b{field}Inverted"), V::Bool(p.field("bInvert").truthy()));
            vm.set(o, &format!("{field}Sensitivity"), V::Float(p.field("Sensitivity").f()));
            if field.starts_with("Gamepad") {
                vm.set(o, &format!("{field}Deadzone"), V::Float(p.field("DeadZone").f()));
            }
        }
    }
    for k in ["ControlScheme", "AngleAttacksWithMovement", "MouseXIsFlipAttackSide", "InverseAttackDirectionX", "InverseAttackDirectionY", "AngleAttackAfterPress", "ToggleSprint", "ToggleCrouch", "bMouseXInverted", "MouseXSensitivity", "bMouseYInverted", "MouseYSensitivity"] {
        order.push(V::Str(k.to_string()));
    }
    for g in ["GamepadLeftX", "GamepadLeftY", "GamepadRightX", "GamepadRightY"] {
        for f in [format!("b{g}Inverted"), format!("{g}Sensitivity"), format!("{g}Deadzone")] {
            order.push(V::Str(f));
        }
    }
    vm.set(o, "__order", V::Array(order));
}

pub fn load(vm: &mut Vm, o: Id) {
    vanilla(vm, o);
    let p = crate::settings::config_dir().join("Input.ini");
    let Ok(t) = std::fs::read_to_string(&p) else { return };
    if !t.contains(SECTION) {
        return;
    }
    let mut in_sec = false;
    let mut arrays: std::collections::HashMap<&str, Vec<V>> = ARRAYS.iter().map(|a| (*a, vec![])).collect();
    let mut order = vec![];
    for l in t.lines() {
        let l = l.trim_end_matches('\r');
        if l.starts_with('[') {
            in_sec = l == SECTION;
            continue;
        }
        if !in_sec {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let k = k.trim_start_matches(['+', '-', '.']);
        if !order.contains(&k.to_string()) {
            order.push(k.to_string());
        }
        if let Some(a) = arrays.get_mut(k) {
            let mut x = parse_text(v);
            if k == "ConsoleKeys" {
                x = V::st(&[("KeyName", V::Name(v.to_string()))]);
            }
            a.push(x);
        } else {
            vm.set(o, k, parse_text(v));
        }
    }
    for (k, a) in arrays {
        vm.set(o, k, V::Array(a));
    }
    vm.set(o, "__order", V::Array(order.into_iter().map(V::Str).collect()));
}

pub fn save(vm: &Vm, o: Id) -> std::io::Result<()> {
    let p = crate::settings::config_dir().join("Input.ini");
    let t = std::fs::read_to_string(&p).unwrap_or_default();
    let mut body = String::new();
    for k in vm.prop(o, "__order").arr() {
        let k = k.s();
        if ARRAYS.contains(&k.as_str()) {
            for x in vm.prop(o, &k).arr() {
                body.push_str(&format!("{k}={}\n", export_text(x)));
            }
        } else {
            body.push_str(&format!("{k}={}\n", export_text(&vm.prop(o, &k))));
        }
    }
    let mut out = String::new();
    let mut skipping = false;
    let mut done = false;
    for l in t.lines() {
        let l2 = l.trim_end_matches('\r');
        if l2.starts_with('[') {
            skipping = l2 == SECTION;
            if skipping {
                out.push_str(l2);
                out.push('\n');
                out.push_str(&body);
                done = true;
                continue;
            }
        }
        if skipping {
            if l2.is_empty() {
                out.push('\n');
            }
            continue;
        }
        out.push_str(l2);
        out.push('\n');
    }
    if !done {
        out.push_str(&format!("\n{SECTION}\n{body}"));
    }
    // the rewrite's own config dir does not exist before the first save (fidelity-audit r7)
    if let Some(d) = std::path::Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(&p, out)
}

/// the input object's natives; Some((return, outs))
pub fn call(vm: &mut Vm, o: Id, name: &str, a: &[V]) -> Option<(V, Vec<(usize, V)>)> {
    let a0 = a.first().cloned().unwrap_or_default();
    let lim = |x: f64, y: f64| Some((V::st(&[("X", V::Float(x)), ("Y", V::Float(y))]), vec![]));
    match name {
        "GetActionKeyBindings" => Some((V::None, vec![(0, vm.prop(o, "ActionMappings"))])),
        "GetAxisKeyBindings" => Some((V::None, vec![(0, vm.prop(o, "AxisMappings"))])),
        "GetConsoleKeyBindings" => Some((V::None, vec![(0, vm.prop(o, "ConsoleKeys"))])),
        "GetActionName" => Some((a0.field("ActionName").clone(), vec![])),
        "GetMouseSensitivityLimits" => lim(0.01, 0.2),
        "GetGamepadSensitivityLimits" => lim(0.1, 5.0),
        "GetGamepadDeadzoneLimits" => lim(0.0, 0.75),
        "SaveSettings" | "ApplySettings" => {
            if name == "SaveSettings" {
                if let Err(e) = save(vm, o) {
                    eprintln!("mh-ui: input settings not saved: {e}");
                }
                // the live keymaps rebuild (UMordhauInput::ApplySettings -> ForceRebuildKeymaps rva=0x1594240): the
                // host re-reads the saved Input.ini after this frame's VM work (mh-runtime input.rs)
                vm.actions.push(crate::vm::Action::SettingsApplied { input: true });
            }
            Some((V::None, vec![]))
        }
        "RestoreDefaultSettings" | "LoadSettings" => {
            load(vm, o);
            Some((V::None, vec![]))
        }
        "ClearKeyBindings" => {
            for k in ["ActionMappings", "AxisMappings", "ConsoleKeys"] {
                vm.set(o, k, V::Array(vec![]));
            }
            Some((V::None, vec![]))
        }
        // `AddActionKeyBinding` 0x1581df0 (ActionName, Key): a new FInputActionKeyMapping without modifiers
        "AddActionKeyBinding" => {
            let key = a.get(1).cloned().unwrap_or_default();
            let m = V::st(&[("ActionName", a0), ("bShift", V::Bool(false)), ("bCtrl", V::Bool(false)), ("bAlt", V::Bool(false)), ("bCmd", V::Bool(false)), ("Key", key)]);
            push(vm, o, "ActionMappings", m);
            Some((V::None, vec![]))
        }
        // `UMordhauInput::AddAxisKeyBinding` rva 0x1581f00 (AxisName, Key): Scale -1 for MouseY else +1 (0xbf800000 /
        // 0x3f800000); a name ending "Backward" / "Down" / "Left" is the UI's opposite-direction row: an Axis1D key
        // (MouseY, a stick axis) adds NOTHING there (FKey::IsAxis1D -> skip), a button is stored under the positive
        // name (Backward->Forward, Down->Up, Left->Right; .rdata 0x144352d40..) with the scale negated. So the saved
        // file keeps "Look Up" MouseY -1 and never gets a "Look Down" axis (the real game's Input.ini)
        "AddAxisKeyBinding" => {
            let key = a.get(1).cloned().unwrap_or_default();
            let kn = key.field("KeyName").s();
            let mut n = a0.s();
            let mut scale: f64 = if kn == "MouseY" { -1.0 } else { 1.0 };
            if ["Backward", "Down", "Left"].iter().any(|x| n.ends_with(x)) {
                if key_is_axis_1d(&kn) {
                    return Some((V::None, vec![]));
                }
                n = n.replace("Backward", "Forward").replace("Down", "Up").replace("Left", "Right");
                scale = -scale;
            }
            let m = V::st(&[("AxisName", V::Name(n)), ("Scale", V::Float(scale)), ("Key", key)]);
            push(vm, o, "AxisMappings", m);
            Some((V::None, vec![]))
        }
        "AddConsoleKeyBinding" => {
            push(vm, o, "ConsoleKeys", a0);
            Some((V::None, vec![]))
        }
        _ => None,
    }
}

/// FKey::IsAxis1D: the keys EKeys registers with FKeyDetails::Axis1D (UE 4.26 InputCoreTypes.cpp: mouse axes and wheel
/// axis, the gamepad stick and trigger axes, the motion-controller thumbstick / trigger axes)
pub fn key_is_axis_1d(k: &str) -> bool {
    matches!(k, "MouseX" | "MouseY" | "MouseWheelAxis" | "Gamepad_LeftX" | "Gamepad_LeftY" | "Gamepad_RightX" | "Gamepad_RightY" | "Gamepad_LeftTriggerAxis" | "Gamepad_RightTriggerAxis")
        || (k.starts_with("MotionController_") && (k.contains("Thumbstick_X") || k.contains("Thumbstick_Y") || k.contains("TriggerAxis") || k.contains("Grip1Axis") || k.contains("Grip2Axis")))
}

/// FKey display names (UE 4.26 InputCoreTypes.cpp EKeys::Initialize LOCTEXT strings) for the keys the default
/// bindings use; others show their FName
pub fn key_display_name(k: &str) -> String {
    let s = match k {
        "LeftMouseButton" => "Left Mouse Button",
        "RightMouseButton" => "Right Mouse Button",
        "MiddleMouseButton" => "Middle Mouse Button",
        "ThumbMouseButton" => "Thumb Mouse Button",
        "ThumbMouseButton2" => "Thumb Mouse Button 2",
        "MouseScrollUp" => "Mouse Wheel Up",
        "MouseScrollDown" => "Mouse Wheel Down",
        "MouseWheelAxis" => "Mouse Wheel Axis",
        "MouseX" => "Mouse X",
        "MouseY" => "Mouse Y",
        "SpaceBar" => "Space Bar",
        "LeftShift" => "Left Shift",
        "RightShift" => "Right Shift",
        "LeftControl" => "Left Ctrl",
        "RightControl" => "Right Ctrl",
        "LeftAlt" => "Left Alt",
        "RightAlt" => "Right Alt",
        "BackSpace" => "Backspace",
        "CapsLock" => "Caps Lock",
        "Escape" => "Escape",
        "Tilde" => "`",
        "Caret" => "^",
        "Zero" => "0",
        "One" => "1",
        "Two" => "2",
        "Three" => "3",
        "Four" => "4",
        "Five" => "5",
        "Six" => "6",
        "Seven" => "7",
        "Eight" => "8",
        "Nine" => "9",
        "Gamepad_LeftX" => "Gamepad Left Thumbstick X-Axis",
        "Gamepad_LeftY" => "Gamepad Left Thumbstick Y-Axis",
        "Gamepad_RightX" => "Gamepad Right Thumbstick X-Axis",
        "Gamepad_RightY" => "Gamepad Right Thumbstick Y-Axis",
        "Gamepad_LeftThumbstick" => "Gamepad Left Thumbstick Button",
        "Gamepad_RightThumbstick" => "Gamepad Right Thumbstick Button",
        "Gamepad_FaceButton_Bottom" => "Gamepad Face Button Bottom",
        "Gamepad_FaceButton_Right" => "Gamepad Face Button Right",
        "Gamepad_FaceButton_Left" => "Gamepad Face Button Left",
        "Gamepad_FaceButton_Top" => "Gamepad Face Button Top",
        "Gamepad_LeftShoulder" => "Gamepad Left Shoulder",
        "Gamepad_RightShoulder" => "Gamepad Right Shoulder",
        "Gamepad_LeftTrigger" => "Gamepad Left Trigger",
        "Gamepad_RightTrigger" => "Gamepad Right Trigger",
        "Gamepad_DPad_Up" => "Gamepad D-pad Up",
        "Gamepad_DPad_Down" => "Gamepad D-pad Down",
        "Gamepad_DPad_Left" => "Gamepad D-pad Left",
        "Gamepad_DPad_Right" => "Gamepad D-pad Right",
        "Gamepad_Special_Left" => "Gamepad Special Left",
        "Gamepad_Special_Right" => "Gamepad Special Right",
        "None" | "" => "",
        other => other,
    };
    s.to_string()
}

fn push(vm: &mut Vm, o: Id, k: &str, v: V) {
    let mut arr = vm.prop(o, k).arr().to_vec();
    arr.push(v);
    vm.set(o, k, V::Array(arr));
}

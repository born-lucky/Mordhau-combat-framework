//! input.rs - InputPlugin: the local player's controls, ported from the exe's input path (fidelity-audit r1; was a port
//! of godot/game/character/mordhau_input.gd + godot/game/actor/melee_input.gd, whose citations are kept).
//!
//! Bindings: UMordhauInput's own config arrays (ActionMappings / AxisMappings / AxisConfig in the rewrite's own saved
//! Input.ini, mh_ui::settings::config_dir() = $MH_CONFIG_DIR or %LOCALAPPDATA%/MordhauRewrite/Saved/Config/WindowsClient
//! (never the Steam install's), section [/Script/Mordhau.MordhauInput]) replace every
//! UPlayerInput's arrays wholesale (UMordhauInput::ForceRebuildKeymaps rva=0x1594240). Action / axis names the saved
//! lists lack are appended from UInputSettings (DefaultInput.ini [/Script/Engine.InputSettings], read from the paks;
//! UMordhauInput::Tick rva=0x15ac320). Without a saved file the arrays are UInputSettings' (the ctor rva=0x157f400
//! and RestoreDefaultSettings rva=0x15a6fb0 copy them). MouseX/Y sensitivity + invert come from MouseXSensitivity / bMouseXInverted
//! (ApplyAxisConfig rva=0x1582630). The m.* cvars are UMordhauInput settings (ApplySettings rva=0x1583f60 writes
//! m.AngleAttacksWithMovement, m.MouseXIsFlipAttackSide, m.InverseAttackDirectionX/Y, m.AngleAttackAfterPress).
//!
//! Axis values: UPlayerInput::MassageAxisInput rva=0x353d740 (disasm, scripts/ue_dis.py): DeadZone (only when > 0),
//! Exponent, x Sensitivity, bInvert; MouseX/MouseY then x PlayerCameraManager->GetFOVAngle() x FOVScale when
//! bEnableFOVScaling (InputSettings flag bit 0x10, FOVScale +0x74; DefaultInput.ini bEnableFOVScaling=True,
//! FOVScale=0.011110); bEnableMouseSmoothing is False in DefaultInput.ini (flag bit 0x08 path not taken).
//!
//! Per frame, in the exe's order (UPlayerInput::ProcessInputStack: action delegates, then axis delegates; then the
//! character's LODTick rva=0x154c390): actions (SetupPlayerInputComponent rva=0x156c560 bindings), Turn / LookUp
//! (AAdvancedCharacter::Turn rva=0x14a8a90 / LookUp rva=0x1489930: angling registers, the TurnCaps clamp that drops
//! the excess, the LookUpLimit / LookDownLimit clamp), then FlushPendingAnglingInputs rva=0x15d4880 and the
//! bWants360Strike / bWants360Stab requests.

use crate::sim::{FrameInput, Inputs};
use crate::specdata::SpecData;
use bevy::input::mouse::{AccumulatedMouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use std::collections::HashMap;

/// A UE FKey this runtime can read (EKeys names, InputCoreTypes)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    K(KeyCode),
    M(MouseButton),
    WheelUp,
    WheelDown,
}

pub fn ue_key(name: &str) -> Option<Key> {
    use KeyCode::*;
    Some(match name {
        "LeftMouseButton" => Key::M(MouseButton::Left),
        "RightMouseButton" => Key::M(MouseButton::Right),
        "MiddleMouseButton" => Key::M(MouseButton::Middle),
        "ThumbMouseButton" => Key::M(MouseButton::Back),
        "ThumbMouseButton2" => Key::M(MouseButton::Forward),
        "MouseScrollUp" => Key::WheelUp,
        "MouseScrollDown" => Key::WheelDown,
        "SpaceBar" => Key::K(Space),
        "LeftShift" => Key::K(ShiftLeft),
        "RightShift" => Key::K(ShiftRight),
        "LeftControl" => Key::K(ControlLeft),
        "RightControl" => Key::K(ControlRight),
        "LeftAlt" => Key::K(AltLeft),
        "RightAlt" => Key::K(AltRight),
        "Tab" => Key::K(Tab),
        "Enter" => Key::K(Enter),
        "CapsLock" => Key::K(CapsLock),
        // These EKeys names are also emitted by the real controls screen (mh_ui::real::ue_key). Rebinding an
        // action to one used to succeed in the UI and then be silently dropped by apply_bindings' filter_map.
        "Escape" => Key::K(Escape),
        "BackSpace" => Key::K(Backspace),
        "Up" => Key::K(ArrowUp),
        "Down" => Key::K(ArrowDown),
        "Left" => Key::K(ArrowLeft),
        "Right" => Key::K(ArrowRight),
        "Delete" => Key::K(Delete),
        "Home" => Key::K(Home),
        "End" => Key::K(End),
        "PageUp" => Key::K(PageUp),
        "PageDown" => Key::K(PageDown),
        "Tilde" => Key::K(Backquote),
        "F1" => Key::K(F1),
        "F10" => Key::K(F10),
        // EKeys digit names
        "Zero" => Key::K(Digit0),
        "One" => Key::K(Digit1),
        "Two" => Key::K(Digit2),
        "Three" => Key::K(Digit3),
        "Four" => Key::K(Digit4),
        "Five" => Key::K(Digit5),
        "Six" => Key::K(Digit6),
        "Seven" => Key::K(Digit7),
        "Eight" => Key::K(Digit8),
        "Nine" => Key::K(Digit9),
        s if s.len() == 1 => {
            let c = s.chars().next()?;
            Key::K(match c {
                'A' => KeyA, 'B' => KeyB, 'C' => KeyC, 'D' => KeyD, 'E' => KeyE, 'F' => KeyF, 'G' => KeyG, 'H' => KeyH,
                'I' => KeyI, 'J' => KeyJ, 'K' => KeyK, 'L' => KeyL, 'M' => KeyM, 'N' => KeyN, 'O' => KeyO, 'P' => KeyP,
                'Q' => KeyQ, 'R' => KeyR, 'S' => KeyS, 'T' => KeyT, 'U' => KeyU, 'V' => KeyV, 'W' => KeyW, 'X' => KeyX,
                'Y' => KeyY, 'Z' => KeyZ,
                _ => return None,
            })
        }
        _ => return None,
    })
}

/// FInputAxisProperties (DeadZone, Sensitivity, Exponent, bInvert)
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct AxisProps {
    pub dead_zone: f32,
    pub sensitivity: f32,
    pub exponent: f32,
    pub invert: bool,
}

impl Default for AxisProps {
    /// FInputAxisProperties' C++ member defaults (engine InputCoreTypes / PlayerInput.h: DeadZone 0.2, Sensitivity 1,
    /// Exponent 1, bInvert false); UNCONFIRMED against the exe's struct ctor (only used for an AxisConfig line missing a
    /// field)
    fn default() -> Self {
        AxisProps { dead_zone: 0.2, sensitivity: 1.0, exponent: 1.0, invert: false }
    }
}

impl AxisProps {
    /// UPlayerInput::MassageAxisInput rva=0x353d740, the per-key part (disasm 0x14353d7c2..0x14353d849)
    pub fn massage(&self, raw: f32) -> f32 {
        let mut v = raw;
        let dz = self.dead_zone;
        if dz > 0.0 {
            v = if v > 0.0 { (v - dz).max(0.0) / (1.0 - dz) } else { (-v - dz).max(0.0) / (dz - 1.0) };
        }
        if self.exponent != 1.0 {
            let s = if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 };
            v = v.abs().powf(self.exponent) * s;
        }
        v *= self.sensitivity;
        if self.invert {
            v = -v;
        }
        v
    }
}

/// The UMordhauInput settings that drive the m.* cvars (ApplySettings rva=0x1583f60)
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct MordhauInputSettings {
    pub mouse_x_is_flip_attack_side: i32,
    pub angle_attack_after_press: i32,
    pub angle_attacks_with_movement: i32,
    pub inverse_attack_direction_x: i32,
    pub inverse_attack_direction_y: i32,
}

impl Default for MordhauInputSettings {
    /// UMordhauInput::UMordhauInput rva=0x157f400 (decomp UMordhauInput.cpp 2038-2045; the same values as
    /// RestoreDefaultSettings rva=0x15a6fb0): MouseXIsFlipAttackSide 1, AngleAttackAfterPress 1, the rest 0. A fresh install
    /// has no saved Input.ini and DefaultInput.ini has no [/Script/Mordhau.MordhauInput] section, so the ctor values and
    /// its copy of the UInputSettings CDO (ActionMappings / AxisMappings / AxisConfig from DefaultInput.ini) stand
    fn default() -> Self {
        MordhauInputSettings {
            mouse_x_is_flip_attack_side: 1,
            angle_attack_after_press: 1,
            angle_attacks_with_movement: 0,
            inverse_attack_direction_x: 0,
            inverse_attack_direction_y: 0,
        }
    }
}

/// The parsed input config: ActionMappings (name -> UE key names), AxisMappings (name -> (key, scale)), AxisConfig
/// (key name -> props), plain key=value settings of the section.
#[derive(Clone, Debug, Default)]
pub struct InputIni {
    pub actions: Vec<(String, String)>,
    pub axes: Vec<(String, String, f32)>,
    pub axis_config: Vec<(String, AxisProps)>,
    pub settings: HashMap<String, String>,
}

fn field(l: &str, k: &str) -> Option<String> {
    let pat = format!("{k}=");
    let mut from = 0;
    // whole-word match ("Key=" must not match "AxisKeyName=")
    while let Some(i) = l[from..].find(&pat) {
        let at = from + i;
        let ok = at == 0 || matches!(l.as_bytes()[at - 1], b'(' | b',');
        if ok {
            let rest = &l[at + pat.len()..];
            let rest = rest.strip_prefix('"').unwrap_or(rest);
            return Some(rest.split(|c| c == '"' || c == ',' || c == ')').next()?.to_string());
        }
        from = at + pat.len();
    }
    None
}

fn num(s: &str) -> Option<f32> {
    s.trim().trim_end_matches('f').parse().ok()
}

/// One section of an input ini. `+Key=` adds, `Key=` (a saved file) adds, `-Key=` removes nothing here (the shipped
/// file's `-AxisConfig` lines are followed by `+AxisConfig` lines for the same keys).
pub fn parse_section(text: &str, section: &str) -> InputIni {
    let head = format!("[{section}]");
    let mut out = InputIni::default();
    let mut in_sec = false;
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            in_sec = l == head;
            continue;
        }
        if !in_sec || l.starts_with('-') || l.starts_with(';') {
            continue;
        }
        let l = l.strip_prefix('+').unwrap_or(l);
        if let Some(r) = l.strip_prefix("ActionMappings=") {
            if let (Some(a), Some(k)) = (field(r, "ActionName"), field(r, "Key")) {
                out.actions.push((a, k));
            }
        } else if let Some(r) = l.strip_prefix("AxisMappings=") {
            if let (Some(a), Some(k)) = (field(r, "AxisName"), field(r, "Key")) {
                let s = field(r, "Scale").and_then(|s| num(&s)).unwrap_or(1.0);
                out.axes.push((a, k, s));
            }
        } else if let Some(r) = l.strip_prefix("AxisConfig=") {
            if let Some(k) = field(r, "AxisKeyName") {
                let d = AxisProps::default();
                let p = AxisProps {
                    dead_zone: field(r, "DeadZone").and_then(|s| num(&s)).unwrap_or(d.dead_zone),
                    sensitivity: field(r, "Sensitivity").and_then(|s| num(&s)).unwrap_or(d.sensitivity),
                    exponent: field(r, "Exponent").and_then(|s| num(&s)).unwrap_or(d.exponent),
                    invert: field(r, "bInvert").is_some_and(|s| s.eq_ignore_ascii_case("true")),
                };
                out.axis_config.push((k, p));
            }
        } else if let Some((k, v)) = l.split_once('=') {
            out.settings.insert(k.to_string(), v.to_string());
        }
    }
    out
}

#[allow(dead_code)] // tests + script tooling
/// Back-compat view of a DefaultInput.ini text: (actions, axes, MouseX/MouseY sensitivity) of [/Script/Engine.InputSettings]
pub fn parse_input_ini(text: &str) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<(String, f32)>>, HashMap<String, f32>) {
    let ini = parse_section(text, "/Script/Engine.InputSettings");
    let ini = if ini.actions.is_empty() && ini.axes.is_empty() { parse_any(text) } else { ini };
    let mut acts: HashMap<String, Vec<String>> = HashMap::new();
    for (a, k) in &ini.actions {
        if k != "None" && !k.is_empty() {
            acts.entry(a.clone()).or_default().push(k.clone());
        }
    }
    let mut axes: HashMap<String, Vec<(String, f32)>> = HashMap::new();
    for (a, k, s) in &ini.axes {
        axes.entry(a.clone()).or_default().push((k.clone(), *s));
    }
    let sens = ini.axis_config.iter().map(|(k, p)| (k.clone(), p.sensitivity)).collect();
    (acts, axes, sens)
}

/// every section at once (unit-test snippets without a section header)
#[allow(dead_code)]
fn parse_any(text: &str) -> InputIni {
    parse_section(&format!("[x]\n{text}"), "x")
}

/// The effective bindings: the saved UMordhauInput arrays + the defaults' missing names (see the module header).
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    pub actions: HashMap<String, Vec<String>>,
    pub axes: HashMap<String, Vec<(String, f32)>>,
    pub axis_config: HashMap<String, AxisProps>,
    pub settings: MordhauInputSettings,
    pub source: String,
}

pub fn merge_bindings(saved: Option<&InputIni>, dflt: &InputIni) -> Bindings {
    let mut b = Bindings::default();
    let empty = InputIni::default();
    let s = saved.unwrap_or(&empty);
    // ForceRebuildKeymaps rva=0x1594240 copies the UMordhauInput arrays; UMordhauInput::Tick rva=0x15ac320 appends the
    // UInputSettings mappings whose ActionName / AxisName the saved arrays do not have
    let mut acts: Vec<(String, String)> = s.actions.clone();
    for (a, k) in &dflt.actions {
        if !s.actions.iter().any(|(n, _)| n == a) {
            acts.push((a.clone(), k.clone()));
        }
    }
    for (a, k) in acts {
        let e = b.actions.entry(a).or_default();
        if k != "None" && !k.is_empty() {
            e.push(k);
        }
    }
    let mut axes: Vec<(String, String, f32)> = s.axes.clone();
    for (a, k, sc) in &dflt.axes {
        if !s.axes.iter().any(|(n, _, _)| n == a) {
            axes.push((a.clone(), k.clone(), *sc));
        }
    }
    for (a, k, sc) in axes {
        if k != "None" && !k.is_empty() {
            b.axes.entry(a).or_default().push((k, sc));
        }
    }
    // AxisConfig: the saved array when it has one (ForceRebuildKeymaps copies it), else UInputSettings'
    let ac = if s.axis_config.is_empty() { &dflt.axis_config } else { &s.axis_config };
    for (k, p) in ac {
        b.axis_config.insert(k.clone(), *p);
    }
    // ApplyAxisConfig rva=0x1582630: MouseX / MouseY Sensitivity and bInvert from the UMordhauInput settings
    let gs = |k: &str| s.settings.get(k).cloned();
    for (key, sk, ik) in [("MouseX", "MouseXSensitivity", "bMouseXInverted"), ("MouseY", "MouseYSensitivity", "bMouseYInverted")] {
        if let Some(p) = b.axis_config.get_mut(key) {
            if let Some(v) = gs(sk).and_then(|v| num(&v)) {
                p.sensitivity = v;
            }
            if let Some(v) = gs(ik) {
                p.invert = v.eq_ignore_ascii_case("true");
            }
        }
    }
    if saved.is_some() {
        let gi = |k: &str, d: i32| gs(k).and_then(|v| v.trim().parse().ok()).unwrap_or(d);
        let d = MordhauInputSettings::default();
        b.settings = MordhauInputSettings {
            mouse_x_is_flip_attack_side: gi("MouseXIsFlipAttackSide", d.mouse_x_is_flip_attack_side),
            angle_attack_after_press: gi("AngleAttackAfterPress", d.angle_attack_after_press),
            angle_attacks_with_movement: gi("AngleAttacksWithMovement", d.angle_attacks_with_movement),
            inverse_attack_direction_x: gi("InverseAttackDirectionX", d.inverse_attack_direction_x),
            inverse_attack_direction_y: gi("InverseAttackDirectionY", d.inverse_attack_direction_y),
        };
    }
    b
}

/// The player's saved Input.ini (UE's per-user config layer; Mordhau saves under WindowsClient)
pub fn saved_input_ini_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("MORDHAU_INPUT_INI") {
        return Some(p.into());
    }
    // the rewrite's own config dir, never the Steam install's (user directive 2026-10-05, fidelity-audit r7)
    Some(mh_ui::settings::config_dir().join("Input.ini"))
}

/// AMordhauPlayerController angling state (offsets: AMordhauPlayerController.h)
#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct Angling {
    pub x: f32, // +0x59c AnglingX
    pub y: f32, // +0x5a4 AnglingY
    pend_x: f32, // +0x594
    pend_y: f32, // +0x598
    has_pending: bool, // +0x58c
    pub frame_x: i64, // +0x5a0 AnglingXFrame
    pub frame_y: i64, // +0x5a8 AnglingYFrame
}

fn sgn(v: f32) -> f32 {
    if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 }
}

impl Angling {
    /// RegisterAnglingXInput rva=0x15eee50
    pub fn register_x(&mut self, x: f32) {
        if x != 0.0 {
            self.pend_x = x;
            self.has_pending = true;
        }
    }
    /// RegisterAnglingYInput rva=0x15eee70
    pub fn register_y(&mut self, y: f32) {
        if y != 0.0 {
            self.pend_y = y;
            self.has_pending = true;
        }
    }
    #[allow(dead_code)]
    pub fn register(&mut self, x: f32, y: f32) {
        self.register_x(x);
        self.register_y(y);
    }
    /// FlushPendingAnglingInputs rva=0x15d4880 (m.AngleAttacksWithMovement = 0 branch). wants360 = the character's
    /// bWants360Stab || bWants360Strike (+0xe34/+0xe35), listen_frame = Listen360FrameStart (+0xe28)
    pub fn flush(&mut self, k: &AngK, frame: i64, wants360: bool, listen_frame: i64) {
        if !self.has_pending {
            return;
        }
        self.has_pending = false;
        if self.pend_x != 0.0 {
            self.x = if sgn(self.x) == sgn(self.pend_x) {
                if wants360 && listen_frame <= self.frame_x { self.x + self.pend_x } else { (self.x + self.pend_x) * k.average }
            } else {
                self.pend_x
            };
            self.frame_x = frame;
            if self.pend_y == 0.0 && (!wants360 || self.frame_y < listen_frame) {
                self.y = (self.y * k.average).abs().max(k.floor) * sgn(self.y);
            }
        }
        if self.pend_y != 0.0 {
            self.y = if sgn(self.y) == sgn(self.pend_y) {
                if wants360 && listen_frame <= self.frame_y { self.y + self.pend_y } else { (self.y + self.pend_y) * k.average }
            } else {
                self.pend_y
            };
            self.frame_y = frame;
            if self.pend_x == 0.0 && (!wants360 || self.frame_x < listen_frame) {
                self.x = (self.x * k.average).abs().max(k.floor) * sgn(self.x);
            }
        }
        self.pend_x = 0.0;
        self.pend_y = 0.0;
    }
    /// GetAnglingVector / GetAnglingAngle apply m.InverseAnglingX/Y after accumulation.
    /// UMordhauInput::ApplySettings maps the saved InverseAttackDirectionX/Y settings to those cvars.
    pub fn vector(&self, settings: &MordhauInputSettings) -> (f32, f32) {
        (self.x * if settings.inverse_attack_direction_x != 0 { -1.0 } else { 1.0 },
         self.y * if settings.inverse_attack_direction_y != 0 { -1.0 } else { 1.0 })
    }

    /// GetAnglingAngle rva=0x15d52b0; inversion does not change the stored accumulation/frame history.
    pub fn angle(&self, k: &AngK, settings: &MordhauInputSettings) -> f32 {
        let (x, y) = self.vector(settings);
        let mut a = mordhau_core::ue::ue_atan2(y, x) * k.rad_to_deg;
        while a > k.max {
            a += k.wrap_down;
        }
        while a < k.min {
            a += k.wrap_up;
        }
        a
    }
    /// UMotionSystemComponent::RequestStrike360 rva=0x14d1a80 / RequestStab360 rva=0x14d1950 (m.StabOnStrikeYAxis 0):
    /// fold into [-90, 90]; left = AnglingX < 0 (`X <= 0 && X != 0`), inverted while bWantsFlipAttackSide (+0x1032?:
    /// the `RelativeScale3D.Y` byte the decomp reads; set by FlipAttackSidePressed rva=0x153e8a0); negate for the left
    /// side. Moves: 0 Right / 1 Left strike, 2 Stab / 3 AltStab.
    pub fn attack(&self, k: &AngK, stab: bool, flip: bool, settings: &MordhauInputSettings) -> (i64, f64) {
        let mut a = self.angle(k, settings);
        if a > k.fold_max {
            a += k.fold_down;
        } else if a < k.fold_min {
            a += k.fold_up;
        }
        let mut left = self.vector(settings).0 < 0.0;
        if flip {
            left = !left;
        }
        if left {
            a = -a;
        }
        let mv = match (stab, left) {
            (true, true) => 3,
            (true, false) => 2,
            (false, true) => 1,
            (false, false) => 0,
        };
        (mv, a as f64)
    }
}

/// Angling constants (exe .rdata via the spec matrix: ENT_CONST_PLAYER_*)
#[derive(Clone, Debug, serde::Serialize)]
pub struct AngK {
    pub average: f32,
    pub floor: f32,
    pub rad_to_deg: f32,
    pub max: f32,
    pub min: f32,
    pub wrap_down: f32,
    pub wrap_up: f32,
    pub fold_max: f32,
    pub fold_min: f32,
    pub fold_down: f32,
    pub fold_up: f32,
    pub upper_strike: f32,
    pub lower_strike: f32,
}

impl AngK {
    pub fn from_spec(s: Option<&SpecData>) -> AngK {
        let c = |n: &str, d: f64| s.map(|s| s.constant(&format!("PLAYER_{n}"), d)).unwrap_or(d) as f32;
        AngK {
            average: c("angling_average", 0.5),
            floor: c("angling_floor", 0.01),
            rad_to_deg: c("angling_rad_to_deg", 57.29578),
            max: c("angling_max", 180.0),
            min: c("angling_min", -180.0),
            wrap_down: c("angling_wrap_down", -360.0),
            wrap_up: c("angling_wrap_up", 360.0),
            fold_max: c("fold_max", 90.0),
            fold_min: c("fold_min", -90.0),
            fold_down: c("fold_down", -180.0),
            fold_up: c("fold_up", 180.0),
            // RequestRightUpperStrike rva=0x1564a00 -57.5, RequestRightLowerStrike rva=0x15649a0 60.0
            upper_strike: c("upper_strike_angle", -57.5),
            lower_strike: c("lower_strike_angle", 60.0),
        }
    }
}

/// UAttackMotion::IsLeft rva=0x162c140: (Move - 1) & 0xfd == 0
pub fn is_left(mv: i64) -> bool {
    ((mv - 1) & 0xfd) == 0
}

/// UAttackMotion::FlipSide rva=0x161dc80
pub fn flip_side(mv: i64) -> i64 {
    match mv {
        0 => 1,
        1 => 0,
        2 => 3,
        3 => 2,
        m => m,
    }
}

/// The sim's turn / look state for this frame (the character's AAdvancedCharacter fields)
#[derive(Clone, Copy, Debug, Default)]
pub struct CapView {
    /// sim fixed steps so far (pending turn resets when the sim consumed it)
    pub tick: u64,
    /// the actor's control yaw / LookUpValue the sim holds
    pub yaw: f32,
    pub look_up: f64,
    /// TurnRateCap (-1 = uncapped) / TurnCapRemaining, LookUpRateCap / LookUpCapRemaining
    pub turn_rate_cap: f64,
    pub turn_cap_remaining: f64,
    pub look_up_rate_cap: f64,
    pub look_up_cap_remaining: f64,
    /// LookUpLimit / LookDownLimit
    pub look_up_limit: f64,
    pub look_down_limit: f64,
}

/// One frame of raw input as action names and mouse counts
#[derive(Clone, Debug, Default)]
pub struct RawFrame {
    /// raw mouse counts: MouseX grows right, MouseY grows up (UE)
    pub mouse_x: f32,
    pub mouse_y: f32,
    /// action names pressed / released this frame (IE_Pressed / IE_Released)
    pub pressed: Vec<String>,
    pub released: Vec<String>,
    /// UWorld RealTimeSeconds
    pub now: f64,
    /// PlayerCameraManager->GetFOVAngle() (original POV scalar in degrees, vertical FOV in Mordhau).
    pub fov: f32,
    /// the frame's delta time (s)
    pub dt: f32,
}

/// The local player's control state.
#[derive(Resource, Debug, Clone, serde::Serialize)]
pub struct PlayerControl {
    pub enabled: bool,
    pub id: u32,
    /// control rotation, degrees (UE yaw; pitch = LookUpValue, + up): the sim's capped values + this frame's pending
    pub yaw: f32,
    pub pitch: f32,
    pub angling: Angling,
    pub k: AngK,
    pub frame: i64,
    pub third_person: bool,
    pub cursor_grabbed: bool,
    /// the in-match escape menu (BP_MainMenu) was up last frame
    pub menu_open: bool,
    #[serde(skip)]
    pub actions: HashMap<String, Vec<Key>>,
    #[serde(skip)]
    pub axes: HashMap<String, Vec<(Key, f32)>>,
    /// axis name -> (UE key name, scale): the mouse axes
    #[serde(skip)]
    pub axis_keys: HashMap<String, Vec<(String, f32)>>,
    #[serde(skip)]
    pub axis_config: HashMap<String, AxisProps>,
    pub settings: MordhauInputSettings,
    pub bindings_source: String,
    pub yaw_initialised: bool,
    /// APlayerController InputYawScale / InputPitchScale (BaseGame.ini)
    pub input_yaw_scale: f32,
    pub input_pitch_scale: f32,
    /// UInputSettings bEnableFOVScaling / FOVScale (DefaultInput.ini)
    pub fov_scaling: bool,
    pub fov_scale: f32,
    /// AMordhauCharacter bWants360Strike / bWants360Stab / Listen360FrameStart / Listen360FirstDetectedXInputFrame /
    /// Listen360FirstDetectedXTime / bWantsFlipAttackSide
    pub wants_strike: bool,
    pub wants_stab: bool,
    pub listen_start: i64,
    pub first_x_frame: i64,
    pub first_x_time: f64,
    pub flip_held: bool,
    /// turn / look-up applied since the sim's last step, and the cap budget left of it
    pub pend_yaw: f32,
    pub pend_pitch: f64,
    /// rust-combat r10 (marked addition): a script-set absolute look-up (degrees, + up) applied as this frame's pitch
    pub script_pitch: Option<f64>,
    pub yaw_budget: f64,
    pub pitch_budget: f64,
    pub last_tick: u64,
    /// EVD_SWG_002: the TurnRateCap / LookUpRateCap seen last frame (-1 = uncapped), to mirror SetTurnCaps' trim
    pub last_turn_cap: f64,
    pub last_look_cap: f64,
}

impl PlayerControl {
    pub fn new(k: AngK) -> PlayerControl {
        PlayerControl {
            enabled: false,
            id: 0,
            yaw: 0.0,
            pitch: 0.0,
            angling: Angling::default(),
            k,
            frame: 0,
            // first person: AMordhauPlayerController ctor rva 0x15b4840 sets LastCharacterCameraStyle = 1
            // (ECameraStyle 1 = first person: AMordhauCharacter::CameraStyleChanged rva 0x1532190 bIsFirstPerson =
            // CameraStyle == 1), applied to the pawn by OnAfterPossess_Implementation rva 0x15e2420 SetCameraStyle
            third_person: false,
            cursor_grabbed: false,
            menu_open: false,
            actions: HashMap::new(),
            axes: HashMap::new(),
            axis_keys: HashMap::new(),
            axis_config: HashMap::new(),
            settings: MordhauInputSettings::default(),
            bindings_source: String::new(),
            yaw_initialised: false,
            input_yaw_scale: 2.5,
            input_pitch_scale: -2.5,
            fov_scaling: true,
            fov_scale: 0.01111,
            wants_strike: false,
            wants_stab: false,
            listen_start: 0,
            first_x_frame: 0,
            first_x_time: 0.0,
            flip_held: false,
            pend_yaw: 0.0,
            pend_pitch: 0.0,
            script_pitch: None,
            yaw_budget: 0.0,
            pitch_budget: 0.0,
            last_tick: u64::MAX,
            last_turn_cap: -1.0,
            last_look_cap: -1.0,
        }
    }

    pub fn apply_bindings(&mut self, b: &Bindings) {
        self.actions = b.actions.iter().map(|(a, ks)| (a.clone(), ks.iter().filter_map(|k| ue_key(k)).collect())).collect();
        self.axes = b.axes.iter().map(|(a, ks)| (a.clone(), ks.iter().filter_map(|(k, s)| ue_key(k).map(|k| (k, *s))).collect())).collect();
        self.axis_keys = b.axes.clone();
        self.axis_config = b.axis_config.clone();
        self.settings = b.settings;
        self.bindings_source = b.source.clone();
    }

    /// An axis' value from the mouse: sum over its mouse keys of Scale x MassageAxisInput(key, raw)
    fn mouse_axis(&self, name: &str, f: &RawFrame) -> f32 {
        let Some(ks) = self.axis_keys.get(name) else { return 0.0 };
        let mut v = 0.0;
        for (k, scale) in ks {
            let raw = match k.as_str() {
                "MouseX" => f.mouse_x,
                "MouseY" => f.mouse_y,
                _ => continue,
            };
            if raw == 0.0 {
                continue;
            }
            let mut m = self.axis_config.get(k).map(|p| p.massage(raw)).unwrap_or(raw);
            if self.fov_scaling {
                m *= f.fov * self.fov_scale;
            }
            v += scale * m;
        }
        v
    }

    fn fire_strike360(&mut self, fi: &mut FrameInput) {
        // UMotionSystemComponent::RequestStrike360 rva=0x14d1a80: clears bWants360Strike (+0xe35), flushes, requests
        self.wants_strike = false;
        let k = self.k.clone();
        let (f, w, l) = (self.frame, self.wants_strike || self.wants_stab, self.listen_start);
        self.angling.flush(&k, f, w, l);
        fi.attack = Some(self.angling.attack(&k, false, self.flip_held, &self.settings));
    }

    fn fire_stab360(&mut self, fi: &mut FrameInput) {
        // RequestStab360 rva=0x14d1950: clears bWants360Stab (+0xe34)
        self.wants_stab = false;
        let k = self.k.clone();
        let (f, w, l) = (self.frame, self.wants_strike || self.wants_stab, self.listen_start);
        self.angling.flush(&k, f, w, l);
        fi.attack = Some(self.angling.attack(&k, true, self.flip_held, &self.settings));
    }

    /// AdjustPresetAttackAngleRequest rva=0x14b2d80 then AMordhauEquipment::RequestAttack(move, angle)
    fn preset(&self, mv: i64, angle: f32) -> (i64, f64) {
        let keep = if self.settings.mouse_x_is_flip_attack_side == 1 {
            // the mouse's angling side picks the side
            let x = self.angling.vector(&self.settings).0;
            let left = x <= 0.0 && x != 0.0;
            left == is_left(mv)
        } else {
            !self.flip_held
        };
        (if keep { mv } else { flip_side(mv) }, angle as f64)
    }

    /// One frame of the exe's input path (module header order). `fi` is the fighter's pending sim input.
    pub fn step(&mut self, f: &RawFrame, c: &CapView, fi: &mut FrameInput) {
        self.frame += 1;
        // the sim consumed the last pending turn: restart from its capped values and its remaining cap budget
        if c.tick != self.last_tick {
            self.last_tick = c.tick;
            self.pend_yaw = 0.0;
            self.pend_pitch = 0.0;
        }
        let has = |v: &Vec<String>, n: &str| v.iter().any(|x| x == n);
        let after_press = self.settings.angle_attack_after_press != 0;
        // ---- action delegates (SetupPlayerInputComponent rva=0x156c560)
        if has(&f.released, "Flip Attack Side") {
            self.flip_held = false; // FlipAttackSideReleased rva=0x153e8b0
        }
        if has(&f.pressed, "Flip Attack Side") {
            self.flip_held = true; // FlipAttackSidePressed rva=0x153e8a0
        }
        if has(&f.released, "Strike") && after_press && self.wants_strike {
            // StopListenForStrike360 rva=0x156ddf0 (UseAngleAttackAfterPress only): the strike goes on release
            self.fire_strike360(fi);
        }
        if has(&f.pressed, "Strike") {
            // ListenForStrike360 rva=0x154ecd0
            if after_press {
                self.listen_start = self.frame; // UMordhauUtilityLibrary::GetCurrentFrame
            }
            self.wants_stab = false;
            self.wants_strike = true;
        }
        if has(&f.pressed, "Stab") {
            // ListenForStab360 rva=0x154ecc0 (released: StopListenForStab360 rva=0x7bf350, a `ret`)
            self.wants_stab = true;
            self.wants_strike = false;
        }
        let presets: [(&str, i64, f32); 8] = [
            ("Right Stab", 2, 0.0),                        // RequestRightStab rva=0x15649c0
            ("Right Upper Strike", 0, self.k.upper_strike), // RequestRightUpperStrike rva=0x1564a00
            ("Right Strike", 0, 0.0),                       // RequestRightStrike rva=0x15649e0
            ("Right Lower Strike", 0, self.k.lower_strike), // RequestRightLowerStrike rva=0x15649a0
            ("Left Stab", 3, 0.0),                          // RequestLeftStab rva=0x1564800
            ("Left Upper Strike", 1, self.k.upper_strike),  // RequestLeftUpperStrike rva=0x1564840
            ("Left Strike", 1, 0.0),                        // RequestLeftStrike rva=0x1564820
            ("Left Lower Strike", 1, self.k.lower_strike),  // RequestLeftLowerStrike (RequestAttack(.., 60.0))
        ];
        for (name, mv, ang) in presets {
            if has(&f.pressed, name) {
                fi.attack = Some(self.preset(mv, ang));
            }
        }
        if has(&f.pressed, "Kick") {
            fi.attack = Some((4, 0.0)); // RequestKick rva=0x15647c0 (move 4, angle 0)
        }
        if has(&f.pressed, "Shove") {
            fi.attack = Some((5, 0.0)); // RequestBash rva=0x1564570: AMordhauEquipment::RequestAttack(5 Bash, 0.0)
        }
        if has(&f.pressed, "Feint") {
            fi.feint = true; // RequestFeint rva=0x15646b0 -> UMordhauMotion::ProcessFeint
        }
        if has(&f.pressed, "Parry") {
            // BlockPressed rva=0x1531650 -> RequestParry(0, true); retry = core wants_block. The swivel (left) parry:
            // AMordhauCharacter::RequestParry / UMotionSystemComponent::RequestParry rva=0x14d1590 machine code
            // 1414d1685-1414d16b1 (the decompile drops it): a player-controlled character with BlockType 0 and
            // GetAnglingVector().X < 0 (the mouse moving left; InverseAnglingX off) requests AltRegular (1) instead
            fi.parry = Some(if self.angling.vector(&self.settings).0 < 0.0 { 1 } else { 0 });
        }
        if has(&f.released, "Parry") {
            fi.release_block = true; // BlockReleased rva=0x15316a0
        }
        if has(&f.pressed, "Weapon Mode / Reload") {
            fi.toggle_mode = true; // ToggleWeaponModePressed -> RequestToggleWeaponMode rva=0x1564a60
        }
        // EVD_SWG_002: AAdvancedCharacter::SetTurnCaps rva=0x14a15f0 runs on the owning client too (the motion's
        // OnBegin / OnLeave): a cap set from -1 or below the current one trims TurnCapRemaining to min(Remaining, Cap x
        // 0.0333) (LODTick only raises the cap, so a new or lower cap is a SetTurnCaps). Without this the budget left
        // from the last attack's ramp (up to 1500 x 0.0333 = 50 deg) turned at once at the next attack's start.
        let trim = |cap: f64, last: f64, budget: &mut f64| {
            if cap != -1.0 && (last == -1.0 || cap < last) {
                *budget = budget.min(cap * 0.0333);
            }
        };
        trim(c.turn_rate_cap, self.last_turn_cap, &mut self.yaw_budget);
        trim(c.look_up_rate_cap, self.last_look_cap, &mut self.pitch_budget);
        self.last_turn_cap = c.turn_rate_cap;
        self.last_look_cap = c.look_up_rate_cap;
        // ---- axis delegates: "Turn Right" -> TurnNotAbsolute rva=0x14a8c80 -> Turn(Value, false)
        let turn = self.mouse_axis("Turn Right", f);
        let look = self.mouse_axis("Look Up", f);
        // Turn rva=0x14a8a90: RegisterAnglingXInput(InputYawScale * Value) (bTurnUsesControllerInputYawScale = 1,
        // AAdvancedCharacter ctor rva=0x144b0e0), then the TurnCaps clamp (excess dropped), TargetControlYaw += it
        self.angling.register_x(self.input_yaw_scale * turn);
        if turn != 0.0 {
            let mut d = (self.input_yaw_scale * turn) as f64;
            if c.turn_rate_cap != -1.0 {
                d = d.clamp(-self.yaw_budget, self.yaw_budget);
                self.yaw_budget = (self.yaw_budget - d.abs()).max(0.0);
            }
            self.pend_yaw += d as f32;
        }
        // LookUp rva=0x1489930: RegisterAnglingYInput(-(InputPitchScale * Value)); delta InputPitchScale * Value,
        // LookUpRateCap clamp, then clamp(-LookDownLimit, LookUpLimit) (m.MouseSmoothing 0: no FSmoothDamp)
        self.angling.register_y(-(self.input_pitch_scale * look));
        if look != 0.0 {
            let mut d = (self.input_pitch_scale * look) as f64;
            if c.look_up_rate_cap != -1.0 {
                d = d.clamp(-self.pitch_budget, self.pitch_budget);
                self.pitch_budget = (self.pitch_budget - d.abs()).max(0.0);
            }
            let cur = c.look_up + self.pend_pitch;
            let nv = (cur + d).clamp(-c.look_down_limit, c.look_up_limit);
            self.pend_pitch = nv - c.look_up;
        }
        if let Some(v) = self.script_pitch {
            // rust-combat r10: the offscreen script's `look` (the sim still applies LookUp's clamp)
            self.pend_pitch = v.clamp(-c.look_down_limit, c.look_up_limit) - c.look_up;
        }
        // ---- LODTick rva=0x154c390: FlushPendingAnglingInputs, then the 360 requests
        let k = self.k.clone();
        let wants = self.wants_strike || self.wants_stab;
        let (fr, ls) = (self.frame, self.listen_start);
        self.angling.flush(&k, fr, wants, ls);
        if wants {
            if after_press && !self.wants_stab {
                // UseAngleAttackAfterPress (mouse): wait for X input after the press, then Y input or 0.07 s
                if self.listen_start <= self.angling.frame_x {
                    if self.first_x_frame < self.listen_start {
                        self.first_x_frame = self.angling.frame_x;
                        self.first_x_time = f.now;
                    }
                    if self.listen_start <= self.angling.frame_y || self.first_x_time + 0.07 < f.now {
                        self.fire_strike360(fi);
                    }
                }
            } else if !self.wants_strike {
                self.fire_stab360(fi);
            } else {
                self.fire_strike360(fi);
            }
        }
        self.yaw = c.yaw + self.pend_yaw;
        self.pitch = (c.look_up + self.pend_pitch) as f32;
        fi.yaw = Some(self.yaw);
        fi.look_up = Some(c.look_up + self.pend_pitch);
        // the owning client's own TurnCapRemaining refill, every rendered frame after the input (APlayerController
        // input runs before the pawn's tick; AAdvancedCharacter::LODTick rva=0x14887b0 decomp 524-576: Remaining =
        // clamp(Cap x dt + Remaining, 0, Cap x 0.0333)); Cap / Target come from the sim's TurnCaps (SetTurnCaps at attack
        // begin / leave). The turn caps are an owning-client rule (Turn is an input function; the server takes the
        // control rotation from ServerMove), so the per-frame budget here is the authoritative one for the player.
        // Was (r1): refilled only when a 60 Hz sim step ran -> the 1P view turned in 16.7 ms steps while capped.
        let refill = |cap: f64, rem: &mut f64| {
            if cap != -1.0 {
                *rem = (cap * f.dt as f64 + *rem).clamp(0.0, cap * 0.0333);
            }
        };
        refill(c.turn_rate_cap, &mut self.yaw_budget);
        refill(c.look_up_rate_cap, &mut self.pitch_budget);
    }
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, (player_input, apply_saved_settings));
    }
}

/// the effective bindings: the saved Input.ini's UMordhauInput arrays over the paks' DefaultInput.ini (merge_bindings)
pub fn load_bindings(default_input_ini: &str) -> Bindings {
    let dflt = parse_section(default_input_ini, "/Script/Engine.InputSettings");
    let saved_path = saved_input_ini_path();
    let saved = saved_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_section(&t, "/Script/Mordhau.MordhauInput"));
    let saved = saved.filter(|s| !s.actions.is_empty() || !s.axes.is_empty() || !s.axis_config.is_empty() || !s.settings.is_empty());
    let mut b = merge_bindings(saved.as_ref(), &dflt);
    b.source = match (&saved, &saved_path) {
        (Some(_), Some(p)) => format!("saved {} [/Script/Mordhau.MordhauInput] + paks DefaultInput.ini", p.display()),
        _ => "paks: DefaultInput.ini [/Script/Engine.InputSettings]".into(),
    };
    b
}

/// (rust-ui r3) Settings applied in the menu take effect at once, as in the game: UMordhauInput::ApplySettings ->
/// ForceRebuildKeymaps rva=0x1594240 rebuilds the player's keymaps from the new arrays (here: the Input.ini mh-ui
/// just saved), and UGameUserSettings::ApplySettings -> ApplyNonResolutionSettings rva=0x1582870 pushes FOV / gore /
/// head bob into the m.* cvars (here: the UserSettings resource re-read from GameUserSettings.ini)
fn apply_saved_settings(mut ev: MessageReader<mh_ui::UiSettingsApplied>, src: Res<crate::source::Source>, pc: Option<ResMut<PlayerControl>>, mut commands: Commands) {
    let (mut input, mut game) = (false, false);
    for e in ev.read() {
        if e.input {
            input = true;
        } else {
            game = true;
        }
    }
    if input {
        if let (Some(mut pc), Some(vfs)) = (pc, &src.vfs) {
            if let Some(text) = mh_level::config::text(vfs, "DefaultInput.ini") {
                let b = load_bindings(&text);
                pc.apply_bindings(&b);
                info!("input: keymaps rebuilt from {}", b.source);
            }
        }
    }
    if game {
        commands.insert_resource(crate::usersettings::UserSettings::load());
        info!("settings: game user settings re-applied");
    }
}

fn setup(mut commands: Commands, src: Res<crate::source::Source>, spec: Option<Res<SpecData>>) {
    let mut pc = PlayerControl::new(AngK::from_spec(spec.as_deref()));
    if let Some(vfs) = &src.vfs {
        if let Some(text) = mh_level::config::text(vfs, "DefaultInput.ini") {
            let b = load_bindings(&text);
            pc.apply_bindings(&b);
            let fv = |k: &str, d: f64| mh_level::config::float_value(vfs, "DefaultInput.ini", "/Script/Engine.InputSettings", k).unwrap_or(d);
            pc.fov_scale = fv("FOVScale", 0.01111) as f32;
            pc.fov_scaling = !mh_level::config::value(vfs, "DefaultInput.ini", "/Script/Engine.InputSettings", "bEnableFOVScaling").eq_ignore_ascii_case("false");
            pc.input_yaw_scale = mh_level::config::float_value(vfs, "BaseGame.ini", "/Script/Engine.PlayerController", "InputYawScale").unwrap_or(2.5) as f32;
            pc.input_pitch_scale = mh_level::config::float_value(vfs, "BaseGame.ini", "/Script/Engine.PlayerController", "InputPitchScale").unwrap_or(-2.5) as f32;
        }
    }
    commands.insert_resource(pc);
}

fn down(k: &Key, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
    match k {
        // Both physical Return keys are saved as EKeys::Enter by the controls screen.
        Key::K(KeyCode::Enter) => keys.pressed(KeyCode::Enter) || keys.pressed(KeyCode::NumpadEnter),
        Key::K(c) => keys.pressed(*c),
        Key::M(b) => mouse.pressed(*b),
        _ => false,
    }
}
fn pressed(k: &Key, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>, wheel: (bool, bool)) -> bool {
    match k {
        Key::K(KeyCode::Enter) => keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter),
        Key::K(c) => keys.just_pressed(*c),
        Key::M(b) => mouse.just_pressed(*b),
        Key::WheelUp => wheel.0,
        Key::WheelDown => wheel.1,
    }
}
fn released(k: &Key, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>, wheel: (bool, bool)) -> bool {
    match k {
        Key::K(KeyCode::Enter) => (keys.just_released(KeyCode::Enter) || keys.just_released(KeyCode::NumpadEnter)) && !down(k, keys, mouse),
        Key::K(c) => keys.just_released(*c),
        Key::M(b) => mouse.just_released(*b),
        // a wheel notch is a press and a release in the same frame (FKey MouseScrollUp/Down)
        Key::WheelUp => wheel.0,
        Key::WheelDown => wheel.1,
    }
}

#[allow(clippy::too_many_arguments)]
/// Script `real_input` (offscreen evidence runs): read the input messages without a window, cursor grabbed
#[derive(Resource)]
pub struct HeadlessRealInput;

/// UPlayerInput::MassageAxisInput uses the camera-manager POV scalar, not horizontal-FOV conversion.
fn mouse_input_fov(p: &Projection) -> Option<f32> {
    match p {
        Projection::Perspective(p) => Some(p.fov.to_degrees()),
        _ => None,
    }
}

pub fn player_input(
    mut pc: ResMut<PlayerControl>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    motion: Option<Res<AccumulatedMouseMotion>>,
    mut wheel_ev: MessageReader<MouseWheel>,
    mut inputs: ResMut<Inputs>,
    sim: NonSend<crate::sim::Sim>,
    mut cursor: Query<&mut bevy::window::CursorOptions>,
    flycam: Res<crate::camera::CamMode>,
    cams: Query<&Projection, With<crate::camera::FlyCam>>,
    time: Res<Time<Real>>,
    screen: Option<Res<mh_ui::Screen>>,
    rt: Option<NonSend<mh_ui::real::Rt>>,
    headless_input: Option<Res<HeadlessRealInput>>,
    dev: Option<Res<crate::devmenu::DevMenu>>,
) {
    let (Some(keys), Some(mouse), Some(motion)) = (keys, mouse, motion) else { return };
    let views = sim.0.fighters();
    let Some(me) = views.iter().find(|v| v.id == pc.id) else {
        // no pawn yet (the front end, a match loading): remember whether a menu is up, so the first frame of play after
        // it (Start Match from the front end) captures the cursor as the game's input mode change does
        // (BP_MordhauHUD ReceiveBeginPlay@550 CustomSetInputModeGameOnly)
        pc.menu_open = screen.as_deref().is_some_and(|s| *s == mh_ui::Screen::MainMenu) || rt.as_ref().is_some_and(|r| r.ui.main_menu_visible()) || pc.menu_open;
        return;
    };
    if !pc.yaw_initialised {
        pc.yaw = me.yaw;
        pc.yaw_initialised = true;
    }
    pc.enabled = !matches!(*flycam, crate::camera::CamMode::Fly);
    // no window (--offscreen / --headless): the script's move / input verbs drive the fighter, nothing to read here
    // (script verb `real_input`, evidence runs only: an offscreen run reads the script's Bevy key / mouse messages here
    // as from a window with the cursor grabbed, so the real input path is tested without a window on the user's screen)
    let headless = cursor.iter().next().is_none();
    if !pc.enabled || (headless && headless_input.is_none()) {
        if headless {
            // offscreen scripted replay (`drive`): no input is read here, so the control rotation the camera uses is
            // the sim's own (the script's yaw / look went straight to the sim); was left at the spawn values, which
            // kept the 1P camera pitch at 0 while the pose followed the look (camera1p gauntlet 2026-10-07)
            pc.yaw = me.yaw;
            pc.pitch = me.look_up;
        }
        return;
    }
    if headless {
        pc.cursor_grabbed = true;
    }
    // cursor: grabbed while playing; Escape releases, a click grabs again (runtime glue, not game behaviour). While the
    // game's front end shows a menu (rust-ui: Screen::MainMenu, or BP_MainMenu up as the pause menu in a match) the
    // cursor is free, as the game's UI input mode shows it
    let ui_menu = screen.as_deref().is_some_and(|s| *s == mh_ui::Screen::MainMenu) || rt.as_ref().is_some_and(|r| r.ui.main_menu_visible());
    // the Development menu (devmenu.rs, F9): the cursor is free and the pawn gets no input while it is open
    let ui_menu = ui_menu || dev.as_ref().is_some_and(|d| d.open);
    // The escape menu: the game shows it in the UI input mode (cursor free, no pawn input) and closing it (Return,
    // Escape again) goes back to the game input mode with the cursor captured. Without the in-match UI (no Rt) Escape
    // only frees the cursor and a click captures it again (runtime glue).
    let had_menu = pc.menu_open;
    pc.menu_open = ui_menu;
    if ui_menu || (keys.just_pressed(KeyCode::Escape) && rt.is_none()) {
        pc.cursor_grabbed = false;
    } else if had_menu || mouse.just_pressed(MouseButton::Left) {
        pc.cursor_grabbed = true;
    }
    for mut c in cursor.iter_mut() {
        c.grab_mode = if pc.cursor_grabbed { bevy::window::CursorGrabMode::Locked } else { bevy::window::CursorGrabMode::None };
        c.visible = !pc.cursor_grabbed;
    }
    if ui_menu || had_menu {
        // menu up (or closed by this frame's click / key): the input belongs to the UI, the pawn gets none
        for _ in wheel_ev.read() {}
        return;
    }
    let (mut wu, mut wd) = (false, false);
    for w in wheel_ev.read() {
        let y = match w.unit {
            MouseScrollUnit::Line | MouseScrollUnit::Pixel => w.y,
        };
        if y > 0.0 {
            wu = true;
        } else if y < 0.0 {
            wd = true;
        }
    }
    // raw counts (Bevy AccumulatedMouseMotion = the OS raw device delta, as UE's WM_INPUT path); UE MouseY grows up
    let d = if pc.cursor_grabbed { motion.delta } else { Vec2::ZERO };
    let pr = |k: &Key| pressed(k, &keys, &mouse, (wu, wd));
    let rl = |k: &Key| released(k, &keys, &mouse, (wu, wd));
    let dn = |k: &Key| down(k, &keys, &mouse);
    let names = |f: &dyn Fn(&Key) -> bool| -> Vec<String> { pc.actions.iter().filter(|(_, ks)| ks.iter().any(|k| f(k))).map(|(a, _)| a.clone()).collect() };
    // MassageAxisInput rva0x353d740 uses GetFOVAngle (rva0x333f230), the original POV scalar.
    // CameraPlugin already stores Mordhau's vertical FOV in pp.fov; no aspect-ratio conversion belongs here.
    let fov = cams
        .iter()
        .next()
        .and_then(mouse_input_fov)
        .unwrap_or(90.0);
    let raw = RawFrame { mouse_x: d.x, mouse_y: -d.y, pressed: names(&pr), released: names(&rl), now: time.elapsed_secs_f64(), fov, dt: time.delta_secs() };
    let axis = |pc: &PlayerControl, name: &str| -> f32 {
        pc.axes.get(name).map(|ks| ks.iter().filter(|(k, _)| dn(k)).map(|(_, s)| *s).sum::<f32>().clamp(-1.0, 1.0)).unwrap_or(0.0)
    };
    if raw.pressed.iter().any(|a| a == "Cycle Camera") {
        pc.third_person = !pc.third_person;
    }
    let mut cap = CapView { tick: sim.0.ticks(), yaw: me.yaw, turn_rate_cap: -1.0, look_up_rate_cap: -1.0, look_up_limit: 89.0, look_down_limit: 89.0, ..Default::default() };
    if let (Some(w), Some(fi)) = (sim.0.combat(), sim.0.fighter_index(pc.id)) {
        if let Some(f) = w.fighters.get(fi) {
            let t = &f.turn_caps;
            cap.turn_rate_cap = t.turn_rate_cap;
            cap.turn_cap_remaining = t.turn_cap_remaining;
            cap.look_up_rate_cap = t.look_up_rate_cap;
            cap.look_up_cap_remaining = t.look_up_cap_remaining;
            cap.look_up = f.look_up_value;
            cap.look_up_limit = f.character.look_up_limit;
            cap.look_down_limit = f.character.look_down_limit;
        }
    }
    let mut fi = inputs.0.remove(&pc.id).unwrap_or_default();
    fi.fwd = axis(&pc, "Move Forward");
    fi.right = axis(&pc, "Move Right");
    fi.sprint = pc.actions.get("Sprint").is_some_and(|ks| ks.iter().any(|k| dn(k)));
    fi.crouch = pc.actions.get("Crouch").is_some_and(|ks| ks.iter().any(|k| dn(k)));
    fi.jump |= raw.pressed.iter().any(|a| a == "Jump");
    pc.step(&raw, &cap, &mut fi);
    inputs.0.insert(pc.id, fi);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_parse_and_keys() {
        let t = "+ActionMappings=(ActionName=\"Strike\",bShift=False,Key=LeftMouseButton)\n+ActionMappings=(ActionName=\"Sprint\",Key=None)\n+AxisMappings=(AxisName=\"Move Right\",Scale=-1.000000,Key=A)\n+AxisConfig=(AxisKeyName=\"MouseX\",AxisProperties=(DeadZone=0.000000,Sensitivity=0.070000,Exponent=1.000000,bInvert=False))";
        let (a, x, s) = parse_input_ini(t);
        assert_eq!(a["Strike"], ["LeftMouseButton"]);
        assert!(!a.contains_key("Sprint"));
        assert_eq!(x["Move Right"], [("A".to_string(), -1.0)]);
        assert!((s["MouseX"] - 0.07).abs() < 1e-6);
        assert_eq!(ue_key("Q"), Some(Key::K(KeyCode::KeyQ)));
        assert_eq!(ue_key("MouseScrollUp"), Some(Key::WheelUp));
        assert_eq!(ue_key("Five"), Some(Key::K(KeyCode::Digit5)));
    }

    #[test]
    fn angling_picks_side_and_fold() {
        let k = AngK::from_spec(None);
        let mut a = Angling::default();
        a.register(-1.0, 0.0); // mouse left
        a.flush(&k, 1, false, 0);
        let (mv, ang) = a.attack(&k, false, false, &MordhauInputSettings::default());
        assert_eq!(mv, 1); // left strike
        assert!(ang.abs() < 1e-3, "{ang}");
        let mut b = Angling::default();
        b.register(1.0, 1.0); // up-right: 45 deg
        b.flush(&k, 1, false, 0);
        let (mv, ang) = b.attack(&k, false, false, &MordhauInputSettings::default());
        assert_eq!(mv, 0);
        assert!((ang - 45.0).abs() < 1e-3, "{ang}");
        // bWantsFlipAttackSide inverts the side (RequestStrike360 rva=0x14d1a80)
        assert_eq!(b.attack(&k, false, true, &MordhauInputSettings::default()).0, 1);
    }

    /// RequestStab360 applies axis inversion before choosing the side, folds the angle, then applies held Flip.
    /// Expected angles here are analytic atan(1/2), with room only for the original minimax approximation.
    #[test]
    fn stab_selector_applies_saved_axis_inversion_and_flip() {
        let k = AngK::from_spec(None);
        for (ix, iy, flip, want_move, want_angle) in [
            (0, 0, false, 2, 26.56505), (1, 0, false, 3, 26.56505),
            (0, 1, false, 2, -26.56505), (1, 1, false, 3, -26.56505),
            (0, 0, true, 3, -26.56505), (1, 0, true, 2, -26.56505),
        ] {
            let settings = MordhauInputSettings { inverse_attack_direction_x: ix,
                inverse_attack_direction_y: iy, ..Default::default() };
            let a = Angling { x: 0.8, y: 0.4, ..Default::default() };
            let (mv, angle) = a.attack(&k, true, flip, &settings);
            assert_eq!(mv, want_move, "inverse {ix}/{iy}, flip {flip}");
            assert!((angle - want_angle).abs() < 0.0002, "{angle} vs {want_angle}");
            assert_eq!((a.x, a.y), (0.8, 0.4), "inversion must not rewrite accumulation history");
            assert_eq!(Angling::default().angle(&k, &settings).to_bits(), 0.0f32.to_bits());
        }
    }

    #[test]
    fn mouse_scaling_uses_original_camera_fov_independent_of_aspect() {
        for aspect in [4.0 / 3.0, 16.0 / 9.0, 21.0 / 9.0] {
            let p = Projection::Perspective(PerspectiveProjection {
                fov: 82.0f32.to_radians(), aspect_ratio: aspect, ..Default::default()
            });
            let fov = mouse_input_fov(&p).unwrap();
            assert!((fov - 82.0).abs() < 0.00001);
            let mut pc = pc_with(Some(SAVED));
            let mut f = frame(1000.0, 0.0, &[], &[]);
            f.fov = fov;
            let mut fi = FrameInput::default();
            pc.step(&f, &uncapped(0), &mut fi);
            let expected = 1000.0f32 * 0.00035 * (82.0 * 0.01111) * 2.5;
            assert!((fi.yaw.unwrap() - expected).abs() < 0.00001);
        }
    }

    #[test]
    fn saved_inversion_reaches_real_stab_preset_and_parry_paths() {
        let saved = format!("{SAVED}\nInverseAttackDirectionX=1\nInverseAttackDirectionY=1\n");
        let mut pc = pc_with(Some(&saved));
        assert_eq!(pc.settings.inverse_attack_direction_x, 1);
        let mut fi = FrameInput::default();
        pc.step(&frame(1000.0, 500.0, &["Stab"], &[]), &uncapped(0), &mut fi);
        assert_eq!(fi.attack.unwrap().0, 3, "positive raw X is an inverted left stab");
        pc.settings.mouse_x_is_flip_attack_side = 1;
        assert_eq!(pc.preset(0, -57.5), (1, -57.5), "preset uses the same queried side");
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 0.0, &["Parry"], &[]), &uncapped(1), &mut fi);
        assert_eq!(fi.parry, Some(1), "parry queries the same inverted X");
    }

    // ---- the real input path (PlayerControl::step) with the shipped / saved config shapes
    const DEFAULT: &str = "[/Script/Engine.InputSettings]\n\
        +AxisConfig=(AxisKeyName=\"MouseX\",AxisProperties=(DeadZone=0.000000,Sensitivity=0.070000,Exponent=1.000000,bInvert=False))\n\
        +AxisConfig=(AxisKeyName=\"MouseY\",AxisProperties=(DeadZone=0.000000,Sensitivity=0.070000,Exponent=1.000000,bInvert=False))\n\
        +ActionMappings=(ActionName=\"Strike\",bShift=False,bCtrl=False,bAlt=False,bCmd=False,Key=LeftMouseButton)\n\
        +ActionMappings=(ActionName=\"Stab\",bShift=False,bCtrl=False,bAlt=False,bCmd=False,Key=MouseScrollUp)\n\
        +ActionMappings=(ActionName=\"Use\",bShift=False,bCtrl=False,bAlt=False,bCmd=False,Key=E)\n\
        +AxisMappings=(AxisName=\"Turn Right\",Scale=1.000000,Key=MouseX)\n\
        +AxisMappings=(AxisName=\"Look Up\",Scale=-1.000000,Key=MouseY)\n";
    // the shape of the player's saved WindowsClient/Input.ini (keys without '+')
    const SAVED: &str = "[/Script/Mordhau.MordhauInput]\n\
        AxisConfig=(AxisKeyName=\"MouseX\",AxisProperties=(DeadZone=0.000000,Sensitivity=0.000350,Exponent=1.000000,bInvert=False))\n\
        AxisConfig=(AxisKeyName=\"MouseY\",AxisProperties=(DeadZone=0.000000,Sensitivity=0.000350,Exponent=1.000000,bInvert=False))\n\
        ActionMappings=(ActionName=\"Strike\",bShift=False,bCtrl=False,bAlt=False,bCmd=False,Key=None)\n\
        ActionMappings=(ActionName=\"Right Strike\",bShift=False,bCtrl=False,bAlt=False,bCmd=False,Key=LeftMouseButton)\n\
        ActionMappings=(ActionName=\"Flip Attack Side\",bShift=False,bCtrl=False,bAlt=False,bCmd=False,Key=LeftAlt)\n\
        AxisMappings=(AxisName=\"Turn Right\",Scale=1.000000,Key=MouseX)\n\
        AxisMappings=(AxisName=\"Look Up\",Scale=-1.000000,Key=MouseY)\n\
        MouseXIsFlipAttackSide=0\nAngleAttackAfterPress=0\nMouseXSensitivity=0.000350\nMouseYSensitivity=0.000350\n";

    fn pc_with(saved: Option<&str>) -> PlayerControl {
        let d = parse_section(DEFAULT, "/Script/Engine.InputSettings");
        let s = saved.map(|t| parse_section(t, "/Script/Mordhau.MordhauInput"));
        let mut pc = PlayerControl::new(AngK::from_spec(None));
        pc.apply_bindings(&merge_bindings(s.as_ref(), &d));
        pc
    }
    fn uncapped(tick: u64) -> CapView {
        CapView { tick, turn_rate_cap: -1.0, look_up_rate_cap: -1.0, look_up_limit: 70.0, look_down_limit: 60.0, ..Default::default() }
    }
    fn frame(mx: f32, my: f32, p: &[&str], r: &[&str]) -> RawFrame {
        RawFrame { mouse_x: mx, mouse_y: my, pressed: p.iter().map(|s| s.to_string()).collect(), released: r.iter().map(|s| s.to_string()).collect(), now: 0.0, fov: 110.0, dt: 0.0 }
    }

    /// saved arrays replace the defaults (ForceRebuildKeymaps rva=0x1594240); missing names come from the defaults
    /// (UMordhauInput::Tick rva=0x15ac320); MouseXSensitivity applies (ApplyAxisConfig rva=0x1582630)
    #[test]
    fn saved_bindings_replace_defaults() {
        let pc = pc_with(Some(SAVED));
        assert!(pc.actions["Strike"].is_empty(), "the saved Strike=None wins over the default LMB");
        assert_eq!(pc.actions["Right Strike"], [Key::M(MouseButton::Left)]);
        assert_eq!(pc.actions["Stab"], [Key::WheelUp], "Stab is missing from the saved list: appended from defaults");
        assert_eq!(pc.actions["Use"], [Key::K(KeyCode::KeyE)]);
        assert_eq!(pc.axis_config["MouseX"].sensitivity, 0.00035);
        assert_eq!(pc.settings.mouse_x_is_flip_attack_side, 0);
        assert_eq!(pc.settings.angle_attack_after_press, 0);
        // no saved file: UInputSettings + RestoreDefaultSettings rva=0x15a6fb0 settings
        let pc = pc_with(None);
        assert_eq!(pc.actions["Strike"], [Key::M(MouseButton::Left)]);
        assert_eq!(pc.settings, MordhauInputSettings::default());
    }

    /// MassageAxisInput rva=0x353d740 x FOV scaling, Turn rva=0x14a8a90 x InputYawScale 2.5: 1000 counts at
    /// Sensitivity 0.00035, FOV 110, FOVScale 0.01111 = 1000 * 0.00035 * 110 * 0.01111 * 2.5 = 1.0692838 deg
    #[test]
    fn mouse_turn_matches_exe_scaling() {
        let mut pc = pc_with(Some(SAVED));
        pc.fov_scale = 0.01111;
        let mut fi = FrameInput::default();
        pc.step(&frame(1000.0, 0.0, &[], &[]), &uncapped(0), &mut fi);
        let want = 1000.0f32 * 0.00035 * (110.0 * 0.01111) * 2.5;
        assert!((fi.yaw.unwrap() - want).abs() < 1e-5, "{:?} vs {want}", fi.yaw);
        // angling X registered as InputYawScale x Value (flushed this frame: first input, sign change -> set)
        assert!((pc.angling.x - want).abs() < 1e-5);
        // mouse up (+MouseY) looks up: "Look Up" Scale -1, InputPitchScale -2.5 (LookUp rva=0x1489930)
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 1000.0, &[], &[]), &uncapped(1), &mut fi);
        assert!(fi.look_up.unwrap() > 0.0);
        // LookUpLimit clamp
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 1.0e7, &[], &[]), &uncapped(2), &mut fi);
        assert_eq!(fi.look_up.unwrap(), 70.0);
    }

    /// Turn rva=0x14a8a90 with a TurnRateCap: each input is clamped to TurnCapRemaining and the excess is dropped (not
    /// carried into later frames); the budget comes back only as the sim refills it
    #[test]
    fn turn_cap_drops_excess() {
        let mut pc = pc_with(Some(SAVED));
        let cap = CapView { tick: 5, turn_rate_cap: 200.0, turn_cap_remaining: 0.5, ..uncapped(5) };
        pc.yaw_budget = 0.5;
        let mut fi = FrameInput::default();
        pc.step(&frame(5000.0, 0.0, &[], &[]), &cap, &mut fi);
        assert!((fi.yaw.unwrap() - 0.5).abs() < 1e-6, "{:?}", fi.yaw);
        // same sim step, more input: the budget is spent
        pc.step(&frame(5000.0, 0.0, &[], &[]), &cap, &mut fi);
        assert!((fi.yaw.unwrap() - 0.5).abs() < 1e-6);
        // the sim stepped (yaw now 0.5, budget 0.3): no leftover from the dropped input
        let cap2 = CapView { tick: 6, yaw: 0.5, turn_rate_cap: 200.0, turn_cap_remaining: 0.3, ..uncapped(6) };
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 0.0, &[], &[]), &cap2, &mut fi);
        assert!((fi.yaw.unwrap() - 0.5).abs() < 1e-6, "{:?}", fi.yaw);
        // LODTick rva=0x14887b0 refill every rendered frame: 200 deg/s x 1/240 s = 0.8333, capped at 200 x 0.0333 = 6.66
        let mut f = frame(0.0, 0.0, &[], &[]);
        f.dt = 1.0 / 240.0;
        pc.step(&f, &cap2, &mut fi);
        assert!((pc.yaw_budget - 200.0 / 240.0).abs() < 1e-6, "{}", pc.yaw_budget);
        for _ in 0..100 {
            pc.step(&f, &cap2, &mut fi);
        }
        assert!((pc.yaw_budget - 200.0 * 0.0333).abs() < 1e-6);
    }

    /// EVD_SWG_002: the greatsword strike's TurnCaps (245, 171.5; BP_Greatsword StrikeAttack) against the reader record's
    /// yaw envelope (everything.json ctrl yaw by motion stage, 0.25 s windows: greatsword strikes max 266-275 deg/s,
    /// release p95 206-248; the game ran at 240 fps, anim LastDeltaSeconds 0.00417). A flick held through the attack at
    /// 240 Hz: the turn starts from the trimmed budget (SetTurnCaps: min(stale 50, 245 x 0.0333 = 8.16)), then the
    /// LODTick refill (after the frame's input) allows 245 / 240 per frame: a 0.25 s window peaks at (8.16 + 59 x 245 /
    /// 240) / 0.25 = 273.6 (the record's 275.4 within its sample-time jitter) and settles at 245. Untrimmed, the stale
    /// 50 deg budget would put the first window at 441.
    #[test]
    fn strike_turn_cap_matches_the_real_envelope() {
        let mut pc = pc_with(Some(SAVED));
        pc.yaw_budget = 50.0; // left over from the previous attack's ramp (cap 1500 x 0.0333)
        let (mut yaw, mut ys) = (0.0f32, vec![0.0f32]);
        let mut f = frame(1.0e5, 0.0, &[], &[]);
        f.dt = 1.0 / 240.0;
        for tick in 0..240u64 {
            let cap = CapView { tick, yaw, turn_rate_cap: 245.0, turn_cap_remaining: 0.0, ..uncapped(tick) };
            let mut fi = FrameInput::default();
            pc.step(&f, &cap, &mut fi);
            yaw = fi.yaw.unwrap();
            ys.push(yaw);
        }
        let w = 60; // 0.25 s
        let rates: Vec<f32> = (w..ys.len()).map(|i| (ys[i] - ys[i - w]) / 0.25).collect();
        let peak = rates.iter().cloned().fold(0.0, f32::max);
        let last = *rates.last().unwrap();
        println!("0.25 s window: peak {peak} deg/s, settled {last}");
        assert!((peak - 273.6).abs() < 0.2, "peak {peak}, want 273.6 (record 275.4)");
        assert!((last - 245.0).abs() < 0.5, "settled {last}");
    }

    /// The user's LMB "Right Strike" + LeftAlt "Flip Attack Side": AdjustPresetAttackAngleRequest rva=0x14b2d80 flips
    /// RightStrike to LeftStrike (FlipSide rva=0x161dc80) while flip is held (m.MouseXIsFlipAttackSide 0)
    #[test]
    fn preset_strike_and_flip() {
        let mut pc = pc_with(Some(SAVED));
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 0.0, &["Right Strike"], &[]), &uncapped(0), &mut fi);
        assert_eq!(fi.attack, Some((0, 0.0)));
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 0.0, &["Flip Attack Side", "Right Strike"], &[]), &uncapped(1), &mut fi);
        assert_eq!(fi.attack, Some((1, 0.0)));
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 0.0, &["Right Strike"], &["Flip Attack Side"]), &uncapped(2), &mut fi);
        assert_eq!(fi.attack, Some((0, 0.0)));
        // m.MouseXIsFlipAttackSide 1: the side follows the angling X (mouse moved left -> left strike)
        pc.settings.mouse_x_is_flip_attack_side = 1;
        let mut fi = FrameInput::default();
        pc.step(&frame(-100.0, 0.0, &[], &[]), &uncapped(3), &mut fi);
        pc.step(&frame(0.0, 0.0, &["Right Strike"], &[]), &uncapped(4), &mut fi);
        assert_eq!(fi.attack.unwrap().0, 1);
    }

    /// Keybind capture -> saved FKey name -> runtime keymap preserves controls-screen keyboard bindings.
    /// This exercises the actual cross-crate conversion, not a hand-written list of expected UE names.
    #[test]
    fn controls_screen_keyboard_bindings_reach_runtime() {
        use KeyCode::*;
        let keys = [Escape, Enter, NumpadEnter, Space, Tab, Backspace, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, ShiftLeft,
            ShiftRight, ControlLeft, AltLeft, Delete, Home, End, PageUp, PageDown, Backquote, F1, F10, KeyA, KeyZ,
            Digit0, Digit9];
        for physical in keys {
            let name = mh_ui::real::ue_key(physical).expect("key accepted by controls UI");
            let saved = format!("[/Script/Mordhau.MordhauInput]\nActionMappings=(ActionName=\"Flip Attack Side\",Key={name})\nMouseXIsFlipAttackSide=0\n");
            let mut pc = pc_with(Some(&saved));
            let canonical = if physical == NumpadEnter { Enter } else { physical };
            assert_eq!(pc.actions["Flip Attack Side"], vec![Key::K(canonical)], "UI binding {name} was dropped");
            let mut keys = ButtonInput::<KeyCode>::default();
            keys.press(physical);
            let mouse = ButtonInput::<MouseButton>::default();
            let bound = &pc.actions["Flip Attack Side"][0];
            assert!(pressed(bound, &keys, &mouse, (false, false)));
            let mut fi = FrameInput::default();
            pc.step(&frame(0.0, 0.0, &["Flip Attack Side", "Right Strike"], &[]), &uncapped(0), &mut fi);
            assert_eq!(fi.attack, Some((1, 0.0)), "{name} must flip a right strike");
        }
    }

    #[test]
    fn enter_binding_stays_held_until_both_physical_keys_release() {
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        let enter = Key::K(KeyCode::Enter);
        keys.press(KeyCode::Enter);
        keys.press(KeyCode::NumpadEnter);
        assert!(pressed(&enter, &keys, &mouse, (false, false)));
        keys.clear();
        keys.release(KeyCode::Enter);
        assert!(down(&enter, &keys, &mouse));
        assert!(!released(&enter, &keys, &mouse, (false, false)));
        keys.clear();
        keys.release(KeyCode::NumpadEnter);
        assert!(!down(&enter, &keys, &mouse));
        assert!(released(&enter, &keys, &mouse, (false, false)));
    }

    /// the swivel parry (UMotionSystemComponent::RequestParry rva=0x14d1590, machine code 1414d1685-1414d16b1): a parry
    /// pressed while the angling vector points left (mouse moved left) requests AltRegular, the left parry
    #[test]
    fn swivel_left_requests_the_left_parry() {
        for (mx, want) in [(-100.0, 1), (100.0, 0), (0.0, 0)] {
            let mut pc = pc_with(Some(SAVED));
            let mut fi = FrameInput::default();
            pc.step(&frame(mx, 0.0, &[], &[]), &uncapped(0), &mut fi);
            let mut fi = FrameInput::default();
            pc.step(&frame(0.0, 0.0, &["Parry"], &[]), &uncapped(1), &mut fi);
            assert_eq!(fi.parry, Some(want), "mouse x {mx}: parry request {:?}", fi.parry);
        }
    }

    /// ListenForStrike360 rva=0x154ecd0 + LODTick rva=0x154c390 (AngleAttackAfterPress 0): the strike goes on the
    /// press frame, after the flush; while bWants360Strike is set the pending input is summed, not averaged
    #[test]
    fn strike360_on_press_frame() {
        let mut pc = pc_with(None);
        pc.settings.angle_attack_after_press = 0;
        let mut fi = FrameInput::default();
        pc.step(&frame(100.0, 0.0, &[], &[]), &uncapped(0), &mut fi); // angling X = a
        let a = pc.angling.x;
        let mut fi = FrameInput::default();
        pc.step(&frame(100.0, 0.0, &["Strike"], &[]), &uncapped(1), &mut fi);
        assert_eq!(fi.attack.map(|x| x.0), Some(0));
        assert!((pc.angling.x - 2.0 * a).abs() < 1e-6, "summed while listening (listen frame 0 <= AnglingXFrame)");
        assert!(!pc.wants_strike, "RequestStrike360 clears bWants360Strike");
        // up-left mouse -> left strike, angle negated (RequestStrike360 rva=0x14d1a80)
        let mut pc = pc_with(None);
        pc.settings.angle_attack_after_press = 0;
        let mut fi = FrameInput::default();
        pc.step(&frame(-100.0, 100.0, &["Strike"], &[]), &uncapped(0), &mut fi);
        let (mv, ang) = fi.attack.unwrap();
        assert_eq!(mv, 1);
        // mouse up makes AnglingY negative (LookUp registers -(InputPitchScale x Value), "Look Up" = -MouseY): atan2(-, -)
        // = -135 -> fold 45 -> left side negates: -45 (negative = upper, as RequestRightUpperStrike -57.5)
        assert!((ang + 45.0).abs() < 1e-3, "{ang}");
    }

    /// AngleAttackAfterPress 1 (RestoreDefaultSettings): the strike waits for X input after the press, then fires on Y
    /// input or 0.07 s after the first X input (LODTick rva=0x154c390); a release before that fires it
    /// (StopListenForStrike360 rva=0x156ddf0)
    #[test]
    fn strike360_after_press() {
        let mut pc = pc_with(None);
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 0.0, &["Strike"], &[]), &uncapped(0), &mut fi);
        assert!(fi.attack.is_none() && pc.wants_strike);
        let mut f = frame(100.0, 0.0, &[], &[]);
        f.now = 1.0;
        pc.step(&f, &uncapped(1), &mut fi);
        assert!(fi.attack.is_none(), "X only, < 0.07 s");
        let mut f = frame(0.0, 0.0, &[], &[]);
        f.now = 1.08;
        pc.step(&f, &uncapped(2), &mut fi);
        assert_eq!(fi.attack.map(|x| x.0), Some(0), "0.07 s after the first X input");
        let mut pc = pc_with(None);
        let mut fi = FrameInput::default();
        pc.step(&frame(0.0, 0.0, &["Strike"], &[]), &uncapped(0), &mut fi);
        pc.step(&frame(0.0, 0.0, &[], &["Strike"]), &uncapped(1), &mut fi);
        assert!(fi.attack.is_some() && !pc.wants_strike, "release fires it");
    }

    /// (rust-ui r3) a rebind saved by the menu (mh-ui input::save: UMordhauInput's ActionMappings in
    /// [/Script/Mordhau.MordhauInput], ExportText with the quoted ActionName) replaces that action's default keys
    #[test]
    fn saved_rebind_replaces_default() {
        let dflt = parse_section("[/Script/Engine.InputSettings]
+ActionMappings=(ActionName=\"Kick\",bShift=False,bCtrl=False,bAlt=False,bCmd=False,Key=F)
+ActionMappings=(ActionName=\"Jump\",Key=SpaceBar)
", "/Script/Engine.InputSettings");
        let saved = parse_section("[/Script/Mordhau.MordhauInput]
ControlScheme=3
ActionMappings=(ActionName=\"Kick\",Key=G,bAlt=False,bCmd=False,bCtrl=False,bShift=False)
ActionMappings=(ActionName=\"Kick\",Key=Gamepad_LeftShoulder,bAlt=False,bCmd=False,bCtrl=False,bShift=False)
", "/Script/Mordhau.MordhauInput");
        let b = merge_bindings(Some(&saved), &dflt);
        assert_eq!(b.actions["Kick"], vec!["G".to_string(), "Gamepad_LeftShoulder".to_string()]);
        assert_eq!(b.actions["Jump"], vec!["SpaceBar".to_string()]);
        let mut pc = PlayerControl::new(AngK::from_spec(None));
        pc.apply_bindings(&b);
        let kick = &pc.actions["Kick"];
        assert!(kick.contains(&ue_key("G").unwrap()) && !kick.contains(&ue_key("F").unwrap()));
    }
}

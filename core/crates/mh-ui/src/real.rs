//! Bevy side of the real-widget UI (host.rs UiRuntime): which screen is up, input (mouse + keyboard as Slate gets
//! it), the per-frame update and drawing of the Slate draw list as absolutely positioned Bevy UI nodes, and the
//! actions the widgets ask of the game (open a level, quit, console commands, UI sounds).
//!
//! Host interface (mh-runtime):
//!   `Screen`        resource: Off (the legacy stand-in draw list) | MainMenu (the menu map's UI: BP_MordhauHUD +
//!                   BP_MainMenu shown) | Match (the HUD of a match; Escape opens BP_MainMenu as the pause menu)
//!   `MatchMap`      resource: the map display name GetMapName returns in a match
//!   `UiAction`      message out: OpenLevel { map, options } / Quit / Console(cmd)
//!   `UiSound`       message out: a SoundCue / SoundWave package the widgets played (hover / click / PlaySound2D)
//!   `UiCursor`      resource: cursor position in pixels when there is no window (offscreen tests)
//!   `UiInput`       message in: synthetic input (tests, scripts): Move / Down / Up / Key

use crate::host::UiRuntime;
use crate::slate::Prim;
use crate::vm::Action;
use crate::{Paks, UiTarget, UiViewport};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::ButtonState;
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Off,
    MainMenu,
    Match,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct MatchMap(pub String);

#[derive(Message, Clone, Debug, PartialEq)]
pub enum UiAction {
    OpenLevel { map: String, options: String },
    Quit,
    Console(String),
    /// (first-person r3) Action::Pawn: the escape menu's Suicide / Change camera perspective on the player's pawn
    Pawn(String),
}

/// the player applied Settings: `input` = UMordhauInput (keymaps, sensitivity), else UMordhauGameUserSettings; the
/// files in settings::config_dir() are already written when this arrives
#[derive(Message, Clone, Debug, PartialEq)]
pub struct UiSettingsApplied {
    pub input: bool,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct UiSound {
    pub cue: String,
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct UiCursor(pub Option<Vec2>);

/// Synthetic input (UE key names: "Escape", "Enter", "LeftMouseButton", ...)
#[derive(Message, Clone, Debug)]
pub enum UiInput {
    Move(Vec2),
    Down(String),
    Up(String),
    Key(String),
    KeyUp(String),
    Wheel(f32),
}

/// Match events for the in-match HUD, written by the runtime (the game's client RPC / camera-manager entry points)
#[derive(Message, Clone, Debug)]
pub enum HudEvent {
    /// the local player damaged someone: AMordhauCharacter::TakeDamage 0x156e980 -> ClientReceiveScoreNoState(Damage 4 /
    /// TeamDamage 5, ReasonParam head 1 / leg 2 / else 0) -> BP_MordhauPlayerController ClientReceiveScoreBP
    /// @21615 ScoreFeed.AddDamage(amount, param), @20649 Crosshair.ShowHitMarker(param)
    DealtDamage { amount: i64, param: i64 },
    /// the view target took directional damage: AMordhauCameraManager::DoHitFlash 0x14f6600 -> BP_MordhauCameraManager
    /// OnHitFlash -> HUD.Crosshair.TriggerDamageIndicator(HitFlashDegrees) (@124); degrees per UpdateHitFlashDegrees 0x151e260
    TookDamage { degrees: f64 },
    /// a chat line from a player: ClientMessage -> ChatBox.OnMessageReceived (host player_chat)
    Chat { sender_id: u64, msg: String, team: bool },
}

/// The runtime + its Bevy assets (NonSend: the VM is single-threaded)
pub struct Rt {
    pub ui: UiRuntime,
    pub screen: Screen,
    tex: HashMap<String, Option<Handle<Image>>>,
    fonts: HashMap<String, Handle<Font>>,
}

#[derive(Component)]
pub struct RtRoot;

/// A host with its own interface can retain HUD events without routing input to hidden widgets.
#[derive(Resource)]
pub struct NativeUiInput(pub bool);
impl Default for NativeUiInput {
    fn default() -> Self { Self(true) }
}

#[derive(Component)]
struct RtItem;

pub fn plugin(app: &mut App) {
    app.init_resource::<Screen>()
        .init_resource::<NativeUiInput>()
        .init_resource::<crate::slate::UiBlurs>() // (rust-render) blur elements for mh-runtime
        .init_resource::<MatchMap>()
        .init_resource::<UiCursor>()
        .add_message::<UiAction>()
        .add_message::<UiSound>()
        .add_message::<UiSettingsApplied>()
        .add_message::<UiInput>()
        .add_message::<HudEvent>()
        .add_systems(Update, (mount, input, hud_events, update, draw).chain());
}

fn hud_events(rt: Option<NonSendMut<Rt>>, mut ev: MessageReader<HudEvent>) {
    let Some(mut rt) = rt else {
        ev.clear();
        return;
    };
    for e in ev.read() {
        match e {
            HudEvent::DealtDamage { amount, param } => rt.ui.dealt_damage(*amount, *param),
            HudEvent::TookDamage { degrees } => rt.ui.damage_taken(*degrees),
            HudEvent::Chat { sender_id, msg, team } => rt.ui.player_chat(*sender_id, msg, *team),
        }
    }
}

/// Bevy KeyCode -> UE FKey name (EKeys)
pub fn ue_key(k: KeyCode) -> Option<String> {
    use KeyCode as K;
    let s = match k {
        K::Escape => "Escape",
        K::Enter | K::NumpadEnter => "Enter",
        K::Space => "SpaceBar",
        K::Tab => "Tab",
        K::Backspace => "BackSpace",
        K::ArrowUp => "Up",
        K::ArrowDown => "Down",
        K::ArrowLeft => "Left",
        K::ArrowRight => "Right",
        K::ShiftLeft => "LeftShift",
        K::ShiftRight => "RightShift",
        K::ControlLeft => "LeftControl",
        K::AltLeft => "LeftAlt",
        K::Delete => "Delete",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PageUp",
        K::PageDown => "PageDown",
        K::Backquote => "Tilde",
        K::F1 => "F1",
        K::F10 => "F10",
        _ => {
            let d = format!("{k:?}");
            if let Some(c) = d.strip_prefix("Key") {
                return Some(c.to_string());
            }
            if let Some(c) = d.strip_prefix("Digit") {
                return Some(["Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine"][c.parse::<usize>().ok()?.min(9)].to_string());
            }
            return None;
        }
    };
    Some(s.to_string())
}

fn mount(world: &mut World) {
    let want = *world.resource::<Screen>();
    let have = world.get_non_send_resource::<Rt>().map(|r| r.screen);
    if have == Some(want) || (want == Screen::Off && have.is_none()) {
        return;
    }
    if want == Screen::Off {
        world.remove_non_send_resource::<Rt>();
        return;
    }
    let Some(p) = Paks::get_or_mount(world) else { return };
    let map = match want {
        Screen::MainMenu => crate::host::MAIN_MENU_MAP_NAME.to_string(),
        _ => world.resource::<MatchMap>().0.clone(),
    };
    // the viewport the UI draws into: the primary window's physical size, else (offscreen) the target's UiViewport
    let win = {
        let mut q = world.query::<&Window>();
        q.iter(world).next().map(|w| [w.physical_width() as f64, w.physical_height() as f64])
    };
    let vp = win.or_else(|| world.get_resource::<UiViewport>().map(|v| [v.0.x as f64, v.0.y as f64]));
    let mut ui = UiRuntime::new_sized(p.0.clone(), &map, vp);
    ui.begin();
    world.insert_non_send_resource(Rt { ui, screen: want, tex: HashMap::new(), fonts: HashMap::new() });
    if world.query_filtered::<Entity, With<RtRoot>>().iter(world).next().is_none() {
        let target = world.get_resource::<UiTarget>().copied();
        let mut e = world.spawn((RtRoot, Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, GlobalZIndex(10)));
        if let Some(t) = target {
            e.insert(UiTargetCamera(t.0));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn input(
    enabled: Res<NativeUiInput>,
    rt: Option<NonSendMut<Rt>>,
    windows: Query<&Window>,
    cursor: Res<UiCursor>,
    mut keys: MessageReader<KeyboardInput>,
    mut buttons: MessageReader<MouseButtonInput>,
    mut wheel: MessageReader<MouseWheel>,
    mut synth: MessageReader<UiInput>,
) {
    if !enabled.0 {
        keys.clear(); buttons.clear(); wheel.clear(); synth.clear();
        return;
    }
    let Some(mut rt) = rt else {
        keys.clear();
        buttons.clear();
        wheel.clear();
        synth.clear();
        return;
    };
    let pos = windows.iter().next().and_then(|w| w.cursor_position()).or(cursor.0);
    if let Some(p) = pos {
        let p = [p.x as f64, p.y as f64];
        if p != rt.ui.mouse {
            rt.ui.mouse_move(p);
        }
    }
    for b in buttons.read() {
        let n = match b.button {
            MouseButton::Left => "LeftMouseButton",
            MouseButton::Right => "RightMouseButton",
            MouseButton::Middle => "MiddleMouseButton",
            _ => continue,
        };
        match b.state {
            ButtonState::Pressed => rt.ui.mouse_down(n),
            ButtonState::Released => rt.ui.mouse_up(n),
        }
    }
    for w in wheel.read() {
        // one notch = 1 line; pixel deltas (touchpads) in 120-per-notch units as Windows reports WM_MOUSEWHEEL
        let d = match w.unit {
            MouseScrollUnit::Line => w.y as f64,
            MouseScrollUnit::Pixel => w.y as f64 / 120.0,
        };
        rt.ui.mouse_wheel(d);
    }
    for k in keys.read() {
        let name = ue_key(k.key_code).or_else(|| match &k.logical_key {
            Key::Character(c) => Some(c.to_uppercase()),
            _ => None,
        });
        let Some(name) = name else { continue };
        match k.state {
            ButtonState::Pressed if !k.repeat => rt.ui.key_down(&name),
            ButtonState::Released => rt.ui.key_up(&name),
            _ => {}
        }
    }
    for s in synth.read() {
        match s {
            UiInput::Move(p) => rt.ui.mouse_move([p.x as f64, p.y as f64]),
            UiInput::Down(b) => rt.ui.mouse_down(b),
            UiInput::Up(b) => rt.ui.mouse_up(b),
            UiInput::Key(k) => rt.ui.key_down(k),
            UiInput::KeyUp(k) => rt.ui.key_up(k),
            UiInput::Wheel(d) => rt.ui.mouse_wheel(*d as f64),
        }
    }
}

fn update(rt: Option<NonSendMut<Rt>>, time: Res<Time>, vp: Res<UiViewport>, mut act: MessageWriter<UiAction>, mut snd: MessageWriter<UiSound>, mut applied: MessageWriter<UiSettingsApplied>) {
    let Some(mut rt) = rt else { return };
    let dt = time.delta_secs_f64().min(0.1);
    rt.ui.update(dt, [vp.0.x as f64, vp.0.y as f64]);
    for a in rt.ui.take_actions() {
        match a {
            Action::OpenLevel { map, options } => {
                act.write(UiAction::OpenLevel { map, options });
            }
            Action::Quit => {
                act.write(UiAction::Quit);
            }
            Action::Pawn(c) => {
                act.write(UiAction::Pawn(c));
            }
            Action::Console(c) => {
                act.write(UiAction::Console(c));
            }
            Action::Sound(cue) => {
                snd.write(UiSound { cue });
            }
            Action::SettingsApplied { input } => {
                applied.write(UiSettingsApplied { input });
            }
        }
    }
}

fn texture(rt: &mut Rt, images: &mut Assets<Image>, pkg: &str) -> Option<Handle<Image>> {
    if let Some(h) = rt.tex.get(pkg) {
        return h.clone();
    }
    let h = (|| {
        use mh_assets::texture;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        if pkg.starts_with("gen:") {
            let (w, hgt, mut px) = crate::slate::gen_texture(pkg)?;
            slate_blend_texels(&mut px, false);
            return Some(images.add(Image::new(Extent3d { width: w, height: hgt, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, bevy::asset::RenderAssetUsages::default())));
        }
        if pkg.starts_with("png:") {
            // engine Slate images are sRGB PNGs
            let (w, hgt, mut px) = crate::slate::slate_png(&rt.ui.res.rd, pkg)?;
            slate_blend_texels(&mut px, true);
            return Some(images.add(Image::new(Extent3d { width: w, height: hgt, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, bevy::asset::RenderAssetUsages::default())));
        }
        let src = &rt.ui.res.src;
        let t = texture::info(src, pkg, None).ok()?;
        let data = texture::mip_data(src, &t, 0).ok()?;
        let (w, hgt) = (t.mips[0].size_x as u32, t.mips[0].size_y as u32);
        let mut px = match texture::decode(t.format?, w as usize, hgt as usize, &data).ok()? {
            texture::Pixels::Rgba8(v) => v,
            texture::Pixels::RgbaF32(v) => v.iter().map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8).collect(),
        };
        // texels re-encoded for Slate's gamma-space blend (slate_blend_texels), always as sRGB RGBA8
        slate_blend_texels(&mut px, t.srgb);
        Some(images.add(Image::new(Extent3d { width: w, height: hgt, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, bevy::asset::RenderAssetUsages::default())))
    })();
    rt.tex.insert(pkg.to_string(), h.clone());
    h
}

/// Slate's display gamma: UE 4.26 FSlateRHIRenderer passes GammaValues (1 / DisplayGamma, DisplayGamma) with
/// DisplayGamma 2.2 (UEngine default, BaseEngine.ini [/Script/Engine.Engine] DisplayGamma=2.2) to
/// SlateElementPixelShader.usf GammaCorrect, which encodes the source colour before the blend unit runs.
pub const SLATE_GAMMA: f64 = 2.2;

/// Slate blends in gamma (display-encoded) space: the back buffer is PF_B8G8R8A8 UNORM, not an sRGB view
/// (UE 4.26 FSlateRHIRenderer::GetSlateRecommendedColorFormat), the vertex tint is packed sRGB
/// (FSlateElementBatcher::PackVertexColor) and SlateElementPixelShader.usf GammaCorrect encodes the output before the
/// fixed-function SrcAlpha / InvSrcAlpha blend, so a black 0.8-alpha panel over a 0.5 pixel gives 0.1 (cross-checked
/// with Codex 2026-10-06). Bevy's UI blends linear values into an sRGB / float target (0.36 for the same pixel: the
/// Settings panels looked see-through). This maps a Slate draw (linear colour `c`, alpha `a`) to the straight-alpha
/// linear colour that gives Slate's gamma blend result exactly when the destination is black or the source is black,
/// and close to it in between: (1 - a') = (1 - a)^g (destination weight) and a' * c' = (a * c^(1/g))^g (source
/// contribution). Since a^g + (1 - a)^g <= 1 for g >= 1, c' stays <= 1.
pub fn slate_blend(c: [f64; 4]) -> [f64; 4] {
    let a = c[3].clamp(0.0, 1.0);
    if a >= 1.0 || a <= 0.0 {
        return [c[0], c[1], c[2], a];
    }
    let g = SLATE_GAMMA;
    let a2 = 1.0 - (1.0 - a).powf(g);
    let ch = |x: f64| ((a * x.max(0.0).powf(1.0 / g)).powf(g) / a2).min(1.0);
    [ch(c[0]), ch(c[1]), ch(c[2]), a2]
}

/// slate_blend over a texture's texels (`srgb`: the texture's sRGB flag, else the texels are linear values), written
/// back as sRGB-encoded RGBA8 for an Rgba8UnormSrgb image. The draw's tint multiplies afterwards (exact for opaque
/// tints; a translucent tint over a translucent texel is the product of the two remapped alphas: UNCONFIRMED in-between).
pub fn slate_blend_texels(px: &mut [u8], srgb: bool) {
    let to_lin = |v: u8| {
        let x = v as f64 / 255.0;
        if srgb {
            if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
        } else {
            x
        }
    };
    let to_srgb = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        let s = if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0).round() as u8
    };
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as f64 / 255.0;
        if p[3] == 255 || p[3] == 0 {
            if !srgb {
                let (r, g, b) = (to_lin(p[0]), to_lin(p[1]), to_lin(p[2]));
                p[0] = to_srgb(r);
                p[1] = to_srgb(g);
                p[2] = to_srgb(b);
            }
            continue;
        }
        let o = slate_blend([to_lin(p[0]), to_lin(p[1]), to_lin(p[2]), a]);
        p[0] = to_srgb(o[0]);
        p[1] = to_srgb(o[1]);
        p[2] = to_srgb(o[2]);
        p[3] = (o[3] * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

fn col(c: [f64; 4]) -> Color {
    // Slate colours are linear (FLinearColor); blended the way Slate blends (slate_blend)
    let c = slate_blend(c);
    Color::linear_rgba(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32)
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    rt: Option<NonSendMut<Rt>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
    root: Query<Entity, With<RtRoot>>,
    items: Query<Entity, With<RtItem>>,
    mut blurs: ResMut<crate::slate::UiBlurs>,
) {
    for e in items.iter() {
        commands.entity(e).despawn();
    }
    let Some(mut rt) = rt else {
        blurs.els.clear();
        return;
    };
    // (rust-render) the UBackgroundBlur elements to the renderer (uepost_render.rs ui_blur)
    let nb = crate::slate::UiBlurs { els: rt.ui.out.blurs.clone(), size: rt.ui.out.size };
    if blurs.els != nb.els || blurs.size != nb.size {
        *blurs = nb;
    }
    let Ok(root) = root.single() else { return };
    let list = rt.ui.out.items.clone();
    for (z, it) in list.iter().enumerate() {
        // edges snapped to whole pixels (Slate's pixel snapping of element vertices): adjacent 9-slice pieces with
        // fractional rects otherwise round apart in Bevy's layout and leave 1 px seams that show what lies under them
        // (the settings rows' hover highlight read as a bright line)
        let node = |r: [f64; 4]| {
            let (x0, y0) = (r[0].round(), r[1].round());
            let (x1, y1) = ((r[0] + r[2].max(0.0)).round(), (r[1] + r[3].max(0.0)).round());
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x0 as f32),
                top: Val::Px(y0 as f32),
                width: Val::Px((x1 - x0) as f32),
                height: Val::Px((y1 - y0) as f32),
                ..default()
            }
        };
        // a clipped item sits in a clip node at the clip rect; its own rect is then relative to it
        let (outer, rel) = match it.clip {
            Some(c) => (Some(c), [c[0].round(), c[1].round()]),
            None => (None, [0.0, 0.0]),
        };
        let rr = |r: [f64; 4]| [r[0] - rel[0], r[1] - rel[1], r[2], r[3]];
        let id = match &it.prim {
            Prim::Rect { rect, color } => commands.spawn((node(rr(*rect)), BackgroundColor(col(*color)))).id(),
            Prim::Image { rect, tex, tint, uv } => match texture(&mut rt, &mut images, tex) {
                Some(h) => {
                    // Slate fills the element's rect with the brush (or 9-slice piece): no aspect fit (Bevy's default
                    // NodeImageMode::Auto letterboxes a piece whose aspect differs from its source rect)
                    let mut img = ImageNode::new(h).with_color(col(*tint)).with_mode(bevy::ui::widget::NodeImageMode::Stretch);
                    if let Some(u) = uv {
                        img.rect = Some(bevy::math::Rect::new(u[0] as f32, u[1] as f32, (u[0] + u[2]) as f32, (u[1] + u[3]) as f32));
                        // (rust-armory) a negative-width UV rect = a mirrored widget (slate.rs Image, RenderTransform
                        // Scale X < 0)
                        img.flip_x = u[2] < 0.0;
                    }
                    commands.spawn((node(rr(*rect)), img)).id()
                }
                None => continue,
            },
            Prim::Text { rect, text, face, px, color } => {
                let h = match rt.fonts.get(face) {
                    Some(h) => h.clone(),
                    None => {
                        let Some(f) = rt.ui.res.faces.values().flatten().find(|f| &f.package == face).cloned() else { continue };
                        let h = fonts.add(Font::from_bytes(f.data.to_vec()));
                        rt.fonts.insert(face.clone(), h.clone());
                        h
                    }
                };
                let mut r = rr(*rect);
                r[2] += 4.0;
                commands
                    .spawn((
                        node(r),
                        Text::new(text.clone()),
                        TextFont { font: bevy::text::FontSource::Handle(h), font_size: bevy::text::FontSize::Px(*px as f32), ..default() },
                        TextColor(col(*color)),
                        TextLayout::new(Justify::Left, LineBreak::NoWrap),
                    ))
                    .id()
            }
        };
        let top = match outer {
            Some(c) => {
                let mut n = node(c);
                n.overflow = Overflow::clip();
                let cl = commands.spawn((n, RtItem, ZIndex(z as i32))).id();
                commands.entity(cl).add_child(id);
                cl
            }
            None => {
                commands.entity(id).insert((RtItem, ZIndex(z as i32)));
                id
            }
        };
        commands.entity(root).add_child(top);
    }
}

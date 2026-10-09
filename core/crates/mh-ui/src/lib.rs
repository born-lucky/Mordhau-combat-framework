//! mh-ui: the Mordhau HUD and main menu in Bevy UI, laid out from the game's UMG WidgetBlueprints (mh-assets umg, read
//! from the paks) the way the Godot port's views lay them out (layout.rs: duel_hud_view.gd, ffa_hud_view.gd,
//! main_menu.gd), driven by mh-mode's view models (HudCmd) and the HUD models of models.rs (BP_StatusBar,
//! BP_Announcement, BP_Victory/DefeatPopup, BP_KillFeed bytecode ports).
//!
//! Drawing is an immediate-mode draw list (the Godot views' canvas calls: rects, textures, 9-slice boxes, text) rebuilt
//! every frame and placed as absolutely positioned Bevy UI nodes under one root node.
//! Textures: mh-assets texture decode (sRGB for SRGB textures). Fonts: the MordhauFont faces' .ufont bytes
//! (mh-assets fonts) as Bevy Font assets.
//!
//! Host interface (mh-runtime): insert `Paks` (or let the plugin mount the default install), write `HudMsg`, update
//! `HudVitals` / `Scoreboard` / `MainMenu`, read `MenuChoice`; `UiTarget` picks the camera, `UiViewport` the logical
//! size when there is no window (offscreen).

pub mod anim;
pub mod armory;
pub mod armory_natives;
pub mod evidence;
pub mod game;
pub mod host;
pub mod input;
pub mod kismet;
pub mod menu_data;
pub mod model;
pub mod natives;
pub mod news;
pub mod real;
pub mod settings;
pub mod slate;
pub mod vm;
pub mod layout;
pub mod models;

use bevy::prelude::*;
use layout::*;
use mh_assets::umg::{srgb, Brush, UmgPackage};
use models::*;
use std::collections::HashMap;
use std::sync::Arc;
pub use real::{HudEvent, MatchMap, Screen, UiAction, UiCursor, UiInput, UiSettingsApplied, UiSound};

/// The mounted paks, shared by mh-ui / mh-fx / mh-audio (insert the host's; the plugins mount the default otherwise)
#[derive(Resource, Clone)]
pub struct Paks(pub Arc<mh_pak::Vfs>);

impl Paks {
    pub fn get_or_mount(world: &mut World) -> Option<Paks> {
        if let Some(p) = world.get_resource::<Paks>() {
            return Some(p.clone());
        }
        let v = mh_pak::Vfs::mount_default().ok()?;
        let p = Paks(Arc::new(v));
        world.insert_resource(p.clone());
        Some(p)
    }
}

/// HUD input: the view models' commands (PlayerView / DuelView drain())
#[derive(Message, Clone, Debug)]
pub enum HudMsg {
    Cmd(mh_mode::views::HudCmd),
}

/// the observed character's bars (bytes, as BP_StatusBar reads them); target = any stable id of the view target
#[derive(Resource, Clone, Debug, Default)]
pub struct HudVitals {
    pub target: Option<u64>,
    pub health: i64,
    pub stamina: i64,
    pub visible: bool,
    /// round wins per team (duel pips)
    pub wins: [i64; 2],
    pub show_pips: bool,
    /// team colours (linear), BP_MordhauGameState TeamColors
    pub team_colors: [[f64; 4]; 2],
}

#[derive(Clone, Debug, Default)]
pub struct ScoreRow {
    pub name: String,
    pub score: i64,
    pub kills: i64,
    pub deaths: i64,
    pub local: bool,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct Scoreboard {
    pub rows: Vec<ScoreRow>,
    pub visible: bool,
    pub title: String,
    pub seconds_left: i64,
}

/// BP_LocalPlay: the mode list (ModeTable rows' metadata names), the map label, the selection
#[derive(Resource, Clone, Debug, Default)]
pub struct MainMenu {
    pub visible: bool,
    pub modes: Vec<String>,
    pub map_label: String,
    pub selected: usize,
}

/// the menu's Start Match result (index into MainMenu.modes)
#[derive(Message, Clone, Debug)]
pub struct MenuChoice(pub usize);

/// the camera the UI renders to (offscreen runs), else Bevy's default UI camera
#[derive(Resource, Clone, Copy)]
pub struct UiTarget(pub Entity);

/// logical viewport size in pixels (offscreen: the target image size); updated from the primary window when there is one
#[derive(Resource, Clone, Copy)]
pub struct UiViewport(pub Vec2);

impl Default for UiViewport {
    fn default() -> Self {
        UiViewport(Vec2::new(1280.0, 720.0))
    }
}

/// what was drawn, for evidence (item count, textures, missing textures, fonts loaded)
#[derive(Resource, Clone, Debug, Default)]
pub struct UiStats {
    pub items: usize,
    pub textures: usize,
    pub missing: Vec<String>,
    pub fonts: Vec<String>,
}

/// The loaded UMG packages, fonts, textures and models (NonSend: mh-pak's Reader is single-threaded)
pub struct UiState {
    pub ui: Ui,
    pub status: StatusBarModel,
    pub ann: AnnouncementModel,
    pub popup: ResultPopup,
    pub feed: KillFeedModel,
    pub kismet: mh_mode::Kismet,
    tex: HashMap<String, Option<Handle<Image>>>,
    font_handles: HashMap<String, Handle<Font>>,
    rd: mh_pak::Reader,
    src: mh_assets::pak_source::PakSource,
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<HudMsg>()
            .add_message::<MenuChoice>()
            .init_resource::<HudVitals>()
            .init_resource::<Scoreboard>()
            .init_resource::<MainMenu>()
            .init_resource::<UiViewport>()
            .init_resource::<UiStats>()
            .add_systems(Startup, setup)
            .add_systems(Update, (viewport, tick_models, draw.run_if(|s: Res<real::Screen>| *s == real::Screen::Off), clear_legacy.run_if(|s: Res<real::Screen>| *s != real::Screen::Off)).chain());
        real::plugin(app);
        app.add_plugins(armory::ArmoryPlugin); // rust-armory: SpawnProfile / ArmoryPreview
    }
}

#[derive(Component)]
struct UiRoot;

#[derive(Component)]
struct UiItem;

/// mode_kismet.json (scripts/mode_kismet.py) from the repo's godot/data_gen, or $MH_KISMET
pub fn kismet() -> mh_mode::Kismet {
    let p = std::env::var("MH_KISMET").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../godot/data_gen/mode/mode_kismet.json")
    });
    std::fs::read_to_string(p).ok().and_then(|t| mh_mode::Kismet::from_json(&t).ok()).unwrap_or_default()
}

/// build the state (usable without Bevy: layouts and models)
pub fn load_state(paks: &Paks) -> UiState {
    let rd = mh_pak::Reader::new(paks.0.clone());
    let mut u = HashMap::new();
    for p in PACKAGES {
        if let Some(x) = UmgPackage::read(&rd, p) {
            u.insert(p, x);
        }
    }
    let mut fonts = HashMap::new();
    if let Some(cf) = mh_assets::fonts::composite(&rd, FONT) {
        for e in &cf.default {
            if let Some(f) = mh_assets::fonts::face(&rd, &e.face) {
                if let Some(a) = mh_assets::fonts::Advances::new(&f.data) {
                    fonts.insert(e.name.clone(), a);
                }
            }
        }
    }
    let scale_keys = mh_assets::umg::ui_scale_keys(&rd);
    let ui = Ui { u, fonts, scale_keys };
    let k = kismet();
    let anim = |p: &str, s: &str| ui.u.get(p).map_or(0.0, |x| x.movie_scene_last_tick(s).max(0.0) / TICK_RESOLUTION);
    let ann = AnnouncementModel::new(anim(ANNOUNCE, "Entry Anim"), anim(ANNOUNCE, "Exit Anim"));
    let status = StatusBarModel::new(k.clone(), ui.u.get(STATUS).and_then(|x| x.defaults()));
    let feed = KillFeedModel::new(ui.u.get(KILL_FEED).and_then(|x| x.defaults()));
    UiState {
        status,
        ann,
        popup: ResultPopup::new(),
        feed,
        kismet: k,
        tex: HashMap::new(),
        font_handles: HashMap::new(),
        src: mh_assets::pak_source::PakSource::new(paks.0.clone()),
        rd,
        ui,
    }
}

fn setup(world: &mut World) {
    let Some(paks) = Paks::get_or_mount(world) else {
        warn!("mh-ui: no paks mounted; UI disabled");
        return;
    };
    let mut st = load_state(&paks);
    let mut names = vec![];
    if let Some(cf) = mh_assets::fonts::composite(&st.rd, FONT) {
        let mut fonts = world.resource_mut::<Assets<Font>>();
        for e in &cf.default {
            if let Some(f) = mh_assets::fonts::face(&st.rd, &e.face) {
                st.font_handles.insert(e.name.clone(), fonts.add(Font::from_bytes(f.data)));
                names.push(e.name.clone());
            }
        }
    }
    world.resource_mut::<UiStats>().fonts = names;
    let target = world.get_resource::<UiTarget>().copied();
    let mut e = world.spawn((UiRoot, Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }));
    if let Some(t) = target {
        e.insert(UiTargetCamera(t.0));
    }
    world.insert_non_send(st);
}

fn viewport(windows: Query<&Window>, mut vp: ResMut<UiViewport>) {
    if let Some(w) = windows.iter().next() {
        let s = Vec2::new(w.width(), w.height());
        if s.x > 0.0 && vp.0 != s {
            vp.0 = s;
        }
    }
}

/// apply one HudCmd to the models (kill feed, announcement, match result)
pub fn apply_cmd(st: &mut UiState, c: &mh_mode::views::HudCmd) {
    use mh_mode::views::HudCmd as H;
    match c {
        H::Announce { text, subtext, duration, .. } => st.ann.show(text, subtext, *duration),
        H::MatchResult { victory, text, subtext } => {
            let ui = &st.ui;
            let anim = |v: bool, s: &str| ui.u.get(if v { VICTORY } else { DEFEAT }).map_or(0.0, |x| x.movie_scene_last_tick(s).max(0.0) / TICK_RESOLUTION);
            st.popup.show(&st.kismet, &anim, *victory, text, subtext)
        }
        H::KillFeed { killer, with, victim, killer_color, victim_color } => st.feed.add(killer, with, victim, *killer_color, *victim_color),
        _ => {}
    }
}

fn tick_models(st: Option<NonSendMut<UiState>>, time: Res<Time>, vit: Res<HudVitals>, mut msgs: MessageReader<HudMsg>) {
    let Some(mut st) = st else { return };
    let dt = time.delta_secs_f64();
    let st = &mut *st;
    st.status.tick(dt, vit.target, vit.health, vit.stamina);
    st.ann.tick(dt);
    st.popup.tick(dt);
    st.feed.tick(dt);
    for HudMsg::Cmd(c) in msgs.read() {
        apply_cmd(st, c);
    }
}

/// One draw-list item (the Godot views' canvas calls)
#[derive(Clone, Debug)]
pub enum Draw {
    Rect { rect: [f64; 4], color: [f64; 4] },
    /// texture package, tint (sRGB), 9-slice margins (brush Margin fractions, Box / Border) or None
    Image { rect: [f64; 4], tex: String, tint: [f64; 4], slice: Option<[f64; 4]> },
    Text { rect: [f64; 4], text: String, typeface: String, px: f64, color: [f64; 4], shadow: [f64; 4], halign: u8, valign: u8 },
}

fn col(c: [f64; 4]) -> Color {
    Color::srgba(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32)
}

/// the frame's draw list (duel_hud_view.gd _draw_hud / ffa_hud_view.gd _draw_extra / main_menu.gd _build)
pub fn draw_list(st: &UiState, vit: &HudVitals, sb: &Scoreboard, menu: &MainMenu, vp: [f64; 2]) -> Vec<(String, Draw)> {
    let ui = &st.ui;
    let mut out: Vec<(String, Draw)> = vec![];
    if PACKAGES.iter().any(|p| !ui.u.contains_key(p)) {
        return out;
    }
    let brush_img = |out: &mut Vec<(String, Draw)>, id: String, b: &Brush, r: [f64; 4], tint: Option<[f64; 4]>| {
        let t = tint.unwrap_or(srgb(b.tint));
        match &b.resource {
            Some((p, cls)) if cls.starts_with("Texture") => {
                let slice = if b.draw_as.ends_with("Box") || b.draw_as.ends_with("Border") { Some(b.margin) } else { None };
                out.push((id, Draw::Image { rect: r, tex: p.clone(), tint: t, slice }))
            }
            _ => out.push((id, Draw::Rect { rect: r, color: t })),
        }
    };
    let text_px = |p: &str, n: &str, s: f64| -> (String, f64, [f64; 4], [f64; 4]) {
        let t = ui.u[p].text_style(n);
        let (tf, sz) = t.as_ref().and_then(|t| t.font.as_ref()).map(|f| (f.typeface.clone(), f.size)).unwrap_or(("".into(), 24.0));
        let (c, sh) = t.map(|t| (srgb(t.color), srgb(t.shadow))).unwrap_or(([1.0; 4], [0.0; 4]));
        (tf, mh_assets::fonts::font_px(sz, s) as f64, c, sh)
    };
    let txt = |p: &str, n: &str| ui.u[p].text_style(n).map(|t| t.text).unwrap_or_default();
    if menu.visible {
        let m = menu_layout(ui, vp);
        let lp = ui.pkg(LOCAL_PLAY);
        brush_img(&mut out, "menu.dim".into(), &lp.brush("Image"), m.dim, None);
        brush_img(&mut out, "menu.left".into(), &lp.brush("Image_52"), m.left, None);
        brush_img(&mut out, "menu.right".into(), &lp.brush("Image_0"), m.right, None);
        brush_img(&mut out, "menu.head".into(), &lp.brush("Image_4"), m.head, None);
        let (tf, px, c, sh) = text_px(LOCAL_PLAY, "Text", m.scale);
        out.push(("menu.head_text".into(), Draw::Text { rect: m.head_text, text: txt(LOCAL_PLAY, "Text"), typeface: tf.clone(), px, color: c, shadow: sh, halign: 0, valign: 1 }));
        out.push(("menu.map".into(), Draw::Rect { rect: m.map_entry, color: [0.2, 0.2, 0.2, 0.8] }));
        // BP_MapEntry not read (14 pt, as the reference: UNCONFIRMED)
        out.push(("menu.map_text".into(), Draw::Text { rect: m.map_entry, text: menu.map_label.clone(), typeface: tf, px: mh_assets::fonts::font_px(14.0, m.scale) as f64, color: c, shadow: sh, halign: 1, valign: 1 }));
        let bs = lp.button_brushes("StartButton");
        brush_img(&mut out, "menu.start".into(), &bs[0], m.start, None);
        let (tf2, px2, c2, sh2) = text_px(LOCAL_PLAY, "TextStartMatch", m.scale);
        out.push(("menu.start_text".into(), Draw::Text { rect: m.start, text: txt(LOCAL_PLAY, "TextStartMatch"), typeface: tf2, px: px2, color: c2, shadow: sh2, halign: 1, valign: 1 }));
        let (tf3, px3, c3, sh3) = text_px(LOCAL_PLAY, "TextBlock_0", m.scale);
        out.push(("menu.mode_label".into(), Draw::Text { rect: m.mode_label, text: txt(LOCAL_PLAY, "TextBlock_0"), typeface: tf3.clone(), px: px3, color: c3, shadow: sh3, halign: 0, valign: 0 }));
        let tint = lp.props("GameModeComboBox").and_then(|p| p.pointer("/WidgetStyle/ComboButtonStyle/ButtonStyle/Normal/TintColor"));
        out.push(("menu.mode_box".into(), Draw::Rect { rect: m.mode_box, color: srgb(mh_assets::umg::color(tint, [0.1, 0.1, 0.1, 1.0])) }));
        let mut mb = m.mode_box;
        mb[0] += 12.0 * m.scale;
        out.push(("menu.mode".into(), Draw::Text { rect: mb, text: menu.modes.get(menu.selected).cloned().unwrap_or_default(), typeface: tf3, px: px3, color: c3, shadow: sh3, halign: 0, valign: 1 }));
        return out;
    }
    if !vit.visible {
        return out;
    }
    let l = duel_layout(ui, vp);
    let s = l.scale;
    let sbp = ui.pkg(STATUS);
    let inset = |r: [f64; 4], p: [f64; 4]| [r[0] + p[0] * s, r[1] + p[1] * s, r[2] - (p[0] + p[2]) * s, r[3] - (p[1] + p[3]) * s];
    let hr = inset(l.health, ui.padding(STATUS, "ProgressBar_DisplayedHealth"));
    let sr = inset(l.stamina, ui.padding(STATUS, "ProgressBar_DisplayedStam"));
    let (dhb, dhf) = sbp.progress_tints("ProgressBar_DelayedHealth");
    let (dsb, dsf) = sbp.progress_tints("ProgressBar_DelayedStam");
    out.push(("hp.bg".into(), Draw::Rect { rect: hr, color: srgb(dhb) }));
    out.push(("st.bg".into(), Draw::Rect { rect: sr, color: srgb(dsb) }));
    // LeftToRight fill; the stamina bar is mirrored (Scale X -1)
    let bar = |r: [f64; 4], pct: f64, mirrored: bool| {
        let w = r[2] * pct.clamp(0.0, 1.0);
        [if mirrored { r[0] + r[2] - w } else { r[0] }, r[1], w, r[3]]
    };
    out.push(("hp.delayed".into(), Draw::Rect { rect: bar(hr, st.status.delayed_health_percent(), false), color: srgb(dhf) }));
    out.push(("st.delayed".into(), Draw::Rect { rect: bar(sr, st.status.delayed_stamina_percent(), true), color: srgb(dsf) }));
    let kc = |k: &str| {
        let a: Vec<f64> = if st.kismet.has(k) { st.kismet.arr(k).iter().map(mh_mode::kismet::vf).collect() } else { vec![] };
        if a.len() == 4 { srgb([a[0], a[1], a[2], a[3]]) } else { [1.0, 0.0, 1.0, 1.0] }
    };
    out.push(("hp.bar".into(), Draw::Rect { rect: bar(hr, st.status.health_percent(), false), color: kc("sb_health_bar_color") }));
    out.push(("st.bar".into(), Draw::Rect { rect: bar(sr, st.status.stamina_percent(), true), color: kc("sb_stam_bar_color") }));
    let pulse = sbp.brush("BorderPulseImage");
    let mut pc = srgb(sbp.image_color("BorderPulseImage"));
    pc[3] = st.status.health_pulse.clamp(0.0, 1.0);
    brush_img(&mut out, "hp.pulse".into(), &pulse, l.health, Some(pc));
    let mut pc2 = srgb(sbp.image_color("BorderPulseImage_1"));
    pc2[3] = st.status.stamina_pulse.clamp(0.0, 1.0);
    brush_img(&mut out, "st.pulse".into(), &pulse, l.stamina, Some(pc2));
    brush_img(&mut out, "status.container".into(), &sbp.brush("container_Image"), l.status, Some([1.0; 4]));
    brush_img(&mut out, "st.icon".into(), &sbp.brush("StaminaIcon"), l.stamina_icon, Some([1.0; 4]));
    brush_img(&mut out, "hp.icon".into(), &sbp.brush("HealthIcon"), l.health_icon, Some([1.0; 4]));
    let (tf, px, c, sh) = text_px(STATUS, "TextBlock_2", s);
    out.push(("st.text".into(), Draw::Text { rect: l.stamina_text, text: st.status.stamina_text(), typeface: tf, px, color: c, shadow: sh, halign: 2, valign: 2 }));
    let (tf, px, c, sh) = text_px(STATUS, "TextBlock_5", s);
    out.push(("hp.text".into(), Draw::Text { rect: l.health_text, text: st.status.health_text(), typeface: tf, px, color: c, shadow: sh, halign: 0, valign: 2 }));
    if st.feed.visible {
        let ent: Vec<(String, String, String)> = st.feed.entries.iter().map(|e| (e.killer.clone(), e.with.clone(), e.victim.clone())).collect();
        let rows = feed_layout(ui, vp, &ent);
        let bg = srgb(ui.pkg(KILL_FEED_ENTRY).brush("Image_14").tint);
        for (i, (r, parts)) in rows.iter().enumerate() {
            let e = &st.feed.entries[i];
            out.push((format!("feed.{i}.bg"), Draw::Rect { rect: *r, color: bg }));
            let names = ["TextBlock_Killer", "TextBlock_Weapon", "TextBlock_Victim"];
            let texts = [&e.killer, &e.with, &e.victim];
            for j in 0..3 {
                let (tf, px, c, sh) = text_px(KILL_FEED_ENTRY, names[j], s);
                let color = match j {
                    0 => srgb(e.killer_color),
                    2 => srgb(e.victim_color),
                    _ => c,
                };
                out.push((format!("feed.{i}.{j}"), Draw::Text { rect: parts[j], text: texts[j].clone(), typeface: tf, px, color, shadow: sh, halign: 0, valign: 1 }));
            }
        }
    }
    if sb.visible {
        let (panel, rows) = scoreboard_layout(ui, vp, sb.rows.len());
        let (tf, px, c, sh) = text_px(ONE_TEAM, "TextBlock_8", s);
        out.push(("sb.title".into(), Draw::Text { rect: [panel[0], panel[1], panel[2], 32.0 * s], text: sb.title.clone(), typeface: tf, px, color: c, shadow: sh, halign: 1, valign: 0 }));
        let (tf, px, c, sh) = text_px(ONE_TEAM, "TextBlock_2", s);
        let clock = format!("{:02}:{:02}", sb.seconds_left / 60, sb.seconds_left % 60);
        out.push(("sb.clock".into(), Draw::Text { rect: [panel[0], panel[1] + 28.0 * s, panel[2], 36.0 * s], text: clock, typeface: tf, px, color: c, shadow: sh, halign: 1, valign: 0 }));
        let bgc = srgb(ui.pkg(SB_ENTRY).image_color("Image_49"));
        let cols = ["TextBlock_786", "TextBlock_787", "TextBlock_788", "TextBlock_789"];
        let xs = [0.15, 0.62, 0.74, 0.86]; // column positions (the reference's, UNCONFIRMED: Fill split not computed)
        for (i, r) in rows.iter().enumerate() {
            out.push((format!("sb.{i}.bg"), Draw::Rect { rect: [r[0] + 1.0, r[1] + 1.0, r[2] - 2.0, r[3] - 2.0], color: bgc }));
            let row = &sb.rows[i];
            let vals = [row.name.clone(), row.score.to_string(), row.kills.to_string(), row.deaths.to_string()];
            for j in 0..4 {
                let (tf, px, mut c, sh) = text_px(SB_ENTRY, cols[j], s);
                if j == 0 && row.local {
                    c = kc("kf_local_color");
                }
                out.push((format!("sb.{i}.{j}"), Draw::Text { rect: [r[0] + r[2] * xs[j], r[1], r[2] * 0.3, r[3]], text: vals[j].clone(), typeface: tf, px, color: c, shadow: sh, halign: 0, valign: 1 }));
            }
        }
    }
    if vit.show_pips {
        let pip = ui.pkg(PIP);
        let (bg, fill) = (pip.brush("Image_0"), pip.brush("Image_1"));
        for t in 0..2 {
            for (i, r) in l.pips[t].iter().enumerate() {
                brush_img(&mut out, format!("pip.{t}.{i}.bg"), &bg, *r, None);
                if (i as i64) < vit.wins[t] {
                    brush_img(&mut out, format!("pip.{t}.{i}.fill"), &fill, *r, Some(srgb(vit.team_colors[t])));
                }
            }
        }
    }
    if st.ann.showing {
        let a = st.ann.opacity();
        let (tf, px, mut c, mut sh) = text_px(ANNOUNCE, "TextBlock_0", s);
        c[3] *= a;
        sh[3] *= a;
        let th = ui.fonts.get(&tf).map_or(px * 1.2, |f| f.height(px));
        let ar = l.announce;
        out.push(("ann.text".into(), Draw::Text { rect: [ar[0] - 2000.0 * s, ar[1], 4000.0 * s, th], text: st.ann.text.clone(), typeface: tf, px, color: c, shadow: sh, halign: 1, valign: 0 }));
        if st.ann.subtext_visible() {
            let (tf, px, mut c, mut sh) = text_px(ANNOUNCE, "TextBlock_1", s);
            c[3] *= a;
            sh[3] *= a;
            let sp = ui.padding(ANNOUNCE, "Overlay_4");
            out.push(("ann.sub".into(), Draw::Text { rect: [ar[0] - 2000.0 * s, ar[1] + th + sp[1] * s, 4000.0 * s, px * 1.2], text: st.ann.subtext.clone(), typeface: tf, px, color: c, shadow: sh, halign: 1, valign: 0 }));
        }
    }
    if st.popup.showing {
        let p = if st.popup.victory { VICTORY } else { DEFEAT };
        let a = st.popup.opacity();
        let (tf, px, mut c, _) = text_px(p, "PrimaryHeader", s);
        c[3] *= a;
        let tw = ui.fonts.get(&tf).map_or(0.6 * px * st.popup.text.len() as f64, |f| f.width(&st.popup.text, px));
        let th = ui.fonts.get(&tf).map_or(px * 1.2, |f| f.height(px));
        let rib = ui.pkg(p).brush("PrimaryRibbon");
        let rsz = [rib.image_size[0] * s, rib.image_size[1] * s];
        let (tp, rp) = (ui.padding(p, "Overlay_5"), ui.padding(p, "Overlay_0"));
        let w = (tw + (tp[0] + tp[2]) * s - (rp[0] + rp[2]) * s).max(rsz[0]);
        let cp = l.popup_center;
        let mut b2 = rib.clone();
        b2.margin = [0.5, 0.0, 0.5, 0.0]; // Brush Margin Left / Right 0.5 (Box), as the reference's StyleBoxTexture
        b2.draw_as = "ESlateBrushDrawType::Box".into();
        brush_img(&mut out, "popup.ribbon".into(), &b2, [cp[0] - w * 0.5, cp[1] - rsz[1] * 0.5, w, rsz[1]], Some([1.0, 1.0, 1.0, a]));
        out.push(("popup.text".into(), Draw::Text { rect: [cp[0] - w * 0.5, cp[1] - th * 0.5, w, th], text: st.popup.text.clone(), typeface: tf, px, color: c, shadow: [0.0; 4], halign: 1, valign: 1 }));
    }
    out
}

fn texture(st: &mut UiState, images: &mut Assets<Image>, pkg: &str) -> Option<Handle<Image>> {
    if let Some(h) = st.tex.get(pkg) {
        return h.clone();
    }
    let h = (|| {
        use mh_assets::texture;
        let t = texture::info(&st.src, pkg, None).ok()?;
        let data = texture::mip_data(&st.src, &t, 0).ok()?;
        let (w, hgt) = (t.mips[0].size_x as u32, t.mips[0].size_y as u32);
        let px = match texture::decode(t.format?, w as usize, hgt as usize, &data).ok()? {
            texture::Pixels::Rgba8(v) => v,
            texture::Pixels::RgbaF32(v) => v.iter().map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8).collect(),
        };
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let fmt = if t.srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
        let img = Image::new(Extent3d { width: w, height: hgt, depth_or_array_layers: 1 }, TextureDimension::D2, px, fmt, bevy::asset::RenderAssetUsages::default());
        Some(images.add(img))
    })();
    st.tex.insert(pkg.to_string(), h.clone());
    h
}

/// the real-widget screens replace the stand-in draw list
fn clear_legacy(mut commands: Commands, items: Query<Entity, With<UiItem>>) {
    for e in items.iter() {
        commands.entity(e).despawn();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    st: Option<NonSendMut<UiState>>,
    vit: Res<HudVitals>,
    sb: Res<Scoreboard>,
    menu: Res<MainMenu>,
    vp: Res<UiViewport>,
    mut images: ResMut<Assets<Image>>,
    mut stats: ResMut<UiStats>,
    root: Query<Entity, With<UiRoot>>,
    items: Query<Entity, With<UiItem>>,
) {
    let Some(mut st) = st else { return };
    let Ok(root) = root.single() else { return };
    let list = draw_list(&st, &vit, &sb, &menu, [vp.0.x as f64, vp.0.y as f64]);
    // immediate mode: last frame's nodes go, this frame's list is spawned (the HUD is a few dozen nodes)
    for e in items.iter() {
        commands.entity(e).despawn();
    }
    stats.items = list.len();
    let mut tex_n = 0;
    for (z, (_, d)) in list.iter().enumerate() {
        let node = |r: [f64; 4]| Node {
            position_type: PositionType::Absolute,
            left: Val::Px(r[0] as f32),
            top: Val::Px(r[1] as f32),
            width: Val::Px(r[2].max(0.0) as f32),
            height: Val::Px(r[3].max(0.0) as f32),
            ..default()
        };
        let id = match d {
            Draw::Rect { rect, color } => commands.spawn((node(*rect), BackgroundColor(col(*color)))).id(),
            Draw::Image { rect, tex, tint, slice } => match texture(&mut st, &mut images, tex) {
                Some(h) => {
                    tex_n += 1;
                    let sz = images.get(&h).map(|i| i.size_f32()).unwrap_or(Vec2::ONE);
                    let mut img = ImageNode::new(h).with_color(col(*tint));
                    img.image_mode = match slice {
                        Some(m) => NodeImageMode::Sliced(TextureSlicer {
                            border: BorderRect { min_inset: Vec2::new(m[0] as f32 * sz.x, m[1] as f32 * sz.y), max_inset: Vec2::new(m[2] as f32 * sz.x, m[3] as f32 * sz.y) },
                            center_scale_mode: SliceScaleMode::Stretch,
                            sides_scale_mode: SliceScaleMode::Stretch,
                            max_corner_scale: 1.0,
                        }),
                        None => NodeImageMode::Stretch,
                    };
                    commands.spawn((node(*rect), img)).id()
                }
                None => {
                    if !stats.missing.contains(tex) {
                        stats.missing.push(tex.clone());
                    }
                    commands.spawn((node(*rect), BackgroundColor(col(*tint)))).id()
                }
            },
            Draw::Text { rect, text, typeface, px, color, shadow, halign, valign } => {
                let f = st.ui.fonts.get(typeface);
                let w = f.map_or(0.6 * px * text.chars().count() as f64, |a| a.width(text, *px));
                let h = f.map_or(1.2 * px, |a| a.height(*px));
                let x = rect[0] + (rect[2] - w) * [0.0, 0.5, 1.0][*halign as usize % 3];
                let y = rect[1] + (rect[3] - h) * [0.0, 0.5, 1.0][*valign as usize % 3];
                let font = st.font_handles.get(typeface).cloned().map(bevy::text::FontSource::Handle).unwrap_or_default();
                let mut ec = commands.spawn((
                    node([x, y, w + 2.0, h]),
                    Text::new(text.clone()),
                    TextFont { font, font_size: bevy::text::FontSize::Px(*px as f32), ..default() },
                    TextColor(col(*color)),
                    TextLayout::new(Justify::Left, LineBreak::NoWrap),
                ));
                if shadow[3] > 0.0 {
                    ec.insert(TextShadow { offset: Vec2::splat(1.0), color: col(*shadow) });
                }
                ec.id()
            }
        };
        commands.entity(id).insert((UiItem, ZIndex(z as i32)));
        commands.entity(root).add_child(id);
    }
    stats.textures = tex_n;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64, e: f64) -> bool {
        (a - b).abs() <= e
    }

    /// godot/tests/test_ui.gd test_hud_layout_from_widgets / test_main_menu_data / test_ffa_hud_layout numbers
    #[test]
    fn layouts_equal_godot_views() {
        let Ok(v) = mh_pak::Vfs::mount_default() else { panic!("paks") };
        let st = load_state(&Paks(Arc::new(v)));
        let ui = &st.ui;
        let l = duel_layout(ui, [1920.0, 1080.0]);
        assert_eq!(l.scale, 1.0);
        assert!(near(l.status[2], 749.0, 1e-3) && near(l.status[3], 32.0, 1e-3) && near(l.status[0], 585.5, 1e-3) && near(l.status[1], 984.0, 1e-3), "{:?}", l.status);
        assert!(near(l.health[0], 1010.0, 1e-3) && near(l.health[1], 985.0, 1e-3) && near(l.health[2], 321.75, 0.01), "{:?}", l.health);
        assert!(near(l.stamina[0], 960.0 - 372.33334, 0.01));
        assert_eq!((l.pips[0].len(), l.pips[1].len()), (5, 5));
        assert!(near(l.pips[0][0][0], 888.0, 1e-3) && near(l.pips[0][0][1], 32.0, 1e-3) && near(l.pips[0][0][2], 12.0, 1e-3));
        assert!(near(l.pips[1][0][0], 1020.0, 1e-3));
        assert!(near(l.announce[1], 110.0, 0.01) && near(l.announce[0], 960.0, 0.01));
        let l2 = duel_layout(ui, [1280.0, 720.0]);
        assert!(near(l2.scale, 0.666, 1e-4));
        let m = menu_layout(ui, [1920.0, 1080.0]);
        // start button inside the right column: (10, 12, 400 x 64) relative to the column
        assert!(near(m.start[0] - m.right[0], 10.0, 1e-3) && near(m.start[1] - m.right[1], 12.0, 1e-3) && near(m.start[2], 400.0, 1e-3) && near(m.start[3], 64.0, 1e-3), "{:?}", m.start);
        let rows = feed_layout(ui, [1920.0, 1080.0], &[("bot1".into(), "Longsword".into(), "player".into()), ("player".into(), "Kick".into(), "bot2".into())]);
        assert!(near(rows[0].0[0] + rows[0].0[2], 1920.0 - 100.03003, 0.01) && near(rows[0].0[1], 82.0, 0.01), "{:?}", rows[0].0);
        assert!(near(rows[1].0[1], rows[0].0[1] + rows[0].0[3], 0.01) && rows[0].0[3] >= 22.0);
    }

    /// test_status_bar_damage_and_regen / test_announcement_timing / test_kill_feed_model
    #[test]
    fn models_as_godot() {
        let k = kismet();
        assert!(k.ok() && k.has("sb_wait_on_drop"), "mode_kismet.json");
        let mut sb = StatusBarModel::new(k.clone(), None);
        let dt = 1.0 / 60.0;
        sb.tick(dt, Some(1), 100, 100);
        assert_eq!((sb.health_percent(), sb.health_text().as_str()), (1.0, "100"));
        sb.tick(dt, Some(1), 60, 100);
        assert_eq!(sb.displayed_health, 60.0);
        assert!(sb.delayed_health_target == 100.0 && sb.delayed_health_wait > 0.9);
        let mut t = 0.0;
        while sb.delayed_health_wait > 0.0 && t < 2.0 {
            sb.tick(dt, Some(1), 60, 100);
            t += dt;
        }
        assert!((t - 0.5).abs() <= dt * 1.5, "{t}");
        for _ in 0..120 {
            sb.tick(dt, Some(1), 60, 100);
        }
        assert!(near(sb.delayed_health, 60.0, 0.5));
        sb.tick(dt, Some(1), 35, 100);
        assert!(near(sb.health_pulse, 0.5, 0.01));
        let mut a = AnnouncementModel::new(0.5, 0.5);
        a.show("Fight!", "", 2.0);
        let mut t = 0.0;
        while a.showing && t < 5.0 {
            a.tick(dt);
            t += dt;
        }
        assert!(near(t, 2.5, dt * 2.5), "{t}");
        let mut f = KillFeedModel::new(None);
        for i in 0..10 {
            f.add(&format!("a{i}"), "Longsword", &format!("b{i}"), [1.0; 4], [1.0; 4]);
            f.tick(1.0);
        }
        assert_eq!((f.entries.len(), f.entries[0].killer.as_str()), (8, "a2"));
        f.add("same", "Fall", "same", [1.0; 4], [1.0; 4]);
        assert_eq!(f.entries.last().unwrap().killer, "");
    }
}

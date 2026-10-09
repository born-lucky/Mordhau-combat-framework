//! HUD and menu layouts in pixels, computed from the UMG packages exactly as the Godot views compute them (no renderer
//! needed, so they are unit-tested against the same numbers as godot/tests/test_ui.gd): duel_hud_view.gd layout(),
//! ffa_hud_view.gd feed_layout() / scoreboard_layout(), main_menu.gd _build(). Rects are [x, y, w, h].

use mh_assets::fonts::{font_px, Advances};
use mh_assets::umg::{CanvasLayout, Rect, UmgPackage};
use serde_json::Value;
use std::collections::HashMap;

pub const HUD: &str = "Mordhau/Content/Mordhau/UI/BP_HUDWidget";
pub const STATUS: &str = "Mordhau/Content/Mordhau/UI/BP_StatusBar";
pub const ANNOUNCE: &str = "Mordhau/Content/Mordhau/UI/BP_Announcement";
pub const DUEL_HUD: &str = "Mordhau/Content/Mordhau/Blueprints/GameModes/Duel/BP_DuelHUDWidget";
pub const SCORE: &str = "Mordhau/Content/Mordhau/Blueprints/GameModes/Group3v3/BP_TeamfightScoreWidget";
pub const ROUND_WINS: &str = "Mordhau/Content/Mordhau/UI/BP_DuelRoundWinsWidget";
pub const PIP: &str = "Mordhau/Content/Mordhau/UI/BP_RankedScoreElementEntry";
pub const VICTORY: &str = "Mordhau/Content/Mordhau/UI/BP_VictoryPopup";
pub const DEFEAT: &str = "Mordhau/Content/Mordhau/UI/BP_DefeatPopup";
pub const KILL_FEED: &str = "Mordhau/Content/Mordhau/UI/BP_KillFeed";
pub const KILL_FEED_ENTRY: &str = "Mordhau/Content/Mordhau/UI/BP_KillFeedEntry";
pub const ONE_TEAM: &str = "Mordhau/Content/Mordhau/UI/BP_OneTeamScoreboard";
pub const SB_ENTRY: &str = "Mordhau/Content/Mordhau/UI/BP_ScoreboardEntry";
pub const LOCAL_PLAY: &str = "Mordhau/Content/Mordhau/UI/BP_LocalPlay";
pub const FONT: &str = "Mordhau/Content/Mordhau/UI/UIAssets/Fonts/MordhauFont";

pub const PACKAGES: [&str; 14] = [HUD, STATUS, ANNOUNCE, DUEL_HUD, SCORE, ROUND_WINS, PIP, VICTORY, DEFEAT, KILL_FEED, KILL_FEED_ENTRY, ONE_TEAM, SB_ENTRY, LOCAL_PLAY];

/// The UMG packages + font measurers the layouts read
pub struct Ui {
    pub u: HashMap<&'static str, UmgPackage>,
    /// typeface name -> advances of its face (mh-assets fonts)
    pub fonts: HashMap<String, Advances>,
    pub scale_keys: Vec<(f64, f64)>,
}

fn f(v: Option<&Value>, k: &str) -> Option<f64> {
    v.and_then(|p| p.get(k)).and_then(Value::as_f64)
}

impl Ui {
    pub fn pkg(&self, p: &str) -> &UmgPackage {
        &self.u[p]
    }
    /// a canvas slot's layout (FAnchorData defaults when the widget has none)
    pub fn canvas(&self, p: &str, name: &str) -> CanvasLayout {
        self.u[p].slot(name).map(|s| s.canvas()).unwrap_or_default()
    }
    pub fn padding(&self, p: &str, name: &str) -> [f64; 4] {
        self.u[p].slot(name).map(|s| s.padding()).unwrap_or([0.0; 4])
    }
    /// USizeBox WidthOverride / HeightOverride
    pub fn size_override(&self, p: &str, name: &str) -> [f64; 2] {
        let pr = self.u[p].props(name);
        [f(pr, "WidthOverride").unwrap_or(0.0), f(pr, "HeightOverride").unwrap_or(0.0)]
    }
    /// USpacer Size (absent = (1, 1))
    pub fn spacer_size(&self, p: &str, name: &str) -> [f64; 2] {
        let s = self.u[p].props(name).and_then(|x| x.get("Size"));
        [f(s, "X").unwrap_or(1.0), f(s, "Y").unwrap_or(1.0)]
    }
    /// a canvas slot's stored size (Offsets Right / Bottom)
    pub fn slot_size(&self, p: &str, name: &str) -> [f64; 2] {
        let c = self.canvas(p, name);
        [c.offsets[2], c.offsets[3]]
    }
    pub fn default_f(&self, p: &str, key: &str) -> f64 {
        f(self.u[p].defaults(), key).unwrap_or(0.0)
    }
    pub fn ui_scale(&self, vp: [f64; 2]) -> f64 {
        mh_assets::umg::ui_scale(&self.scale_keys, vp)
    }
    /// text width / line height at scale 1 for a TextBlock's font (unknown typeface -> 0.6 em per char)
    pub fn text_size(&self, p: &str, name: &str, text: &str) -> [f64; 2] {
        let st = self.u[p].text_style(name);
        let (tf, sz) = st.and_then(|s| s.font).map(|f| (f.typeface, f.size)).unwrap_or_default();
        let px = font_px(sz, 1.0) as f64;
        match self.fonts.get(&tf) {
            Some(a) => [a.width(text, px), a.height(px)],
            None => [0.6 * px * text.chars().count() as f64, 1.2 * px],
        }
    }
}

fn px(r: Rect, s: f64) -> Rect {
    [r[0] * s, r[1] * s, r[2] * s, r[3] * s]
}

/// duel_hud_view.gd layout(viewport)
#[derive(Clone, Debug, Default)]
pub struct DuelLayout {
    pub scale: f64,
    pub status: Rect,
    pub health: Rect,
    pub stamina: Rect,
    pub stamina_icon: Rect,
    pub health_icon: Rect,
    pub stamina_text: Rect,
    pub health_text: Rect,
    pub announce: Rect,
    pub pips: [Vec<Rect>; 2],
    pub popup_center: [f64; 2],
}

pub fn duel_layout(ui: &Ui, vp: [f64; 2]) -> DuelLayout {
    let s = ui.ui_scale(vp);
    let root = [0.0, 0.0, vp[0] / s, vp[1] / s];
    let sb = ui.pkg(STATUS);
    let cont = sb.brush("container_Image").image_size;
    let sbr = ui.canvas(HUD, "VerticalBox_0").rect(root, cont);
    let bars = [sbr[0], sbr[1], cont[0], cont[1]];
    let health = ui.canvas(STATUS, "Health").rect(bars, [0.0; 2]);
    let stamina = ui.canvas(STATUS, "Stamina").rect(bars, [0.0; 2]);
    let mid_sz = ui.size_override(STATUS, "middle_SizeBox");
    let mid = [bars[0] + (bars[2] - mid_sz[0]) * 0.5, bars[1] + (bars[3] - mid_sz[1]) * 0.5, mid_sz[0], mid_sz[1]];
    let stamina_icon = ui.canvas(STATUS, "StaminaIcon").rect(mid, sb.brush("StaminaIcon").image_size);
    let health_icon = ui.canvas(STATUS, "HealthIcon").rect(mid, sb.brush("HealthIcon").image_size);
    let stamina_text = ui.canvas(STATUS, "SizeBox_1").rect(mid, [0.0; 2]);
    let health_text = ui.canvas(STATUS, "SizeBox_2").rect(mid, [0.0; 2]);
    let announce = ui.canvas(HUD, "AnnouncementContainer").rect(root, [0.0; 2]);
    let outer = ui.canvas(DUEL_HUD, "BP_TeamfightScoreWidget").rect(root, [0.0; 2]);
    let inner = ui.canvas(SCORE, "Overlay_0").rect([outer[0], outer[1], outer[2], 0.0], [0.0; 2]);
    let n = ui.default_f(ROUND_WINS, "Rounds to win") as usize;
    let pip = ui.pkg(PIP).brush("Image_0").image_size;
    let row_w = pip[0] * n as f64;
    let spacer = ui.spacer_size(SCORE, "Spacer_0");
    let total = row_w * 2.0 + spacer[0];
    let x0 = inner[0] + inner[2] * 0.5 - total * 0.5;
    let mut pips: [Vec<Rect>; 2] = [vec![], vec![]];
    for i in 0..n {
        pips[0].push(px([x0 + i as f64 * pip[0], inner[1], pip[0], pip[1]], s));
        let rx = x0 + row_w + spacer[0] + (n - 1 - i) as f64 * pip[0];
        pips[1].push(px([rx, inner[1], pip[0], pip[1]], s));
    }
    DuelLayout {
        scale: s,
        status: px(bars, s),
        health: px(health, s),
        stamina: px(stamina, s),
        stamina_icon: px(stamina_icon, s),
        health_icon: px(health_icon, s),
        stamina_text: px(stamina_text, s),
        health_text: px(health_text, s),
        announce: px(announce, s),
        pips,
        popup_center: [root[2] * 0.5 * s, root[3] * 0.5 * s],
    }
}

/// ffa_hud_view.gd feed_layout: per entry the row rect and the killer / weapon / victim rects (pixels)
pub fn feed_layout(ui: &Ui, vp: [f64; 2], entries: &[(String, String, String)]) -> Vec<(Rect, [Rect; 3])> {
    let s = ui.ui_scale(vp);
    let root = [0.0, 0.0, vp[0] / s, vp[1] / s];
    let cont = ui.canvas(HUD, "KillFeedContainer").rect(root, [0.0; 2]);
    let row_h0 = ui.pkg(KILL_FEED_ENTRY).brush("Image_14").image_size[1].max(0.0);
    let pad = ui.padding(KILL_FEED_ENTRY, "HorizontalBox_953");
    let mut out = Vec::new();
    let mut y = cont[1];
    for (k, w_, v) in entries {
        let mut row_h = row_h0;
        let mut parts = vec![];
        let mut w = pad[0] + pad[2];
        for (nm, t) in [("TextBlock_Killer", k), ("TextBlock_Weapon", w_), ("TextBlock_Victim", v)] {
            let sz = ui.text_size(KILL_FEED_ENTRY, nm, t);
            let p = ui.padding(KILL_FEED_ENTRY, nm);
            parts.push((p[0], sz[0]));
            w += p[0] + sz[0];
            row_h = row_h.max(sz[1]);
        }
        let r = [cont[0] + cont[2] - w, y, w, row_h];
        let mut x = r[0] + pad[0];
        let mut rects = [[0.0; 4]; 3];
        for (i, (pl, tw)) in parts.iter().enumerate() {
            x += pl;
            rects[i] = px([x, y, *tw, row_h], s);
            x += tw;
        }
        out.push((px(r, s), rects));
        y += row_h;
    }
    out
}

/// ffa_hud_view.gd scoreboard_layout (one team): the panel and one rect per row
pub fn scoreboard_layout(ui: &Ui, vp: [f64; 2], rows: usize) -> (Rect, Vec<Rect>) {
    let s = ui.ui_scale(vp);
    let row = ui.size_override(SB_ENTRY, "SizeBox_5840");
    let head = 64.0; // header block height (ribbon art not drawn, UNCONFIRMED in the reference)
    let total = [row[0], head + row[1] * rows as f64];
    let root = [0.0, 0.0, vp[0] / s, vp[1] / s];
    let r = ui.canvas(HUD, "ScoreboardContainer").rect(root, total);
    let rects = (0..rows).map(|i| px([r[0], r[1] + head + row[1] * i as f64, row[0], row[1]], s)).collect();
    (px(r, s), rects)
}

/// main_menu.gd _build: the BP_LocalPlay rects (pixels)
#[derive(Clone, Debug, Default)]
pub struct MenuLayout {
    pub scale: f64,
    pub dim: Rect,
    pub left: Rect,
    pub right: Rect,
    pub head: Rect,
    pub head_text: Rect,
    pub map_entry: Rect,
    pub start: Rect,
    pub mode_label: Rect,
    pub mode_box: Rect,
}

pub fn menu_layout(ui: &Ui, vp: [f64; 2]) -> MenuLayout {
    let s = ui.ui_scale(vp);
    let root = [0.0, 0.0, vp[0] / s, vp[1] / s];
    let lp = ui.pkg(LOCAL_PLAY);
    let dim = ui.canvas(LOCAL_PLAY, "Overlay").rect(root, [0.0; 2]);
    let panel = ui.canvas(LOCAL_PLAY, "HorizontalBox_2").rect(root, ui.slot_size(LOCAL_PLAY, "HorizontalBox_2"));
    let rw = ui.size_override(LOCAL_PLAY, "SizeBox_16")[0];
    let left = [panel[0], panel[1], panel[2] - rw, panel[3]];
    let right = [panel[0] + panel[2] - rw, panel[1], rw, panel[3]];
    let pad = ui.padding(LOCAL_PLAY, "VerticalBox_61");
    let hsz = lp.brush("Image_4").image_size;
    let head = [left[0] + pad[0], left[1] + pad[1], left[2] - pad[0], hsz[1]];
    let tp = ui.padding(LOCAL_PLAY, "Text");
    let head_text = [head[0] + tp[0], head[1] + tp[1], head[2] - tp[0] - tp[2], hsz[1]];
    let map_entry = [head[0], head[1] + hsz[1] + 8.0, head[2], 48.0];
    let start = ui.canvas(LOCAL_PLAY, "StartButton").rect(right, [0.0; 2]);
    let gm = ui.canvas(LOCAL_PLAY, "VerticalBox_1").rect(right, [0.0; 2]);
    let mode_label = [gm[0], gm[1], 300.0, 24.0];
    let bx = ui.size_override(LOCAL_PLAY, "SizeBox_64");
    let mode_box = [gm[0], gm[1] + 24.0 + 4.0, bx[0], bx[1]];
    MenuLayout {
        scale: s,
        dim: px(dim, s),
        left: px(left, s),
        right: px(right, s),
        head: px(head, s),
        head_text: px(head_text, s),
        map_entry: px(map_entry, s),
        start: px(start, s),
        mode_label: px(mode_label, s),
        mode_box: px(mode_box, s),
    }
}

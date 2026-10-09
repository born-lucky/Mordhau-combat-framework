//! UMG WidgetBlueprints as an engine-neutral UI tree, read from the paks (mh-pak export JSON). Port of
//! godot/components/ue/records/umg.gd (its value reads, Slate layout rules and defaults; UE 4.26 UMG/Slate, from the UE
//! source as umg.gd states, UNCONFIRMED unless noted) plus the tree itself:
//!   WidgetTree.RootWidget -> widget; a panel / content widget's `Slots` -> slot exports, each slot's `Content` -> the
//!   child widget (UPanelWidget / UPanelSlot); a widget's `Slot` -> its slot in the parent.
//! Values stay as stored (UE units = Slate units, colours linear); `srgb` converts a linear colour the way umg.gd does
//! (Color.linear_to_srgb, alpha kept). `canvas_rect` is UCanvasPanelSlot layout, `ui_scale` the DPI curve
//! (UUserInterfaceSettings ShortestSide over UIScaleCurve, DefaultEngine.ini), `font_px` in fonts.rs.

use crate::material::strip_index;
use mh_pak::Reader;
use serde_json::Value;
use std::collections::BTreeMap;

/// FAnchorData (+ UCanvasPanelSlot bAutoSize, ZOrder): offsets (Left, Top, Right, Bottom), anchors min / max, alignment
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CanvasLayout {
    pub offsets: [f64; 4],
    pub anchor_min: [f64; 2],
    pub anchor_max: [f64; 2],
    pub alignment: [f64; 2],
    pub auto_size: bool,
    pub z_order: i64,
}

/// A rect: x, y, width, height
pub type Rect = [f64; 4];

impl CanvasLayout {
    /// UCanvasPanelSlot rect inside `parent` (umg.gd canvas_rect): point-anchored axis: size = Right/Bottom (desired
    /// with bAutoSize), position = anchor + Left/Top - alignment x size; stretched axis: Left/Right (Top/Bottom) are
    /// margins from the anchors
    pub fn rect(&self, parent: Rect, desired: [f64; 2]) -> Rect {
        let mut out = [0.0; 4];
        for ax in 0..2 {
            let (p0, ps) = (parent[ax], parent[2 + ax]);
            let (lo, hi) = (self.offsets[ax], self.offsets[2 + ax]);
            if (self.anchor_min[ax] - self.anchor_max[ax]).abs() < 1e-5 {
                let size = if self.auto_size { desired[ax] } else { hi };
                out[2 + ax] = size;
                out[ax] = p0 + self.anchor_min[ax] * ps + lo - self.alignment[ax] * size;
            } else {
                let a = p0 + self.anchor_min[ax] * ps + lo;
                let b = p0 + self.anchor_max[ax] * ps - hi;
                out[ax] = a;
                out[2 + ax] = b - a;
            }
        }
        out
    }
}

/// FSlateBrush (umg.gd Brush): ImageSize absent = 32 x 32, TintColor absent = white, DrawAs absent = Image
#[derive(Debug, Clone, PartialEq)]
pub struct Brush {
    /// ResourceObject (Texture2D / material package) and its class
    pub resource: Option<(String, String)>,
    pub image_size: [f64; 2],
    /// ESlateBrushDrawType ("ESlateBrushDrawType::Box", ...; "" = the default Image)
    pub draw_as: String,
    /// FMargin (Left, Top, Right, Bottom)
    pub margin: [f64; 4],
    /// linear RGBA
    pub tint: [f64; 4],
    pub tiling: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontInfo {
    pub font_object: String,
    pub typeface: String,
    /// points (FSlateFontInfo Size, default 24)
    pub size: f64,
}

/// UTextBlock: ColorAndOpacity absent = white, ShadowColorAndOpacity absent = transparent (UTextBlock ctor)
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub text: String,
    pub font: Option<FontInfo>,
    pub color: [f64; 4],
    pub shadow: [f64; 4],
    pub shadow_offset: [f64; 2],
    /// ETextJustify ("ETextJustify::Center"; "" = Left)
    pub justification: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub class: String,
    pub props: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    pub name: String,
    pub class: String,
    pub props: Value,
    pub slot: Option<Slot>,
    pub children: Vec<Widget>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub name: String,
    pub movie_scene: String,
    /// PlaybackRange upper bound in ticks, inclusive (umg.gd movie_scene_last_tick; -1 unknown)
    pub last_tick: f64,
    /// TickResolution / DisplayRate numerators (frames per second x denominator)
    pub tick_resolution: f64,
    /// MovieScene track classes
    pub tracks: Vec<String>,
}

pub struct UmgPackage {
    pub package: String,
    pub exports: Vec<Value>,
    pub root: Option<Widget>,
    /// widgets by name: exports with a Slot / Slots or of type TextBlock / Image / ProgressBar (umg.gd by_name)
    pub by_name: BTreeMap<String, usize>,
    pub animations: Vec<Animation>,
}

fn f(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn v2(v: Option<&Value>, d: [f64; 2]) -> [f64; 2] {
    match v {
        Some(v) if v.is_object() => [f(v, "X", d[0]), f(v, "Y", d[1])],
        _ => d,
    }
}
fn ref_index(v: Option<&Value>) -> Option<usize> {
    let op = v?.get("ObjectPath")?.as_str()?;
    op.rsplit_once('.')?.1.parse().ok()
}

/// FLinearColor or FSlateColor {SpecifiedColor} -> linear RGBA; `d` when absent / not a colour (umg.gd color)
pub fn color(v: Option<&Value>, d: [f64; 4]) -> [f64; 4] {
    let Some(mut o) = v.filter(|v| v.is_object()) else { return d };
    if let Some(s) = o.get("SpecifiedColor") {
        o = s;
    }
    if o.get("R").is_none() {
        return d;
    }
    [f(o, "R", 0.0), f(o, "G", 0.0), f(o, "B", 0.0), f(o, "A", 1.0)]
}

/// linear -> sRGB per channel, alpha kept (Godot Color.linear_to_srgb, as umg.gd draws colours)
pub fn srgb(c: [f64; 4]) -> [f64; 4] {
    let s = |x: f64| if x < 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
    [s(c[0]), s(c[1]), s(c[2]), c[3]]
}

/// FMargin (Left, Top, Right, Bottom), missing = 0
pub fn margin(v: Option<&Value>) -> [f64; 4] {
    match v {
        Some(o) if o.is_object() => [f(o, "Left", 0.0), f(o, "Top", 0.0), f(o, "Right", 0.0), f(o, "Bottom", 0.0)],
        _ => [0.0; 4],
    }
}

/// FText -> string (UeRec text_or: LocalizedString, SourceString, CultureInvariantString)
pub fn text(v: Option<&Value>) -> String {
    let Some(v) = v else { return String::new() };
    for k in ["LocalizedString", "SourceString", "CultureInvariantString"] {
        if let Some(s) = v.get(k).and_then(Value::as_str) {
            return s.to_string();
        }
    }
    v.as_str().unwrap_or("").to_string()
}

/// An FSlateBrush value (umg.gd brush_of)
pub fn brush(v: Option<&Value>) -> Brush {
    let null = Value::Null;
    let b = v.unwrap_or(&null);
    let resource = b.get("ResourceObject").filter(|r| r.is_object()).map(|r| {
        let on = r.get("ObjectName").and_then(Value::as_str).unwrap_or("");
        (strip_index(r.get("ObjectPath").and_then(Value::as_str).unwrap_or("")).to_string(), on.split('\'').next().unwrap_or("").to_string())
    });
    Brush {
        resource,
        image_size: v2(b.get("ImageSize"), [32.0, 32.0]),
        draw_as: b.get("DrawAs").and_then(Value::as_str).unwrap_or("").to_string(),
        margin: margin(b.get("Margin")),
        tint: if b.get("TintColor").is_some() { color(b.get("TintColor"), [1.0; 4]) } else { [1.0; 4] },
        tiling: b.get("Tiling").and_then(Value::as_str).unwrap_or("").to_string(),
    }
}

impl Widget {
    pub fn find(&self, name: &str) -> Option<&Widget> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }
    pub fn walk<'a>(&'a self, out: &mut Vec<&'a Widget>) {
        out.push(self);
        for c in &self.children {
            c.walk(out);
        }
    }
}

impl Slot {
    /// UCanvasPanelSlot LayoutData (FAnchorData defaults: offsets 0, anchors 0, alignment 0)
    pub fn canvas(&self) -> CanvasLayout {
        let null = Value::Null;
        let ld = self.props.get("LayoutData").unwrap_or(&null);
        let off = ld.get("Offsets").unwrap_or(&null);
        let an = ld.get("Anchors").unwrap_or(&null);
        CanvasLayout {
            offsets: [f(off, "Left", 0.0), f(off, "Top", 0.0), f(off, "Right", 0.0), f(off, "Bottom", 0.0)],
            anchor_min: v2(an.get("Minimum"), [0.0; 2]),
            anchor_max: v2(an.get("Maximum"), [0.0; 2]),
            alignment: v2(ld.get("Alignment"), [0.0; 2]),
            auto_size: self.props.get("bAutoSize").and_then(Value::as_bool).unwrap_or(false),
            z_order: self.props.get("ZOrder").and_then(Value::as_i64).unwrap_or(0),
        }
    }
    /// Padding (absent = 0, the panel slot ctors)
    pub fn padding(&self) -> [f64; 4] {
        margin(self.props.get("Padding"))
    }
    /// EHorizontalAlignment / EVerticalAlignment as stored ("" = the slot class default)
    pub fn align(&self) -> (String, String) {
        let s = |k: &str| self.props.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        (s("HorizontalAlignment"), s("VerticalAlignment"))
    }
    /// FSlateChildSize of box slots: (rule "ESlateSizeRule::Fill" / "" = Automatic, value default 1)
    pub fn size(&self) -> (String, f64) {
        let s = self.props.get("Size");
        (s.and_then(|s| s.get("SizeRule")).and_then(Value::as_str).unwrap_or("").to_string(), s.map_or(1.0, |s| f(s, "Value", 1.0)))
    }
}

impl UmgPackage {
    pub fn read(rd: &Reader, pkg: &str) -> Option<UmgPackage> {
        let pkg = strip_index(pkg).to_string();
        let exports = rd.read(&pkg)?;
        let mut by_name = BTreeMap::new();
        for (i, e) in exports.iter().enumerate() {
            let p = e.get("Properties");
            let t = e.get("Type").and_then(Value::as_str).unwrap_or("");
            if p.is_some_and(|p| p.get("Slot").is_some() || p.get("Slots").is_some()) || ["TextBlock", "Image", "ProgressBar"].contains(&t) {
                by_name.insert(e.get("Name").and_then(Value::as_str).unwrap_or("").to_string(), i);
            }
        }
        let mut u = UmgPackage { package: pkg, exports, root: None, by_name, animations: vec![] };
        let tree = u.exports.iter().position(|e| e.get("Type").and_then(Value::as_str) == Some("WidgetTree") && e.get("Outer").is_none_or(|o| !o.to_string().contains("WidgetTree")));
        if let Some(t) = tree {
            let r = ref_index(u.exports[t].pointer("/Properties/RootWidget"));
            u.root = r.and_then(|i| u.widget_at(i, None, 0));
        }
        u.animations = u.read_animations();
        Some(u)
    }

    fn widget_at(&self, i: usize, slot: Option<Slot>, depth: usize) -> Option<Widget> {
        let e = self.exports.get(i)?;
        if depth > 64 {
            return None;
        }
        let null = Value::Null;
        let p = e.get("Properties").unwrap_or(&null);
        let mut children = Vec::new();
        for s in p.get("Slots").and_then(Value::as_array).into_iter().flatten() {
            let Some(si) = ref_index(Some(s)) else { continue };
            let Some(se) = self.exports.get(si) else { continue };
            let sp = se.get("Properties").cloned().unwrap_or(Value::Null);
            let sl = Slot { class: se.get("Type").and_then(Value::as_str).unwrap_or("").to_string(), props: sp.clone() };
            if let Some(ci) = ref_index(sp.get("Content")) {
                if let Some(w) = self.widget_at(ci, Some(sl), depth + 1) {
                    children.push(w);
                }
            }
        }
        Some(Widget {
            name: e.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
            class: e.get("Type").and_then(Value::as_str).unwrap_or("").to_string(),
            props: p.clone(),
            slot,
            children,
        })
    }

    /// A widget export's properties by name (umg.gd props)
    pub fn props(&self, name: &str) -> Option<&Value> {
        self.by_name.get(name).and_then(|&i| self.exports[i].get("Properties"))
    }
    /// A widget's slot (umg.gd slot)
    pub fn slot(&self, name: &str) -> Option<Slot> {
        let si = ref_index(self.props(name)?.get("Slot"))?;
        let se = self.exports.get(si)?;
        Some(Slot { class: se.get("Type").and_then(Value::as_str).unwrap_or("").to_string(), props: se.get("Properties").cloned().unwrap_or(Value::Null) })
    }
    pub fn class(&self, name: &str) -> &str {
        self.by_name.get(name).and_then(|&i| self.exports[i].get("Type")).and_then(Value::as_str).unwrap_or("")
    }

    /// UTextBlock text style (umg.gd text_style / text_of)
    pub fn text_style(&self, name: &str) -> Option<TextStyle> {
        let p = self.props(name)?;
        let font = p.get("Font").map(|ft| FontInfo {
            font_object: strip_index(ft.pointer("/FontObject/ObjectPath").and_then(Value::as_str).unwrap_or("")).to_string(),
            typeface: ft.get("TypefaceFontName").and_then(Value::as_str).unwrap_or("").to_string(),
            size: f(ft, "Size", crate::fonts::SLATE_DEFAULT_FONT_SIZE),
        });
        Some(TextStyle {
            text: text(p.get("Text")),
            font,
            color: if p.get("ColorAndOpacity").is_some() { color(p.get("ColorAndOpacity"), [1.0; 4]) } else { [1.0; 4] },
            shadow: if p.get("ShadowColorAndOpacity").is_some() { color(p.get("ShadowColorAndOpacity"), [0.0; 4]) } else { [0.0; 4] },
            shadow_offset: v2(p.get("ShadowOffset"), [1.0, 1.0]),
            justification: p.get("Justification").and_then(Value::as_str).unwrap_or("").to_string(),
        })
    }
    /// UImage Brush
    pub fn brush(&self, name: &str) -> Brush {
        brush(self.props(name).and_then(|p| p.get("Brush")))
    }
    /// UImage ColorAndOpacity (absent = white)
    pub fn image_color(&self, name: &str) -> [f64; 4] {
        color(self.props(name).and_then(|p| p.get("ColorAndOpacity")), [1.0; 4])
    }
    /// UProgressBar WidgetStyle BackgroundImage / FillImage tints (absent = white, as umg.gd's color(null))
    pub fn progress_tints(&self, name: &str) -> ([f64; 4], [f64; 4]) {
        let ws = self.props(name).and_then(|p| p.get("WidgetStyle"));
        let t = |k: &str| color(ws.and_then(|w| w.get(k)).and_then(|b| b.get("TintColor")), [1.0; 4]);
        (t("BackgroundImage"), t("FillImage"))
    }
    /// UButton WidgetStyle Normal / Hovered / Pressed
    pub fn button_brushes(&self, name: &str) -> [Brush; 3] {
        let ws = self.props(name).and_then(|p| p.get("WidgetStyle"));
        let b = |k: &str| brush(ws.and_then(|w| w.get(k)));
        [b("Normal"), b("Hovered"), b("Pressed")]
    }
    /// the widget class's CDO (Blueprint variables)
    pub fn defaults(&self) -> Option<&Value> {
        self.exports.iter().find(|e| e.get("Name").and_then(Value::as_str).is_some_and(|n| n.starts_with("Default__"))).and_then(|e| e.get("Properties"))
    }

    fn read_animations(&self) -> Vec<Animation> {
        let mut out = Vec::new();
        for e in &self.exports {
            if e.get("Type").and_then(Value::as_str) != Some("WidgetAnimation") {
                continue;
            }
            let null = Value::Null;
            let p = e.get("Properties").unwrap_or(&null);
            let Some(ms) = ref_index(p.get("MovieScene")).and_then(|i| self.exports.get(i)) else { continue };
            let mp = ms.get("Properties").unwrap_or(&null);
            let tracks = mp
                .get("MasterTracks")
                .into_iter()
                .chain(mp.get("ObjectBindings").and_then(Value::as_array).into_iter().flatten().filter_map(|b| b.get("Tracks")))
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(|t| ref_index(Some(t)).and_then(|i| self.exports.get(i)).and_then(|x| x.get("Type")).and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            out.push(Animation {
                name: e.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
                movie_scene: ms.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
                last_tick: movie_scene_last_tick(ms),
                tick_resolution: mp.pointer("/TickResolution/Numerator").and_then(Value::as_f64).unwrap_or(60000.0),
                tracks,
            });
        }
        out
    }

    /// last tick of the package's MovieScene export named `name` (-1 when none)
    pub fn movie_scene_last_tick(&self, name: &str) -> f64 {
        self.exports
            .iter()
            .find(|e| e.get("Type").and_then(Value::as_str) == Some("MovieScene") && e.get("Name").and_then(Value::as_str) == Some(name))
            .map_or(-1.0, movie_scene_last_tick)
    }
}

/// A MovieScene's PlaybackRange upper bound, inclusive (ERangeBoundTypes::Exclusive = 0 -> -1 tick; umg.gd
/// movie_scene_last_tick); -1 when absent
pub fn movie_scene_last_tick(ms: &Value) -> f64 {
    let Some(ub) = ms.pointer("/Properties/PlaybackRange/Value/UpperBound") else { return -1.0 };
    let Some(v) = ub.pointer("/Value/Value").and_then(Value::as_f64) else { return -1.0 };
    let ty = ub.get("Type");
    let excl = ty.and_then(Value::as_i64) == Some(0) || ty.and_then(Value::as_str).is_some_and(|s| s.ends_with("Exclusive"));
    if excl { v - 1.0 } else { v }
}

/// UIScaleCurve keys (Time = shortest viewport side, Value = scale) from DefaultEngine.ini
/// [/Script/Engine.UserInterfaceSettings] (umg.gd _keys)
pub fn ui_scale_keys(rd: &Reader) -> Vec<(f64, f64)> {
    let ini = rd.file("Mordhau/Config/DefaultEngine.ini").map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let mut out = Vec::new();
    for l in ini.lines().filter(|l| l.starts_with("UIScaleCurve=")) {
        for part in l.split("Time=").skip(1) {
            let t: f64 = part.split(',').next().and_then(|x| x.parse().ok()).unwrap_or(f64::NAN);
            let v: f64 = part.split("Value=").nth(1).and_then(|x| x.split([')', ',']).next()).and_then(|x| x.parse().ok()).unwrap_or(f64::NAN);
            if t.is_finite() && v.is_finite() {
                out.push((t, v));
            }
        }
    }
    if out.is_empty() {
        out.push((1080.0, 1.0));
    }
    out
}

/// DPI scale for a viewport: ShortestSide rule, linear keys, constant extrapolation (umg.gd ui_scale)
pub fn ui_scale(keys: &[(f64, f64)], viewport: [f64; 2]) -> f64 {
    let x = viewport[0].min(viewport[1]);
    if x <= keys[0].0 {
        return keys[0].1;
    }
    for w in keys.windows(2) {
        if x <= w[1].0 {
            return w[0].1 + (w[1].1 - w[0].1) * (x - w[0].0) / (w[1].0 - w[0].0);
        }
    }
    keys[keys.len() - 1].1
}

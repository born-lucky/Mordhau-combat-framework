//! Slate layout and paint of the live widget objects (vm.rs), the way UMG's widgets build their Slate counterparts:
//! ComputeDesiredSize bottom-up, OnArrangeChildren top-down, OnPaint into draw elements. Rules follow the exe's Slate
//! functions (names and rvas from the PDB, extract/native/labels.tsv); their bodies are read as the UE 4.26 source they
//! were compiled from (UNCONFIRMED where noted: not disassembled):
//!   SBoxPanel::ComputeDesiredSize 0x1c77f00, SBoxPanel::OnArrangeChildren 0x1c827e0 (ArrangeChildrenAlong)
//!   SOverlay::ComputeDesiredSize 0x1c78120, SOverlay::OnArrangeChildren 0x1c82840
//!   SConstraintCanvas::ComputeDesiredSize 0x1d83be0, SConstraintCanvas::ArrangeLayeredChildren 0x1d818d0
//!   SBox::ComputeDesiredSize 0x1d83a30, SBox::OnArrangeChildren 0x1d9fa40
//!   SBorder::ComputeDesiredSize 0x1d83980, SBorder::OnPaint 0x1dad9f0
//!   SImage::ComputeDesiredSize 0x1c78090, SImage::OnPaint 0x1c84d00
//!   SScaleBox::ComputeContentScale 0x1d83400, SScaleBox::OnArrangeChildren 0x1da1ab0
//!   SWidgetSwitcher::ComputeDesiredSize 0x1d84e50, SWidgetSwitcher::OnArrangeChildren 0x1da42b0
//!   SButton::ComputeDesiredSize 0x1d444f0, SButton::OnPaint 0x1d65bf0
//!   SProgressBar::OnPaint 0x1de7c10, SSpacer::ComputeDesiredSize 0x1d84810, STextBlock::ComputeDesiredSize 0x1dc89b0
//!   SUniformGridPanel::OnArrangeChildren 0x1da3bd0, SGridPanel::OnArrangeChildren 0x1da0770,
//!   SWrapBox::OnArrangeChildren 0x1da4a10, SScrollPanel::OnArrangeChildren 0x1da2fa0
//!   FSlateDrawElement::MakeBox 0x1c65a60 (Image / Box / Border brush draw types)
//! Widget and slot defaults are the UMG constructors' (UTextBlock::UTextBlock 0x2bad6e0, UBorder::UBorder 0x2ba84b0,
//! UImage::UImage 0x2baa620, UButton::UButton 0x2ba8740, USizeBox::USizeBox 0x2bace90, UScaleBox::UScaleBox 0x2bac970,
//! UOverlaySlot::UOverlaySlot 0x2bac3a0, UHorizontalBoxSlot::UHorizontalBoxSlot 0x2baa5c0,
//! UVerticalBoxSlot::UVerticalBoxSlot 0x2bae240, UCanvasPanelSlot::UCanvasPanelSlot 0x2ba8980): values UNCONFIRMED
//! (4.26 source), except where a package shows them (a slot that stores Offsets Right/Bottom = 0 explicitly proves
//! the 100 x 30 default is non-zero).
//!
//! Units: layout runs in Slate units; a geometry carries the absolute position (pixels) and the accumulated scale
//! (DPI scale from the UI settings curve x ScaleBox / render-transform scales).

use crate::model::*;
use crate::vm::Vm;
use mh_assets::fonts::{Advances, CompositeFont};
use mh_assets::pak_source::PakSource;
use std::collections::HashMap;
use std::rc::Rc;

pub type Rect = [f64; 4];

/// The engine's default UMG font (UWidget::GetDefaultFontName, UTextBlock ctor: Roboto, 24, "Bold")
pub const DEFAULT_FONT: &str = "Engine/Content/EngineFonts/Roboto";

/// A draw primitive in absolute pixels
#[derive(Clone, Debug)]
pub enum Prim {
    Rect { rect: Rect, color: [f64; 4] },
    /// texture package; `uv` = sub-rectangle in texture pixels (9-slice pieces), None = whole texture
    Image { rect: Rect, tex: String, tint: [f64; 4], uv: Option<Rect> },
    /// one line of text; `face` = the font face package, `px` = pixel size, rect top-left = line top
    Text { rect: Rect, text: String, face: String, px: f64, color: [f64; 4] },
}

/// (rust-render) one UBackgroundBlur widget painted this frame: SBackgroundBlur::OnPaint (rva 0x1dae580) turns it into
/// FSlateDrawElement::MakePostProcessPass; the kernel / downsample math and the blur itself run in mh-runtime
/// (uepost_render.rs ui_blur). `rect` = the widget's absolute rect (layout px, as Item rects); `strength` = BlurStrength
/// x the widget style's ColorAndOpacityTint alpha when bApplyAlphaToBlur (OnPaint 0x1dae614-0x1dae639); `radius` =
/// BlurRadius when bOverrideAutoRadiusCalculation (UBackgroundBlur::SynchronizeProperties 0x2c27080 -> SetBlurRadius);
/// `layer` = the number of items painted before it (the blur sits under every later item).
#[derive(Clone, Debug, PartialEq)]
pub struct BlurEl {
    pub rect: Rect,
    pub clip: Option<Rect>,
    pub strength: f64,
    pub radius: Option<i32>,
    pub layer: usize,
}

/// (rust-render) the frame's blur elements, for the renderer (real.rs `draw` writes it; mh-runtime extracts it).
/// `size` = the layout viewport the rects are in.
#[derive(bevy::prelude::Resource, Clone, Debug, Default)]
pub struct UiBlurs {
    pub els: Vec<BlurEl>,
    pub size: [f64; 2],
}

#[derive(Clone, Debug)]
pub struct Item {
    pub prim: Prim,
    pub clip: Option<Rect>,
    pub widget: Id,
}

/// A hit-testable widget's absolute rect (later = on top)
#[derive(Clone, Debug)]
pub struct Hit {
    pub rect: Rect,
    pub widget: Id,
    pub clip: Option<Rect>,
}

pub struct Face {
    pub package: String,
    pub adv: Advances,
    pub data: Rc<Vec<u8>>,
}

/// Fonts and texture metadata, cached
pub struct Res {
    pub src: PakSource,
    pub rd: mh_pak::Reader,
    fonts: HashMap<String, Option<Rc<CompositeFont>>>,
    pub faces: HashMap<String, Option<Rc<Face>>>,
    tex: HashMap<String, Option<(f64, f64)>>,
    /// URichTextBlock TextStyleSet tables: row name -> FRichTextStyleRow.TextStyle (FTextBlockStyle)
    rich: HashMap<String, Rc<Vec<(String, V)>>>,
}

impl Res {
    pub fn new(vfs: std::sync::Arc<mh_pak::Vfs>) -> Res {
        Res { src: PakSource::new(vfs.clone()), rd: mh_pak::Reader::new(vfs), fonts: HashMap::new(), faces: HashMap::new(), tex: HashMap::new(), rich: HashMap::new() }
    }
    fn composite(&mut self, pkg: &str) -> Option<Rc<CompositeFont>> {
        if let Some(f) = self.fonts.get(pkg) {
            return f.clone();
        }
        let f = mh_assets::fonts::composite(&self.rd, pkg).map(Rc::new);
        self.fonts.insert(pkg.to_string(), f.clone());
        f
    }
    /// the face for (font object, typeface) (CompositeFont::face: the named default-typeface entry, else the first)
    pub fn face(&mut self, font: &str, typeface: &str) -> Option<Rc<Face>> {
        let font = if font.is_empty() { DEFAULT_FONT } else { font };
        let cf = self.composite(font).or_else(|| self.composite(DEFAULT_FONT))?;
        let e = cf.face(typeface)?;
        let key = e.face.clone();
        if let Some(f) = self.faces.get(&key) {
            return f.clone();
        }
        let f = mh_assets::fonts::face(&self.rd, &key).and_then(|ff| {
            let adv = Advances::new(&ff.data)?;
            Some(Rc::new(Face { package: key.clone(), adv, data: Rc::new(ff.data) }))
        });
        self.faces.insert(key, f.clone());
        f
    }
    /// a rich text style DataTable's rows in table order (UDataTable::LoadStructData rows of FRichTextStyleRow)
    pub fn rich_styles(&mut self, pkg: &str) -> Rc<Vec<(String, V)>> {
        if let Some(r) = self.rich.get(pkg) {
            return r.clone();
        }
        let mut rows = vec![];
        if let Some(pk) = self.rd.open(pkg) {
            for i in 0..pk.exports.len() {
                let j = self.rd.export_json(&pk, i);
                if let Some(rs) = j.get("Rows").and_then(|r| r.as_object()) {
                    for (k, v) in rs {
                        rows.push((k.clone(), crate::model::json_v(v, &|_| None).field("TextStyle").clone()));
                    }
                }
            }
        }
        let r = Rc::new(rows);
        self.rich.insert(pkg.to_string(), r.clone());
        r
    }

    pub fn tex_size(&mut self, pkg: &str) -> Option<(f64, f64)> {
        if let Some(t) = self.tex.get(pkg) {
            return *t;
        }
        let t = if pkg.starts_with("gen:") { gen_texture(pkg).map(|(w, h, _)| (w as f64, h as f64)) } else if pkg.starts_with("png:") { slate_png(&self.rd, pkg).map(|(w, h, _)| (w as f64, h as f64)) } else { texture_size(&self.src, pkg) };
        self.tex.insert(pkg.to_string(), t);
        t
    }
}

/// the UI gradient materials as textures (cooked materials keep only their function / parameter references, the
/// graphs are stripped): "gen:lin" = M_UIGradient (engine LinearGradient -> opacity = U), white with alpha U;
/// "gen:rad:<radius>:<density>:<invert>" = M_UIGradientRadial (parameters Radius 0.5 / Density 2.33 / Invert 0 /
/// Color white by default; RadialGradientExponential form alpha = (1 - saturate(dist(UV, 0.5) / Radius)) ^ Density,
/// 1 - that when Invert: UNCONFIRMED shader math)
pub fn gen_texture(key: &str) -> Option<(u32, u32, Vec<u8>)> {
    if key == "gen:grid" {
        // REWRITE-ONLY: the Combat Test map tile's thumbnail (menu_data::combat_test_metadata), the test level's grid
        // floor seen from above: light lines every 16 px (1 m), darker every 80 px (5 m), on a mid grey
        let (w, h) = (256u32, 128u32);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let major = x % 80 == 0 || y % 80 == 0;
                let minor = x % 16 == 0 || y % 16 == 0;
                let v: u8 = if major { 40 } else if minor { 150 } else { 110 };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        return Some((w, h, px));
    }
    if key == "gen:lin" {
        let w = 256u32;
        let px = (0..w).flat_map(|x| [255, 255, 255, ((x as f64 + 0.5) / w as f64 * 255.0).round() as u8]).collect();
        return Some((w, 1, px));
    }
    let rest = key.strip_prefix("gen:rad:")?;
    let p: Vec<f64> = rest.split(':').filter_map(|x| x.parse().ok()).collect();
    let (radius, density, invert) = (p.first().copied().unwrap_or(0.5).max(1e-4), p.get(1).copied().unwrap_or(2.33), p.get(2).copied().unwrap_or(0.0) > 0.5);
    let n = 128u32;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let u = (x as f64 + 0.5) / n as f64 - 0.5;
            let v = (y as f64 + 0.5) / n as f64 - 0.5;
            let d = (u * u + v * v).sqrt();
            let mut g = (1.0 - (d / radius).clamp(0.0, 1.0)).powf(density);
            if invert {
                g = 1.0 - g;
            }
            px.extend_from_slice(&[255, 255, 255, (g * 255.0).round() as u8]);
        }
    }
    Some((n, n, px))
}

/// an engine Slate image ("png:Engine/Content/Slate/Common/Button.png"): RGBA8 pixels from the paks' loose PNG
pub fn slate_png(rd: &mh_pak::Reader, key: &str) -> Option<(u32, u32, Vec<u8>)> {
    let path = key.strip_prefix("png:")?;
    let bytes = rd.file(path)?;
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let n = (w * h) as usize;
    let px: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..n * 4].to_vec(),
        png::ColorType::Rgb => buf[..n * 3].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..n * 2].chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..n].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    Some((w, h, px))
}

/// FSlateBrush::ResourceName of an engine image brush (serialized relative to the binaries dir:
/// "../../../Engine/Content/Slate/Common/Button.png") -> the pak path key "png:Engine/Content/Slate/..."
pub fn brush_png_key(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".png") {
        return None;
    }
    let i = lower.find("engine/content/")?;
    Some(format!("png:{}", &name[i..]))
}

/// a texture package's size (top mip in the cook), None when it is not a Texture2D
pub fn texture_size(src: &PakSource, pkg: &str) -> Option<(f64, f64)> {
    let t = mh_assets::texture::info(src, pkg, None).ok()?;
    let m = t.mips.first()?;
    Some((m.size_x as f64, m.size_y as f64))
}

/// Absolute geometry: position in pixels, size in local Slate units, scale per axis (local unit -> pixels)
#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub sx: f64,
    pub sy: f64,
}

impl Geo {
    /// absolute rect, normalized: a mirrored geometry (negative render scale) has its origin on the far edge
    pub fn abs(&self) -> Rect {
        let (w, h) = (self.w * self.sx, self.h * self.sy);
        [if w < 0.0 { self.x + w } else { self.x }, if h < 0.0 { self.y + h } else { self.y }, w.abs(), h.abs()]
    }
    /// a child at local offset `o` with local size `s` and extra scale `k`
    pub fn child(&self, o: [f64; 2], s: [f64; 2], k: f64) -> Geo {
        Geo { x: self.x + o[0] * self.sx, y: self.y + o[1] * self.sy, w: s[0], h: s[1], sx: self.sx * k, sy: self.sy * k }
    }
}

#[derive(Clone, Copy)]
struct Style {
    tint: [f64; 4],
    fg: [f64; 4],
    clip: Option<Rect>,
    hit: bool,
    disabled: bool,
}

fn mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

fn isect(a: Option<Rect>, b: Rect) -> Option<Rect> {
    Some(match a {
        None => b,
        Some(a) => {
            let x0 = a[0].max(b[0]);
            let y0 = a[1].max(b[1]);
            let x1 = (a[0] + a[2]).min(b[0] + b[2]);
            let y1 = (a[1] + a[3]).min(b[1] + b[3]);
            [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
        }
    })
}

/// slot defaults (padding, h-align, v-align) by slot class: the UMG slot constructors (see module doc; UNCONFIRMED)
fn slot_defaults(c: &str) -> ([f64; 4], i64, i64) {
    match c {
        // UOverlaySlot ctor: HAlign_Left, VAlign_Top
        "OverlaySlot" => ([0.0; 4], 1, 1),
        // UButtonSlot ctor: Padding (4, 2), HAlign_Center, VAlign_Center
        "ButtonSlot" => ([4.0, 2.0, 4.0, 2.0], 2, 2),
        // UScaleBoxSlot ctor: HAlign_Center, VAlign_Center
        "ScaleBoxSlot" => ([0.0; 4], 2, 2),
        // UUniformGridSlot ctor: HAlign_Left, VAlign_Top
        "UniformGridSlot" => ([0.0; 4], 1, 1),
        // UBorderSlot (synced from UBorder: Padding (4, 2), Fill, Fill)
        "BorderSlot" => ([4.0, 2.0, 4.0, 2.0], 0, 0),
        "BackgroundBlurSlot" => ([4.0, 2.0, 4.0, 2.0], 0, 0),
        _ => ([0.0; 4], 0, 0),
    }
}

/// AlignmentArrangeResult (Layout/LayoutUtils.h AlignChild) for one axis: (offset, size)
fn align(allotted: f64, desired: f64, pre: f64, post: f64, a: i64) -> (f64, f64) {
    let total = pre + post;
    match a {
        0 => (pre, (allotted - total).max(0.0)),
        _ => {
            let size = desired.min(allotted - total).max(0.0);
            match a {
                1 => (pre, size),
                2 => ((allotted - total - size) / 2.0 + pre, size),
                _ => (allotted - size - post, size),
            }
        }
    }
}

pub struct Frame<'a> {
    pub vm: &'a mut Vm,
    pub res: &'a mut Res,
    desired: HashMap<Id, [f64; 2]>,
    pub items: Vec<Item>,
    pub hits: Vec<Hit>,
    /// (rust-render) UBackgroundBlur elements in paint order
    pub blurs: Vec<BlurEl>,
    /// last arranged absolute rect per widget (for evidence and input)
    pub rects: HashMap<Id, Rect>,
    pub hovered: Option<Id>,
    pub pressed: Option<Id>,
    /// open combo-box menus: (combo, its rect, pixel scale), painted above everything
    open_combos: Vec<(Id, Rect, f64)>,
    /// the open menus' rows: (rect, combo, option index)
    pub popup_rows: Vec<(Rect, Id, usize)>,
    /// the cursor (menu row hover)
    pub mouse: [f64; 2],
    /// painted widget -> the widget it was painted inside (the Slate widget path, for event bubbling)
    pub parents: HashMap<Id, Id>,
    stack: Vec<Id>,
}

impl<'a> Frame<'a> {
    pub fn new(vm: &'a mut Vm, res: &'a mut Res) -> Frame<'a> {
        Frame { vm, res, desired: HashMap::new(), items: vec![], hits: vec![], blurs: vec![], rects: HashMap::new(), hovered: None, pressed: None, open_combos: vec![], popup_rows: vec![], mouse: [-1.0, -1.0], parents: HashMap::new(), stack: vec![] }
    }

    fn g(&self, w: Id, k: &str) -> V {
        self.vm.prop(w, k)
    }
    fn kind(&self, w: Id) -> String {
        // any widget with a WidgetTree is a user widget, whatever its native parent (UMordhauDialog, ...)
        if self.vm.o(w).root.is_some() || self.vm.o(w).class.tree_class().is_some() {
            return "UserWidget".into();
        }
        self.vm.o(w).class.native().to_string()
    }
    /// ESlateVisibility; absent = the widget class constructor's default: SelfHitTestInvisible (4) for the panels,
    /// text and user widgets, Visible (0) for Image / Border / Button / ScrollBox and the rest (UE 4.26 UMG
    /// constructors, e.g. `UUserWidget::UUserWidget` 0x2badfc0, `UCanvasPanel::UCanvasPanel` 0x2ba8920,
    /// `UTextBlock::UTextBlock` 0x2bad6e0, `UImage::UImage` 0x2baa620; values UNCONFIRMED)
    fn vis(&self, w: Id) -> i64 {
        match self.g(w, "Visibility") {
            V::None => default_visibility(&self.kind(w)),
            v => v.i(),
        }
    }
    fn kids(&self, w: Id) -> Vec<(Id, Option<Id>)> {
        if matches!(self.vm.o(w).class.native(), "ListView" | "TileView" | "TreeView") {
            return self.g(w, "__entries").arr().iter().filter_map(V::obj).filter(|&e| self.vm.alive(e)).map(|e| (e, None)).collect();
        }
        self.vm.o(w).slots.iter().filter_map(|&s| self.vm.o(s).content.map(|c| (c, Some(s)))).collect()
    }
    fn slot_layout(&self, s: Option<Id>, owner_widget: Id) -> ([f64; 4], i64, i64) {
        let Some(s) = s else { return ([0.0; 4], 0, 0) };
        let sc = self.vm.o(s).class.name.clone();
        let (dp, dh, dv) = slot_defaults(&sc);
        // Border / BackgroundBlur keep the content layout on the widget (UBorder Padding / alignment props)
        let src = if matches!(sc.as_str(), "BorderSlot" | "BackgroundBlurSlot") { owner_widget } else { s };
        let p = match self.g(src, "Padding") {
            V::None => match self.g(s, "Padding") {
                V::None => dp,
                v => margin_of(&v),
            },
            v => margin_of(&v),
        };
        let h = match self.g(src, "HorizontalAlignment") {
            V::None => match self.g(s, "HorizontalAlignment") {
                V::None => dh,
                v => v.i(),
            },
            v => v.i(),
        };
        let v = match self.g(src, "VerticalAlignment") {
            V::None => match self.g(s, "VerticalAlignment") {
                V::None => dv,
                v => v.i(),
            },
            v => v.i(),
        };
        (p, h, v)
    }

    fn text_of(&self, w: Id) -> String {
        let t = self.g(w, "Text").s();
        // (rust-armory) an empty editable text shows its HintText (SEditableText / FEditableTextLayout ShouldShowHint;
        // painted at the hint opacity in paint_text): BP_SearchBar NameFilter "Search"
        if t.is_empty() && self.is_hinted(w) {
            return self.g(w, "HintText").s();
        }
        t
    }

    /// an editable text box whose own text is empty and that has a HintText
    fn is_hinted(&self, w: Id) -> bool {
        matches!(self.vm.o(w).class.native(), "EditableTextBox" | "EditableText" | "MultiLineEditableTextBox") && self.g(w, "Text").s().is_empty() && !self.g(w, "HintText").s().is_empty()
    }

    /// FSlateFontInfo -> (face, size pt). Fields a package leaves out keep the widget class constructor's font:
    /// UTextBlock FSlateFontInfo(Roboto, 24, "Bold"), UComboBoxString FSlateFontInfo(Roboto, 16, "Bold") (UE 4.26
    /// TextBlock.cpp / ComboBoxString.cpp; `UTextBlock::UTextBlock` 0x2bad6e0, values UNCONFIRMED)
    fn font_sized(&mut self, f: &V, default_size: f64) -> (Option<Rc<Face>>, f64) {
        let (face, size) = self.font(f);
        (face, if matches!(f.field("Size"), V::None) { default_size } else { size })
    }

    /// FSlateFontInfo -> (face, size pt)
    fn font(&mut self, f: &V) -> (Option<Rc<Face>>, f64) {
        let obj = match f.field("FontObject") {
            V::Asset(a) => a.package.clone(),
            _ => String::new(),
        };
        let tf = match f.field("TypefaceFontName") {
            V::None => "Bold".to_string(),
            v => v.s(),
        };
        let size = match f.field("Size") {
            V::None => mh_assets::fonts::SLATE_DEFAULT_FONT_SIZE,
            v => v.f(),
        };
        (self.res.face(&obj, &tf), size)
    }

    /// a text widget's Font / ColorAndOpacity / ShadowOffset / ShadowColorAndOpacity from where its Slate widget takes
    /// them (UE 4.26 UMG RebuildWidget): UTextBlock its own properties; UMultiLineEditableTextBox and
    /// UMultiLineEditableText the FTextBlockStyle `TextStyle` / `WidgetStyle` (SMultiLineEditableText draws with
    /// TextStyle's font, colour and shadow); UEditableTextBox WidgetStyle.Font / ForegroundColor; UEditableText
    /// WidgetStyle.Font / ColorAndOpacity
    fn text_prop(&self, w: Id, k: &str) -> V {
        let own = self.g(w, k);
        let kind = self.vm.o(w).class.native();
        let from = |st: &V, key: &str| -> V { st.field(key).clone() };
        let v = match kind {
            "MultiLineEditableTextBox" => from(&self.g(w, "TextStyle"), k),
            "MultiLineEditableText" => from(&self.g(w, "WidgetStyle"), k),
            "EditableTextBox" => from(&self.g(w, "WidgetStyle"), if k == "ColorAndOpacity" { "ForegroundColor" } else { k }),
            "EditableText" => from(&self.g(w, "WidgetStyle"), k),
            _ => V::None,
        };
        if matches!(v, V::None) {
            own
        } else {
            v
        }
    }

    /// URichTextBlock layout (FRichTextMarkupParser / FDefaultRichTextMarkupParser): "<Row>text</>" runs take the
    /// TextStyleSet row "Row"'s FTextBlockStyle, plain text the "Default" row (URichTextBlock::UpdateStyleData: the row
    /// named Default, else the first); self-closing decorator tags ("<img id=.../>") are not drawn (UNCONFIRMED:
    /// decorator classes not run); &lt; &gt; &quot; &amp; unescaped. Hard line breaks only (no auto wrap here).
    /// Returns lines of (text, face, px, style) with the line height and line widths, None when no style table.
    fn rich_layout(&mut self, w: Id) -> Option<(Vec<Vec<(String, Rc<Face>, f64, V)>>, f64, Vec<f64>)> {
        if self.vm.o(w).class.native() != "RichTextBlock" {
            return None;
        }
        let tbl = match self.g(w, "TextStyleSet") {
            V::Asset(a) => a.package.clone(),
            _ => return None,
        };
        let rows = self.res.rich_styles(&tbl);
        if rows.is_empty() {
            return None;
        }
        let default = rows.iter().find(|(k, _)| k.eq_ignore_ascii_case("Default")).unwrap_or(&rows[0]).1.clone();
        let style_of = |tag: &str| rows.iter().find(|(k, _)| k == tag).map(|r| r.1.clone()).unwrap_or_else(|| default.clone());
        let text = self.text_of(w);
        // runs
        let mut runs: Vec<(String, V)> = vec![];
        let mut rest = text.as_str();
        let unescape = |s: &str| s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&");
        while !rest.is_empty() {
            match rest.find('<') {
                None => {
                    runs.push((unescape(rest), default.clone()));
                    break;
                }
                Some(i) => {
                    if i > 0 {
                        runs.push((unescape(&rest[..i]), default.clone()));
                    }
                    let tail = &rest[i..];
                    let Some(close) = tail.find('>') else {
                        runs.push((unescape(tail), default.clone()));
                        break;
                    };
                    let tag = &tail[1..close];
                    let after = &tail[close + 1..];
                    if tag.ends_with('/') || tag.starts_with('/') {
                        rest = after;
                        continue;
                    }
                    let name = tag.split_whitespace().next().unwrap_or("").to_string();
                    let (inner, next) = match after.find("</>") {
                        Some(e) => (&after[..e], &after[e + 3..]),
                        None => (after, ""),
                    };
                    if !inner.is_empty() {
                        runs.push((unescape(inner), style_of(&name)));
                    }
                    rest = next;
                }
            }
        }
        let mut lines: Vec<Vec<(String, Rc<Face>, f64, V)>> = vec![vec![]];
        let mut lh = 0.0f64;
        for (t, st) in runs {
            let (face, size) = self.font(&st.field("Font").clone());
            let face = face?;
            let px = size * mh_assets::fonts::RENDER_DPI_RATIO;
            lh = lh.max(face.adv.height(px));
            for (k, part) in t.split('\n').enumerate() {
                if k > 0 {
                    lines.push(vec![]);
                }
                if !part.is_empty() {
                    lines.last_mut().unwrap().push((part.trim_end_matches('\r').to_string(), face.clone(), px, st.clone()));
                }
            }
        }
        if lh == 0.0 {
            let (face, size) = self.font(&default.field("Font").clone());
            let px = size * mh_assets::fonts::RENDER_DPI_RATIO;
            lh = face.map_or(px * 1.2, |f| f.adv.height(px));
        }
        let ws = lines.iter().map(|l| l.iter().map(|(t, f, px, _)| f.adv.width(t, *px)).sum()).collect();
        Some((lines, lh, ws))
    }

    /// lines of a text block at local scale 1 (Slate units): (lines, line height, widths)
    fn text_lines(&mut self, w: Id, wrap_at: Option<f64>) -> (Vec<String>, f64, Vec<f64>, Option<Rc<Face>>, f64) {
        let text = self.text_of(w);
        let fontv = self.text_prop(w, "Font");
        let (face, size) = self.font(&fontv);
        let px = size * mh_assets::fonts::RENDER_DPI_RATIO;
        let lh = face.as_ref().map_or(px * 1.2, |f| f.adv.height(px)) * match self.g(w, "LineHeightPercentage") {
            V::None => 1.0,
            v => v.f(),
        };
        let width = |s: &str| face.as_ref().map_or(0.6 * px * s.chars().count() as f64, |f| f.adv.width(s, px));
        let mut lines = vec![];
        for para in text.split('\n') {
            let para = para.trim_end_matches('\r');
            match wrap_at {
                Some(wa) if wa > 0.0 && width(para) > wa => {
                    let mut cur = String::new();
                    for word in para.split(' ') {
                        let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                        if width(&cand) > wa && !cur.is_empty() {
                            lines.push(cur);
                            cur = word.to_string();
                        } else {
                            cur = cand;
                        }
                    }
                    lines.push(cur);
                }
                _ => lines.push(para.to_string()),
            }
        }
        let ws = lines.iter().map(|l| width(l)).collect();
        (lines, lh, ws, face, px)
    }

    fn brush_size(&mut self, b: &V) -> [f64; 2] {
        v2_of(b.field("ImageSize"), [32.0, 32.0])
    }

    // ---- ComputeDesiredSize ------------------------------------------------------------------------------------------

    pub fn desired(&mut self, w: Id) -> [f64; 2] {
        if let Some(d) = self.desired.get(&w) {
            return *d;
        }
        let d = if self.vis(w) == 1 { [0.0, 0.0] } else { self.compute_desired(w) };
        self.desired.insert(w, d);
        self.vm.set(w, "__desired", V::st(&[("X", V::Float(d[0])), ("Y", V::Float(d[1]))]));
        d
    }

    fn content_desired(&mut self, w: Id) -> [f64; 2] {
        let mut d = [0.0f64; 2];
        for (c, s) in self.kids(w) {
            if self.vis(c) == 1 {
                continue;
            }
            let (p, _, _) = self.slot_layout(s, w);
            let cd = self.desired(c);
            d[0] = d[0].max(cd[0] + p[0] + p[2]);
            d[1] = d[1].max(cd[1] + p[1] + p[3]);
        }
        d
    }

    fn compute_desired(&mut self, w: Id) -> [f64; 2] {
        let k = self.kind(w);
        match k.as_str() {
            "UserWidget" => match self.vm.o(w).root {
                Some(r) => {
                    let p = margin_of(&self.g(w, "Padding"));
                    let d = self.desired(r);
                    [d[0] + p[0] + p[2], d[1] + p[1] + p[3]]
                }
                None => [0.0, 0.0],
            },
            "HorizontalBox" | "VerticalBox" => {
                let ax = if k == "HorizontalBox" { 0 } else { 1 };
                let mut d = [0.0f64; 2];
                for (c, s) in self.kids(w) {
                    if self.vis(c) == 1 {
                        continue;
                    }
                    let (p, _, _) = self.slot_layout(s, w);
                    let cd = self.desired(c);
                    let along = cd[ax] + if ax == 0 { p[0] + p[2] } else { p[1] + p[3] };
                    let cross = cd[1 - ax] + if ax == 0 { p[1] + p[3] } else { p[0] + p[2] };
                    d[ax] += along;
                    d[1 - ax] = d[1 - ax].max(cross);
                }
                d
            }
            "CanvasPanel" => {
                let mut d = [0.0f64; 2];
                for (c, s) in self.kids(w) {
                    if self.vis(c) == 1 {
                        continue;
                    }
                    let Some(s) = s else { continue };
                    let ld = self.canvas_layout(s);
                    let cd = self.desired(c);
                    let size = if ld.auto_size { cd } else { [ld.offsets[2], ld.offsets[3]] };
                    for ax in 0..2 {
                        let docked = ld.anchor_min[ax] == ld.anchor_max[ax] && (ld.anchor_min[ax] == 0.0 || ld.anchor_min[ax] == 1.0);
                        d[ax] = d[ax].max(size[ax] + if docked { ld.offsets[ax].abs() } else { 0.0 });
                    }
                }
                d
            }
            "WidgetSwitcher" => {
                let i = self.g(w, "ActiveWidgetIndex").i().max(0) as usize;
                match self.kids(w).get(i).copied() {
                    Some((c, s)) => {
                        let (p, _, _) = self.slot_layout(s, w);
                        let cd = self.desired(c);
                        [cd[0] + p[0] + p[2], cd[1] + p[1] + p[3]]
                    }
                    None => [0.0, 0.0],
                }
            }
            "SizeBox" => {
                // SBox::ComputeDesiredSize (PDB; RVA 0x1d84a30): a Collapsed child makes the box (0, 0) whatever its
                // overrides (the child's Visibility attribute compared with EVisibility::Collapsed @0x1d84a96, then the
                // jump past the override code) -> BP_MainMenu's collapsed gamepad prompt / return button boxes take no
                // room and the top-bar icon groups sit at the bar's ends
                let kids = self.kids(w);
                if !kids.is_empty() && kids.iter().all(|&(c, _)| self.vis(c) == 1) {
                    return [0.0, 0.0];
                }
                let cd = self.content_desired(w);
                let o = |s: &Self, n: &str| -> Option<f64> { s.g(w, &format!("bOverride_{n}")).truthy().then(|| s.g(w, n).f()) };
                let mut d = cd;
                for (ax, (ov, mn, mx)) in [("WidthOverride", "MinDesiredWidth", "MaxDesiredWidth"), ("HeightOverride", "MinDesiredHeight", "MaxDesiredHeight")].iter().enumerate() {
                    if let Some(v) = o(self, ov) {
                        d[ax] = v;
                    } else {
                        if let Some(m) = o(self, mn) {
                            d[ax] = d[ax].max(m);
                        }
                        if let Some(m) = o(self, mx) {
                            d[ax] = d[ax].min(m);
                        }
                    }
                }
                d
            }
            "ScaleBox" => {
                // SScaleBox desired = content desired x the user-specified scale (other stretch modes report the
                // unscaled content size; UNCONFIRMED: 4.26 SScaleBox::ComputeDesiredSize)
                let cd = self.content_desired(w);
                let st = match self.g(w, "Stretch") {
                    V::None => 2,
                    v => v.i(),
                };
                if st == 7 {
                    let s = match self.g(w, "UserSpecifiedScale") {
                        V::None => 1.0,
                        v => v.f(),
                    };
                    [cd[0] * s, cd[1] * s]
                } else {
                    cd
                }
            }
            "Border" => {
                let cd = self.content_desired(w);
                let s = v2_of(&self.g(w, "DesiredSizeScale"), [1.0, 1.0]);
                [cd[0] * s[0], cd[1] * s[1]]
            }
            "Button" => {
                // SButton::ComputeDesiredSize (UE 4.26 SButton.cpp): with no content the button is its border brush's
                // ImageSize (the Normal brush here; hover / press swap brushes but layout uses the current one)
                if self.kids(w).is_empty() {
                    let st = self.g(w, "WidgetStyle");
                    let k = if self.hovered == Some(w) { "Hovered" } else { "Normal" };
                    return self.brush_size(&st.field(k).clone());
                }
                let cd = self.content_desired(w);
                let st = self.g(w, "WidgetStyle");
                let np = margin_of(st.field("NormalPadding"));
                [cd[0] + np[0] + np[2], cd[1] + np[1] + np[3]]
            }
            "Image" => {
                let b = self.g(w, "Brush");
                self.brush_size(&b)
            }
            "TextBlock" | "RichTextBlock" | "EditableTextBox" | "EditableText" | "MultiLineEditableTextBox" => {
                if let Some((lines, lh, ws)) = self.rich_layout(w) {
                    let m = margin_of(&self.g(w, "Margin"));
                    let width = ws.iter().cloned().fold(0.0, f64::max);
                    return [width + m[0] + m[2], lh * lines.len().max(1) as f64 + m[1] + m[3]];
                }
                let wrap = self.wrap_width(w);
                let (lines, lh, ws, _, _) = self.text_lines(w, wrap);
                let m = margin_of(&self.g(w, "Margin"));
                let mw = self.g(w, "MinDesiredWidth").f().max(self.g(w, "MinimumDesiredWidth").f());
                let width = ws.iter().cloned().fold(0.0, f64::max).max(mw);
                [width + m[0] + m[2], lh * lines.len().max(1) as f64 + m[1] + m[3]]
            }
            "ProgressBar" => {
                let st = self.g(w, "WidgetStyle");
                let b = st.field("MarqueeImage").clone();
                if matches!(b, V::None) {
                    [32.0, 12.0]
                } else {
                    self.brush_size(&b)
                }
            }
            "Spacer" => v2_of(&self.g(w, "Size"), [1.0, 1.0]),
            "CircularThrobber" => {
                let r = match self.g(w, "Radius") {
                    V::None => 16.0,
                    v => v.f(),
                };
                [2.0 * r, 2.0 * r]
            }
            "CheckBox" => {
                let st = self.g(w, "WidgetStyle");
                let b = st.field("UncheckedImage").clone();
                let i = if matches!(b, V::None) { [16.0, 16.0] } else { self.brush_size(&b) };
                let c = self.content_desired(w);
                [i[0] + c[0], i[1].max(c[1])]
            }
            "ComboBoxString" | "ComboBoxText" => {
                let f = self.g(w, "Font");
                let (face, size) = self.font_sized(&f, 16.0);
                let px = size * mh_assets::fonts::RENDER_DPI_RATIO;
                let s = self.g(w, "SelectedOption").s();
                let tw = face.as_ref().map_or(0.0, |f| f.adv.width(&s, px));
                let p = margin_of(&self.g(w, "ContentPadding"));
                [tw + p[0] + p[2] + 24.0, face.as_ref().map_or(px, |f| f.adv.height(px)) + p[1] + p[3]]
            }
            "Slider" => [64.0, 16.0],
            "UniformGridPanel" => {
                let (rows, cols, cell) = self.uniform_cells(w);
                [cols as f64 * cell[0], rows as f64 * cell[1]]
            }
            "ScrollBox" | "ListView" | "TreeView" => {
                let vertical = self.g(w, "Orientation").i() != 0 || matches!(self.g(w, "Orientation"), V::None);
                let ax = if vertical { 1 } else { 0 };
                let mut d = [0.0f64; 2];
                for (c, s) in self.kids(w) {
                    if self.vis(c) == 1 {
                        continue;
                    }
                    let (p, _, _) = self.slot_layout(s, w);
                    let cd = self.desired(c);
                    d[ax] += cd[ax] + if ax == 1 { p[1] + p[3] } else { p[0] + p[2] };
                    d[1 - ax] = d[1 - ax].max(cd[1 - ax] + if ax == 1 { p[0] + p[2] } else { p[1] + p[3] });
                }
                d
            }
            // SWrapBox::ComputeDesiredSize: the children wrapped at PreferredWidth (WrapSize when explicit, else the
            // width it was last arranged at: SWrapBox ties PreferredWidth to its allotted width)
            "WrapBox" => {
                let inner = margin_of(&self.g(w, "InnerSlotPadding"));
                let wrap = if self.g(w, "bExplicitWrapSize").truthy() { self.g(w, "WrapSize").f() } else { self.g(w, "__wrapw").f() };
                let (mut x, mut y, mut lh, mut mw) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
                for (c, s) in self.kids(w) {
                    if self.vis(c) == 1 {
                        continue;
                    }
                    let (p, _, _) = self.slot_layout(s, w);
                    let d = self.desired(c);
                    let cw = d[0] + p[0] + p[2];
                    if x > 0.0 && wrap > 0.0 && x + cw > wrap {
                        x = 0.0;
                        y += lh + inner[1];
                        lh = 0.0;
                    }
                    x += cw + inner[0];
                    mw = mw.max(x - inner[0]);
                    lh = lh.max(d[1] + p[1] + p[3]);
                }
                [mw, y + lh]
            }
            _ => self.content_desired(w),
        }
    }

    fn wrap_width(&self, w: Id) -> Option<f64> {
        let wa = self.g(w, "WrapTextAt").f();
        if wa > 0.0 {
            return Some(wa);
        }
        if self.g(w, "AutoWrapText").truthy() {
            // STextBlock with AutoWrapText wraps at the width it was last arranged at (the previous frame's)
            let last = self.g(w, "__wrapw").f();
            if last > 0.0 {
                return Some(last);
            }
        }
        None
    }

    fn uniform_cells(&mut self, w: Id) -> (usize, usize, [f64; 2]) {
        let pad = margin_of(&self.g(w, "SlotPadding"));
        let mut rows = 0;
        let mut cols = 0;
        let mut cell = [0.0f64; 2];
        for (c, s) in self.kids(w) {
            if self.vis(c) == 1 {
                continue;
            }
            let (r, cl) = s.map(|s| (self.g(s, "Row").i().max(0) as usize, self.g(s, "Column").i().max(0) as usize)).unwrap_or((0, 0));
            rows = rows.max(r + 1);
            cols = cols.max(cl + 1);
            let d = self.desired(c);
            cell[0] = cell[0].max(d[0] + pad[0] + pad[2]);
            cell[1] = cell[1].max(d[1] + pad[1] + pad[3]);
        }
        (rows, cols, cell)
    }

    fn canvas_layout(&self, s: Id) -> mh_assets::umg::CanvasLayout {
        let ld = self.g(s, "LayoutData");
        let off = ld.field("Offsets");
        let an = ld.field("Anchors");
        let g = |v: &V, k: &str, d: f64| match v.field(k) {
            V::None => d,
            x => x.f(),
        };
        // FAnchorData default: Offsets (0, 0, 100, 30) (UCanvasPanelSlot ctor; packages store an explicit Right /
        // Bottom of 0, so absent means the default)
        mh_assets::umg::CanvasLayout {
            offsets: [g(off, "Left", 0.0), g(off, "Top", 0.0), g(off, "Right", 100.0), g(off, "Bottom", 30.0)],
            anchor_min: v2_of(an.field("Minimum"), [0.0, 0.0]),
            anchor_max: v2_of(an.field("Maximum"), [0.0, 0.0]),
            alignment: v2_of(ld.field("Alignment"), [0.0, 0.0]),
            auto_size: self.g(s, "bAutoSize").truthy(),
            z_order: self.g(s, "ZOrder").i(),
        }
    }

    // ---- arrange + paint ---------------------------------------------------------------------------------------------

    /// lay out and paint widget `w` in geometry `geo`
    fn paint(&mut self, w: Id, geo: Geo, st: Style) {
        if let Some(&p) = self.stack.last() {
            self.parents.insert(w, p);
        }
        self.stack.push(w);
        self.paint_w(w, geo, st);
        self.stack.pop();
    }

    fn paint_w(&mut self, w: Id, geo: Geo, st: Style) {
        let vis = self.vis(w);
        if vis == 1 {
            return;
        }
        let visible = vis != 2;
        // render transform (UWidget RenderTransform about RenderTransformPivot; shear / angle not applied: UNCONFIRMED)
        let mut geo = geo;
        let rt = self.g(w, "RenderTransform");
        if let V::Struct(_) = rt {
            let t = v2_of(rt.field("Translation"), [0.0, 0.0]);
            let sc = v2_of(rt.field("Scale"), [1.0, 1.0]);
            let pv = v2_of(&self.g(w, "RenderTransformPivot"), [0.5, 0.5]);
            let (px, py) = (geo.x + pv[0] * geo.w * geo.sx, geo.y + pv[1] * geo.h * geo.sy);
            geo.x = px + (geo.x - px) * sc[0] + t[0] * geo.sx;
            geo.y = py + (geo.y - py) * sc[1] + t[1] * geo.sy;
            geo.sx *= sc[0];
            geo.sy *= sc[1];
        }
        let op = match self.g(w, "RenderOpacity") {
            V::None => 1.0,
            v => v.f(),
        };
        let mut st = st;
        st.tint[3] *= op;
        // a disabled widget and its subtree draw with ESlateDrawEffect::DisabledEffect (SWidget::IsEnabled false;
        // the effect's look, a dimmed / desaturated draw, approximated as 45 % alpha: UNCONFIRMED)
        if matches!(self.g(w, "bIsEnabled"), V::Bool(false)) && !st.disabled {
            st.disabled = true;
            st.tint[3] *= 0.45;
        }
        let abs = geo.abs();
        self.rects.insert(w, abs);
        self.vm.set(w, "__scale", V::Float(geo.sx));
        // EWidgetClipping: ClipToBounds and stronger clip this widget and its children
        let clip = self.g(w, "Clipping").i();
        if (1..=3).contains(&clip) {
            st.clip = isect(st.clip, abs);
        }
        // ESlateVisibility: HitTestInvisible 3 / SelfHitTestInvisible 4
        let self_hit = st.hit && vis != 3 && vis != 4 && visible;
        if vis == 3 {
            st.hit = false;
        }
        let k = self.kind(w);
        // every self-hit-testable widget takes part in the hit test (SWidget::GetVisibility().IsHitTestVisible(): a
        // Visible image above a button blocks it, as in Slate)
        if self_hit {
            self.hits.push(Hit { rect: abs, widget: w, clip: st.clip });
        }
        if !visible {
            // Hidden: arranged (takes space) but neither painted nor hit-testable
            return;
        }
        if st.tint[3] <= 0.0 {
            return;
        }
        match k.as_str() {
            "UserWidget" => {
                if let Some(r) = self.vm.o(w).root {
                    let p = margin_of(&self.g(w, "Padding"));
                    let c = color_of(&self.g(w, "ColorAndOpacity"), [1.0; 4]);
                    let mut s2 = st;
                    s2.tint = mul(st.tint, c);
                    if let V::Struct(_) = self.g(w, "ForegroundColor") {
                        s2.fg = color_of(&self.g(w, "ForegroundColor"), st.fg);
                    }
                    let g2 = geo.child([p[0], p[1]], [(geo.w - p[0] - p[2]).max(0.0), (geo.h - p[1] - p[3]).max(0.0)], 1.0);
                    self.paint(r, g2, s2);
                }
            }
            "CanvasPanel" => {
                let mut kids: Vec<(i64, usize, Id, Id)> = vec![];
                for (i, (c, s)) in self.kids(w).into_iter().enumerate() {
                    if let Some(s) = s {
                        let z = self.g(s, "ZOrder").i();
                        kids.push((z, i, c, s));
                    }
                }
                // SConstraintCanvas::ArrangeLayeredChildren: children sorted by ZOrder (stable)
                kids.sort_by_key(|x| (x.0, x.1));
                for (_, _, c, s) in kids {
                    if self.vis(c) == 1 {
                        continue;
                    }
                    let ld = self.canvas_layout(s);
                    let d = self.desired(c);
                    let r = ld.rect([0.0, 0.0, geo.w, geo.h], d);
                    self.paint(c, geo.child([r[0], r[1]], [r[2], r[3]], 1.0), st);
                }
            }
            // (rust-armory) UTileView -> STileView / SListPanel::ArrangeChildren (UE 4.26): every entry gets an
            // EntryWidth x EntryHeight tile (UTileView defaults 128 x 128), floor(width / EntryWidth) tiles per row (at
            // least one), rows top to bottom; EListItemAlignment LeftAligned packs from the left, EvenlyDistributed (the
            // default) spreads the row's spare width between the tiles. Scrolls vertically, clipped.
            "TileView" => {
                let ew = match self.g(w, "EntryWidth") {
                    V::None => 128.0,
                    v => v.f(),
                };
                let eh = match self.g(w, "EntryHeight") {
                    V::None => 128.0,
                    v => v.f(),
                };
                let mut st2 = st;
                st2.clip = isect(st.clip, abs);
                let kids: Vec<_> = self.kids(w).into_iter().filter(|(c, _)| self.vis(*c) != 1).collect();
                let per = ((geo.w / ew.max(1.0)).floor() as usize).max(1);
                let rows = kids.len().div_ceil(per);
                let mx = (rows as f64 * eh - geo.h).max(0.0);
                self.vm.set(w, "__scroll_max", V::Float(mx));
                let sc = self.g(w, "__scroll").f().min(mx).max(0.0);
                // EListItemAlignment: EvenlyDistributed 0, EvenlySize, EvenlyWide, LeftAligned 3, RightAligned,
                // CenterAligned, Fill (UE 4.26 ListViewBase.h); only LeftAligned and the default are ported
                let left = match self.g(w, "TileAlignment") {
                    V::Name(n) => n.ends_with("LeftAligned"),
                    v => v.i() == 3,
                };
                let gap = if left { 0.0 } else { (geo.w - per as f64 * ew).max(0.0) / per as f64 };
                for (i, (c, _)) in kids.into_iter().enumerate() {
                    let (col, row) = ((i % per) as f64, (i / per) as f64);
                    let x = col * (ew + gap) + gap * 0.5;
                    self.paint(c, geo.child([x, row * eh - sc], [ew, eh], 1.0), st2);
                }
            }
            "HorizontalBox" | "VerticalBox" | "ScrollBox" | "ListView" | "TreeView" => {
                let ax = if k == "HorizontalBox" || (k == "ScrollBox" && self.g(w, "Orientation").i() == 0 && !matches!(self.g(w, "Orientation"), V::None)) { 0 } else { 1 };
                let is_scroll = k == "ScrollBox" || k == "ListView" || k == "TreeView";
                let mut st2 = st;
                if is_scroll {
                    st2.clip = isect(st.clip, abs);
                }
                let kids: Vec<_> = self.kids(w).into_iter().filter(|(c, _)| self.vis(*c) != 1).collect();
                let size = [geo.w, geo.h];
                let mut fixed = 0.0;
                let mut coef = 0.0;
                let mut info = vec![];
                for &(c, s) in &kids {
                    let (p, h, v) = self.slot_layout(s, w);
                    let (rule, val) = match s.map(|s| self.g(s, "Size")) {
                        Some(V::Struct(m)) => (m.get("SizeRule").map(V::i).unwrap_or(0), m.get("Value").map(V::f).unwrap_or(1.0)),
                        _ => (0, 1.0),
                    };
                    let fill = rule == 1 && !is_scroll;
                    let d = self.desired(c);
                    let pad_along = if ax == 0 { p[0] + p[2] } else { p[1] + p[3] };
                    fixed += pad_along + if fill { 0.0 } else { d[ax] };
                    if fill {
                        coef += val;
                    }
                    info.push((c, p, h, v, fill, val, d));
                }
                let free = (size[ax] - fixed).max(0.0);
                let mut pos = if is_scroll {
                    let total: f64 = info.iter().map(|x| x.6[ax] + if ax == 0 { x.1[0] + x.1[2] } else { x.1[1] + x.1[3] }).sum();
                    let mx = (total - size[ax]).max(0.0);
                    self.vm.set(w, "__scroll_max", V::Float(mx));
                    let sc = self.g(w, "__scroll").f().min(mx).max(0.0);
                    -sc
                } else {
                    0.0
                };
                for (c, p, h, v, fill, val, d) in info {
                    let child_size = if fill { if coef > 0.0 { free * val / coef } else { 0.0 } } else { d[ax] };
                    let pad_along = if ax == 0 { p[0] + p[2] } else { p[1] + p[3] };
                    let slot_along = child_size + pad_along;
                    let (ox, sx, oy, sy) = if ax == 0 {
                        let xr = align(slot_along, d[0], p[0], p[2], if fill { h } else { h });
                        let yr = align(size[1], d[1], p[1], p[3], v);
                        (pos + xr.0, xr.1, yr.0, yr.1)
                    } else {
                        let xr = align(size[0], d[0], p[0], p[2], h);
                        let yr = align(slot_along, d[1], p[1], p[3], v);
                        (xr.0, xr.1, pos + yr.0, yr.1)
                    };
                    self.paint(c, geo.child([ox, oy], [sx, sy], 1.0), st2);
                    pos += slot_along;
                }
            }
            "WidgetSwitcher" => {
                let i = self.g(w, "ActiveWidgetIndex").i().max(0) as usize;
                if let Some((c, s)) = self.kids(w).get(i).copied() {
                    self.paint_aligned(w, c, s, geo, st);
                }
            }
            "Image" => {
                let b = self.g(w, "Brush");
                let c = color_of(&self.g(w, "ColorAndOpacity"), [1.0; 4]);
                // (rust-armory) a negative render-transform scale mirrors the image (Slate draws the element through
                // the flipped layout transform: e.g. BP_LoadoutPicker Image_0, M_UIGradient with RenderTransform Scale
                // X -1 = opaque at the left edge, clear toward the doll)
                self.vm.set(w, "__flip_x", V::Bool(geo.sx < 0.0));
                self.brush(w, &b, abs, mul(st.tint, c), st.clip);
            }
            "Border" => {
                let b = self.g(w, "Background");
                let bc = color_of(&self.g(w, "BrushColor"), [1.0; 4]);
                self.brush(w, &b, abs, mul(st.tint, bc), st.clip);
                let cc = color_of(&self.g(w, "ContentColorAndOpacity"), [1.0; 4]);
                let mut s2 = st;
                s2.tint = mul(st.tint, cc);
                for (c, s) in self.kids(w) {
                    self.paint_aligned(w, c, s, geo, s2);
                }
            }
            "Button" => {
                let ws = self.g(w, "WidgetStyle");
                let enabled = !matches!(self.g(w, "bIsEnabled"), V::Bool(false));
                let pressed = self.pressed == Some(w) && self.hovered == Some(w);
                let hovered = self.hovered == Some(w);
                let b = if !enabled {
                    ws.field("Disabled").clone()
                } else if pressed {
                    ws.field("Pressed").clone()
                } else if hovered {
                    ws.field("Hovered").clone()
                } else {
                    ws.field("Normal").clone()
                };
                let bg = color_of(&self.g(w, "BackgroundColor"), [1.0; 4]);
                self.brush(w, &b, abs, mul(st.tint, bg), st.clip);
                let fg = color_of(&self.g(w, "ColorAndOpacity"), [1.0; 4]);
                let mut s2 = st;
                s2.tint = mul(st.tint, fg);
                let pad = margin_of(if pressed { ws.field("PressedPadding") } else { ws.field("NormalPadding") });
                let inner = geo.child([pad[0], pad[1]], [(geo.w - pad[0] - pad[2]).max(0.0), (geo.h - pad[1] - pad[3]).max(0.0)], 1.0);
                for (c, s) in self.kids(w) {
                    self.paint_aligned(w, c, s, inner, s2);
                }
            }
            "ProgressBar" => {
                let ws = self.g(w, "WidgetStyle");
                let bgb = ws.field("BackgroundImage").clone();
                self.brush(w, &bgb, abs, st.tint, st.clip);
                let pct = self.g(w, "Percent").f().clamp(0.0, 1.0);
                let fill = ws.field("FillImage").clone();
                let fc = color_of(&self.g(w, "FillColorAndOpacity"), [1.0; 4]);
                let bp = margin_of(&self.g(w, "BorderPadding"));
                let inner = [abs[0] + bp[0] * geo.sx, abs[1] + bp[1] * geo.sy, abs[2] - (bp[0] + bp[2]) * geo.sx, abs[3] - (bp[1] + bp[3]) * geo.sy];
                // EProgressBarFillType: LeftToRight 0, RightToLeft 1, FillFromCenter 2, TopToBottom 3, BottomToTop 4
                let r = match self.g(w, "BarFillType").i() {
                    1 => [inner[0] + inner[2] * (1.0 - pct), inner[1], inner[2] * pct, inner[3]],
                    2 => [inner[0] + inner[2] * (1.0 - pct) * 0.5, inner[1], inner[2] * pct, inner[3]],
                    3 => [inner[0], inner[1], inner[2], inner[3] * pct],
                    4 => [inner[0], inner[1] + inner[3] * (1.0 - pct), inner[2], inner[3] * pct],
                    _ => [inner[0], inner[1], inner[2] * pct, inner[3]],
                };
                // a mirrored bar (render scale X < 0, BP_StatusBar's stamina side) fills from the other edge
                let r = if geo.sx < 0.0 { [inner[0] + inner[2] - (r[0] - inner[0]) - r[2], r[1], r[2], r[3]] } else { r };
                let clip = isect(st.clip, r);
                self.brush(w, &fill, inner, mul(st.tint, fc), clip);
            }
            "TextBlock" | "RichTextBlock" | "EditableTextBox" | "EditableText" | "MultiLineEditableTextBox" => self.paint_text(w, geo, st),
            "SizeBox" | "BackgroundBlur" | "InvalidationBox" | "RetainerBox" | "SafeZone" | "NamedSlot" | "MenuAnchor" | "InteractableWidget" | "SnapWidget" | "Overlay" | "WindowTitleBarArea" => {
                if k == "BackgroundBlur" {
                    // (rust-render) UBackgroundBlur ctor 0x2ba92e0: BlurStrength +0x134 = 0, bApplyAlphaToBlur +0x132 = 1,
                    // bOverrideAutoRadiusCalculation +0x138 = 0, BlurRadius +0x13c = 0
                    let strength = match self.g(w, "BlurStrength") { V::None => 0.0, v => v.f() };
                    let alpha = !matches!(self.g(w, "bApplyAlphaToBlur"), V::Bool(false));
                    let radius = if self.g(w, "bOverrideAutoRadiusCalculation").truthy() {
                        Some(match self.g(w, "BlurRadius") { V::None => 0, v => v.i() as i32 })
                    } else {
                        None
                    };
                    let strength = if alpha { strength * st.tint[3] } else { strength };
                    self.blurs.push(BlurEl { rect: abs, clip: st.clip, strength, radius, layer: self.items.len() });
                }
                for (c, s) in self.kids(w) {
                    self.paint_aligned(w, c, s, geo, st);
                }
            }
            "ScaleBox" => {
                for (c, s) in self.kids(w) {
                    if self.vis(c) == 1 {
                        continue;
                    }
                    let d = self.desired(c);
                    let (p, h, v) = self.slot_layout(s, w);
                    let area = [(geo.w - p[0] - p[2]).max(0.0), (geo.h - p[1] - p[3]).max(0.0)];
                    let k = self.scale_box_scale(w, d, area);
                    let size = [d[0] * k, d[1] * k];
                    // (rust-armory) SScaleBox::OnArrangeChildren (UE 4.26): AlignChild with the slot's own alignment;
                    // an explicit HAlign_Fill / VAlign_Fill puts the scaled child at the padding's edge (BP_LoadoutEntry
                    // ScaleBoxSlot_0/_1 HAlign_Fill: the names start at the left); the ctor default Center comes from
                    // slot_defaults("ScaleBoxSlot")
                    let xr = align(geo.w, size[0], p[0], p[2], h);
                    let yr = align(geo.h, size[1], p[1], p[3], v);
                    self.paint(c, geo.child([xr.0, yr.0], d, k), st);
                }
            }
            "UniformGridPanel" => {
                let (_, _, cell) = self.uniform_cells(w);
                let (rows, cols, _) = self.uniform_cells(w);
                let cw = if cols > 0 { geo.w / cols as f64 } else { 0.0 };
                let ch = if rows > 0 { geo.h / rows as f64 } else { 0.0 };
                let _ = cell;
                let pad = margin_of(&self.g(w, "SlotPadding"));
                for (c, s) in self.kids(w) {
                    if self.vis(c) == 1 {
                        continue;
                    }
                    let (r, cl) = s.map(|s| (self.g(s, "Row").i().max(0) as f64, self.g(s, "Column").i().max(0) as f64)).unwrap_or((0.0, 0.0));
                    let (_, h, v) = self.slot_layout(s, w);
                    let d = self.desired(c);
                    let xr = align(cw, d[0], pad[0], pad[2], h);
                    let yr = align(ch, d[1], pad[1], pad[3], v);
                    self.paint(c, geo.child([cl * cw + xr.0, r * ch + yr.0], [xr.1, yr.1], 1.0), st);
                }
            }
            "WrapBox" => {
                let inner = margin_of(&self.g(w, "InnerSlotPadding"));
                let wrap = if self.g(w, "bExplicitWrapSize").truthy() { self.g(w, "WrapSize").f() } else { geo.w };
                self.vm.set(w, "__wrapw", V::Float(geo.w));
                let (mut x, mut y, mut lh) = (0.0, 0.0, 0.0f64);
                for (c, s) in self.kids(w) {
                    if self.vis(c) == 1 {
                        continue;
                    }
                    let (p, _, _) = self.slot_layout(s, w);
                    let d = self.desired(c);
                    let cw = d[0] + p[0] + p[2];
                    if x > 0.0 && x + cw > wrap {
                        x = 0.0;
                        y += lh + inner[1];
                        lh = 0.0;
                    }
                    self.paint(c, geo.child([x + p[0], y + p[1]], d, 1.0), st);
                    x += cw + inner[0];
                    lh = lh.max(d[1] + p[1] + p[3]);
                }
            }
            "CheckBox" => {
                let ws = self.g(w, "WidgetStyle");
                let checked = self.g(w, "CheckedState").i() == 1;
                let hovered = self.hovered == Some(w);
                let key = match (checked, hovered) {
                    (true, true) => "CheckedHoveredImage",
                    (true, false) => "CheckedImage",
                    (false, true) => "UncheckedHoveredImage",
                    (false, false) => "UncheckedImage",
                };
                let b = ws.field(key).clone();
                let sz = if matches!(b, V::None) { [16.0, 16.0] } else { self.brush_size(&b) };
                // ESlateCheckBoxType::ToggleButton draws the image over the whole box, CheckBox at its size
                let r = if ws.field("CheckBoxType").i() == 1 { abs } else { [abs[0], abs[1] + (abs[3] - sz[1] * geo.sy) * 0.5, sz[0] * geo.sx, sz[1] * geo.sy] };
                self.brush(w, &b, r, st.tint, st.clip);
                for (c, s) in self.kids(w) {
                    self.paint_aligned(w, c, s, geo, st);
                }
            }
            "Slider" => {
                let ws = self.g(w, "WidgetStyle");
                let v = self.g(w, "Value").f().clamp(0.0, 1.0);
                let bar = ws.field("NormalBarImage").clone();
                let thumb = ws.field("NormalThumbImage").clone();
                let bc = color_of(&self.g(w, "SliderBarColor"), [1.0; 4]);
                let ts = if matches!(thumb, V::None) { [8.0, 14.0] } else { self.brush_size(&thumb) };
                let bh = ws.field("BarThickness").f().max(2.0);
                self.brush(w, &bar, [abs[0], abs[1] + (abs[3] - bh * geo.sy) * 0.5, abs[2], bh * geo.sy], mul(st.tint, bc), st.clip);
                let tx = abs[0] + (abs[2] - ts[0] * geo.sx) * v;
                self.brush(w, &thumb, [tx, abs[1] + (abs[3] - ts[1] * geo.sy) * 0.5, ts[0] * geo.sx, ts[1] * geo.sy], st.tint, st.clip);
            }
            "ComboBoxString" | "ComboBoxText" => {
                let ws = self.g(w, "WidgetStyle");
                let b = ws.field("ComboButtonStyle").field("ButtonStyle").field(if self.hovered == Some(w) { "Hovered" } else { "Normal" }).clone();
                self.brush(w, &b, abs, st.tint, st.clip);
                let f = self.g(w, "Font");
                let (face, size) = self.font_sized(&f, 16.0);
                let px = size * mh_assets::fonts::RENDER_DPI_RATIO;
                // the combo button is an SButton: its content padding is ContentPadding + the ButtonStyle's
                // NormalPadding / PressedPadding (UE 4.26 SButton::GetCombinedPadding; UComboBoxText::RebuildWidget
                // rva 0x149c930 passes ContentPadding to SComboBox), the selected item's STextBlock
                // (UComboBoxText::HandleGenerateWidget rva 0x1486100) centred in what is left (VAlign UNCONFIRMED)
                let mut p = margin_of(&self.g(w, "ContentPadding"));
                let bp = margin_of(ws.field("ComboButtonStyle").field("ButtonStyle").field(if self.pressed == Some(w) { "PressedPadding" } else { "NormalPadding" }));
                for i in 0..4 {
                    p[i] += bp[i];
                }
                let fg = color_of(&self.g(w, "ForegroundColor"), [1.0; 4]);
                let text = self.g(w, "SelectedOption").s();
                if let Some(face) = face {
                    let lh = face.adv.height(px);
                    let (top, h) = (abs[1] + p[1] * geo.sy, abs[3] - (p[1] + p[3]) * geo.sy);
                    let r = [abs[0] + p[0] * geo.sx, top + (h - lh * geo.sy) * 0.5, abs[2], lh * geo.sy];
                    self.items.push(Item { prim: Prim::Text { rect: r, text, face: face.package.clone(), px: px * geo.sy, color: mul(st.tint, fg) }, clip: st.clip, widget: w });
                }
                if self.g(w, "__open").truthy() {
                    self.open_combos.push((w, abs, geo.sy));
                }
            }
            "CircularThrobber" => {
                // SCircularThrobber: NumberOfPieces chunks on a circle of Radius, rotating with Period (pieces drawn as
                // small squares: the engine's Throbber.CircleChunk brush is not in Mordhau's paks, UNCONFIRMED look)
                let n = match self.g(w, "NumberOfPieces") {
                    V::None => 6,
                    v => v.i().max(1),
                };
                let period = match self.g(w, "Period") {
                    V::None => 0.75,
                    v => v.f().max(0.01),
                };
                let rad = abs[2].min(abs[3]) * 0.5;
                let cx = abs[0] + abs[2] * 0.5;
                let cy = abs[1] + abs[3] * 0.5;
                let phase = (self.vm.time / period).fract() * std::f64::consts::TAU;
                let cs = rad * 0.25;
                for i in 0..n {
                    let a = phase + i as f64 / n as f64 * std::f64::consts::TAU;
                    let (x, y) = (cx + a.cos() * (rad - cs), cy + a.sin() * (rad - cs));
                    let mut c = st.tint;
                    c[3] *= 0.3 + 0.7 * (i as f64 / n as f64);
                    self.items.push(Item { prim: Prim::Rect { rect: [x - cs * 0.5, y - cs * 0.5, cs, cs], color: c }, clip: st.clip, widget: w });
                }
            }
            _ => {
                for (c, s) in self.kids(w) {
                    self.paint_aligned(w, c, s, geo, st);
                }
            }
        }
    }

    /// SScaleBox::ComputeContentScale: EStretch None 0, Fill 1, ScaleToFit 2, ScaleToFitX 3, ScaleToFitY 4,
    /// ScaleToFill 5, ScaleBySafeZone 6, UserSpecified 7; EStretchDirection Both 0, DownOnly 1, UpOnly 2
    fn scale_box_scale(&self, w: Id, d: [f64; 2], area: [f64; 2]) -> f64 {
        let st = match self.g(w, "Stretch") {
            V::None => 2,
            v => v.i(),
        };
        let fx = if d[0] > 0.0 { area[0] / d[0] } else { 1.0 };
        let fy = if d[1] > 0.0 { area[1] / d[1] } else { 1.0 };
        let mut k = match st {
            0 | 6 => 1.0,
            1 | 2 => fx.min(fy),
            3 => fx,
            4 => fy,
            5 => fx.max(fy),
            7 => match self.g(w, "UserSpecifiedScale") {
                V::None => 1.0,
                v => v.f(),
            },
            _ => 1.0,
        };
        match self.g(w, "StretchDirection").i() {
            1 => k = k.min(1.0),
            2 => k = k.max(1.0),
            _ => {}
        }
        k
    }

    /// a content / overlay child: slot padding + alignment inside `geo` (AlignChild on both axes)
    fn paint_aligned(&mut self, owner: Id, c: Id, s: Option<Id>, geo: Geo, st: Style) {
        if self.vis(c) == 1 {
            return;
        }
        let (p, h, v) = self.slot_layout(s, owner);
        let d = self.desired(c);
        let xr = align(geo.w, d[0], p[0], p[2], h);
        let yr = align(geo.h, d[1], p[1], p[3], v);
        self.paint(c, geo.child([xr.0, yr.0], [xr.1, yr.1], 1.0), st);
    }

    fn paint_text(&mut self, w: Id, geo: Geo, st: Style) {
        if let Some((lines, lh, ws)) = self.rich_layout(w) {
            let m = margin_of(&self.g(w, "Margin"));
            let just = self.g(w, "Justification").i();
            let avail = geo.w - m[0] - m[2];
            for (i, (line, lw)) in lines.iter().zip(ws).enumerate() {
                let mut x = m[0] + match just {
                    1 => (avail - lw) * 0.5,
                    2 => avail - lw,
                    _ => 0.0,
                };
                let y = m[1] + i as f64 * lh;
                for (t, face, px, sty) in line {
                    let tw = face.adv.width(t, *px);
                    let cv = sty.field("ColorAndOpacity").clone();
                    let color = mul(st.tint, if cv.field("ColorUseRule").i() == 2 { st.fg } else { color_of(&cv, [1.0; 4]) });
                    let sh = color_of(sty.field("ShadowColorAndOpacity"), [0.0; 4]);
                    let so = v2_of(sty.field("ShadowOffset"), [1.0, 1.0]);
                    let r = [geo.x + x * geo.sx, geo.y + y * geo.sy, tw * geo.sx + 2.0, lh * geo.sy];
                    if sh[3] > 0.0 {
                        let rs = [r[0] + so[0] * geo.sx, r[1] + so[1] * geo.sy, r[2], r[3]];
                        self.items.push(Item { prim: Prim::Text { rect: rs, text: t.clone(), face: face.package.clone(), px: px * geo.sy, color: mul(st.tint, sh) }, clip: st.clip, widget: w });
                    }
                    self.items.push(Item { prim: Prim::Text { rect: r, text: t.clone(), face: face.package.clone(), px: px * geo.sy, color }, clip: st.clip, widget: w });
                    x += tw;
                }
            }
            return;
        }
        let wrap = if self.g(w, "AutoWrapText").truthy() {
            self.vm.set(w, "__wrapw", V::Float(geo.w));
            Some(geo.w)
        } else {
            self.wrap_width(w)
        };
        let (lines, lh, ws, face, px) = self.text_lines(w, wrap);
        let Some(face) = face else { return };
        // FSlateColor ColorUseRule: UseColor_Specified 0 (white when absent), UseColor_Foreground 2 -> inherited
        let cv = self.text_prop(w, "ColorAndOpacity");
        let color = if cv.field("ColorUseRule").i() == 2 { st.fg } else { color_of(&cv, [1.0; 4]) };
        let mut color = mul(st.tint, color);
        // (rust-armory) the hint draws in the text colour at opacity 0.35 (UE 4.26 FEditableTextLayout hint paint:
        // UNCONFIRMED, not disassembled)
        if self.is_hinted(w) {
            color[3] *= 0.35;
        }
        let sh = color_of(&self.text_prop(w, "ShadowColorAndOpacity"), [0.0; 4]);
        let so = v2_of(&self.text_prop(w, "ShadowOffset"), [1.0, 1.0]);
        let m = margin_of(&self.g(w, "Margin"));
        // ETextJustify Left 0, Center 1, Right 2
        let just = self.g(w, "Justification").i();
        let avail = geo.w - m[0] - m[2];
        for (i, (line, lw)) in lines.iter().zip(ws).enumerate() {
            let x = m[0] + match just {
                1 => (avail - lw) * 0.5,
                2 => avail - lw,
                _ => 0.0,
            };
            let y = m[1] + i as f64 * lh;
            let r = [geo.x + x * geo.sx, geo.y + y * geo.sy, lw * geo.sx + 2.0, lh * geo.sy];
            if sh[3] > 0.0 {
                let mut sc = mul(st.tint, sh);
                sc[3] *= 1.0;
                let rs = [r[0] + so[0] * geo.sx, r[1] + so[1] * geo.sy, r[2], r[3]];
                self.items.push(Item { prim: Prim::Text { rect: rs, text: line.clone(), face: face.package.clone(), px: px * geo.sy, color: sc }, clip: st.clip, widget: w });
            }
            self.items.push(Item { prim: Prim::Text { rect: r, text: line.clone(), face: face.package.clone(), px: px * geo.sy, color }, clip: st.clip, widget: w });
        }
    }

    /// FSlateDrawElement::MakeBox for an FSlateBrush value: ESlateBrushDrawType NoDrawType 0, Box 1, Border 2, Image 3
    /// (absent DrawAs = Image); Box / Border margins are fractions of the texture (9-slice), corners drawn at
    /// Margin x the texture's pixel size (AddBoxElement, below). A brush without a resource draws a solid quad (the default white texture).
    fn brush(&mut self, w: Id, b: &V, r: Rect, tint: [f64; 4], clip: Option<Rect>) {
        if matches!(b, V::None) {
            return;
        }
        let da = match b.field("DrawAs") {
            V::None => 3,
            v => v.i(),
        };
        if da == 0 {
            return;
        }
        let bt = color_of(b.field("TintColor"), [1.0; 4]);
        let tint = mul(tint, bt);
        if tint[3] <= 0.0 {
            return;
        }
        if let Some((key, color)) = self.gradient_material(b.field("ResourceObject")) {
            let mut b2 = b.clone();
            if let V::Struct(m) = &mut b2 {
                m.insert("ResourceObject".into(), V::Asset(std::sync::Arc::new(crate::kismet::ObjRef { package: key, name: String::new(), class: "Texture2D".into(), outer: String::new() })));
                m.insert("DrawAs".into(), V::Int(3));
            }
            self.brush(w, &b2, r, mul(tint, color), clip);
            return;
        }
        // a MaterialInstanceDynamic resource: a quad in its colour parameter ("BarColor", "Color", "Tint": the UI
        // materials' tint inputs; the material graph is not evaluated, UNCONFIRMED look)
        if let V::Obj(mid) = b.field("ResourceObject") {
            let mut c = [1.0; 4];
            for k in ["param:BarColor", "param:Color", "param:Tint", "param:TintColor"] {
                let v = self.vm.prop(*mid, k);
                if !matches!(v, V::None) {
                    c = color_of(&v, [1.0; 4]);
                    break;
                }
            }
            self.items.push(Item { prim: Prim::Rect { rect: r, color: mul(tint, c) }, clip, widget: w });
            return;
        }
        let tex = match b.field("ResourceObject") {
            V::Asset(a) => Some(a.clone()),
            _ => brush_png_key(&b.field("ResourceName").s()).map(|k| std::sync::Arc::new(crate::kismet::ObjRef { package: k, name: String::new(), class: "Texture2D".into(), outer: String::new() })),
        };
        let Some(tex) = tex else {
            self.items.push(Item { prim: Prim::Rect { rect: r, color: tint }, clip, widget: w });
            return;
        };
        if !tex.class.contains("Texture") && !tex.class.is_empty() {
            // a material brush (the material graph is not evaluated): a MaterialInstanceConstant with a texture
            // parameter draws that texture tinted; a Material (UI masks such as M_stmBar, whose texture is an
            // opacity mask) draws a quad in the brush tint (UNCONFIRMED look)
            if tex.class == "MaterialInstanceConstant" {
                if let Some(t) = material_texture(&self.res.rd, &tex.package) {
                    let mut b2 = b.clone();
                    if let V::Struct(m) = &mut b2 {
                        m.insert("ResourceObject".into(), V::Asset(std::sync::Arc::new(crate::kismet::ObjRef { package: t, name: String::new(), class: "Texture2D".into(), outer: String::new() })));
                    }
                    self.brush(w, &b2, r, tint, clip);
                    return;
                }
            }
            self.items.push(Item { prim: Prim::Rect { rect: r, color: tint }, clip, widget: w });
            return;
        }
        let Some((tw, th)) = self.res.tex_size(&tex.package) else {
            self.items.push(Item { prim: Prim::Rect { rect: r, color: tint }, clip, widget: w });
            return;
        };
        if da == 3 {
            // a mirrored widget: the UV rect runs right to left (negative width; real.rs draws it with flip_x)
            let uv = self.vm.prop(w, "__flip_x").truthy().then_some([tw as f64, 0.0, -(tw as f64), th as f64]);
            self.items.push(Item { prim: Prim::Image { rect: r, tex: tex.package.clone(), tint, uv }, clip, widget: w });
            return;
        }
        let m = margin_of(b.field("Margin"));
        // FSlateElementBatcher::AddBoxElement<0> (PDB; label 0x1c0cb40 = .text offset, RVA 0x1c0db40): the destination margins are Margin x
        // the resource's ActualSize (the texture's pixel size, integers), in local units, not Margin x ImageSize:
        // LeftMarginX = TexW * Margin.Left, RightMarginX = LocalSize.X - TexW * Margin.Right, and when they overlap
        // (RightMarginX < LeftMarginX) both become LocalSize.X * 0.5 (same for Y) (disassembly RVA 0x1c0e00d-0x1c0e0a0)
        let ds = self.draw_scale_hint(w);
        let (lw, lh) = (r[2] / ds, r[3] / ds);
        let (mut l, mut rr) = (m[0] * tw, lw - m[2] * tw);
        if rr < l {
            l = lw * 0.5;
            rr = l;
        }
        let (mut t, mut bb) = (m[1] * th, lh - m[3] * th);
        if bb < t {
            t = lh * 0.5;
            bb = t;
        }
        let (dl, dr, dt, db) = (l * ds, (lw - rr) * ds, t * ds, (lh - bb) * ds);
        let (ul, ut, ur, ub) = (m[0] * tw, m[1] * th, m[2] * tw, m[3] * th);
        let xs = [r[0], r[0] + dl, r[0] + r[2] - dr, r[0] + r[2]];
        let ys = [r[1], r[1] + dt, r[1] + r[3] - db, r[1] + r[3]];
        let us = [0.0, ul, tw - ur, tw];
        let vs = [0.0, ut, th - ub, th];
        for j in 0..3 {
            for i in 0..3 {
                if da == 2 && i == 1 && j == 1 {
                    continue;
                }
                let rect = [xs[i], ys[j], xs[i + 1] - xs[i], ys[j + 1] - ys[j]];
                let uv = [us[i], vs[j], us[i + 1] - us[i], vs[j + 1] - vs[j]];
                if rect[2] <= 0.01 || rect[3] <= 0.01 || uv[2] <= 0.0 || uv[3] <= 0.0 {
                    continue;
                }
                self.items.push(Item { prim: Prim::Image { rect, tex: tex.package.clone(), tint, uv: Some(uv) }, clip, widget: w });
            }
        }
    }

    /// a brush resource that is (an instance of) M_UIGradient / M_UIGradientRadial: the generated texture key and the
    /// Color parameter (MID parameters override the material defaults Radius 0.5, Density 2.33, Invert 0, Color 1)
    fn gradient_material(&self, res: &V) -> Option<(String, [f64; 4])> {
        let (pkg, mid) = match res {
            V::Asset(a) => (a.package.clone(), None),
            V::Obj(o) if self.vm.alive(*o) => match self.vm.prop(*o, "Parent") {
                V::Asset(a) => (a.package.clone(), Some(*o)),
                _ => return None,
            },
            _ => return None,
        };
        let name = pkg.rsplit('/').next().unwrap_or("");
        let p = |k: &str, d: f64| mid.map(|o| self.vm.prop(o, &format!("param:{k}"))).filter(|v| !matches!(v, V::None)).map(|v| v.f()).unwrap_or(d);
        let color = mid.map(|o| self.vm.prop(o, "param:Color")).filter(|v| !matches!(v, V::None)).map(|v| color_of(&v, [1.0; 4])).unwrap_or([1.0; 4]);
        if name.starts_with("M_UIGradientRadial") {
            return Some((format!("gen:rad:{:.3}:{:.3}:{}", p("Radius", 0.5), p("Density", 2.33), p("Invert", 0.0)), color));
        }
        if name.starts_with("M_UIGradient") {
            return Some(("gen:lin".to_string(), color));
        }
        None
    }

    /// the pixel scale of the widget being painted (its last geometry's scale)
    fn draw_scale_hint(&self, w: Id) -> f64 {
        self.vm.prop(w, "__scale").f().max(1e-6)
    }

    /// Paint the viewport: AddToViewport widgets by ZOrder (UGameViewportClient AddViewportWidgetContent: higher
    /// ZOrder on top; equal ZOrder in insertion order), each filling the viewport unless positioned
    pub fn paint_viewport(&mut self, size_px: [f64; 2], scale: f64) {
        let mut roots = self.vm.viewport.clone();
        roots.sort_by_key(|x| x.1);
        let full = Geo { x: 0.0, y: 0.0, w: size_px[0] / scale, h: size_px[1] / scale, sx: scale, sy: scale };
        for (w, _) in roots {
            if !self.vm.alive(w) {
                continue;
            }
            let st = Style { tint: [1.0; 4], fg: [1.0; 4], clip: Some([0.0, 0.0, size_px[0], size_px[1]]), hit: true, disabled: false };
            let pos = self.g(w, "__vp_pos");
            let geo = if matches!(pos, V::Struct(_)) {
                let p = v2_of(&pos, [0.0, 0.0]);
                let d = self.desired(w);
                let sz = v2_of(&self.g(w, "__vp_size"), d);
                let al = v2_of(&self.g(w, "__vp_align"), [0.0, 0.0]);
                Geo { x: p[0] - al[0] * sz[0] * scale, y: p[1] - al[1] * sz[1] * scale, w: sz[0], h: sz[1], sx: scale, sy: scale }
            } else {
                full
            };
            self.paint_root(w, geo, st);
        }
        self.paint_combo_menus(size_px);
    }

    /// SComboBox's menu (an SMenuAnchor popup under the button, one STableRow per option with the ItemStyle brushes;
    /// row padding and the menu background from ComboButtonStyle.MenuBorderBrush: UE 4.26 SComboBox.h /
    /// `SComboBox<TSharedPtr<FString,0> >::GenerateMenuItemRow` 0x1d996e0, layout values UNCONFIRMED)
    fn paint_combo_menus(&mut self, size_px: [f64; 2]) {
        let combos = std::mem::take(&mut self.open_combos);
        for (w, r, k) in combos {
            let ws = self.g(w, "WidgetStyle");
            let is = self.g(w, "ItemStyle");
            let f = self.g(w, "Font");
            let (face, size) = self.font_sized(&f, 16.0);
            let Some(face) = face else { continue };
            let px = size * mh_assets::fonts::RENDER_DPI_RATIO;
            let lh = face.adv.height(px) * k;
            let pad = 4.0 * k;
            let opts: Vec<String> = self.g(w, "DefaultOptions").arr().iter().map(V::s).collect();
            let rh = lh + 2.0 * pad;
            let total = rh * opts.len() as f64;
            // below the button, or above when it would leave the viewport
            let y0 = if r[1] + r[3] + total > size_px[1] { (r[1] - total).max(0.0) } else { r[1] + r[3] };
            let menu = [r[0], y0, r[2], total];
            self.items.push(Item { prim: Prim::Rect { rect: menu, color: [0.0, 0.0, 0.0, 1.0] }, clip: None, widget: w });
            let bg = ws.field("ComboButtonStyle").field("MenuBorderBrush").clone();
            if !matches!(bg, V::None) {
                self.brush(w, &bg, menu, [1.0; 4], None);
            }
            let sel = self.g(w, "SelectedOption").s();
            let fg = color_of(&self.g(w, "ForegroundColor"), [1.0; 4]);
            for (i, o) in opts.iter().enumerate() {
                let row = [r[0], y0 + i as f64 * rh, r[2], rh];
                let hov = self.mouse[0] >= row[0] && self.mouse[0] < row[0] + row[2] && self.mouse[1] >= row[1] && self.mouse[1] < row[1] + row[3];
                let key = match (o == &sel, hov) {
                    (true, true) => "ActiveHoveredBrush",
                    (true, false) => "ActiveBrush",
                    (false, true) => "InactiveHoveredBrush",
                    (false, false) => if i % 2 == 0 { "EvenRowBackgroundBrush" } else { "OddRowBackgroundBrush" },
                };
                let b = is.field(key).clone();
                if matches!(b, V::None) {
                    if hov {
                        self.items.push(Item { prim: Prim::Rect { rect: row, color: [1.0, 1.0, 1.0, 0.08] }, clip: None, widget: w });
                    }
                } else {
                    self.brush(w, &b, row, [1.0; 4], None);
                }
                let tr = [row[0] + pad, row[1] + pad, face.adv.width(o, px) * k + 2.0, lh];
                self.items.push(Item { prim: Prim::Text { rect: tr, text: o.clone(), face: face.package.clone(), px: px * k, color: fg }, clip: Some(row), widget: w });
                self.popup_rows.push((row, w, i));
            }
        }
    }

    fn paint_root(&mut self, w: Id, geo: Geo, st: Style) {
        self.paint(w, geo, st);
    }
}

/// a UI material's stand-in texture: a MaterialInstanceConstant's first texture parameter value, else (a Material)
/// the first Texture2D the package imports (the material graph is not evaluated: UNCONFIRMED look, texture x tint)
pub fn material_texture(rd: &mh_pak::Reader, pkg: &str) -> Option<String> {
    if let Some(pk) = rd.open(pkg) {
        let mut tex = None;
        let mut has_params = false;
        for (i, im) in pk.imports.iter().enumerate() {
            if im.class_name == "Texture2D" && tex.is_none() {
                if let Some(r) = crate::kismet::resolve(&pk, -(i as i32) - 1) {
                    if !r.package.is_empty() {
                        tex = Some(r.package);
                    }
                }
            }
            if im.name == "MaterialInstanceConstant" {
                has_params = true;
            }
        }
        if !has_params {
            if let Some(t) = tex {
                return Some(t);
            }
        }
    }
    let ex = rd.read(pkg)?;
    for e in &ex {
        if let Some(a) = e.pointer("/Properties/TextureParameterValues").and_then(serde_json::Value::as_array) {
            for t in a {
                if let Some(p) = t.pointer("/ParameterValue/ObjectPath").and_then(serde_json::Value::as_str) {
                    return Some(crate::kismet::content_path(mh_pak::reader::strip_index(p)));
                }
            }
        }
    }
    None
}

/// Run the widgets' property bindings (UWidgetBlueprintGeneratedClass Bindings; UMG evaluates them when the bound
/// Slate attribute is read during paint): Function bindings call the getter, Property bindings read the member path
pub fn run_bindings(vm: &mut Vm, res: &mut Res) {
    // (perf) Slate polls a bound attribute only on widgets it lays out: a Collapsed widget's subtree is skipped by its
    // parent's prepass / arrange (SWidget::Prepass_ChildLoop, ArrangeChildren drop EVisibility::Collapsed children), so
    // the bindings of widgets under a collapsed ancestor are not run; a collapsed widget's own Visibility attribute is
    // still read by its parent (that is how it shows again). `live`: widgets reached without crossing a collapsed one;
    // `edge`: the collapsed widgets met on the way (Visibility bindings only).
    let mut live: std::collections::HashSet<crate::model::Id> = Default::default();
    let mut edge: std::collections::HashSet<crate::model::Id> = Default::default();
    let mut users = vec![];
    let mut seen: std::collections::HashSet<crate::model::Id> = Default::default();
    for &(w, _) in &vm.viewport.clone() {
        collect_live(vm, w, &mut seen, &mut live, &mut edge, &mut users);
    }
    for u in users {
        if vm.prop(u, "Visibility").i() == 1 {
            continue;
        }
        let mut binds = vec![];
        let mut c = Some(vm.o(u).class.clone());
        while let Some(k) = c {
            binds.extend(k.bindings.iter().cloned());
            c = k.parent.clone();
        }
        for (obj, prop, func, path) in binds {
            let Some(target) = vm.child(u, &obj) else { continue };
            // (perf) collapsed subtrees are not polled (see above)
            if !live.contains(&target) && !(prop == "Visibility" && edge.contains(&target)) {
                continue;
            }
            if vm.prop(target, &format!("__unbound:{prop}")).truthy() {
                continue;
            }
            // an event binding (UBorder / UImage OnMouseButtonDownEvent / OnMouseButtonUpEvent / OnMouseMoveEvent /
            // OnMouseDoubleClickEvent: FOnPointerEvent delegates) binds the function; it is called by the pointer
            // handlers (UBorder::HandleMouseButtonDown etc.), not polled
            if prop.ends_with("Event") && !func.is_empty() && func != "None" {
                vm.set(target, &prop, V::Delegate(u, func.clone()));
                continue;
            }
            // (perf) the ToolTipWidget binding (UWidget ToolTipWidgetDelegate) is a lazy Slate attribute: the engine calls
            // it only when the tooltip is about to show (hover delay), never per frame. Polling it here ran
            // BP_CustomizationTopBar:GetToolTipWidget_0 -> BP_LoadoutPointListTooltip:Update every frame, which clears and
            // re-creates its 27 entry widgets (~900 VM objects a frame, never freed: the in-match HUD leak, 60 ms frames).
            // Tooltips are not displayed by mh-ui, so the binding is skipped.
            if prop == "ToolTipWidget" {
                continue;
            }
            // FDelegateRuntimeBinding: FunctionName when the class has it, else the SourcePath, whose segments are
            // functions (called) or properties (read) (UE 4.26 FEditorPropertyPath / FDynamicPropertyPath)
            let v = if !func.is_empty() && func != "None" && vm.has_func(u, &func) {
                vm.call_named(u, &func, vec![]).0
            } else {
                let mut v = V::Obj(u);
                for seg in &path {
                    v = match v {
                        V::Obj(o) if vm.has_func(o, seg) => vm.call_named(o, seg, vec![]).0,
                        V::Obj(o) => vm.prop(o, seg),
                        V::Struct(_) => v.field(seg).clone(),
                        _ => V::None,
                    };
                }
                v
            };
            let v = match (prop.as_str(), v) {
                // UBrushBinding: a texture source becomes the brush's resource at the texture's size
                ("Brush", V::Asset(a)) => {
                    let mut b = vm.prop(target, "Brush");
                    if !matches!(b, V::Struct(_)) {
                        b = V::Struct(Box::default());
                    }
                    if let V::Struct(m) = &mut b {
                        m.insert("ResourceObject".into(), V::Asset(a.clone()));
                        if let Some((w, h)) = res.tex_size(&a.package) {
                            m.insert("ImageSize".into(), V::st(&[("X", V::Float(w)), ("Y", V::Float(h))]));
                        }
                    }
                    b
                }
                ("Brush", V::None) => continue,
                (_, v) => v,
            };
            vm.set(target, &prop, v);
        }
    }
}

/// (perf) run_bindings' walk: the widget tree in Vm::descendants order (user widget root, panel slots, list entries),
/// not descending into Collapsed widgets (recorded in `edge`); user widgets reached are pushed to `users`
fn collect_live(vm: &Vm, id: crate::model::Id, seen: &mut std::collections::HashSet<crate::model::Id>, live: &mut std::collections::HashSet<crate::model::Id>, edge: &mut std::collections::HashSet<crate::model::Id>, users: &mut Vec<crate::model::Id>) {
    if !seen.insert(id) {
        return;
    }
    if vm.prop(id, "Visibility").i() == 1 {
        edge.insert(id);
        return;
    }
    live.insert(id);
    let o = vm.o(id);
    if o.class.tree_class().is_some() {
        users.push(id);
    }
    let mut kids: Vec<crate::model::Id> = vec![];
    if let Some(r) = o.root {
        kids.push(r);
    }
    for &sl in &o.slots {
        if let Some(c) = vm.o(sl).content {
            kids.push(c);
        }
    }
    if let Some(V::Array(es)) = o.props.get("__entries") {
        kids.extend(es.iter().filter_map(V::obj).filter(|&e| vm.alive(e)));
    }
    for k in kids {
        collect_live(vm, k, seen, live, edge, users);
    }
}

/// the UMG constructor default visibility per widget kind (see Frame::vis)
pub fn default_visibility(kind: &str) -> i64 {
    match kind {
        "UserWidget" | "CanvasPanel" | "Overlay" | "HorizontalBox" | "VerticalBox" | "WrapBox" | "UniformGridPanel" | "GridPanel" | "SizeBox" | "ScaleBox" | "WidgetSwitcher" | "SafeZone" | "InvalidationBox" | "RetainerBox" | "NamedSlot" | "TextBlock" | "RichTextBlock" | "Spacer" => 4,
        _ => 0,
    }
}

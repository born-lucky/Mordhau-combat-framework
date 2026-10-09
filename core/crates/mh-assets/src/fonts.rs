//! UFont / UFontFace as engine-neutral data: a composite font's typefaces (default, fallback, sub-typefaces with their
//! character ranges and scaling), each entry's font-face bytes (the `.ufont` file next to the UFontFace package in the
//! paks: the TTF / OTF itself) and the face's metrics read from the font's own tables. Port of
//! godot/components/ue/records/umg.gd `font` (FSlateFontInfo {FontObject, TypefaceFontName, Size}; the face of the
//! default typeface entry named TypefaceFontName, else its first entry, UNCONFIRMED there) and `font_px`
//! (FSlateFontInfo Size is points at 96 DPI: px = Size x 96 / 72, FontConstants::RenderDPI, default Size 24).
//! UE 4.26 layouts (CompositeFont.h): FCompositeFont {DefaultTypeface, FallbackTypeface, SubTypefaces[]
//! {Typeface, CharacterRanges[], Cultures, ScalingFactor}}, FTypeface {Fonts[] {Name, Font: FFontData
//! {LocalFontFaceAsset, SubFaceIndex}}}. UFontFace (FontFace.h) fields as stored; Hinting / LoadingPolicy defaults
//! (EFontHinting::Default, EFontLoadingPolicy::LazyLoad) UNCONFIRMED (header recalled).
//! Font tables: OpenType spec (https://learn.microsoft.com/typography/opentype/spec/): head.unitsPerEm (offset 18),
//! hhea ascender / descender / lineGap (4 / 6 / 8), OS/2 sTypoAscender / Descender / LineGap (68 / 70 / 72),
//! usWinAscent / Descent (74 / 76), name table records 1 (family) and 4 (full name).

use crate::material::strip_index;
use mh_pak::Reader;
use serde_json::Value;

/// FSlateFontInfo Size points -> pixels at 96 DPI (FontConstants::RenderDPI / 72), as umg.gd font_px (x scale, rounded,
/// at least 1)
pub const RENDER_DPI_RATIO: f64 = 96.0 / 72.0;
pub const SLATE_DEFAULT_FONT_SIZE: f64 = 24.0;

pub fn font_px(size_pt: f64, scale: f64) -> i64 {
    ((size_pt * RENDER_DPI_RATIO * scale).round() as i64).max(1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypefaceEntry {
    pub name: String,
    /// UFontFace package
    pub face: String,
    pub sub_face_index: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubTypeface {
    pub fonts: Vec<TypefaceEntry>,
    /// inclusive code point ranges
    pub ranges: Vec<(u32, u32)>,
    pub cultures: String,
    pub scaling: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompositeFont {
    pub package: String,
    /// EFontCacheType as stored ("EFontCacheType::Runtime")
    pub cache_type: String,
    pub default: Vec<TypefaceEntry>,
    pub fallback: Vec<TypefaceEntry>,
    pub sub: Vec<SubTypeface>,
}

fn entries(t: Option<&Value>) -> Vec<TypefaceEntry> {
    t.and_then(|t| t.get("Fonts"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|e| TypefaceEntry {
                    name: e.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
                    face: strip_index(e.pointer("/Font/LocalFontFaceAsset/ObjectPath").and_then(Value::as_str).unwrap_or("")).to_string(),
                    sub_face_index: e.pointer("/Font/SubFaceIndex").and_then(Value::as_i64).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The UFont of a package
pub fn composite(rd: &Reader, pkg: &str) -> Option<CompositeFont> {
    let ex = rd.read(strip_index(pkg))?;
    let f = ex.iter().find(|e| e.get("Type").and_then(Value::as_str) == Some("Font"))?;
    let p = f.get("Properties")?;
    let cf = p.get("CompositeFont");
    let sub = cf
        .and_then(|c| c.get("SubTypefaces"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|s| SubTypeface {
                    fonts: entries(s.get("Typeface")),
                    ranges: s
                        .get("CharacterRanges")
                        .and_then(Value::as_array)
                        .map(|r| {
                            r.iter()
                                .map(|x| {
                                    let b = |k: &str| x.pointer(&format!("/{k}/Value")).and_then(Value::as_u64).unwrap_or(0) as u32;
                                    (b("LowerBound"), b("UpperBound"))
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    cultures: s.get("Cultures").and_then(Value::as_str).unwrap_or("").to_string(),
                    scaling: s.get("ScalingFactor").and_then(Value::as_f64).unwrap_or(1.0),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(CompositeFont {
        package: strip_index(pkg).to_string(),
        cache_type: p.get("FontCacheType").and_then(Value::as_str).unwrap_or("EFontCacheType::Offline").to_string(),
        default: entries(cf.and_then(|c| c.get("DefaultTypeface"))),
        fallback: entries(cf.and_then(|c| c.get("FallbackTypeface"))),
        sub,
    })
}

impl CompositeFont {
    /// The face for FSlateFontInfo.TypefaceFontName: the default typeface entry of that name, else its first entry
    /// (umg.gd font; Slate's fallback UNCONFIRMED)
    pub fn face(&self, typeface: &str) -> Option<&TypefaceEntry> {
        self.default.iter().find(|e| e.name == typeface).or_else(|| self.default.first())
    }
    /// The sub-typeface covering code point `c`, if any
    pub fn sub_for(&self, c: u32) -> Option<&SubTypeface> {
        self.sub.iter().find(|s| s.ranges.iter().any(|&(a, b)| c >= a && c <= b))
    }
}

/// Metrics from the font file's own tables (font units; divide by units_per_em)
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metrics {
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub typo_ascender: Option<i16>,
    pub typo_descender: Option<i16>,
    pub typo_line_gap: Option<i16>,
    pub win_ascent: Option<u16>,
    pub win_descent: Option<u16>,
    pub family: String,
    pub full_name: String,
    pub num_glyphs: u16,
}

#[derive(Debug, Clone)]
pub struct FontFace {
    pub package: String,
    /// the TTF / OTF bytes (the .ufont file)
    pub data: Vec<u8>,
    pub source_filename: String,
    pub hinting: String,
    pub loading_policy: String,
    pub metrics: Metrics,
}

/// A UFontFace package and its .ufont bytes
pub fn face(rd: &Reader, pkg: &str) -> Option<FontFace> {
    let pkg = strip_index(pkg);
    let ex = rd.read(pkg)?;
    let f = ex.iter().find(|e| e.get("Type").and_then(Value::as_str) == Some("FontFace"))?;
    let null = Value::Null;
    let p = f.get("Properties").unwrap_or(&null);
    let data = rd.file(&format!("{}.ufont", rd.vfs.spelling(&format!("{pkg}.uasset")).trim_end_matches(".uasset")))?;
    let metrics = metrics(&data)?;
    Some(FontFace {
        package: pkg.to_string(),
        source_filename: p.get("SourceFilename").and_then(Value::as_str).unwrap_or("").to_string(),
        hinting: p.get("Hinting").and_then(Value::as_str).unwrap_or("EFontHinting::Default").to_string(),
        loading_policy: p.get("LoadingPolicy").and_then(Value::as_str).unwrap_or("EFontLoadingPolicy::LazyLoad").to_string(),
        data,
        metrics,
    })
}

fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(o)?, *b.get(o + 1)?]))
}
fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(o)?, *b.get(o + 1)?, *b.get(o + 2)?, *b.get(o + 3)?]))
}

/// head / hhea / OS/2 / maxp / name of a TrueType or OpenType font (the first face of a collection)
pub fn metrics(b: &[u8]) -> Option<Metrics> {
    let mut base = 0usize;
    if b.get(0..4)? == b"ttcf" {
        base = be32(b, 12)? as usize;
    }
    let n = be16(b, base + 4)? as usize;
    let table = |tag: &[u8; 4]| -> Option<(usize, usize)> {
        (0..n).find_map(|i| {
            let r = base + 12 + 16 * i;
            (b.get(r..r + 4)? == tag).then(|| Some((be32(b, r + 8)? as usize, be32(b, r + 12)? as usize))).flatten()
        })
    };
    let (head, _) = table(b"head")?;
    let (hhea, _) = table(b"hhea")?;
    let mut m = Metrics {
        units_per_em: be16(b, head + 18)?,
        ascender: be16(b, hhea + 4)? as i16,
        descender: be16(b, hhea + 6)? as i16,
        line_gap: be16(b, hhea + 8)? as i16,
        ..Default::default()
    };
    if let Some((os2, len)) = table(b"OS/2") {
        if len >= 78 {
            m.typo_ascender = be16(b, os2 + 68).map(|x| x as i16);
            m.typo_descender = be16(b, os2 + 70).map(|x| x as i16);
            m.typo_line_gap = be16(b, os2 + 72).map(|x| x as i16);
            m.win_ascent = be16(b, os2 + 74);
            m.win_descent = be16(b, os2 + 76);
        }
    }
    if let Some((maxp, _)) = table(b"maxp") {
        m.num_glyphs = be16(b, maxp + 4).unwrap_or(0);
    }
    if let Some((name, _)) = table(b"name") {
        let count = be16(b, name + 2).unwrap_or(0) as usize;
        let strings = name + be16(b, name + 4).unwrap_or(0) as usize;
        for i in 0..count {
            let r = name + 6 + 12 * i;
            let (Some(pid), Some(nid), Some(len), Some(off)) = (be16(b, r), be16(b, r + 6), be16(b, r + 8), be16(b, r + 10)) else { break };
            let Some(raw) = b.get(strings + off as usize..strings + off as usize + len as usize) else { continue };
            let s = if pid == 3 || pid == 0 {
                String::from_utf16_lossy(&raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>())
            } else {
                raw.iter().map(|&c| c as char).collect()
            };
            match nid {
                1 if m.family.is_empty() || pid == 3 => m.family = s,
                4 if m.full_name.is_empty() || pid == 3 => m.full_name = s,
                _ => {}
            }
        }
    }
    Some(m)
}

/// Glyph advances of a TrueType / OpenType font (cmap format 4 or 12 + hmtx), for laying out text without a renderer
pub struct Advances {
    upem: f64,
    ascender: f64,
    descender: f64,
    map: std::collections::HashMap<u32, u16>,
    adv: Vec<u16>,
}

impl Advances {
    pub fn new(b: &[u8]) -> Option<Advances> {
        let m = metrics(b)?;
        let mut base = 0usize;
        if b.get(0..4)? == b"ttcf" {
            base = be32(b, 12)? as usize;
        }
        let n = be16(b, base + 4)? as usize;
        let table = |tag: &[u8; 4]| -> Option<usize> {
            (0..n).find_map(|i| {
                let r = base + 12 + 16 * i;
                (b.get(r..r + 4)? == tag).then(|| be32(b, r + 8).map(|x| x as usize)).flatten()
            })
        };
        let hhea = table(b"hhea")?;
        let nh = be16(b, hhea + 34)? as usize;
        let hmtx = table(b"hmtx")?;
        let adv: Vec<u16> = (0..nh).map(|i| be16(b, hmtx + 4 * i).unwrap_or(0)).collect();
        let cmap = table(b"cmap")?;
        let nsub = be16(b, cmap + 2)? as usize;
        let mut map = std::collections::HashMap::new();
        // prefer format 12 (full Unicode), else format 4 (BMP); platform 3 / 0
        let mut subs: Vec<usize> = (0..nsub)
            .filter_map(|i| {
                let r = cmap + 4 + 8 * i;
                let pid = be16(b, r)?;
                (pid == 3 || pid == 0).then(|| be32(b, r + 4).map(|o| cmap + o as usize)).flatten()
            })
            .collect();
        subs.sort_by_key(|&o| std::cmp::Reverse(be16(b, o).unwrap_or(0)));
        for o in subs {
            match be16(b, o)? {
                12 => {
                    let ng = be32(b, o + 12)? as usize;
                    for g in 0..ng {
                        let r = o + 16 + 12 * g;
                        let (s, e, gid) = (be32(b, r)?, be32(b, r + 4)?, be32(b, r + 8)?);
                        for c in s..=e.min(s + 0xffff) {
                            map.entry(c).or_insert((gid + (c - s)) as u16);
                        }
                    }
                    break;
                }
                4 => {
                    let segx2 = be16(b, o + 6)? as usize;
                    let ends = o + 14;
                    let starts = ends + segx2 + 2;
                    let deltas = starts + segx2;
                    let ranges = deltas + segx2;
                    for s in 0..segx2 / 2 {
                        let (e, st) = (be16(b, ends + 2 * s)? as u32, be16(b, starts + 2 * s)? as u32);
                        let d = be16(b, deltas + 2 * s)?;
                        let ro = be16(b, ranges + 2 * s)? as usize;
                        if st == 0xffff {
                            continue;
                        }
                        for c in st..=e {
                            let g = if ro == 0 {
                                (c as u16).wrapping_add(d)
                            } else {
                                let gi = ranges + 2 * s + ro + 2 * (c - st) as usize;
                                let g = be16(b, gi).unwrap_or(0);
                                if g == 0 { 0 } else { g.wrapping_add(d) }
                            };
                            map.entry(c).or_insert(g);
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
        Some(Advances { upem: m.units_per_em as f64, ascender: m.ascender as f64, descender: m.descender as f64, map, adv })
    }

    fn glyph_adv(&self, c: char) -> f64 {
        let g = self.map.get(&(c as u32)).copied().unwrap_or(0) as usize;
        self.adv.get(g).or(self.adv.last()).copied().unwrap_or(0) as f64
    }

    /// width in pixels of `s` at `px` (sum of advances, no kerning)
    pub fn width(&self, s: &str, px: f64) -> f64 {
        s.chars().map(|c| self.glyph_adv(c)).sum::<f64>() * px / self.upem
    }
    /// line height in pixels (hhea ascender - descender)
    pub fn height(&self, px: f64) -> f64 {
        (self.ascender - self.descender) * px / self.upem
    }
    pub fn ascent(&self, px: f64) -> f64 {
        self.ascender * px / self.upem
    }
}

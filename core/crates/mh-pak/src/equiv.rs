//! Field-by-field comparison of the pak reader against extract/json (mdx json / CUE4Parse) for the same package.
//! Port of `godot/tests/pak/pak_equiv.gd` + the package sets of `godot/tests/test_pak_equiv.gd`.
//!
//! Rules (docs/PAK_FORMAT.md "Equivalence with extract/json"):
//! - floats: the JSON number, rounded to float32, must equal the float32 read from the pak (bit-exact);
//! - keys CUE4Parse computes instead of reading (FQuat IsNormalized/Size/SizeSquared, FColor/FLinearColor Hex) are not
//!   in the file and are skipped when the pak side lacks them;
//! - FName text: CUE4Parse trims names (FNameEntrySerialized.cs:27), UE does not; a difference only in surrounding
//!   whitespace counts as "trimmed", not as a mismatch;
//! - a value the pak reader could not decode counts as "unsupported" (never as a match);
//! - known CUE4Parse misread: FVector_NetQuantize* stored as tagged structs are read by CUE4Parse as 12 raw bytes
//!   (FScriptStruct.cs:192-195), so the JSON holds nonzero float32 denormals; `split_misreads` puts only those structs
//!   down to it.

use crate::reader::{Reader, UNSUPPORTED};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub const DERIVED: &[&str] = &["IsNormalized", "Size", "SizeSquared", "Hex", "IsRotationNormalized"];
pub const COMPARED: &[&str] = &[
    "Type", "Name", "Class", "Outer", "Package", "Super", "Template", "Properties", "SuperStruct", "Rows", "Names",
    "CppForm", "ReferenceSkeleton",
];
const FLT_MIN: f64 = 1.17549435e-38;

const M: &str = "Mordhau/Content/Mordhau/";
/// [dir, recursive]: the packages the game's readers load (test_pak_equiv.gd SETS)
pub const SETS: &[(&str, bool)] = &[
    ("Blueprints/Equipment", true),
    ("Blueprints/Motions", true),
    ("Blueprints/Characters", true),
    ("Blueprints/BotProfiles", true),
    ("AI", true),
    ("Blueprints/GameModes", false),
    ("Blueprints/GameModes/Duel", true),
    ("Blueprints/GameModes/Group3v3", true),
    ("Blueprints", false),
    ("Blueprints/Wearables", false),
    ("Maps/Arena_Map/Metadata", true),
];
/// test_pak_equiv.gd SINGLES (relative to Mordhau/Content/Mordhau/ unless absolute)
pub const SINGLES: &[&str] = &[
    "Animations/Blueprints/AB_MordhauCharacterAnimation",
    "Blueprints/Wearables/Head/Tier3/BP_Houndskull",
    "UI/BP_Announcement", "UI/BP_DefeatPopup", "UI/BP_DuelRoundWinsWidget", "UI/BP_HUDWidget",
    "UI/BP_KillFeed", "UI/BP_KillFeedEntry", "UI/BP_LocalPlay", "UI/BP_OneTeamScoreboard",
    "UI/BP_RankedScoreElementEntry", "UI/BP_ScoreboardEntry", "UI/BP_StatusBar", "UI/BP_VictoryPopup",
    "/Mordhau/Content/UMA/UMA/Master/UMA_Master_Skeleton",
    // the maps the modes load (UeLevel.levels: each persistent map and its streaming sub-levels)
    "Maps/Arena_Map/Arena", "Maps/Arena_Map/DU_Arena", "Maps/Arena_Map/FFA_Arena",
    "Maps/Arena_Map/TDM_Arena", "Maps/Arena_Map/SKM_Arena", "Maps/Arena_Map/TF_Arena",
    "Maps/DuelCamp/FL_Camp", "Maps/DuelCamp/Camp", "Maps/DuelCamp/Camp_Interactables",
];
/// MORDHAU_PAK_EQUIV=full: every package under Blueprints/ and UI/
pub const FULL: &[(&str, bool)] = &[("Blueprints", true), ("UI", true)];

fn rel(p: &str) -> String {
    match p.strip_prefix('/') {
        Some(abs) => abs.to_string(),
        None => format!("{M}{p}"),
    }
}

/// The extract/ folder: $MORDHAU_EXTRACT, else <repo>/extract (the Godot project setting mordhau/data_path)
pub fn extract_root() -> PathBuf {
    match std::env::var("MORDHAU_EXTRACT") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extract"),
    }
}

#[derive(Debug, Clone)]
pub struct Mismatch {
    pub pkg: String,
    pub path: String,
    pub pak: Value,
    pub json: Value,
}

#[derive(Default)]
pub struct Equiv {
    pub fields: usize, // top-level export fields compared
    pub leaves: usize, // scalar values compared
    pub matched: usize,
    pub trimmed: usize,
    pub resolved_further: usize,
    pub unsupported: usize,
    pub unsupported_types: BTreeMap<String, usize>, // export Type -> values the pak reader could not decode
    pub unsupported_reasons: BTreeMap<String, usize>, // first three words of the reader's reason -> count
    pub packages: usize,
    pub packages_exact: usize,
    pub missing_json: usize,
    pub mismatches: Vec<Mismatch>,
    ty: String,
}

/// Parse a JSON file CUE4Parse wrote; Newtonsoft writes NaN/Infinity as bare tokens, read here as the strings
/// "NaN"/"Infinity"/"-Infinity" (the pak side writes non-finite floats the same way, reader::fnum)
pub fn parse_lenient(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str(text) {
        return Some(v);
    }
    let mut out = String::with_capacity(text.len() + 64);
    let (mut in_str, mut esc) = (false, false);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if let Some(t) = ["-Infinity", "Infinity", "NaN"].iter().find(|t| rest.starts_with(**t)) {
            out.push('"');
            out.push_str(t);
            out.push('"');
            i += t.len();
            continue;
        }
        if c == '"' {
            in_str = true;
        }
        // copy one UTF-8 char
        let n = rest.chars().next().map_or(1, |ch| ch.len_utf8());
        out.push_str(&rest[..n]);
        i += n;
    }
    serde_json::from_str(&out).ok()
}

pub fn json_of(root: &Path, pkg: &str) -> Vec<Value> {
    let p = root.join("json").join(format!("{pkg}.json"));
    let Ok(t) = std::fs::read_to_string(p) else { return vec![] };
    match parse_lenient(&t) {
        Some(Value::Array(a)) => a,
        _ => vec![],
    }
}

/// Package paths ("Mordhau/Content/...", no extension) of every JSON under extract/json/<dir>, sorted
pub fn packages_under(root: &Path, dir: &str, recursive: bool) -> Vec<String> {
    let base = root.join("json");
    let mut out = Vec::new();
    let mut stack = vec![dir.to_string()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(base.join(&d)) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if recursive {
                    stack.push(format!("{d}/{n}"));
                }
            } else if n.ends_with(".json") && !n.ends_with(".vcolors.json") {
                out.push(format!("{d}/{}", &n[..n.len() - 5]));
            }
        }
    }
    out.sort();
    out
}

fn f32r(x: f64) -> f64 {
    x as f32 as f64
}

impl Equiv {
    pub fn compare_package(&mut self, rd: &Reader, root: &Path, pkg: &str) {
        let js = json_of(root, pkg);
        if js.is_empty() {
            self.missing_json += 1;
            return;
        }
        let Some(asset) = rd.open(pkg) else {
            self.mismatches.push(Mismatch { pkg: pkg.into(), path: String::new(), pak: "package not in paks".into(), json: "".into() });
            self.packages += 1;
            return;
        };
        let before = self.mismatches.len();
        let unsup0 = self.unsupported;
        self.packages += 1;
        if asset.exports.len() != js.len() {
            self.mismatches.push(Mismatch {
                pkg: pkg.into(),
                path: "export count".into(),
                pak: asset.exports.len().into(),
                json: js.len().into(),
            });
        }
        for (i, theirs) in js.iter().enumerate().take(asset.exports.len()) {
            let mine = rd.export_json(&asset, i);
            for &k in COMPARED {
                let (a, b) = (mine.get(k), theirs.get(k));
                if a.is_none() && b.is_none() {
                    continue;
                }
                self.fields += 1;
                self.ty = theirs.get("Type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = theirs.get("Name").and_then(|v| v.as_str()).unwrap_or("?");
                let null = Value::Null;
                self.eq(pkg, &format!("{name}.{k}"), a.unwrap_or(&null), b.unwrap_or(&null));
            }
        }
        if self.mismatches.len() == before && self.unsupported == unsup0 {
            self.packages_exact += 1;
        }
    }

    fn miss(&mut self, pkg: &str, path: String, a: Value, b: Value) {
        self.mismatches.push(Mismatch { pkg: pkg.into(), path, pak: a, json: b });
    }

    pub fn eq(&mut self, pkg: &str, path: &str, a: &Value, b: &Value) {
        if let Value::Object(am) = a {
            if let Some(why) = am.get(UNSUPPORTED) {
                self.unsupported += 1;
                *self.unsupported_types.entry(self.ty.clone()).or_insert(0) += 1;
                let w: Vec<&str> = why.as_str().unwrap_or("").split(' ').take(3).collect();
                *self.unsupported_reasons.entry(w.join(" ")).or_insert(0) += 1;
                return;
            }
        }
        match (a, b) {
            (Value::Object(am), Value::Object(bm)) => {
                let mut ak: BTreeMap<String, &Value> = BTreeMap::new();
                let mut order = Vec::new();
                for (k, v) in am {
                    let t = k.trim().to_string();
                    if ak.insert(t.clone(), v).is_none() {
                        order.push(t);
                    }
                }
                let mut bk: BTreeMap<String, &Value> = BTreeMap::new();
                for (k, v) in bm {
                    let ks = k.trim().to_string();
                    if !ak.contains_key(&ks) && !DERIVED.contains(&ks.as_str()) {
                        self.miss(pkg, format!("{path}/{ks}"), "<missing>".into(), v.clone());
                    }
                    bk.entry(ks).or_insert(v);
                }
                for k in order {
                    match bk.get(&k) {
                        None => self.miss(pkg, format!("{path}/{k}"), ak[&k].clone(), "<missing>".into()),
                        Some(bv) => self.eq(pkg, &format!("{path}/{k}"), ak[&k], bv),
                    }
                }
            }
            (Value::Array(aa), Value::Array(ba)) => {
                if aa.len() != ba.len() {
                    self.miss(pkg, path.into(), format!("len {}", aa.len()).into(), format!("len {}", ba.len()).into());
                    return;
                }
                for (i, (x, y)) in aa.iter().zip(ba).enumerate() {
                    self.eq(pkg, &format!("{path}[{i}]"), x, y);
                }
            }
            _ => {
                self.leaves += 1;
                let ok = match (a, b) {
                    (Value::Number(x), Value::Number(y)) => {
                        if x.is_f64() {
                            let (xf, yf) = (x.as_f64().unwrap(), y.as_f64().unwrap_or(f64::NAN));
                            xf == f32r(yf) || xf == yf
                        } else if let (Some(xi), Some(yi)) = (as_i128(x), as_i128(y)) {
                            xi == yi
                        } else {
                            x.as_f64() == y.as_f64()
                        }
                    }
                    (Value::String(x), Value::String(y)) => {
                        if x == y {
                            true
                        } else if path.ends_with("/ObjectPath") && y.starts_with('/') && !x.starts_with('/') {
                            // an import CUE4Parse left unresolved (e.g. a /Niagara/ plugin package) found in the paks
                            self.resolved_further += 1;
                            return;
                        } else if x.trim() == y.trim() {
                            self.trimmed += 1;
                            return;
                        } else {
                            false
                        }
                    }
                    (Value::Bool(x), Value::Bool(y)) => x == y,
                    (Value::Null, Value::Null) => true,
                    _ => false,
                };
                if ok {
                    self.matched += 1;
                } else {
                    self.miss(pkg, path.into(), a.clone(), b.clone());
                }
            }
        }
    }

    /// Mismatches explained by CUE4Parse's FVector_NetQuantize* misread: the JSON side is a nonzero float32 denormal
    /// (an int field of the tagged struct read as a float), or a sibling field of such a value in the same struct.
    /// Returns (explained, unexplained).
    pub fn split_misreads(&self) -> (Vec<&Mismatch>, Vec<&Mismatch>) {
        let base = |p: &str| p.rsplit_once('/').map_or(String::new(), |(a, _)| a.to_string());
        let parents: HashSet<String> = self
            .mismatches
            .iter()
            .filter(|m| matches!(&m.json, Value::Number(n) if n.as_f64().is_some_and(|j| j != 0.0 && j.abs() < FLT_MIN)))
            .map(|m| base(&m.path))
            .collect();
        self.mismatches.iter().partition(|m| parents.contains(&base(&m.path)))
    }

    pub fn summary(&self) -> String {
        format!(
            "{} packages ({} exact), {} export fields, {} values: {} equal, {} differ only by CUE4Parse's name trim, {} imports resolved where CUE4Parse left them unresolved, {} unsupported, {} mismatches{}",
            self.packages,
            self.packages_exact,
            self.fields,
            self.leaves,
            self.matched,
            self.trimmed,
            self.resolved_further,
            self.unsupported,
            self.mismatches.len(),
            if self.missing_json == 0 { String::new() } else { format!(", {} without JSON", self.missing_json) }
        )
    }
}

fn as_i128(n: &serde_json::Number) -> Option<i128> {
    n.as_i64().map(|x| x as i128).or_else(|| n.as_u64().map(|x| x as i128))
}

/// Compare every package of `sets` ([dir under Mordhau/Content/Mordhau, recursive]) then `singles`, clearing the
/// reader's cache after each set (test_pak_equiv.gd _run)
pub fn run(rd: &Reader, root: &Path, sets: &[(&str, bool)], singles: &[&str]) -> Equiv {
    let mut pe = Equiv::default();
    for &(d, rec) in sets {
        for p in packages_under(root, &rel(d), rec) {
            pe.compare_package(rd, root, &p);
        }
        rd.clear_cache();
    }
    for s in singles {
        pe.compare_package(rd, root, &rel(s));
    }
    pe
}

/// Undecoded values allowed only in UMG animation exports (MovieScene*, WidgetAnimation: sequencer channel internals)
/// and the maps' Landscape* exports (terrain, which no reader builds yet) - test_pak_equiv.gd
pub fn disallowed_unsupported(pe: &Equiv) -> BTreeMap<String, usize> {
    pe.unsupported_types
        .iter()
        .filter(|(t, _)| !(t.starts_with("MovieScene") || t.starts_with("WidgetAnimation") || t.starts_with("Landscape")))
        .map(|(t, n)| (t.clone(), *n))
        .collect()
}

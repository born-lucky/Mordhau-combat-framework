//! Wearable colours from the colour tables, read from the paks: BP_MordhauSingleton ColorTables[table].Entries[entry]
//! -> that colour Blueprint's `Color` (class defaults over the Blueprint chain), and a wearable's colours for a loadout
//! slot. Port of godot/components/ue/records/ue_wearable.gd `color` and game/character/character_builder.gd
//! `colors_of`:
//!   FWearableCustomization.Colors are copied onto the wearable instance (AMordhauCharacter::
//!   UpdateWearableInstanceColorsAndPatterns rva=0x1575d90); colour i = ColorTables[wearable.ColorTables[i]]
//!   .Entries[Colors[i]]; UseColorsFromSlot (+0x60, 10 = Invalid) borrows another slot's colour indexes.
//!   A wearable without a ColorTables property has the native default: the UMordhauWearable ctor (rva=0x1612dc0)
//!   appends two entries of 0 (ue_wearable.gd load_wearable).
//! Out-of-range table / entry -> white (ue_wearable.gd color). The sheets builder's spec (data_gen/spec/entities/
//! color.json, `UeWearable.color`) holds the same table; tests/cosmetics.rs checks both.

use crate::material::ue_pkg_path;
use mh_pak::Reader;
use serde_json::Value;

pub const SINGLETON: &str = "Mordhau/Content/Mordhau/Blueprints/BP_MordhauSingleton";
/// EWearableSlot::Invalid (UseColorsFromSlot default)
pub const SLOT_INVALID: i64 = 10;
/// EWearableSlot names (MordhauCustomizationTypes.h:38; ue_wearable.gd SLOT_NAMES)
pub const SLOT_NAMES: [&str; 9] = ["Head", "Coif", "UpperChest", "LowerChest", "Shoulders", "Arms", "Hands", "Legs", "Feet"];

pub struct ColorTables {
    /// per table, per entry: linear RGBA
    pub tables: Vec<Vec<[f64; 4]>>,
}

impl ColorTables {
    /// BP_MordhauSingleton ColorTables, each entry's Blueprint `Color` (absent -> white, ue_wearable.gd color)
    pub fn read(rd: &Reader) -> ColorTables {
        let d = rd.defaults(SINGLETON);
        let mut tables = Vec::new();
        for t in d.get("ColorTables").and_then(Value::as_array).into_iter().flatten() {
            let mut entries = Vec::new();
            for e in t.get("Entries").and_then(Value::as_array).into_iter().flatten() {
                let p = ue_pkg_path(e);
                let c = if p.is_empty() { None } else { rd.defaults(&p).get("Color").cloned() };
                let g = |k: &str| c.as_ref().and_then(|c| c.get(k)).and_then(Value::as_f64).unwrap_or(1.0);
                entries.push([g("R"), g("G"), g("B"), g("A")]);
            }
            tables.push(entries);
        }
        ColorTables { tables }
    }

    /// colour `entry` of table `table`; white when out of range
    pub fn color(&self, table: i64, entry: i64) -> [f64; 4] {
        if table < 0 || entry < 0 {
            return [1.0; 4];
        }
        self.tables.get(table as usize).and_then(|t| t.get(entry as usize)).copied().unwrap_or([1.0; 4])
    }
}

/// A wearable class's colour inputs: ColorTables (native default [0, 0]) and UseColorsFromSlot (EWearableSlot; a
/// Blueprint stores "EWearableSlot::Name", the ctor byte 10 = Invalid)
pub fn wearable_color_setup(rd: &Reader, class_pkg: &str) -> (Vec<i64>, i64) {
    let d = rd.defaults(class_pkg);
    let tables = d
        .get("ColorTables")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|x| x.as_i64().unwrap_or(0)).collect())
        .unwrap_or_else(|| vec![0, 0]);
    let slot = match d.get("UseColorsFromSlot") {
        Some(Value::String(s)) => {
            let n = s.split("::").nth(1).unwrap_or("");
            SLOT_NAMES.iter().position(|x| *x == n).map_or(if n == "Invalid" { SLOT_INVALID } else { -1 }, |i| i as i64)
        }
        Some(v) => v.as_i64().unwrap_or(SLOT_INVALID),
        None => SLOT_INVALID,
    };
    (tables, slot)
}

/// ColorA/B/C (linear RGB) of the wearable in `slot` for a loadout's per-slot colour indexes (FWearableCustomization
/// .Colors; character_builder.gd colors_of): the wearable's tables, indexes from its own slot or UseColorsFromSlot's;
/// slots past the wearable's tables keep the master defaults (FF0000 / 000000 / FFFFFF, SHADERS.md 7)
pub fn wearable_colors(ct: &ColorTables, tables: &[i64], use_slot: i64, slot: usize, slot_colors: &[Vec<i64>]) -> [[f32; 3]; 3] {
    let src = if use_slot < 0 || use_slot >= SLOT_NAMES.len() as i64 { slot } else { use_slot as usize };
    let idx: &[i64] = slot_colors.get(src).map_or(&[], |v| v.as_slice());
    let mut out = [[1.0, 0.0, 0.0], [0.0; 3], [1.0; 3]];
    for (i, &t) in tables.iter().enumerate().take(3) {
        let c = ct.color(t, idx.get(i).copied().unwrap_or(0));
        out[i] = [c[0] as f32, c[1] as f32, c[2] as f32];
    }
    out
}

/// One FCharacterProfile of BP_MordhauSingleton (`field` = "DefaultProfiles" / "BotCharacterProfiles"): its display name
/// and per-slot (wearable Id, Colors, Pattern)
#[derive(Debug, Clone)]
pub struct ProfileGear {
    pub name: String,
    pub ids: Vec<i64>,
    pub colors: Vec<Vec<i64>>,
    pub patterns: Vec<i64>,
}

pub fn profiles(rd: &Reader, field: &str) -> Vec<ProfileGear> {
    let d = rd.defaults(SINGLETON);
    let mut out = Vec::new();
    for p in d.get(field).and_then(Value::as_array).into_iter().flatten() {
        let n = p.get("Name");
        let name = n
            .and_then(|n| n.get("LocalizedString").or_else(|| n.get("SourceString")).or_else(|| n.get("CultureInvariantString")))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let w = p.get("GearCustomization").and_then(|g| g.get("Wearables")).and_then(Value::as_array).cloned().unwrap_or_default();
        let ints = |v: Option<&Value>| -> Vec<i64> { v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default() };
        out.push(ProfileGear {
            name,
            ids: w.iter().map(|x| x.get("Id").and_then(Value::as_i64).unwrap_or(0)).collect(),
            colors: w.iter().map(|x| ints(x.get("Colors"))).collect(),
            patterns: w.iter().map(|x| x.get("Pattern").and_then(Value::as_i64).unwrap_or(0)).collect(),
        });
    }
    out
}

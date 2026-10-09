//! armory.rs - the Mercenaries (loadout / customization) data the Armory screens' Blueprints read and write through
//! natives: FCharacterProfile values (as the VM holds them: BP_MordhauSingleton's DefaultProfiles / CharacterProfiles
//! struct shape), the point budget rules, profile validation, and the singleton's config save / load.
//!
//! Ported (extract/native/decomp, Mordhau-Win64-Shipping.exe):
//!   UMordhauUtilityLibrary::ComputePointsLeft rva=0x16189a0 (UMordhauUtilityLibrary.cpp 14916-14992)
//!   UMordhauUtilityLibrary::ForceValidCharacterProfile (UMordhauUtilityLibrary.cpp 21972-22262): perk budget, name
//!     length, the points pass over wearables then equipment, peasant restrictions
//!   FSkillsCustomization::GetPerksCost / GetArchetypeObject (FSkillsCustomization.cpp), GetIsPeasant rva=0x153fca0
//!   FCharacterGearCustomization::GetWearableClass (FCharacterGearCustomization.cpp): parent-slot child lists
//!   UMordhauSingleton::GetEquipmentDefaultObject (UMordhauSingleton.cpp 5876-5910)
//!   UMordhauSingleton::LoadFromConfig / SaveToConfig rva=0x15f2920 (UMordhauSingleton.cpp 5812-5869):
//!     UObject::LoadConfig / SaveConfig(CPF_Config) of the singleton into the "Game" config; here the rewrite's own
//!     settings::config_dir()/Game.ini, never the Steam install's files.
//!   UMordhauSingleton::PostLoad (UMordhauSingleton.cpp 3403-3584): a fresh install (no saved profiles) keeps the CDO's
//!     CharacterProfiles (BP_MordhauSingleton CDO: one "Unnamed" profile, SingletonVersion 11 = current, so no upgrade).
//! Field offsets / defaults: extract/native/types (UMordhauWearable.h CharacterPointCost +0x1b8, bIsAllowedForPeasants
//! +0x1bd; AMordhauEquipment.h CharacterPointCost +0x6c0 = 1 in AMordhauEquipment::AMordhauEquipment, bOnlyPeasants
//! +0x68d, bIsAllowedForPeasants +0x68e; UPerk.h Cost +0x40; UArchetype.h CharacterPoints +0x28).

use crate::model::*;
use crate::vm::Vm;
use std::collections::HashMap;

pub const SINGLETON_SECTION: &str = "/Game/Mordhau/Blueprints/BP_MordhauSingleton.BP_MordhauSingleton_C";

/// EWearableSlot (extract/native/types/EWearableSlot.h)
pub const HEAD: usize = 0;
pub const COIF: usize = 1;
pub const UPPER_CHEST: usize = 2;
pub const LOWER_CHEST: usize = 3;
pub const SHOULDERS: usize = 4;
pub const ARMS: usize = 5;
pub const HANDS: usize = 6;
pub const LEGS: usize = 7;
pub const FEET: usize = 8;
pub const SLOTS: usize = 9;

/// a class reference value (TSubclassOf object path or TSoftClassPtr {AssetPathName}) -> its Blueprint package
pub fn class_pkg(v: &V) -> Option<String> {
    let p = match v {
        V::Asset(a) => return (!a.package.is_empty() && a.package != "None").then(|| a.package.clone()),
        V::Struct(_) => v.field("AssetPathName").s(),
        V::Name(s) | V::Str(s) => s.clone(),
        _ => return None,
    };
    if p.is_empty() || p == "None" {
        return None;
    }
    let pk = p.rsplit_once('.').map(|x| x.0).unwrap_or(&p);
    Some(crate::kismet::content_path(pk))
}

/// the class default object's properties of a Blueprint class package (CDO merged over the parents')
pub fn cdo(vm: &mut Vm, pkg: &str) -> Option<HashMap<String, V>> {
    vm.bp_class(pkg).map(|c| c.cdo.clone())
}

fn singleton(vm: &Vm) -> Option<crate::model::Id> {
    vm.world.get("singleton").copied()
}

fn sprop(vm: &Vm, k: &str) -> V {
    singleton(vm).map(|s| vm.prop(s, k)).unwrap_or_default()
}

fn at(v: &V, i: i64) -> Option<V> {
    if i < 0 {
        return None;
    }
    v.arr().get(i as usize).cloned()
}

/// wearable ids of a FCharacterGearCustomization value
pub fn wearable_ids(gear: &V) -> Vec<i64> {
    (0..SLOTS).map(|i| gear.field("Wearables").arr().get(i).map(|w| w.field("Id").i()).unwrap_or(0)).collect()
}

/// FCharacterGearCustomization::GetWearableClass: (wearable class package, the slot's default id `DefaultValid`).
/// The child slots read their parent's class lists: Coif <- Head CDO CoifWearables, LowerChest / Shoulders / Arms <-
/// UpperChest CDO, Hands <- the Arms class (UpperChest CDO ArmsWearables[ids[Arms]]) HandsWearables, Feet <- Legs CDO
/// FeetWearables; the three root slots index the singleton's HeadWearables / UpperChestWearables / LegsWearables and
/// take DefaultHead / DefaultUpperChest / DefaultLegs. (The decomp shows the Coif / LowerChest / Feet getters ICF-folded
/// into UArmsWearable::GetHandsWearable: same array-at-offset code.)
pub fn wearable_class(vm: &mut Vm, gear: &V, slot: usize) -> (Option<String>, i64) {
    let own = wearable_ids(gear).get(slot).copied().unwrap_or(0);
    match wearable_list(vm, gear, slot) {
        Some((list, def)) => (at(&V::Array(list), own).and_then(|v| class_pkg(&v)), def),
        None => (None, 0),
    }
}

/// the class list a slot's Id indexes (FCharacterGearCustomization::GetWearableArray, as GetWearableClass walks it) and
/// the slot's default id; None when the parent slot's class is missing
pub fn wearable_list(vm: &mut Vm, gear: &V, slot: usize) -> Option<(Vec<V>, i64)> {
    let ids = wearable_ids(gear);
    let parent = match slot {
        COIF => HEAD,
        LOWER_CHEST..=ARMS => UPPER_CHEST,
        HANDS => ARMS,
        FEET => LEGS,
        s => s,
    };
    let pid = ids.get(parent).copied().unwrap_or(0);
    let root = |vm: &Vm, list: &str, i: i64| at(&sprop(vm, list), i).and_then(|v| class_pkg(&v));
    let list = |d: &HashMap<String, V>, k: &str| d.get(k).map(|l| l.arr().to_vec()).unwrap_or_default();
    let def = |d: &HashMap<String, V>, k: &str| d.get(k).map(V::i).unwrap_or(0);
    match parent {
        UPPER_CHEST => {
            let k = root(vm, "UpperChestWearables", pid)?;
            let ucd = cdo(vm, &k)?;
            Some(match slot {
                UPPER_CHEST => (sprop(vm, "UpperChestWearables").arr().to_vec(), sprop(vm, "DefaultUpperChest").i()),
                SHOULDERS => (list(&ucd, "ShouldersWearables"), def(&ucd, "DefaultShoulders")),
                ARMS => (list(&ucd, "ArmsWearables"), def(&ucd, "DefaultArms")),
                _ => (list(&ucd, "LowerChestWearables"), def(&ucd, "DefaultLowerChest")),
            })
        }
        ARMS => {
            // (pFVar2 + 0x80) = Wearables[UpperChest].Id; -1 < iVar1 < ArmsWearables.ArrayNum
            let k = root(vm, "UpperChestWearables", ids[UPPER_CHEST])?;
            let ucd = cdo(vm, &k)?;
            let arms = ucd.get("ArmsWearables").and_then(|l| at(l, pid)).and_then(|v| class_pkg(&v))?;
            let ad = cdo(vm, &arms)?;
            Some((list(&ad, "HandsWearables"), def(&ad, "DefaultHands")))
        }
        HEAD => {
            let k = root(vm, "HeadWearables", pid)?;
            let hdd = cdo(vm, &k)?;
            Some(if slot == HEAD { (sprop(vm, "HeadWearables").arr().to_vec(), sprop(vm, "DefaultHead").i()) } else { (list(&hdd, "CoifWearables"), def(&hdd, "DefaultCoif")) })
        }
        LEGS => {
            let k = root(vm, "LegsWearables", pid)?;
            let lgd = cdo(vm, &k)?;
            Some(if slot == LEGS { (sprop(vm, "LegsWearables").arr().to_vec(), sprop(vm, "DefaultLegs").i()) } else { (list(&lgd, "FeetWearables"), def(&lgd, "DefaultFeet")) })
        }
        _ => None,
    }
}

/// UMordhauSingleton::GetEquipmentDefaultObject: Equipment[Id]'s class package (None when out of range / empty)
pub fn equipment_class(vm: &Vm, id: i64) -> Option<String> {
    at(&sprop(vm, "Equipment"), id).and_then(|v| class_pkg(&v))
}

/// FSkillsCustomization::GetArchetypeObject: Archetypes[0]'s CDO CharacterPoints (UArchetype +0x28)
pub fn archetype_points(vm: &mut Vm) -> Option<i64> {
    let a = at(&sprop(vm, "Archetypes"), 0).and_then(|v| class_pkg(&v))?;
    Some(cdo(vm, &a)?.get("CharacterPoints").map(V::i).unwrap_or(0))
}

/// the singleton's perk classes whose bits are set (bit i = Perks[i]; GetPerksCost walks the set bits from the top
/// via leading-zero count, index = 31 - lzcnt = the bit number)
pub fn perk_classes(vm: &Vm, perks: i64) -> Vec<(usize, String)> {
    let list = sprop(vm, "Perks");
    let p = perks as u32;
    (0..32usize).rev().filter(|b| p & (1u32 << b) != 0).filter_map(|b| at(&list, b as i64).and_then(|v| class_pkg(&v)).map(|c| (b, c))).collect()
}

/// FSkillsCustomization::GetPerksCost: sum of the set perks' CDO Cost (UPerk +0x40)
pub fn perks_cost(vm: &mut Vm, perks: i64) -> i64 {
    let mut t = 0;
    for (_, c) in perk_classes(vm, perks) {
        t += cdo(vm, &c).and_then(|d| d.get("Cost").map(V::i)).unwrap_or(0);
    }
    t
}

fn wearable_cost(vm: &mut Vm, pkg: &str) -> (i64, bool) {
    let d = cdo(vm, pkg).unwrap_or_default();
    // UMordhauWearable ctor sets neither (zeroed): absent = 0 / false
    (d.get("CharacterPointCost").map(V::i).unwrap_or(0), d.get("bIsAllowedForPeasants").map(V::truthy).unwrap_or(false))
}

/// (CharacterPointCost, bOnlyPeasants, bIsAllowedForPeasants) of an equipment class (ctor: cost 1, flags false)
fn equipment_cost(vm: &mut Vm, pkg: &str) -> (i64, bool, bool) {
    let d = cdo(vm, pkg).unwrap_or_default();
    (d.get("CharacterPointCost").map(V::i).unwrap_or(1), d.get("bOnlyPeasants").map(V::truthy).unwrap_or(false), d.get("bIsAllowedForPeasants").map(V::truthy).unwrap_or(false))
}

/// UMordhauUtilityLibrary::ComputePointsLeft rva=0x16189a0: Archetype CharacterPoints - perks cost - every equipment
/// CDO's CharacterPointCost - every slot's wearable CDO CharacterPointCost
pub fn points_left(vm: &mut Vm, profile: &V) -> i64 {
    let Some(base) = archetype_points(vm) else { return 0 };
    let gear = profile.field("GearCustomization").clone();
    let mut left = base - perks_cost(vm, profile.field("SkillsCustomization").field("Perks").i());
    for e in gear.field("Equipment").arr() {
        if let Some(c) = equipment_class(vm, e.field("Id").i()) {
            left -= equipment_cost(vm, &c).0;
        }
    }
    for s in 0..SLOTS {
        if let (Some(c), _) = wearable_class(vm, &gear, s) {
            left -= wearable_cost(vm, &c).0;
        }
    }
    left
}

/// FWearableCustomization::FWearableCustomization rva=0x1528610: Id 0, two 0 colours in each colour array, Pattern 0
pub fn empty_wearable(id: i64) -> V {
    let z = || V::Array(vec![V::Int(0), V::Int(0)]);
    V::st(&[("Id", V::Int(id)), ("Colors", z()), ("Team1Colors", z()), ("Team2Colors", z()), ("Pattern", V::Int(0))])
}

/// the empty FEquipmentCustomization ForceValidCharacterProfile writes over a disallowed item: Id 0, Colors / Parts
/// three 0 bytes each (decomp 22198-22223: ResizeGrow by 3 then zeroed), Pattern 0, Skin 0
fn empty_equipment() -> V {
    let z = || V::Array(vec![V::Int(0), V::Int(0), V::Int(0)]);
    V::st(&[("Id", V::Int(0)), ("Colors", z()), ("Parts", z()), ("Pattern", V::Int(0)), ("Skin", V::Int(0))])
}

fn set_path(v: &mut V, path: &[&str], nv: V) {
    if path.is_empty() {
        *v = nv;
        return;
    }
    if !matches!(v, V::Struct(_)) {
        *v = V::Struct(Box::default());
    }
    if let V::Struct(m) = v {
        let e = m.entry(path[0].to_string()).or_insert(V::None);
        set_path(e, &path[1..], nv);
    }
}

fn set_index(v: &mut V, i: usize, nv: V) {
    if let V::Array(a) = v {
        if i < a.len() {
            a[i] = nv;
        }
    }
}

/// UMordhauUtilityLibrary::ForceValidCharacterProfile (decomp 21972-22262), the gameplay part:
/// - perks over the archetype's points -> Perks 0;
/// - a name longer than 32 characters keeps its first 32 (FText::FromString of the truncated copy);
/// - points pass in slot order: a wearable that the remaining points cannot pay for, or (peasant: Perks & 8) one that
///   is not bIsAllowedForPeasants, is replaced by FWearableCustomization() with Id = the slot's default;
/// - then equipment in order: over budget -> empty equipment; non-peasant with bOnlyPeasants -> empty; peasant with
///   neither bIsAllowedForPeasants nor bOnlyPeasants -> empty.
/// UNCONFIRMED / not ported: FAppearanceCustomization::Validate and FCharacterGearCustomization::Validate (index ranges,
/// colour array sizes, MaxAmountPerLoadout; godot/game/ui/customization_model.gd ports the latter), inventory ownership
/// (offline: everything owned).
pub fn force_valid(vm: &mut Vm, profile: &V) -> (bool, V) {
    let mut p = profile.clone();
    let Some(base) = archetype_points(vm) else { return (false, p) };
    let perks = p.field("SkillsCustomization").field("Perks").i();
    if perks_cost(vm, perks) > base {
        set_path(&mut p, &["SkillsCustomization", "Perks"], V::Int(0));
    }
    let name = p.field("Name").s();
    if name.chars().count() > 32 {
        set_path(&mut p, &["Name"], V::Text(name.chars().take(32).collect()));
    }
    let peasant = p.field("SkillsCustomization").field("Perks").i() & 8 != 0;
    let mut left = base - perks_cost(vm, p.field("SkillsCustomization").field("Perks").i());
    let n = p.field("GearCustomization").field("Wearables").arr().len();
    for s in 0..n.min(SLOTS) {
        let gear = p.field("GearCustomization").clone();
        let (cls, def) = wearable_class(vm, &gear, s);
        let Some(c) = cls else { continue };
        let (cost, allowed) = wearable_cost(vm, &c);
        if (peasant && !allowed) || left - cost < 0 {
            if let V::Struct(m) = &mut p {
                if let Some(V::Struct(g)) = m.get_mut("GearCustomization") {
                    if let Some(w) = g.get_mut("Wearables") {
                        set_index(w, s, empty_wearable(def));
                    }
                }
            }
        } else {
            left -= cost;
        }
    }
    let ne = p.field("GearCustomization").field("Equipment").arr().len();
    for i in 0..ne {
        let id = p.field("GearCustomization").field("Equipment").arr()[i].field("Id").i();
        let Some(c) = equipment_class(vm, id) else { continue };
        let (cost, only_peasants, allowed_peasants) = equipment_cost(vm, &c);
        let bad = left - cost < 0 || (!peasant && only_peasants) || (peasant && !allowed_peasants && !only_peasants);
        if bad {
            if let V::Struct(m) = &mut p {
                if let Some(V::Struct(g)) = m.get_mut("GearCustomization") {
                    if let Some(e) = g.get_mut("Equipment") {
                        set_index(e, i, empty_equipment());
                    }
                }
            }
            // the over-budget branch leaves the remaining points unchanged (iVar21 = iVar9, decomp 22221)
        } else {
            left -= cost;
        }
    }
    (true, p)
}

// ---- config: UObject::SaveConfig / LoadConfig of the singleton ------------------------------------------------------

/// the singleton's config properties written by SaveToConfig (CPF_Config members of UMordhauSingleton: UNCONFIRMED
/// which members carry the flag - the reflection data isn't in the decomp; these are the ones the Armory Blueprints
/// change before calling SaveToConfig: BP_ProfileCustomization:SaveFunction, BP_LoadoutPicker)
pub const CONFIG_PROPS: &[&str] = &["SingletonVersion", "CharacterProfiles", "DefaultCharacterEquipment", "DefaultCharacterTier", "DefaultCharacterFace", "DefaultCharacterAppearance"];

pub fn game_ini() -> std::path::PathBuf {
    crate::settings::config_dir().join("Game.ini")
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// FProperty::ExportTextItem in UE text form: struct (A=..,B=..), array (a,b), text INVTEXT("..") / NSLOCTEXT, string
/// "..", names bare, bools True/False
pub fn export_text(v: &V) -> String {
    match v {
        V::None => "None".into(),
        V::Bool(b) => if *b { "True" } else { "False" }.into(),
        V::Int(i) => i.to_string(),
        V::Float(f) => format!("{f:.6}"),
        V::Name(s) => s.clone(),
        V::Str(s) => format!("\"{}\"", esc(s)),
        V::Text(s) => format!("INVTEXT(\"{}\")", esc(s)),
        V::Asset(a) => format!("\"{}.{}\"", a.package, a.name),
        V::Struct(m) => format!("({})", m.iter().map(|(k, x)| format!("{k}={}", export_text(x))).collect::<Vec<_>>().join(",")),
        V::Array(a) => format!("({})", a.iter().map(export_text).collect::<Vec<_>>().join(",")),
        _ => String::new(),
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> u8 {
        self.s.get(self.i).copied().unwrap_or(0)
    }
    fn quoted(&mut self) -> String {
        self.i += 1;
        let mut out = vec![];
        while self.i < self.s.len() && self.peek() != b'"' {
            if self.peek() == b'\\' {
                self.i += 1;
            }
            out.push(self.peek());
            self.i += 1;
        }
        self.i += 1;
        String::from_utf8_lossy(&out).into_owned()
    }
    fn value(&mut self) -> V {
        match self.peek() {
            b'"' => V::Str(self.quoted()),
            b'(' => {
                self.i += 1;
                // struct if the first item is Key=
                let save = self.i;
                let mut j = self.i;
                while j < self.s.len() && !matches!(self.s[j], b'=' | b',' | b'(' | b')' | b'"') {
                    j += 1;
                }
                let is_struct = self.s.get(j) == Some(&b'=');
                self.i = save;
                if is_struct {
                    let mut m = std::collections::BTreeMap::new();
                    while self.i < self.s.len() && self.peek() != b')' {
                        let k0 = self.i;
                        while self.peek() != b'=' && self.i < self.s.len() {
                            self.i += 1;
                        }
                        let k = String::from_utf8_lossy(&self.s[k0..self.i]).trim().to_string();
                        self.i += 1;
                        let v = self.value();
                        m.insert(k, v);
                        if self.peek() == b',' {
                            self.i += 1;
                        }
                    }
                    self.i += 1;
                    V::Struct(Box::new(m))
                } else {
                    let mut a = vec![];
                    while self.i < self.s.len() && self.peek() != b')' {
                        a.push(self.value());
                        if self.peek() == b',' {
                            self.i += 1;
                        }
                    }
                    self.i += 1;
                    V::Array(a)
                }
            }
            _ => {
                let k0 = self.i;
                while self.i < self.s.len() && !matches!(self.peek(), b',' | b')') {
                    if self.peek() == b'(' {
                        // INVTEXT("...") / NSLOCTEXT("ns","key","src"): the last quoted argument is the source string
                        let head = String::from_utf8_lossy(&self.s[k0..self.i]).trim().to_string();
                        self.i += 1;
                        let mut last = String::new();
                        while self.i < self.s.len() && self.peek() != b')' {
                            if self.peek() == b'"' {
                                last = self.quoted();
                            } else {
                                self.i += 1;
                            }
                        }
                        self.i += 1;
                        if head.ends_with("TEXT") {
                            return V::Text(last);
                        }
                        return V::Str(last);
                    }
                    self.i += 1;
                }
                let t = String::from_utf8_lossy(&self.s[k0..self.i]).trim().to_string();
                match t.as_str() {
                    "True" | "true" => V::Bool(true),
                    "False" | "false" => V::Bool(false),
                    _ => t.parse::<i64>().map(V::Int).or_else(|_| t.parse::<f64>().map(V::Float)).unwrap_or(V::Name(t)),
                }
            }
        }
    }
}

pub fn import_text(s: &str) -> V {
    P { s: s.as_bytes(), i: 0 }.value()
}

/// lay a parsed config value over the typed default (keeps value kinds: Text names, Str categories)
fn retype(def: &V, v: V) -> V {
    match (def, v) {
        (V::Text(_), V::Str(s)) | (V::Text(_), V::Name(s)) => V::Text(s),
        (V::Str(_), V::Name(s)) => V::Str(s),
        (V::Name(_), V::Str(s)) => V::Name(s),
        (V::Struct(d), V::Struct(m)) => {
            let mut out = d.clone();
            for (k, x) in m.into_iter() {
                let nv = match d.get(&k) {
                    Some(dk) => retype(dk, x),
                    None => x,
                };
                out.insert(k, nv);
            }
            V::Struct(out)
        }
        (V::Array(d), V::Array(a)) => {
            let proto = d.first().cloned().unwrap_or_default();
            V::Array(a.into_iter().map(|x| retype(&proto, x)).collect())
        }
        (_, v) => v,
    }
}

/// UMordhauSingleton::LoadFromConfig: the saved config properties over the CDO's (a fresh install has none: the CDO's
/// CharacterProfiles, one "Unnamed" profile, stay; UMordhauSingleton::PostLoad adds MakeEmptyProfile only when that
/// array is empty, which the shipped CDO never is). Category "Default" etc. keep their kinds via the CDO's values.
pub fn load_config(vm: &mut Vm, so: crate::model::Id) {
    // once per singleton (host.rs at mount, or the ArmoryPlugin's first frame)
    if vm.prop(so, "__armory_loaded").truthy() {
        return;
    }
    vm.set(so, "__armory_loaded", V::Bool(true));
    let Ok(text) = std::fs::read_to_string(game_ini()) else { return };
    let mut in_sec = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_sec = l == format!("[{SINGLETON_SECTION}]");
            continue;
        }
        if !in_sec {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        if !CONFIG_PROPS.contains(&k) {
            continue;
        }
        let def = vm.prop(so, k);
        let proto = match (&def, k) {
            (V::Array(a), _) if a.is_empty() && k == "CharacterProfiles" => V::Array(sprop(vm, "DefaultProfiles").arr().first().cloned().into_iter().collect()),
            _ => def.clone(),
        };
        vm.set(so, k, retype(&proto, import_text(v)));
    }
}

/// UMordhauSingleton::SaveToConfig: rewrite the singleton's section of the rewrite's Game.ini (other sections kept)
pub fn save_config(vm: &Vm, so: crate::model::Id) -> std::io::Result<()> {
    let path = game_ini();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::new();
    let mut skip = false;
    for line in old.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            skip = l == format!("[{SINGLETON_SECTION}]");
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&format!("[{SINGLETON_SECTION}]\n"));
    for k in CONFIG_PROPS {
        // a property the singleton never had a value for (not in the CDO, never set) stays at its default
        let v = vm.prop(so, k);
        if !matches!(v, V::None) {
            out.push_str(&format!("{k}={}\n", export_text(&v)));
        }
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, out)
}

// ---- the host side: spawning with the chosen mercenary, the 3D preview --------------------------------------------

pub use crate::armory_natives::{set_doll_socket, set_equipment_bounds, set_level_actor, tick, Xf};

const SPAWN_QUEUE: &str = "__armory_spawn";

/// a profile the local player controller "sent to the server" (PrepareAndSendCustomizationIfChanged); drained by the
/// Bevy side into SpawnProfile messages
pub fn queue_spawn(vm: &mut Vm, index: usize, profile: V) {
    let Some(s) = singleton(vm) else { return };
    let mut q = vm.prop(s, SPAWN_QUEUE).arr().to_vec();
    q.push(V::st(&[("Index", V::Int(index as i64)), ("Profile", profile)]));
    vm.set(s, SPAWN_QUEUE, V::Array(q));
}

pub fn take_spawns(vm: &mut Vm) -> Vec<(usize, V)> {
    let Some(s) = singleton(vm) else { return vec![] };
    let q = vm.prop(s, SPAWN_QUEUE).arr().to_vec();
    vm.set(s, SPAWN_QUEUE, V::Array(vec![]));
    q.iter().map(|e| (e.field("Index").i().max(0) as usize, e.field("Profile").clone())).collect()
}

/// a VM profile value as JSON in BP_MordhauSingleton's DefaultProfiles shape (Name {"SourceString"}, numbers, bools,
/// strings) - what mh-runtime's loadout.rs resolves
pub fn to_json(v: &V) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        V::None => J::Null,
        V::Bool(b) => J::Bool(*b),
        V::Int(i) => J::from(*i),
        V::Float(f) => J::from(*f),
        V::Text(s) => serde_json::json!({ "SourceString": s, "LocalizedString": s }),
        V::Name(s) | V::Str(s) => J::String(s.clone()),
        V::Asset(a) => serde_json::json!({ "ObjectPath": format!("{}.0", a.package), "ObjectName": a.name }),
        V::Struct(m) => J::Object(m.iter().map(|(k, x)| (k.clone(), to_json(x))).collect()),
        V::Array(a) => J::Array(a.iter().map(to_json).collect()),
        _ => J::Null,
    }
}

/// what the renderer needs to draw the Armory's 3D character preview, read from the game's own actors as the
/// Blueprints left them: BP_MainMenu.CustomizationPlatform (BP_MordhauCustomizationPlatform), its CharacterDoll
/// (placed at BP_CharacterCustomizationSpot's AttachComponent, rotated by UpdateCharacterDollRotation) and its
/// CustomizationObserver (the camera UpdateCamera moves; AMordhauCameraManager::EnterCustomization views through it,
/// FOV 26 = BP_MordhauCustomizationPlatform:UpdateCamera@1701/@4379)
#[derive(bevy::prelude::Resource, Clone, Debug, Default)]
pub struct ArmoryPreview {
    pub active: bool,
    /// the profile on the doll (ApplyProfileTo), DefaultProfiles JSON shape
    pub profile: Option<serde_json::Value>,
    /// doll actor world transform (UE cm, UE rotator degrees: pitch, yaw, roll)
    pub doll_location: [f32; 3],
    pub doll_rotation: [f32; 3],
    /// the observer camera's world transform and field of view
    pub camera_location: [f32; 3],
    pub camera_rotation: [f32; 3],
    pub fov: f32,
    /// the platform's state variables (for checks): CharacterDollRotation, CurrentZoom, Focused Socket
    pub doll_yaw: f32,
    pub zoom: f32,
    pub focused_socket: String,
    /// the equipment-only preview (BP_EquipmentCustomizationSpot) is up instead of the character
    pub equipment: Option<serde_json::Value>,
    /// the equipment doll's class package and world transform (BP_MordhauCustomizationPlatform:SpawnEquipment@1254-1644:
    /// ArmoryTransformOffset composed with BP_EquipmentCustomizationSpot's AttachComponent, then centred on
    /// ComputeAccurateBounds' origin)
    pub equipment_class: String,
    pub equipment_location: [f32; 3],
    pub equipment_rotation: [f32; 3],
}

pub fn preview(vm: &Vm) -> ArmoryPreview {
    let mut p = ArmoryPreview::default();
    let Some(hud) = vm.world.get("hud").copied() else { return p };
    let Some(mm) = vm.prop(hud, "MainMenu").obj().filter(|&m| vm.alive(m)) else { return p };
    let vis = vm.prop(mm, "Visibility").i();
    let Some(pl) = vm.prop(mm, "CustomizationPlatform").obj().filter(|&x| vm.alive(x)) else { return p };
    let doll = vm.prop(pl, "CharacterDoll").obj().filter(|&x| vm.alive(x));
    let eq = vm.prop(pl, "EquipmentDoll").obj().filter(|&x| vm.alive(x));
    // the observer view is on between AMordhauCameraManager::EnterCustomization and LeaveCustomization
    // (armory_natives); leaving the Armory hands the view back to the menu pawn even while the dolls still exist
    let in_cust = vm.world.get("map").map(|&m| vm.prop(m, "__in_customization").truthy()).unwrap_or(false);
    p.active = in_cust && vis != 1 && vis != 2 && (doll.is_some() || eq.is_some());
    let arr = |v: mh_character::ue::FVector| [v.x, v.y, v.z];
    if let Some(d) = doll {
        let w = crate::armory_natives::world_xf(vm, d);
        p.doll_location = arr(w.t);
        let r = w.rotator();
        p.doll_rotation = [r.0, r.1, r.2];
        // a doll no profile has been applied to (ApplyProfileTo) has no character mesh: the character's parts come from
        // its profile (AMordhauCharacter::AssignProfile), so the Armory menu (BP_CustomizationTab
        // SpawnCharacterDollIfNone@135 before any UpdateCharacterDoll) shows an empty room (P26 f_353.5; the empty-mesh
        // rule itself UNCONFIRMED in the exe)
        p.profile = match (vm.prop(d, "Profile"), vm.prop(pl, "QueuedProfileUpdate")) {
            (V::Struct(m), _) if !m.is_empty() => Some(to_json(&V::Struct(m))),
            (_, V::Struct(m)) if !m.is_empty() => Some(to_json(&V::Struct(m))),
            _ => None,
        };
    }
    if let Some(e) = eq {
        p.equipment = Some(to_json(&vm.prop(e, "AssignedCustomization")));
        p.equipment_class = vm.o(e).class.package.clone();
        let w = crate::armory_natives::world_xf(vm, e);
        p.equipment_location = arr(w.t);
        let r = w.rotator();
        p.equipment_rotation = [r.0, r.1, r.2];
    }
    if let Some(o) = vm.prop(pl, "CustomizationObserver").obj().filter(|&x| vm.alive(x)) {
        // the view is the observer's Camera component (its SCS node under DefaultSceneRoot), else the actor
        let cam = vm.prop(o, "Camera").obj().filter(|&c| vm.alive(c)).unwrap_or(o);
        let w = crate::armory_natives::world_xf(vm, cam);
        p.camera_location = arr(w.t);
        let r = w.rotator();
        p.camera_rotation = [r.0, r.1, r.2];
        p.fov = vm.prop(o, "Camera").obj().map(|c| vm.prop(c, "FieldOfView").f() as f32).unwrap_or(0.0);
    }
    p.doll_yaw = vm.prop(pl, "CharacterDollRotation").f() as f32;
    p.zoom = vm.prop(pl, "CurrentZoom").f() as f32;
    p.focused_socket = vm.prop(pl, "Focused Socket").s();
    p
}

/// a loadout the player chose in the Armory (in a match: Escape -> Mercenaries -> pick -> close): AMordhauPlayerController
/// PrepareAndSendCustomizationIfChanged's ServerRequestSetDefaultProfile / ServerRequestSet*Customization. `index` is
/// the picker's id (DefaultProfiles first, then CharacterProfiles); `profile` is the (validated) FCharacterProfile in
/// DefaultProfiles JSON shape.
#[derive(bevy::prelude::Message, Clone, Debug, PartialEq)]
pub struct SpawnProfile {
    pub index: usize,
    pub name: String,
    pub profile: serde_json::Value,
}

/// registers SpawnProfile / ArmoryPreview and fills them from the UI runtime each frame (after the UI update)
pub struct ArmoryPlugin;

impl bevy::prelude::Plugin for ArmoryPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        use bevy::prelude::*;
        app.add_message::<SpawnProfile>().init_resource::<ArmoryPreview>().add_systems(PostUpdate, sync);
    }
}

fn sync(rt: Option<bevy::prelude::NonSendMut<crate::real::Rt>>, time: bevy::prelude::Res<bevy::prelude::Time>, mut prev: bevy::prelude::ResMut<ArmoryPreview>, mut out: bevy::prelude::MessageWriter<SpawnProfile>) {
    let Some(mut rt) = rt else {
        prev.active = false;
        return;
    };
    if let Some(so) = singleton(&rt.ui.vm) {
        load_config(&mut rt.ui.vm, so);
    }
    tick(&mut rt.ui.vm, time.delta_secs_f64().min(0.1));
    for (index, p) in take_spawns(&mut rt.ui.vm) {
        let name = p.field("Name").s();
        out.write(SpawnProfile { index, name, profile: to_json(&p) });
    }
    *prev = preview(&rt.ui.vm);
}

//! loadout.rs - a fighter's look from the game's data: a BP_MordhauSingleton DefaultProfiles loadout resolved to the
//! skeletal-mesh parts the character is built from. Port of godot/game/character/{loadout.gd from_profile, armor.gd
//! construction, character_builder.gd colors_of} and godot/components/ue/records/ue_wearable.gd (class_for, face_def,
//! hair_mesh, color, profile_of); their headers carry the exe citations (AMordhauCharacter::BuildCharacter
//! rva=0x15319e0, UHumanMeshComponent::SetupWearableConstruction_Internal rva=0x14d5fd0, ...).
//!
//! Data: the Singleton's class defaults, the face / hair classes and colour items through mh-level's Blueprint-chain
//! defaults (mh_pak Reader, the paks); wearable classes from the spec matrix (ENT_WEAR_*: mesh, aux mesh, flags,
//! children, colour tables, patterns). The materials are rust-assets' (Resolver::wearable_from_class for wearable
//! parts, Resolver::build for body parts), on the ue_tint shader.

use crate::specdata::SpecData;
use serde_json::Value;

pub const SINGLETON: &str = "Mordhau/Content/Mordhau/Blueprints/BP_MordhauSingleton";
/// EWearableSlot (extract/native/types/EWearableSlot.h): HEAD, COIF, UPPER_CHEST, LOWER_CHEST, SHOULDERS, ARMS, HANDS,
/// LEGS, FEET
pub const SLOTS: usize = 9;
const HEAD: usize = 0;
const COIF: usize = 1;
const UPPER_CHEST: usize = 2;
const LOWER_CHEST: usize = 3;
const ARMS: usize = 5;
const HANDS: usize = 6;
const LEGS: usize = 7;
const FEET: usize = 8;

/// One skeletal mesh of the built character
#[derive(Clone, Debug, serde::Serialize)]
pub struct Part {
    pub mesh: String,
    /// the wearable class that brought it (None = a body part of the face class)
    pub wearable: Option<String>,
    pub slot: Option<usize>,
    pub pattern: usize,
    /// resolved ColorA/B/C (linear), None = the master defaults
    pub colors: Option<[[f32; 3]; 3]>,
    /// which mesh the part belongs to: the 3P UnifiedMesh, the 1P FPMesh, or both (fidelity-audit r2; see `View`)
    pub view: View,
}

/// UHumanMeshComponent::UpdateMeshVisibility rva=0x14ddc50 swaps the character's mesh to FPMesh in first person;
/// CreateFPMeshIfNone rva=0x14b8d30 builds it with CreateMergedMesh(bAddMasterMeshes 0, bIs1PMesh 1, bAddHead 0,
/// bAddTorso 0) (decomp UHumanMeshComponent.cpp 7164-7167), whose part list is SetupWearableConstruction_Internal
/// rva=0x14d5fd0: wearables with bHideIn1P (+0xef) skipped in 1P (decomp 2124); Mesh1POverride (+0x138) /
/// AuxiliaryMesh1POverride (+0x188) replace Mesh (+0x110) / AuxiliaryMesh (+0x160) when set (decomp 2875-2905); face
/// LeftArm1P / RightArm1P / LeftHand1P / RightHand1P replace the arm / hand parts (decomp 2426-2470, UCharacterFace
/// +0x1a8...); ForeArm / FullArm auxiliary 1P variants (decomp 2917-2930); no head (face Mesh, Eyes, hair, beard:
/// bAddHead 0; UNCONFIRMED that hair / beard / eyes ride on bAddHead) and no torso (bAddTorso 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum View {
    Both,
    Only3P,
    Only1P,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Loadout {
    pub profile: String,
    pub classes: Vec<String>,
    pub face: String,
    pub parts: Vec<Part>,
    pub notes: Vec<String>,
}

/// "/Game/X/Y.Y_C" | {"AssetPathName"} | {"ObjectPath": "pkg.N"} -> "Mordhau/Content/X/Y" (ue_wearable.gd pkg_path)
pub fn pkg_path(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.as_str(),
        Value::Object(o) => o.get("AssetPathName").or_else(|| o.get("ObjectPath")).and_then(|x| x.as_str()).unwrap_or(""),
        _ => "",
    };
    if s.is_empty() || s == "None" {
        return String::new();
    }
    if let Some(r) = s.strip_prefix("/Game/") {
        return format!("Mordhau/Content/{}", r.split('.').next().unwrap_or(""));
    }
    if let Some(r) = s.strip_prefix('/') {
        return r.split('.').next().unwrap_or("").to_string();
    }
    crate::ue::strip(s).to_string()
}

/// Make every supported selectable Equipment class available to a later mercenary spawn. Native GetEquipment
/// 0x15d65e0 / SpawnEquipment 0x15feb70 resolves the selected registry entry; the runtime loads records once.
/// Preserve the initial weapon order (and MhSim's empty-list fallback). Return nonempty unsupported classes for
/// dump_state, rather than reporting their fallback as successful equipment support.
pub fn extend_equipment_preload(pk: &mh_level::Pkgs, spec: &SpecData, weapons: &mut Vec<String>) -> Vec<String> {
    if weapons.is_empty() {
        weapons.push(crate::sim_mh::DEFAULT_WEAPON.into());
    }
    let mut unsupported = Vec::new();
    let defaults = pk.defaults(SINGLETON);
    for item in defaults.get("Equipment").and_then(Value::as_array).into_iter().flatten() {
        let path = pkg_path(item);
        if path.is_empty() {
            continue;
        }
        if spec.m.entities.contains_key(&SpecData::ent_of("ENT_WPN_", &path)) {
            if !weapons.contains(&path) {
                weapons.push(path);
            }
        } else if !unsupported.contains(&path) {
            unsupported.push(path);
        }
    }
    unsupported
}

fn at(arr: Option<&Value>, i: i64) -> String {
    arr.and_then(|a| a.as_array()).and_then(|a| a.get(i.max(0) as usize)).map(pkg_path).unwrap_or_default()
}

fn wear_ent(class: &str) -> String {
    SpecData::ent_of("ENT_WEAR_", class)
}

/// UMordhauWearable children lists of a class (spec FLD_WEAR_CHILDREN: {"ArmsWearables": [...], ...})
fn child(spec: &SpecData, class: &str, list: &str, i: i64) -> String {
    spec.m
        .entities
        .get(&wear_ent(class))
        .and_then(|e| e.values.get("FLD_WEAR_CHILDREN"))
        .and_then(|c| c.get(list))
        .and_then(|a| a.as_array())
        .and_then(|a| a.get(i.max(0) as usize))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// ue_wearable.gd class_for: the wearable class of `slot` from the slot ids (parent slots pick the child lists)
pub fn class_for(sing: &serde_json::Map<String, Value>, spec: &SpecData, ids: &[i64], slot: usize) -> String {
    let parent = match slot {
        COIF => HEAD,
        LOWER_CHEST..=ARMS => UPPER_CHEST,
        HANDS => ARMS,
        FEET => LEGS,
        s => s,
    };
    let (own, pid) = (ids[slot], ids[parent]);
    match parent {
        UPPER_CHEST => {
            let uc = at(sing.get("UpperChestWearables"), pid);
            if uc.is_empty() {
                return String::new();
            }
            match slot {
                UPPER_CHEST => at(sing.get("UpperChestWearables"), own),
                4 => child(spec, &uc, "ShouldersWearables", own),
                ARMS => child(spec, &uc, "ArmsWearables", own),
                _ => child(spec, &uc, "LowerChestWearables", own),
            }
        }
        ARMS => {
            let uc = at(sing.get("UpperChestWearables"), ids[UPPER_CHEST]);
            let arms = child(spec, &uc, "ArmsWearables", pid);
            if arms.is_empty() { String::new() } else { child(spec, &arms, "HandsWearables", own) }
        }
        HEAD => {
            let hd = at(sing.get("HeadWearables"), pid);
            if hd.is_empty() {
                return String::new();
            }
            if slot == HEAD { at(sing.get("HeadWearables"), own) } else { child(spec, &hd, "CoifWearables", own) }
        }
        LEGS => {
            let lg = at(sing.get("LegsWearables"), pid);
            if lg.is_empty() {
                return String::new();
            }
            if slot == LEGS { at(sing.get("LegsWearables"), own) } else { child(spec, &lg, "FeetWearables", own) }
        }
        _ => String::new(),
    }
}

/// Profile keys: `k` < BOT_KEY = BP_MordhauSingleton DefaultProfiles[k] (the player's loadouts), `BOT_KEY + i` =
/// BotCharacterProfiles[i], the loadout AMordhauAIController::BeginPlay rva=0x14f0ba0 gives a roster bot (its
/// UseBotLoadoutProfileID / random pick: mh-mode bot_profiles.rs)
pub const BOT_KEY: usize = 1000;

/// Profile keys >= CUSTOM_KEY: profiles registered at run time (register_profile: the Mercenaries screen's custom
/// mercenaries)
pub const CUSTOM_KEY: usize = 2000;

/// process-wide (not thread-local: the part-set builder runs on Bevy's task-pool threads, main.rs registers from the
/// main thread; a thread_local here left `--profile @file` spawning the Knight's parts, camera1p gauntlet 2026-10-07)
static CUSTOM: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<usize, Value>>> = std::sync::OnceLock::new();

fn custom() -> &'static std::sync::Mutex<std::collections::HashMap<usize, Value>> {
    CUSTOM.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// store a profile JSON under a key (>= CUSTOM_KEY) so resolve / profile_gear / the part-set builder find it
pub fn register_profile(key: usize, prof: Value) {
    custom().lock().unwrap().insert(key, prof);
}

/// the profile JSON behind a key: a registered one, else the singleton's
pub fn profile_value(pk: &mh_level::Pkgs, key: usize) -> Option<Value> {
    if key >= CUSTOM_KEY {
        return custom().lock().unwrap().get(&key).cloned();
    }
    profile_entry(&pk.defaults(SINGLETON), key).cloned()
}

/// the FMordhauCharacterProfile behind a profile key
pub fn profile_entry(sing: &serde_json::Map<String, serde_json::Value>, key: usize) -> Option<&serde_json::Value> {
    let (arr, i) = if key >= BOT_KEY { ("BotCharacterProfiles", key - BOT_KEY) } else { ("DefaultProfiles", key) };
    sing.get(arr).and_then(|a| a.as_array()).and_then(|a| a.get(i))
}

/// Resolve the profile `index` (a profile key: DefaultProfiles[i], 0 = "Knight", or BOT_KEY + BotCharacterProfiles
/// index) into parts (armor.gd construction order).
pub fn resolve(pk: &mh_level::Pkgs, spec: &SpecData, index: usize) -> Result<Loadout, String> {
    let prof = profile_value(pk, index).ok_or(format!("Singleton has no profile {index}"))?;
    resolve_value(pk, spec, &prof)
}

/// Resolve an FCharacterProfile given as JSON (DefaultProfiles' shape: a custom mercenary from the Mercenaries screen,
/// rust-armory's mh_ui::armory::SpawnProfile)
pub fn resolve_value(pk: &mh_level::Pkgs, spec: &SpecData, prof: &Value) -> Result<Loadout, String> {
    let sing = pk.defaults(SINGLETON);
    let mut out = Loadout { profile: prof.pointer("/Name/SourceString").or_else(|| prof.pointer("/Name/CultureInvariantString")).and_then(|s| s.as_str()).unwrap_or("").into(), ..Default::default() };
    let wl = prof.pointer("/GearCustomization/Wearables").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let ids: Vec<i64> = (0..SLOTS).map(|i| wl.get(i).and_then(|w| w.get("Id")).and_then(|v| v.as_i64()).unwrap_or(0)).collect();
    let patterns: Vec<usize> = (0..SLOTS).map(|i| wl.get(i).and_then(|w| w.get("Pattern")).and_then(|v| v.as_u64()).unwrap_or(0) as usize).collect();
    let cols: Vec<Vec<i64>> = (0..SLOTS)
        .map(|i| wl.get(i).and_then(|w| w.get("Colors")).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default())
        .collect();
    out.classes = (0..SLOTS).map(|s| class_for(&sing, spec, &ids, s)).collect();
    let ap = prof.get("AppearanceCustomization").cloned().unwrap_or(Value::Null);
    let female = ap.get("bIsFemale").and_then(|b| b.as_bool()).unwrap_or(false);
    let face_i = ap.get("Face").and_then(|v| v.as_i64()).unwrap_or(0);
    out.face = at(sing.get(if female { "FemaleFaces" } else { "MaleFaces" }), face_i);
    let face = pk.defaults(&out.face);
    let soft = |k: &str| face.get(k).map(pkg_path).unwrap_or_default();
    let hair_cls = at(face.get("Hair"), ap.get("Hair").and_then(|v| v.as_i64()).unwrap_or(-1));
    let beard_cls = at(face.get("FacialHair"), ap.get("FacialHair").and_then(|v| v.as_i64()).unwrap_or(-1));
    let mesh_of = |c: &str| if c.is_empty() { String::new() } else { pk.defaults(c).get("Mesh").map(pkg_path).unwrap_or_default() };
    // WearableData.Flags merged over the loadout (FLD_WEAR_FLAGS_*)
    let classes = out.classes.clone();
    let flag = |f: &str| classes.iter().any(|c| !c.is_empty() && spec.m.entities.get(&wear_ent(c)).and_then(|e| e.values.get(f)).and_then(|v| v.as_bool()).unwrap_or(false));
    let add = |out: &mut Loadout, mesh: String, w: Option<(usize, String)>, view: View| {
        if mesh.is_empty() {
            return;
        }
        let (slot, wearable) = match w {
            Some((s, c)) => (Some(s), Some(c)),
            None => (None, None),
        };
        let pattern = slot.map(|s| patterns[s]).unwrap_or(0);
        let colors = slot.and_then(|s| colors_of(pk, spec, &sing, classes[s].as_str(), &cols[s]));
        out.parts.push(Part { mesh, wearable, slot, pattern, colors, view });
    };
    if !flag("FLD_WEAR_FLAGS_HIDE_CHEST") {
        add(&mut out, soft("Torso"), None, View::Only3P);
    }
    add(&mut out, soft("Mesh"), None, View::Only3P);
    if !flag("FLD_WEAR_FLAGS_HIDE_HAIR") {
        add(&mut out, mesh_of(&hair_cls), None, View::Only3P);
    }
    if !flag("FLD_WEAR_FLAGS_HIDE_BEARD") {
        add(&mut out, mesh_of(&beard_cls), None, View::Only3P);
    }
    add(&mut out, soft("Eyes"), None, View::Only3P);
    // a part with a 1P override: the 3P mesh in 3P, the override in 1P; else the same mesh in both
    let pair = |out: &mut Loadout, m3: String, m1: String, w: Option<(usize, String)>| {
        if m1.is_empty() {
            add(out, m3, w, View::Both);
        } else {
            add(out, m3, w.clone(), View::Only3P);
            add(out, m1, w, View::Only1P);
        }
    };
    for (f, k) in [
        ("FLD_WEAR_FLAGS_HIDE_LEFT_ARM", "LeftArm"),
        ("FLD_WEAR_FLAGS_HIDE_RIGHT_ARM", "RightArm"),
        ("FLD_WEAR_FLAGS_HIDE_LEFT_HAND", "LeftHand"),
        ("FLD_WEAR_FLAGS_HIDE_RIGHT_HAND", "RightHand"),
        ("FLD_WEAR_FLAGS_HIDE_LEFT_LEG", "LeftLeg"),
        ("FLD_WEAR_FLAGS_HIDE_RIGHT_LEG", "RightLeg"),
        ("FLD_WEAR_FLAGS_HIDE_LEFT_FOOT", "LeftFoot"),
        ("FLD_WEAR_FLAGS_HIDE_RIGHT_FOOT", "RightFoot"),
    ] {
        if !flag(f) {
            pair(&mut out, soft(k), soft(&format!("{k}1P")), None);
        }
    }
    for s in 0..SLOTS {
        let c = out.classes[s].clone();
        if c.is_empty() {
            continue;
        }
        let Some(e) = spec.m.entities.get(&wear_ent(&c)) else {
            out.notes.push(format!("slot {s}: {c} not in the spec matrix"));
            continue;
        };
        let g = |k: &str| e.values.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let hide1p = e.values.get("FLD_WEAR_B_HIDE_IN_1P").and_then(|v| v.as_bool()).unwrap_or(false);
        if hide1p {
            add(&mut out, g("FLD_WEAR_MESH"), Some((s, c.clone())), View::Only3P);
            add(&mut out, g("FLD_WEAR_AUX_MESH"), Some((s, c.clone())), View::Only3P);
        } else {
            pair(&mut out, g("FLD_WEAR_MESH"), g("FLD_WEAR_MESH_1P"), Some((s, c.clone())));
            pair(&mut out, g("FLD_WEAR_AUX_MESH"), g("FLD_WEAR_AUX_MESH_1P"), Some((s, c.clone())));
        }
    }
    for (f, k) in [
        ("FLD_WEAR_FLAGS_REQUIRES_FOREARM_AUX", "ForeArmAuxiliaryMesh"),
        ("FLD_WEAR_FLAGS_REQUIRES_FULL_ARM_AUX", "FullArmAuxiliaryMesh"),
        ("FLD_WEAR_FLAGS_REQUIRES_UPPER_CHEST_AUX", "UpperChestAuxiliaryMesh"),
        ("FLD_WEAR_FLAGS_REQUIRES_ANKLE_AUX", "AnkleAuxiliaryMesh"),
    ] {
        if flag(f) {
            pair(&mut out, soft(k), soft(&format!("{k}1P")), None);
        }
    }
    Ok(out)
}

/// character_builder.gd colors_of + ue_wearable.gd color: ColorTables[w.ColorTables[i]].Entries[Colors[i]] -> the
/// colour item class's Color; UseColorsFromSlot (10 = Invalid) is not followed (UNCONFIRMED gap). Missing -> master
/// defaults ColorA/B/C (1,0,0) / 0 / 1 (SHADERS.md 7) for that slot.
fn colors_of(pk: &mh_level::Pkgs, spec: &SpecData, sing: &serde_json::Map<String, Value>, class: &str, idx: &[i64]) -> Option<[[f32; 3]; 3]> {
    let tables = spec.m.entities.get(&wear_ent(class))?.values.get("FLD_WEAR_COLOR_TABLES")?.as_array()?.clone();
    let mut out = [[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]];
    let ct = sing.get("ColorTables")?.as_array()?;
    for (i, t) in tables.iter().enumerate().take(3) {
        let t = t.as_i64()? as usize;
        let item = at(ct.get(t)?.get("Entries"), *idx.get(i).unwrap_or(&0));
        if item.is_empty() {
            continue;
        }
        let c = pk.defaults(&item).get("Color").cloned()?;
        let g = |k: &str| c.get(k).and_then(|x| x.as_f64()).unwrap_or(1.0) as f32;
        out[i] = [g("R"), g("G"), g("B")];
    }
    Some(out)
}

/// A DefaultProfiles entry's gear: the first equipment entry (the main weapon) and its FEquipmentCustomization skin /
/// part choice. Equipment Id -> BP_MordhauSingleton Equipment[Id] (ue_wearable.gd equipment_path); Parts[k] indexes
/// Skins[Skin].PartTypes[k].Parts, 0-based (FEquipmentCustomization::Validate rva per PDB: an index >= the part type's
/// Parts count is reset to 0, decomp FEquipmentCustomization.cpp 131-153).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ProfileGear {
    pub index: usize,
    pub name: String,
    pub weapon: String,
    pub skin: i64,
    pub parts: Vec<i64>,
    /// the voice pack Blueprint: AppearanceCustomization.Voice indexes BP_MordhauSingleton MaleVoices (FemaleVoices when
    /// bIsFemale) (index semantics as the Equipment / Wearables ids; UNCONFIRMED: the native lookup not read)
    pub voice: String,
    /// the wearable classes by EWearableSlot (loadout::resolve's), for the movement's armor factors
    pub wearables: Vec<String>,
    /// the equipment the spawn puts in the left hand ("" none) and its skin / parts (fidelity-audit r5: `hands`)
    pub left: String,
    pub left_skin: i64,
    pub left_parts: Vec<i64>,
    /// FEquipmentCustomization Colors[3] / Pattern of the weapon and of the left-hand item, and the profile's
    /// AppearanceCustomization Emblem / EmblemColors (AMordhauEquipment::UpdateMaterial's inputs,
    /// mh_assets::equipment_paint; rust-assets r11)
    pub colors: [i64; 3],
    pub pattern: i64,
    pub left_colors: [i64; 3],
    pub left_pattern: i64,
    pub emblem: i64,
    pub emblem_colors: [i64; 2],
}

fn colors3(e: &serde_json::Value) -> [i64; 3] {
    let a: Vec<i64> = e.get("Colors").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
    [a.first().copied().unwrap_or(0), a.get(1).copied().unwrap_or(0), a.get(2).copied().unwrap_or(0)]
}

/// UMordhauSingleton::ApplyProfileTo rva=0x15c4910 picks up every profile equipment in order
/// (AMordhauCharacter::PickUp -> UEquipmentSystemComponent::PickUpToSlot rva=0x14d0490, decomp
/// UEquipmentSystemComponent.cpp ~3720-3790): a one-handed item goes into its hand (bIsRightHanded) when that hand is
/// free; a two-handed item only when the right hand is free and the left is free or holds a one-handed shield; quickthrow
/// items and items with no free hand are holstered. Returns the profile indices equipped in (right, left); the
/// equipment fields from the class defaults over the AMordhauEquipment ctor (bIsRightHanded true, bIsTwoHanded false,
/// AMordhauEquipment.cpp 1653-1655). UNCONFIRMED: CheckCanEquip is taken as true and the alternate-mode switch of a
/// two-hander next to a shield is not modelled.
pub fn hands(items: &[(bool, bool, bool, bool)]) -> (Option<usize>, Option<usize>) {
    // (unit test: loadout::tests::protector_hands)
    // items: (bIsRightHanded, bIsTwoHanded, bQuickthrowOnly, IsA AMordhauShield)
    let (mut right, mut left): (Option<usize>, Option<usize>) = (None, None);
    for (i, &(rh, th, quick, _shield)) in items.iter().enumerate() {
        if quick {
            continue;
        }
        let mut right_free = right.is_none();
        let mut left_free = left.is_none();
        let left_shield_1h = left.is_some_and(|l| items[l].3 && !items[l].1);
        let any_two = right.is_some_and(|r| items[r].1) || left.is_some_and(|l| items[l].1);
        if any_two {
            right_free = false;
            left_free = false;
        }
        if !th {
            if rh && right_free {
                right = Some(i);
            } else if !rh && left_free {
                left = Some(i);
            }
        } else if right_free && (left_free || left_shield_1h) {
            right = Some(i);
        }
    }
    (right, left)
}

pub fn profile_count(pk: &mh_level::Pkgs) -> usize {
    pk.defaults(SINGLETON).get("DefaultProfiles").and_then(|a| a.as_array()).map(|a| a.len()).unwrap_or(0)
}

pub fn profile_gear(pk: &mh_level::Pkgs, index: usize) -> Result<ProfileGear, String> {
    let prof = profile_value(pk, index).ok_or(format!("no profile {index}"))?;
    profile_gear_value(pk, index, &prof)
}

/// profile_gear of a profile given as JSON (`index` = the key it is stored under)
pub fn profile_gear_value(pk: &mh_level::Pkgs, index: usize, prof: &Value) -> Result<ProfileGear, String> {
    let sing = pk.defaults(SINGLETON);
    let eq = prof.pointer("/GearCustomization/Equipment/0").ok_or("profile has no equipment")?;
    let id = eq.get("Id").and_then(|v| v.as_i64()).unwrap_or(-1);
    let weapon = at(sing.get("Equipment"), id);
    if weapon.is_empty() {
        return Err(format!("Equipment[{id}] empty"));
    }
    Ok(ProfileGear {
        index,
        name: prof.pointer("/Name/SourceString").or_else(|| prof.pointer("/Name/CultureInvariantString")).and_then(|s| s.as_str()).unwrap_or("").into(),
        weapon,
        skin: eq.get("Skin").and_then(|v| v.as_i64()).unwrap_or(0),
        parts: eq.get("Parts").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default(),
        voice: {
            let ap = prof.get("AppearanceCustomization");
            let female = ap.and_then(|a| a.get("bIsFemale")).and_then(|b| b.as_bool()).unwrap_or(false);
            let i = ap.and_then(|a| a.get("Voice")).and_then(|v| v.as_i64()).unwrap_or(0);
            at(sing.get(if female { "FemaleVoices" } else { "MaleVoices" }), i)
        },
        wearables: Vec::new(),
        left: String::new(),
        left_skin: 0,
        left_parts: Vec::new(),
        colors: colors3(eq),
        pattern: eq.get("Pattern").and_then(|v| v.as_i64()).unwrap_or(0),
        left_colors: [0; 3],
        left_pattern: 0,
        emblem: prof.pointer("/AppearanceCustomization/Emblem").and_then(|v| v.as_i64()).unwrap_or(0),
        emblem_colors: {
            let a: Vec<i64> = prof.pointer("/AppearanceCustomization/EmblemColors").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
            [a.first().copied().unwrap_or(0), a.get(1).copied().unwrap_or(0)]
        },
    })
    .map(|mut g| {
        let all: Vec<serde_json::Value> = prof.pointer("/GearCustomization/Equipment").and_then(|a| a.as_array()).cloned().unwrap_or_default();
        let classes: Vec<String> = all.iter().map(|e| at(sing.get("Equipment"), e.get("Id").and_then(|v| v.as_i64()).unwrap_or(-1))).collect();
        let items: Vec<(bool, bool, bool, bool)> = classes
            .iter()
            .map(|c| {
                if c.is_empty() {
                    return (true, false, true, false);
                }
                let d = pk.defaults(c);
                let b = |k: &str, dflt: bool| d.get(k).and_then(|v| v.as_bool()).unwrap_or(dflt);
                // IsA AMordhauShield: the native parent of the Blueprint chain (its root's SuperStruct) is MordhauShield
                let shield = pk.rd.chain(c).last().is_some_and(|root| pk.rd.super_of(root)["ObjectName"].as_str().is_some_and(|n| n.contains("'MordhauShield'")));
                // native default bIsRightHanded: AMordhauEquipment ctor true, AMordhauShield ctor rva=0x15b61a0 false
                // (decomp AMordhauShield.cpp 275)
                (b("bIsRightHanded", !shield), b("bIsTwoHanded", false), b("bQuickthrowOnly", false), shield)
            })
            .collect();
        let (_, l) = hands(&items);
        if let Some(l) = l {
            g.left = classes[l].clone();
            g.left_skin = all[l].get("Skin").and_then(|v| v.as_i64()).unwrap_or(0);
            g.left_parts = all[l].get("Parts").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
            g.left_colors = colors3(&all[l]);
            g.left_pattern = all[l].get("Pattern").and_then(|v| v.as_i64()).unwrap_or(0);
        }
        g
    })
}

/// The DefaultProfiles indices this run uses (`--profile <i|name>` for the player, `--bot-profile <i|name|all>` for
/// bots; `all` = bot k wears DefaultProfiles[(player + 1 + k) mod count], so `--bots 8 --bot-profile all` puts every
/// profile on a fighter) and
/// their gear
#[derive(bevy::prelude::Resource, Clone, Debug, Default, serde::Serialize)]
pub struct Selected {
    pub player: usize,
    pub bot: usize,
    pub bot_cycle: bool,
    pub count: usize,
    pub gear: std::collections::HashMap<usize, ProfileGear>,
    pub names: Vec<String>,
    /// selected/profile or registry equipment the sim has no records for (unsupported; main-hand requests fall
    /// back to the initial weapon, left-hand requests are omitted)
    pub unsupported: Vec<String>,
}

impl Selected {
    /// every profile some fighter of this run may wear (player first)
    pub fn list(&self) -> Vec<usize> {
        let mut v = vec![self.player];
        let bots: Vec<usize> = if self.bot_cycle { (0..self.count).collect() } else { vec![self.bot] };
        for b in bots {
            if !v.contains(&b) {
                v.push(b);
            }
        }
        v
    }
    /// the profile of the k-th bot spawned
    pub fn bot_profile(&self, k: usize) -> usize {
        if self.bot_cycle && self.count > 0 {
            (self.player + 1 + k) % self.count
        } else {
            self.bot
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercenary_registry_preload_loads_every_supported_record_and_geometry() {
        let required = std::env::var("MORDHAU_GOLDEN_REQUIRED").is_ok_and(|v| v == "1");
        let fixtures = mh_pak::Vfs::mount_default().map_err(|e| format!("{e:?}"))
            .and_then(|vfs| SpecData::load().map(|spec| (std::sync::Arc::new(vfs), spec)));
        let (vfs, spec) = match fixtures {
            Ok(f) => f,
            Err(e) => {
                assert!(!required, "required mercenary fixtures unavailable: {e}");
                eprintln!("SKIP mercenary registry preload: {e}");
                return;
            }
        };
        let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
        let registry = pk.defaults(SINGLETON).get("Equipment").and_then(Value::as_array).cloned().expect("original Equipment registry");
        assert_eq!(registry.len(), 71, "installed registry revision changed; re-audit the proof");
        let knight = profile_gear(&pk, 0).expect("Knight");
        let protector = profile_gear(&pk, 1).expect("Protector");
        let veteran = profile_gear(&pk, 2).expect("Veteran");
        let mut weapons = vec![knight.weapon.clone(), protector.weapon.clone()];
        let initial = weapons.clone();
        let unsupported = extend_equipment_preload(&pk, &spec, &mut weapons);
        assert_eq!(&weapons[..initial.len()], initial.as_slice(), "preserve initial weapon priority");
        assert_eq!(weapons.len(), 57);
        assert_eq!(unsupported.len(), 13, "empty registry index0 is not an unsupported weapon");
        assert_eq!(registry.iter().map(pkg_path).filter(String::is_empty).count(), 1);
        for path in [&veteran.weapon, &protector.left, &pkg_path(&registry[19])] {
            assert!(weapons.contains(path), "later mercenary equipment absent: {path}");
        }
        assert!(unsupported.iter().any(|p| p.ends_with("/BP_Longbow")));
        assert!(!unsupported.iter().any(String::is_empty));
        let again = extend_equipment_preload(&pk, &spec, &mut weapons);
        assert_eq!(again, unsupported);
        assert_eq!(weapons.len(), 57, "repeated extension must not duplicate classes");
        let mut empty = Vec::new();
        extend_equipment_preload(&pk, &spec, &mut empty);
        assert_eq!(empty.first().map(String::as_str), Some(crate::sim_mh::DEFAULT_WEAPON), "preserve old empty-input fallback");

        // Use the production record/physics loader, not just the registry/spec-name intersection.
        let paths: Vec<&str> = weapons.iter().map(String::as_str).collect();
        let loaded = mh_sim::load::load(&spec.m, vfs, &paths).unwrap_or_else(|e| panic!("supported registry failed actual load: {e}"));
        for path in &weapons {
            assert!(loaded.spec.weapons.contains_key(path), "missing loaded record {path}");
            assert!(loaded.geo.weapons.contains_key(path), "missing loaded geometry {path}");
        }
        eprintln!("mercenary registry: {} supported records+geometry loaded; unsupported={unsupported:?}", weapons.len());
    }

    /// Every BP_MordhauSingleton DefaultProfiles entry (`--profile <i|name>`) resolves: gear (main weapon + its
    /// FEquipmentCustomization skin / parts), the weapon's chosen part meshes exist in the paks, and the wearables
    /// resolve to classes. Skipped without the install (game data is never in the repo).
    #[test]
    fn all_default_profiles_resolve() {
        let Ok(vfs) = mh_pak::vfs::Vfs::mount_default() else {
            eprintln!("skip: no Mordhau install");
            return;
        };
        let vfs = std::sync::Arc::new(vfs);
        let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
        let n = profile_count(&pk);
        assert_eq!(n, 9, "DefaultProfiles count");
        let spec = SpecData::load().ok();
        let rd = mh_pak::Reader::new(vfs.clone());
        for i in 0..n {
            let g = profile_gear(&pk, i).unwrap_or_else(|e| panic!("profile {i}: {e}"));
            assert!(!g.name.is_empty(), "profile {i} unnamed");
            assert!(g.voice.contains("/Blueprints/Voices/BP_") && vfs.has(&format!("{}.uasset", g.voice)), "profile {i} voice {}", g.voice);
            let meshes = crate::weapon::weapon_part_meshes(&rd, &g.weapon, g.skin, &g.parts);
            assert!(!meshes.is_empty(), "profile {i} {} {}: no weapon part meshes", g.name, g.weapon);
            for m in &meshes {
                assert!(vfs.has(&format!("{m}.uasset")), "profile {i}: missing {m}");
            }
            if let Some(spec) = &spec {
                let lo = resolve(&pk, spec, i).unwrap_or_else(|e| panic!("profile {i} loadout: {e}"));
                assert_eq!(lo.profile, g.name);
                assert!(!lo.parts.is_empty(), "profile {i} {}: no wearable parts", g.name);
            }
            eprintln!("profile {i} {}: {} skin {} parts {:?} -> {} meshes", g.name, g.weapon, g.skin, g.parts, meshes.len());
        }
    }
}

#[cfg(test)]
mod hand_tests {
    use super::hands;
    /// the Protector profile (BP_Mace, BP_KiteShield, BP_Bandage): mace right, shield left (AMordhauShield ctor
    /// bIsRightHanded false), bandage holstered; Knight (BP_Greatsword two-handed) right only; a two-hander after a
    /// shield still goes right (the shield is one-handed)
    #[test]
    fn protector_hands() {
        let mace = (true, false, false, false);
        let shield = (false, false, false, true);
        let bandage = (true, false, false, false);
        assert_eq!(hands(&[mace, shield, bandage]), (Some(0), Some(1)));
        let gs = (true, true, false, false);
        assert_eq!(hands(&[gs]), (Some(0), None));
        assert_eq!(hands(&[shield, gs]), (Some(1), Some(0)));
    }
}

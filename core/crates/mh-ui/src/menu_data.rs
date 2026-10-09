//! UMordhauGameInstance's game-mode / map metadata, which the local-play screen reads (BP_LocalPlay:Construct ->
//! ExecuteUbergraph_BP_LocalPlay@4643 Map_Values(GameInstance.MapMetadata), @4753 Map_Keys(GameInstance.GameModeMetadata)):
//!   `UMordhauGameInstance::RegisterMetadata` 0x15616e0: the asset registry's Blueprint classes derived from
//!     UGameModeMetadata / UMapMetadata, loaded and added to TMap<FString, TSubclassOf<..>> GameModeMetadata / MapMetadata,
//!     keyed by an FText of the class default object (GameModeMetadata +0x38, MapMetadata +0x28; read here as their
//!     `Name` property: UNCONFIRMED field mapping). Registration order = asset registry order (UNCONFIRMED; package
//!     path order here).
//!   `UMordhauGameInstance::GetMapInfo` 0x1540690 (called by `UMordhauUtilityLibrary::GetMapInfo` 0x1622650 with the
//!     world's game instance): a path ending "MainMenu" is no map; else the part after the last '/',
//!     "UEDPC" removed from its start, cut at the last '.', is GameModeMapName; split at the first '_' it is
//!     <prefix>_<short name> (no '_': prefix "FFA", short = the whole name); `FindGameModeMetadata` 0x153c7a0 by prefix,
//!     `FindMapMetadata` 0x153d390 by the map name (none: a new UMapMetadata named after the short name).
//!   `UMordhauGameInstance::ClientTravel` 0x1535ce0: the map URL gets "PlayerCount=<n>" unless it has it; "closed" and
//!     "failed" options are removed; then the client travels (here: an OpenLevel action for the host).

use crate::kismet::ObjRef;
use crate::model::*;
use crate::vm::Vm;
use std::sync::Arc;

/// Blueprint classes deriving from `native` among the packages whose path contains "Metadata" (the asset registry
/// scan of RegisterMetadata), sorted by package path
pub fn derived(vm: &mut Vm, vfs: &mh_pak::Vfs, native: &str) -> Vec<String> {
    let mut pk: Vec<String> = vfs
        .list()
        .filter(|p| p.starts_with("Mordhau/Content/") && p.ends_with(".uasset") && p.contains("Metadata"))
        .map(|p| p.trim_end_matches(".uasset").to_string())
        .collect();
    pk.sort();
    pk.dedup();
    pk.into_iter()
        .filter(|p| match vm.bp_class(p) {
            Some(c) => c.isa(native) && c.native() == native,
            None => false,
        })
        .collect()
}

pub fn class_ref(vm: &mut Vm, pkg: &str) -> V {
    match vm.bp_class(pkg) {
        Some(c) => V::Asset(Arc::new(ObjRef { package: pkg.to_string(), name: c.name.clone(), class: "BlueprintGeneratedClass".into(), outer: String::new() })),
        None => V::None,
    }
}

/// RegisterMetadata: fill the game instance's GameModeMetadata / MapMetadata maps
pub fn register(vm: &mut Vm, vfs: &mh_pak::Vfs, gi: Id) {
    for (native, key) in [("GameModeMetadata", "GameModeMetadata"), ("MapMetadata", "MapMetadata")] {
        let mut m = vec![];
        for p in derived(vm, vfs, native) {
            let c = vm.bp_class(&p).unwrap();
            let name = c.cdo.get("Name").map(V::s).unwrap_or_default();
            if name.is_empty() || m.iter().any(|(k, _): &(V, V)| k.s() == name) {
                continue;
            }
            m.push((V::Str(name), class_ref(vm, &p)));
        }
        if native == "MapMetadata" {
            m.insert(0, combat_test_metadata(vm));
        }
        vm.set(gi, key, V::Map(m));
    }
}

/// REWRITE-ONLY, not ported (user request 2026-10-06): the combat test level (mh-runtime level.rs TEST_LEVEL, a flat
/// grid floor) as the first Local Match map. A MapMetadata object like the game's BP_*MapMetadata CDOs: Name, Maps (one
/// "<prefix>_TestLevel" per local-play mode, so it lists under any selected mode; the host loads the test level for
/// any of them) and a generated grid thumbnail ("gen:grid", slate::gen_texture)
fn combat_test_metadata(vm: &mut Vm) -> (V, V) {
    let c = vm.native_class("MapMetadata");
    let o = vm.new_obj(c, "CombatTestMapMetadata");
    vm.set(o, "Name", V::Text("Combat Test".into()));
    let maps = ["FFA", "TDM", "SKM", "TF", "DU", "HRD", "BR", "INV", "FL"]
        .iter()
        .map(|p| V::st(&[("AssetPathName", V::Str(format!("/Game/Mordhau/Maps/TestLevel/{p}_TestLevel.{p}_TestLevel"))), ("SubPathString", V::Str(String::new()))]))
        .collect();
    vm.set(o, "Maps", V::Array(maps));
    vm.set(o, "Thumbnail", V::Asset(Arc::new(ObjRef { package: "gen:grid".into(), name: "CombatTestThumb".into(), class: "Texture2D".into(), outer: String::new() })));
    (V::Str("Combat Test".into()), V::Obj(o))
}

/// the default object of a class value (Blueprint class -> its CDO object)
pub fn cdo(vm: &mut Vm, c: &V) -> Option<Id> {
    match c {
        V::Asset(a) if !a.package.is_empty() && !a.package.starts_with("/Script/") => vm.library(&a.package),
        V::Obj(o) => Some(*o),
        _ => None,
    }
}

/// GetMapInfo -> FMapInfo { GameModeMapName, GameModeMetadata, MapMetadata }
pub fn map_info(vm: &mut Vm, gi: Id, path: &str) -> V {
    let empty = V::st(&[("GameModeMapName", V::Str(String::new())), ("GameModeMetadata", V::None), ("MapMetadata", V::None)]);
    if path.chars().count() < 2 || path.to_lowercase().ends_with("mainmenu") {
        return empty;
    }
    let mut name = path.rsplit_once('/').map(|x| x.1).unwrap_or(path).to_string();
    if name.to_lowercase().starts_with("uedpc") {
        name = name[5..].to_string();
    }
    if let Some(i) = name.rfind('.') {
        name.truncate(i);
    }
    let (prefix, short) = match name.split_once('_') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => ("FFA".to_string(), name.clone()),
    };
    // FindGameModeMetadata: the registered class whose default Prefix equals the prefix (FString ==, case-insensitive)
    let gm = vm.prop(gi, "GameModeMetadata");
    let mut gmo = None;
    if let V::Map(m) = &gm {
        for (_, c) in m {
            if let Some(o) = cdo(vm, c) {
                if vm.prop(o, "Prefix").s().eq_ignore_ascii_case(&prefix) {
                    gmo = Some(o);
                    break;
                }
            }
        }
    }
    let Some(gmo) = gmo else { return empty };
    // FindMapMetadata: the registered map whose Maps soft paths name this map (UNCONFIRMED: compared by asset name)
    let mm = vm.prop(gi, "MapMetadata");
    let mut mmo = None;
    if let V::Map(m) = &mm {
        'outer: for (_, c) in m.clone() {
            if let Some(o) = cdo(vm, &c) {
                for p in vm.prop(o, "Maps").arr() {
                    let s = p.field("AssetPathName").s();
                    let an = s.rsplit_once('.').map(|x| x.1).unwrap_or(&s);
                    if an.eq_ignore_ascii_case(&name) {
                        mmo = Some(o);
                        break 'outer;
                    }
                }
            }
        }
    }
    let mmo = mmo.unwrap_or_else(|| {
        let c = vm.native_class("MapMetadata");
        let o = vm.new_obj(c, "MapMetadata");
        vm.set(o, "Name", V::Text(short.clone()));
        o
    });
    V::st(&[("GameModeMapName", V::Str(name)), ("GameModeMetadata", V::Obj(gmo)), ("MapMetadata", V::Obj(mmo))])
}

/// ClientTravel URL options: "PlayerCount=<n>" added unless present, "closed" / "failed" removed
pub fn travel_options(map: &str, player_count: i64) -> (String, String) {
    let mut parts = map.split('?');
    let base = parts.next().unwrap_or("").to_string();
    let mut opts: Vec<String> = parts.filter(|o| !o.eq_ignore_ascii_case("closed") && !o.eq_ignore_ascii_case("failed")).map(str::to_string).collect();
    if !opts.iter().any(|o| o.to_lowercase().starts_with("playercount")) {
        opts.push(format!("PlayerCount={player_count}"));
    }
    (base, opts.join("?"))
}

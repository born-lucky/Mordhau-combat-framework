//! The Armory / Mercenaries screens run by their own Blueprints (headless VM + Slate layout) against the game's paks:
//! the default mercenaries from BP_MordhauSingleton, the point budget (UMordhauUtilityLibrary::ComputePointsLeft
//! rva=0x16189a0), opening Armory -> Mercenaries the way the player clicks, editing through UCharacterProfileBPWrapper,
//! saving to the rewrite's own config dir, and the in-match pick reaching the spawn (PrepareAndSendCustomizationIfChanged).
//! Skipped without the install (the game data is never in the repo).

use mh_ui::armory;
use mh_ui::host::UiRuntime;
use mh_ui::model::V;
use std::sync::{Arc, Mutex};

static CFG: Mutex<()> = Mutex::new(());
const VP: [f64; 2] = [1920.0, 1080.0];

fn frames(ui: &mut UiRuntime, n: usize) {
    for _ in 0..n {
        ui.update(1.0 / 60.0, VP);
        mh_ui::armory_natives::tick(&mut ui.vm, 1.0 / 60.0);
    }
}

fn click(ui: &mut UiRuntime, name: &str) {
    let w = ui.vm.viewport.iter().rev().find_map(|&(r, _)| ui.find(r, name).filter(|&w| ui.painted(w))).unwrap_or_else(|| panic!("{name} not painted"));
    let c = ui.centre(w).unwrap();
    ui.mouse_move(c);
    frames(ui, 2);
    ui.click(c);
    frames(ui, 10);
}

fn fresh(tag: &str) -> Option<(UiRuntime, std::path::PathBuf)> {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().ok()?);
    let d = std::env::temp_dir().join(format!("mh-ui-armory-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("MH_CONFIG_DIR", &d);
    let mut ui = UiRuntime::new(vfs, "Main Menu");
    let so = ui.vm.world["singleton"];
    armory::load_config(&mut ui.vm, so);
    Some((ui, d))
}

fn active(ui: &UiRuntime, sw: &str) -> String {
    let s = ui.find_any(sw).unwrap();
    let i = ui.vm.prop(s, "ActiveWidgetIndex").i().max(0) as usize;
    ui.vm.o(s).slots.get(i).and_then(|&sl| ui.vm.o(sl).content).map(|c| ui.vm.o(c).name.clone()).unwrap_or_default()
}

/// BP_MordhauSingleton CDO: 9 DefaultProfiles (Knight .. Engineer) and, on a fresh install, the CDO's one "Unnamed"
/// custom profile (UMordhauSingleton::PostLoad keeps a non-empty CharacterProfiles). Every default fits its budget.
#[test]
fn default_mercenaries_and_points() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = fresh("defaults") else { return };
    let so = ui.vm.world["singleton"];
    let defs = ui.vm.prop(so, "DefaultProfiles").arr().to_vec();
    let names: Vec<String> = defs.iter().map(|p| p.field("Name").s()).collect();
    assert_eq!(names, ["Knight", "Protector", "Veteran", "Raider", "Brigand", "Footman", "Huntsman", "Scoundrel", "Engineer"]);
    let custom = ui.vm.prop(so, "CharacterProfiles").arr().to_vec();
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].field("Name").s(), "Unnamed");
    let base = armory::archetype_points(&mut ui.vm).expect("Archetypes[0] CharacterPoints");
    assert!(base > 0);
    for p in &defs {
        let left = armory::points_left(&mut ui.vm, p);
        eprintln!("{}: {left} of {base} points left", p.field("Name").s());
        assert!((0..=base).contains(&left), "{} over budget: {left}", p.field("Name").s());
        // a valid default profile passes ForceValidCharacterProfile unchanged (gear-wise)
        let (ok, v) = armory::force_valid(&mut ui.vm, p);
        assert!(ok);
        assert!(v.field("GearCustomization").same(p.field("GearCustomization")), "{} changed by validation", p.field("Name").s());
    }
}

/// the singleton's config section round-trips through the rewrite's own Game.ini (never Steam's)
#[test]
fn config_round_trip() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, d)) = fresh("config") else { return };
    let so = ui.vm.world["singleton"];
    let mut list = ui.vm.prop(so, "CharacterProfiles").arr().to_vec();
    let mut p = ui.vm.prop(so, "DefaultProfiles").arr()[2].clone();
    if let V::Struct(m) = &mut p {
        m.insert("Name".into(), V::Text("My \"Vet\", copy".into()));
        m.insert("Category".into(), V::Name("Custom".into()));
    }
    list.push(p.clone());
    ui.vm.set(so, "CharacterProfiles", V::Array(list.clone()));
    armory::save_config(&ui.vm, so).unwrap();
    assert!(armory::game_ini().starts_with(&d), "saves under MH_CONFIG_DIR / settings::config_dir()");
    let Some((mut ui2, _)) = fresh_keep(&d) else { return };
    let so2 = ui2.vm.world["singleton"];
    armory::load_config(&mut ui2.vm, so2);
    let back = ui2.vm.prop(so2, "CharacterProfiles").arr().to_vec();
    assert_eq!(back.len(), 2);
    assert_eq!(back[1].field("Name").s(), "My \"Vet\", copy");
    assert!(back[1].field("GearCustomization").same(p.field("GearCustomization")));
    assert!(back[1].field("AppearanceCustomization").same(p.field("AppearanceCustomization")));
}

fn fresh_keep(d: &std::path::Path) -> Option<(UiRuntime, ())> {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().ok()?);
    std::env::set_var("MH_CONFIG_DIR", d);
    Some((UiRuntime::new(vfs, "Main Menu"), ()))
}

/// Armory -> Mercenaries with real clicks: BP_MainMenu spawns BP_MordhauCustomizationPlatform (Select Armory Tab@3978),
/// so BP_ProfileCustomization:SetArmoryState(1) switches WidgetSwitcher_Main to BP_LoadoutPicker, whose list holds the
/// 9 defaults + the custom profile; the platform has a doll with the selected profile applied
#[test]
fn menu_opens_mercenaries() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = fresh("menu") else { return };
    frames(&mut ui, 30);
    click(&mut ui, "ArmoryButton");
    assert_eq!(active(&ui, "WidgetSwitcher_Main"), "BP_ArmoryTypeSelection");
    let mm = ui.main_menu().unwrap();
    assert!(ui.vm.prop(mm, "CustomizationPlatform").obj().is_some_and(|p| ui.vm.alive(p)), "platform spawned");
    click(&mut ui, "BP_PromptButton_Mercenaries");
    assert_eq!(active(&ui, "WidgetSwitcher_Main"), "BP_LoadoutPicker");
    let picker = ui.find_any("BP_LoadoutPicker").unwrap();
    let data = ui.vm.prop(picker, "Loadout Data").arr().len();
    assert_eq!(data, 10, "9 defaults + 1 custom");
}

/// in a match, the pick reaches the spawn: Set Spawn Loadout(SelectedId) then PrepareAndSendCustomizationIfChanged
#[test]
fn match_pick_queues_spawn() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = fresh("pick") else { return };
    let pc = ui.vm.world["pc"];
    ui.vm.set(pc, "SelectedDefaultProfile", V::Int(3));
    mh_ui::natives::call(&mut ui.vm, mh_ui::vm::Ctx::Obj(pc), "MordhauPlayerController", "PrepareAndSendCustomizationIfChanged", &[]);
    let s = armory::take_spawns(&mut ui.vm);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].0, 3);
    assert_eq!(s[0].1.field("Name").s(), "Raider");
    // unchanged -> nothing new is sent
    mh_ui::natives::call(&mut ui.vm, mh_ui::vm::Ctx::Obj(pc), "MordhauPlayerController", "PrepareAndSendCustomizationIfChanged", &[]);
    assert!(armory::take_spawns(&mut ui.vm).is_empty());
    // a custom profile: SelectedCharacterProfile 0, SelectedDefaultProfile -1 -> picker index 9
    ui.vm.set(pc, "SelectedDefaultProfile", V::Int(-1));
    ui.vm.set(pc, "SelectedCharacterProfile", V::Int(0));
    mh_ui::natives::call(&mut ui.vm, mh_ui::vm::Ctx::Obj(pc), "MordhauPlayerController", "PrepareAndSendCustomizationIfChanged", &[]);
    let s = armory::take_spawns(&mut ui.vm);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].0, 9);
}

/// UCharacterProfileBPWrapper::SetEquipmentId (exec rva 0x1689160): a new item id resets the item and carries matching
/// colours over; the points left follow the new item's CharacterPointCost
#[test]
fn wrapper_set_equipment_id() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = fresh("wrap") else { return };
    let so = ui.vm.world["singleton"];
    let knight = ui.vm.prop(so, "DefaultProfiles").arr()[0].clone();
    let c = ui.vm.native_class("CharacterProfileBPWrapper");
    let w = ui.vm.new_obj(c, "Wrapper");
    ui.vm.set(w, "Profile", knight.clone());
    let before = armory::points_left(&mut ui.vm, &knight);
    let old_id = knight.field("GearCustomization").field("Equipment").arr()[0].field("Id").i();
    let new_id = (1..60).find(|&i| i != old_id && armory::equipment_class(&ui.vm, i).is_some()).unwrap();
    mh_ui::natives::call(&mut ui.vm, mh_ui::vm::Ctx::Obj(w), "CharacterProfileBPWrapper", "SetEquipmentId", &[V::Int(0), V::Int(new_id)]);
    let p = ui.vm.prop(w, "Profile");
    let e = &p.field("GearCustomization").field("Equipment").arr()[0];
    assert_eq!(e.field("Id").i(), new_id);
    assert_eq!(e.field("Skin").i(), 0);
    assert_eq!(e.field("Parts").arr().len(), 3);
    let after = armory::points_left(&mut ui.vm, &p);
    eprintln!("Knight weapon {old_id} -> {new_id}: points left {before} -> {after}");
}

fn nth(ui: &UiRuntime, cls: &str, n: usize) -> Option<[f64; 2]> {
    let mut f: Vec<(i64, i64, u32)> = (1..ui.vm.objs.len() as u32)
        .filter(|&w| ui.vm.alive(w) && ui.vm.o(w).class.name.starts_with(cls) && ui.painted(w))
        .filter_map(|w| ui.out.rects.get(&w).map(|r| (r[1].round() as i64, r[0].round() as i64, w)))
        .collect();
    f.sort();
    ui.centre(f.get(n)?.2)
}

/// picking a mercenary with the mouse: BP_LoadoutEntry:OnMouseButtonDown_1@308 -> ListView_Loadouts.BP_SetSelectedItem
/// -> BP_LoadoutPicker:Select New Loadout -> Set Selected Loadout(ID): the picker's SelectedId and the profile being
/// edited follow; then a weapon change through the wrapper + the screen's Save reaches the rewrite's Game.ini
#[test]
fn pick_mercenary_edit_and_save() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, d)) = fresh("pickedit") else { return };
    frames(&mut ui, 30);
    click(&mut ui, "ArmoryButton");
    click(&mut ui, "BP_PromptButton_Mercenaries");
    frames(&mut ui, 30);
    let c = nth(&ui, "BP_LoadoutEntry", 9).expect("10 loadout entries painted");
    ui.mouse_move(c);
    frames(&mut ui, 2);
    ui.click(c);
    frames(&mut ui, 20);
    // the real click reaches Border_287's bound OnMouseButtonDownEvent (BP_LoadoutEntry Bindings -> OnMouseButtonDown_1)
    let picker = ui.find_any("BP_LoadoutPicker").unwrap();
    let lv = ui.find_any("ListView_Loadouts").unwrap();
    eprintln!("click at {c:?}: hovered {:?}; list selected {:?}; hits {:?}", ui.hovered.map(|h| ui.vm.o(h).name.clone()), ui.vm.prop(lv, "__selected"), ui.hits_at(c).iter().map(|&h| ui.vm.o(h).name.clone()).collect::<Vec<_>>());
    assert_eq!(ui.vm.prop(picker, "SelectedId").i(), 9, "the custom (Unnamed) mercenary selected");
    let pc = ui.find_any("BP_MordhauProfileCustomization").unwrap();
    let w = ui.vm.prop(pc, "ProfileWrapper").obj().unwrap();
    assert_eq!(ui.vm.prop(w, "Profile").field("Name").s(), "Unnamed");
    // equip the first available weapon in the primary slot (the equipment screen's EquipmentSelectionClicked calls
    // ProfileWrapper.SetEquipmentId(EquipmentSlotIndex, Id))
    let id = (1..60).find(|&i| mh_ui::armory::equipment_class(&ui.vm, i).is_some()).unwrap();
    mh_ui::natives::call(&mut ui.vm, mh_ui::vm::Ctx::Obj(w), "CharacterProfileBPWrapper", "SetEquipmentId", &[V::Int(0), V::Int(id)]);
    // BP_LoadoutPicker saves an edited custom mercenary with ProfileCustomization.SaveFunction(the custom index)
    // (BP_LoadoutPicker ubergraph @3919; BP_ProfileCustomization:SaveFunction@405 Array_Set CharacterProfiles, SaveToConfig@487)
    ui.vm.call_named(pc, "SaveFunction", vec![V::Int(0)]);
    frames(&mut ui, 5);
    let ini = std::fs::read_to_string(d.join("Game.ini")).expect("Game.ini written under MH_CONFIG_DIR");
    assert!(ini.contains(&format!("(Colors=(0,0,0),Id={id},")), "{ini}");
}

fn click_nth(ui: &mut UiRuntime, cls: &str, n: usize) {
    let c = nth(ui, cls, n).unwrap_or_else(|| panic!("{cls} {n} not painted"));
    ui.mouse_move(c);
    frames(ui, 2);
    ui.click(c);
    frames(ui, 20);
}

fn editing(ui: &UiRuntime) -> V {
    let pc = ui.find_any("BP_MordhauProfileCustomization").unwrap();
    let w = ui.vm.prop(pc, "ProfileWrapper").obj().unwrap();
    ui.vm.prop(w, "Profile")
}

fn open_mercenaries(tag: &str) -> Option<(UiRuntime, std::path::PathBuf)> {
    let (mut ui, d) = fresh(tag)?;
    frames(&mut ui, 30);
    click(&mut ui, "ArmoryButton");
    click(&mut ui, "BP_PromptButton_Mercenaries");
    frames(&mut ui, 30);
    Some((ui, d))
}

/// selecting a mercenary runs BP_LoadoutPicker:Update Loadout Breakdown, gated on the doll's movement component
/// (Character Doll.GetMovementComponent() cast to MordhauMovementComponent @43-154): the top bar shows the profile's
/// category and name, and ApplyProfileTo (rva UMordhauSingleton.cpp 4138-4251) put the profile's items in the doll's
/// Equipment slots
#[test]
fn select_updates_breakdown_and_doll() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = open_mercenaries("breakdown") else { return };
    click_nth(&mut ui, "BP_LoadoutEntry", 2);
    assert_eq!(editing(&ui).field("Name").s(), "Veteran");
    let meta = ui.find_any("BP_LoadoutMetaInfo").unwrap();
    let mut d = vec![];
    ui.vm.descendants(meta, &mut d);
    let texts: Vec<String> = d.iter().filter(|&&w| ui.painted(w)).map(|&w| ui.vm.prop(w, "Text").s()).filter(|s| !s.is_empty()).collect();
    assert!(texts.iter().any(|t| t == "Default") && texts.iter().any(|t| t == "Veteran"), "top bar {texts:?}");
    let mm = ui.main_menu().unwrap();
    let pl = ui.vm.prop(mm, "CustomizationPlatform").obj().unwrap();
    let doll = ui.vm.prop(pl, "CharacterDoll").obj().expect("doll");
    let held = ui.vm.prop(doll, "Equipment").arr().to_vec();
    let id0 = editing(&ui).field("GearCustomization").field("Equipment").arr()[0].field("Id").i();
    let want = armory::equipment_class(&ui.vm, id0).unwrap();
    let e0 = held.first().and_then(V::obj).expect("primary item spawned on the doll");
    assert_eq!(ui.vm.o(e0).class.package, want);
}

/// Equipment -> primary slot -> the grid (BP_EquipmentSlotCustomization:GenerateEquipmentEntries + BP_SelectionMenu
/// Filter and Sort Entries by Function Delegate, whose FilterArrayByFunction verdict is the delegate's bool& out) ->
/// a real click on an item: EquipmentSelectionClicked@1841 ProfileWrapper.SetEquipmentId
#[test]
fn equipment_grid_click_sets_item() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = open_mercenaries("eqgrid") else { return };
    click_nth(&mut ui, "BP_LoadoutEntry", 9);
    assert_eq!(editing(&ui).field("Name").s(), "Unnamed");
    click_nth(&mut ui, "BP_BreakdownEquipmentEntry", 0);
    let grid = ui.find_any("BP_EquipmentSelectionMenu").and_then(|m| ui.vm.prop(m, "Active Layout").obj()).unwrap();
    let painted = (1..ui.vm.objs.len() as u32).filter(|&w| ui.vm.alive(w) && ui.vm.o(w).class.name.starts_with("BP_EquipmentItemEntry") && ui.painted(w)).count();
    assert!(painted > 4, "equipment grid painted {painted} entries (grid {grid})");
    // the grid is sorted by cost (BP_SelectionMenu SortByCost); the first entries can be the empty item: click down the
    // grid until one equips
    let mut id = 0;
    for n in 1..10 {
        click_nth(&mut ui, "BP_EquipmentItemEntry", n);
        id = editing(&ui).field("GearCustomization").field("Equipment").arr()[0].field("Id").i();
        if id != 0 {
            break;
        }
    }
    assert!(id > 0 && armory::equipment_class(&ui.vm, id).is_some(), "primary set to {id}");
}

/// Armor -> legs slot -> TileView_Wearables (EntryWidth 136 x EntryHeight 200 tiles) -> a real click on a tile: the
/// row selection (STableRow::OnMouseButtonDown) -> BP_ArmorCustomization:On Wearable Selection Changed -> Select New
/// Wearable -> Set Wearable in Profile and Update Doll@224 ProfileWrapper.SetWearableId; the slot widgets resolve
/// their class from the EWearableSlot enum name (EWearableSlot::Legs = 7)
#[test]
fn armor_tile_click_sets_wearable() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = open_mercenaries("armor") else { return };
    click_nth(&mut ui, "BP_LoadoutEntry", 9);
    click_nth(&mut ui, "BP_BreakdownArmorEntry", 0);
    let legs = ui.find_any("LegsSlot").unwrap();
    assert_ne!(ui.vm.prop(legs, "WearableName").s(), "Nothing", "the legs slot names the worn class");
    let before = armory::wearable_ids(editing(&ui).field("GearCustomization"))[armory::LEGS];
    // the legs slot widget, then a tile other than the worn one
    let c = ui.centre(legs).unwrap();
    ui.mouse_move(c);
    frames(&mut ui, 2);
    ui.click(c);
    frames(&mut ui, 30);
    let tv = ui.find_any("TileView_Wearables").unwrap();
    let tiles: Vec<u32> = ui.vm.prop(tv, "__entries").arr().iter().filter_map(V::obj).collect();
    assert!(tiles.len() > 2);
    let rects: Vec<[f64; 4]> = tiles.iter().filter_map(|t| ui.out.rects.get(t).copied()).collect();
    assert!(rects.len() > 2 && rects[0] != rects[1], "tiles laid out side by side: {:?}", &rects[..2]);
    let pick = (0..tiles.len()).find(|&i| ui.vm.prop(ui.vm.prop(tv, "ListItems").arr()[i].obj().unwrap(), "WearableData").field("Id").i() != before).unwrap();
    let c = ui.centre(tiles[pick]).unwrap();
    ui.mouse_move(c);
    frames(&mut ui, 2);
    ui.click(c);
    frames(&mut ui, 20);
    let after = armory::wearable_ids(editing(&ui).field("GearCustomization"))[armory::LEGS];
    assert_ne!(before, after, "legs wearable changed by the tile click");
}

/// UMordhauSingleton::MakeEmptyProfile (UMordhauSingleton.cpp 3589-3797): Head / UpperChest / Legs from the singleton's
/// Default* ids, the other slots their class defaults, a voice among MaleVoices when randomized
#[test]
fn make_empty_profile_defaults() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = fresh("empty") else { return };
    let so = ui.vm.world["singleton"];
    let p = mh_ui::armory_natives::make_empty_profile(&mut ui.vm, true);
    let ids = armory::wearable_ids(p.field("GearCustomization"));
    assert_eq!(ids[armory::HEAD], ui.vm.prop(so, "DefaultHead").i());
    assert_eq!(ids[armory::UPPER_CHEST], ui.vm.prop(so, "DefaultUpperChest").i());
    assert_eq!(ids[armory::LEGS], ui.vm.prop(so, "DefaultLegs").i());
    let voices = ui.vm.prop(so, "MaleVoices").arr().len() as i64;
    let v = p.field("AppearanceCustomization").field("Voice").i();
    assert!(voices == 0 || (0..voices).contains(&v));
    assert!((0..=255).contains(&p.field("AppearanceCustomization").field("VoicePitch").i()));
}

/// the mouse wheel scrolls the equipment grid (BP_CustomizationItemGrid ScrollBox_0, 71 items): SScrollBox::OnMouseWheel
/// ScrollBy(-delta x GlobalScrollAmount 32 x WheelScrollMultiplier); the tiles move up by the scrolled amount (the
/// same host path scrolls the armor TileView, STableViewBase::OnMouseWheel, when its tier list overflows)
#[test]
fn wheel_scrolls_equipment_grid() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = open_mercenaries("wheel") else { return };
    click_nth(&mut ui, "BP_LoadoutEntry", 9);
    click_nth(&mut ui, "BP_BreakdownEquipmentEntry", 0);
    let grid = ui.find_any("BP_EquipmentSelectionMenu").and_then(|m| ui.vm.prop(m, "Active Layout").obj()).unwrap();
    let sb = ui.vm.prop(grid, "ScrollBox_0").obj().unwrap();
    let first = |ui: &UiRuntime| {
        let mut ys: Vec<f64> = (1..ui.vm.objs.len() as u32).filter(|&w| ui.vm.alive(w) && ui.vm.o(w).class.name.starts_with("BP_EquipmentItemEntry") && ui.painted(w)).filter_map(|w| ui.out.rects.get(&w).map(|r| r[1])).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ys.first().copied().unwrap_or(0.0)
    };
    let before = first(&ui);
    eprintln!("scroll_max {:?}", ui.vm.prop(sb, "__scroll_max"));
    assert!(ui.vm.prop(sb, "__scroll_max").f() > 0.0, "71 items overflow the grid");
    ui.mouse_move(ui.centre(sb).unwrap());
    ui.mouse_wheel(-3.0);
    frames(&mut ui, 3);
    let after = first(&ui);
    eprintln!("top tile y {before} -> {after} (scroll {:?})", ui.vm.prop(sb, "__scroll"));
    assert!(after < before, "the wheel scrolled the grid up");
}

/// rotate and zoom the Armory doll the way the player does: a right-button drag over the 3D view (BP_CustomizationPreview
/// OnMouseMove@170-382: RightMouseButton held and a horizontal CursorDelta -> OnDrag(dx) -> the screen's OnDrag ->
/// RotateCharacterDoll) and the wheel there (OnMouseWheel@37 -> OnMouseWheelScrolling -> ZoomCharacterDoll)
#[test]
fn drag_rotates_and_wheel_zooms_the_doll() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((mut ui, _d)) = open_mercenaries("rotzoom") else { return };
    click_nth(&mut ui, "BP_LoadoutEntry", 2);
    let mm = ui.main_menu().unwrap();
    let pl = ui.vm.prop(mm, "CustomizationPlatform").obj().unwrap();
    let rot0 = ui.vm.prop(pl, "CharacterDollRotationTarget").f();
    let zoom0 = ui.vm.prop(pl, "CharacterDollZoom").f();
    // over the doll (right half, clear of the list and the buttons)
    let at = [1390.0, 600.0];
    ui.mouse_move(at);
    frames(&mut ui, 2);
    ui.mouse_down("RightMouseButton");
    for i in 1..=10 {
        ui.mouse_move([at[0] + 15.0 * i as f64, at[1]]);
        frames(&mut ui, 1);
    }
    ui.mouse_up("RightMouseButton");
    frames(&mut ui, 5);
    let rot1 = ui.vm.prop(pl, "CharacterDollRotationTarget").f();
    eprintln!("rotation target {rot0} -> {rot1}");
    assert!((rot1 - rot0).abs() > 1.0, "the drag turned the doll");
    // wheel up zooms in (zoom out is already at Character Zoom Max 0)
    ui.mouse_wheel(1.0);
    frames(&mut ui, 5);
    let zoom1 = ui.vm.prop(pl, "CharacterDollZoom").f();
    eprintln!("zoom {zoom0} -> {zoom1}");
    assert!((zoom1 - zoom0).abs() > 1e-3, "the wheel zoomed");
}

/// user report 2026-10-07 14:14 "swapping characters and mercenaries still doesn't work": in a match, the "Show Profile
/// Select" action (DefaultInput.ini Key=B) -> BP_MordhauPlayerController HandleShowProfileSelect -> @12386
/// HUD.Show Profile Customization(1) opens the Mercenaries list; a pick + Play (BP_ProfileCustomization CloseMenu ->
/// BP_MordhauUtilityLibrary:Set Spawn Loadout@721 SelectedDefaultProfile = Id + PrepareAndSendCustomizationIfChanged
/// 0x15eb160 -> ServerRequestSetDefaultProfile) sends exactly that mercenary to the spawn (armory::take_spawns ->
/// mh-runtime SpawnProfile -> the player's next spawn)
#[test]
fn match_profile_select_key_sends_the_picked_mercenary() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(vfs) = mh_pak::Vfs::mount_default().ok().map(Arc::new) else { return };
    let d = std::env::temp_dir().join(format!("mh-ui-armory-matchpick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::env::set_var("MH_CONFIG_DIR", &d);
    let mut ui = UiRuntime::new(vfs, "FFA_Arena");
    let so = ui.vm.world["singleton"];
    armory::load_config(&mut ui.vm, so);
    ui.set_vitals(100, 100, true);
    frames(&mut ui, 30);
    assert!(!ui.main_menu_visible());
    ui.key_down("B");
    ui.key_up("B");
    frames(&mut ui, 60);
    assert!(ui.main_menu_visible(), "B opens the in-match customization");
    assert_eq!(active(&ui, "WidgetSwitcher_Main"), "BP_LoadoutPicker", "on the Mercenaries list");
    let picker = ui.find_any("BP_LoadoutPicker").unwrap();
    // the fourth row (reading order) of the list
    let c = nth(&ui, "BP_LoadoutEntry", 3).expect("a 4th mercenary row painted");
    ui.mouse_move(c);
    frames(&mut ui, 2);
    ui.click(c);
    frames(&mut ui, 30);
    let id = ui.vm.prop(picker, "SelectedId").i();
    let defs = ui.vm.prop(so, "DefaultProfiles").arr().to_vec();
    let want = defs.get(id as usize).map(|p| p.field("Name").s()).unwrap_or_default();
    eprintln!("picked SelectedId {id} ({want})");
    assert!(id > 0, "a mercenary other than the first is selected");
    let _ = armory::take_spawns(&mut ui.vm);
    // BP_LoadoutSelectionMenu's BP_PromptButton_Play -> BP_LoadoutPicker:Play button clicked (ubergraph @12786)
    let play = ui.vm.viewport.iter().rev().find_map(|&(r, _)| ui.find(r, "BP_PromptButton_Play").filter(|&w| ui.painted(w))).expect("Play painted");
    let c = ui.centre(play).unwrap();
    ui.mouse_move(c);
    frames(&mut ui, 2);
    ui.click(c);
    frames(&mut ui, 30);
    let s = armory::take_spawns(&mut ui.vm);
    eprintln!("sent: {:?}", s.iter().map(|(i, p)| (*i, p.field("Name").s())).collect::<Vec<_>>());
    assert_eq!(s.len(), 1, "one profile sent");
    assert_eq!(s[0].0 as i64, id);
    assert_eq!(s[0].1.field("Name").s(), want);
    let pc = ui.vm.world["pc"];
    assert_eq!(ui.vm.prop(pc, "SelectedDefaultProfile").i(), id);
    assert!(!ui.main_menu_visible(), "Play closes the menu");
}

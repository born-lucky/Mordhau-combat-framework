//! The real UMG widgets run by the Blueprint VM (headless: VM + Slate layout, no renderer), against the game's paks:
//! the menu map's front end driven by input the way the game navigates it, and the settings object's ini round trip.

use mh_ui::host::UiRuntime;
use mh_ui::model::V;
use mh_ui::vm::Action;
use std::sync::{Arc, Mutex};

/// MH_CONFIG_DIR is process-wide: tests that read / write settings take this lock
static CFG: Mutex<()> = Mutex::new(());

const VP: [f64; 2] = [1920.0, 1080.0];

fn frames(ui: &mut UiRuntime, n: usize) {
    for _ in 0..n {
        ui.update(1.0 / 60.0, VP);
    }
}

fn click(ui: &mut UiRuntime, name: &str) {
    let w = ui.find_any(name).unwrap_or_else(|| panic!("no widget {name}"));
    let c = ui.centre(w).unwrap_or_else(|| panic!("{name} not painted"));
    ui.click(c);
    frames(ui, 2);
}

fn temp_config(tag: &str, ini: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mh-ui-test-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    std::fs::write(d.join("GameUserSettings.ini"), ini).unwrap();
    std::env::set_var("MH_CONFIG_DIR", &d);
    d
}

/// BP_MordhauHUD:ReceiveBeginPlay creates BP_MainMenu (AddToViewport(100), CreateMainMenu@144); on the "Main Menu" map
/// a PC launch never shows the title screen (UMordhauGameInstance ctor 0x1528830 pairs the controller, so
/// BP_MainMenu:UpdateInitialInteractionOverlay collapses BP_TitleScreen on the first frame); "fight" ->
/// "local match" opens BP_LocalPlay, whose map list comes from the registered metadata; Start Match travels through
/// UMordhauGameInstance::ClientTravel 0x1535ce0
#[test]
fn main_menu_to_local_match() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("menu", "[/Script/Mordhau.MordhauGameUserSettings]\nGore=1\n");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "Main Menu");
    frames(&mut ui, 3);
    let mm = ui.main_menu().expect("BP_MainMenu created by BP_MordhauHUD");
    assert!(ui.vm.viewport.iter().any(|&(w, z)| w == mm && z == 100), "AddToViewport(100)");
    assert!(ui.main_menu_visible());
    let title = ui.find_any("BP_TitleScreen").unwrap();
    assert_eq!(ui.vm.prop(title, "Visibility").i(), 1, "no title screen on PC: straight to Home");
    assert!(!ui.painted(title));
    // the loading blur goes once the (offline) backend reports ready (BP_MainMenu Tick@2058)
    let blur = ui.find_any("LoadingBlur").unwrap();
    assert_eq!(ui.vm.prop(blur, "Visibility").i(), 1);
    // nav button text through its property binding (BP_NavButton Bindings: NavText.Text <- GetText)
    let play = ui.find_any("PlayButton").unwrap();
    let nt = ui.vm.child(play, "NavText").unwrap();
    assert_eq!(ui.vm.prop(nt, "Text").s(), "fight");
    click(&mut ui, "PlayButton");
    click(&mut ui, "LocalMatchButton");
    let lp = ui.find_any("BP_LocalPlay").unwrap();
    let cs = ui.find_any("ContentSwitcher").unwrap();
    let idx = ui.vm.prop(cs, "ActiveWidgetIndex").i();
    let slots = ui.vm.o(cs).slots.clone();
    assert_eq!(ui.vm.o(slots[idx as usize]).content, Some(lp), "local play is the active content");
    assert!(ui.painted(lp));
    let cb = ui.vm.child(lp, "GameModeComboBox").unwrap();
    let modes: Vec<String> = ui.vm.prop(cb, "DefaultOptions").arr().iter().map(V::s).collect();
    assert!(modes.contains(&"Deathmatch".to_string()) && modes.contains(&"Team Deathmatch".to_string()), "{modes:?}");
    assert!(!modes.contains(&"Duel".to_string()), "Duel is filtered out of local play (BP_LocalPlay@3401)");
    let ml = ui.vm.child(lp, "BP_MapList").unwrap();
    let grid = ui.vm.child(ml, "EntryGrid").unwrap();
    assert!(ui.vm.o(grid).slots.len() >= 15, "map entries {}", ui.vm.o(grid).slots.len());
    // nothing is preselected: BP_MapList:SelectFirstEntry casts to BP_MapEntry_C, which BP_MapEntryLocalPlay_C
    // (parent UserWidget) is not, so the player clicks a map first, as in the game
    assert!(matches!(ui.vm.prop(ml, "SelectedEntry"), V::None));
    let first = ui.vm.o(ui.vm.o(grid).slots[0]).content.unwrap();
    let c = ui.centre(first).expect("first map tile painted");
    ui.click(c);
    frames(&mut ui, 2);
    assert_eq!(ui.vm.prop(ml, "SelectedEntry").obj(), Some(first), "clicking a tile selects it");
    ui.take_actions();
    click(&mut ui, "StartButton");
    let acts = ui.take_actions();
    let open = acts.iter().find_map(|a| match a {
        Action::OpenLevel { map, options } => Some((map.clone(), options.clone())),
        _ => None,
    });
    let (map, opts) = open.unwrap_or_else(|| panic!("no OpenLevel in {acts:?}"));
    assert!(map.starts_with("FFA_"), "{map}");
    assert!(opts.contains("PlayerCount=1"), "{opts}");
}

/// LoadSettings / the settings screen getters read the ini; ApplySettings writes it back in place
#[test]
fn settings_ini_round_trip() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let ini = "[/Script/Mordhau.MordhauGameUserSettings]\nGore=0\nFieldOfView=93.000000\nShowStatusBar=0\nKeepMe=1\n\n[/Script/Engine.GameUserSettings]\nbUseDesiredScreenHeight=False\n";
    let d = temp_config("settings", ini);
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "Main Menu");
    let s = ui.vm.world["settings"];
    let (v, _) = ui.vm.call_named(s, "GetFieldOfView", vec![]);
    assert_eq!(v.f(), 93.0);
    assert_eq!(ui.vm.call_named(s, "GetGore", vec![]).0.i(), 0);
    // SetToDefaults 0x15a7c90 values for keys the file does not have
    assert_eq!(ui.vm.call_named(s, "GetMaxRagdolls", vec![]).0.i(), 10);
    assert!(!ui.vm.call_named(s, "ShouldShowStatusBar", vec![]).0.truthy());
    let lim = ui.vm.call_named(s, "GetFieldOfViewLimits", vec![]).0;
    assert_eq!((lim.field("X").f(), lim.field("Y").f()), (30.0, 101.0));
    ui.vm.call_named(s, "SetFieldOfView", vec![V::Float(100.0)]);
    ui.vm.call_named(s, "ApplySettings", vec![V::Bool(false)]);
    let t = std::fs::read_to_string(d.join("GameUserSettings.ini")).unwrap();
    assert!(t.contains("FieldOfView=100.000000"), "{t}");
    assert!(t.contains("KeepMe=1") && t.contains("bUseDesiredScreenHeight=False"), "other lines kept: {t}");
    assert!(t.contains("MaxRagdolls=10"), "missing keys added: {t}");
    let _ = std::fs::remove_dir_all(d);
}

/// the settings screen: BP_MainMenu settings nav -> BP_GameSettings shows the ini's values in its widgets
#[test]
fn settings_screen_reads_values() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("screen", "[/Script/Mordhau.MordhauGameUserSettings]\nFieldOfView=93.000000\nMaxRagdolls=7\n");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "Main Menu");
    frames(&mut ui, 3);
    click(&mut ui, "SettingsButton");
    let gs = ui.find_any("BP_GameSettings").unwrap();
    assert!(ui.painted(gs), "game settings shown");
    let n = ui.out.rects.keys().filter(|&&w| ui.vm.o(w).class.name.contains("Slider")).count();
    assert!(n >= 3, "sliders painted {n}");
}

/// laid-out sizes equal the asset's numbers at 1920x1080 (DPI scale 1 from DefaultEngine.ini UIScaleCurve):
/// BP_MainMenu SizeBox_3 around ArmoryButton: WidthOverride 92 (bOverride_WidthOverride; its HeightOverride 74 is
/// stored but not enabled), height = its HorizontalBox slot's Fill = the bar's 92 (MaxDesiredHeight 92); the nav bar background
/// Image_0 (Brush ImageSize 1920 x 88, Fill in its overlay) spanning the viewport width; at 1280x720 everything scales
/// by the curve's 0.666
#[test]
fn layout_matches_asset() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("layout", "[/Script/Mordhau.MordhauGameUserSettings]\n");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "Main Menu");
    frames(&mut ui, 3);
    assert_eq!(ui.out.scale, 1.0);
    let armory = ui.find_any("ArmoryButton").unwrap();
    let r = ui.out.rects[&armory];
    assert!((r[2] - 92.0).abs() < 1e-3 && (r[3] - 92.0).abs() < 1e-3, "{r:?}");
    let mm = ui.main_menu().unwrap();
    let bg = ui.vm.child(mm, "Image_0").unwrap();
    let rb = ui.out.rects[&bg];
    assert!((rb[0]).abs() < 1e-3 && (rb[2] - 1920.0).abs() < 1e-3, "{rb:?}");
    ui.update(1.0 / 60.0, [1280.0, 720.0]);
    let s = ui.out.scale;
    assert!((s - 0.666).abs() < 1e-3, "{s}");
    let r2 = ui.out.rects[&armory];
    assert!((r2[2] - 92.0 * s).abs() < 1e-3 && (r2[3] - 92.0 * s).abs() < 1e-3, "{r2:?}");
}

/// the Game settings tab driven by input like a player: tick a checkbox, drag a slider, pick a dropdown option from its
/// opened menu, then Apply: the BP_GameSettings logic writes the values through UMordhauGameUserSettings into the
/// config dir's GameUserSettings.ini (a fresh dir: created by the save)
#[test]
fn settings_controls_apply() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let d = std::env::temp_dir().join(format!("mh-ui-test-controls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::env::set_var("MH_CONFIG_DIR", &d);
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "Main Menu");
    frames(&mut ui, 3);
    click(&mut ui, "SettingsButton");
    let gs = ui.find_any("BP_GameSettings").unwrap();
    let s = ui.vm.world["settings"];
    // a checkbox entry: its CheckBox widget
    let entry = ui.vm.child(gs, "ThirdPersonDeathCameraCheckbox").unwrap();
    let mut all = vec![];
    ui.vm.descendants(entry, &mut all);
    let cb = all.into_iter().find(|&w| ui.vm.o(w).class.native() == "CheckBox").expect("checkbox");
    let before = ui.vm.prop(cb, "CheckedState").i();
    let c = ui.centre(cb).unwrap();
    ui.click(c);
    frames(&mut ui, 2);
    assert_ne!(ui.vm.prop(cb, "CheckedState").i(), before, "checkbox toggles");
    // a slider: drag to 75 %
    let hb = ui.vm.child(gs, "HeadbobSlider").unwrap();
    let mut all = vec![];
    ui.vm.descendants(hb, &mut all);
    let sl = all.into_iter().find(|&w| ui.vm.o(w).class.native() == "Slider" && ui.painted(w)).expect("slider");
    let r = ui.out.rects[&sl];
    ui.mouse_move([r[0] + 2.0, r[1] + r[3] * 0.5]);
    ui.mouse_down("LeftMouseButton");
    ui.mouse_move([r[0] + r[2] * 0.75, r[1] + r[3] * 0.5]);
    ui.mouse_up("LeftMouseButton");
    frames(&mut ui, 2);
    let sv = ui.vm.prop(sl, "Value").f();
    assert!(sv > 0.6 && sv < 0.9, "slider value {sv}");
    // the Gore dropdown: open, choose its first option
    let gore = ui.vm.child(gs, "GoreDropdown").unwrap();
    let combo = ui.vm.child(gore, "Dropdown").unwrap();
    let c = ui.centre(combo).unwrap();
    ui.click(c);
    frames(&mut ui, 2);
    let row = ui.out.popup_rows.first().cloned().expect("menu open");
    let opt = ui.vm.prop(combo, "DefaultOptions").arr()[row.2].s();
    ui.click([row.0[0] + 5.0, row.0[1] + 5.0]);
    frames(&mut ui, 2);
    assert_eq!(ui.vm.prop(combo, "SelectedOption").s(), opt);
    // Apply
    let apply = ui.vm.child(gs, "ApplyButton").unwrap();
    let c = ui.centre(apply).unwrap();
    ui.click(c);
    frames(&mut ui, 3);
    let t = std::fs::read_to_string(d.join("GameUserSettings.ini")).expect("ini written on Apply");
    let tp = ui.vm.prop(s, "ThirdPersonDeathCamera").i();
    assert!(t.contains(&format!("ThirdPersonDeathCamera={tp}")), "{t}");
    // Headbob limits 0..2 (GetHeadbobLimits = GetCombatHeadbobLimits 0x1595300): 75 % of the bar ~ 1.5
    let hb_v = ui.vm.prop(s, "Headbob").f();
    assert!(hb_v > 1.0 && hb_v < 2.0, "head bob from the slider: {hb_v}");
    assert!(t.contains(&format!("Headbob={hb_v:.6}")), "{t}");
    let _ = std::fs::remove_dir_all(d);
}

/// click the painted text reading `text` (case-insensitive) where a player would: at its centre, which the topmost
/// hit-testable widget there (the button over it) receives
fn click_text(ui: &mut UiRuntime, text: &str) -> bool {
    let mut found = None;
    for (&w, r) in &ui.out.rects {
        if ui.vm.alive(w) && ui.vm.o(w).class.native() == "TextBlock" && ui.vm.prop(w, "Text").s().trim().eq_ignore_ascii_case(text) && r[2] > 0.0 {
            found = Some([r[0] + r[2] * 0.5, r[1] + r[3] * 0.5]);
        }
    }
    let Some(c) = found else { return false };
    ui.click(c);
    frames(ui, 3);
    true
}

/// Quit tab -> "Exit to desktop" -> the game's choice dialog -> "yes" -> UGameInstance QuitGame -> Action::Quit;
/// "no" closes the dialog
#[test]
fn quit_flow() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("quit", "");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "Main Menu");
    frames(&mut ui, 3);
    click(&mut ui, "QuitButton");
    assert!(click_text(&mut ui, "Exit to desktop"), "exit button");
    assert!(click_text(&mut ui, "no"), "dialog no");
    ui.take_actions();
    assert!(click_text(&mut ui, "Exit to desktop"), "exit button again");
    assert!(click_text(&mut ui, "yes"), "dialog yes");
    let acts = ui.take_actions();
    assert!(acts.contains(&Action::Quit), "{acts:?}");
}

/// in a match Escape opens BP_MainMenu (the pause menu, Return button shown); Return closes it again
#[test]
fn escape_menu_return() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("esc", "");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "FFA_Arena");
    frames(&mut ui, 3);
    assert!(!ui.main_menu_visible());
    ui.key_down("Escape");
    frames(&mut ui, 3);
    assert!(ui.main_menu_visible(), "Escape opens the menu");
    let ret = ui.find_any("ReturnButton").unwrap();
    assert!(ui.painted(ret), "Return shown in a match");
    let c = ui.centre(ret).unwrap();
    ui.click(c);
    frames(&mut ui, 3);
    assert!(!ui.main_menu_visible(), "Return closes the menu");
}

/// Escape again closes the escape menu: BP_MainMenu:OnPreviewKeyDown -> HandleInput(Escape) -> not on the "Main Menu"
/// map -> AskHUDToHideUs (state/ui_kismet/BP_MainMenu.txt HandleInput@464..@3573), the game's UI-only input mode
/// sending the key to the menu widget (BP_MordhauHUD:Show Main Menu@242 CustomSetInputModeUIOnly)
#[test]
fn escape_menu_toggle() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("esc2", "");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "DU_TestLevel");
    frames(&mut ui, 3);
    ui.key_down("Escape");
    ui.key_up("Escape");
    frames(&mut ui, 3);
    assert!(ui.main_menu_visible(), "Escape opens the menu");
    ui.key_down("Escape");
    ui.key_up("Escape");
    frames(&mut ui, 3);
    assert!(!ui.main_menu_visible(), "Escape again closes it");
    ui.key_down("Escape");
    frames(&mut ui, 3);
    assert!(ui.main_menu_visible(), "and opens it again");
}

/// probe (first-person r3): the escape menu's Quit tab in a match, the texts painted and the actions a click on each emits
#[test]
#[ignore]
fn probe_match_quit_tab() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("mquit", "");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "DU_TestLevel");
    ui.set_vitals(100, 100, true);
    frames(&mut ui, 3);
    ui.key_down("Escape");
    // BP_ArmoryTypeSelection's Tick -> Delay(0.1) -> GetIsLoadingAssets hides ArmoryNotLoadedBlur (ubergraph @15-@168)
    frames(&mut ui, 15);
    if std::env::var("MH_PROBE_NOQUIT").is_err() {
        click(&mut ui, "QuitButton");
    }
    let mut texts: Vec<String> = ui.out.rects.iter().filter(|(&w, r)| ui.vm.alive(w) && ui.vm.o(w).class.native() == "TextBlock" && r[2] > 0.0).map(|(&w, _)| ui.vm.prop(w, "Text").s()).collect();
    texts.sort();
    println!("quit tab texts: {texts:?}");
    let leave = std::env::var("MH_PROBE_TEXT").unwrap_or("Leave match".into());
    if std::env::var("MH_PROBE_HITS").is_ok() {
        for (&w, r) in &ui.out.rects {
            if ui.vm.alive(w) && ui.vm.o(w).class.native() == "TextBlock" && ui.vm.prop(w, "Text").s().trim().eq_ignore_ascii_case(&leave) && r[2] > 0.0 {
                let c = [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
                let hits: Vec<String> = ui.hits_at(c).iter().map(|&h| format!("{}[{}]", ui.vm.o(h).name, ui.vm.o(h).class.native())).collect();
                println!("hits at {c:?}: {hits:?}");
            }
        }
    }
    ui.vm.trace = std::env::var("MH_PROBE_TRACE").is_ok();
    println!("click {leave}: {}", click_text(&mut ui, &leave));
    ui.vm.trace = false;
    for _ in 0..60 {
        frames(&mut ui, 1);
    }
    println!("actions: {:?}", ui.take_actions());
}

/// probe (first-person r3 leak hunt): which VM containers grow per frame in a match HUD
#[test]
#[ignore]
fn probe_hud_growth() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("grow", "");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "DU_TestLevel");
    for step in 0..6 {
        let t = std::time::Instant::now();
        ui.perf = [0.0; 4];
        frames(&mut ui, 50);
        println!("phases ms/frame: ticks {:.1} widget Tick {:.1} bindings {:.1} paint {:.1}", ui.perf[0] * 20.0, ui.perf[1] * 20.0, ui.perf[2] * 20.0, ui.perf[3] * 20.0);
        println!("after {} frames ({:.1} ms/frame): objs {} latent {} playing {} actions {} budget_hits {} rects {} viewport {}", (step + 1) * 50, t.elapsed().as_secs_f64() * 1e3 / 50.0,
            ui.vm.objs.len(), ui.vm.latent.len(), ui.vm.playing.len(), ui.vm.actions.len(), ui.vm.budget_hits.len(), ui.out.rects.len(), ui.vm.viewport.len());
    }
    let n0 = ui.vm.objs.len();
    frames(&mut ui, 1);
    let mut h: std::collections::BTreeMap<String, usize> = Default::default();
    for o in &ui.vm.objs[n0..] {
        *h.entry(format!("{} [{}] alive {}", o.name, o.class.native(), o.alive)).or_default() += 1;
    }
    let mut v: Vec<_> = h.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("one frame's new objects: {:?}", &v[..v.len().min(25)]);
}

/// Settings > Video: BP_VideoSettings:UpdateResolutionDropdown fills the list from
/// UMordhauUtilityLibrary::GetSupportedScreenResolutions ("WxH a:b") and selects the current mode;
/// UpdateCharacterQualityDropdown selects the option at GetCharacterFidelity
#[test]
fn video_dropdowns_filled() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("video", "");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "Main Menu");
    frames(&mut ui, 3);
    click(&mut ui, "SettingsButton");
    click(&mut ui, "VideoButton");
    frames(&mut ui, 3);
    let vs = ui.find_any("BP_VideoSettings").unwrap();
    let res = ui.vm.child(ui.vm.child(vs, "ResolutionDropdown").unwrap(), "Dropdown").unwrap();
    let opts: Vec<String> = ui.vm.prop(res, "DefaultOptions").arr().iter().map(|v| v.s()).collect();
    assert!(opts.iter().any(|o| o == "1920x1080 16:9"), "{opts:?}");
    assert!(opts.iter().any(|o| o == "1280x1024 5:4"), "{opts:?}");
    assert!(!opts.iter().any(|o| o.starts_with("800x")), "modes under 1024x720 dropped");
    let cq = ui.vm.child(ui.vm.child(vs, "CharacterQualityDropdown").unwrap(), "Dropdown").unwrap();
    let cqo = ui.vm.prop(cq, "DefaultOptions").arr().len();
    let sel = ui.vm.prop(cq, "SelectedOption").s();
    assert!(cqo > 0 && !sel.is_empty(), "character quality options {cqo}, selected {sel:?}");
}

/// Slate blends in gamma space (SlateElementPixelShader.usf GammaCorrect before the SrcAlpha / InvSrcAlpha blend into a
/// UNORM back buffer): BP_GameSettings Image_39 (black, alpha 0.8) over a 0.5 display-encoded pixel gives 0.1, which
/// the linear-space remap must reproduce (it gave 0.36 before: the see-through Settings panel).
#[test]
fn slate_gamma_blend_matches() {
    let g = mh_ui::real::SLATE_GAMMA;
    let blend = |c: [f64; 4], bg_disp: f64| {
        let o = mh_ui::real::slate_blend(c);
        let out_lin = o[0] * o[3] + bg_disp.powf(g) * (1.0 - o[3]);
        out_lin.powf(1.0 / g)
    };
    assert!((blend([0.0, 0.0, 0.0, 0.8], 0.5) - 0.1).abs() < 1e-9);
    assert!((blend([0.0, 0.0, 0.0, 0.5], 0.8) - 0.4).abs() < 1e-9);
    // white at 0.5 over black: Slate gives 0.5 display-encoded
    assert!((blend([1.0, 1.0, 1.0, 0.5], 0.0) - 0.5).abs() < 1e-9);
    // opaque draws are unchanged
    assert_eq!(mh_ui::real::slate_blend([0.2, 0.3, 0.4, 1.0]), [0.2, 0.3, 0.4, 1.0]);
}

/// Settings > Action Bindings, the way a player rebinds: click Kick's primary key button
/// (BP_KeyBindingElementWidget StartEditBinding), press G (BP_KeyBindingsSettings:OnPreviewKeyDown -> HandleInputEvent ->
/// SetWidgetBinding), Apply (SaveSettings@125: UMordhauInput ClearKeyBindings + AddActionKeyBinding per row, then
/// ApplySettings / SaveSettings) -> the rewrite's Input.ini has Kick on G and not on F
#[test]
fn rebind_kick_saves_input_ini() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let d = temp_config("rebind", "");
    let _ = std::fs::remove_file(d.join("Input.ini"));
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new_sized(vfs, "Main Menu", Some(VP));
    frames(&mut ui, 3);
    click(&mut ui, "SettingsButton");
    click(&mut ui, "KeyBindingsButton");
    frames(&mut ui, 5);
    let kick = ui.find_any("Kick").expect("Kick row");
    let btn = ui.vm.child(kick, "PrimaryKeyButton").expect("primary button");
    // scroll it into view if needed: the list is a ScrollBox; wheel until painted
    for _ in 0..60 {
        if ui.centre(btn).is_some_and(|c| c[1] > 250.0 && c[1] < 950.0) {
            break;
        }
        let c = ui.centre(ui.find_any("BP_KeyBindingsSettings").unwrap()).unwrap();
        ui.mouse_move(c);
        ui.mouse_wheel(-1.0);
        frames(&mut ui, 1);
    }
    let c = ui.centre(btn).expect("Kick primary painted");
    let hs: Vec<String> = ui.hits_at(c).iter().map(|&h| format!("{}:{}", ui.vm.o(h).name, ui.vm.o(h).class.name)).collect();
    eprintln!("btn={btn} at {c:?} hits {hs:?}");
    ui.click(c);
    frames(&mut ui, 3);
    let kbs0 = ui.find_any("BP_KeyBindingsSettings").unwrap();
    eprintln!("selected after click: {:?} editing={:?} parent={:?} kbs0={kbs0}", ui.vm.prop(kbs0, "SelectedWidget"), ui.vm.prop(kick, "bIsEditingPrimary"), ui.vm.prop(kick, "ParentWidget"));
    ui.key_down("G");
    ui.key_up("G");
    frames(&mut ui, 3);
    let dlg = ui.vm.prop(kbs0, "DuplicateBindingDialog");
    eprintln!("dialog {:?} painted {:?} class {:?}", dlg, dlg.obj().map(|d| ui.painted(d)), dlg.obj().map(|d| ui.vm.o(d).class.name.clone()));
    // G is already bound elsewhere: BP_KeyBindingsSettings:HandleInputEvent@727 asks through its duplicate-binding
    // dialog; confirm with its left button as the player would
    if let Some(d) = dlg.obj().filter(|&d| ui.painted(d)) {
        let b = ui.find(d, "BP_PromptButton_LeftDialog").expect("left dialog button");
        let c = ui.centre(b).expect("dialog button painted");
        ui.click(c);
        frames(&mut ui, 3);
    }
    let sel = ui.vm.prop(kbs0, "SelectedWidget");
    eprintln!("after G: kick primary {:?} selected {:?} selected primary {:?}", ui.vm.prop(kick, "PrimaryBindings"), sel, sel.obj().map(|o| ui.vm.prop(o, "PrimaryBindings")));
    let kbs = ui.find_any("BP_KeyBindingsSettings").unwrap();
    let apply = ui.find(kbs, "ApplyButton").expect("ApplyButton");
    let c = ui.centre(apply).expect("ApplyButton painted");
    ui.click(c);
    frames(&mut ui, 3);
    let ini = std::fs::read_to_string(d.join("Input.ini")).expect("Input.ini written");
    let kick_lines: Vec<&str> = ini.lines().filter(|l| l.starts_with("ActionMappings=") && l.contains("ActionName=\"Kick\"")).collect();
    assert!(kick_lines.iter().any(|l| l.contains("Key=G)") || l.contains("Key=G,")), "{kick_lines:?}");
    assert!(!kick_lines.iter().any(|l| l.contains("Key=F)") || l.contains("Key=F,")), "{kick_lines:?}");
    // the axis rows as the real game saves them (UMordhauInput::AddAxisKeyBinding rva 0x1581f00): "Look Up" MouseY -1
    // and Gamepad_RightY +1, no "Look Down" axis at all, S on "Move Forward" -1, A on "Move Right" -1 (the user's real
    // Input.ini lines 282-294). ClearKeyBindings rva 0x1587c00 empties both arrays first, so names without a UI row
    // (Look Up Aim, Spectator Fly Up) are re-added at runtime from UInputSettings (UMordhauInput::Tick rva 0x15ac320)
    let axes: Vec<&str> = ini.lines().filter(|l| l.starts_with("AxisMappings=")).collect();
    let has = |l: &str| axes.contains(&l);
    assert!(has("AxisMappings=(AxisName=\"Look Up\",Scale=-1.000000,Key=MouseY)"), "{axes:?}");
    assert!(has("AxisMappings=(AxisName=\"Look Up\",Scale=1.000000,Key=Gamepad_RightY)"), "{axes:?}");
    assert!(has("AxisMappings=(AxisName=\"Move Forward\",Scale=-1.000000,Key=S)"), "{axes:?}");
    assert!(has("AxisMappings=(AxisName=\"Move Right\",Scale=-1.000000,Key=A)"), "{axes:?}");
    assert!(!axes.iter().any(|l| l.contains("Down\"") || l.contains("Backward\"") || l.contains("Left\"")), "{axes:?}");
    assert_eq!(axes.iter().filter(|l| l.contains("Key=MouseY)")).count(), 1, "{axes:?}");
}

/// Fight > Matchmaking and Fight > Server Browser open without any error dialog (rewrite choice: CanPlayOnline true,
/// natives.rs), and the top-bar navigation still works afterwards
#[test]
fn matchmaking_and_browser_open_cleanly() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("mm", "");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new_sized(vfs, "Main Menu", Some(VP));
    frames(&mut ui, 3);
    let dialogs = |ui: &UiRuntime| -> Vec<String> {
        ui.out.rects.keys().filter(|&&w| ui.vm.alive(w) && ui.vm.o(w).class.name.contains("Dialog")).map(|&w| ui.vm.o(w).class.name.clone()).collect()
    };
    click(&mut ui, "PlayButton");
    for tab in ["MatchmakingButton", "ServerBrowserButton"] {
        click(&mut ui, tab);
        frames(&mut ui, 30);
        let d = dialogs(&ui);
        assert!(d.is_empty(), "{tab}: dialogs {d:?}");
    }
    click(&mut ui, "SettingsButton");
    frames(&mut ui, 5);
    assert!(ui.painted(ui.find_any("BP_GameSettings").unwrap()), "navigation still works");
}

/// REWRITE-ONLY Combat Test tile (menu_data::combat_test_metadata): first in Local Match's map grid, and Start Match
/// travels to "<mode prefix>_TestLevel" with the bot count's PlayerCount
#[test]
fn local_match_lists_combat_test_first() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let _d = temp_config("combattest", "");
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new_sized(vfs, "Main Menu", Some(VP));
    frames(&mut ui, 3);
    click(&mut ui, "PlayButton");
    click(&mut ui, "LocalMatchButton");
    frames(&mut ui, 5);
    let ml = ui.find_any("BP_MapList").unwrap();
    let grid = ui.vm.child(ml, "EntryGrid").unwrap();
    let first = ui.vm.o(ui.vm.o(grid).slots[0]).content.unwrap();
    let name = ui.vm.child(first, "MapName").map(|t| ui.vm.prop(t, "Text").s()).unwrap_or_default();
    assert_eq!(name, "Combat Test");
    let c = ui.centre(first).expect("tile painted");
    ui.click(c);
    frames(&mut ui, 2);
    ui.take_actions();
    click(&mut ui, "StartButton");
    let acts = ui.take_actions();
    let map = acts.iter().find_map(|a| match a {
        Action::OpenLevel { map, .. } => Some(map.clone()),
        _ => None,
    });
    assert!(map.as_deref().is_some_and(|m| m.ends_with("_TestLevel")), "{acts:?}");
}

/// user report 2026-10-07: rebinding Strike to a MOUSE button through the menu saved Key=None. The way a player does
/// it: click Strike's primary key button, press the middle mouse button (FSlateApplication tunnels
/// OnPreviewMouseButtonDown to BP_KeyBindingsSettings, which takes the effecting button as the key), Apply -> the
/// rewrite's Input.ini has Strike on MiddleMouseButton and no Key=None row for it
#[test]
fn rebind_strike_to_a_mouse_button_saves_input_ini() {
    let _g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let d = temp_config("rebind_mouse", "");
    let _ = std::fs::remove_file(d.join("Input.ini"));
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new_sized(vfs, "Main Menu", Some(VP));
    frames(&mut ui, 3);
    click(&mut ui, "SettingsButton");
    click(&mut ui, "KeyBindingsButton");
    frames(&mut ui, 5);
    let row = ui.find_any("Strike").expect("Strike row");
    let btn = ui.vm.child(row, "PrimaryKeyButton").expect("primary button");
    for _ in 0..60 {
        if ui.centre(btn).is_some_and(|c| c[1] > 250.0 && c[1] < 950.0) {
            break;
        }
        let c = ui.centre(ui.find_any("BP_KeyBindingsSettings").unwrap()).unwrap();
        ui.mouse_move(c);
        ui.mouse_wheel(-1.0);
        frames(&mut ui, 1);
    }
    let c = ui.centre(btn).expect("Strike primary painted");
    ui.click(c);
    frames(&mut ui, 3);
    let kbs0 = ui.find_any("BP_KeyBindingsSettings").unwrap();
    ui.mouse_down("MiddleMouseButton");
    ui.mouse_up("MiddleMouseButton");
    frames(&mut ui, 3);
    let dlg = ui.vm.prop(kbs0, "DuplicateBindingDialog");
    if let Some(dw) = dlg.obj().filter(|&dw| ui.painted(dw)) {
        let b = ui.find(dw, "BP_PromptButton_LeftDialog").expect("left dialog button");
        let c = ui.centre(b).expect("dialog button painted");
        ui.click(c);
        frames(&mut ui, 3);
    }
    eprintln!("after MMB: strike primary {:?}", ui.vm.prop(row, "PrimaryBindings"));
    let kbs = ui.find_any("BP_KeyBindingsSettings").unwrap();
    let apply = ui.find(kbs, "ApplyButton").expect("ApplyButton");
    let c = ui.centre(apply).expect("ApplyButton painted");
    ui.click(c);
    frames(&mut ui, 3);
    let ini = std::fs::read_to_string(d.join("Input.ini")).expect("Input.ini written");
    let lines: Vec<&str> = ini.lines().filter(|l| l.starts_with("ActionMappings=") && l.contains("ActionName=\"Strike\"")).collect();
    assert!(lines.iter().any(|l| l.contains("Key=MiddleMouseButton")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("Key=None")), "{lines:?}");
}

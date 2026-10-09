//! Headless probe of the UI runtime: mount the menu map's UI, run frames, print the viewport, painted widgets and
//! missing natives.   sh scripts/cargo.sh run -p mh-ui --example probe [map name] [widget-to-dump]
use mh_ui::host::UiRuntime;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let map = a.get(1).cloned().unwrap_or_else(|| "Main Menu".into());
    let vfs = std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let t0 = std::time::Instant::now();
    let mut ui = UiRuntime::new(vfs, &map);
    for _ in 0..3 {
        ui.update(1.0 / 60.0, [1920.0, 1080.0]);
    }
    println!("init+3 frames {:.2}s", t0.elapsed().as_secs_f64());
    for (w, z) in &ui.vm.viewport {
        let o = ui.vm.o(*w);
        println!("viewport {} {} z={} vis={:?} painted={}", w, o.name, z, ui.vm.prop(*w, "Visibility"), ui.painted(*w));
    }
    println!("items {} hits {} rects {}", ui.out.items.len(), ui.out.hits.len(), ui.out.rects.len());
    println!("missing natives:");
    for (k, v) in &ui.vm.missing {
        println!("  {k} x{v}");
    }
    if let Some(t) = ui.find_any("BP_TitleScreen") {
        println!("title: vis={:?} key={:?} pad={:?} focus={:?} class_chain={:?}", ui.vm.prop(t, "Visibility"), ui.vm.prop(t, "KeyToContinue"), ui.vm.prop(t, "GamepadKeyToContinue"), ui.vm.focus.map(|f| ui.vm.o(f).name.clone()), ui.vm.o(t).class.chain());
        if let Some(bp) = ui.vm.child(t, "BP_ButtonPrompt") {
            println!("prompt keys: {:?} / {:?}", ui.vm.prop(bp, "Forced Key"), ui.vm.prop(bp, "Forced Key_Gamepad"));
        }
        let f = ui.vm.o(t).class.func("ExecuteUbergraph_BP_TitleScreen").unwrap();
        println!("{:?}", f.code[f.index[&868]]);
        ui.key_down("SpaceBar");
        ui.update(1.0 / 60.0, [1920.0, 1080.0]);
        println!("after key: vis={:?} focus={:?}", ui.vm.prop(t, "Visibility"), ui.vm.focus.map(|f| ui.vm.o(f).name.clone()));
    }
    if let Some(lp) = ui.find_any("BP_LocalPlay") {
        ui.vm.construct(lp);
        ui.update(1.0 / 60.0, [1920.0, 1080.0]);
        let gi = ui.vm.world["gi"];
        let gm = ui.vm.prop(gi, "GameModeMetadata");
        let mm = ui.vm.prop(gi, "MapMetadata");
        if let (mh_ui::model::V::Map(a), mh_ui::model::V::Map(b)) = (&gm, &mm) {
            println!("gi modes {} maps {}", a.len(), b.len());
            println!("modes {:?}", a.iter().map(|x| x.0.s()).collect::<Vec<_>>());
        }
        println!("lp Maps {} GameModes {:?}", ui.vm.prop(lp, "Maps").arr().len(), match ui.vm.prop(lp, "GameModes") { mh_ui::model::V::Map(m) => m.len(), _ => 0 });
        println!("first map: {:?}", ui.vm.prop(lp, "Maps").arr().first());
        let info = mh_ui::menu_data::map_info(&mut ui.vm, gi, "/Game/Mordhau/Maps/Arena_Map/FFA_Arena.FFA_Arena");
        println!("info {:?}", info);
        if let Some(ml) = ui.vm.child(lp, "BP_MapList") {
            let eg = ui.vm.child(ml, "EntryGrid").unwrap();
            println!("entrygrid slots {}", ui.vm.o(eg).slots.len());
            let cb = ui.vm.child(lp, "GameModeComboBox").unwrap();
            println!("combo opts {:?} sel {:?} delegates {:?}", ui.vm.prop(cb, "DefaultOptions"), ui.vm.prop(cb, "SelectedOption"), ui.vm.prop(cb, "OnSelectionChanged"));
            ui.vm.trace = true;
            ui.vm.call_named(lp, "BndEvt__GameModeComboBox_K2Node_ComponentBoundEvent_0_OnSelectionChangedEvent__DelegateSignature", vec![mh_ui::model::V::Str("Deathmatch".into()), mh_ui::model::V::Int(3)]);
            ui.vm.trace = false;
            println!("entrygrid slots after {}", ui.vm.o(eg).slots.len());
            let u = ui.vm.o(lp).uber.borrow();
            for k in ["CallFunc_Array_Get_Item_1", "Temp_struct_Variable_1", "CallFunc_Array_Get_Item_4", "CallFunc_Conv_SoftObjectPathToString_ReturnValue_1", "CallFunc_GetMapInfo_ReturnValue_1"] {
                println!("uber {k} = {:?}", u.get(k).map(|v| format!("{v:?}").chars().take(300).collect::<String>()));
            }
            println!("maplist class {} props {:?}", ui.vm.o(ml).class.name, ui.vm.o(ml).props.keys().collect::<Vec<_>>());
        }
    }
    if let Some(name) = a.get(2) {
        if let Some(w) = ui.find_any(name) {
            dump(&ui, w, 0);
        }
    }
}

fn dump(ui: &UiRuntime, w: u32, d: usize) {
    if d > 12 {
        return;
    }
    let o = ui.vm.o(w);
    println!("{}{} [{}] vis={} rect={:?}", "  ".repeat(d), o.name, o.class.name, ui.vm.prop(w, "Visibility").i(), ui.out.rects.get(&w).map(|r| r.map(|x| x.round())));
    if let Some(r) = o.root {
        dump(ui, r, d + 1);
    }
    for &s in &o.slots {
        if let Some(c) = ui.vm.o(s).content {
            dump(ui, c, d + 1);
        }
    }
}

//! probe: the status bar's progress bars in a match runtime (Percent, rect, fill brush)
use mh_ui::host::UiRuntime;
fn main() {
    let vfs = std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, "Contraband");
    ui.set_vitals(72, 55, true);
    for _ in 0..60 {
        ui.update(1.0 / 60.0, [1280.0, 720.0]);
    }
    let sb = ui.find_any("BP_StatusBar").unwrap();
    for k in ["ObservedHealth", "ObservedStamina", "DisplayedHealth", "DisplayedStamina", "ObservedCharacter"] {
        println!("{k} = {:?}", ui.vm.prop(sb, k));
    }
    for n in ["ProgressBar_DisplayedHealth", "ProgressBar_DelayedHealth", "ProgressBar_DisplayedStam", "ProgressBar_DelayedStam"] {
        let w = ui.vm.child(sb, n).unwrap();
        println!("{n} pct={:?} vis={:?} rect={:?} fill={:?}", ui.vm.prop(w, "Percent"), ui.vm.prop(w, "Visibility"), ui.out.rects.get(&w), ui.vm.prop(w, "WidgetStyle").field("FillImage").field("ResourceObject"));
    }
    for it in &ui.out.items {
        let n = &ui.vm.o(it.widget).name;
        if n.starts_with("ProgressBar_") {
            println!("item {n} {:?}", it.prim);
        }
    }
    for (k, v) in &ui.vm.missing {
        if k.contains("Observ") || k.contains("Character") || k.contains("Material") || k.contains("Scalar") {
            println!("missing {k} x{v}");
        }
    }
}

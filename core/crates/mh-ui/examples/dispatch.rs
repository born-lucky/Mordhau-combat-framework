//! probe: Local Match screen at a given window size: rects + draw items of StartButton, the header, map tiles, slider
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let w: f64 = a.get(1).and_then(|x| x.parse().ok()).unwrap_or(1277.0);
    let h: f64 = a.get(2).and_then(|x| x.parse().ok()).unwrap_or(735.0);
    let vfs = std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = mh_ui::host::UiRuntime::new(vfs, "Main Menu");
    let vp = [w, h];
    for _ in 0..5 {
        ui.update(1.0 / 60.0, vp);
    }
    for b in ["PlayButton", "LocalMatchButton"] {
        let x = ui.find_any(b).unwrap();
        let c = ui.centre(x).unwrap();
        ui.click(c);
        for _ in 0..5 {
            ui.update(1.0 / 60.0, vp);
        }
    }
    println!("scale {}", ui.out.scale);
    for it in &ui.out.items {
        let s = format!("{:?}", it.prim);
        let r = match &it.prim {
            mh_ui::slate::Prim::Rect { rect, .. } | mh_ui::slate::Prim::Image { rect, .. } => *rect,
            _ => continue,
        };
        if r[0] < 600.0 && r[0] + r[2] > 400.0 && r[1] < 165.0 && r[1] + r[3] > 135.0 && r[2] < 900.0 {
            let o = ui.vm.o(it.widget);
            println!("band {} [{}] {}", o.name, o.class.name, s.chars().take(200).collect::<String>());
        }
    }
    let lp = ui.find_any("BP_LocalPlay").unwrap();
    let dump = |ui: &mh_ui::host::UiRuntime, w: mh_ui::model::Id, depth: usize| {
        let mut all = vec![w];
        ui.vm.descendants(w, &mut all);
        for x in all {
            let o = ui.vm.o(x);
            let r = ui.out.rects.get(&x).map(|r| r.map(|v| v.round()));
            let items: Vec<String> = ui.out.items.iter().filter(|i| i.widget == x).map(|i| format!("{:?}", i.prim).chars().take(160).collect()).collect();
            if r.is_some() {
                println!("{:depth$}{} [{}] vis={:?} rect={:?}", "", o.name, o.class.name, ui.vm.prop(x, "Visibility"), r, depth = depth);
                for i in items {
                    println!("{:depth$}    item {}", "", i, depth = depth);
                }
            }
        }
    };
    for n in ["StartButton", "Image_4", "BotSettings"] {
        if let Some(x) = ui.find(lp, n) {
            println!("== {n}");
            dump(&ui, x, 2);
        }
    }
    let ml = ui.find(lp, "BP_MapList").unwrap();
    let mut all = vec![];
    ui.vm.descendants(ml, &mut all);
    let tiles: Vec<_> = all.into_iter().filter(|&x| ui.vm.o(x).class.name.starts_with("BP_MapEntry")).take(3).collect();
    for t in tiles {
        println!("== tile");
        dump(&ui, t, 2);
    }
}

//! Headless (no GPU) dump of what the Settings screens paint: for each tab, every painted item with its owning widget
//! chain, class, primitive, rect, font face / size and colour, to compare against the real game's frames.
//!   sh scripts/cargo.sh run -p mh-ui --example settings_dump -- [Tab ...] > out.tsv
//! Tabs: GameButton VideoButton AudioButton ControlsButton KeyBindingsButton ModsButton
use mh_ui::host::UiRuntime;
use mh_ui::slate::Prim;
use std::sync::Arc;

const VP: [f64; 2] = [1920.0, 1080.0];

fn main() {
    let cfg = std::env::temp_dir().join(format!("mh-ui-settings-dump-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&cfg);
    std::env::set_var("MH_CONFIG_DIR", &cfg);
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new_sized(vfs, "Main Menu", Some(VP));
    let frames = |ui: &mut UiRuntime, n: usize| {
        for _ in 0..n {
            ui.update(1.0 / 60.0, VP);
        }
    };
    let click = |ui: &mut UiRuntime, name: &str| {
        match ui.find_any(name).and_then(|w| ui.centre(w)) {
            Some(c) => ui.click(c),
            None => eprintln!("{name}: not painted, skipped"),
        }
    };
    frames(&mut ui, 3);
    click(&mut ui, "SettingsButton");
    frames(&mut ui, 30);
    let mut tabs: Vec<String> = std::env::args().skip(1).collect();
    if tabs.is_empty() {
        tabs = vec!["GameButton".into()];
    }
    if let Ok(pkg) = std::env::var("TEX") {
        // TEX=<texture package>: print its texels (r g b a, 0-255), row by row
        use mh_assets::texture;
        let t = texture::info(&ui.res.src, &pkg, None).expect("texture");
        let data = texture::mip_data(&ui.res.src, &t, 0).unwrap();
        let (w, h) = (t.mips[0].size_x as usize, t.mips[0].size_y as usize);
        if let Ok(texture::Pixels::Rgba8(px)) = texture::decode(t.format.unwrap(), w, h, &data) {
            println!("{pkg} {w}x{h} srgb={}", t.srgb);
            for y in 0..h {
                let row: Vec<String> = (0..w).map(|x| {
                    let p = &px[(y * w + x) * 4..][..4];
                    format!("{:02x}{:02x}", p[0], p[3])
                }).collect();
                println!("{}", row.join(" "));
            }
        }
        return;
    }
    for tab in tabs {
        // "hover:<widget path>" moves the mouse onto a widget instead of clicking it
        if let Some(h) = tab.strip_prefix("hover:") {
            let mut it = h.split('/');
            let mut w = ui.find_any(it.next().unwrap());
            for c in it {
                w = w.and_then(|w| ui.vm.child(w, c));
            }
            match w.and_then(|w| ui.centre(w)) {
                Some(c) => ui.mouse_move(c),
                None => eprintln!("{h}: not painted"),
            }
        } else {
            click(&mut ui, &tab);
        }
        frames(&mut ui, 30);
        println!("## {tab}");
        let items = ui.out.items.clone();
        for it in &items {
            // owning chain: widget names up through the user widgets
            let mut chain = vec![];
            let mut w = Some(it.widget);
            let mut n = 0;
            while let Some(x) = w {
                let o = ui.vm.o(x);
                chain.push(format!("{}:{}", o.name, o.class.name));
                w = o.parent_slot.and_then(|s| ui.vm.o(s).panel).or(o.owner.filter(|_| o.parent_slot.is_none()));
                n += 1;
                if n > 6 {
                    break;
                }
            }
            let r = |r: &[f64; 4]| format!("{:.0},{:.0},{:.0},{:.0}", r[0], r[1], r[2], r[3]);
            let c = |c: &[f64; 4]| format!("{:.2},{:.2},{:.2},{:.2}", c[0], c[1], c[2], c[3]);
            let line = match &it.prim {
                Prim::Rect { rect, color } => format!("rect\t{}\t{}\t", r(rect), c(color)),
                Prim::Image { rect, tex, tint, uv } => format!("img\t{}\t{}\t{} uv={:?}", r(rect), c(tint), tex, uv.map(|u| r(&u))),
                Prim::Text { rect, text, face, px, color } => format!("text\t{}\t{}\t{:?} {} {:.1}px", r(rect), c(color), text, face.rsplit('/').next().unwrap_or(face), px),
            };
            println!("{line}\t{}", chain.join(" < "));
        }
    }
    if let Ok(root) = std::env::var("TREE") {
        // TREE=<widget>: the widget tree under it (slot -> content), with class, visibility and laid-out rect
        fn walk(ui: &UiRuntime, w: mh_ui::model::Id, d: usize) {
            if d > 12 {
                return;
            }
            let o = ui.vm.o(w);
            let r = ui.out.rects.get(&w).map(|r| format!("{:.0},{:.0} {:.0}x{:.0}", r[0], r[1], r[2], r[3])).unwrap_or("-".into());
            println!("{}{}:{} vis={:?} {r}", "  ".repeat(d), o.name, o.class.name, ui.vm.prop(w, "Visibility"));
            let kids: Vec<_> = if let Some(root) = o.root { vec![root] } else { o.slots.iter().filter_map(|&s| ui.vm.o(s).content).collect() };
            for c in kids {
                walk(ui, c, d + 1);
            }
        }
        let w = ui.find_any(&root).expect("tree root");
        walk(&ui, w, 0);
    }
    if let Ok(probe) = std::env::var("PROBE") {
        // PROBE=<widget>/<child>:<prop>: print a property of a widget (debugging brush merges)
        let (path, prop) = probe.split_once(':').unwrap();
        let mut it = path.split('/');
        let mut w = ui.find_any(it.next().unwrap()).unwrap();
        for c in it {
            w = ui.vm.child(w, c).unwrap();
        }
        println!("{probe} = {:?}\n__scale = {:?}", ui.vm.prop(w, prop), ui.vm.prop(w, "__scale"));
    }
}

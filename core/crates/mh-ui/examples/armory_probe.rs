//! Headless probe of the Armory -> Mercenaries path (VM + Slate layout, no renderer): clicks the way the player does
//! and prints the active widgets, the natives the screens miss and the VM's step-budget overruns.
//!   sh scripts/cargo.sh run -p mh-ui --example armory_probe [-- "Main Menu"|<match map name>]
use mh_ui::host::UiRuntime;
use std::sync::Arc;

const VP: [f64; 2] = [1920.0, 1080.0];
/// host.rs calls armory tick / load_config itself (UiRuntime::new / update)
const HOOKED: bool = true;

fn frames(ui: &mut UiRuntime, n: usize) {
    for _ in 0..n {
        ui.update(1.0 / 60.0, VP);
        if !HOOKED {
            mh_ui::armory_natives::tick(&mut ui.vm, 1.0 / 60.0);
        }
    }
}

fn click(ui: &mut UiRuntime, name: &str) -> bool {
    let w = ui.vm.viewport.iter().rev().find_map(|&(r, _)| ui.find(r, name).filter(|&w| ui.painted(w)));
    let Some(c) = w.and_then(|w| ui.centre(w)) else {
        println!("click {name}: NOT PAINTED");
        return false;
    };
    println!("click {name} at {c:?}");
    ui.mouse_move(c);
    frames(ui, 2);
    ui.click(c);
    frames(ui, 10);
    true
}

fn active(ui: &UiRuntime, sw: &str) -> String {
    let Some(s) = ui.find_any(sw) else { return format!("<no {sw}>") };
    let i = ui.vm.prop(s, "ActiveWidgetIndex").i().max(0) as usize;
    ui.vm.o(s).slots.get(i).and_then(|&sl| ui.vm.o(sl).content).map(|c| ui.vm.o(c).name.clone()).unwrap_or_default()
}

fn report(ui: &mut UiRuntime, label: &str, seen: &mut std::collections::BTreeSet<String>) {
    let new: Vec<String> = ui.vm.missing.iter().filter(|(k, _)| seen.insert((*k).clone())).map(|(k, v)| format!("{k} x{v}")).collect();
    println!("== {label}: content {} / main {} / panel {}", active(ui, "ContentSwitcher"), active(ui, "WidgetSwitcher_Main"), active(ui, "CustomizationPanelSwitcher"));
    for m in new {
        println!("   missing {m}");
    }
    for b in ui.vm.budget_hits.drain(..) {
        println!("   budget {b}");
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let map = a.get(1).cloned().unwrap_or_else(|| "Main Menu".into());
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let mut ui = UiRuntime::new(vfs, &map);
    let mut seen = Default::default();
    frames(&mut ui, 30);
    if map != "Main Menu" {
        ui.show_main_menu();
        frames(&mut ui, 10);
    }
    report(&mut ui, "launch", &mut seen);
    click(&mut ui, "ArmoryButton");
    report(&mut ui, "armory", &mut seen);
    click(&mut ui, "BP_PromptButton_Mercenaries");
    report(&mut ui, "mercenaries", &mut seen);
    for n in a.iter().skip(2) {
        if let Some(w) = n.strip_prefix("why:") {
            why(&ui, w);
            continue;
        }
        if let Some(k) = n.strip_prefix("wait:") {
            frames(&mut ui, k.parse().unwrap_or(60));
            continue;
        }
        if let Some(spec) = n.strip_prefix("nth:") {
            // nth:<widget class prefix>:<n> - the n-th painted widget of that class in reading order (top, then left)
            let (cls, k) = spec.rsplit_once(':').unwrap_or((spec, "0"));
            click_nth(&mut ui, cls, k.parse().unwrap_or(0));
            report(&mut ui, n, &mut seen);
            state(&ui);
            continue;
        }
        if let Some(spec) = n.strip_prefix("props:") {
            // props:<widget name>:<prop>,<prop>... - raw VM values (arrays as their length + first item)
            let (wn, ps) = spec.split_once(':').unwrap_or((spec, ""));
            // a numeric name is an object id
            let found = wn.parse::<u32>().ok().filter(|&i| (i as usize) < ui.vm.objs.len()).or_else(|| ui.find_any(wn));
            if let Some(w) = found {
                let o = ui.vm.o(w);
                println!("   {wn} = obj {w} {} [{}] slots {} painted {}", o.name, o.class.name, o.slots.len(), ui.painted(w));
            }
            match found {
                Some(w) => {
                    for k in ps.split(',') {
                        let v = ui.vm.prop(w, k);
                        let s = match &v {
                            mh_ui::model::V::Array(a) => format!("array len {} first {:?}", a.len(), a.first()),
                            v => format!("{v:?}"),
                        };
                        println!("   {wn}.{k} = {}", s.chars().take(300).collect::<String>());
                    }
                }
                None => println!("   props {wn}: none"),
            }
            continue;
        }
        if let Some(spec) = n.strip_prefix("world:") {
            // world:<vm world key>:<prop> - a property of a VM world object (map, singleton, pc, ...)
            let (k, prop) = spec.split_once(':').unwrap_or((spec, ""));
            let v = ui.vm.world.get(k).map(|&o| ui.vm.prop(o, prop)).unwrap_or_default();
            println!("   world {k}.{prop} = {v:?}");
            continue;
        }
        if let Some(path) = n.strip_prefix("path:") {
            // path:<prop>.<prop>... - object properties walked from BP_MainMenu (e.g. CustomizationPlatform.CharacterDoll)
            let mut v = ui.main_menu().map(mh_ui::model::V::Obj).unwrap_or_default();
            for k in path.split('.') {
                v = match v.obj() {
                    Some(o) => ui.vm.prop(o, k),
                    None => v.field(k).clone(),
                };
            }
            let s = format!("{v:?}");
            let cls = v.obj().map(|o| ui.vm.o(o).class.name.clone()).unwrap_or_default();
            println!("   {path} = {} {cls}", s.chars().take(400).collect::<String>());
            continue;
        }
        if n == "trace:on" || n == "trace:off" {
            // every executed Blueprint statement to stderr (Vm::trace)
            ui.vm.trace = n == "trace:on";
            continue;
        }
        if n == "state" {
            state(&ui);
            continue;
        }
        if let Some(k) = n.strip_prefix("key:") {
            ui.key_down(k);
            frames(&mut ui, 2);
            ui.key_up(k);
            frames(&mut ui, 20);
            report(&mut ui, n, &mut seen);
            state(&ui);
            continue;
        }
        if let Some(root) = n.strip_prefix("dump:") {
            dump(&ui, root);
            continue;
        }
        click(&mut ui, n);
        report(&mut ui, n, &mut seen);
    }
}

/// every painted widget under `root` (name, class, rect, text)
fn dump(ui: &UiRuntime, root: &str) {
    let Some(r) = ui.find_any(root) else {
        println!("dump {root}: none");
        return;
    };
    let mut d = vec![];
    ui.vm.descendants(r, &mut d);
    for w in d {
        if !ui.painted(w) {
            continue;
        }
        let o = ui.vm.o(w);
        let txt = ui.vm.prop(w, "Text").s();
        let rc = ui.out.rects.get(&w).map(|r| format!("{:.0},{:.0} {:.0}x{:.0}", r[0], r[1], r[2], r[3])).unwrap_or_default();
        println!("  {} [{}] {rc} {}", o.name, o.class.name, txt);
    }
}

/// a widget and its ancestors: visibility, opacity, painted, rect (why something is not drawn)
fn why(ui: &UiRuntime, name: &str) {
    let Some(mut w) = ui.find_any(name) else {
        println!("why {name}: none");
        return;
    };
    loop {
        let o = ui.vm.o(w);
        println!("  {} [{}] vis {:?} opacity {:?} painted {} rect {:?}", o.name, o.class.name, ui.vm.prop(w, "Visibility"), ui.vm.prop(w, "RenderOpacity"), ui.painted(w), ui.out.rects.get(&w));
        let parent = o.parent_slot.and_then(|s| ui.vm.o(s).panel).or(o.owner.filter(|&x| ui.vm.o(x).root == Some(w)));
        match parent {
            Some(p) if p != w => w = p,
            _ => break,
        }
    }
}

fn click_nth(ui: &mut UiRuntime, cls: &str, n: usize) {
    let mut f: Vec<(i64, i64, u32)> = (1..ui.vm.objs.len() as u32)
        .filter(|&w| ui.vm.alive(w) && ui.vm.o(w).class.name.starts_with(cls) && ui.painted(w))
        .filter_map(|w| ui.out.rects.get(&w).map(|r| (r[1].round() as i64, r[0].round() as i64, w)))
        .collect();
    f.sort();
    let Some(c) = f.get(n).and_then(|x| ui.centre(x.2)) else {
        println!("nth {cls} {n}: NOT PAINTED ({} painted)", f.len());
        return;
    };
    let hits: Vec<String> = ui.hits_at(c).iter().take(4).map(|&h| ui.vm.o(h).name.clone()).collect();
    println!("nth {cls} {n} ({}) at {c:?}; hit path {hits:?}", ui.vm.o(f[n].2).name);
    ui.mouse_move(c);
    frames(ui, 2);
    ui.click(c);
    frames(ui, 20);
}

/// the picker selection and the profile being edited (ids, points left)
fn state(ui: &UiRuntime) {
    let sel = ui.find_any("BP_LoadoutPicker").map(|p| ui.vm.prop(p, "SelectedId").i());
    let Some(pc) = ui.find_any("BP_MordhauProfileCustomization") else { return };
    let Some(w) = ui.vm.prop(pc, "ProfileWrapper").obj() else { return };
    let p = ui.vm.prop(w, "Profile");
    let eq: Vec<i64> = p.field("GearCustomization").field("Equipment").arr().iter().map(|e| e.field("Id").i()).collect();
    let we = mh_ui::armory::wearable_ids(p.field("GearCustomization"));
    println!("   state: picker SelectedId {sel:?}; editing {:?} equipment {eq:?} wearables {we:?} perks {}", p.field("Name").s(), p.field("SkillsCustomization").field("Perks").i());
}

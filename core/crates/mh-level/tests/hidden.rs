//! `MeshPlacement::hidden_in_game`: bHiddenInGame / !bVisible through the component archetype chain (by name through
//! the Blueprint class chain, the InheritableComponentHandler override records included), hidden / editor-only actors.
//! FL_Camp's wagon push-area cylinder (BP_FrontlinePushable's Area_GEN_VARIABLE override: bHiddenInGame), FL_Taiga's
//! minecart rubble, and the DU_Arena list. SKIP (pass) without the install.

use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::sync::Arc;

fn pkgs() -> Option<Pkgs> {
    Vfs::mount_default().ok().map(|v| Pkgs::new(Reader::new(Arc::new(v))))
}

#[test]
fn hidden_placements() {
    let Some(pk) = pkgs() else { return eprintln!("SKIP") };
    let camp = read(&pk, "Mordhau/Content/Mordhau/Maps/DuelCamp/FL_Camp");
    let area: Vec<_> = camp.meshes.iter().filter(|m| m.actor.starts_with("BP_FrontlineWagon") && m.name.ends_with(".Area")).collect();
    assert!(!area.is_empty());
    for m in &area {
        println!("FL_Camp {} hidden {}", m.name, m.hidden_in_game);
        assert!(m.hidden_in_game, "{}", m.name);
    }
    let camp_hidden: Vec<&str> = camp.meshes.iter().filter(|m| m.hidden_in_game).map(|m| m.name.as_str()).collect();
    println!("FL_Camp hidden: {} of {}: {:?}", camp_hidden.len(), camp.meshes.len(), &camp_hidden[..camp_hidden.len().min(20)]);

    let taiga = read(&pk, "Mordhau/Content/Mordhau/Maps/TaigaMap/FL_Taiga");
    let rubble: Vec<_> = taiga.meshes.iter().filter(|m| m.actor.starts_with("BP_FrontlineMinecart") && (m.name.contains("BrokenScatterPile") || m.name.contains("RubblePile05"))).collect();
    for m in &rubble {
        println!("FL_Taiga {} hidden {}", m.name, m.hidden_in_game);
        assert!(m.hidden_in_game, "{}", m.name);
    }

    let arena = read(&pk, "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena");
    let hidden: Vec<_> = arena.meshes.iter().filter(|m| m.hidden_in_game).collect();
    println!("DU_Arena hidden placements: {} of {}", hidden.len(), arena.meshes.len());
    for m in &hidden {
        let t = m.xf.translation();
        println!("  {} mesh {} at ({:.0}, {:.0}, {:.0}) instances {}", m.name, m.mesh, t[0], t[1], t[2], m.instances.len());
    }
    println!("DU_Arena skips: {:?}", arena.skips.iter().map(|(k, v)| (k, v.len())).collect::<Vec<_>>());
}

/// What DU_Arena places within 6 m of each player start (candidates for the white bars at the bottom right of the
/// first-person view; prints only)
#[test]
fn arena_near_spawns() {
    let Some(pk) = pkgs() else { return eprintln!("SKIP") };
    let d = read(&pk, "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena");
    for s in &d.spawns {
        let p = s.xf.translation();
        println!("{} at ({:.0}, {:.0}, {:.0})", s.name, p[0], p[1], p[2]);
        for m in &d.meshes {
            let pts: Vec<[f64; 3]> = if m.instances.is_empty() { vec![m.xf.translation()] } else { m.instances.iter().map(|i| (m.xf * *i).translation()).collect() };
            let near = pts.iter().filter(|q| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() < 600.0).count();
            if near > 0 {
                println!("  {} [{}] {} x{near} hidden {}", m.name, m.component, m.mesh.rsplit('/').next().unwrap_or(""), m.hidden_in_game);
            }
        }
        for g in &d.gameplay {
            if let Some(x) = g.xf {
                let q = x.translation();
                if ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() < 600.0 {
                    println!("  actor {} {}", g.name, g.class);
                }
            }
        }
    }
}

/// Dump DU_Arena / FL_Camp placements (name, mesh, materials, translation, hidden) to $MH_DUMP for before/after diffs
#[test]
#[ignore]
fn dump_placements() {
    let Some(pk) = pkgs() else { return };
    let out = std::env::var("MH_DUMP").unwrap_or_else(|_| "placements.tsv".into());
    let mut s = String::new();
    for m in ["Arena_Map/DU_Arena", "DuelCamp/FL_Camp"] {
        let d = read(&pk, &format!("Mordhau/Content/Mordhau/Maps/{m}"));
        for p in &d.meshes {
            let t = p.xf.translation();
            let r = p.xf.m;
            s += &format!("{m}\t{}\t{}\t{:?}\t{:.2},{:.2},{:.2}\t{:.4},{:.4},{:.4}\t{}\t{}\n", p.name, p.mesh, p.materials, t[0], t[1], t[2], r[0][0], r[1][0], r[2][2], p.cast_shadow, p.hidden_in_game);
        }
        for p in &d.splines {
            s += &format!("{m}\t{}\t{}\t{:?}\tspline\n", p.name, p.mesh, p.materials);
        }
        pk.clear();
    }
    std::fs::write(out, s).unwrap();
}

/// Every map: render-relevant component props that would change if the template chain followed the parent classes'
/// same-named templates by name instead of the cooked Template links (`legacy_props`): 0 on every map, so the links
/// already reach the InheritableComponentHandler overrides
#[test]
#[ignore]
fn template_chain_changes() {
    let Some(v) = Vfs::mount_default().ok() else { return };
    let maps: Vec<String> = v.list().filter(|p| p.contains("/Maps/") && p.ends_with(".umap")).map(|p| p.trim_end_matches(".umap").to_string()).collect();
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    let keys = ["StaticMesh", "SkeletalMesh", "OverrideMaterials", "RelativeLocation", "RelativeRotation", "RelativeScale3D", "bVisible", "bHiddenInGame", "CastShadow"];
    let mut total = 0;
    for m in &maps {
        let d = read(&pk, m);
        let mut n = 0;
        for lv in &d.levels {
            for e in pk.load_pkg(&lv.pkg).iter() {
                let t = e.get("Type").and_then(|t| t.as_str()).unwrap_or("");
                if !(t.contains("MeshComponent")) {
                    continue;
                }
                let Some(tp) = e.get("Template").and_then(|t| t.get("ObjectPath")).and_then(|v| v.as_str()) else { continue };
                let Some(o) = pk.obj(e.get("Template")) else { continue };
                if !o.get("Name").and_then(|v| v.as_str()).unwrap_or("").ends_with("_GEN_VARIABLE") {
                    continue;
                }
                // old = the cooked Template links (`Pkgs::props`); new = by-name chain under it
                let old = pk.props(e);
                let mut new = name_chain(&pk, tp);
                for (k, v) in legacy_props(&pk, e) {
                    new.insert(k, v);
                }
                let changed: Vec<&str> = keys.iter().copied().filter(|k| new.get(*k) != old.get(*k)).collect();
                if !changed.is_empty() {
                    n += 1;
                    if std::env::var("MH_MAPS").is_ok_and(|v| v.split(',').any(|x| m.ends_with(x))) {
                        for k in &changed {
                            println!("    {} {} [{t}] {k}: {:?} -> {:?}", m.rsplit('/').next().unwrap(), e.get("Outer").and_then(|o| o.get("ObjectName")).and_then(|v| v.as_str()).unwrap_or("").rsplit('.').next().unwrap_or(""), old.get(*k).map(|v| v.to_string().chars().take(100).collect::<String>()), new.get(*k).map(|v| v.to_string().chars().take(100).collect::<String>()));
                        }
                    }
                    if n <= 5 {
                        println!("  {} {} {tp}: {changed:?} mesh {:?}", m.rsplit('/').next().unwrap(), e.get("Name").and_then(|v| v.as_str()).unwrap_or(""), new.get("StaticMesh").or(new.get("SkeletalMesh")).and_then(|v| v.get("ObjectPath")));
                    }
                }
            }
        }
        if n > 0 {
            println!("CHANGED {} {n}", m.rsplit('/').next().unwrap());
        }
        total += n;
        pk.clear();
    }
    println!("total components changed: {total}");
}

/// `Pkgs::props` before the name-based template chain: own properties over the Template link's, recursively
fn legacy_props(pk: &Pkgs, e: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let own = e.get("Properties").and_then(|p| p.as_object()).cloned().unwrap_or_default();
    let Some(t) = e.get("Template") else { return own };
    let mut out = pk.obj(Some(t)).map(|o| legacy_props(pk, &o)).unwrap_or_default();
    for (k, v) in own {
        out.insert(k, v);
    }
    out
}

/// The same-named <Name>_GEN_VARIABLE templates through the Blueprint class chain of `template_path`, parent first
fn name_chain(pk: &Pkgs, template_path: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    let Some((pkg, idx)) = template_path.rsplit_once('.') else { return out };
    let exps = pk.load_pkg(pkg);
    let Some(name) = idx.parse::<usize>().ok().and_then(|i| exps.get(i)).and_then(|e| e.get("Name")).and_then(|v| v.as_str()).map(|s| s.to_string()) else { return out };
    for cls in pk.rd.chain(pkg).iter().rev() {
        for ce in pk.load_pkg(cls).iter() {
            if ce.get("Name").and_then(|v| v.as_str()) == Some(name.as_str()) {
                for (k, v) in ce.get("Properties").and_then(|p| p.as_object()).cloned().unwrap_or_default() {
                    out.insert(k, v);
                }
            }
        }
    }
    out
}

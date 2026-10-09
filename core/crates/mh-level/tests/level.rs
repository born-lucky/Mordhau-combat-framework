//! mh-level against the Godot generator's scenes (godot/data_gen/maps/<Map>.tscn, written by tools/gen_level.gd from
//! UeLevel.read + build) and against extract/json actor counts, for DU_Arena, FFA_Arena and DU_Contraband.
//! The scene holds a node only where the exported .glb exists (godot/data = extract/gltf), so placements are compared
//! by name + mesh, and counts after the same glb filter. Transforms: Xf::to_gltf vs the node's Transform3D, 1 mm and
//! 1e-4 on the basis (the scene stores float32). Without an install, extract/ or the scenes: SKIP (pass).

use mh_level::{read, Pkgs, Xf};
use mh_pak::{Reader, Vfs};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAPS: &str = "Mordhau/Content/Mordhau/Maps/";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn pkgs() -> Option<Pkgs> {
    match Vfs::mount_default() {
        Ok(v) => Some(Pkgs::new(Reader::new(Arc::new(v)))),
        Err(e) => {
            eprintln!("SKIP: {e}");
            None
        }
    }
}

#[derive(Debug, Default, Clone)]
struct Node {
    name: String,
    parent: String,
    xf: Option<[f64; 12]>,
    meta: HashMap<String, String>,
}

/// The nodes of a .tscn (name, parent, transform, metadata/* raw values)
fn tscn(path: &Path) -> Option<Vec<Node>> {
    let t = std::fs::read_to_string(path).ok()?;
    let mut out: Vec<Node> = vec![];
    let mut cur: Option<Node> = None;
    for l in t.lines() {
        if l.starts_with('[') {
            if let Some(n) = cur.take() {
                out.push(n);
            }
            if l.starts_with("[node ") {
                let attr = |k: &str| l.split(&format!("{k}=\"")).nth(1).and_then(|r| r.split('"').next()).unwrap_or("").to_string();
                cur = Some(Node { name: attr("name"), parent: attr("parent"), ..Default::default() });
            }
        } else if let Some(n) = cur.as_mut() {
            if let Some(r) = l.strip_prefix("transform = Transform3D(") {
                n.xf = parse12(r);
            } else if let Some(r) = l.strip_prefix("metadata/") {
                if let Some((k, v)) = r.split_once(" = ") {
                    n.meta.insert(k.to_string(), v.to_string());
                }
            }
        }
    }
    out.extend(cur);
    Some(out)
}

fn parse12(r: &str) -> Option<[f64; 12]> {
    let v: Vec<f64> = r.trim_end_matches(')').split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() == 12).then(|| v.try_into().unwrap())
}

/// Every Transform3D(...) in a metadata array (ue_inst)
fn xf_list(v: &str) -> Vec<[f64; 12]> {
    v.split("Transform3D(").skip(1).filter_map(|p| parse12(p.split(')').next().unwrap_or(""))).collect()
}

/// Godot Transform3D text: 9 basis values then the origin. Godot writes basis.rows[i][j] (row-major).
fn godot_xf(a: &[f64; 12]) -> Xf {
    Xf { m: [[a[0], a[1], a[2], a[9]], [a[3], a[4], a[5], a[10]], [a[6], a[7], a[8], a[11]]] }
}

fn diff(mine: &Xf, theirs: Option<&[f64; 12]>) -> f64 {
    let g = theirs.map(godot_xf).unwrap_or(mh_level::xf::IDENTITY);
    let a = mine.to_gltf();
    let mut d: f64 = 0.0;
    for r in 0..3 {
        for c in 0..3 {
            d = d.max((a.m[r][c] - g.m[r][c]).abs() / 10.0); // basis tolerance 1e-4 vs origin 1e-3 m
        }
        d = d.max((a.m[r][3] - g.m[r][3]).abs());
    }
    d
}

/// Node.validate_node_name: . : @ / " % become _
fn valid(n: &str) -> String {
    n.chars().map(|c| if ".:@/\"%".contains(c) { '_' } else { c }).collect()
}

fn unquote(s: &str) -> &str {
    s.trim_matches('"')
}

/// Actors per level in extract/json: exports whose Outer is the level's "Level" export
fn json_actor_count(pkg: &str) -> Option<usize> {
    let t = std::fs::read_to_string(repo().join("extract/json").join(format!("{pkg}.json"))).ok()?;
    let a: Vec<Value> = mh_pak::equiv::parse_lenient(&t)?.as_array()?.clone();
    let li = a.iter().position(|e| e["Type"] == "Level")?;
    let want = format!(".{li}");
    Some(a.iter().enumerate().filter(|(i, e)| *i != li && e["Outer"]["ObjectPath"].as_str().is_some_and(|p| p.ends_with(&want) && !p[..p.len() - want.len()].contains('.'))).count())
}

fn check_map(pk: &Pkgs, map: &str) {
    let name = map.rsplit('/').next().unwrap();
    let t0 = std::time::Instant::now();
    let d = read(pk, &format!("{MAPS}{map}"));
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "{name}: {} levels, {} actors, {} statics, {} instanced ({} instances), {} hlods, {} spawns, {} crowd, {} lights, {} volumes, seen {}, skips {:?} ({secs:.2} s)",
        d.levels.len(),
        d.actors.len(),
        d.statics().count(),
        d.instanced().count(),
        d.instanced().map(|m| m.instances.len()).sum::<usize>(),
        d.hlods.len(),
        d.spawns.len(),
        d.crowd.len(),
        d.lights.locals.len() + d.lights.sun.is_some() as usize,
        d.volumes.len(),
        d.seen,
        d.skip_counts()
    );
    assert!(!d.meshes.is_empty(), "{name}: no meshes");

    // actors per level == extract/json
    for (i, lv) in d.levels.iter().enumerate() {
        if let Some(n) = json_actor_count(&lv.pkg) {
            let mine = d.actors.iter().filter(|a| a.level == i).count();
            assert_eq!(mine, n, "{}: actors vs extract/json", lv.pkg);
        }
    }

    let Some(nodes) = tscn(&repo().join(format!("godot/data_gen/maps/{name}.tscn"))) else {
        eprintln!("SKIP scene compare: no godot/data_gen/maps/{name}.tscn");
        return;
    };
    let data = repo().join("godot/data");
    let has_glb = |pkg: &str| data.join(format!("{pkg}.glb")).exists();

    // static meshes: every scene node matches a placement of the same name and mesh, within tolerance
    let scene: Vec<&Node> = nodes.iter().filter(|n| n.parent == "Meshes").collect();
    let mut by_name: HashMap<String, Vec<&mh_level::MeshPlacement>> = HashMap::new();
    for m in d.statics() {
        by_name.entry(valid(&m.name)).or_default().push(m);
    }
    let mut worst: f64 = 0.0;
    let mut unmatched = vec![];
    // the scenes written before ue_level.gd skipped Ultra_Dynamic_Sky_BP's own meshes (DU_Contraband, 10:56) still hold
    // them; the current rule (and mh-level) skips them as "sky"
    let stale_sky = |n: &Node| unquote(n.meta.get("ue_mesh").map(|s| s.as_str()).unwrap_or("")).starts_with("Mordhau/Content/UltraDynamicSky/Meshes/");
    let n_stale = scene.iter().filter(|n| stale_sky(n)).count();
    for n in scene.iter().filter(|n| !stale_sky(n)) {
        let mesh = unquote(n.meta.get("ue_mesh").map(|s| s.as_str()).unwrap_or(""));
        // duplicates were renamed "<name>_<count>"
        let base = match by_name.get(&n.name) {
            Some(_) => n.name.clone(),
            None => n.name.rsplit_once('_').map(|(a, _)| a.to_string()).unwrap_or_default(),
        };
        let best = by_name
            .get(&base)
            .into_iter()
            .flatten()
            .filter(|m| m.mesh == mesh)
            .map(|m| diff(&m.xf, n.xf.as_ref()))
            .fold(f64::INFINITY, f64::min);
        if best.is_finite() {
            worst = worst.max(best);
        } else {
            unmatched.push(n.name.clone());
        }
    }
    let placeable = d.statics().filter(|m| has_glb(m.mesh_pkg())).count();
    println!("  statics: scene {} ({n_stale} stale UDS sky nodes), mine with a glb {} (of {}), worst diff {worst:.2e}, unmatched {}", scene.len(), placeable, d.statics().count(), unmatched.len());
    assert!(unmatched.is_empty(), "{name}: scene nodes without a placement: {:?}", &unmatched[..unmatched.len().min(5)]);
    assert_eq!(scene.len() - n_stale, placeable, "{name}: static count (after the glb filter)");
    assert!(worst < 1e-3, "{name}: static transform diff {worst}");

    // instanced: per node the component transform and every instance
    let scene_i: Vec<&Node> = nodes.iter().filter(|n| n.parent == "Instanced").collect();
    let mut worst_i: f64 = 0.0;
    let mut inst_scene = 0;
    for n in &scene_i {
        let mesh = unquote(n.meta.get("ue_mesh").map(|s| s.as_str()).unwrap_or(""));
        let inst = xf_list(n.meta.get("ue_inst").map(|s| s.as_str()).unwrap_or(""));
        inst_scene += inst.len();
        let m = d
            .instanced()
            .find(|m| valid(&m.name) == n.name && m.mesh == mesh)
            .unwrap_or_else(|| panic!("{name}: no instanced placement {}", n.name));
        assert_eq!(m.instances.len(), inst.len(), "{name}: {} instance count", n.name);
        worst_i = worst_i.max(diff(&m.xf, n.xf.as_ref()));
        for (a, b) in m.instances.iter().zip(&inst) {
            worst_i = worst_i.max(diff(a, Some(b)));
        }
    }
    let placeable_i = d.instanced().filter(|m| has_glb(m.mesh_pkg())).count();
    println!("  instanced: scene {} ({inst_scene} instances), mine with a glb {placeable_i}, worst diff {worst_i:.2e}", scene_i.len());
    assert_eq!(scene_i.len(), placeable_i, "{name}: instanced count");
    assert!(worst_i < 1e-3, "{name}: instance transform diff {worst_i}");

    // HLOD proxies (glb = <mesh pkg>/<mesh name>.glb)
    let scene_h: Vec<&Node> = nodes.iter().filter(|n| n.parent == "HLOD").collect();
    let mut worst_h: f64 = 0.0;
    for n in &scene_h {
        let h = d.hlods.iter().find(|h| valid(&h.name) == n.name).unwrap_or_else(|| panic!("{name}: no hlod {}", n.name));
        worst_h = worst_h.max(diff(&h.xf, n.xf.as_ref()));
        let min: f64 = n.meta.get("ue_hlod_min").and_then(|v| v.parse().ok()).unwrap_or(f64::NAN);
        assert!((min - h.min_draw).abs() < 1e-2, "{name}: {} min_draw {} vs {min}", h.name, h.min_draw);
        assert_eq!(n.meta.get("ue_hlod_level").map(|s| s.as_str()), Some(h.lod_level.to_string().as_str()));
    }
    let placeable_h = d.hlods.iter().filter(|h| data.join(format!("{}/{}.glb", mh_level::level::pkg_of(&h.mesh), h.mesh_name)).exists()).count();
    println!("  hlods: scene {}, mine with a glb {placeable_h} (of {}), worst diff {worst_h:.2e}", scene_h.len(), d.hlods.len());
    assert_eq!(scene_h.len(), placeable_h, "{name}: hlod count");
    assert!(worst_h < 1e-3);

    // PlayerStarts: names, transforms, team
    let scene_s: Vec<&Node> = nodes.iter().filter(|n| n.parent == "PlayerStarts").collect();
    assert_eq!(scene_s.len(), d.spawns.len(), "{name}: spawn count");
    for n in &scene_s {
        let s = d.spawns.iter().find(|s| valid(&s.name) == n.name).unwrap_or_else(|| panic!("{name}: no spawn {}", n.name));
        assert!(diff(&s.xf, n.xf.as_ref()) < 1e-3, "{name}: spawn {} transform", s.name);
        let team = n.meta.get("ue_team").and_then(|v| v.parse::<f64>().ok());
        assert_eq!(team, s.team.as_ref().and_then(|v| v.as_f64()), "{name}: spawn {} team", s.name);
    }

    // spectators
    let scene_c: Vec<&Node> = nodes.iter().filter(|n| n.parent == "Crowd").collect();
    let mut worst_c: f64 = 0.0;
    for n in &scene_c {
        let c = d.crowd.iter().find(|c| valid(&c.name) == n.name || n.name.starts_with(&format!("{}_", valid(&c.name))));
        let c = c.unwrap_or_else(|| panic!("{name}: no crowd {}", n.name));
        assert_eq!(n.meta.get("ue_anim").map(|s| unquote(s)), Some(c.anim.as_str()));
        worst_c = worst_c.max(diff(&c.xf, n.xf.as_ref()));
    }
    println!("  crowd: scene {}, mine {}, worst diff {worst_c:.2e}; spawns {}", scene_c.len(), d.crowd.len(), d.spawns.len());
    assert!(scene_c.len() <= d.crowd.len());
    assert!(worst_c < 1e-3);
}

#[test]
fn du_arena() {
    let Some(pk) = pkgs() else { return };
    check_map(&pk, "Arena_Map/DU_Arena");
    let d = read(&pk, &format!("{MAPS}Arena_Map/DU_Arena"));
    // the persistent map streams Arena (UeLevel.levels), the sun and the sky are found
    assert!(d.levels.iter().any(|l| l.pkg.ends_with("Arena_Map/Arena")), "{:?}", d.levels.iter().map(|l| &l.pkg).collect::<Vec<_>>());
    assert!(d.lights.sun.is_some() && d.sun_actor.is_some());
}

#[test]
fn ffa_arena() {
    let Some(pk) = pkgs() else { return };
    check_map(&pk, "Arena_Map/FFA_Arena");
}

#[test]
fn du_contraband() {
    let Some(pk) = pkgs() else { return };
    let map = (|| {
        for p in pk.rd.vfs.list() {
            if p.ends_with("/DU_Contraband.umap") {
                return Some(p.trim_start_matches(MAPS).trim_end_matches(".umap").to_string());
            }
        }
        None
    })();
    let Some(map) = map else {
        eprintln!("SKIP: no DU_Contraband in the paks");
        return;
    };
    check_map(&pk, &map);
}

/// NavMeshBoundsVolume boxes (ModeData.nav_bounds) and the Recast agent of the shipped ini (ModeData.nav_agent)
#[test]
fn nav_bounds_and_agent() {
    let Some(pk) = pkgs() else { return };
    let d = read(&pk, &format!("{MAPS}Arena_Map/FFA_Arena"));
    let nav: Vec<_> = d.volumes.iter().filter(|v| v.class == "NavMeshBoundsVolume").collect();
    assert!(!nav.is_empty(), "no NavMeshBoundsVolume");
    for v in &nav {
        assert!((0..3).all(|k| v.max[k] > v.min[k]), "{} empty box", v.name);
        // the arena (|x|, |y| < 30 m around the origin, test_ai_world) lies inside the bounds
        assert!(v.min[0] < -1000.0 && v.max[0] > 1000.0, "{} {:?}..{:?}", v.name, v.min, v.max);
    }
    let a = mh_level::config::nav_agent(&pk.rd.vfs);
    // ai r9 log: r 50, h 192, step 60, cell 5/5 cm, slope 44 (DefaultEngine.ini / BaseEngine.ini)
    assert_eq!((a.radius, a.height, a.max_step, a.cell_size, a.cell_height, a.max_slope), (50.0, 192.0, 60.0, 5.0, 5.0, 44.0));
    println!("nav: {} volumes, agent {a:?}", nav.len());
}

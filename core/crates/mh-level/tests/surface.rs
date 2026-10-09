//! `CollisionWorld::surface_at`: the EPhysicalSurface of a hit (footsteps, world hits). DU_Arena: the floor under
//! every player start resolves through the mesh's physical material chain; FL_Camp: the landscape's cooked heightfield
//! material under a point agrees with the dominant painted layer's LayerInfo PhysMaterial. SKIP (pass) without the
//! install.

use mh_level::collision::{phys_material_surface, CollisionWorld};
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// DefaultEngine.ini [/Script/Engine.PhysicsSettings] PhysicalSurfaces
const NAMES: [&str; 11] = ["Default", "Flesh", "Wood", "Metal", "Stone", "Dirt", "Grass", "Pebbles", "Wet", "Sand", "Snow"];

fn setup() -> Option<(Arc<Vfs>, Pkgs)> {
    let vfs = Arc::new(Vfs::mount_default().ok()?);
    Some((vfs.clone(), Pkgs::new(Reader::new(vfs))))
}

#[test]
fn arena_floor_surfaces() {
    let Some((vfs, pk)) = setup() else { return eprintln!("SKIP") };
    let d = read(&pk, "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena");
    let src = mh_assets::pak_source::PakSource::new(vfs);
    let tris = |pkg: &str| -> Option<Vec<[f64; 3]>> {
        let m = mh_assets::static_mesh::lod0(&src, pkg).ok()?;
        Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
    };
    let w = CollisionWorld::build(&pk, &d, Some(&tris));
    let mut hist: BTreeMap<u8, usize> = BTreeMap::new();
    for b in &w.bodies {
        *hist.entry(b.surfaces.first().copied().unwrap_or(0)).or_default() += 1;
    }
    println!("DU_Arena body surfaces: {:?}", hist.iter().map(|(k, n)| (NAMES[*k as usize], *n)).collect::<Vec<_>>());
    let mut floors = 0;
    for s in &d.spawns {
        let p = s.xf.translation();
        let Some(h) = w.trace([p[0], p[1], p[2] + 50.0], [p[0], p[1], p[2] - 2000.0], &|b| w.blocks[b as usize]) else { continue };
        let surf = w.surface_at(h.body, h.impact_point);
        let b = &w.bodies[h.body as usize];
        println!("  {} floor {} ({}) phys {:?} -> {} {}", s.name, b.name, b.kind, b.physical_materials, surf, NAMES[surf as usize]);
        floors += 1;
        assert!((surf as usize) < NAMES.len());
    }
    assert!(floors > 0);
    // the arena is built from real surfaces, not all SurfaceType_Default
    assert!(hist.keys().any(|k| *k != 0), "{hist:?}");
}

#[test]
fn camp_landscape_surfaces_match_paint_layers() {
    let Some((_, pk)) = setup() else { return eprintln!("SKIP") };
    let d = read(&pk, "Mordhau/Content/Mordhau/Maps/DuelCamp/FL_Camp");
    let w = CollisionWorld::build(&pk, &d, None);
    let mut cache = HashMap::new();
    // LayerInfo -> its PhysMaterial's surface (ULandscapeLayerInfoObject PhysMaterial)
    let mut layer_surface = |info: &str| -> Option<u8> {
        let o = pk.obj(Some(&serde_json::json!({ "ObjectPath": info })))?;
        let pp = pk.props(&o);
        let pm = pp.get("PhysMaterial").and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str())?;
        Some(phys_material_surface(&pk, &mut cache, mh_level::level::pkg_of(pm)))
    };
    let (mut agree, mut total) = (0usize, 0usize);
    let mut hist: BTreeMap<u8, usize> = BTreeMap::new();
    for h in &w.heightfields {
        let Some(rc) = d.landscape.iter().find(|c| c.section_base == h.data.section_base && c.level == h.data.level && c.actor == h.data.actor) else { continue };
        if rc.verts != h.data.verts {
            continue;
        }
        let n = rc.verts;
        for y in (1..n - 1).step_by(3) {
            for x in (1..n - 1).step_by(3) {
                // the cell centre next to vertex (x, y): avoid the triangle edges
                let p0 = rc.vertex_world(x, y);
                let p1 = rc.vertex_world(x + 1, y + 1);
                let p = [(p0[0] * 3.0 + p1[0]) / 4.0, (p0[1] * 3.0 + p1[1]) / 4.0, p0[2]];
                let surf = w.surface_at(h.body, p);
                *hist.entry(surf).or_default() += 1;
                // the dominant painted layer at the vertex
                let Some(l) = rc.layers.iter().max_by_key(|l| l.weights.get(y * n + x).copied().unwrap_or(0)) else { continue };
                let Some(ls) = layer_surface(&l.info) else { continue };
                total += 1;
                agree += (ls == surf) as usize;
            }
        }
    }
    println!("FL_Camp landscape surfaces {:?}; dominant-layer agreement {agree}/{total}", hist.iter().map(|(k, n)| (NAMES[*k as usize], *n)).collect::<Vec<_>>());
    assert!(total > 100);
    assert!(hist.keys().any(|k| *k != 0));
    assert!(agree as f64 >= 0.8 * total as f64, "{agree}/{total}");
}

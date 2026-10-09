//! The cooked ARecastNavMesh of the maps (mh_nav::detour + cooked; SKIP without the install): the decode matches
//! CUE4Parse's tile sizes (extract/json), and the spawn-pair connectivity the game's own navmesh gives is the oracle the
//! voxel stand-in (lib.rs NavMesh) is compared against.

use mh_level::{read, Pkgs};
use mh_nav::detour::DetourMesh;
use mh_pak::{Reader, Vfs};
use mordhau_core::ue::FVector;
use std::path::PathBuf;
use std::sync::Arc;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn open(map: &str) -> Option<(Pkgs, String)> {
    let vfs = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let want = format!("/{map}.umap").to_lowercase();
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with(&want))?.trim_end_matches(".umap").to_string();
    Some((Pkgs::new(Reader::new(vfs)), pkg))
}

pub fn spawns(d: &mh_level::LevelData) -> Vec<FVector> {
    d.spawns.iter().map(|s| { let p = s.xf.translation(); FVector::new(p[0] as f32, p[1] as f32, p[2] as f32) }).collect()
}

/// FFA_Camp: every tile's sizes == CUE4Parse's SizeInfo (extract/json), the polys / links / spawn connectivity
#[test]
fn camp_cooked_navmesh_decodes() {
    let Some((pk, pkg)) = open("FFA_Camp") else { return };
    let m = mh_nav::cooked::load(&pk, &pkg).expect("FFA_Camp has a RecastNavMesh").expect("decodes");
    let links: usize = m.links.iter().map(|l| l.len()).sum();
    eprintln!("FFA_Camp cooked navmesh: version {}, {} tiles, {} polys, {} links, extent {:?}", m.version, m.tiles.len(), m.poly_count(), links, m.query_extent);
    if let Ok(t) = std::fs::read_to_string(repo().join("extract/json/Mordhau/Content/Mordhau/Maps/DuelCamp/FFA_Camp.json")) {
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        let e = v.as_array().unwrap().iter().find(|e| e["Type"] == "RecastNavMesh").unwrap();
        let tiles: Vec<&serde_json::Value> = e["RecastNavMeshImpl"]["DetourMeshTiles"].as_array().unwrap().iter().filter(|t| !t.is_null()).collect();
        assert_eq!(tiles.len(), m.tiles.len());
        for (a, b) in tiles.iter().zip(&m.tiles) {
            let s = &a["SizeInfo"];
            assert_eq!(s["VertCount"].as_u64().unwrap() as usize, b.verts.len());
            assert_eq!(s["PolyCount"].as_u64().unwrap() as usize, b.polys.len());
            assert_eq!(s["DetailTriCount"].as_u64().unwrap() as usize, b.dtris.len());
            assert_eq!(s["OffMeshConCount"].as_u64().unwrap() as usize, b.offmesh.len());
        }
        eprintln!("== CUE4Parse SizeInfo for all {} tiles", tiles.len());
    }
    assert!(m.poly_count() > 1000 && links > m.poly_count());
}

fn report(m: &DetourMesh, s: &[FVector], pairs: &[(usize, usize)]) -> Vec<bool> {
    let mut out = vec![];
    for &(i, j) in pairs {
        let (a, b) = (s[i], s[j]);
        let pa = m.project(a);
        let pb = m.project(b);
        let ok = match (pa, pb) {
            (Some((x, _)), Some((y, _))) => m.reachable_from(x)[y as usize],
            _ => false,
        };
        eprintln!(
            "  pair {i}->{j}: {} (proj a {:?}, b {:?})",
            if ok { "connected" } else { "NOT connected" },
            pa.map(|p| ((p.1 - a).length() as i32, m.area(p.0))),
            pb.map(|p| ((p.1 - b).length() as i32, m.area(p.0)))
        );
        out.push(ok);
    }
    out
}

/// FFA_Camp spawn pairs (the voxel test's 12 pairs): which the game's navmesh connects, and its paths
#[test]
fn camp_spawn_pairs_on_the_cooked_mesh() {
    let Some((pk, pkg)) = open("FFA_Camp") else { return };
    let d = read(&pk, &pkg);
    let m = mh_nav::cooked::load(&pk, &pkg).unwrap().unwrap();
    let s = spawns(&d);
    let pairs: Vec<(usize, usize)> = (0..s.len().min(12)).map(|i| (i, (i * 7 + 3) % s.len())).collect();
    let ok = report(&m, &s, &pairs);
    let n = ok.iter().filter(|x| **x).count();
    eprintln!("FFA_Camp cooked: {n} of {} spawn pairs connected", ok.len());
    for (k, &(i, j)) in pairs.iter().enumerate() {
        if ok[k] {
            let (p, complete) = m.path(s[i], s[j]).unwrap();
            assert!(complete && !p.is_empty());
        }
    }
    // components of the whole mesh
    let mut comp = vec![u32::MAX; m.poly_count()];
    let mut sizes = vec![];
    for g in 0..m.poly_count() as u32 {
        if comp[g as usize] != u32::MAX {
            continue;
        }
        let r = m.reachable_from(g);
        let c = sizes.len() as u32;
        let mut k = 0;
        for (x, &y) in r.iter().enumerate() {
            if y && comp[x] == u32::MAX {
                comp[x] = c;
                k += 1;
            }
        }
        sizes.push(k);
    }
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    eprintln!("FFA_Camp cooked: {} forward-reachability groups, largest {:?}", sizes.len(), &sizes[..sizes.len().min(8)]);
}

/// diagnostics: link symmetry, unmatched tile-border edges, and where each FFA_Camp spawn projects
#[test]
fn camp_cooked_diagnostics() {
    let Some((pk, pkg)) = open("FFA_Camp") else { return };
    let d = read(&pk, &pkg);
    let m = mh_nav::cooked::load(&pk, &pkg).unwrap().unwrap();
    let (mut one_way, mut ext_edges, mut ext_unmatched) = (0, 0, 0);
    for g in 0..m.poly_count() as u32 {
        for l in &m.links[g as usize] {
            if !m.links[l.to as usize].iter().any(|k| k.to == g) {
                one_way += 1;
            }
        }
        let (ti, pi) = m.poly_tile[g as usize];
        let p = &m.tiles[ti as usize].polys[pi as usize];
        for j in 0..p.vert_count as usize {
            if p.neis[j] & mh_nav::detour::DT_EXT_LINK != 0 {
                ext_edges += 1;
                if !m.links[g as usize].iter().any(|l| l.edge as usize == j) {
                    ext_unmatched += 1;
                }
            }
        }
    }
    let offmesh: usize = m.tiles.iter().map(|t| t.offmesh.len()).sum();
    eprintln!("links one-way {one_way}; tile-border edges {ext_edges}, without a link {ext_unmatched}; off-mesh links {offmesh}");
    let r0 = m.reachable_from(m.project(spawns(&d)[0]).unwrap().0);
    for (i, s) in spawns(&d).iter().enumerate() {
        let (g, p) = m.project(*s).unwrap();
        let below = m.project(FVector::new(s.x, s.y, s.z - 96.0)).unwrap();
        let size = m.reachable_from(g).iter().filter(|x| **x).count();
        eprintln!("spawn {i} {:?}: poly {g} at {:?} (dz {:.0}), area {}, reach {size}, in main {}; below-capsule poly {} in main {}", (s.x as i32, s.y as i32, s.z as i32), (p.x as i32, p.y as i32, p.z as i32), s.z - p.z, m.area(g), r0[g as usize], below.0, r0[below.0 as usize]);
    }
}

/// every mode map's cooked navmesh decodes (version, tiles, polys, links, off-mesh links), parse ending exactly at
/// RecastNavMeshSizeBytes
#[test]
fn every_mode_map_decodes() {
    let Some((pk, _)) = open("FFA_Camp") else { return };
    let maps: Vec<String> = pk.rd.vfs.list().filter(|p| p.ends_with(".umap") && p.contains("/Maps/")).filter(|p| {
        let f = p.rsplit('/').next().unwrap();
        ["FFA_", "TDM_", "SKM_", "DU_", "FL_", "HRD_", "BR_", "INV_"].iter().any(|k| f.starts_with(k))
    }).map(|p| p.trim_end_matches(".umap").to_string()).collect();
    let (mut ok, mut none) = (0, vec![]);
    for m in &maps {
        match mh_nav::cooked::load(&pk, m) {
            Some(Ok(n)) => {
                ok += 1;
                let links: usize = n.links.iter().map(|l| l.len()).sum();
                eprintln!("{}: v{} {} tiles {} polys {} links {} off-mesh", m.rsplit('/').next().unwrap(), n.version, n.tiles.len(), n.poly_count(), links, n.offmesh_links().len());
            }
            Some(Err(e)) => panic!("{m}: {e}"),
            None => none.push(m.rsplit('/').next().unwrap().to_string()),
        }
        pk.clear();
    }
    eprintln!("{ok} of {} mode maps have a cooked navmesh; none in {none:?}", maps.len());
    assert!(ok > 0);
}

/// tool (ignored): copy packages' .uasset / .uexp out of the paks into $MH_RAW_OUT (default target/raw) for
/// scripts/kismet/pp.py; $MH_RAW_PKGS = ;-list of package paths without extension
#[test]
#[ignore]
fn dump_raw_packages() {
    let Ok(vfs) = mh_pak::Vfs::mount_default() else { return eprintln!("SKIP: no paks") };
    let out = std::env::var("MH_RAW_OUT").unwrap_or_else(|_| "target/raw".into());
    for p in std::env::var("MH_RAW_PKGS").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
        for ext in ["uasset", "uexp"] {
            let f = format!("{p}.{ext}");
            let b = vfs.read(&f, false).unwrap_or_else(|| panic!("{f} not in the paks"));
            let dst = std::path::Path::new(&out).join(&f);
            std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
            std::fs::write(&dst, &b[..]).unwrap();
            eprintln!("wrote {} ({} bytes)", dst.display(), b.len());
        }
    }
}

/// HRD_Camp: every BP_HordeSpawn to the player starts on the cooked mesh (complete path? off-mesh links used?)
#[test]
#[ignore]
fn hrd_camp_spawners_reach_players() {
    let Some((pk, pkg)) = open("HRD_Camp") else { return };
    let d = read(&pk, &pkg);
    let m = mh_nav::cooked::load(&pk, &pkg).unwrap().unwrap();
    let starts = spawns(&d);
    let goal = starts[0];
    for a in d.actors.iter().filter(|a| a.class == "BP_HordeSpawn_C") {
        let t = a.xf.as_ref().unwrap().translation();
        let p = FVector::new(t[0] as f32, t[1] as f32, t[2] as f32);
        let pr = m.project(p);
        let (polys, complete) = match pr {
            Some((s, sp)) => {
                let (e, ep) = m.project(goal).unwrap();
                m.find_poly_path(s, e, mh_nav::detour::to_recast(sp), mh_nav::detour::to_recast(ep))
            }
            None => (vec![], false),
        };
        let off: Vec<u32> = polys.iter().copied().filter(|&g| { let (t, q) = m.poly_tile[g as usize]; m.tiles[t as usize].polys[q as usize].ptype == 1 }).collect();
        let links: Vec<String> = off.iter().map(|&g| { let (t, q) = m.poly_tile[g as usize]; let tile = &m.tiles[t as usize]; let o = tile.offmesh.iter().find(|o| o.poly == q).unwrap(); let (a, b) = (mh_nav::detour::to_unreal([o.pos[0], o.pos[1], o.pos[2]]), mh_nav::detour::to_unreal([o.pos[3], o.pos[4], o.pos[5]])); format!("({:.0},{:.0},{:.0})->({:.0},{:.0},{:.0}) area {} flags {}", a.x, a.y, a.z, b.x, b.y, b.z, tile.polys[q as usize].area, o.flags) }).collect();
        if let Some((corners, _)) = m.path(p, goal) {
            let mut prev = m.project(p).unwrap().1;
            let mut blocked = 0;
            for c in &corners {
                if m.raycast_blocked(FVector::new(prev.x, prev.y, prev.z + 50.0), *c) {
                    blocked += 1;
                }
                prev = *c;
            }
            eprintln!("  straight path: {} corners, {blocked} legs blocked by raycast; first {:?}", corners.len(), corners.first());
        }
        eprintln!("{} at {:?}: proj {:?} complete {complete} polys {} offmesh {links:?}", a.name, (p.x as i32, p.y as i32, p.z as i32), pr.map(|x| (x.1 - p).length() as i32), polys.len());
    }
}

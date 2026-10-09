//! mh-nav on a box world (floor, a wall with a door, a raised platform reachable by a ramp of steps) and on real maps
//! (DU_Arena / FFA_Camp collision from the install; SKIP without it).

use mh_character::world::{BoxWorld, Shape, World};
use mh_mode::ai::NavQueries;
use mh_nav::{BuildOptions, Bounds, NavAgent, NavMesh};
use mordhau_core::ue::FVector;

fn bx(min: [f32; 3], max: [f32; 3]) -> Shape {
    Shape::Box { min: FVector::new(min[0], min[1], min[2]), max: FVector::new(max[0], max[1], max[2]) }
}

/// floor 2000 x 1000 at z 0; a wall at x 1000..1040 across y, with a door y 400..600; a box 300 high at x 1400..1800,
/// y 0..300 (not climbable)
fn world() -> BoxWorld {
    let mut w = BoxWorld::default();
    w.shapes.push(bx([0.0, 0.0, -20.0], [2000.0, 1000.0, 0.0]));
    w.shapes.push(bx([1000.0, 0.0, 0.0], [1040.0, 400.0, 400.0]));
    w.shapes.push(bx([1000.0, 600.0, 0.0], [1040.0, 1000.0, 400.0]));
    w.shapes.push(bx([1400.0, 0.0, 0.0], [1800.0, 300.0, 300.0]));
    w
}

fn mesh(w: &dyn World) -> NavMesh {
    NavMesh::build(w, &[Bounds { min: [0.0, 0.0, -100.0], max: [2000.0, 1000.0, 600.0] }], NavAgent::mordhau(), BuildOptions::default())
}

#[test]
fn box_world_paths_through_the_door() {
    let w = world();
    let mut m = mesh(&w);
    assert!(m.nodes.len() > 1000);
    let a = FVector::new(300.0, 200.0, 98.0);
    let b = FVector::new(1700.0, 700.0, 98.0);
    assert!(m.raycast(a, b), "the wall blocks the straight walk");
    let p = m.find_path(a, b).expect("a path through the door");
    let (last, first) = (*p.last().unwrap(), p[0]);
    assert!((last.x - 1700.0).abs() < 30.0 && (last.y - 700.0).abs() < 30.0);
    assert!(p.iter().any(|q| q.x > 900.0 && q.x < 1150.0 && q.y > 400.0 && q.y < 600.0), "through the door: {p:?} {first:?}");
    // the box top (300 cm up, no ramp) is its own region: no path there
    assert!(m.find_path(a, FVector::new(1600.0, 150.0, 398.0)).map(|q| q.last().unwrap().z < 100.0).unwrap_or(true));
    // eroded by the agent radius: no floor within 50 cm of the wall
    assert!(m.nodes.iter().all(|n| !(n.p.x > 960.0 && n.p.x < 1080.0 && (n.p.y < 440.0 || n.p.y > 560.0))), "eroded: no floor within 50 cm of the wall, none on its 40 cm top");
    // random reachable points stay on this side of the wall within the radius
    for _ in 0..50 {
        let q = m.random_reachable_point(a, 400.0).unwrap();
        assert!(((q.x - a.x).powi(2) + (q.y - a.y).powi(2)).sqrt() <= 400.0 + 1.0);
    }
    assert!(!m.raycast(a, FVector::new(600.0, 800.0, 98.0)));
}

// ---- real maps ------------------------------------------------------------------------------------------------------

use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use std::sync::Arc;

fn real(pkg_suffix: &str) -> Option<(mh_level::LevelData, CollisionWorld)> {
    let vfs = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let want = format!("/{pkg_suffix}.umap").to_lowercase();
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with(&want))?.to_string();
    let pkg = pkg.trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs.clone()));
    let d = read(&pk, &pkg);
    let src = mh_assets::pak_source::PakSource::new(vfs);
    let tri = |p: &str| -> Option<Vec<[f64; 3]>> {
        let m = mh_assets::static_mesh::lod0(&src, p).ok()?;
        Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
    };
    let w = CollisionWorld::build(&pk, &d, Some(&tri));
    Some((d, w))
}

/// DU_Arena: the navmesh inside its NavMeshBoundsVolume(s), a path from one spawn to the other along walkable floors
#[test]
fn arena_spawn_to_spawn() {
    let Some((d, w)) = real("DU_Arena") else { return };
    let bounds: Vec<Bounds> = d
        .volumes
        .iter()
        .filter(|v| v.class.contains("NavMeshBoundsVolume"))
        .map(|v| Bounds { min: v.min.map(|c| c as f32), max: v.max.map(|c| c as f32) })
        .collect();
    assert!(!bounds.is_empty(), "DU_Arena has a NavMeshBoundsVolume");
    let t = std::time::Instant::now();
    let m = NavMesh::build(&w, &bounds, NavAgent::mordhau(), BuildOptions { cell: 25.0, max_layers: 4 });
    eprintln!("DU_Arena navmesh: {} floors in {:.2?} over {} x {} cells", m.nodes.len(), t.elapsed(), m.w, m.h);
    let red = d.spawns.iter().find(|s| s.name == "SpawnRed").unwrap().xf.translation();
    let blue = d.spawns.iter().find(|s| s.name == "SpawnBlue").unwrap().xf.translation();
    let (a, b) = (FVector::new(red[0] as f32, red[1] as f32, red[2] as f32), FVector::new(blue[0] as f32, blue[1] as f32, blue[2] as f32));
    let p = m.path(a, b).expect("spawn to spawn");
    let mut prev = a;
    for q in &p {
        // every leg is walkable in a straight line on the mesh (the string-pulled corridor)
        assert!(!m.raycast_blocked(FVector::new(prev.x, prev.y, prev.z + 90.0), *q) || prev == a, "{prev:?} -> {q:?}");
        prev = *q;
    }
    let end = *p.last().unwrap();
    assert!(((end.x - b.x).powi(2) + (end.y - b.y).powi(2)).sqrt() < 150.0);
    walk_with_follower(&w, m, a, b);
}

/// a pawn steered by PathFollower at walking speed reaches the goal with its capsule (radius 34, half height 88:
/// the character's 42 / 96 shrunk for the eroded-but-coarse grid) never inside the collision
fn walk_with_follower(w: &dyn World, mut m: NavMesh, a: FVector, b: FVector) {
    let mut f = mh_nav::PathFollower::default();
    let mut me = m.nodes[m.project(a).unwrap() as usize].p;
    let goal = m.nodes[m.project(b).unwrap() as usize].p;
    let dt = 1.0 / 30.0;
    let mut reached = false;
    for step in 0..30 * 120 {
        let t = f.steer(&mut m, FVector::new(me.x, me.y, me.z + 98.0), goal).expect("a path");
        let (dx, dy) = (t.x - me.x, t.y - me.y);
        let d = (dx * dx + dy * dy).sqrt();
        if ((goal.x - me.x).powi(2) + (goal.y - me.y).powi(2)).sqrt() < 30.0 {
            reached = true;
            eprintln!("reached the goal after {:.1} s", step as f32 * dt);
            break;
        }
        let s = (300.0 * dt).min(d);
        let nx = me.x + dx / d.max(1e-3) * s;
        let ny = me.y + dy / d.max(1e-3) * s;
        // stand on the floor under the new xy (the mesh's floor there)
        let n = m.project(FVector::new(nx, ny, me.z + 98.0)).expect("still on the mesh");
        me = FVector::new(nx, ny, m.nodes[n as usize].p.z);
        assert!(!w.overlap_capsule(FVector::new(me.x, me.y, me.z + 88.0 + 30.0), 34.0, 88.0), "inside the collision at {me:?}");
    }
    assert!(reached, "the follower reaches the goal");
}

/// FFA_Camp: a large outdoor map (landscape + buildings): the build over its nav bounds, a long path, random points
#[test]
fn camp_paths_and_random_points() {
    let Some((d, w)) = real("FFA_Camp") else { return };
    let bounds: Vec<Bounds> = d
        .volumes
        .iter()
        .filter(|v| v.class.contains("NavMeshBoundsVolume"))
        .map(|v| Bounds { min: v.min.map(|c| c as f32), max: v.max.map(|c| c as f32) })
        .collect();
    assert!(!bounds.is_empty());
    let t = std::time::Instant::now();
    let mut m = NavMesh::build(&w, &bounds, NavAgent::mordhau(), BuildOptions { cell: 50.0, max_layers: 4 });
    eprintln!("FFA_Camp navmesh: {} floors in {:.2?} over {} x {} cells (50 cm)", m.nodes.len(), t.elapsed(), m.w, m.h);
    let s: Vec<FVector> = d.spawns.iter().map(|s| { let p = s.xf.translation(); FVector::new(p[0] as f32, p[1] as f32, p[2] as f32) }).collect();
    let mut found = 0;
    for i in 0..s.len().min(12) {
        let (a, b) = (s[i], s[(i * 7 + 3) % s.len()]);
        if let Some(p) = m.path(a, b) {
            found += 1;
            assert!(!p.is_empty());
        }
    }
    eprintln!("FFA_Camp: {found} of {} spawn pairs connected", s.len().min(12));
    assert!(found >= s.len().min(12) / 2, "most spawn pairs are connected");
    let o = s[0];
    for _ in 0..20 {
        let q = m.random_reachable_point(o, 1500.0).expect("a reachable point");
        assert!(((q.x - o.x).powi(2) + (q.y - o.y).powi(2)).sqrt() <= 1501.0);
        assert!(m.path(o, FVector::new(q.x, q.y, q.z + 98.0)).is_some());
    }
}

/// a spawn point dropped to the floor under it (the pawn falls there before it moves): the capsule centre 96 cm above
/// the first hit of a downward trace
fn dropped(w: &dyn World, p: FVector) -> FVector {
    match w.line_trace(p, FVector::new(p.x, p.y, p.z - 2000.0)) {
        Some(h) => FVector::new(p.x, p.y, h.impact_point.z + 96.0),
        None => p,
    }
}

/// FFA_Camp: the voxel stand-in against the game's cooked navmesh on the same spawn pairs (spawns dropped to the
/// floor). For every pair the cooked mesh connects and the voxel one does not, the cooked path is examined: does it
/// use an off-mesh link (NavLink: jump-down / ladder / climb), and where along it the voxel's region ends.
#[test]
fn camp_voxel_vs_cooked() {
    let Some((d, w)) = real("FFA_Camp") else { return };
    let vfs = Arc::new(Vfs::mount_default().unwrap());
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/ffa_camp.umap")).unwrap().trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs));
    let cooked = mh_nav::cooked::load(&pk, &pkg).unwrap().unwrap();
    let bounds: Vec<Bounds> = d.volumes.iter().filter(|v| v.class.contains("NavMeshBoundsVolume")).map(|v| Bounds { min: v.min.map(|c| c as f32), max: v.max.map(|c| c as f32) }).collect();
    let mut opt = BuildOptions::default();
    if let Ok(c) = std::env::var("MH_NAV_CELL") {
        opt.cell = c.parse().unwrap();
    }
    if let Ok(c) = std::env::var("MH_NAV_LAYERS") {
        opt.max_layers = c.parse().unwrap();
    }
    let mut vox = NavMesh::build(&w, &bounds, NavAgent::mordhau(), opt);
    if std::env::var("MH_NAV_NO_LINKS").is_err() {
        let n = vox.add_offmesh_links(&cooked.offmesh_links());
        eprintln!("voxel: {n} of {} off-mesh links attached from the cooked mesh", cooked.offmesh_links().len());
    }
    let s: Vec<FVector> = d.spawns.iter().map(|s| { let p = s.xf.translation(); dropped(&w, FVector::new(p[0] as f32, p[1] as f32, p[2] as f32)) }).collect();
    let (mut nv, mut nc) = (0, 0);
    let n = s.len().min(12);
    for i in 0..n {
        let (a, b) = (s[i], s[(i * 7 + 3) % s.len()]);
        let v = vox.path(a, b).is_some();
        let c = cooked.path(a, b).map(|p| p.1).unwrap_or(false);
        nv += v as usize;
        nc += c as usize;
        let mut why = String::new();
        if c && !v {
            let (sa, pa) = cooked.project(a).unwrap();
            let (sb, pb) = cooked.project(b).unwrap();
            let (polys, _) = cooked.find_poly_path(sa, sb, mh_nav::detour::to_recast(pa), mh_nav::detour::to_recast(pb));
            let off = polys.iter().filter(|&&g| { let (t, p) = cooked.poly_tile[g as usize]; cooked.tiles[t as usize].polys[p as usize].ptype == 1 }).count();
            let ca = vox.project(a).map(|n| vox.nodes[n as usize].comp);
            // the first cooked-path poly whose centre the voxel puts outside a's region
            let mut at = None;
            for &g in &polys {
                let v = cooked.poly_verts(g);
                let c3 = v.iter().fold([0.0f32; 3], |acc, q| [acc[0] + q[0], acc[1] + q[1], acc[2] + q[2]]).map(|x| x / v.len() as f32);
                let u = mh_nav::detour::to_unreal(c3);
                if vox.project(FVector::new(u.x, u.y, u.z + 96.0)).map(|n| vox.nodes[n as usize].comp) != ca {
                    at = Some((g, u, cooked.area(g)));
                    break;
                }
            }
            why = format!(" cooked path {} polys, {off} off-mesh links; voxel region of a ends at {at:?}", polys.len());
        }
        eprintln!("pair {i}: voxel {v} cooked {c}{why}");
    }
    eprintln!("FFA_Camp (dropped spawns): voxel {nv} / cooked {nc} of {n} pairs");
    // pair 11: spawn 11 drops onto an object (hit z 1680, normal z 0.99) whose top is not navmesh: the ground under it
    // is carved (no poly under the spawn) and the nearest poly is a one-poly island on the object (poly 2728). The
    // game's own mesh does not connect it; every other pair is connected.
    assert_eq!(nc, n - 1, "the game's navmesh connects every pair but the one from spawn 11");
    assert!(!cooked.path(s[11], s[(11 * 7 + 3) % s.len()]).map(|p| p.1).unwrap_or(false));
    assert!(nv >= 9, "the voxel stand-in with the map's off-mesh links connects most of them");
}

/// diagnostics around given FFA_Camp points: the floors of the voxel mesh vs the cooked polys, and spawn 11's drop
#[test]
#[ignore]
fn camp_probe() {
    let Some((d, w)) = real("FFA_Camp") else { return };
    let vfs = Arc::new(Vfs::mount_default().unwrap());
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/ffa_camp.umap")).unwrap().trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs));
    let cooked = mh_nav::cooked::load(&pk, &pkg).unwrap().unwrap();
    let bounds: Vec<Bounds> = d.volumes.iter().filter(|v| v.class.contains("NavMeshBoundsVolume")).map(|v| Bounds { min: v.min.map(|c| c as f32), max: v.max.map(|c| c as f32) }).collect();
    let vox = NavMesh::build(&w, &bounds, NavAgent::mordhau(), BuildOptions { cell: 50.0, max_layers: 4 });
    let sp = d.spawns[11].xf.translation();
    let s11 = FVector::new(sp[0] as f32, sp[1] as f32, sp[2] as f32);
    let h = w.line_trace(s11, FVector::new(s11.x, s11.y, s11.z - 2000.0));
    eprintln!("spawn 11 {s11:?} drop hit {:?}", h.map(|h| (h.impact_point, h.impact_normal)));
    let dr = dropped(&w, s11);
    let (g, p) = cooked.project(dr).unwrap();
    eprintln!("  dropped {dr:?} -> poly {g} at {p:?}, group {}", cooked.reachable_from(g).iter().filter(|x| **x).count());
    for z in [0.0, 50.0, 100.0, 150.0, 200.0, 300.0] {
        let q = FVector::new(dr.x, dr.y, dr.z - 96.0 + z);
        let (g, p) = cooked.project(q).unwrap();
        eprintln!("  probe z+{z}: poly {g} at {p:?} group {}", cooked.reachable_from(g).iter().filter(|x| **x).count());
    }
    for (x, y) in [(667.0f32, -2374.0f32), (-1932.0, -148.0)] {
        eprintln!("around ({x}, {y}):");
        for dy in [-100.0f32, -50.0, 0.0, 50.0, 100.0] {
            let mut line = String::new();
            for dx in [-100.0f32, -50.0, 0.0, 50.0, 100.0] {
                let q = FVector::new(x + dx, y + dy, 1500.0);
                let fl: Vec<String> = vox.nodes.iter().filter(|n| (n.p.x - q.x).abs() < 25.0 && (n.p.y - q.y).abs() < 25.0).map(|n| format!("{:.0}c{}", n.p.z, n.comp)).collect();
                let c = cooked.project(FVector::new(q.x, q.y, 1400.0)).map(|(_, p)| p.z as i32).unwrap_or(-1);
                line += &format!(" [{} | {c}]", fl.join(","));
            }
            eprintln!("  {line}");
        }
    }
}

/// the off-mesh links the cooked paths of pairs 8 / 9 use, and the voxel floors around their ends
#[test]
#[ignore]
fn camp_probe_links() {
    let Some((d, w)) = real("FFA_Camp") else { return };
    let vfs = Arc::new(Vfs::mount_default().unwrap());
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/ffa_camp.umap")).unwrap().trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs));
    let cooked = mh_nav::cooked::load(&pk, &pkg).unwrap().unwrap();
    let bounds: Vec<Bounds> = d.volumes.iter().filter(|v| v.class.contains("NavMeshBoundsVolume")).map(|v| Bounds { min: v.min.map(|c| c as f32), max: v.max.map(|c| c as f32) }).collect();
    let vox = NavMesh::build(&w, &bounds, NavAgent::mordhau(), BuildOptions { cell: 25.0, max_layers: 12 });
    let s: Vec<FVector> = d.spawns.iter().map(|s| { let p = s.xf.translation(); dropped(&w, FVector::new(p[0] as f32, p[1] as f32, p[2] as f32)) }).collect();
    for i in [8usize, 9] {
        let (a, b) = (s[i], s[(i * 7 + 3) % s.len()]);
        let (sa, pa) = cooked.project(a).unwrap();
        let (sb, pb) = cooked.project(b).unwrap();
        let (polys, _) = cooked.find_poly_path(sa, sb, mh_nav::detour::to_recast(pa), mh_nav::detour::to_recast(pb));
        for &g in &polys {
            let (t, p) = cooked.poly_tile[g as usize];
            let tile = &cooked.tiles[t as usize];
            if tile.polys[p as usize].ptype != 1 {
                continue;
            }
            let o = tile.offmesh.iter().find(|o| o.poly == p).unwrap();
            let la = mh_nav::detour::to_unreal([o.pos[0], o.pos[1], o.pos[2]]);
            let lb = mh_nav::detour::to_unreal([o.pos[3], o.pos[4], o.pos[5]]);
            eprintln!("pair {i}: link {la:?} -> {lb:?} rad {} flags {} height {}", o.rad, o.flags, o.height);
            for (nm, e) in [("a", la), ("b", lb)] {
                let near: Vec<String> = vox.nodes.iter().filter(|n| ((n.p.x - e.x).powi(2) + (n.p.y - e.y).powi(2)).sqrt() < 150.0).map(|n| format!("({:.0},{:.0},{:.0})c{}", n.p.x - e.x, n.p.y - e.y, n.p.z - e.z, n.comp)).take(30).collect();
                eprintln!("  end {nm}: voxel floors within 150: {}", near.join(" "));
            }
        }
    }
}

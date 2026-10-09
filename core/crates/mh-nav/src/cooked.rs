//! Finding a map's cooked ARecastNavMesh in its packages (feature "level": mh-level / mh-pak). The persistent level
//! and every streaming sub-level (mh_level::levels) are searched for an export of class RecastNavMesh; its native
//! data (after the tagged properties: mh_level::native::after_props) is decoded by detour::DetourMesh::parse, and its
//! NavDataConfig.DefaultQueryExtent (tagged property) becomes the projection extent.

use crate::detour::DetourMesh;
use mh_level::Pkgs;
use mordhau_core::ue::FVector;

/// (package, export index) of every RecastNavMesh export among the map's levels
pub fn find_all(pk: &Pkgs, map_pkg: &str) -> Vec<(String, usize)> {
    let mut out = vec![];
    for l in mh_level::levels(pk, map_pkg) {
        let ex = pk.load_pkg(&l.pkg);
        for (i, e) in ex.iter().enumerate() {
            if e.get("Type").and_then(|t| t.as_str()) == Some("RecastNavMesh") {
                out.push((l.pkg.clone(), i));
            }
        }
    }
    out
}

/// the first RecastNavMesh export among the map's levels
pub fn find(pk: &Pkgs, map_pkg: &str) -> Option<(String, usize)> {
    find_all(pk, map_pkg).into_iter().next()
}

/// decode one RecastNavMesh export
pub fn load_export(pk: &Pkgs, pkg: &str, i: usize) -> Option<Result<DetourMesh, String>> {
    let a = pk.rd.open(pkg)?;
    let (r, end) = mh_level::native::after_props(pk, &a, i)?;
    let bytes = r.at(r.p, (end - r.p) as usize)?.into_owned();
    Some(DetourMesh::parse(&bytes).map(|mut m| {
        let ex = pk.load_pkg(pkg);
        let props = pk.props(&ex[i]);
        if let Some(e) = props.get("NavDataConfig").and_then(|c| c.get("DefaultQueryExtent")) {
            let f = |k: &str| e.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
            if let (Some(x), Some(y), Some(z)) = (f("X"), f("Y"), f("Z")) {
                m.query_extent = FVector::new(x, y, z);
            }
        }
        m
    }))
}

/// the cooked navmesh of a map: of its RecastNavMesh exports (persistent level and sub-levels; a map can hold an
/// empty one beside the built one) the one with the most polys. None: no export; Err: undecodable
pub fn load(pk: &Pkgs, map_pkg: &str) -> Option<Result<DetourMesh, String>> {
    let mut best: Option<Result<DetourMesh, String>> = None;
    for (pkg, i) in find_all(pk, map_pkg) {
        let Some(r) = load_export(pk, &pkg, i) else { continue };
        let better = match (&best, &r) {
            (None, _) => true,
            (Some(Err(_)), Ok(_)) => true,
            (Some(Ok(b)), Ok(m)) => m.poly_count() > b.poly_count(),
            _ => false,
        };
        if better {
            best = Some(r);
        }
    }
    best
}

/// the bots' navigation for a map: its cooked navmesh when the packages hold one (the game's own), else the voxel
/// stand-in built over `world` inside `bounds`, with the cooked mesh's off-mesh links when only those decode
pub fn nav_for_map(pk: &Pkgs, map_pkg: &str, world: &dyn mh_character::world::World, bounds: &[crate::Bounds]) -> Box<dyn mh_mode::ai::NavQueries> {
    match load(pk, map_pkg) {
        Some(Ok(m)) if m.poly_count() > 0 => Box::new(m),
        _ => Box::new(crate::NavMesh::build(world, bounds, crate::NavAgent::mordhau(), crate::BuildOptions::default())),
    }
}

//! The game's own navmesh: the cooked ARecastNavMesh of a map package (rust-mode-ai r5), decoded and queried the way
//! Detour does. This is the real oracle the voxel stand-in (lib.rs NavMesh) approximates.
//!
//! Serialization (UE 4.26, NavMeshVersion 13 = NAVMESHVER_LANDSCAPE_HEIGHT on the Mordhau maps), read after the actor's
//! tagged properties, layout as CUE4Parse decodes it at the pinned commit (tools/CUE4Parse-src/CUE4Parse/UE4/Assets/
//! Exports/NavigationSystem: ARecastNavMesh.cs:23-46, FPImplRecastNavMesh.cs:22-79, URecastNavMeshDataChunk.cs:60-68,
//! FDetourTileSizeInfo.cs:22-50, Detour/DetourMeshTile.cs:20-60, DetourMeshHeader.cs:66-98, DetourPoly.cs:18-27,
//! DetourPolyDetail.cs:16-28, DetourBVNode.cs:11-16, DetourOffMeshConnection.cs:31-43, DetourOffMeshSegmentConnection.cs,
//! DetourTileCacheLayerHeader.cs):
//!   u32 NavMeshVersion, u32 RecastNavMeshSizeBytes (from its own offset), i32 NumTiles,
//!   dtNavMeshParams {f32 orig[3], tileWidth, tileHeight, i32 maxTiles, maxPolys},
//!   per tile: u64 TileRef (MAX = empty), i32 TileDataSize (<= 0: nothing else), 10 x i32 sizes, dtMeshHeader,
//!   verts (f32 x3, world space, Recast axes), dtPoly (u32 firstLink, u16 verts[6], u16 neis[6], u16 flags,
//!   u8 vertCount, u8 areaAndType), dtPolyDetail (u32 vertBase, u32 triBase, u8 vertCount, u8 triCount), detail verts,
//!   detail tris (u8 x4), dtBVNode (u16 x6 + i32), dtOffMeshConnection (f32 pos[6], f32 rad, u16 poly, u8 flags,
//!   u8 side, u32 userId) then one f32 height each (>= NAVMESHVER_OFFMESH_HEIGHT_BUG), segment links, clusters (f32 x3),
//!   u16 polyClusters[offMeshBase], then the compressed tile-cache layer (i32 size incl. its 60-byte header, skipped).
//! The parse must end exactly at RecastNavMeshSizeBytes (checked).
//!
//! Recast axes: Unreal (x, y, z) = Recast (-x, -z, y) (RecastHelpers.h Unreal2RecastPoint / Recast2UnrealPoint).
//!
//! Queries are the Recast/Detour library algorithms (public source; UE 4.26 ThirdParty/Recast Detour/DetourNavMesh.cpp,
//! DetourNavMeshQuery.cpp): tile linking (connectIntLinks, connectExtLinks + findConnectingPolys / overlapSlabs,
//! baseOffMeshLinks, connectExtOffMeshLinks), findNearestPoly, findPath (A*, edge-midpoint nodes, H_SCALE 0.999),
//! findStraightPath (funnel), raycast, findRandomPointAroundCircle + dtRandomPointInConvexPoly. UE's fork of them is
//! engine code that is not disassembled here: equality with the exe's results is UNCONFIRMED (the structure is the
//! game's data; the per-area query filter costs of FRecastQueryFilter are taken as 1, UNCONFIRMED).

use mordhau_core::ue::{CrtRand, FVector};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

pub const DT_EXT_LINK: u16 = 0x8000;
pub const DT_OFFMESH_CON_BIDIR: u8 = 1;
pub const POLYTYPE_GROUND: u8 = 0;
pub const POLYTYPE_OFFMESH_POINT: u8 = 1;
const H_SCALE: f32 = 0.999;

#[derive(Clone, Debug, Default)]
pub struct Poly {
    pub verts: [u16; 6],
    pub neis: [u16; 6],
    pub flags: u16,
    pub vert_count: u8,
    pub area: u8,
    pub ptype: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Detail {
    pub vert_base: u32,
    pub tri_base: u32,
    pub vert_count: u8,
    pub tri_count: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OffMesh {
    pub pos: [f32; 6],
    pub rad: f32,
    pub height: f32,
    pub poly: u16,
    pub flags: u8,
    pub side: u8,
    pub user_id: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub layer: i32,
    pub walkable_height: f32,
    pub walkable_radius: f32,
    pub walkable_climb: f32,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub offmesh_base: i32,
    pub verts: Vec<[f32; 3]>,
    pub polys: Vec<Poly>,
    pub details: Vec<Detail>,
    pub dverts: Vec<[f32; 3]>,
    pub dtris: Vec<[u8; 4]>,
    pub offmesh: Vec<OffMesh>,
    /// global index of polys[0]
    pub base: u32,
}

/// a link: from a poly to `to` (global poly index) across edge `edge` (0xff: off-mesh), `side` 0xff = inside the
/// tile, else the neighbour side; `bmin` / `bmax` the portal's part of the edge (x255)
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub to: u32,
    pub edge: u8,
    pub side: u8,
    pub bmin: u8,
    pub bmax: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Params {
    pub orig: [f32; 3],
    pub tile_width: f32,
    pub tile_height: f32,
    pub max_tiles: i32,
    pub max_polys: i32,
}

pub struct DetourMesh {
    pub version: u32,
    pub params: Params,
    pub tiles: Vec<Tile>,
    /// global poly -> (tile, poly)
    pub poly_tile: Vec<(u32, u16)>,
    pub links: Vec<Vec<Link>>,
    /// polys whose area is excluded by the query filter (default: NavArea_Null = 0)
    pub excluded_areas: Vec<u8>,
    /// per-area traversal cost (FRecastQueryFilter: default 1 per area, UNCONFIRMED per Mordhau's NavArea classes)
    pub area_cost: [f32; 64],
    /// NavDataConfig.DefaultQueryExtent (Unreal cm; FFA_Camp RecastNavMesh-Default: 500, 500, 500)
    pub query_extent: FVector,
    /// FMath::FRand's stream for findRandomPointAroundCircle (UE: the global CRT rand(); here the mesh's own)
    pub rng: CrtRand,
}

// ---- reader ----------------------------------------------------------------------------------------------------------

struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        if self.p + n > self.b.len() {
            return Err(format!("detour: read past the end at {} (+{n} of {})", self.p, self.b.len()));
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn v3(&mut self) -> Result<[f32; 3], String> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    fn count(&mut self, what: &str) -> Result<usize, String> {
        let n = self.i32()?;
        if !(0..=1 << 20).contains(&n) {
            return Err(format!("detour: bad {what} count {n} at {}", self.p - 4));
        }
        Ok(n as usize)
    }
}

/// Unreal cm -> Recast axes
pub fn to_recast(v: FVector) -> [f32; 3] {
    [-v.x, v.z, -v.y]
}

/// Recast axes -> Unreal cm
pub fn to_unreal(v: [f32; 3]) -> FVector {
    FVector::new(-v[0], -v[2], v[1])
}

impl DetourMesh {
    /// decode the ARecastNavMesh native data: `b` starts right after the actor's tagged properties (+ UObject guid)
    pub fn parse(b: &[u8]) -> Result<DetourMesh, String> {
        let mut r = R { b, p: 0 };
        let version = r.u32()?;
        if !(11..=13).contains(&version) {
            return Err(format!("detour: NavMeshVersion {version} (decoder covers 11..=13, UE 4.x)"));
        }
        let size_pos = r.p;
        let size = r.u32()? as usize;
        let num_tiles = r.count("tile")?;
        let params = Params { orig: r.v3()?, tile_width: r.f32()?, tile_height: r.f32()?, max_tiles: r.i32()?, max_polys: r.i32()? };
        let mut tiles = Vec::new();
        for _ in 0..num_tiles {
            let tile_ref = r.u64()?;
            if tile_ref == u64::MAX {
                continue;
            }
            let data_size = r.i32()?;
            if data_size <= 0 {
                continue;
            }
            let mut c = [0usize; 10];
            for (k, x) in c.iter_mut().enumerate() {
                *x = r.count(["vert", "poly", "maxlink", "detailmesh", "detailvert", "detailtri", "bvnode", "offmeshcon", "offmeshsegcon", "cluster"][k])?;
            }
            let [nv, np, _nl, ndm, ndv, ndt, nbv, noff, nseg, ncl] = c;
            let _magic = r.u32()?;
            let _ver = r.i32()?;
            let (x, y, layer) = (r.i32()?, r.i32()?, r.i32()?);
            let _user = r.u32()?;
            let mut h = [0i32; 9];
            for v in h.iter_mut() {
                *v = r.i32()?;
            }
            let offmesh_base = h[8];
            let (wh, wr, wc) = (r.f32()?, r.f32()?, r.f32()?);
            let (bmin, bmax) = (r.v3()?, r.v3()?);
            let _bvq = r.f32()?;
            let _clusters = r.i32()?;
            let (_sc, _sp, _sv) = (r.i32()?, r.i32()?, r.i32()?);
            let mut t = Tile { x, y, layer, walkable_height: wh, walkable_radius: wr, walkable_climb: wc, bmin, bmax, offmesh_base, ..Default::default() };
            for _ in 0..nv {
                t.verts.push(r.v3()?);
            }
            for _ in 0..np {
                let _first_link = r.u32()?;
                let mut p = Poly::default();
                for v in p.verts.iter_mut() {
                    *v = r.u16()?;
                }
                for v in p.neis.iter_mut() {
                    *v = r.u16()?;
                }
                p.flags = r.u16()?;
                p.vert_count = r.u8()?;
                let at = r.u8()?;
                p.area = at & 0x3f;
                p.ptype = at >> 6;
                t.polys.push(p);
            }
            for _ in 0..ndm {
                t.details.push(Detail { vert_base: r.u32()?, tri_base: r.u32()?, vert_count: r.u8()?, tri_count: r.u8()? });
            }
            for _ in 0..ndv {
                t.dverts.push(r.v3()?);
            }
            for _ in 0..ndt {
                t.dtris.push([r.u8()?, r.u8()?, r.u8()?, r.u8()?]);
            }
            r.take(nbv * 16)?;
            for _ in 0..noff {
                let mut o = OffMesh::default();
                for v in o.pos.iter_mut() {
                    *v = r.f32()?;
                }
                o.rad = r.f32()?;
                o.poly = r.u16()?;
                o.flags = r.u8()?;
                o.side = r.u8()?;
                o.user_id = r.u32()?;
                t.offmesh.push(o);
            }
            if version >= 11 {
                for o in t.offmesh.iter_mut() {
                    o.height = r.f32()?;
                }
            }
            r.take(nseg * (12 * 4 + 4 + 2 + 1 + 1 + 4))?;
            r.take(ncl * 12)?;
            // polyClusters: one u16 per ground poly (CUE4Parse: SizeInfo.OffMeshBase = DetailMeshCount)
            r.take(ndm * 2)?;
            let n = r.i32()?;
            if n > 0 {
                r.take(n as usize)?;
            }
            tiles.push(t);
        }
        if r.p != size_pos + size {
            return Err(format!("detour: parsed to {} but RecastNavMeshSizeBytes ends at {}", r.p, size_pos + size));
        }
        let mut m = DetourMesh {
            version,
            params,
            tiles,
            poly_tile: Vec::new(),
            links: Vec::new(),
            excluded_areas: vec![0],
            area_cost: [1.0; 64],
            query_extent: FVector::new(500.0, 500.0, 500.0),
            rng: CrtRand::new(1),
        };
        m.link_all();
        Ok(m)
    }

    pub fn poly_count(&self) -> usize {
        self.poly_tile.len()
    }

    fn tile_poly(&self, g: u32) -> (&Tile, &Poly) {
        let (t, p) = self.poly_tile[g as usize];
        let t = &self.tiles[t as usize];
        (t, &t.polys[p as usize])
    }

    pub fn poly_verts(&self, g: u32) -> Vec<[f32; 3]> {
        let (t, p) = self.tile_poly(g);
        (0..p.vert_count as usize).map(|k| t.verts[p.verts[k] as usize]).collect()
    }

    pub fn area(&self, g: u32) -> u8 {
        self.tile_poly(g).1.area
    }

    fn pass(&self, g: u32) -> bool {
        !self.excluded_areas.contains(&self.area(g))
    }

    // ---- linking (dtNavMesh::addTile for every tile) --------------------------------------------------------------

    fn link_all(&mut self) {
        let mut n = 0u32;
        for (ti, t) in self.tiles.iter_mut().enumerate() {
            t.base = n;
            for pi in 0..t.polys.len() {
                self.poly_tile.push((ti as u32, pi as u16));
            }
            n += t.polys.len() as u32;
        }
        self.links = vec![Vec::new(); n as usize];
        let mut at: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (ti, t) in self.tiles.iter().enumerate() {
            at.entry((t.x, t.y)).or_default().push(ti);
        }
        for ti in 0..self.tiles.len() {
            self.connect_int_links(ti);
            self.base_offmesh_links(ti);
        }
        for ti in 0..self.tiles.len() {
            let (x, y) = (self.tiles[ti].x, self.tiles[ti].y);
            self.connect_ext_offmesh_links(ti, ti, -1);
            for &nj in at.get(&(x, y)).into_iter().flatten() {
                if nj != ti {
                    self.connect_ext_links(ti, nj, -1);
                    self.connect_ext_offmesh_links(ti, nj, -1);
                }
            }
            for side in 0..8 {
                let (dx, dy) = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)][side];
                for &nj in at.get(&(x + dx, y + dy)).into_iter().flatten() {
                    self.connect_ext_links(ti, nj, side as i32);
                    // connectExtOffMeshLinks(tile, target, side): `tile` holds the end points of `target`'s links
                    self.connect_ext_offmesh_links(ti, nj, side as i32);
                }
            }
        }
    }

    fn connect_int_links(&mut self, ti: usize) {
        let t = &self.tiles[ti];
        for (pi, p) in t.polys.iter().enumerate() {
            if p.ptype != POLYTYPE_GROUND {
                continue;
            }
            for j in 0..p.vert_count as usize {
                let nei = p.neis[j];
                if nei == 0 || nei & DT_EXT_LINK != 0 {
                    continue;
                }
                self.links[(t.base + pi as u32) as usize].push(Link { to: t.base + nei as u32 - 1, edge: j as u8, side: 0xff, bmin: 0, bmax: 0 });
            }
        }
    }

    fn base_offmesh_links(&mut self, ti: usize) {
        let mut add = Vec::new();
        {
            let t = &self.tiles[ti];
            for o in &t.offmesh {
                let p = [o.pos[0], o.pos[1], o.pos[2]];
                let ext = [o.rad, t.walkable_climb.max(o.height), o.rad];
                let Some((r, near)) = self.nearest_in_tile(ti, p, ext) else { continue };
                if dist2d_sq(near, p) > o.rad * o.rad {
                    continue;
                }
                let op = t.base + o.poly as u32;
                add.push((op, Link { to: r, edge: 0, side: 0xff, bmin: 0, bmax: 0 }));
                add.push((r, Link { to: op, edge: 0xff, side: 0xff, bmin: 0, bmax: 0 }));
            }
        }
        for (a, l) in add {
            self.links[a as usize].push(l);
        }
    }

    /// the end points of `target`'s off-mesh links that land in tile `ti`
    fn connect_ext_offmesh_links(&mut self, ti: usize, target: usize, side: i32) {
        let opposite: u8 = if side == -1 { 0xff } else { ((side + 4) & 7) as u8 };
        let mut add = Vec::new();
        {
            let tt = &self.tiles[target];
            for o in &tt.offmesh {
                if o.side != opposite {
                    continue;
                }
                let op = tt.base + o.poly as u32;
                if self.links[op as usize].is_empty() {
                    continue; // the start is not connected
                }
                let ext = [o.rad, tt.walkable_climb.max(o.height), o.rad];
                let p = [o.pos[3], o.pos[4], o.pos[5]];
                let Some((r, near)) = self.nearest_in_tile(ti, p, ext) else { continue };
                if dist2d_sq(near, p) > o.rad * o.rad {
                    continue;
                }
                add.push((op, Link { to: r, edge: 1, side: opposite, bmin: 0, bmax: 0 }));
                if o.flags & DT_OFFMESH_CON_BIDIR != 0 {
                    add.push((r, Link { to: op, edge: 0xff, side: if side == -1 { 0xff } else { side as u8 }, bmin: 0, bmax: 0 }));
                }
            }
        }
        for (a, l) in add {
            self.links[a as usize].push(l);
        }
    }

    fn connect_ext_links(&mut self, ti: usize, target: usize, side: i32) {
        let mut add = Vec::new();
        {
            let t = &self.tiles[ti];
            for (pi, p) in t.polys.iter().enumerate() {
                if p.ptype != POLYTYPE_GROUND {
                    continue;
                }
                let nv = p.vert_count as usize;
                for j in 0..nv {
                    if p.neis[j] & DT_EXT_LINK == 0 {
                        continue;
                    }
                    let dir = (p.neis[j] & 0xff) as i32;
                    if side != -1 && dir != side {
                        continue;
                    }
                    let va = t.verts[p.verts[j] as usize];
                    let vb = t.verts[p.verts[(j + 1) % nv] as usize];
                    for (to, lo, hi) in self.find_connecting_polys(va, vb, target, (dir + 4) & 7) {
                        let (mut tmin, mut tmax) = if dir == 0 || dir == 4 {
                            ((lo - va[2]) / (vb[2] - va[2]), (hi - va[2]) / (vb[2] - va[2]))
                        } else {
                            ((lo - va[0]) / (vb[0] - va[0]), (hi - va[0]) / (vb[0] - va[0]))
                        };
                        if tmin > tmax {
                            std::mem::swap(&mut tmin, &mut tmax);
                        }
                        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
                        add.push((t.base + pi as u32, Link { to, edge: j as u8, side: dir as u8, bmin: q(tmin), bmax: q(tmax) }));
                    }
                }
            }
        }
        for (a, l) in add {
            self.links[a as usize].push(l);
        }
    }

    fn find_connecting_polys(&self, va: [f32; 3], vb: [f32; 3], target: usize, side: i32) -> Vec<(u32, f32, f32)> {
        let t = &self.tiles[target];
        let (amin, amax) = slab_end_points(va, vb, side);
        let apos = slab_coord(va, side);
        let m = DT_EXT_LINK | side as u16;
        let mut out = Vec::new();
        for (pi, p) in t.polys.iter().enumerate() {
            let nv = p.vert_count as usize;
            for j in 0..nv {
                if p.neis[j] != m {
                    continue;
                }
                let vc = t.verts[p.verts[j] as usize];
                let vd = t.verts[p.verts[(j + 1) % nv] as usize];
                if (apos - slab_coord(vc, side)).abs() > 0.01 {
                    continue;
                }
                let (bmin, bmax) = slab_end_points(vc, vd, side);
                if !overlap_slabs(amin, amax, bmin, bmax, 0.01, t.walkable_climb) {
                    continue;
                }
                out.push((t.base + pi as u32, amin[0].max(bmin[0]), amax[0].min(bmax[0])));
                break;
            }
        }
        out
    }

    // ---- geometry ------------------------------------------------------------------------------------------------

    /// the poly's height at `p` from its detail triangles (dtNavMeshQuery::getPolyHeight)
    pub fn poly_height(&self, g: u32, p: [f32; 3]) -> Option<f32> {
        let (t, poly) = self.tile_poly(g);
        if poly.ptype == POLYTYPE_OFFMESH_POINT {
            let v0 = t.verts[poly.verts[0] as usize];
            let v1 = t.verts[poly.verts[1] as usize];
            let d0 = dist2d_sq(p, v0).sqrt();
            let d1 = dist2d_sq(p, v1).sqrt();
            let u = d0 / (d0 + d1).max(1e-6);
            return Some(v0[1] + (v1[1] - v0[1]) * u);
        }
        let pi = self.poly_tile[g as usize].1 as usize;
        let d = t.details.get(pi)?;
        for j in 0..d.tri_count as u32 {
            let tri = t.dtris[(d.tri_base + j) as usize];
            let v: Vec<[f32; 3]> = (0..3)
                .map(|k| {
                    let i = tri[k] as usize;
                    if i < poly.vert_count as usize {
                        t.verts[poly.verts[i] as usize]
                    } else {
                        t.dverts[d.vert_base as usize + i - poly.vert_count as usize]
                    }
                })
                .collect();
            if let Some(h) = closest_height_point_triangle(p, v[0], v[1], v[2]) {
                return Some(h);
            }
        }
        None
    }

    /// dtNavMeshQuery::closestPointOnPoly: inside (2D) -> the detail height there, else the nearest boundary point
    pub fn closest_point_on_poly(&self, g: u32, p: [f32; 3]) -> ([f32; 3], bool) {
        let v = self.poly_verts(g);
        if point_in_poly(p, &v) {
            if let Some(h) = self.poly_height(g, p) {
                return ([p[0], h, p[2]], true);
            }
        }
        (closest_on_boundary(p, &v), false)
    }

    fn nearest_in_tile(&self, ti: usize, c: [f32; 3], ext: [f32; 3]) -> Option<(u32, [f32; 3])> {
        let t = &self.tiles[ti];
        let (qmin, qmax) = ([c[0] - ext[0], c[1] - ext[1], c[2] - ext[2]], [c[0] + ext[0], c[1] + ext[1], c[2] + ext[2]]);
        let mut best: Option<(f32, u32, [f32; 3])> = None;
        for (pi, p) in t.polys.iter().enumerate() {
            if p.ptype != POLYTYPE_GROUND {
                continue;
            }
            let g = t.base + pi as u32;
            if !poly_overlaps(&self.poly_verts(g), qmin, qmax) {
                continue;
            }
            let (q, over) = self.closest_point_on_poly(g, c);
            let d = nearest_metric(c, q, over, t.walkable_climb);
            if best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, g, q));
            }
        }
        best.map(|b| (b.1, b.2))
    }

    /// dtNavMeshQuery::findNearestPoly (Recast axes, half extents): the walkable poly nearest to `c` and the point on it
    pub fn find_nearest_poly(&self, c: [f32; 3], ext: [f32; 3]) -> Option<(u32, [f32; 3])> {
        let mut best: Option<(f32, u32, [f32; 3])> = None;
        let (qmin, qmax) = ([c[0] - ext[0], c[1] - ext[1], c[2] - ext[2]], [c[0] + ext[0], c[1] + ext[1], c[2] + ext[2]]);
        for (ti, t) in self.tiles.iter().enumerate() {
            if t.bmax[0] < qmin[0] || t.bmin[0] > qmax[0] || t.bmax[2] < qmin[2] || t.bmin[2] > qmax[2] {
                continue;
            }
            let _ = ti;
            for (pi, p) in t.polys.iter().enumerate() {
                let g = t.base + pi as u32;
                if p.ptype != POLYTYPE_GROUND || !self.pass(g) {
                    continue;
                }
                if !poly_overlaps(&self.poly_verts(g), qmin, qmax) {
                    continue;
                }
                let (q, over) = self.closest_point_on_poly(g, c);
                let d = nearest_metric(c, q, over, t.walkable_climb);
                if best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, g, q));
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }

    /// ProjectPointToNavigation with the navmesh's DefaultQueryExtent: (poly, point in Unreal cm)
    pub fn project(&self, p: FVector) -> Option<(u32, FVector)> {
        let e = self.query_extent;
        self.find_nearest_poly(to_recast(p), [e.x, e.z, e.y]).map(|(g, q)| (g, to_unreal(q)))
    }

    /// dtNavMeshQuery::getPortalPoints (from -> to): (left, right)
    fn portal(&self, from: u32, to: u32) -> Option<([f32; 3], [f32; 3])> {
        let l = *self.links[from as usize].iter().find(|l| l.to == to)?;
        let (ft, fp) = self.tile_poly(from);
        if fp.ptype == POLYTYPE_OFFMESH_POINT {
            let v = ft.verts[fp.verts[l.edge as usize % 2] as usize];
            return Some((v, v));
        }
        let (tt, tp) = self.tile_poly(to);
        if tp.ptype == POLYTYPE_OFFMESH_POINT {
            let back = self.links[to as usize].iter().find(|k| k.to == from)?;
            let v = tt.verts[tp.verts[back.edge as usize % 2] as usize];
            return Some((v, v));
        }
        let nv = fp.vert_count as usize;
        let v0 = ft.verts[fp.verts[l.edge as usize] as usize];
        let v1 = ft.verts[fp.verts[(l.edge as usize + 1) % nv] as usize];
        if l.side != 0xff && (l.bmin != 0 || l.bmax != 255) {
            let s = 1.0 / 255.0;
            return Some((lerp(v0, v1, l.bmin as f32 * s), lerp(v0, v1, l.bmax as f32 * s)));
        }
        Some((v0, v1))
    }

    fn edge_mid(&self, from: u32, to: u32) -> [f32; 3] {
        match self.portal(from, to) {
            Some((a, b)) => lerp(a, b, 0.5),
            None => [0.0; 3],
        }
    }

    fn cost(&self, a: [f32; 3], b: [f32; 3], poly: u32) -> f32 {
        dist(a, b) * self.area_cost[self.area(poly) as usize]
    }

    /// dtNavMeshQuery::findPath: (polys, complete). Not complete = the partial path to the node nearest the goal.
    pub fn find_poly_path(&self, s: u32, e: u32, sp: [f32; 3], ep: [f32; 3]) -> (Vec<u32>, bool) {
        if s == e {
            return (vec![s], true);
        }
        #[derive(Clone, Copy)]
        struct N {
            pos: [f32; 3],
            cost: f32,
            total: f32,
            parent: u32,
            open: bool,
            closed: bool,
        }
        let mut nodes: HashMap<u32, N> = HashMap::new();
        let h0 = dist(sp, ep) * H_SCALE;
        nodes.insert(s, N { pos: sp, cost: 0.0, total: h0, parent: u32::MAX, open: true, closed: false });
        let mut open = BinaryHeap::new();
        open.push(Q(h0, s));
        let (mut last, mut last_h) = (s, h0);
        let mut found = false;
        while let Some(Q(tot, b)) = open.pop() {
            let bn = nodes[&b];
            if bn.closed || tot > bn.total {
                continue;
            }
            nodes.get_mut(&b).unwrap().closed = true;
            nodes.get_mut(&b).unwrap().open = false;
            if b == e {
                last = e;
                found = true;
                break;
            }
            for l in &self.links[b as usize] {
                let nb = l.to;
                if nb == bn.parent || !self.pass(nb) {
                    continue;
                }
                let npos = match nodes.get(&nb) {
                    Some(n) => n.pos,
                    None => self.edge_mid(b, nb),
                };
                let (cost, heur) = if nb == e {
                    let c = bn.cost + self.cost(bn.pos, npos, b) + self.cost(npos, ep, nb);
                    (c, 0.0)
                } else {
                    (bn.cost + self.cost(bn.pos, npos, b), dist(npos, ep) * H_SCALE)
                };
                let total = cost + heur;
                if let Some(n) = nodes.get(&nb) {
                    if (n.open || n.closed) && total >= n.total {
                        continue;
                    }
                }
                nodes.insert(nb, N { pos: npos, cost, total, parent: b, open: true, closed: false });
                open.push(Q(total, nb));
                if heur < last_h {
                    last_h = heur;
                    last = nb;
                }
            }
        }
        let mut path = vec![last];
        while let Some(n) = nodes.get(path.last().unwrap()) {
            if n.parent == u32::MAX {
                break;
            }
            path.push(n.parent);
        }
        path.reverse();
        (path, found)
    }

    /// dtNavMeshQuery::findStraightPath (funnel over the portals): the corners after `sp`, the last = `ep`
    pub fn straight_path(&self, path: &[u32], sp: [f32; 3], ep: [f32; 3]) -> Vec<[f32; 3]> {
        let mut out: Vec<[f32; 3]> = Vec::new();
        let start = closest_on_boundary_if_outside(sp, &self.poly_verts(path[0]));
        let end = closest_on_boundary_if_outside(ep, &self.poly_verts(*path.last().unwrap()));
        if path.len() > 1 {
            let (mut apex, mut pl, mut pr) = (start, start, start);
            let (mut li, mut ri) = (0usize, 0usize);
            let mut ai: usize;
            let mut i = 0usize;
            while i < path.len() {
                let (left, right) = if i + 1 < path.len() {
                    match self.portal(path[i], path[i + 1]) {
                        Some((l, r)) => (l, r),
                        None => break,
                    }
                } else {
                    (end, end)
                };
                if i == 0 && i + 1 < path.len() && dist_pt_seg_sq_2d(apex, left, right) < 0.001 * 0.001 {
                    i += 1;
                    continue;
                }
                if tri_area_2d(apex, pr, right) <= 0.0 {
                    if vequal(apex, pr) || tri_area_2d(apex, pl, right) > 0.0 {
                        pr = right;
                        ri = i;
                    } else {
                        apex = pl;
                        ai = li;
                        out.push(apex);
                        pl = apex;
                        pr = apex;
                        li = ai;
                        ri = ai;
                        i = ai + 1;
                        continue;
                    }
                }
                if tri_area_2d(apex, pl, left) >= 0.0 {
                    if vequal(apex, pl) || tri_area_2d(apex, pr, left) < 0.0 {
                        pl = left;
                        li = i;
                    } else {
                        apex = pr;
                        ai = ri;
                        out.push(apex);
                        pl = apex;
                        pr = apex;
                        li = ai;
                        ri = ai;
                        i = ai + 1;
                        continue;
                    }
                }
                i += 1;
            }
        }
        if out.last().map(|l| !vequal(*l, end)).unwrap_or(true) {
            out.push(end);
        }
        out
    }

    /// FindPathSync on this mesh: the straight-path corners after `a` (Unreal cm) and whether the goal was reached
    pub fn path(&self, a: FVector, b: FVector) -> Option<(Vec<FVector>, bool)> {
        let (s, sp) = self.project(a)?;
        let (e, ep) = self.project(b)?;
        let (sp, ep) = (to_recast(sp), to_recast(ep));
        let (polys, complete) = self.find_poly_path(s, e, sp, ep);
        let goal = if complete { ep } else { self.closest_point_on_poly(*polys.last().unwrap(), ep).0 };
        Some((self.straight_path(&polys, sp, goal).into_iter().map(to_unreal).collect(), complete))
    }

    /// the connected component of every poly (links followed in both directions only where they exist)
    pub fn reachable_from(&self, s: u32) -> Vec<bool> {
        let mut seen = vec![false; self.poly_count()];
        let mut st = vec![s];
        seen[s as usize] = true;
        while let Some(a) = st.pop() {
            for l in &self.links[a as usize] {
                if !seen[l.to as usize] && self.pass(l.to) {
                    seen[l.to as usize] = true;
                    st.push(l.to);
                }
            }
        }
        seen
    }

    /// dtNavMeshQuery::raycast: true = the walk from a to b (Unreal cm) leaves the mesh (NavigationRaycast's hit)
    pub fn raycast_blocked(&self, a: FVector, b: FVector) -> bool {
        let Some((mut cur, sp)) = self.project(a) else { return true };
        let (s, e) = (to_recast(sp), to_recast(b));
        for _ in 0..4096 {
            let v = self.poly_verts(cur);
            let Some((_tmin, tmax, _smin, smax)) = intersect_segment_poly_2d(s, e, &v) else { return true };
            if smax < 0 {
                return false;
            }
            let (t, p) = self.tile_poly(cur);
            let nv = p.vert_count as usize;
            let mut next = None;
            for l in &self.links[cur as usize] {
                if l.edge as i32 != smax {
                    continue;
                }
                let (_, np) = self.tile_poly(l.to);
                if np.ptype == POLYTYPE_OFFMESH_POINT || !self.pass(l.to) {
                    continue;
                }
                if l.side == 0xff || (l.bmin == 0 && l.bmax == 255) {
                    next = Some(l.to);
                    break;
                }
                let v0 = t.verts[p.verts[l.edge as usize] as usize];
                let v1 = t.verts[p.verts[(l.edge as usize + 1) % nv] as usize];
                let k = if l.side == 0 || l.side == 4 { 2 } else { 0 };
                let sc = 1.0 / 255.0;
                let (mut lmin, mut lmax) = (v0[k] + (v1[k] - v0[k]) * (l.bmin as f32 * sc), v0[k] + (v1[k] - v0[k]) * (l.bmax as f32 * sc));
                if lmin > lmax {
                    std::mem::swap(&mut lmin, &mut lmax);
                }
                let x = s[k] + (e[k] - s[k]) * tmax;
                if x >= lmin && x <= lmax {
                    next = Some(l.to);
                    break;
                }
            }
            match next {
                Some(n) => cur = n,
                None => return true,
            }
        }
        true
    }

    /// dtNavMeshQuery::findRandomPointAroundCircle (+ dtRandomPointInConvexPoly) from `origin` (Unreal cm)
    pub fn random_point(&mut self, origin: FVector, radius: f32) -> Option<FVector> {
        let fr = mh_mode::consts::bot::FRAND_SCALE;
        let (s, sp) = self.project(origin)?;
        let c = to_recast(sp);
        let r2 = radius * radius;
        let mut nodes: HashMap<u32, (f32, [f32; 3], u32, bool)> = HashMap::new();
        nodes.insert(s, (0.0, c, u32::MAX, false));
        let mut open = BinaryHeap::from([Q(0.0, s)]);
        let (mut area_sum, mut pick) = (0.0f32, None);
        while let Some(Q(tot, b)) = open.pop() {
            let (bt, bpos, bpar, closed) = nodes[&b];
            if closed || tot > bt {
                continue;
            }
            nodes.get_mut(&b).unwrap().3 = true;
            if self.tile_poly(b).1.ptype == POLYTYPE_GROUND {
                let v = self.poly_verts(b);
                let mut pa = 0.0;
                for j in 2..v.len() {
                    pa += tri_area_2d(v[0], v[j - 1], v[j]).abs();
                }
                area_sum += pa;
                let u = self.rng.frand(fr) as f32;
                if u * area_sum <= pa {
                    pick = Some(b);
                }
            }
            for l in self.links[b as usize].clone() {
                let nb = l.to;
                if nb == bpar || !self.pass(nb) {
                    continue;
                }
                let Some((va, vb)) = self.portal(b, nb) else { continue };
                if dist_pt_seg_sq_2d(c, va, vb) > r2 {
                    continue;
                }
                let npos = match nodes.get(&nb) {
                    Some(n) if n.3 => continue,
                    Some(n) => n.1,
                    None => lerp(va, vb, 0.5),
                };
                let total = bt + dist(bpos, npos);
                if let Some(n) = nodes.get(&nb) {
                    if total >= n.0 {
                        continue;
                    }
                }
                nodes.insert(nb, (total, npos, b, false));
                open.push(Q(total, nb));
            }
        }
        let g = pick?;
        let v = self.poly_verts(g);
        let (s1, t1) = (self.rng.frand(fr) as f32, self.rng.frand(fr) as f32);
        let mut p = random_point_in_convex_poly(&v, s1, t1);
        p[1] = self.poly_height(g, p).unwrap_or(p[1]);
        Some(to_unreal(p))
    }
}

#[derive(PartialEq)]
struct Q(f32, u32);
impl Eq for Q {}
impl PartialOrd for Q {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Q {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal).then(o.1.cmp(&self.1))
    }
}

/// findNearestPoly's distance: a point directly over the poly (2D inside) is as near as its height above it minus
/// walkableClimb (clamped at 0, squared), else the 3D distance squared (Recast/Detour 1.4+ "favor the polygon below";
/// that UE 4.26's fork does the same: UNCONFIRMED)
fn nearest_metric(c: [f32; 3], q: [f32; 3], over: bool, climb: f32) -> f32 {
    if over {
        let d = (c[1] - q[1]).abs() - climb;
        if d > 0.0 {
            d * d
        } else {
            0.0
        }
    } else {
        dist_sq(c, q)
    }
}

// ---- Detour's geometry helpers (DetourCommon.cpp) ------------------------------------------------------------------

fn slab_coord(v: [f32; 3], side: i32) -> f32 {
    if side == 0 || side == 4 {
        v[0]
    } else {
        v[2]
    }
}

fn slab_end_points(va: [f32; 3], vb: [f32; 3], side: i32) -> ([f32; 2], [f32; 2]) {
    let k = if side == 0 || side == 4 { 2 } else { 0 };
    if va[k] < vb[k] {
        ([va[k], va[1]], [vb[k], vb[1]])
    } else {
        ([vb[k], vb[1]], [va[k], va[1]])
    }
}

fn overlap_slabs(amin: [f32; 2], amax: [f32; 2], bmin: [f32; 2], bmax: [f32; 2], px: f32, py: f32) -> bool {
    let minx = (amin[0] + px).max(bmin[0] + px);
    let maxx = (amax[0] - px).min(bmax[0] - px);
    if minx > maxx {
        return false;
    }
    let ad = (amax[1] - amin[1]) / (amax[0] - amin[0]);
    let ak = amin[1] - ad * amin[0];
    let bd = (bmax[1] - bmin[1]) / (bmax[0] - bmin[0]);
    let bk = bmin[1] - bd * bmin[0];
    let (aminy, amaxy) = (ad * minx + ak, ad * maxx + ak);
    let (bminy, bmaxy) = (bd * minx + bk, bd * maxx + bk);
    let (dmin, dmax) = (bminy - aminy, bmaxy - amaxy);
    if dmin * dmax < 0.0 {
        return true;
    }
    let thr = (py * 2.0) * (py * 2.0);
    dmin * dmin <= thr || dmax * dmax <= thr
}

pub fn tri_area_2d(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let (abx, abz) = (b[0] - a[0], b[2] - a[2]);
    let (acx, acz) = (c[0] - a[0], c[2] - a[2]);
    acx * abz - abx * acz
}

fn vequal(a: [f32; 3], b: [f32; 3]) -> bool {
    let thr = (1.0f32 / 16384.0) * (1.0 / 16384.0);
    dist_sq(a, b) < thr
}

fn dist_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    dist_sq(a, b).sqrt()
}

fn dist2d_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn dist_pt_seg_sq_2d(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let (px, pz) = (b[0] - a[0], b[2] - a[2]);
    let (dx, dz) = (p[0] - a[0], p[2] - a[2]);
    let d = px * px + pz * pz;
    let mut t = px * dx + pz * dz;
    if d > 0.0 {
        t /= d;
    }
    t = t.clamp(0.0, 1.0);
    let (x, z) = (a[0] + t * px - p[0], a[2] + t * pz - p[2]);
    x * x + z * z
}

/// dtPointInPolygon (xz)
pub fn point_in_poly(p: [f32; 3], v: &[[f32; 3]]) -> bool {
    let n = v.len();
    let mut c = false;
    let mut j = n - 1;
    for i in 0..n {
        let (vi, vj) = (v[i], v[j]);
        if ((vi[2] > p[2]) != (vj[2] > p[2])) && (p[0] < (vj[0] - vi[0]) * (p[2] - vi[2]) / (vj[2] - vi[2]) + vi[0]) {
            c = !c;
        }
        j = i;
    }
    c
}

fn closest_on_boundary(p: [f32; 3], v: &[[f32; 3]]) -> [f32; 3] {
    let n = v.len();
    let (mut best, mut bt, mut bi) = (f32::MAX, 0.0, 0);
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (v[j], v[i]);
        let (px, pz) = (b[0] - a[0], b[2] - a[2]);
        let (dx, dz) = (p[0] - a[0], p[2] - a[2]);
        let d = px * px + pz * pz;
        let t = if d > 0.0 { ((px * dx + pz * dz) / d).clamp(0.0, 1.0) } else { 0.0 };
        let (x, z) = (a[0] + t * px - p[0], a[2] + t * pz - p[2]);
        let dd = x * x + z * z;
        if dd < best {
            best = dd;
            bt = t;
            bi = j;
        }
        j = i;
    }
    lerp(v[bi], v[(bi + 1) % n], bt)
}

fn closest_on_boundary_if_outside(p: [f32; 3], v: &[[f32; 3]]) -> [f32; 3] {
    if point_in_poly(p, v) {
        p
    } else {
        closest_on_boundary(p, v)
    }
}

fn poly_overlaps(v: &[[f32; 3]], qmin: [f32; 3], qmax: [f32; 3]) -> bool {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in v {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (0..3).all(|k| lo[k] <= qmax[k] && hi[k] >= qmin[k])
}

/// dtClosestHeightPointTriangle
fn closest_height_point_triangle(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<f32> {
    let v0 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let v1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v2 = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let d2 = |x: [f32; 3], y: [f32; 3]| x[0] * y[0] + x[2] * y[2];
    let (dot00, dot01, dot02, dot11, dot12) = (d2(v0, v0), d2(v0, v1), d2(v0, v2), d2(v1, v1), d2(v1, v2));
    let denom = dot00 * dot11 - dot01 * dot01;
    if denom.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / denom;
    let u = (dot11 * dot02 - dot01 * dot12) * inv;
    let v = (dot00 * dot12 - dot01 * dot02) * inv;
    let eps = 1e-4;
    if u >= -eps && v >= -eps && (u + v) <= 1.0 + eps {
        Some(a[1] + v0[1] * u + v1[1] * v)
    } else {
        None
    }
}

/// dtIntersectSegmentPoly2D: (tmin, tmax, segMin, segMax)
fn intersect_segment_poly_2d(p0: [f32; 3], p1: [f32; 3], v: &[[f32; 3]]) -> Option<(f32, f32, i32, i32)> {
    let eps = 0.00000001;
    let (mut tmin, mut tmax, mut smin, mut smax) = (0.0f32, 1.0f32, -1i32, -1i32);
    let dir = [p1[0] - p0[0], 0.0, p1[2] - p0[2]];
    let perp = |u: [f32; 3], w: [f32; 3]| u[2] * w[0] - u[0] * w[2];
    let n = v.len();
    let mut j = n - 1;
    for i in 0..n {
        let edge = [v[i][0] - v[j][0], 0.0, v[i][2] - v[j][2]];
        let diff = [p0[0] - v[j][0], 0.0, p0[2] - v[j][2]];
        let num = perp(edge, diff);
        let den = perp(dir, edge);
        if den.abs() < eps {
            if num < 0.0 {
                return None;
            }
            j = i;
            continue;
        }
        let t = num / den;
        if den < 0.0 {
            if t > tmin {
                tmin = t;
                smin = j as i32;
                if tmin > tmax {
                    return None;
                }
            }
        } else if t < tmax {
            tmax = t;
            smax = j as i32;
            if tmax < tmin {
                return None;
            }
        }
        j = i;
    }
    Some((tmin, tmax, smin, smax))
}

/// dtRandomPointInConvexPoly
fn random_point_in_convex_poly(pts: &[[f32; 3]], s: f32, t: f32) -> [f32; 3] {
    let n = pts.len();
    let mut areas = vec![0.0f32; n];
    let mut sum = 0.0;
    for i in 2..n {
        areas[i] = tri_area_2d(pts[0], pts[i - 1], pts[i]).abs();
        sum += areas[i].max(0.001);
    }
    let thr = s * sum;
    let (mut acc, mut u, mut tri) = (0.0, 1.0, n - 1);
    for i in 2..n {
        let d = areas[i];
        if thr >= acc && thr < acc + d {
            u = (thr - acc) / d;
            tri = i;
            break;
        }
        acc += d;
    }
    let v = t.sqrt();
    let (a, b, c) = (1.0 - v, (1.0 - u) * v, u * v);
    let (pa, pb, pc) = (pts[0], pts[tri - 1], pts[tri]);
    [a * pa[0] + b * pb[0] + c * pc[0], a * pa[1] + b * pb[1] + c * pc[1], a * pa[2] + b * pb[2] + c * pc[2]]
}

/// an off-mesh connection of the cooked mesh (NavLinkProxy / smart links: jump-downs, ladders, climbs), Unreal cm
#[derive(Clone, Copy, Debug)]
pub struct NavLink {
    pub a: FVector,
    pub b: FVector,
    pub rad: f32,
    pub bidir: bool,
    pub area: u8,
}

impl DetourMesh {
    /// every off-mesh connection of every tile
    pub fn offmesh_links(&self) -> Vec<NavLink> {
        let mut out = vec![];
        for t in &self.tiles {
            for o in &t.offmesh {
                let area = t.polys.get(o.poly as usize).map(|p| p.area).unwrap_or(0);
                out.push(NavLink {
                    a: to_unreal([o.pos[0], o.pos[1], o.pos[2]]),
                    b: to_unreal([o.pos[3], o.pos[4], o.pos[5]]),
                    rad: o.rad,
                    bidir: o.flags & DT_OFFMESH_CON_BIDIR != 0,
                    area,
                });
            }
        }
        out
    }
}

impl mh_mode::ai::NavQueries for DetourMesh {
    fn raycast(&mut self, a: FVector, b: FVector) -> bool {
        self.raycast_blocked(a, b)
    }
    fn random_reachable_point(&mut self, origin: FVector, radius: f64) -> Option<FVector> {
        self.random_point(origin, radius as f32)
    }
    /// AAIController::MoveToLocation allows partial paths (bAllowPartialPaths default true): the partial path is
    /// returned too
    fn find_path(&mut self, a: FVector, b: FVector) -> Option<Vec<FVector>> {
        self.path(a, b).map(|p| p.0)
    }
    /// FindPathToLocationSynchronously's IsValid && !IsPartial
    fn path_complete(&mut self, a: FVector, b: FVector) -> Option<bool> {
        Some(self.path(a, b).map(|p| p.1).unwrap_or(false))
    }
}

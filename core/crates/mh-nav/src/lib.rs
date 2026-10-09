//! mh-nav: the bots' navmesh (rust-mode-ai r4). UE builds a Recast navmesh (ARecastNavMesh, dtNavMeshQuery) over the
//! map's collision inside its NavMeshBoundsVolumes with the agent of DefaultEngine.ini
//! [/Script/NavigationSystem.RecastNavMesh] (mh_level::config::nav_agent: AgentRadius 50, AgentHeight 192,
//! AgentMaxStepHeight 60, CellSize 5, CellHeight 5; AgentMaxSlope 44 from BaseEngine.ini). That pipeline is engine
//! code (Recast / Detour, not disassembled) and its polygons are not in the paks, so this is a stand-in built the same
//! way Recast builds its heightfield, at a coarser grid (UNCONFIRMED equality with the game's navmesh, as the Godot
//! BotWorld's bake already was):
//!   1. columns: every grid cell (BuildOptions::cell, default 25 cm) inside a bounds box is probed top-down with line
//!      traces against the collision world (mh_character::World: mh_level::collision::CollisionWorld on real maps);
//!      each hit with a walkable slope (impact normal z >= cos(AgentMaxSlope)) is a candidate floor (span), and the
//!      probe continues below it (several floors per column: bridges, buildings);
//!   2. clearance: a floor is walkable when a column of the agent's height (half a cell wide) standing on it, lifted
//!      by half the step height, overlaps nothing (Recast's rcFilterWalkableLowHeightSpans);
//!   2b. erosion (rcErodeWalkableArea): floors within AgentRadius of a boundary (wall, ledge, bounds) along the
//!      floors are removed;
//!   3. links: floors in 8-neighbour columns connect when their heights differ by at most AgentMaxStepHeight
//!      (Recast's walkableClimb); diagonals only when both orthogonal links exist (no corner cutting);
//!   4. queries: project (nearest floor), A* paths with string pulling (FindPathSync + the path corridor's straight
//!      path), raycast along the links (dtNavMeshQuery::raycast: blocked when the walk leaves the walkable floors),
//!      random reachable point (findRandomPointAroundCircle: a floor of the origin's connected region within the
//!      radius, picked with this mesh's own rand() stream; Detour's polygon-area weighting: UNCONFIRMED).
//! Engine-neutral: no I/O; the host hands the collision world, the bounds and the agent.

pub mod detour;
#[cfg(feature = "level")]
pub mod cooked;

use mh_character::world::World;
use mh_mode::ai::NavQueries;
use mordhau_core::ue::{CrtRand, FVector};
use std::collections::{BinaryHeap, HashMap, VecDeque};

/// the Recast agent (cm, degrees)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavAgent {
    pub radius: f32,
    pub height: f32,
    pub max_step: f32,
    pub max_slope: f32,
}

impl NavAgent {
    /// DefaultEngine.ini [/Script/NavigationSystem.RecastNavMesh] + BaseEngine.ini AgentMaxSlope (mh_level::config::
    /// nav_agent reads them from the install; these are its values, as godot/game/actor/bot_world.gd states them)
    pub fn mordhau() -> NavAgent {
        NavAgent { radius: 50.0, height: 192.0, max_step: 60.0, max_slope: 44.0 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BuildOptions {
    /// grid cell (cm)
    pub cell: f32,
    /// floors kept per column
    pub max_layers: usize,
}

impl Default for BuildOptions {
    fn default() -> Self {
        // max_layers 12: canopies / roofs above the ground take floor slots of a column (FFA_Camp: 4 layers lose
        // ground floors under trees, tests/nav.rs camp_probe)
        BuildOptions { cell: 25.0, max_layers: 12 }
    }
}

/// an axis-aligned bounds box (a NavMeshBoundsVolume's world box), UE cm
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// a walkable floor sample
#[derive(Clone, Debug)]
pub struct Node {
    /// the floor point (UE cm)
    pub p: FVector,
    pub col: u32,
    pub links: Vec<u32>,
    pub comp: u32,
}

pub struct NavMesh {
    pub agent: NavAgent,
    pub cell: f32,
    pub origin: [f32; 2],
    pub w: u32,
    pub h: u32,
    pub nodes: Vec<Node>,
    /// column -> its nodes
    cols: HashMap<u32, Vec<u32>>,
    /// this mesh's own rand() stream (random_reachable_point)
    pub rng: CrtRand,
}

#[derive(PartialEq)]
struct Open(f32, u32);
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(std::cmp::Ordering::Equal).then(o.1.cmp(&self.1))
    }
}

const DIRS: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

impl NavMesh {
    /// build over `bounds` (the map's NavMeshBoundsVolumes, or any boxes) against `world`
    pub fn build(world: &dyn World, bounds: &[Bounds], agent: NavAgent, opt: BuildOptions) -> NavMesh {
        let cell = opt.cell;
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for b in bounds {
            for k in 0..2 {
                lo[k] = lo[k].min(b.min[k]);
                hi[k] = hi[k].max(b.max[k]);
            }
        }
        if bounds.is_empty() {
            lo = [0.0; 2];
            hi = [0.0; 2];
        }
        let w = ((hi[0] - lo[0]) / cell).ceil().max(0.0) as u32;
        let h = ((hi[1] - lo[1]) / cell).ceil().max(0.0) as u32;
        let mut m = NavMesh { agent, cell, origin: lo, w, h, nodes: Vec::new(), cols: HashMap::new(), rng: CrtRand::new(1) };
        let cos_slope = agent.max_slope.to_radians().cos();
        let lift = (agent.max_step * 0.5).max(20.0);
        let hh = agent.height * 0.5;
        for j in 0..h {
            for i in 0..w {
                let x = lo[0] + (i as f32 + 0.5) * cell;
                let y = lo[1] + (j as f32 + 0.5) * cell;
                let (mut ztop, mut zbot) = (f32::MIN, f32::MAX);
                for b in bounds {
                    if x >= b.min[0] && x <= b.max[0] && y >= b.min[1] && y <= b.max[1] {
                        ztop = ztop.max(b.max[2]);
                        zbot = zbot.min(b.min[2]);
                    }
                }
                if ztop < zbot {
                    continue;
                }
                let col = j * w + i;
                let mut layers = 0;
                let mut top = ztop;
                while top > zbot && layers < opt.max_layers {
                    let Some(hit) = world.line_trace(FVector::new(x, y, top), FVector::new(x, y, zbot)) else { break };
                    let z = hit.impact_point.z;
                    if hit.start_penetrating || z >= top {
                        top -= 50.0;
                        continue;
                    }
                    let walkable_slope = hit.impact_normal.z >= cos_slope;
                    if walkable_slope && !world.overlap_capsule(FVector::new(x, y, z + lift + hh), cell * 0.5, hh) {
                        let id = m.nodes.len() as u32;
                        m.nodes.push(Node { p: FVector::new(x, y, z), col, links: Vec::new(), comp: u32::MAX });
                        m.cols.entry(col).or_default().push(id);
                        layers += 1;
                    }
                    top = z - 50.0;
                }
            }
        }
        m.link();
        m.erode();
        m
    }

    /// Recast's rcErodeWalkableArea: floors closer than AgentRadius (along the floors, 8-neighbour chamfer distance)
    /// to a boundary floor (one missing a neighbour: a wall, a ledge, the bounds) are removed, then the rest relinked
    fn erode(&mut self) {
        let n = self.nodes.len();
        let mut dist = vec![f32::MAX; n];
        let mut heap = BinaryHeap::new();
        for (i, nd) in self.nodes.iter().enumerate() {
            if nd.links.len() < 8 {
                dist[i] = 0.0;
                heap.push(Open(0.0, i as u32));
            }
        }
        while let Some(Open(d, a)) = heap.pop() {
            if d > dist[a as usize] {
                continue;
            }
            let pa = self.nodes[a as usize].p;
            for &b in &self.nodes[a as usize].links {
                let pb = self.nodes[b as usize].p;
                let nd = d + ((pb.x - pa.x).powi(2) + (pb.y - pa.y).powi(2)).sqrt();
                if nd < dist[b as usize] {
                    dist[b as usize] = nd;
                    heap.push(Open(nd, b));
                }
            }
        }
        // the boundary floor's centre is half a cell inside the edge
        let keep: Vec<bool> = dist.iter().map(|&d| d + self.cell * 0.5 >= self.agent.radius).collect();
        let mut remap = vec![u32::MAX; n];
        let mut nodes = Vec::new();
        for i in 0..n {
            if keep[i] {
                remap[i] = nodes.len() as u32;
                let mut nd = self.nodes[i].clone();
                nd.links.clear();
                nd.comp = u32::MAX;
                nodes.push(nd);
            }
        }
        self.nodes = nodes;
        self.cols.clear();
        for (i, nd) in self.nodes.iter().enumerate() {
            self.cols.entry(nd.col).or_default().push(i as u32);
        }
        self.link();
    }

    fn col_of(&self, x: f32, y: f32) -> Option<(i32, i32)> {
        let i = ((x - self.origin[0]) / self.cell).floor() as i32;
        let j = ((y - self.origin[1]) / self.cell).floor() as i32;
        if i < 0 || j < 0 || i >= self.w as i32 || j >= self.h as i32 {
            return None;
        }
        Some((i, j))
    }

    fn col_nodes(&self, i: i32, j: i32) -> &[u32] {
        if i < 0 || j < 0 || i >= self.w as i32 || j >= self.h as i32 {
            return &[];
        }
        self.cols.get(&(j as u32 * self.w + i as u32)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn step_to(&self, a: u32, i: i32, j: i32) -> Option<u32> {
        let za = self.nodes[a as usize].p.z;
        self.col_nodes(i, j)
            .iter()
            .copied()
            .filter(|&b| (self.nodes[b as usize].p.z - za).abs() <= self.agent.max_step)
            .min_by(|&x, &y| {
                let dx = (self.nodes[x as usize].p.z - za).abs();
                let dy = (self.nodes[y as usize].p.z - za).abs();
                dx.partial_cmp(&dy).unwrap()
            })
    }

    fn link(&mut self) {
        let n = self.nodes.len();
        let mut links = vec![Vec::new(); n];
        for a in 0..n as u32 {
            let col = self.nodes[a as usize].col;
            let (i, j) = ((col % self.w) as i32, (col / self.w) as i32);
            let mut ortho = [None; 4];
            for (k, (di, dj)) in DIRS.iter().enumerate() {
                if k < 4 {
                    ortho[k] = self.step_to(a, i + di, j + dj);
                    if let Some(b) = ortho[k] {
                        links[a as usize].push(b);
                    }
                } else {
                    let oi = if *di > 0 { 0 } else { 1 };
                    let oj = if *dj > 0 { 2 } else { 3 };
                    if ortho[oi].is_some() && ortho[oj].is_some() {
                        if let Some(b) = self.step_to(a, i + di, j + dj) {
                            links[a as usize].push(b);
                        }
                    }
                }
            }
        }
        for (a, l) in links.into_iter().enumerate() {
            self.nodes[a].links = l;
        }
        // keep links symmetric (step_to picks the closest floor: a one-way link is dropped)
        let snapshot: Vec<Vec<u32>> = self.nodes.iter().map(|x| x.links.clone()).collect();
        for a in 0..n {
            self.nodes[a].links.retain(|&b| snapshot[b as usize].contains(&(a as u32)));
        }
        self.components();
    }

    /// the connected regions, links taken both ways (an off-mesh link may be one-way: A* decides the direction)
    fn components(&mut self) {
        let n = self.nodes.len();
        let mut back: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (a, nd) in self.nodes.iter().enumerate() {
            for &b in &nd.links {
                back[b as usize].push(a as u32);
            }
        }
        for nd in self.nodes.iter_mut() {
            nd.comp = u32::MAX;
        }
        let mut comp = 0;
        for s in 0..n {
            if self.nodes[s].comp != u32::MAX {
                continue;
            }
            let mut q = VecDeque::from([s as u32]);
            self.nodes[s].comp = comp;
            while let Some(a) = q.pop_front() {
                let mut nb = self.nodes[a as usize].links.clone();
                nb.extend_from_slice(&back[a as usize]);
                for b in nb {
                    if self.nodes[b as usize].comp == u32::MAX {
                        self.nodes[b as usize].comp = comp;
                        q.push_back(b);
                    }
                }
            }
            comp += 1;
        }
    }

    /// the map's off-mesh connections (NavLinkProxy etc.; e.g. detour::DetourMesh::offmesh_links of the cooked mesh):
    /// each end attaches to the nearest floor within max(its radius, AgentRadius) + one cell (2D) and AgentMaxStepHeight
    /// (Z), as Detour's
    /// baseOffMeshLinks / connectExtOffMeshLinks snap them; a -> b always, b -> a when bidirectional. Returns the
    /// number attached.
    pub fn add_offmesh_links(&mut self, links: &[detour::NavLink]) -> usize {
        let mut n = 0;
        for l in links {
            // the stand-in's floors are eroded on a grid: the snap reaches an agent radius past the link's own
            let r = l.rad.max(self.agent.radius) + self.cell;
            let (Some(a), Some(b)) = (self.nearest_floor(l.a, r), self.nearest_floor(l.b, r)) else { continue };
            if a == b {
                continue;
            }
            if !self.nodes[a as usize].links.contains(&b) {
                self.nodes[a as usize].links.push(b);
            }
            if l.bidir && !self.nodes[b as usize].links.contains(&a) {
                self.nodes[b as usize].links.push(a);
            }
            n += 1;
        }
        self.components();
        n
    }

    fn nearest_floor(&self, p: FVector, r: f32) -> Option<u32> {
        let (ci, cj) = self.col_of(p.x, p.y)?;
        let k = (r / self.cell).ceil() as i32;
        let mut best: Option<(f32, u32)> = None;
        for dj in -k..=k {
            for di in -k..=k {
                for &n in self.col_nodes(ci + di, cj + dj) {
                    let q = self.nodes[n as usize].p;
                    let d2 = (q.x - p.x).powi(2) + (q.y - p.y).powi(2);
                    if d2 > r * r || (q.z - p.z).abs() > self.agent.max_step {
                        continue;
                    }
                    if best.map(|b| d2 < b.0).unwrap_or(true) {
                        best = Some((d2, n));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// the floor under / nearest to `p` (a pawn location: the capsule centre is above its floor), searched in the
    /// surrounding columns; floors more than AgentMaxStepHeight above `p` or more than two agent heights below are
    /// not taken
    pub fn project(&self, p: FVector) -> Option<u32> {
        let (ci, cj) = self.col_of(p.x, p.y).or_else(|| {
            let x = p.x.clamp(self.origin[0], self.origin[0] + self.w as f32 * self.cell - 1.0);
            let y = p.y.clamp(self.origin[1], self.origin[1] + self.h as f32 * self.cell - 1.0);
            self.col_of(x, y)
        })?;
        let mut best: Option<(f32, u32)> = None;
        let r = 4;
        for dj in -r..=r {
            for di in -r..=r {
                for &n in self.col_nodes(ci + di, cj + dj) {
                    let q = self.nodes[n as usize].p;
                    let dz = p.z - q.z;
                    if dz < -self.agent.max_step || dz > 2.0 * self.agent.height {
                        continue;
                    }
                    let d2 = (q.x - p.x) * (q.x - p.x) + (q.y - p.y) * (q.y - p.y) + 0.25 * dz * dz;
                    if best.map(|b| d2 < b.0).unwrap_or(true) {
                        best = Some((d2, n));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// walk the links from `a` toward b's xy: Some(node reached at b's column) or None when the walk leaves the floors
    fn walk(&self, a: u32, b: FVector) -> Option<u32> {
        let pa = self.nodes[a as usize].p;
        let (dx, dy) = (b.x - pa.x, b.y - pa.y);
        let len = (dx * dx + dy * dy).sqrt();
        let steps = (len / (self.cell * 0.5)).ceil().max(1.0) as i32;
        let mut cur = a;
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            let (x, y) = (pa.x + dx * t, pa.y + dy * t);
            let Some((i, j)) = self.col_of(x, y) else { return None };
            let c = self.nodes[cur as usize].col;
            if c == j as u32 * self.w + i as u32 {
                continue;
            }
            let next = self.nodes[cur as usize].links.iter().copied().find(|&n| self.nodes[n as usize].col == j as u32 * self.w + i as u32)?;
            cur = next;
        }
        Some(cur)
    }

    /// A* over the links, then the straight path (string pulling with `walk`): the waypoints after `a`
    pub fn path(&self, a: FVector, b: FVector) -> Option<Vec<FVector>> {
        let s = self.project(a)?;
        let g = self.project(b)?;
        if self.nodes[s as usize].comp != self.nodes[g as usize].comp {
            return None;
        }
        let pg = self.nodes[g as usize].p;
        let hf = |n: u32| {
            let p = self.nodes[n as usize].p;
            ((p.x - pg.x).powi(2) + (p.y - pg.y).powi(2)).sqrt()
        };
        let mut cost: HashMap<u32, f32> = HashMap::from([(s, 0.0)]);
        let mut from: HashMap<u32, u32> = HashMap::new();
        let mut open = BinaryHeap::from([Open(hf(s), s)]);
        while let Some(Open(_, a)) = open.pop() {
            if a == g {
                break;
            }
            let ca = cost[&a];
            let pa = self.nodes[a as usize].p;
            for &b in &self.nodes[a as usize].links {
                let pb = self.nodes[b as usize].p;
                let c = ca + ((pb.x - pa.x).powi(2) + (pb.y - pa.y).powi(2) + (pb.z - pa.z).powi(2)).sqrt();
                if cost.get(&b).map(|&o| c < o).unwrap_or(true) {
                    cost.insert(b, c);
                    from.insert(b, a);
                    open.push(Open(c + hf(b), b));
                }
            }
        }
        if !cost.contains_key(&g) {
            return None;
        }
        let mut chain = vec![g];
        while let Some(&p) = from.get(chain.last().unwrap()) {
            chain.push(p);
        }
        chain.reverse();
        // string pulling: from each corner, the farthest chain node still walkable in a straight line
        let mut out = Vec::new();
        let mut k = 0;
        while k + 1 < chain.len() {
            let mut far = k + 1;
            for m in (k + 2..chain.len()).rev() {
                if self.walk(chain[k], self.nodes[chain[m] as usize].p) == Some(chain[m]) {
                    far = m;
                    break;
                }
            }
            out.push(self.nodes[chain[far] as usize].p);
            k = far;
        }
        if out.is_empty() {
            out.push(pg);
        }
        Some(out)
    }

    /// dtNavMeshQuery::raycast stand-in: true when the straight walk from a to b leaves the walkable floors
    pub fn raycast_blocked(&self, a: FVector, b: FVector) -> bool {
        match self.project(a) {
            None => true,
            Some(s) => self.walk(s, b).is_none(),
        }
    }

    /// findRandomPointAroundCircle stand-in: a floor of the origin's region within `radius` (2D), uniform over the
    /// floors (this mesh's rand() stream)
    pub fn random_point(&mut self, origin: FVector, radius: f32) -> Option<FVector> {
        let s = self.project(origin)?;
        let r2 = radius * radius;
        let mut seen = vec![s];
        let mut mark: HashMap<u32, ()> = HashMap::from([(s, ())]);
        let mut q = VecDeque::from([s]);
        while let Some(a) = q.pop_front() {
            for &b in &self.nodes[a as usize].links {
                let p = self.nodes[b as usize].p;
                if (p.x - origin.x).powi(2) + (p.y - origin.y).powi(2) > r2 || mark.contains_key(&b) {
                    continue;
                }
                mark.insert(b, ());
                seen.push(b);
                q.push_back(b);
            }
        }
        let f = (self.rng.rand() & 0x7fff) as f32 / 32768.0;
        let i = ((f * seen.len() as f32) as usize).min(seen.len() - 1);
        Some(self.nodes[seen[i] as usize].p)
    }
}

impl NavQueries for NavMesh {
    fn raycast(&mut self, a: FVector, b: FVector) -> bool {
        self.raycast_blocked(a, b)
    }
    fn random_reachable_point(&mut self, origin: FVector, radius: f64) -> Option<FVector> {
        self.random_point(origin, radius as f32)
    }
    fn find_path(&mut self, a: FVector, b: FVector) -> Option<Vec<FVector>> {
        self.path(a, b)
    }
}

/// a navmesh plus the collision world it was built from: the hearing / visibility line traces too
pub struct NavWorld<'w> {
    pub mesh: &'w mut NavMesh,
    pub world: &'w dyn World,
}

impl NavQueries for NavWorld<'_> {
    fn raycast(&mut self, a: FVector, b: FVector) -> bool {
        self.mesh.raycast_blocked(a, b)
    }
    fn random_reachable_point(&mut self, origin: FVector, radius: f64) -> Option<FVector> {
        self.mesh.random_point(origin, radius as f32)
    }
    fn find_path(&mut self, a: FVector, b: FVector) -> Option<Vec<FVector>> {
        self.mesh.path(a, b)
    }
    fn line_blocked(&mut self, a: FVector, b: FVector) -> Option<bool> {
        Some(self.world.line_trace(a, b).is_some())
    }
}

/// path following: mh_mode::ai::PathFollower over this mesh (NavQueries::find_path)
pub use mh_mode::ai::PathFollower;

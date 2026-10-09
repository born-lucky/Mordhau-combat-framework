//! Landscape terrain: every ULandscapeComponent of a level as an engine-neutral heightfield patch with its paint-layer
//! weights. Layout and decoding from CUE4Parse at the pinned commit:
//! - component properties (all tagged): SectionBaseX/Y, ComponentSizeQuads, SubsectionSizeQuads, NumSubsections,
//!   HeightmapScaleBias, HeightmapTexture, WeightmapScaleBias, WeightmapLayerAllocations {LayerInfo,
//!   WeightmapTextureIndex, WeightmapTextureChannel}, WeightmapTextures, CachedLocalBox, MapBuildDataId
//!   (UE4/Assets/Exports/Component/Landscape/ULandscapeComponent.cs:15-57);
//! - heightmap (CUE4Parse-Conversion/Dto/LandscapeDataAccess.cs): a PF_B8G8R8A8 texture shared by several components;
//!   this component's texels start at (SizeX * HeightmapScaleBias.Z, SizeY * HeightmapScaleBias.W) (:50-55); vertex
//!   (x, y) maps to texel (SubNumX * SubsectionSizeVerts + SubX, ...) with the shared last vertex of each subsection
//!   (VertexXYToTexelXY / ComponentXYToSubsectionXY, :264-292); height = R << 8 | G (:318-322), local Z = (height -
//!   32768) / 128 (GetLocalHeight, LANDSCAPE_ZSCALE, :334-343); the normal is B, A as 2 * c / 255 - 1 with
//!   Z = sqrt(1 - x^2 - y^2) (GetLocalTangentVectors, :364-375); local X/Y = vertex index (ComponentSizeQuads /
//!   (ComponentSizeVerts - 1) = 1 at mip 0, no XY offset map, :324-329);
//! - weightmaps: one byte per vertex from channel WeightmapTextureChannel (0 = R, 1 = G, 2 = B, 3 = A of the BGRA
//!   texel, :191-203) of WeightmapTextures[WeightmapTextureIndex], at the same texel mapping offset by
//!   (SizeX * WeightmapScaleBias.Z, SizeY * WeightmapScaleBias.W) (GetLayerWeight, :232-258).
//! Local positions go through the component's world transform (the Landscape actor's root carries the grid scale,
//! typically 100 cm per quad), so world = xf * (x, y, local_z).

use crate::level::{pkg_of, Obj, Pkgs};
use crate::native::after_props;
use crate::texture::{mip0, split_ref, Tex};
use crate::xf::{vec3, Xf};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// A paint layer of one component
#[derive(Clone, Debug)]
pub struct LandscapeLayer {
    /// ULandscapeLayerInfoObject LayerName (else the info object's name)
    pub name: String,
    /// LayerInfo object path
    pub info: String,
    /// one weight (0..255) per vertex, row-major (y * verts + x)
    pub weights: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct LandscapeComponent {
    pub level: usize,
    /// "<landscape actor>.<component>"
    pub name: String,
    pub actor: String,
    /// component -> world (UE cm)
    pub xf: Xf,
    pub section_base: [i64; 2],
    pub size_quads: usize,
    pub subsection_size_quads: usize,
    pub num_subsections: usize,
    /// vertices per side = size_quads + 1
    pub verts: usize,
    /// local Z per vertex, row-major (y * verts + x), in heightmap units (world via `xf`)
    pub heights: Vec<f32>,
    /// heightmap normal bytes (B, A) per vertex: n = (2 b / 255 - 1, 2 a / 255 - 1, sqrt(1 - x^2 - y^2)) in local space
    pub normal_bytes: Vec<[u8; 2]>,
    pub layers: Vec<LandscapeLayer>,
    /// the actor's LandscapeMaterial (or the component's OverrideMaterial), material package
    pub material: String,
    /// the component's LandscapeMaterialInstanceConstant per LOD material index (object paths)
    pub material_instances: Vec<String>,
    /// CachedLocalBox (local units) as serialized
    pub cached_box: Option<([f64; 3], [f64; 3])>,
    pub map_build_data_id: Option<String>,
    /// baked grass data (FLandscapeComponentGrassData), None when the component has none
    pub grass: Option<GrassData>,
}

/// FLandscapeComponentGrassData, UE4 (CUE4Parse FLandscapeComponentGrassData.cs:80-81): bulk TArray<uint16> HeightData
/// (raw heightmap values, one per vertex, row-major like the render heightmap and within one raw unit of it: the grass
/// map is rendered on the GPU) + TMap<FPackageIndex ULandscapeGrassType, TArray<uint8>>
/// WeightData (the material's LandscapeGrassOutput, baked per vertex). The grass varieties (mesh, density, scaling,
/// cull distances) are the grass type asset's GrassVarieties (`LevelData::grass_types`).
#[derive(Clone, Debug, Default)]
pub struct GrassData {
    pub heights: Vec<u16>,
    /// (grass type object path "pkg.N", one weight per vertex)
    pub weights: Vec<(String, Vec<u8>)>,
}

/// A ULandscapeHeightfieldCollisionComponent: the cooked PhysX heightfield (what characters walk on)
#[derive(Clone, Debug)]
pub struct LandscapeCollision {
    pub level: usize,
    pub name: String,
    pub actor: String,
    /// component -> world (UE cm)
    pub xf: Xf,
    pub section_base: [i64; 2],
    pub size_quads: usize,
    pub collision_scale: f64,
    /// vertices per side
    pub verts: usize,
    /// local Z per vertex in UE (x, y) order, row-major y * verts + x, same units as LandscapeComponent::heights
    pub heights: Vec<f32>,
    /// PhysX materialIndex0 per vertex (index into `physical_materials`; HOLE = 127): the material of the cell's first
    /// triangle, the cell being the quad whose PhysX (row, col) corner is this sample
    pub materials: Vec<u8>,
    /// PhysX materialIndex1 (the cell's second triangle)
    pub materials1: Vec<u8>,
    /// PhysX tess flag (bit 7 of materialIndex0): the cell's diagonal runs (row, col)-(row+1, col+1) when set, else
    /// (row+1, col)-(row, col+1) (PxHeightFieldSample::tessFlag)
    pub tess: Vec<bool>,
    pub physical_materials: Vec<String>,
    pub layer_infos: Vec<String>,
    /// the component -> world transform has a negative determinant (e.g. ThePit's Landscape1, scale (-39, 39, 19)):
    /// UE cooks the samples without the row flip (`sample_xy`)
    pub mirrored: bool,
}

/// PxHeightFieldMaterial::eHOLE
pub const HOLE: u8 = 127;

impl LandscapeCollision {
    /// The heightfield's triangles in world space (UE cm), holes left out: per PhysX cell (row, col) two triangles
    /// split along the tess diagonal, materialIndex0 for the first, materialIndex1 for the second
    /// (PxHeightField cell layout; PhysX docs "Heightfields")
    pub fn triangles(&self) -> Vec<([[f64; 3]; 3], u8)> {
        let n = self.verts;
        let mut out = vec![];
        for row in 0..n - 1 {
            for col in 0..n - 1 {
                self.cell_triangles(row, col, &mut out);
            }
        }
        out
    }

    /// The (up to) 2 triangles of PhysX cell (row, col), world space, holes left out
    pub fn cell_triangles(&self, row: usize, col: usize, out: &mut Vec<([[f64; 3]; 3], u8)>) {
        let n = self.verts;
        let mirrored = self.mirrored;
        let at = |row: usize, col: usize| -> (usize, usize) { sample_rc_xy(row, col, n, mirrored) }; // PhysX (row, col) -> UE (x, y)
        let w = |(x, y): (usize, usize)| self.vertex_world(x, y);
        let (x, y) = at(row, col);
        let i = y * n + x;
        let (p00, p10, p01, p11) = (w(at(row, col)), w(at(row + 1, col)), w(at(row, col + 1)), w(at(row + 1, col + 1)));
        let (t0, t1) = if self.tess[i] { ([p00, p10, p11], [p00, p11, p01]) } else { ([p00, p10, p01], [p10, p11, p01]) };
        if self.materials[i] != HOLE {
            out.push((t0, self.materials[i]));
        }
        if self.materials1[i] != HOLE {
            out.push((t1, self.materials1[i]));
        }
    }

    /// Cells (row, col) whose footprint can meet the world-space box [min, max]: the box's corners through the inverse
    /// of `xf` give a local (x, y) range in vertex units (row = verts - 1 - x, col = y)
    pub fn cells_in(&self, inv: &Xf, min: [f64; 3], max: [f64; 3]) -> Option<(usize, usize, usize, usize)> {
        let (mut lx0, mut lx1, mut ly0, mut ly1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..8 {
            let c = [if i & 1 == 0 { min[0] } else { max[0] }, if i & 2 == 0 { min[1] } else { max[1] }, if i & 4 == 0 { min[2] } else { max[2] }];
            let l = inv.apply(c);
            lx0 = lx0.min(l[0]);
            lx1 = lx1.max(l[0]);
            ly0 = ly0.min(l[1]);
            ly1 = ly1.max(l[1]);
        }
        let s = self.collision_scale;
        let last = (self.verts - 1) as f64;
        // one cell of padding each side (box edges on integer vertex lines)
        let (x0, x1, y0, y1) = ((lx0 / s).floor() - 1.0, (lx1 / s).ceil() + 1.0, (ly0 / s).floor() - 1.0, (ly1 / s).ceil() + 1.0);
        let (x0, x1, y0, y1) = (x0.max(0.0), x1.min(last), y0.max(0.0), y1.min(last));
        if x0 > x1 || y0 > y1 || x1 < 0.0 || y1 < 0.0 || x0 > last || y0 > last {
            return None;
        }
        let n = self.verts - 1;
        // cell (row, col) spans x in [verts-2-row, verts-1-row] (mirrored: [row, row+1]), y in [col, col+1]
        let (row0, row1) = if self.mirrored {
            (x0 as usize, (n - 1).min(x1 as usize))
        } else {
            (n.saturating_sub(x1 as usize), (n - 1).min(n.saturating_sub(x0 as usize)))
        };
        let (col0, col1) = (y0 as usize, (n - 1).min(y1 as usize));
        Some((row0, row1, col0, col1))
    }
    pub fn is_hole(&self, x: usize, y: usize) -> bool {
        self.materials[y * self.verts + x] == HOLE
    }
    pub fn vertex_world(&self, x: usize, y: usize) -> [f64; 3] {
        self.xf.apply([x as f64 * self.collision_scale, y as f64 * self.collision_scale, self.heights[y * self.verts + x] as f64])
    }
}

/// A Landscape / LandscapeStreamingProxy actor: material and LOD settings (tagged properties of ALandscapeProxy:
/// LOD0ScreenSize, LODDistributionSetting, LOD0DistributionSetting, StaticLightingLOD, MaxLODLevel,
/// ComponentScreenSizeToUseSubSections, TessellationComponentScreenSize, ...)
#[derive(Clone, Debug)]
pub struct LandscapeActor {
    pub level: usize,
    pub name: String,
    pub class: String,
    pub xf: Xf,
    pub material: String,
    pub hole_material: String,
    pub props: Map<String, Value>,
}

impl LandscapeComponent {
    /// World position of vertex (x, y), UE cm
    pub fn vertex_world(&self, x: usize, y: usize) -> [f64; 3] {
        self.xf.apply([x as f64, y as f64, self.heights[y * self.verts + x] as f64])
    }
    /// Local normal of vertex (x, y) (before `xf`'s rotation/scale)
    pub fn normal_local(&self, x: usize, y: usize) -> [f64; 3] {
        let [b, a] = self.normal_bytes[y * self.verts + x];
        let nx = 2.0 * b as f64 / 255.0 - 1.0;
        let ny = 2.0 * a as f64 / 255.0 - 1.0;
        [nx, ny, (1.0 - (nx * nx + ny * ny)).max(0.0).sqrt()]
    }
    pub fn height_range(&self) -> (f32, f32) {
        self.heights.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &h| (a.min(h), b.max(h)))
    }
}

fn f(v: Option<&Value>, k: &str) -> f64 {
    v.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).unwrap_or(0.0)
}
fn int(p: &Map<String, Value>, k: &str, d: i64) -> i64 {
    p.get(k).and_then(|v| v.as_i64()).unwrap_or(d)
}
fn op(v: Option<&Value>) -> &str {
    v.and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()).unwrap_or("")
}

/// Texel mapping (LandscapeDataAccess.cs:264-292)
fn texel(v: usize, sub_verts: usize) -> usize {
    if v == 0 {
        return 0;
    }
    let sub_num = (v - 1) / (sub_verts - 1);
    let sub = (v - 1) % (sub_verts - 1) + 1;
    sub_num * sub_verts + sub
}

/// Decode one LandscapeComponent export; `cache` keeps decoded textures (heightmaps are shared)
pub fn component(pk: &Pkgs, level: usize, lx: Xf, e: &Obj, tex_cache: &mut HashMap<String, Option<Tex>>) -> Option<LandscapeComponent> {
    let p = pk.props(e);
    let size_quads = int(&p, "ComponentSizeQuads", 0).max(0) as usize;
    let subq = int(&p, "SubsectionSizeQuads", 0).max(0) as usize;
    let nsub = int(&p, "NumSubsections", 1).max(1) as usize;
    if size_quads == 0 || subq == 0 {
        return None;
    }
    let verts = size_quads + 1;
    let sub_verts = subq + 1;
    let mut get_tex = |path: &str| -> Option<Tex> {
        tex_cache
            .entry(path.to_string())
            .or_insert_with(|| split_ref(path).and_then(|(pkg, i)| mip0(pk, pkg, i)))
            .clone()
    };
    let hm = get_tex(op(p.get("HeightmapTexture")))?;
    if hm.pixel_format != "PF_B8G8R8A8" {
        return None;
    }
    let hsb = p.get("HeightmapScaleBias");
    let (ox, oy) = ((hm.width as f64 * f(hsb, "Z")) as usize, (hm.height as f64 * f(hsb, "W")) as usize);
    let mut heights = Vec::with_capacity(verts * verts);
    let mut normal_bytes = Vec::with_capacity(verts * verts);
    for y in 0..verts {
        for x in 0..verts {
            let (tx, ty) = (texel(x, sub_verts) + ox, texel(y, sub_verts) + oy);
            let i = (ty * hm.width + tx) * 4;
            let t = hm.data.get(i..i + 4)?; // B, G, R, A
            let h = ((t[2] as u32) << 8) | t[1] as u32;
            heights.push(((h as f64 - 32768.0) / 128.0) as f32);
            normal_bytes.push([t[0], t[3]]);
        }
    }
    let wsb = p.get("WeightmapScaleBias");
    let wtex: Vec<String> = p
        .get("WeightmapTextures")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(|t| op(Some(t)).to_string()).collect())
        .unwrap_or_default();
    let mut layers = vec![];
    for al in p.get("WeightmapLayerAllocations").and_then(|v| v.as_array()).into_iter().flatten() {
        let info = op(al.get("LayerInfo")).to_string();
        let ti = al.get("WeightmapTextureIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let ch = al.get("WeightmapTextureChannel").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let name = pk
            .obj(al.get("LayerInfo"))
            .and_then(|o| o.get("Properties").and_then(|p| p.get("LayerName")).and_then(|v| v.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| pkg_of(&info).rsplit('/').next().unwrap_or("").to_string());
        let Some(wt) = wtex.get(ti).and_then(|t| get_tex(t)) else { continue };
        if wt.pixel_format != "PF_B8G8R8A8" || ch > 3 {
            continue;
        }
        let (wx, wy) = ((wt.width as f64 * f(wsb, "Z")) as usize, (wt.height as f64 * f(wsb, "W")) as usize);
        let off = [2usize, 1, 0, 3][ch]; // R, G, B, A offsets in a BGRA texel
        let mut weights = Vec::with_capacity(verts * verts);
        for y in 0..verts {
            for x in 0..verts {
                let i = ((texel(y, sub_verts) + wy) * wt.width + texel(x, sub_verts) + wx) * 4 + off;
                weights.push(*wt.data.get(i).unwrap_or(&0));
            }
        }
        layers.push(LandscapeLayer { name, info, weights });
    }
    let actor = pk.obj(e.get("Outer"));
    let actor_name = actor.as_ref().and_then(|a| a.get("Name")).and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let material = {
        let own = pkg_of(op(p.get("OverrideMaterial"))).to_string();
        if own.is_empty() {
            actor.as_ref().map(|a| pkg_of(op(a.get("Properties").and_then(|p| p.get("LandscapeMaterial")))).to_string()).unwrap_or_default()
        } else {
            own
        }
    };
    let cached_box = p.get("CachedLocalBox").map(|b| (vec3(b.get("Min"), 0.0), vec3(b.get("Max"), 0.0)));
    Some(LandscapeComponent {
        level,
        name: format!("{actor_name}.{}", e.get("Name").and_then(|v| v.as_str()).unwrap_or("")),
        actor: actor_name,
        xf: lx * pk.world_xf(e),
        section_base: [int(&p, "SectionBaseX", 0), int(&p, "SectionBaseY", 0)],
        size_quads,
        subsection_size_quads: subq,
        num_subsections: nsub,
        verts,
        heights,
        normal_bytes,
        layers,
        material,
        material_instances: p
            .get("MaterialInstances")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(|m| op(Some(m)).to_string()).collect())
            .unwrap_or_default(),
        cached_box,
        map_build_data_id: p.get("MapBuildDataId").and_then(|v| v.as_str()).map(|s| s.replace('-', "")),
        grass: grass(pk, e),
    })
}

fn pkg_of_obj(e: &Obj) -> &str {
    pkg_of(op(e.get("Outer")))
}

/// ULandscapeComponent native tail, UE 4.26 cooked (ULandscapeComponent.cs:59-139): no legacy lightmap
/// (MapBuildDataSeparatePackage), FLandscapeComponentGrassData (SERIALIZE_LANDSCAPE_GRASS_DATA), no SelectedType
/// (filter editor only), bool bCooked, bool bCookedMobileData (+ mobile data)
fn grass(pk: &Pkgs, e: &Obj) -> Option<GrassData> {
    let a = pk.rd.open(pkg_of_obj(e))?;
    let (mut r, end) = after_props(pk, &a, e.1)?;
    let (es, n) = (r.s32(), r.s32());
    if n < 0 || (n > 0 && es != 2) || r.p + n as i64 * 2 > end {
        return None;
    }
    let heights: Vec<u16> = (0..n).map(|_| r.u16()).collect();
    let m = r.s32();
    if !(0..=64).contains(&m) {
        return None;
    }
    let mut weights = vec![];
    for _ in 0..m {
        let idx = r.s32();
        let path = pk.rd.node(&a, idx).map(|nd| match &nd {
            mh_pak::Node::Export(p, i) => format!("{}.{i}", p.name),
            other => pk.rd.node_name(other),
        });
        let c = r.s32();
        if c < 0 || r.p + c as i64 > end {
            return None;
        }
        let w = r.at(r.p, c as usize)?.into_owned();
        r.skip(c as i64);
        weights.push((path.unwrap_or_default(), w));
    }
    if r.bad || (heights.is_empty() && weights.is_empty()) {
        return None;
    }
    Some(GrassData { heights, weights })
}

/// ULandscapeHeightfieldCollisionComponent (ULandscapeHeightfieldCollisionComponent.cs:22-34): after the properties,
/// bool bCooked and a bulk TArray<uint8> CookedCollisionData = the PhysX cooked height field. Its bytes (PhysX 4.1
/// Gu::HeightField serialization, read off the data and checked against the render heightmap in tests/r4.rs): "NXS"
/// 0x01 platform tag, "HFHF", uint32 version 1, uint32 rows, columns, float rowLimit, colLimit, nbColumns,
/// thickness, convexEdgeThreshold, uint16 flags, uint32 format (1 = eS16_TM), PxBounds3 (6 floats), uint32
/// sampleStride (4), nbSamples, float minHeight, maxHeight, then nbSamples x PxHeightFieldSample {int16 height,
/// uint8 materialIndex0 (bit 7 = tess flag), uint8 materialIndex1}. A height is the landscape raw height - 32768
/// (local units x 128, as the render heightmap), material 127 is a hole. Sample order: `SAMPLE_ORDER`.
pub fn collision(pk: &Pkgs, level: usize, lx: Xf, e: &Obj) -> Option<LandscapeCollision> {
    let p = pk.props(e);
    let a = pk.rd.open(pkg_of_obj(e))?;
    let (mut r, end) = after_props(pk, &a, e.1)?;
    if r.s32() == 0 {
        return None;
    }
    let (_es, n) = (r.s32(), r.s32());
    let start = r.p;
    if n < 94 || start + n as i64 > end {
        return None;
    }
    if &r.at(start + 4, 4)?[..] != b"HFHF" {
        return None;
    }
    r.p = start + 12;
    let (rows, cols) = (r.u32() as usize, r.u32() as usize);
    r.skip(4 * 5 + 2 + 4 + 24);
    let (stride, ns) = (r.u32(), r.u32() as usize);
    r.skip(8);
    if stride != 4 || ns != rows * cols || rows != cols || r.p + ns as i64 * 4 > start + n as i64 {
        return None;
    }
    let verts = rows;
    let xf = lx * pk.world_xf(e);
    let m = &xf.m;
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let mirrored = det < 0.0;
    let mut heights = vec![0f32; ns];
    let mut materials = vec![0u8; ns];
    let mut materials1 = vec![0u8; ns];
    let mut tess = vec![false; ns];
    for i in 0..ns {
        let h = r.s16();
        let m0 = r.u8();
        let m1 = r.u8();
        let (x, y) = sample_xy(i, verts, mirrored);
        heights[y * verts + x] = (h as f64 / 128.0) as f32;
        materials[y * verts + x] = m0 & 0x7f;
        materials1[y * verts + x] = m1 & 0x7f;
        tess[y * verts + x] = m0 & 0x80 != 0;
    }
    let actor = pk.obj(e.get("Outer"));
    let actor_name = actor.as_ref().and_then(|a| a.get("Name")).and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let list = |k: &str| -> Vec<String> {
        p.get(k).and_then(|v| v.as_array()).map(|a| a.iter().map(|m| op(Some(m)).to_string()).collect()).unwrap_or_default()
    };
    Some(LandscapeCollision {
        level,
        name: format!("{actor_name}.{}", e.get("Name").and_then(|v| v.as_str()).unwrap_or("")),
        actor: actor_name,
        xf,
        section_base: [int(&p, "SectionBaseX", 0), int(&p, "SectionBaseY", 0)],
        size_quads: int(&p, "CollisionSizeQuads", 0).max(0) as usize,
        collision_scale: p.get("CollisionScale").and_then(|v| v.as_f64()).unwrap_or(1.0),
        verts,
        heights,
        materials,
        materials1,
        tess,
        physical_materials: list("CookedPhysicalMaterials"),
        layer_infos: list("ComponentLayerInfos"),
        mirrored,
    })
}

/// PhysX sample i -> UE vertex (x, y). PhysX index = row * columns + column; y = column, and x = verts - 1 - row for an
/// unmirrored landscape, x = row for a mirrored one (negative transform determinant). That is UE 4.26
/// ULandscapeHeightfieldCollisionComponent::CookCollisionData's SrcSampleIndex = ColIndex * N + (bMirrored ? RowIndex :
/// N - RowIndex - 1) [UE source recalled]; both branches are proven by the data: every collision sample equals the
/// render heightmap on Camp (unmirrored) and on ThePit (Landscape1 scale (-39, 39, 19), mirrored) (tests/r4.rs, r6.rs)
pub const SAMPLE_ORDER: &str = "y = column; x = verts - 1 - row (x = row when mirrored)";
pub fn sample_xy(i: usize, verts: usize, mirrored: bool) -> (usize, usize) {
    sample_rc_xy(i / verts, i % verts, verts, mirrored)
}
pub fn sample_rc_xy(row: usize, col: usize, verts: usize, mirrored: bool) -> (usize, usize) {
    (if mirrored { row } else { verts - 1 - row }, col)
}

/// Landscape / LandscapeStreamingProxy actor record
pub fn actor(pk: &Pkgs, level: usize, lx: Xf, e: &Value) -> LandscapeActor {
    let p = pk.props(e);
    let rc = pk.obj(p.get("RootComponent"));
    LandscapeActor {
        level,
        name: e.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        class: e.get("Type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        xf: lx * rc.map(|rc| pk.world_xf(&rc)).unwrap_or(crate::xf::IDENTITY),
        material: pkg_of(op(p.get("LandscapeMaterial"))).to_string(),
        hole_material: pkg_of(op(p.get("LandscapeHoleMaterial"))).to_string(),
        props: p,
    }
}

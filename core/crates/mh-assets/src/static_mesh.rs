//! A cooked UE 4.26 StaticMesh read from the paks: LOD 0's sections, positions, normals and tangents, UVs, vertex
//! colours and indices, in UE units. Port of godot/components/ue/pak/ue_static_mesh.gd (only LOD 0 is decoded).
//!
//! Layout (CUE4Parse at the pinned commit, tools/CUE4Parse-src/CUE4Parse/UE4/..., versions as VersionContainer.cs:78-99
//! sets them for UE 4.26):
//!   UStaticMesh (Assets/Exports/StaticMesh/UStaticMesh.cs): tagged properties, UObject Guid (bool + FGuid),
//!     FStripDataFlags (2 bytes), bool bCooked, FPackageIndex BodySetup, FPackageIndex NavCollision
//!     (StaticMesh.HasNavCollision), FGuid LightingGuid, TArray<FPackageIndex> Sockets, then FStaticMeshRenderData:
//!     TArray<FStaticMeshLODResources>.
//!   FStaticMeshLODResources (FStaticMeshLODResources.cs:45-104): FStripDataFlags, TArray<FStaticMeshSection>, float
//!     MaxDeviation, bool bIsLODCookedOut, bool bInlined, then the buffers inline or in an FByteBulkData.
//!   FStaticMeshSection (FStaticMeshSection.cs): int32 MaterialIndex, FirstIndex, NumTriangles, MinVertexIndex,
//!     MaxVertexIndex, bool bEnableCollision, bCastShadow, bForceOpaque, bVisibleInRayTracing (4.26).
//!   SerializeBuffers (FStaticMeshLODResources.cs:314-393): FStripDataFlags; FPositionVertexBuffer {int32 Stride,
//!     NumVertices, bulk FVector[]}; FStaticMeshVertexBuffer {FStripDataFlags, int32 NumTexCoords, NumVertices, bool
//!     bUseFullPrecisionUVs, bUseHighPrecisionTangentBasis, bulk tangents (TangentX, TangentZ), bulk UVs}
//!     (FStaticMeshVertexBuffer.cs); FColorVertexBuffer {FStripDataFlags, int32 Stride, NumVertices, bulk FColor[] if
//!     any} (FColorVertexBuffer.cs); FRawStaticIndexBuffer {bool b32Bit, bulk bytes, bool bShouldExpandTo32Bit}
//!     (FRawStaticIndexBuffer.cs). A "bulk" array is int32 element size + int32 count + the elements (ReadBulkArray).
//! FPackedNormal (Objects/Meshes/FPackedNormal.cs): 4 bytes, each ^ 0x80 (IncreaseNormalPrecision) then / 127.5 - 1;
//! with bUseHighPrecisionTangentBasis, FPackedRGBA16N (4 x uint16, each ^ 0x8000, (v - 32767.5) / 32767.5;
//! Objects/RenderCore/FPackedRGBA16N.cs).

use crate::buf::{le_u16, le_u32, Buf};
use crate::bulk;
use crate::props::{self, Props};
use crate::{err, skip_object_guid, PackageSource, RawPackage, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub material_index: i32,
    pub first_index: i32,
    pub num_triangles: i32,
    pub min_vertex_index: i32,
    pub max_vertex_index: i32,
}

/// Vertex attributes shared by static and skeletal LODs (FStaticMeshVertexBuffer + FPositionVertexBuffer)
#[derive(Debug, Clone, Default)]
pub struct Vertices {
    /// UE cm, Z up
    pub positions: Vec<[f32; 3]>,
    /// TangentZ (UE space, unit length up to quantization); empty when the tangent item size is unknown
    pub normals: Vec<[f32; 3]>,
    /// TangentX xyz + the binormal sign in w (TangentZ.w, +-1); UE space. UNCONFIRMED against an export: the
    /// extract/gltf meshes carry no TANGENT attribute, only the normals are checked
    pub tangents: Vec<[f32; 4]>,
    /// one Vec per UV channel
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub high_precision_tangents: bool,
}

#[derive(Debug, Clone, Default)]
pub struct StaticMesh {
    pub package: String,
    pub sections: Vec<Section>,
    pub vertices: Vertices,
    /// FColor as R, G, B, A (empty when the LOD has no colour buffer)
    pub colors: Vec<[u8; 4]>,
    /// triangle list, UE's corner order (counter-clockwise front after `coords` conversion)
    pub indices: Vec<u32>,
    /// material slot names by MaterialIndex are in properties StaticMaterials (not decoded here)
    pub properties: Props,
}

/// LOD 0 of the package's StaticMesh export
pub fn lod0(src: &dyn PackageSource, pkg_path: &str) -> Result<StaticMesh> {
    lod0_export(src, pkg_path, None)
}

/// LOD 0 of the StaticMesh export named `export_name` (None = the first): packages holding several meshes, e.g. the
/// HLOD proxy packages (Arena_0_HLOD: one StaticMesh export per proxy). Added by bevy-runtime r2.
pub fn lod0_export(src: &dyn PackageSource, pkg_path: &str, export_name: Option<&str>) -> Result<StaticMesh> {
    match lods_export(src, pkg_path, export_name, 1)?.into_iter().next().flatten() {
        Some(m) => Ok(m),
        None => err(format!("{}: LOD 0 cooked out", pkg_path)),
    }
}

/// Every LOD of the package's StaticMesh export (None = cooked out). Layout per LOD (FStaticMeshLODResources.cs:
/// 45-195, UE 4.26 cooked): FStripDataFlags, sections, MaxDeviation, bIsLODCookedOut, bInlined; then, unless cooked
/// out, the buffers inline (SerializeBuffers incl. the reversed / depth-only / wireframe / adjacency index buffers per
/// the strip flags) or an FByteBulkData followed by 88 bytes of buffer descriptors (DepthOnlyNumTriangles + packed,
/// the vertex / index buffer headers, AdjacencyIndexBuffer 4.26); then FStaticMeshBuffersSize (3 x uint32).
pub fn lods(src: &dyn PackageSource, pkg_path: &str) -> Result<Vec<Option<StaticMesh>>> {
    lods_export(src, pkg_path, None, usize::MAX)
}

fn lods_export(src: &dyn PackageSource, pkg_path: &str, export_name: Option<&str>, max: usize) -> Result<Vec<Option<StaticMesh>>> {
    let pkg = src.package(pkg_path)?;
    let e = match pkg.find_export("StaticMesh", export_name) {
        Some(e) => e,
        None => return err(format!("{}: no StaticMesh export", pkg_path)),
    };
    let (start, end) = pkg.export_range(e)?;
    let mut r = Buf::at(&pkg.data, start);
    let properties = props::read_tagged(&pkg, &mut r, end);
    skip_object_guid(&mut r, e);
    r.skip(2); // FStripDataFlags
    if r.s32() == 0 {
        return err(format!("{}: not cooked", pkg_path)); // bCooked
    }
    r.skip(4 + 4); // BodySetup, NavCollision
    r.skip(16); // LightingGuid
    let ns = r.s32();
    r.skip_n(ns, 4); // Sockets
    let nl = r.s32();
    if nl < 1 || nl > 16 {
        return err(format!("{}: {} LODs", pkg_path, nl));
    }
    let mut out = Vec::new();
    for _ in 0..(nl as usize).min(max) {
        let m = lod(src, &pkg, &mut r)?.map(|mut m| {
            m.package = pkg.name.clone();
            m.properties = properties.clone();
            m
        });
        out.push(m);
    }
    Ok(out)
}

fn lod(src: &dyn PackageSource, pkg: &RawPackage, r: &mut Buf) -> Result<Option<StaticMesh>> {
    r.skip(2); // FStripDataFlags
    let n = r.s32();
    if !(0..=4096).contains(&n) {
        return err(format!("{}: {} sections", pkg.name, n));
    }
    let mut sections = Vec::with_capacity(n as usize);
    for _ in 0..n {
        sections.push(Section {
            material_index: r.s32(),
            first_index: r.s32(),
            num_triangles: r.s32(),
            min_vertex_index: r.s32(),
            max_vertex_index: r.s32(),
        });
        r.skip(4 * 4); // bEnableCollision, bCastShadow, bForceOpaque, bVisibleInRayTracing
    }
    r.f32(); // MaxDeviation
    let cooked_out = r.s32() != 0;
    let inlined = r.s32() != 0;
    if r.bad {
        return err(format!("{}: LOD header read past the export", pkg.name));
    }
    if cooked_out {
        r.skip(12); // FStaticMeshBuffersSize
        return Ok(None);
    }
    let mut m = if inlined {
        buffers(&pkg.name, r, sections.len())?
    } else {
        let h = bulk::header(pkg, r);
        r.skip(88); // DepthOnlyNumTriangles + packed, buffer headers, AdjacencyIndexBuffer (FStaticMeshLODResources.cs:143-158)
        let b = bulk::bytes(src, pkg, &h)?;
        buffers(&pkg.name, &mut Buf::new(&b), sections.len())?
    };
    r.skip(12); // FStaticMeshBuffersSize: SerializedBuffersSize, DepthOnlyIBSize, ReversedIBsSize
    m.sections = sections;
    Ok(Some(m))
}

/// FRawStaticIndexBuffer {bool b32Bit, bulk bytes, bool bShouldExpandTo32Bit} (FRawStaticIndexBuffer.cs)
fn skip_index_buffer(r: &mut Buf) {
    r.s32();
    r.bulk_array(None);
    r.skip(4);
}

/// FWeightedRandomSampler {TArray<float> Prob, TArray<int32> Alias, float TotalWeight}
fn skip_sampler(r: &mut Buf) {
    let n = r.s32();
    r.skip_n(n, 4);
    let n = r.s32();
    r.skip_n(n, 4);
    r.skip(4);
}

fn buffers(name: &str, r: &mut Buf, num_sections: usize) -> Result<StaticMesh> {
    let (global_strip, class_strip) = (r.u8(), r.u8()); // FStripDataFlags
    let positions = read_positions(r);
    let mut v = read_vertex_buffer(r, positions.len()).ok_or_else(|| crate::AssetError(format!("{}: vertex buffer", name)))?;
    v.positions = positions;
    let colors = read_colors(r);
    // FRawStaticIndexBuffer
    let is32 = r.s32() != 0;
    let (_, raw) = r.bulk_array(None);
    r.skip(4); // bShouldExpandTo32Bit
    let indices = indices(raw, is32);
    // the other index buffers (SerializeBuffers, FStaticMeshLODResources.cs:330-350): EClassDataStripFlag
    // CDSF_AdjacencyData 1, CDSF_ReversedIndexBuffer 4; global flag 1 = editor data stripped
    if class_strip & 4 == 0 {
        skip_index_buffer(r); // ReversedIndexBuffer
    }
    skip_index_buffer(r); // DepthOnlyIndexBuffer
    if class_strip & 4 == 0 {
        skip_index_buffer(r); // ReversedDepthOnlyIndexBuffer
    }
    if global_strip & 1 == 0 {
        skip_index_buffer(r); // WireframeIndexBuffer
    }
    if class_strip & 1 == 0 {
        skip_index_buffer(r); // AdjacencyIndexBuffer (4.26: before RemovingTessellation)
    }
    // UE 4.26 FStaticMeshLODResources::SerializeBuffers then writes the Niagara mesh-sampling data: one
    // FWeightedRandomSampler per section (AreaWeightedSectionSamplers) and one for the whole LOD (AreaWeightedSampler);
    // UNCONFIRMED as to field names (not in CUE4Parse at the pinned commit), measured: 12 x (sections + 1) bytes of
    // empty samplers sit between the index buffers and FStaticMeshBuffersSize on every LOD decoded so far
    for _ in 0..=num_sections {
        skip_sampler(r);
    }
    if r.bad {
        return err(format!("{}: LOD buffers read past the data", name));
    }
    Ok(StaticMesh { vertices: v, colors, indices, ..Default::default() })
}

/// FPositionVertexBuffer {int32 Stride, NumVertices, bulk FVector[]} (also used by skeletal meshes); UE cm
pub fn read_positions(r: &mut Buf) -> Vec<[f32; 3]> {
    r.skip(4); // Stride
    let nv = r.s32();
    let (cnt, raw) = r.bulk_array(Some(12));
    if nv < 0 || cnt != nv as usize || raw.len() != cnt * 12 {
        r.bad = true;
        return Vec::new();
    }
    raw.chunks_exact(12)
        .map(|c| {
            let f = |o: usize| f32::from_le_bytes([c[o], c[o + 1], c[o + 2], c[o + 3]]);
            [f(0), f(4), f(8)]
        })
        .collect()
}

fn packed_normal(u: u32) -> [f32; 4] {
    let z = u ^ 0x8080_8080;
    let c = |s: u32| ((z >> s) & 0xff) as f32 / 127.5 - 1.0;
    [c(0), c(8), c(16), c(24)]
}

fn packed_rgba16n(b: &[u8], o: usize) -> [f32; 4] {
    let c = |k: usize| ((le_u16(b, o + 2 * k) ^ 0x8000) as f32 - 32767.5) / 32767.5;
    [c(0), c(1), c(2), c(3)]
}

/// FStaticMeshVertexBuffer after the positions (nv vertices): normals, tangents, UVs; None when it does not match nv
pub fn read_vertex_buffer(r: &mut Buf, nv: usize) -> Option<Vertices> {
    r.skip(2); // FStripDataFlags
    let ntex = r.s32();
    if r.s32() as usize != nv || !(0..=8).contains(&ntex) {
        return None;
    }
    let full_uv = r.s32() != 0;
    let high = r.s32() != 0;
    let esz = r.s32();
    let cnt = r.s32();
    if esz < 0 || cnt < 0 {
        r.bad = true;
        return None;
    }
    let raw = r.bytes(esz as usize * cnt as usize);
    let mut normals = Vec::new();
    let mut tangents = Vec::new();
    if !high && esz == 8 {
        for c in raw.chunks_exact(8) {
            let x = packed_normal(le_u32(c, 0)); // TangentX
            let z = packed_normal(le_u32(c, 4)); // TangentZ = the normal, w = binormal sign
            normals.push([z[0], z[1], z[2]]);
            tangents.push([x[0], x[1], x[2], if z[3] < 0.0 { -1.0 } else { 1.0 }]);
        }
    } else if high && esz == 16 {
        for c in raw.chunks_exact(16) {
            let x = packed_rgba16n(c, 0);
            let z = packed_rgba16n(c, 8);
            normals.push([z[0], z[1], z[2]]);
            tangents.push([x[0], x[1], x[2], if z[3] < 0.0 { -1.0 } else { 1.0 }]);
        }
    } // else: an item size no tangent format has: normals left empty (as ue_static_mesh.gd)
    let ntex = ntex as usize;
    let uv_size = if full_uv { 8 } else { 4 };
    let (_, uvraw) = r.bulk_array(None);
    if uvraw.len() < nv * ntex * uv_size {
        r.bad = true;
        return None;
    }
    let mut uvs = vec![Vec::with_capacity(nv); ntex];
    for i in 0..nv {
        for (c, ch) in uvs.iter_mut().enumerate() {
            let o = (i * ntex + c) * uv_size;
            ch.push(if full_uv {
                [crate::buf::le_f32(uvraw, o), crate::buf::le_f32(uvraw, o + 4)]
            } else {
                [crate::buf::half_to_f32(le_u16(uvraw, o)), crate::buf::half_to_f32(le_u16(uvraw, o + 2))]
            });
        }
    }
    Some(Vertices { positions: Vec::new(), normals, tangents, uvs, high_precision_tangents: high })
}

/// FColorVertexBuffer {FStripDataFlags, int32 Stride, NumVertices, bulk FColor[] when NumVertices > 0}; as R, G, B, A
pub fn read_colors(r: &mut Buf) -> Vec<[u8; 4]> {
    r.skip(2);
    r.skip(4); // Stride
    let n = r.s32();
    if n <= 0 {
        return Vec::new();
    }
    let (_, raw) = r.bulk_array(Some(4));
    raw.chunks_exact(4).map(|c| [c[2], c[1], c[0], c[3]]).collect() // FColor in memory order B, G, R, A
}

pub fn indices(raw: &[u8], is32: bool) -> Vec<u32> {
    if is32 {
        raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    } else {
        raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_normal_decodes_axes() {
        // +Z, w = +1: stored byte 0x00 ^ 0x80 = 128 -> ~0, 0x7f ^ 0x80 = 255 -> 1
        let up = packed_normal(u32::from_le_bytes([0x00, 0x00, 0x7f, 0x7f]));
        assert!(up[0].abs() < 0.01 && up[1].abs() < 0.01 && (up[2] - 1.0).abs() < 0.01 && up[3] > 0.99);
        let h = packed_rgba16n(&[0, 0, 0, 0, 0xff, 0x7f, 0, 0], 0);
        assert!(h[0].abs() < 1e-4 && (h[2] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn index_widths() {
        assert_eq!(indices(&[1, 0, 2, 0], false), vec![1, 2]);
        assert_eq!(indices(&[1, 0, 2, 0], true), vec![0x0002_0001]);
    }
}

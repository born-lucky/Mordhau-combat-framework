//! Natively serialized data of level components, read from the export bytes after the tagged properties (mh-pak's
//! export JSON holds tagged properties only; extract/json has these because CUE4Parse decodes them natively).
//! Layouts are UE 4.26 cooked, cited to CUE4Parse at the pinned commit (tools/CUE4Parse-src/CUE4Parse/...).

use crate::level::Pkgs;
use crate::xf::Xf;
use mh_pak::{asset::RF_CLASS_DEFAULT_OBJECT, Cursor};

/// One FStaticMeshComponentLODInfo
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LodInfo {
    /// MapBuildDataId as 32 hex digits without dashes (the MeshBuildData key, ue_lightmap.gd key())
    pub map_build_data_id: Option<String>,
    /// OverrideVertexColors (mesh paint), RGBA8 per LOD vertex in the mesh's own vertex order
    pub override_colors: Option<Vec<[u8; 4]>>,
}

/// The native tail of a (Instanced)StaticMeshComponent
#[derive(Clone, Debug, Default)]
pub struct MeshComponentData {
    pub lods: Vec<LodInfo>,
    /// PerInstanceSMData (instanced components only)
    pub instances: Option<Vec<Xf>>,
}

/// FGuid as UE's EGuidFormats::Digits ("C257E9534C6040B1...")
pub fn guid_digits(r: &mut Cursor) -> String {
    format!("{:08X}{:08X}{:08X}{:08X}", r.u32(), r.u32(), r.u32(), r.u32())
}

/// Cursor positioned after an export's tagged properties and UObject Guid (UObject.cs:226-236), with the export's end
pub fn after_props<'a>(pk: &Pkgs, a: &'a std::rc::Rc<mh_pak::Package>, export: usize) -> Option<(Cursor<'a>, i64)> {
    let e = a.exports.get(export)?;
    let end = e.off + e.size;
    let mut r = a.cursor();
    r.p = e.off;
    pk.rd.tagged(a, &mut r, end);
    if e.flags & RF_CLASS_DEFAULT_OBJECT == 0 && r.s32() != 0 {
        r.skip(16);
    }
    Some((r, end))
}

/// A bulk array header (int32 element size, int32 count) - TBulkData / ReadBulkArray (FArchive.cs ReadBulkArray)
fn bulk_header(r: &mut Cursor) -> (i64, i64) {
    (r.s32() as i64, r.s32() as i64)
}

/// UStaticMeshComponent LODData = TArray<FStaticMeshComponentLODInfo> (UStaticMeshComponent.cs:22;
/// FStaticMeshComponentLODInfo.cs:47-118): FStripDataFlags {u8 global, u8 class}; FGuid MapBuildDataId unless
/// audio-visual data is stripped (global & 2, FStripDataFlags.cs:31); unless class flag 1 (OverrideColorsStripFlag,
/// :41) is set, u8 bLoadVertexColorData and, when 1, an FColorVertexBuffer {FStripDataFlags, int32 Stride, int32
/// NumVertices, bulk TArray<FColor> when not stripped and NumVertices > 0} (FColorVertexBuffer.cs:26-41).
/// PaintedVertices are editor data (stripped in cooks). Then, for UInstancedStaticMeshComponent
/// (UInstancedStaticMeshComponent.cs:16-106): bool bCooked (FEditorObjectVersion SerializeInstancedStaticMeshRenderData)
/// and PerInstanceSMData as a bulk array of FInstancedStaticMeshInstanceData = FMatrix Transform, 64 bytes
/// (FInstancedStaticMeshInstanceData.cs:12-25). FMatrix is row-vector (rows = scaled axes, row 3 = origin), so the
/// column-vector affine is its transpose; CUE4Parse decomposes it into TransformData (FTransform.SetFromMatrix), the
/// matrix itself is what UE renders.
pub fn mesh_component(pk: &Pkgs, level_pkg: &str, export: usize, instanced: bool) -> Option<MeshComponentData> {
    let a = pk.rd.open(level_pkg)?;
    let (mut r, end) = after_props(pk, &a, export)?;
    let nl = r.s32();
    if !(0..=16).contains(&nl) {
        return None;
    }
    let mut out = MeshComponentData::default();
    for _ in 0..nl {
        let mut li = LodInfo::default();
        let (global, class) = (r.u8(), r.u8());
        if global & 2 == 0 {
            li.map_build_data_id = Some(guid_digits(&mut r));
        }
        if class & 1 == 0 && r.u8() == 1 {
            let g2 = r.u8();
            r.u8();
            let _stride = r.s32();
            let nv = r.s32();
            if g2 & 2 == 0 && nv > 0 {
                let (es, n) = bulk_header(&mut r);
                if es != 4 || n < 0 || r.p + n * 4 > end {
                    return None;
                }
                // FColor is stored B, G, R, A
                let mut c = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let (b, g, rr, al) = (r.u8(), r.u8(), r.u8(), r.u8());
                    c.push([rr, g, b, al]);
                }
                li.override_colors = Some(c);
            }
        }
        out.lods.push(li);
    }
    if r.bad || r.p > end {
        return None;
    }
    if instanced {
        r.s32(); // bCooked
        let (es, n) = bulk_header(&mut r);
        if es != 64 || n < 0 || r.p + n * 64 > end || r.bad {
            return Some(out);
        }
        let mut v = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let mut m = [[0f64; 4]; 4];
            for row in m.iter_mut() {
                for x in row.iter_mut() {
                    *x = r.f32raw() as f64;
                }
            }
            v.push(Xf { m: [[m[0][0], m[1][0], m[2][0], m[3][0]], [m[0][1], m[1][1], m[2][1], m[3][1]], [m[0][2], m[1][2], m[2][2], m[3][2]]] });
        }
        out.instances = Some(v);
    }
    Some(out)
}

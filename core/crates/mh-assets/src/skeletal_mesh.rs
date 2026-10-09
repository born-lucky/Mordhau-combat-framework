//! A cooked UE 4.26 SkeletalMesh read from the paks: the reference skeleton and LOD 0 (sections, positions, normals,
//! tangents, UVs, bone influences, indices), in UE units. Port of godot/components/ue/pak/ue_skeletal_mesh.gd.
//!
//! Layout (CUE4Parse at the pinned commit, tools/CUE4Parse-src/CUE4Parse/UE4/Assets/Exports/SkeletalMesh/..., custom
//! versions at their UE 4.26 values as Versions/*.cs default them):
//!   USkeletalMesh (USkeletalMesh.cs:24-172): tagged properties, UObject Guid, FStripDataFlags, FBoxSphereBounds
//!     ImportedBounds (28 bytes), TArray<FSkeletalMaterial>, FReferenceSkeleton, bool bCooked, int32 LOD count, per LOD
//!     FSkeletalMeshLODRenderData (FStaticLODModel.SerializeRenderItem, FStaticLODModel.cs:408-516).
//!   FSkeletalMaterial (FSkeletalMaterial.cs): FPackageIndex MaterialInterface, FName MaterialSlotName, bool
//!     bSerializeImportedMaterialSlotName (+ FName), FMeshUVChannelInfo (bool, bool, float[4]: 24 bytes).
//!   LOD: FStripDataFlags, bool bIsLODCookedOut, bool bInlined, TArray<int16> RequiredBones, TArray<FSkelMeshSection>,
//!     TArray<int16> ActiveBoneIndices, uint32 BuffersSize, then the streamed data inline or in an FByteBulkData.
//!   FSkelMeshSection render item (FSkelMeshSection.cs:245-319): FStripDataFlags, int16 MaterialIndex, int32 BaseIndex,
//!     NumTriangles, bool bRecomputeTangent, uint8 RecomputeTangentsVertexMaskChannel, bool bCastShadow, uint32
//!     BaseVertexIndex, TArray<FMeshToMeshVertData> (64 bytes each), TArray<uint16> BoneMap, int32 NumVertices,
//!     MaxBoneInfluences, int16 CorrespondClothAssetIndex, FClothingSectionData (FGuid + int32), duplicated-vertex
//!     arrays (unless class strip flag 1), bool bDisabled.
//!   Streamed data (FStaticLODModel.SerializeStreamedData): FStripDataFlags, FMultisizeIndexContainer (uint8 size +
//!     bulk indices), FPositionVertexBuffer, FStaticMeshVertexBuffer (static_mesh readers), FSkinWeightVertexBuffer
//!     (4.26 format, FSkinWeightVertexBuffer.cs:19-128: FStripDataFlags, bool bVariableBonesPerVertex, uint32
//!     MaxBoneInfluences, NumBones, NumVertices, bool bUse16BitBoneIndex, bulk bytes; FStripDataFlags, int32
//!     NumLookupVertices, bulk uint32 lookup (offset << 8 | count) for variable influences), then the colour buffer
//!     when bHasVertexColors. Per vertex: the bone indices (into the section's BoneMap), then the weights, one byte each
//!     (FSkinWeightInfo).
//! FReferenceSkeleton (FReferenceSkeleton.cs:15-31, FMeshBoneInfo.cs:17-30; ue_asset.gd ref_skeleton):
//!   TArray<{FName Name; int32 ParentIndex}> FinalRefBoneInfo, TArray<FTransform {FQuat, FVector, FVector}>
//!   FinalRefBonePose, TMap<FName, int32> FinalNameToIndexMap.

use crate::buf::Buf;
use crate::bulk;
use crate::props::{self, Props};
use crate::static_mesh::{self, Vertices};
use crate::{err, skip_object_guid, PackageSource, RawPackage, Result};

/// FSkinWeightVertexBuffer.cs:10: 4 influences per vertex (8 = EXTRA_BONE_INFLUENCES when MaxBoneInfluences > 4)
pub const NUM_INFLUENCES_UE4: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct BoneInfo {
    pub name: String,
    pub parent: i32,
}

/// FTransform: rotation (x, y, z, w), translation (cm), scale; UE space
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefSkeleton {
    pub bones: Vec<BoneInfo>,
    /// local (parent-relative) reference pose per bone
    pub pose: Vec<Transform>,
}

impl RefSkeleton {
    /// Bone index by name, ignoring case (UE FName comparison)
    pub fn find(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }
}

/// FReferenceSkeleton at the cursor (a Skeleton export's native data, or a SkeletalMesh header)
pub fn read_ref_skeleton(pkg: &RawPackage, r: &mut Buf) -> Result<RefSkeleton> {
    let count = |r: &mut Buf| -> Result<usize> {
        let n = r.s32();
        if n < 0 || n as usize > r.left() {
            r.bad = true;
            return err(format!("{}: reference skeleton count {}", pkg.name, n));
        }
        Ok(n as usize)
    };
    let mut s = RefSkeleton::default();
    for _ in 0..count(r)? {
        let name = pkg.fname(r);
        s.bones.push(BoneInfo { name, parent: r.s32() });
    }
    for _ in 0..count(r)? {
        let rotation = [r.f32(), r.f32(), r.f32(), r.f32()];
        let translation = [r.f32(), r.f32(), r.f32()];
        let scale = [r.f32(), r.f32(), r.f32()];
        s.pose.push(Transform { rotation, translation, scale });
    }
    for _ in 0..count(r)? {
        r.skip(12); // FinalNameToIndexMap entry: FName + int32 (rebuilt from bones)
    }
    if r.bad || s.pose.len() != s.bones.len() {
        return err(format!("{}: reference skeleton does not decode", pkg.name));
    }
    Ok(s)
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkelSection {
    pub material_index: i16,
    pub base_index: i32,
    pub num_triangles: i32,
    pub base_vertex_index: u32,
    pub num_vertices: i32,
    pub max_bone_influences: i32,
    /// section-local bone index -> reference skeleton bone (already applied to `SkeletalMesh::bones`)
    pub bone_map: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkelMaterial {
    pub slot: String,
    /// object name of the MaterialInterface ("" when null)
    pub material: String,
    /// content package of the MaterialInterface ("Mordhau/Content/..."; the outermost import, "/Game/" mapped to the
    /// pak's "Mordhau/Content/"; this package for an export; "" when null)
    pub material_package: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkeletalMesh {
    pub package: String,
    pub ref_skeleton: RefSkeleton,
    pub materials: Vec<SkelMaterial>,
    pub sections: Vec<SkelSection>,
    pub vertices: Vertices,
    pub colors: Vec<[u8; 4]>,
    /// triangle list, UE's corner order
    pub indices: Vec<u32>,
    /// `max_influences` entries per vertex: reference-skeleton bone indices and byte weights / 255 (as stored; they sum
    /// to 1 up to the bytes' rounding - `normalized_weights` rescales)
    pub bones: Vec<u16>,
    pub weights: Vec<f32>,
    /// 4 or 8
    pub max_influences: usize,
    pub properties: Props,
}

impl SkeletalMesh {
    /// The weights of vertex v rescaled to sum to 1 (UE byte weights sum to 255; ue_skeletal_mesh.gd array_mesh)
    pub fn normalized_weights(&self, v: usize) -> Vec<f32> {
        let w = &self.weights[v * self.max_influences..(v + 1) * self.max_influences];
        let s: f32 = w.iter().sum();
        if s > 0.0 { w.iter().map(|x| x / s).collect() } else { w.to_vec() }
    }
}

/// The reference skeleton of a package's Skeleton export (USkeleton.cs:44-47: UObject Guid then FReferenceSkeleton),
/// with each bone's TranslationRetargetingMode from the BoneTree property (as stored, e.g.
/// "EBoneTranslationRetargetingMode::Skeleton"; "" when the BoneTree entry leaves it at its default, Animation)
pub fn skeleton(src: &dyn PackageSource, pkg_path: &str) -> Result<(RefSkeleton, Vec<String>)> {
    let pkg = src.package(pkg_path)?;
    let e = match pkg.find_export("Skeleton", None) {
        Some(e) => e,
        None => return err(format!("{}: no Skeleton export", pkg_path)),
    };
    let (start, end) = pkg.export_range(e)?;
    let mut r = Buf::at(&pkg.data, start);
    let p = props::read_tagged(&pkg, &mut r, end);
    skip_object_guid(&mut r, e);
    let s = read_ref_skeleton(&pkg, &mut r)?;
    let mut modes = Vec::new();
    if let Some(props::Value::Array(a)) = p.get("BoneTree") {
        for bt in a {
            let m = match bt {
                props::Value::Struct(f) => props::get_str(f, "TranslationRetargetingMode", "").to_string(),
                _ => String::new(),
            };
            modes.push(m);
        }
    }
    Ok((s, modes))
}

/// Content package of an FPackageIndex: the outermost import's name ("/Game/X" -> "Mordhau/Content/X", "/Engine/X" ->
/// "Engine/Content/X"), or this package for an export
pub fn object_package(pkg: &RawPackage, idx: i32) -> String {
    if idx == 0 {
        return String::new();
    }
    if idx > 0 {
        return pkg.name.clone();
    }
    let mut i = (-idx - 1) as usize;
    for _ in 0..64 {
        let Some(im) = pkg.imports.get(i) else { return String::new() };
        if im.outer == 0 {
            let n = im.object_name.as_str();
            return if let Some(r) = n.strip_prefix("/Game/") {
                format!("Mordhau/Content/{r}")
            } else if let Some(r) = n.strip_prefix("/Engine/") {
                format!("Engine/Content/{r}")
            } else {
                n.trim_start_matches('/').to_string()
            };
        }
        if im.outer > 0 {
            return pkg.name.clone();
        }
        i = (-im.outer - 1) as usize;
    }
    String::new()
}

/// The target mesh reference pose used by FBoneContainer, without decoding vertex/LOD data.
/// Walks the same cooked header as `read_lods` through FReferenceSkeleton.
pub fn mesh_reference(src: &dyn PackageSource, pkg_path: &str) -> Result<RefSkeleton> {
    let pkg = src.package(pkg_path)?;
    let e = pkg.find_export("SkeletalMesh", None).ok_or_else(|| crate::AssetError(format!("{pkg_path}: no SkeletalMesh export")))?;
    let (start, end) = pkg.export_range(e)?;
    let mut r = Buf::at(&pkg.data, start);
    props::read_tagged(&pkg, &mut r, end);
    skip_object_guid(&mut r, e);
    r.skip(2 + 28); // FStripDataFlags, ImportedBounds
    let nm = r.s32();
    if !(0..=1024).contains(&nm) {
        return err(format!("{pkg_path}: {nm} materials"));
    }
    for _ in 0..nm {
        r.s32(); // MaterialInterface
        pkg.fname(&mut r); // MaterialSlotName
        if r.s32() != 0 {
            pkg.fname(&mut r); // ImportedMaterialSlotName
        }
        r.skip(24); // FMeshUVChannelInfo
    }
    read_ref_skeleton(&pkg, &mut r)
}

/// LOD 0 + reference skeleton of the package's SkeletalMesh export
pub fn lod0(src: &dyn PackageSource, pkg_path: &str) -> Result<SkeletalMesh> {
    match read_lods(src, pkg_path, 1)?.into_iter().next().flatten() {
        Some(m) => Ok(m),
        None => err(format!("{}: LOD 0 cooked out", pkg_path)),
    }
}

/// Every LOD (None = cooked out), each with the reference skeleton and materials. After LOD 0 the streamed data's
/// tail and the non-inlined availability info are skipped to reach the next LOD (FStaticLODModel.cs SerializeRenderItem
/// 407-516, SerializeStreamedData 577-672, SerializeAvailabilityInfo 674-720, UE 4.26 versions): adjacency index
/// buffer (class strip flag 1 clear), FSkinWeightProfilesData (TMap count); a LOD with cloth data stops the walk.
pub fn lods(src: &dyn PackageSource, pkg_path: &str) -> Result<Vec<Option<SkeletalMesh>>> {
    read_lods(src, pkg_path, usize::MAX)
}

fn read_lods(src: &dyn PackageSource, pkg_path: &str, max: usize) -> Result<Vec<Option<SkeletalMesh>>> {
    let pkg = src.package(pkg_path)?;
    let e = match pkg.find_export("SkeletalMesh", None) {
        Some(e) => e,
        None => return err(format!("{}: no SkeletalMesh export", pkg_path)),
    };
    let (start, end) = pkg.export_range(e)?;
    let mut r = Buf::at(&pkg.data, start);
    let properties = props::read_tagged(&pkg, &mut r, end);
    skip_object_guid(&mut r, e);
    r.skip(2); // FStripDataFlags
    r.skip(28); // ImportedBounds
    let nm = r.s32();
    if !(0..=1024).contains(&nm) {
        return err(format!("{}: {} materials", pkg_path, nm));
    }
    let mut materials = Vec::new();
    for _ in 0..nm {
        let mi = r.s32(); // MaterialInterface
        let slot = pkg.fname(&mut r); // MaterialSlotName
        if r.s32() != 0 {
            pkg.fname(&mut r); // bSerializeImportedMaterialSlotName
        }
        r.skip(24); // FMeshUVChannelInfo
        materials.push(SkelMaterial { slot, material: pkg.object_name(mi).to_string(), material_package: object_package(&pkg, mi) });
    }
    let ref_skeleton = read_ref_skeleton(&pkg, &mut r)?;
    if r.s32() == 0 {
        return err(format!("{}: not cooked", pkg_path)); // bCooked
    }
    let nl = r.s32();
    if !(1..=16).contains(&nl) {
        return err(format!("{}: {} LODs", pkg_path, nl));
    }
    let colors = props::get_bool(&properties, "bHasVertexColors", false);
    let mut out = Vec::new();
    for i in 0..(nl as usize).min(max) {
        let more = i + 1 < (nl as usize).min(max);
        let m = lod(src, &pkg, &mut r, colors, more)?.map(|mut m| {
            m.package = pkg.name.clone();
            m.ref_skeleton = ref_skeleton.clone();
            m.materials = materials.clone();
            m.properties = properties.clone();
            m
        });
        out.push(m);
    }
    Ok(out)
}

fn lod(src: &dyn PackageSource, pkg: &RawPackage, r: &mut Buf, colors: bool, more: bool) -> Result<Option<SkeletalMesh>> {
    let (_, class_strip_lod) = (r.u8(), r.u8()); // FStripDataFlags
    let cooked_out = r.s32() != 0;
    let inlined = r.s32() != 0;
    let n = r.s32();
    r.skip_n(n, 2); // RequiredBones
    if cooked_out {
        return Ok(None);
    }
    let ns = r.s32();
    if !(0..=1024).contains(&ns) {
        return err(format!("{}: {} sections", pkg.name, ns));
    }
    let mut sections = Vec::new();
    let mut has_cloth = false;
    for _ in 0..ns {
        r.skip(1); // FStripDataFlags: global
        let class_strip = r.u8();
        let material_index = r.s16();
        let base_index = r.s32();
        let num_triangles = r.s32();
        r.skip(4 + 1 + 4); // bRecomputeTangent, RecomputeTangentsVertexMaskChannel, bCastShadow
        let base_vertex_index = r.u32();
        let n = r.s32();
        r.skip_n(n, 64); // ClothMappingData: FMeshToMeshVertData
        has_cloth |= n > 0;
        let nb = r.s32();
        if nb < 0 || nb as usize > r.left() {
            return err(format!("{}: bone map of {}", pkg.name, nb));
        }
        let bone_map = (0..nb).map(|_| r.u16()).collect();
        let num_vertices = r.s32();
        let max_bone_influences = r.s32();
        r.skip(2 + 20); // CorrespondClothAssetIndex, FClothingSectionData
        if class_strip & 1 == 0 {
            // duplicated vertices (UE 4.23+): DupVertData int32[], DupVertIndexData {int32, int32}[]
            let a = r.s32();
            r.skip_n(a, 4);
            let b = r.s32();
            r.skip_n(b, 8);
        }
        r.skip(4); // bDisabled
        sections.push(SkelSection { material_index, base_index, num_triangles, base_vertex_index, num_vertices,
            max_bone_influences, bone_map });
    }
    let n = r.s32();
    r.skip_n(n, 2); // ActiveBoneIndices
    r.skip(4); // BuffersSize
    if r.bad {
        return err(format!("{}: LOD header read past the export", pkg.name));
    }
    if has_cloth && more {
        return err(format!("{}: LOD with cloth data (FSkeletalMeshVertexClothBuffer not decoded): later LODs unreachable", pkg.name));
    }
    let mut m = if inlined {
        let m = streamed(&pkg.name, r, colors)?;
        if more {
            streamed_tail(r);
        }
        m
    } else {
        let h = bulk::header(pkg, r);
        let b = bulk::bytes(src, pkg, &h)?;
        // SerializeAvailabilityInfo: index container meta 1 + 4 (+ 1 + 4 adjacency), static mesh vertex buffer 16,
        // position 8, colour 8, skin weights FSkinWeightVertexBuffer::MetadataSize (4.26: 4 x 4 + 4 + 4)
        let adj = if class_strip_lod & 1 == 0 { 5 } else { 0 };
        r.skip(5 + adj + 16 + 8 + 8 + 24);
        if h.size == 0 {
            return Ok(None);
        }
        streamed(&pkg.name, &mut Buf::new(&b), colors)?
    };
    if r.bad {
        return err(format!("{}: LOD read past the export", pkg.name));
    }
    m.sections = sections;
    to_ref_bones(&mut m);
    Ok(Some(m))
}

/// The streamed data after the colour buffer (inline LODs, to reach the next LOD): AdjacencyIndexBuffer
/// (FMultisizeIndexContainer, when the streamed strip flags keep adjacency), FSkinWeightProfilesData (TMap<FName, ..>;
/// every Mordhau mesh read so far has 0 entries)
fn streamed_tail(r: &mut Buf) {
    if r.p >= r.b.len() {
        return;
    }
    // the streamed FStripDataFlags were consumed by `streamed`; its class flag is kept in STREAM_CLASS_STRIP
    if STREAM_CLASS_STRIP.with(|c| c.get()) & 1 == 0 {
        r.u8();
        r.bulk_array(None);
    }
    r.s32(); // FSkinWeightProfilesData count
}

thread_local! {
    static STREAM_CLASS_STRIP: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

fn streamed(name: &str, r: &mut Buf, colors: bool) -> Result<SkeletalMesh> {
    r.u8(); // FStripDataFlags: global
    let cs = r.u8();
    STREAM_CLASS_STRIP.with(|c| c.set(cs));
    let isz = r.u8(); // FMultisizeIndexContainer: 2 = uint16, 4 = uint32
    let (_, raw) = r.bulk_array(None);
    let indices = static_mesh::indices(raw, isz == 4);
    let positions = static_mesh::read_positions(r);
    let mut v = match static_mesh::read_vertex_buffer(r, positions.len()) {
        Some(v) if !r.bad => v,
        _ => return err(format!("{}: vertex buffer", name)),
    };
    v.positions = positions;
    // FSkinWeightVertexBuffer, 4.26 (UnlimitedBoneInfluences format)
    r.skip(2);
    let variable = r.s32() != 0;
    let max_inf = r.u32() as usize;
    r.u32(); // NumBones
    let nv = r.u32() as usize;
    let b16 = r.s32() != 0;
    let (_, data) = r.bulk_array(None);
    r.skip(2); // lookup FStripDataFlags
    r.s32(); // NumLookupVertices
    let (_, lraw) = r.bulk_array(None);
    let lookup: Vec<u32> = lraw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let cols = if colors { static_mesh::read_colors(r) } else { Vec::new() };
    if r.bad || nv != v.positions.len() || (variable && lookup.len() < nv) {
        return err(format!("{}: skin weights do not decode", name));
    }
    let per = if max_inf > NUM_INFLUENCES_UE4 { 8 } else { NUM_INFLUENCES_UE4 };
    let mut bones = vec![0u16; nv * per];
    let mut weights = vec![0f32; nv * per];
    let ib = if b16 { 2 } else { 1 };
    let mut p = 0usize;
    for vi in 0..nv {
        let mut n = per;
        if variable {
            p = ((lookup[vi] >> 8) & 0xff_ffff) as usize;
            n = (lookup[vi] & 0xff) as usize;
        }
        if n > per || p + n * ib + n > data.len() {
            return err(format!("{}: skin weights of vertex {} beyond the buffer", name, vi));
        }
        for k in 0..n {
            bones[vi * per + k] =
                if b16 { u16::from_le_bytes([data[p + k * 2], data[p + k * 2 + 1]]) } else { data[p + k] as u16 };
            weights[vi * per + k] = data[p + n * ib + k] as f32 / 255.0;
        }
        p += n * ib + n;
    }
    Ok(SkeletalMesh { vertices: v, colors: cols, indices, bones, weights, max_influences: per, ..Default::default() })
}

/// section-local bone indices -> reference skeleton indices through each section's BoneMap (out of range -> 0, as the
/// GDScript does)
fn to_ref_bones(m: &mut SkeletalMesh) {
    let per = m.max_influences;
    for s in &m.sections {
        let lo = s.base_vertex_index as usize;
        let hi = (lo + s.num_vertices.max(0) as usize).min(m.bones.len() / per);
        for i in lo * per..hi * per {
            let local = m.bones[i] as usize;
            m.bones[i] = s.bone_map.get(local).copied().unwrap_or(0);
        }
    }
}

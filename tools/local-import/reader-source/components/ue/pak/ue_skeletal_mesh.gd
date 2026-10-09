# ue_skeletal_mesh.gd - a cooked UE 4.26 SkeletalMesh read from the paks: the reference skeleton and LOD 0 (sections,
# positions, normals, UVs, bone influences, indices), and a Godot ArrayMesh + Skeleton3D built from them with the
# conversion the glTF exports use, without the .glb that `mdx export` writes to extract/gltf.
#
# Layout (CUE4Parse at the pinned commit, tools/CUE4Parse-src/CUE4Parse/UE4/Assets/Exports/SkeletalMesh/..., custom
# versions at their UE 4.26 values as Versions/*.cs default them):
#   USkeletalMesh (USkeletalMesh.cs:24-172): tagged properties, UObject Guid, FStripDataFlags, FBoxSphereBounds
#     ImportedBounds (28 bytes), TArray<FSkeletalMaterial>, FReferenceSkeleton (UeAsset.ref_skeleton), bool bCooked,
#     int32 LOD count, per LOD FSkeletalMeshLODRenderData (FStaticLODModel.SerializeRenderItem, FStaticLODModel.cs:408-516).
#   FSkeletalMaterial (FSkeletalMaterial.cs): FPackageIndex MaterialInterface, FName MaterialSlotName, bool
#     bSerializeImportedMaterialSlotName (+ FName), FMeshUVChannelInfo (bool, bool, float[4]: 24 bytes).
#   LOD: FStripDataFlags, bool bIsLODCookedOut, bool bInlined, TArray<int16> RequiredBones, TArray<FSkelMeshSection>,
#     TArray<int16> ActiveBoneIndices, uint32 BuffersSize, then the streamed data inline or in an FByteBulkData (UeBulk).
#   FSkelMeshSection render item (FSkelMeshSection.cs:245-319): FStripDataFlags, int16 MaterialIndex, int32 BaseIndex,
#     NumTriangles, bool bRecomputeTangent, uint8 RecomputeTangentsVertexMaskChannel, bool bCastShadow, uint32
#     BaseVertexIndex, TArray<FMeshToMeshVertData> (64 bytes each), TArray<uint16> BoneMap, int32 NumVertices,
#     MaxBoneInfluences, int16 CorrespondClothAssetIndex, FClothingSectionData (FGuid + int32), duplicated-vertex
#     arrays (unless class strip flag 1), bool bDisabled.
#   Streamed data (FStaticLODModel.SerializeStreamedData): FStripDataFlags, FMultisizeIndexContainer (uint8 size + bulk
#     indices), FPositionVertexBuffer, FStaticMeshVertexBuffer (UeStaticMesh readers), FSkinWeightVertexBuffer (4.26
#     format, FSkinWeightVertexBuffer.cs:19-128: FStripDataFlags, bool bVariableBonesPerVertex, uint32
#     MaxBoneInfluences, NumBones, NumVertices, bool bUse16BitBoneIndex, bulk bytes; FStripDataFlags, int32
#     NumLookupVertices, bulk uint32 lookup (offset << 8 | count) for variable influences). Per vertex: the bone
#     indices (into the section's BoneMap), then the weights, one byte each (FSkinWeightInfo).
class_name UeSkeletalMesh

const NUM_INFLUENCES_UE4 := 4		# FSkinWeightVertexBuffer.cs:10; 8 (EXTRA_BONE_INFLUENCES) when MaxBoneInfluences > 4

# {RefSkeleton, Materials: [{Slot, Material (object name)}], Sections: [{MaterialIndex, BaseIndex, NumTriangles, BaseVertexIndex, NumVertices, BoneMap}], Positions,
# Normals, UVs, Indices, Influences: [PackedInt32Array bone (ref skeleton index), PackedFloat32Array weight] per vertex
# as two flat arrays of `max_influences` entries, MaxInfluences, Properties}, or {}.
static func lod0(pkg_path: String) -> Dictionary:
	var a := UeAsset.open(pkg_path)
	if a == null:
		return {}
	for e in a.exports:
		var cn = a.node(e.cls)
		if cn == null or UeAsset.node_name(cn) != "SkeletalMesh":
			continue
		var end: int = e.off + e.size
		var r := UePakBuf.new(a.data())
		r.p = e.off
		var props := a.tagged(r, end)
		if (e.flags & UeAsset.RF_CLASS_DEFAULT_OBJECT) == 0 and r.s32() != 0:
			r.skip(16)					# UObject Guid
		r.skip(2)						# FStripDataFlags
		r.skip(28)						# ImportedBounds
		var mats := []
		for i in r.s32():				# FSkeletalMaterial
			var mn = a.node(r.s32())	# MaterialInterface
			var slot := a.fname(r)		# MaterialSlotName
			if r.s32() != 0:			# bSerializeImportedMaterialSlotName
				a.fname(r)
			r.skip(24)					# FMeshUVChannelInfo
			mats.append({"Slot": slot, "Material": UeAsset.node_name(mn) if mn != null else ""})
		var skel := a.ref_skeleton(r, end)
		if r.s32() == 0 or r.s32() < 1:	# bCooked, LOD count
			return {}
		var out := _lod(a, r, pkg_path, bool(props.get("bHasVertexColors", false)))
		if out.is_empty():
			return {}
		out.RefSkeleton = skel
		out.Materials = mats
		out.Properties = props
		return out
	return {}

static func _lod(a: UeAsset, r: UePakBuf, pkg_path: String, colors: bool) -> Dictionary:
	r.skip(2)							# FStripDataFlags
	var cooked_out := r.s32() != 0
	var inlined := r.s32() != 0
	r.skip(2 * r.s32())					# RequiredBones
	if cooked_out:
		return {}
	var sections := []
	for i in r.s32():
		r.skip(1)						# FStripDataFlags: global
		var class_strip := r.u8()
		var s := {"MaterialIndex": r.s16(), "BaseIndex": r.s32(), "NumTriangles": r.s32()}
		r.skip(4 + 1 + 4)				# bRecomputeTangent, RecomputeTangentsVertexMaskChannel, bCastShadow
		s.BaseVertexIndex = r.u32()
		r.skip(64 * r.s32())			# ClothMappingData: FMeshToMeshVertData
		var bm := PackedInt32Array()
		bm.resize(r.s32())
		for k in bm.size():
			bm[k] = r.u16()
		s.BoneMap = bm
		s.NumVertices = r.s32()
		s.MaxBoneInfluences = r.s32()
		r.skip(2 + 20)					# CorrespondClothAssetIndex, FClothingSectionData
		if (class_strip & 1) == 0:		# duplicated vertices (UE 4.23+): DupVertData int32[], DupVertIndexData {int32, int32}[]
			r.skip(4 * r.s32())
			r.skip(8 * r.s32())
		r.skip(4)						# bDisabled
		sections.append(s)
	r.skip(2 * r.s32())					# ActiveBoneIndices
	r.skip(4)							# BuffersSize
	if r.bad:
		return {}
	var out := {}
	if inlined:
		out = _streamed(r, colors)
	else:
		var b := UeBulk.bytes(pkg_path, UeBulk.header(a, r))
		if b.is_empty():
			return {}
		out = _streamed(UePakBuf.new(b), colors)
	if out.is_empty():
		return {}
	out.Sections = sections
	_to_ref_bones(out)
	return out

static func _streamed(r: UePakBuf, colors: bool) -> Dictionary:
	r.skip(2)							# FStripDataFlags
	var isz := r.u8()					# FMultisizeIndexContainer: 2 = uint16, 4 = uint32
	var esz := r.s32()
	var cnt := r.s32()
	var idx := UeStaticMesh.indices(r.bytes(esz * cnt), isz == 4)
	var pos := UeStaticMesh.read_positions(r)
	var vb := UeStaticMesh.read_vertex_buffer(r, pos.size())
	if vb.is_empty() or r.bad:
		return {}
	# FSkinWeightVertexBuffer, 4.26 (UnlimitedBoneInfluences format)
	r.skip(2)
	var variable := r.s32() != 0
	var max_inf := r.u32()
	r.u32()								# NumBones
	var nv := r.u32()
	var b16 := r.s32() != 0
	esz = r.s32()
	cnt = r.s32()
	var data := r.bytes(esz * cnt)
	r.skip(2)							# lookup FStripDataFlags
	r.s32()								# NumLookupVertices
	esz = r.s32()
	cnt = r.s32()
	var lookup := r.bytes(esz * cnt).to_int32_array()
	if colors:
		UeStaticMesh.read_colors(r)
	if r.bad or nv != pos.size():
		return {}
	var per := 8 if max_inf > NUM_INFLUENCES_UE4 else NUM_INFLUENCES_UE4
	var bones := PackedInt32Array()
	var weights := PackedFloat32Array()
	bones.resize(nv * per)
	weights.resize(nv * per)
	var ib := 2 if b16 else 1
	var p := 0
	for v in nv:
		var n := per
		if variable:
			p = (lookup[v] >> 8) & 0xffffff
			n = lookup[v] & 0xff
		for k in n:
			bones[v * per + k] = data.decode_u16(p + k * 2) if b16 else data[p + k]
			weights[v * per + k] = data[p + n * ib + k] / 255.0
		p += n * ib + n
	return {"Positions": pos, "Normals": vb.Normals, "UVs": vb.UVs, "Indices": idx, "Bones": bones, "Weights": weights,
		"MaxInfluences": per, "HighPrecisionTangents": vb.HighPrecisionTangents}

# section-local bone indices -> reference skeleton indices through each section's BoneMap
static func _to_ref_bones(m: Dictionary) -> void:
	var per: int = m.MaxInfluences
	var bones: PackedInt32Array = m.Bones
	for s in m.Sections:
		var bm: PackedInt32Array = s.BoneMap
		for v in range(s.BaseVertexIndex, s.BaseVertexIndex + s.NumVertices):
			for k in per:
				var i := v * per + k
				var local := bones[i]
				bones[i] = bm[local] if local < bm.size() else 0
	m.Bones = bones

# UE bone transform -> Godot, as the glTF writer does (Gltf.cs:129-131: SwapYZ of rotation (X, Z, Y, -W), translation
# * 0.01, scale)
static func bone_rest(t: Dictionary) -> Transform3D:
	var q: Dictionary = t.Rotation
	var p: Dictionary = t.Translation
	var s: Dictionary = t.Scale3D
	var rot := Quaternion(q.X, q.Z, q.Y, -q.W).normalized()
	var basis := Basis(rot).scaled(Vector3(s.X, s.Z, s.Y))
	return Transform3D(basis, Vector3(p.X, p.Z, p.Y) * UeStaticMesh.UNIT_SCALE)

# Skeleton3D with the reference skeleton's bones, names and rest poses
static func skeleton(m: Dictionary) -> Skeleton3D:
	var sk := Skeleton3D.new()
	var info: Array = m.RefSkeleton.FinalRefBoneInfo
	var pose: Array = m.RefSkeleton.FinalRefBonePose
	for i in info.size():
		sk.add_bone(String(info[i].Name))
	for i in info.size():
		if int(info[i].ParentIndex) >= 0:
			sk.set_bone_parent(i, int(info[i].ParentIndex))
		var rest := bone_rest(pose[i])
		sk.set_bone_rest(i, rest)
		sk.set_bone_pose(i, rest)
	return sk

# ArrayMesh of LOD 0 (one surface per section, influences as ARRAY_BONES / ARRAY_WEIGHTS in reference-skeleton bone
# indices, 8 per vertex with ARRAY_FLAG_USE_8_BONE_WEIGHTS when the mesh uses more than 4), or null
static func array_mesh(m: Dictionary) -> ArrayMesh:
	if m.is_empty():
		return null
	var per: int = m.MaxInfluences
	var mesh := ArrayMesh.new()
	for s in m.Sections:
		var lo: int = s.BaseVertexIndex
		var hi: int = lo + s.NumVertices
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		var v := PackedVector3Array()
		for i in range(lo, hi):
			v.append(UeStaticMesh.to_godot(m.Positions[i]))
		arrays[Mesh.ARRAY_VERTEX] = v
		if not m.Normals.is_empty():
			var n := PackedVector3Array()
			for i in range(lo, hi):
				n.append(UeStaticMesh.to_godot(m.Normals[i]).normalized())
			arrays[Mesh.ARRAY_NORMAL] = n
		if m.UVs.size() > 0:
			arrays[Mesh.ARRAY_TEX_UV] = m.UVs[0].slice(lo, hi)
		arrays[Mesh.ARRAY_BONES] = m.Bones.slice(lo * per, hi * per)
		var w: PackedFloat32Array = m.Weights.slice(lo * per, hi * per)
		for i in range(0, w.size(), per):		# UE byte weights sum to 255; Godot wants 1
			var sum := 0.0
			for k in per: sum += w[i + k]
			if sum > 0.0:
				for k in per: w[i + k] /= sum
		arrays[Mesh.ARRAY_WEIGHTS] = w
		var ix := PackedInt32Array()
		ix.resize(s.NumTriangles * 3)
		for t in s.NumTriangles:
			var b: int = s.BaseIndex + t * 3
			ix[t * 3] = m.Indices[b] - lo
			ix[t * 3 + 1] = m.Indices[b + 2] - lo
			ix[t * 3 + 2] = m.Indices[b + 1] - lo
		arrays[Mesh.ARRAY_INDEX] = ix
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays, [], {},
			Mesh.ARRAY_FLAG_USE_8_BONE_WEIGHTS if per == 8 else 0)
	return mesh

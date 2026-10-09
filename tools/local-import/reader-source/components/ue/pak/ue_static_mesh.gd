# ue_static_mesh.gd - a cooked UE 4.26 StaticMesh read from the paks: LOD 0's sections, positions, normals, UVs, vertex
# colours and indices, and a Godot ArrayMesh built from them with the same conversion the glTF exports use, without the
# .glb that `mdx export` writes to extract/gltf. Only LOD 0 is decoded (the export walk stops there).
#
# Layout (CUE4Parse at the pinned commit, tools/CUE4Parse-src/CUE4Parse/UE4/..., versions as VersionContainer.cs:78-99
# sets them for UE 4.26):
#   UStaticMesh (Assets/Exports/StaticMesh/UStaticMesh.cs): tagged properties, UObject Guid (bool + FGuid), FStripDataFlags
#     (2 bytes), bool bCooked, FPackageIndex BodySetup, FPackageIndex NavCollision (StaticMesh.HasNavCollision), FGuid
#     LightingGuid, TArray<FPackageIndex> Sockets, then FStaticMeshRenderData: TArray<FStaticMeshLODResources>.
#   FStaticMeshLODResources (FStaticMeshLODResources.cs:45-104): FStripDataFlags, TArray<FStaticMeshSection>, float
#     MaxDeviation, bool bIsLODCookedOut, bool bInlined, then the buffers inline or in an FByteBulkData (UeBulk).
#   FStaticMeshSection (FStaticMeshSection.cs): int32 MaterialIndex, FirstIndex, NumTriangles, MinVertexIndex,
#     MaxVertexIndex, bool bEnableCollision, bCastShadow, bForceOpaque, bVisibleInRayTracing (4.26).
#   SerializeBuffers (FStaticMeshLODResources.cs:314-393): FStripDataFlags; FPositionVertexBuffer {int32 Stride,
#     NumVertices, bulk FVector[]}; FStaticMeshVertexBuffer {FStripDataFlags, int32 NumTexCoords, NumVertices, bool
#     bUseFullPrecisionUVs, bUseHighPrecisionTangentBasis, bulk tangents (TangentX, TangentZ), bulk UVs}
#     (FStaticMeshVertexBuffer.cs); FColorVertexBuffer {FStripDataFlags, int32 Stride, NumVertices, bulk FColor[] if any}
#     (FColorVertexBuffer.cs); FRawStaticIndexBuffer {bool b32Bit, bulk bytes, bool bShouldExpandTo32Bit}
#     (FRawStaticIndexBuffer.cs). A "bulk" array is int32 element size + int32 count + the elements (ReadBulkArray).
# FPackedNormal (Objects/Meshes/FPackedNormal.cs): 4 bytes, each ^ 0x80 (IncreaseNormalPrecision) then / 127.5 - 1;
# with bUseHighPrecisionTangentBasis, FPackedRGBA16N (4 x uint16, see read_vertex_buffer).
class_name UeStaticMesh

const UNIT_SCALE := 0.01		# cm -> m, CUE4Parse-Conversion/Writers/Gltf/Gltf.cs:25 (UnitScale)

# LOD 0 of the package's StaticMesh export: {Sections: [{MaterialIndex, FirstIndex, NumTriangles, MinVertexIndex,
# MaxVertexIndex}], Positions (UE cm, PackedVector3Array), Normals (TangentZ, or empty), UVs: [PackedVector2Array per
# channel], Colors (PackedColorArray, or empty), Indices (PackedInt32Array), Properties}, or {} when not decodable.
static func lod0(pkg_path: String) -> Dictionary:
	var a := UeAsset.open(pkg_path)
	if a == null:
		return {}
	for e in a.exports:
		var cn = a.node(e.cls)
		if cn == null or UeAsset.node_name(cn) != "StaticMesh":
			continue
		var r := UePakBuf.new(a.data())
		r.p = e.off
		var props := a.tagged(r, e.off + e.size)
		if (e.flags & UeAsset.RF_CLASS_DEFAULT_OBJECT) == 0 and r.s32() != 0:
			r.skip(16)					# UObject Guid
		r.skip(2)						# FStripDataFlags
		if r.s32() == 0:				# bCooked
			return {}
		r.skip(4 + 4)					# BodySetup, NavCollision
		r.skip(16)						# LightingGuid
		r.skip(4 * r.s32())				# Sockets
		if r.s32() < 1:					# LOD count
			return {}
		var out := _lod(a, r, pkg_path)
		if not out.is_empty():
			out.Properties = props
		return out
	return {}

static func _lod(a: UeAsset, r: UePakBuf, pkg_path: String) -> Dictionary:
	r.skip(2)							# FStripDataFlags
	var sections := []
	for i in r.s32():
		var s := {"MaterialIndex": r.s32(), "FirstIndex": r.s32(), "NumTriangles": r.s32(), "MinVertexIndex": r.s32(),
			"MaxVertexIndex": r.s32()}
		r.skip(4 * 4)					# bEnableCollision, bCastShadow, bForceOpaque, bVisibleInRayTracing
		sections.append(s)
	r.f32()								# MaxDeviation
	var cooked_out := r.s32() != 0
	var inlined := r.s32() != 0
	if cooked_out or r.bad:
		return {}
	var out := {}
	if inlined:
		out = _buffers(r)
	else:
		var b := UeBulk.bytes(pkg_path, UeBulk.header(a, r))
		if b.is_empty():
			return {}
		out = _buffers(UePakBuf.new(b))
	if out.is_empty():
		return {}
	out.Sections = sections
	return out

static func _buffers(r: UePakBuf) -> Dictionary:
	r.skip(2)							# FStripDataFlags
	var pos := read_positions(r)
	var vb := read_vertex_buffer(r, pos.size())
	if vb.is_empty():
		return {}
	var col := read_colors(r)
	# FRawStaticIndexBuffer
	var is32 := r.s32() != 0
	var esz := r.s32()
	var cnt := r.s32()
	var raw := r.bytes(esz * cnt)
	r.skip(4)							# bShouldExpandTo32Bit
	var idx := indices(raw, is32)
	if r.bad:
		return {}
	return {"Positions": pos, "Normals": vb.Normals, "UVs": vb.UVs, "Colors": col, "Indices": idx,
		"HighPrecisionTangents": vb.HighPrecisionTangents}

# FPositionVertexBuffer {int32 Stride, NumVertices, bulk FVector[]} (also used by skeletal meshes); UE cm
static func read_positions(r: UePakBuf) -> PackedVector3Array:
	r.skip(4)							# Stride
	var nv := r.s32()
	var esz := r.s32()
	var cnt := r.s32()
	var pos := PackedVector3Array()
	if esz != 12 or cnt != nv or nv < 0:
		r.bad = true
		return pos
	pos.resize(nv)
	for i in nv:
		pos[i] = Vector3(r.f32raw(), r.f32raw(), r.f32raw())
	return pos

# FStaticMeshVertexBuffer: {Normals (TangentZ, FPackedNormal or high-precision FPackedRGBA16N), UVs: [PackedVector2Array]}
static func read_vertex_buffer(r: UePakBuf, nv: int) -> Dictionary:
	r.skip(2)							# FStripDataFlags
	var ntex := r.s32()
	if r.s32() != nv:
		return {}
	var full_uv := r.s32() != 0
	var high_tan := r.s32() != 0
	var esz := r.s32()
	var cnt := r.s32()
	var nrm := PackedVector3Array()
	if not high_tan and esz == 8:
		nrm.resize(cnt)
		for i in cnt:
			r.skip(4)					# TangentX
			var z := r.u32() ^ 0x80808080	# TangentZ = the normal
			nrm[i] = Vector3((z & 0xff) / 127.5 - 1.0, ((z >> 8) & 0xff) / 127.5 - 1.0, ((z >> 16) & 0xff) / 127.5 - 1.0)
	elif high_tan and esz == 16:
		# FPackedRGBA16N (Objects/RenderCore/FPackedRGBA16N.cs): 4 x uint16, each ^ 0x8000 (UE 4.20+), then
		# (v - 32767.5) / 32767.5
		nrm.resize(cnt)
		for i in cnt:
			r.skip(8)					# TangentX
			var x := r.u16() ^ 0x8000
			var y := r.u16() ^ 0x8000
			var z := r.u16() ^ 0x8000
			r.skip(2)					# W (binormal sign)
			nrm[i] = Vector3((x - 32767.5) / 32767.5, (y - 32767.5) / 32767.5, (z - 32767.5) / 32767.5)
	else:
		r.skip(esz * cnt)				# an item size no tangent format has: normals left empty
	esz = r.s32()
	cnt = r.s32()
	var uvs := []
	for c in ntex:
		var u := PackedVector2Array()
		u.resize(nv)
		uvs.append(u)
	for i in nv:
		for c in ntex:
			uvs[c][i] = Vector2(r.f32raw(), r.f32raw()) if full_uv else Vector2(r.f16(), r.f16())
	return {"Normals": nrm, "UVs": uvs, "HighPrecisionTangents": high_tan}

# FColorVertexBuffer {FStripDataFlags, int32 Stride, NumVertices, bulk FColor[] when NumVertices > 0}
static func read_colors(r: UePakBuf) -> PackedColorArray:
	r.skip(2)
	r.skip(4)							# Stride
	var ncol := r.s32()
	var col := PackedColorArray()
	if ncol > 0:
		r.skip(8)						# bulk element size + count
		col.resize(ncol)
		for i in ncol:
			var bgra := r.u32()			# FColor in memory order B, G, R, A
			col[i] = Color8((bgra >> 16) & 0xff, (bgra >> 8) & 0xff, bgra & 0xff, (bgra >> 24) & 0xff)
	return col

static func indices(raw: PackedByteArray, is32: bool) -> PackedInt32Array:
	if is32:
		return raw.to_int32_array()
	var idx := PackedInt32Array()
	idx.resize(raw.size() / 2)
	for i in idx.size():
		idx[i] = raw.decode_u16(i * 2)
	return idx

# UE (cm, Z up) -> Godot (m, Y up): (X, Z, Y) * 0.01, as CUE4Parse's glTF writer does (Gltf.cs:244-250 SwapYZ, :72)
static func to_godot(v: Vector3) -> Vector3:
	return Vector3(v.x, v.z, v.y) * UNIT_SCALE

# A Godot ArrayMesh of LOD 0, one surface per section (in section order, like the .glb), or null. Triangles keep the
# glTF export's corner order (i0, i1, i2), reversed to (i0, i2, i1) as Godot's glTF importer does for its clockwise
# front faces.
static func array_mesh(pkg_path: String) -> ArrayMesh:
	var m := lod0(pkg_path)
	if m.is_empty():
		return null
	var pos: PackedVector3Array = m.Positions
	var nrm: PackedVector3Array = m.Normals
	var uvs: Array = m.UVs
	var col: PackedColorArray = m.Colors
	var idx: PackedInt32Array = m.Indices
	var mesh := ArrayMesh.new()
	for s in m.Sections:
		var lo: int = s.MinVertexIndex
		var hi: int = s.MaxVertexIndex
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		var v := PackedVector3Array()
		var n := PackedVector3Array()
		for i in range(lo, hi + 1):
			v.append(to_godot(pos[i]))
			if not nrm.is_empty():
				n.append(to_godot(nrm[i]).normalized())
		arrays[Mesh.ARRAY_VERTEX] = v
		if not nrm.is_empty():
			arrays[Mesh.ARRAY_NORMAL] = n
		if uvs.size() > 0:
			arrays[Mesh.ARRAY_TEX_UV] = uvs[0].slice(lo, hi + 1)
		if uvs.size() > 1:
			arrays[Mesh.ARRAY_TEX_UV2] = uvs[1].slice(lo, hi + 1)
		if not col.is_empty():
			arrays[Mesh.ARRAY_COLOR] = col.slice(lo, hi + 1)
		var ix := PackedInt32Array()
		ix.resize(s.NumTriangles * 3)
		for t in s.NumTriangles:
			var b: int = s.FirstIndex + t * 3
			ix[t * 3] = idx[b] - lo
			ix[t * 3 + 1] = idx[b + 2] - lo
			ix[t * 3 + 2] = idx[b + 1] - lo
		arrays[Mesh.ARRAY_INDEX] = ix
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh

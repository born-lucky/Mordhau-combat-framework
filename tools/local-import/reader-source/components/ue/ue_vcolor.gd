# ue_vcolor.gd - per-component painted vertex colors (UE mesh paint: UStaticMeshComponent LODData[0].OverrideVertexColors).
# Data: `mdx vcolors` -> <data_path>/json/<level package>.vcolors.json, keyed "<actor>.<component>" (the same name
# UeLevel.read gives each mesh record), with the static mesh's LOD0 positions (UE cm, UE axes) and RGBA8 colors.
# CUE4Parse's glTF writer re-indexes vertices (SharpGLTF MeshBuilder), so colors are matched to the imported surface
# vertices by position: glTF position = SwapYZ(UE) x 0.01 (CUE4Parse Gltf.cs SwapYZ/UnitScale, as in ue_level.gd).
# Each painted instance gets its own ArrayMesh copy with ARRAY_COLOR replaced (Mesh.surface_get_arrays /
# ArrayMesh.add_surface_from_arrays: https://docs.godotengine.org/en/stable/classes/class_arraymesh.html). Colors are
# FColor/255 with no sRGB conversion, the way UE feeds FColor vertex colors to the VertexColor material node.
class_name UeVColor

static var _cache := {}

# All records of the map's levels (UeLevel.levels), {} when `mdx vcolors` has not been run.
# Marshalls.base64_to_raw: https://docs.godotengine.org/en/stable/classes/class_marshalls.html
static func records(map_pkg: String) -> Dictionary:
	if _cache.has(map_pkg):
		return _cache[map_pkg]
	var out := {}
	for lv in UeLevel.levels(map_pkg):
		var f := UePkg.root() + "/json/" + String(lv[0]) + ".vcolors.json"
		if not FileAccess.file_exists(f):
			continue
		var d = JSON.parse_string(FileAccess.get_file_as_string(f))
		if d is Dictionary:
			for k in d:
				out[String(k).validate_node_name()] = d[k]
	_cache[map_pkg] = out
	return out

static func _key(p: Vector3) -> Vector3i:
	return Vector3i((p * 10000.0).round())   # 0.1 mm

# Copy of `mesh` with ARRAY_COLOR from `rec`. Returns null if any vertex has no position match.
static func painted_mesh(mesh: ArrayMesh, rec: Dictionary) -> ArrayMesh:
	var pos := Marshalls.base64_to_raw(rec.pos).to_float32_array()
	var rgba := Marshalls.base64_to_raw(rec.rgba)
	var by_pos := {}
	for i in int(rec.n):
		var g := Vector3(pos[i * 3], pos[i * 3 + 2], pos[i * 3 + 1]) * 0.01
		by_pos[_key(g)] = Color8(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2], rgba[i * 4 + 3])
	var out := ArrayMesh.new()
	for s in mesh.get_surface_count():
		var a := mesh.surface_get_arrays(s)
		var v: PackedVector3Array = a[Mesh.ARRAY_VERTEX]
		var cols := PackedColorArray()
		cols.resize(v.size())
		for i in v.size():
			var k := _key(v[i])
			if not by_pos.has(k):
				return null
			cols[i] = by_pos[k]
		a[Mesh.ARRAY_COLOR] = cols
		# Keep the custom channel formats (TEXCOORD_2 -> CUSTOM0 as float RG, the lightmap UV of LightMapCoordinateIndex 2
		# meshes); without them add_surface_from_arrays expects RGBA8 bytes. Flags: ArrayMesh.add_surface_from_arrays,
		# Mesh.ARRAY_FORMAT_CUSTOM_BASE / _BITS / _MASK (class_mesh.html).
		var fmt := mesh.surface_get_format(s)
		var flags := fmt & Mesh.ARRAY_FLAG_USE_8_BONE_WEIGHTS
		for ci in 4:
			flags |= fmt & (Mesh.ARRAY_FORMAT_CUSTOM_MASK << (Mesh.ARRAY_FORMAT_CUSTOM_BASE + ci * Mesh.ARRAY_FORMAT_CUSTOM_BITS))
		out.add_surface_from_arrays(mesh.surface_get_primitive_type(s), a, [], {}, flags)
		out.surface_set_material(s, mesh.surface_get_material(s))
		out.surface_set_name(s, mesh.surface_get_name(s))
	return out

# Runtime: give every painted mesh instance under root its own colors. Returns instances painted.
static func apply_all(root: Node, map_pkg: String) -> int:
	var recs := records(map_pkg)
	if recs.is_empty():
		return 0
	var c := 0
	for n in root.find_children("*", "Node3D", true, false):
		if not n.has_meta("ue_mesh"):
			continue
		var key: String = String(n.get_meta("ue_comp", n.name)).validate_node_name()
		if not recs.has(key):
			continue
		for mi in n.find_children("*", "MeshInstance3D", true, false) + ([n] if n is MeshInstance3D else []):
			if not (mi.mesh is ArrayMesh):
				continue
			var pm := painted_mesh(mi.mesh, recs[key])
			if pm == null:
				push_warning("UeVColor: %s vertices do not match %s" % [key, recs[key].get("mesh", "")])
				continue
			mi.mesh = pm
			c += 1
	return c

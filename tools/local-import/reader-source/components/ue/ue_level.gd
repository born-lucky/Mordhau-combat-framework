# ue_level.gd - rebuild a UE level (.umap, read as mdx json) as a Godot scene.
# CUE4Parse's own level exporter writes USD only (WorldExporter.cs GetWorldFormat; docs/WORKFLOW_SOURCES.md section 4),
# and Godot does not import USD, so this reads the level JSON directly (docs/WORKFLOW_SOURCES.md section 4, route 2):
#   map package -> World.StreamingLevels -> LevelStreaming*.WorldAsset (recursively)
#   every placed StaticMeshComponent      -> one instance of the mesh's .glb (res://data/<mesh package>.glb)
#   every (Foliage)InstancedStaticMeshComponent -> one MultiMeshInstance3D, one instance per PerInstanceSMData entry
#   component world transform = parent chain (AttachParent) * RelativeLocation/Rotation/Scale3D
#
# Object references. Every reference in the JSON is {"ObjectPath": "<package>.<N>"}; N is the index into that package's
# export array (checked: DU_Arena.json export 28 is the StaticMeshComponent0 that "DU_Arena.28" names).
# "/Game/..." asset paths are the same packages under "Mordhau/Content/..." (BP_ArenaMapMetadata.json lists
# "/Game/Mordhau/Maps/Arena_Map/DU_Arena.DU_Arena"; manifest.tsv has Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena.umap).
#
# Coordinates. The glb meshes were written by CUE4Parse's glTF writer, which maps every UE vertex v (cm, Z up) to
#   g = SwapYZ(v * 0.01)                       Gltf.cs:25 (UnitScale = 0.01f), 72/230 (positions), 244-248 (SwapYZ)
# and every rotation quaternion q = (x,y,z,w) to (x, z, y, -w)                                      Gltf.cs:250
# So a UE placement world = T + R(s * v) (FTransform.TransformPosition, FTransform.cs:389) must become, in glb space,
#   SwapYZ(0.01 T) + SwapYZ(R(s * v))      and since v = 100 SwapYZ(g):
#   origin = SwapYZ(T) * 0.01,  basis = Basis(SwapYZ(q)) * scale(sx, sz, sy)
# (SwapYZ R SwapYZ is the rotation with quaternion SwapYZ(q), the same rule the writer uses for bone rest poses,
# Gltf.cs:129-131.) FRotator -> quaternion is CUE4Parse's FRotator.Quaternion (FRotator.cs:88-111).
# tests/test_level.gd proves this against a glb vertex placed the UE way with FRotationTranslationMatrix.
#
# Not converted (counted in skips, see read()):
#  hlod     components of LODActor: merged HLOD proxies UE draws only far away instead of the real actors
#           (https://docs.unrealengine.com/4.26/en-US/BuildingWorlds/HLOD/)
#  hidden   actor has bHidden (AActor "Actor is hidden in game")
#  sky      Engine/EngineSky/SM_SkySphere: BP_Sky_Sphere's material is a MaterialInstanceDynamic made by its construction
#           script at run time, so there is no material asset; the generated scene uses a Godot sky instead
#  no_mesh  StaticMesh is null after template merge
#  empty    instanced component with zero instances
#  spline   SplineMeshComponent (mesh deformed along a spline; not handled)
# Non-mesh actors (lights except the sun, volumes, audio, particles, crowd skeletal meshes, BSP brushes) are not built.
class_name UeLevel

const DATA := "res://data/"

# ---------------------------------------------------------------- packages / objects

# "/Game/Mordhau/Maps/X/Y.Y" -> "Mordhau/Content/Mordhau/Maps/X/Y"
static func game_pkg(asset_path: String) -> String:
	var p := asset_path.get_basename() if asset_path.get_file().contains(".") else asset_path
	return "Mordhau/Content/" + p.trim_prefix("/Game/")

static func pkg_of(obj_path: String) -> String:
	return obj_path.get_basename() if obj_path.get_extension().is_valid_int() else obj_path

# {"ObjectPath": "pkg.N"} -> export N of pkg, or {}
static func obj(ref) -> Dictionary:
	if not (ref is Dictionary) or not ref.has("ObjectPath"):
		return {}
	var op: String = ref["ObjectPath"]
	if not op.get_extension().is_valid_int():
		return {}
	var a := UePkg.load_pkg(pkg_of(op))
	var i := op.get_extension().to_int()
	return a[i] if i >= 0 and i < a.size() else {}

# Own properties over the Template's (the archetype a Blueprint component was copied from), recursively.
static func props(e: Dictionary) -> Dictionary:
	var own: Dictionary = e.get("Properties", {})
	if not e.has("Template"):
		return own
	var base := props(obj(e["Template"]))
	var out := base.duplicate()
	for k in own:
		out[k] = own[k]
	return out

# Persistent map + every streaming sub-level, depth first. Each entry: [package, level Transform3D].
static func levels(map_pkg: String, xf := Transform3D.IDENTITY, out: Array = []) -> Array:
	out.append([map_pkg, xf])
	for e in UePkg.load_pkg(map_pkg):
		if not String(e.get("Type", "")).begins_with("LevelStreaming"):
			continue
		var p: Dictionary = e.get("Properties", {})
		var wa: String = p.get("WorldAsset", {}).get("AssetPathName", "")
		if wa == "":
			continue
		var lx := xf * (ftransform(p["LevelTransform"]) if p.has("LevelTransform") else Transform3D.IDENTITY)
		levels(game_pkg(wa), lx, out)
	return out

# ---------------------------------------------------------------- transforms (see header)

static func swap(v: Dictionary) -> Vector3:
	return Vector3(v.get("X", 0.0), v.get("Z", 0.0), v.get("Y", 0.0))

# CUE4Parse FRotator.Quaternion (FRotator.cs:88-111), degrees in, UE quaternion out (as x,y,z,w).
static func rot_quat(r: Dictionary) -> Array:
	var h := PI / 360.0
	var p: float = r.get("Pitch", 0.0) * h
	var y: float = r.get("Yaw", 0.0) * h
	var o: float = r.get("Roll", 0.0) * h
	var sp := sin(p); var cp := cos(p)
	var sy := sin(y); var cy := cos(y)
	var sr := sin(o); var cr := cos(o)
	return [cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]

# UE quaternion (x,y,z,w) -> Godot via the writer's SwapYZ(FQuat) = (x, z, y, -w) (Gltf.cs:250).
static func quat(q: Array) -> Quaternion:
	return Quaternion(q[0], q[2], q[1], -q[3]).normalized()

static func xf(loc: Dictionary, rot: Array, scl: Dictionary) -> Transform3D:
	var s := Vector3(scl.get("X", 1.0), scl.get("Z", 1.0), scl.get("Y", 1.0))
	return Transform3D(Basis(quat(rot)) * Basis.from_scale(s), swap(loc) * 0.01)

# Relative transform of a SceneComponent. Absent properties are the USceneComponent defaults: zero location and
# rotation, unit scale (CUE4Parse writes only serialized, i.e. non-default, tagged properties).
static func rel_xf(p: Dictionary) -> Transform3D:
	return xf(p.get("RelativeLocation", {}), rot_quat(p.get("RelativeRotation", {})), p.get("RelativeScale3D", {"X": 1.0, "Y": 1.0, "Z": 1.0}))

# FTransform JSON {Rotation{X,Y,Z,W}, Translation, Scale3D} (foliage TransformData, LevelTransform).
static func ftransform(t: Dictionary) -> Transform3D:
	var r: Dictionary = t.get("Rotation", {})
	return xf(t.get("Translation", {}), [r.get("X", 0.0), r.get("Y", 0.0), r.get("Z", 0.0), r.get("W", 1.0)], t.get("Scale3D", {"X": 1.0, "Y": 1.0, "Z": 1.0}))

# Component -> world, up the AttachParent chain. Godot Transform3D products compose the same affine maps the per-node
# transforms would. (UE's FTransform product drops shear under non-uniform parent scale; no chain in the maps built so
# far has a scaled parent, see test_level.gd.)
static func world_xf(e: Dictionary) -> Transform3D:
	var p := props(e)
	var local := rel_xf(p)
	var par := obj(p.get("AttachParent"))
	return world_xf(par) * local if not par.is_empty() else local

# ---------------------------------------------------------------- read

static func glb(mesh_obj_path: String) -> String:
	return DATA + pkg_of(mesh_obj_path) + ".glb"

static func is_mesh_comp(e: Dictionary) -> bool:
	var t: String = e.get("Type", "")
	return t.ends_with("StaticMeshComponent") or t == "SplineMeshComponent" or e.has("PerInstanceSMData")

static func _skip(info: Dictionary, why: String, name: String) -> void:
	if not info["skips"].has(why):
		info["skips"][why] = []
	info["skips"][why].append(name)

# map_pkg: "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena". Returns
# {levels: [pkg], meshes: [{name, mesh, xf, mats}], isms: [{name, mesh, xf, inst: [Transform3D], mats}],
#  skips: {why: [names]}, starts: [{name, xf}], sun: Transform3D (or null), seen: count of mesh components}
static func read(map_pkg: String) -> Dictionary:
	var info := {"levels": [], "meshes": [], "isms": [], "hlods": [], "skips": {}, "starts": [], "sun": null, "seen": 0}
	for lv in levels(map_pkg):
		var pkg: String = lv[0]
		var lx: Transform3D = lv[1]
		info["levels"].append(pkg)
		var exps := UePkg.load_pkg(pkg)
		var reg := UeLightmap.registry(pkg) # baked lighting of this level (ue_lightmap.gd), {} if unbuilt
		for e in exps:
			var t: String = e.get("Type", "")
			if t == "MordhauPlayerStart" or t == "PlayerStart" or (t.ends_with("PlayerStart_C") and t.begins_with("BP_")):
				# Blueprint subclasses too (FFA_Arena: 14 BP_MordhauPlayerStart_C). Team: instance value, else the
				# Blueprint's class defaults (UePkg.defaults), else absent.
				var rc := obj(e.get("Properties", {}).get("RootComponent"))
				var st := {"name": e["Name"], "xf": lx * world_xf(rc)}
				var team = e.get("Properties", {}).get("Team", null)
				if team == null and t.ends_with("_C"):
					team = UePkg.defaults(class_pkg(e)).get("Team", null)
				if team != null:
					st["team"] = team
				info["starts"].append(st)
			elif t == "DirectionalLight" and info["sun"] == null:
				info["sun"] = lx * world_xf(obj(e.get("Properties", {}).get("RootComponent")))
			if not is_mesh_comp(e):
				continue
			info["seen"] += 1
			var actor := obj(e.get("Outer"))
			var name := "%s.%s" % [actor.get("Name", "?"), e.get("Name", "?")]
			var p := props(e)
			var mesh: String = (p.get("StaticMesh") if p.get("StaticMesh") is Dictionary else {}).get("ObjectPath", "")
			if actor.get("Type", "") == "LODActor":
				# HLOD proxy: not a placed mesh (still counted as a "hlod" skip) but built separately and switched
				# against its sub-actors at run time (hlod_record / update_hlod).
				_skip(info, "hlod", name)
				var hr := hlod_record(actor, e, p, mesh, lx * world_xf(e), reg)
				if not hr.is_empty():
					info["hlods"].append(hr)
				continue
			if actor.get("Properties", {}).get("bHidden", false):
				_skip(info, "hidden", name); continue
			if t == "SplineMeshComponent":
				_skip(info, "spline", name); continue
			if mesh == "":
				_skip(info, "no_mesh", name); continue
			if pkg_of(mesh) == "Engine/Content/EngineSky/SM_SkySphere":
				_skip(info, "sky", name); continue
			# Ultra_Dynamic_Sky_BP's own components: the sphere is drawn by the UdsSky node (UeUds.node, its MID ported
			# in ue_uds_sky.gdshader); the mesh asset's WorldGridMaterial is never what UE shows. moon_plane's moon MID
			# is not ported (UNCONFIRMED gap: no moon), so it is skipped rather than drawn as a grid plane.
			if pkg_of(mesh).begins_with("Mordhau/Content/UltraDynamicSky/Meshes/"):
				_skip(info, "sky", name); continue
			var mats := []
			for m in p.get("OverrideMaterials", []):
				mats.append(pkg_of(m["ObjectPath"]) if m is Dictionary else "")
			var rec := {"name": name, "actor": String(actor.get("Name", "")), "mesh": mesh, "xf": lx * world_xf(e), "mats": mats, "lm": UeLightmap.for_component(e, reg)}
			# UPrimitiveComponent CastShadow: 4 Arena components serialize false (2 awnings, a cube, a rope); absent =
			# the mesh-component default true [UE source recalled, UNCONFIRMED; 61 others serialize true explicitly]
			if p.get("CastShadow", true) == false:
				rec["noshadow"] = true
			if e.has("PerInstanceSMData") or t.contains("Instanced"):
				var inst := []
				for d in e.get("PerInstanceSMData", []):
					inst.append(ftransform(d.get("TransformData", {})))
				if inst.is_empty():
					_skip(info, "empty", name); continue
				rec["inst"] = inst
				info["isms"].append(rec)
			else:
				info["meshes"].append(rec)
	info["crowd"] = crowd(map_pkg)
	return info

# ---------------------------------------------------------------- HLOD (ALODActor)
# A LODActor's StaticMeshComponent0 draws a merged proxy mesh (HLOD/Arena_0_HLOD, Arena_1_HLOD; exported per mesh by
# `mdx exportlist` into <package>/<mesh name>.glb) with MinDrawDistance = LODDrawDistance; its SubActors are the
# actors it replaces (for LODLevel 2: level-1 LODActors). Switching rule from FLODSceneTree::UpdateVisibilityStates
# (.text 0x22c1d30, 0x22c22ab-0x22c23af): d2 = squared distance from the view origin to the proxy's bounding box
# (closest point; box = Bounds.Origin -+ BoxExtent), the proxy is drawn when d2 > MinDrawDistance^2 x F (and
# d2 <= MaxDrawDistance^2 x F when one is set), and then its children are hidden (HideNodeChildren). F = 1:
# FCachedSystemScalabilityCVars::CalculateFieldOfViewDistanceScale is only applied when the cvar at +0x20
# (r.FieldOfViewAffectsHLOD) is non-zero (0x22c20ac-0x22c20d3); neither Mordhau ini sets it [default 0 recalled].
static func hlod_record(actor: Dictionary, comp: Dictionary, p: Dictionary, mesh: String, xf: Transform3D, reg: Dictionary) -> Dictionary:
	var ap: Dictionary = actor.get("Properties", {})
	if mesh == "":
		return {}
	var mobj := obj(p["StaticMesh"])
	var b: Dictionary = mobj.get("Properties", {}).get("ExtendedBounds", {})
	var subs := []
	for s in ap.get("SubActors", []):
		if s is Dictionary:
			subs.append(String(s.get("ObjectName", "")).get_slice("'", 1).get_slice(".", 1))
	return {"name": String(actor.get("Name", "")), "level": int(ap.get("LODLevel", 1)), "mesh": mesh,
		"glb": DATA + pkg_of(mesh) + "/" + String(mobj.get("Name", "")) + ".glb", "xf": xf,
		"min_draw": float(p.get("MinDrawDistance", ap.get("LODDrawDistance", 0.0))),
		"box_origin": b.get("Origin", {}), "box_extent": b.get("BoxExtent", {}), "subs": subs,
		"lm": UeLightmap.for_component(comp, reg)}

# Proxy box in Godot world space (metres): the mesh's UE bounds through the component transform.
static func hlod_aabb(r: Dictionary) -> AABB:
	var o: Dictionary = r["box_origin"]; var x: Dictionary = r["box_extent"]
	var a := AABB()
	var first := true
	for sx in [-1, 1]:
		for sy in [-1, 1]:
			for sz in [-1, 1]:
				var c := swap({"X": o["X"] + sx * x["X"], "Y": o["Y"] + sy * x["Y"], "Z": o["Z"] + sz * x["Z"]}) * 0.01 # UE cm -> m
				c = (r["xf"] as Transform3D) * c
				if first: a = AABB(c, Vector3.ZERO); first = false
				else: a = a.expand(c)
	return a

# Runtime, per frame: UE's HLOD visibility for camera position cam (Godot metres). Returns proxies drawn.
static func update_hlod(root: Node, cam: Vector3) -> int:
	var hl := root.get_node_or_null("HLOD")
	if hl == null:
		return 0
	if not root.has_meta("_hlod_actors"):
		var by_actor := {}
		for n in root.get_node("Meshes").get_children():
			if n.has_meta("ue_actor"):
				var a: String = n.get_meta("ue_actor")
				if not by_actor.has(a): by_actor[a] = []
				by_actor[a].append(n)
		for n in hl.get_children():
			by_actor[String(n.get_meta("ue_hlod_actor"))] = [n]
		root.set_meta("_hlod_actors", by_actor)
	var by_actor: Dictionary = root.get_meta("_hlod_actors")
	var shown := 0
	# reset: children visible, proxies hidden; then walk from the top level down
	for a in by_actor:
		for n in by_actor[a]:
			(n as Node3D).visible = not n.has_meta("ue_hlod_actor")
	var nodes := hl.get_children()
	nodes.sort_custom(func(x, y): return int(x.get_meta("ue_hlod_level")) > int(y.get_meta("ue_hlod_level")))
	var hidden := {}
	for n in nodes:
		var name := String(n.get_meta("ue_hlod_actor"))
		if hidden.has(name):
			continue
		var box: AABB = n.get_meta("ue_hlod_aabb")
		var q := cam.clamp(box.position, box.end)
		var d2 := (cam - q).length_squared() * 10000.0 # m^2 -> cm^2
		var md: float = n.get_meta("ue_hlod_min")
		if d2 > md * md:
			(n as Node3D).visible = true
			shown += 1
			_hlod_hide(by_actor, n.get_meta("ue_hlod_subs"), hidden)
	return shown

static func _hlod_hide(by_actor: Dictionary, subs: Array, hidden: Dictionary) -> void:
	for s in subs:
		hidden[s] = true
		for n in by_actor.get(s, []):
			(n as Node3D).visible = false
			if n.has_meta("ue_hlod_subs"):
				_hlod_hide(by_actor, n.get_meta("ue_hlod_subs"), hidden)

# Unique mesh + override-material object paths, for `mdx exportlist`. full: also every level's BuiltData package
# (its lightmap / shadowmap / sky-occlusion textures, ue_lightmap.gd) and the HLOD proxy meshes (hlod_record), which
# exportlist writes as whole packages (every UStaticMesh / UTexture export of the package, tools/mdx/Program.cs).
static func assets(info: Dictionary, full := false) -> PackedStringArray:
	var s := {}
	for r in info["meshes"] + info["isms"]:
		s[r["mesh"]] = true
		for m in r["mats"]:
			if m != "":
				s[m] = true
	if full:
		for r in info.get("hlods", []):
			s[pkg_of(r["mesh"])] = true
		for lv in info["levels"]:
			var reg := UeLightmap.registry(lv)
			for e in UePkg.load_pkg(lv):
				if e.get("Type", "") == "Level":
					var op: String = e.get("Properties", {}).get("MapBuildData", {}).get("ObjectPath", "") if e.get("Properties", {}).get("MapBuildData") is Dictionary else ""
					if op != "" and not reg.is_empty():
						s[pkg_of(op)] = true
	return PackedStringArray(s.keys())

# ---------------------------------------------------------------- materials

# glb surface s -> material slot index. CUE4Parse's glTF writer emits one primitive per LOD0 section, in section order,
# named after that section's slot material (Gltf.cs:155-161, MeshDto.GetMaterial = Materials[section.MaterialIndex],
# MeshDto.cs:100-109). The section list is RenderData.LODs[0].Sections in the mesh JSON. Sections without triangles
# are dropped (no primitive). Name matching would be ambiguous: every Arena kit mesh has WorldGridMaterial in all slots.
static func section_slots(mesh_obj_path: String) -> PackedInt32Array:
	var out := PackedInt32Array()
	for e in UePkg.load_pkg(pkg_of(mesh_obj_path)):
		if e.get("Type", "") != "StaticMesh" and e.get("Type", "") != "SkeletalMesh":
			continue
		var lods: Array = e.get("RenderData", {}).get("LODs", [])
		if e.get("Type", "") == "SkeletalMesh":
			lods = e.get("LODModels", []) # skeletal: LODModels[0].Sections, same MaterialIndex field
		for sec in (lods[0].get("Sections", []) if lods.size() > 0 else []):
			if int(sec.get("NumTriangles", 0)) > 0:
				out.append(int(sec.get("MaterialIndex", 0)))
	return out

# res://data/<material package>.json (MaterialExporter output), "" if not exported.
static func mat_json(mat_pkg: String) -> String:
	var p := DATA + mat_pkg + ".json"
	return p if FileAccess.file_exists(p) else ""

static var _mat_cache := {}

static func mat(mat_pkg: String) -> Material:
	if not _mat_cache.has(mat_pkg):
		var j := mat_json(mat_pkg)
		_mat_cache[mat_pkg] = UeMaterial.build(j) if j != "" else null
	return _mat_cache[mat_pkg]

# Apply OverrideMaterials (slot i -> material) to every MeshInstance3D under n. Returns surfaces changed.
static func apply_overrides(n: Node, mesh_obj_path: String, mats: Array) -> int:
	if mats.is_empty():
		return 0
	var slots := section_slots(mesh_obj_path)
	var c := 0
	for mi in n.find_children("*", "MeshInstance3D", true, false) + ([n] if n is MeshInstance3D else []):
		var me: Mesh = mi.mesh
		if me.get_surface_count() != slots.size():
			push_warning("UeLevel: %s has %d surfaces, %d sections" % [mesh_obj_path, me.get_surface_count(), slots.size()])
			continue
		for s in me.get_surface_count():
			var k := slots[s]
			if k < mats.size() and mats[k] != "":
				var m := mat(mats[k])
				if m:
					mi.set_surface_override_material(s, m); c += 1
	return c

# ---------------------------------------------------------------- crowd (BP_CrowdSystemActor_*)
# The spectators are Blueprint actors whose SkeletalMeshComponent comes from the class chain:
#   BP_CrowdSystemActor SkeletalMesh_GEN_VARIABLE (SK_ProxyCitizen, AnimationSingleNode, Yaw -90)
#   -> each subclass's own SkeletalMesh_GEN_VARIABLE (mesh / OverrideMaterials, e.g. _red: INST_ProxyCitizenRed)
#   -> the placed instance's properties (SkeletalMesh override, AttachParent DefaultSceneRoot).
# The animation: the class defaults' IdleAnimations (merged up the Super chain with UePkg.defaults; the _red
# classes inherit their parent's list). BP_CrowdSystem picks among them at run time; we take entry 0.
# Mesh + animation are one glb per (mesh, anim) from `mdx anim`, written under res://data/_crowd/<mesh name>/.
const CROWD := "res://data/_crowd/"

static func class_pkg(e: Dictionary) -> String:
	var c: String = e.get("Class", "")
	var q := c.find("'")
	return c.substr(q + 1, c.rfind("'") - q - 1).get_basename() if q >= 0 else ""

static func gen_var(pkg: String, comp: String) -> Dictionary:
	for e in UePkg.load_pkg(pkg):
		if e.get("Name", "") == comp + "_GEN_VARIABLE":
			return e.get("Properties", {})
	return {}

# Super chain of a Blueprint class package, root first (UePkg.defaults walks the same links).
static func class_chain(pkg: String) -> Array:
	var out := []
	var p := pkg
	while p != "" and p.begins_with("Mordhau/Content"):
		var a := UePkg.load_pkg(p)
		if a.is_empty():
			break
		out.push_front(p)
		p = pkg_of(UePkg.export_of(a, "BlueprintGeneratedClass").get("Super", {}).get("ObjectPath", ""))
	return out

static func crowd(map_pkg: String) -> Array:
	var out := []
	for lv in levels(map_pkg):
		var lx: Transform3D = lv[1]
		for e in UePkg.load_pkg(lv[0]):
			if e.get("Type", "") != "SkeletalMeshComponent":
				continue
			var actor := obj(e.get("Outer"))
			var cls := class_pkg(actor)
			if not cls.get_file().begins_with("BP_CrowdSystemActor"):
				continue
			var pr := {}
			for c in class_chain(cls):
				var g := gen_var(c, String(e.get("Name", "")))
				for k in g:
					pr[k] = g[k]
			for k in e.get("Properties", {}):
				pr[k] = e["Properties"][k]
			var idle: Array = UePkg.defaults(cls).get("IdleAnimations", [])
			var mats := []
			for m in pr.get("OverrideMaterials", []):
				mats.append(pkg_of(m["ObjectPath"]) if m is Dictionary else "")
			var par := obj(pr.get("AttachParent"))
			var x := rel_xf(pr)
			out.append({
				"name": "%s.%s" % [actor.get("Name", "?"), e.get("Name", "")],
				"class": cls,
				"mesh": pkg_of(pr.get("SkeletalMesh", {}).get("ObjectPath", "")),
				"anim": pkg_of(idle[0]["ObjectPath"]) if idle.size() > 0 and idle[0] is Dictionary else "",
				"mats": mats,
				"xf": lx * (world_xf(par) * x if not par.is_empty() else x),
			})
	return out

static func crowd_glb(rec: Dictionary) -> String:
	return CROWD + String(rec["mesh"]).get_file() + "/" + String(rec["anim"]) + ".glb"

# ---------------------------------------------------------------- build

# First Mesh in an imported glb scene (CUE4Parse writes one mesh node per static mesh).
static func first_mesh(ps: PackedScene) -> Mesh:
	var n := ps.instantiate()
	var mis := n.find_children("*", "MeshInstance3D", true, false)
	var m: Mesh = mis[0].mesh if mis.size() > 0 else null
	n.free()
	return m

# Scene tree for `info`. Static meshes are glb instances (meta ue_mesh/ue_mats, so overrides are applied at load by
# apply_all); instanced components are MultiMeshInstance3D. Missing glbs are counted in info["missing"].
static func build(info: Dictionary, root_name := "Level") -> Node3D:
	var root := Node3D.new()
	root.name = root_name
	var meshes := Node3D.new(); meshes.name = "Meshes"
	var isms := Node3D.new(); isms.name = "Instanced"
	root.add_child(meshes); meshes.owner = root
	root.add_child(isms); isms.owner = root
	var missing := []
	var used := {}
	for r in info["meshes"]:
		var g := glb(r["mesh"])
		if not ResourceLoader.exists(g):
			missing.append(g); continue
		var ps: PackedScene = load(g)
		var n: Node3D = ps.instantiate()
		var nm: String = r["name"].validate_node_name()
		n.name = nm if not used.has(nm) else "%s_%d" % [nm, used.size()]
		used[String(n.name)] = true
		n.transform = r["xf"]
		n.set_meta("ue_mesh", r["mesh"])
		n.set_meta("ue_actor", r.get("actor", ""))
		if r.get("noshadow", false):
			n.set_meta("ue_noshadow", true)
		if not r["mats"].is_empty():
			n.set_meta("ue_mats", r["mats"])
		if not r.get("lm", {}).is_empty():
			var lm: Dictionary = r["lm"].duplicate()
			lm["uv"] = UeLightmap.uv_index(r["mesh"])
			n.set_meta("ue_lm", lm)
		meshes.add_child(n); n.owner = root
	for r in info["isms"]:
		var g := glb(r["mesh"])
		if not ResourceLoader.exists(g):
			missing.append(g); continue
		var mm := MultiMesh.new()
		mm.transform_format = MultiMesh.TRANSFORM_3D
		var me := first_mesh(load(g))
		if not r["mats"].is_empty() and me is ArrayMesh:
			# MultiMeshInstance3D has no per-surface override, so overrides go into a copy of the mesh
			me = me.duplicate()
			var slots := section_slots(r["mesh"])
			for s in mini(me.get_surface_count(), slots.size()):
				if slots[s] < r["mats"].size() and r["mats"][slots[s]] != "":
					var m := mat(r["mats"][slots[s]])
					if m:
						me.surface_set_material(s, m)
		mm.mesh = me
		mm.instance_count = r["inst"].size()
		for i in r["inst"].size():
			mm.set_instance_transform(i, r["inst"][i])
		var mmi := MultiMeshInstance3D.new()
		mmi.name = r["name"].validate_node_name()
		mmi.multimesh = mm
		mmi.transform = r["xf"]
		mmi.set_meta("ue_mesh", r["mesh"])
		# MultiMesh keeps instance data in the RenderingServer; under --headless (the generator) that server is a
		# dummy and the saved buffer comes out empty, so the transforms also go into meta and fill() restores them.
		mmi.set_meta("ue_inst", r["inst"])
		isms.add_child(mmi); mmi.owner = root
	var hl := Node3D.new(); hl.name = "HLOD"
	root.add_child(hl); hl.owner = root
	for r in info.get("hlods", []):
		if not ResourceLoader.exists(r["glb"]):
			missing.append(r["glb"]); continue
		var hn: Node3D = (load(r["glb"]) as PackedScene).instantiate()
		hn.name = String(r["name"]).validate_node_name()
		hn.transform = r["xf"]
		hn.visible = false
		hn.set_meta("ue_hlod_actor", r["name"])
		hn.set_meta("ue_hlod_level", r["level"])
		hn.set_meta("ue_hlod_min", r["min_draw"])
		hn.set_meta("ue_hlod_subs", r["subs"])
		hn.set_meta("ue_hlod_aabb", hlod_aabb(r))
		# The proxies are lightmapped (LODData MapBuildDataId) and their meshes have one UV set (RenderData
		# NumTexCoords 1), so the lightmap is on UV0: ue_tint lm_uv 0.
		if not r.get("lm", {}).is_empty():
			var lm: Dictionary = r["lm"].duplicate()
			lm["uv"] = 0
			hn.set_meta("ue_lm", lm)
		hl.add_child(hn); hn.owner = root
	var cr := Node3D.new(); cr.name = "Crowd"
	root.add_child(cr); cr.owner = root
	for r in info.get("crowd", []):
		var g := crowd_glb(r)
		if not ResourceLoader.exists(g):
			missing.append(g); continue
		var n: Node3D = (load(g) as PackedScene).instantiate()
		n.name = String(r["name"]).validate_node_name()
		n.transform = r["xf"]
		n.set_meta("ue_mesh", r["mesh"])
		n.set_meta("ue_anim", r["anim"])
		if not r["mats"].is_empty():
			n.set_meta("ue_mats", r["mats"])
		cr.add_child(n); n.owner = root
	var starts := Node3D.new(); starts.name = "PlayerStarts"
	root.add_child(starts); starts.owner = root
	for s in info["starts"]:
		var mk := Marker3D.new()
		mk.name = String(s["name"]).validate_node_name()
		mk.transform = s["xf"]
		if s.has("team"):
			mk.set_meta("ue_team", s["team"])
		starts.add_child(mk); mk.owner = root
	info["missing"] = missing
	return root

# Runtime: loop each spectator's idle (the glb's only animation, from `mdx anim`). AnimationPlayer:
# https://docs.godotengine.org/en/stable/classes/class_animationplayer.html
static func play_crowd(root: Node) -> int:
	var c := 0
	var cr := root.get_node_or_null("Crowd")
	if cr == null:
		return 0
	for n in cr.get_children():
		var ap := n.find_children("*", "AnimationPlayer", true, false)
		if ap.is_empty():
			continue
		var p: AnimationPlayer = ap[0]
		var names := p.get_animation_list()
		if names.is_empty():
			continue
		p.get_animation(names[0]).loop_mode = Animation.LOOP_LINEAR
		p.play(names[0])
		c += 1
	return c

# Runtime: refill MultiMesh instance transforms from meta ue_inst (see build()). Returns instances set.
static func fill(root: Node) -> int:
	var c := 0
	for n in root.find_children("*", "MultiMeshInstance3D", true, false):
		if not n.has_meta("ue_inst"):
			continue
		var inst: Array = n.get_meta("ue_inst")
		var mm: MultiMesh = n.multimesh
		mm.instance_count = inst.size()
		for i in inst.size():
			mm.set_instance_transform(i, inst[i])
		c += inst.size()
	return c

# Runtime: apply the OverrideMaterials recorded as meta by build() and refill instanced meshes. Returns surfaces changed.
static func apply_all(root: Node) -> int:
	fill(root)
	play_crowd(root)
	# painted vertex colours (OverrideVertexColors, ue_vcolor.gd) before materials/lightmaps: it swaps in mesh copies
	if root.has_meta("ue_map"):
		UeVColor.apply_all(root, root.get_meta("ue_map"))
	var c := 0
	for n in root.find_children("*", "Node3D", true, false):
		if n.has_meta("ue_mats") and n.has_meta("ue_mesh"):
			c += apply_overrides(n, n.get_meta("ue_mesh"), n.get_meta("ue_mats"))
		if n.has_meta("ue_noshadow"):
			# GeometryInstance3D.cast_shadow: https://docs.godotengine.org/en/stable/classes/class_geometryinstance3d.html
			for mi in n.find_children("*", "GeometryInstance3D", true, false):
				(mi as GeometryInstance3D).cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	# materials baked into the imported glbs / embedded in the .tscn may predate the current ue_tint/ue_material:
	# rebuild the stale ones from their ue_json meta (UeMaterial.rebuild_all, materials builder) before the lightmap
	# copies are made, so those copies carry the current shader revision.
	UeMaterial.rebuild_all(root)
	apply_lightmaps(root)
	return c

# Runtime: put each lightmapped mesh's surfaces on UeMaterial.with_lightmap copies (one per material + atlas, cached)
# and set its per-instance lm_* values (ue_lightmap.gd, ue_tint.gdshader UE_LIGHTMAP). Lightmap UVs are read from
# LightMapCoordinateIndex 1 = UV2 (glTF TEXCOORD_1) or 2 = CUSTOM0.xy (TEXCOORD_2), told to the shader via the
# per-instance lm_uv; the SkyOcclusion page goes in as sky_tex (ue_tint: AO *= occlusion).
# Returns surfaces converted.
static var _lm_cache := {}
static var skip_lightmaps := false # debug renders (level_walker --debug=sunonly / nolightmap)

static func apply_lightmaps(root: Node) -> int:
	var c := 0
	if skip_lightmaps:
		return 0
	for n in root.find_children("*", "Node3D", true, false):
		if not n.has_meta("ue_lm"):
			continue
		var lm: Dictionary = n.get_meta("ue_lm")
		if not int(lm.get("uv", -1)) in [0, 1, 2] or not ResourceLoader.exists(lm["tex"]):
			continue
		var tex: Texture2D = load(lm["tex"])
		var sky: Texture2D = load(lm["sky"]) if lm.get("sky", "") != "" and ResourceLoader.exists(lm["sky"]) else null
		for mi in n.find_children("*", "MeshInstance3D", true, false):
			var done := false
			for s in mi.mesh.get_surface_count():
				var m: Material = mi.get_surface_override_material(s)
				if m == null:
					m = mi.mesh.surface_get_material(s)
				if not (m is ShaderMaterial):
					continue
				var k := "%d|%s" % [m.get_instance_id(), lm["tex"]]
				if not _lm_cache.has(k):
					_lm_cache[k] = UeMaterial.with_lightmap(m, tex, sky)
				mi.set_surface_override_material(s, _lm_cache[k])
				done = true
				c += 1
			if done:
				for p in ["lm_coord", "lm_scale0", "lm_add0", "lm_scale1", "lm_add1"]:
					mi.set_instance_shader_parameter(p, lm[p])
				mi.set_instance_shader_parameter("lm_uv", float(lm["uv"]))
	return c

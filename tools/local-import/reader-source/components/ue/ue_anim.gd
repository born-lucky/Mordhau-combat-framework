# ue_anim.gd - UE skeletal meshes and AnimSequences -> Godot scenes and AnimationLibrary.
# Data backend "pak" (UePkgPak.enabled()): built from the game's own paks (UeSkeletalMesh / UeStaticMesh /
# UeAnimSequence), each package decoded once and cached. Backend "json": the glb files `mdx export` / `mdx anim` wrote.
# Both give the same tree (<name> > <name>_ao > Skeleton3D > Mesh, as Godot's glTF importer builds it) and clip tracks
# addressed "<name>_ao/Skeleton3D:<bone>" (test_pak_scene).
# glb path: each anim .glb is the skeletal mesh + one glTF animation. Godot's glTF importer turns it into a scene with a
# Skeleton3D and an AnimationPlayer whose tracks address "<path to Skeleton3D>:<bone>"
# (docs.godotengine.org/en/stable/tutorials/assets_pipeline/importing_3d_scenes/, class_animationplayer).
# Axes/units are already converted in the glb (CUE4Parse Gltf.SwapYZ, cm -> m; see tools/mdx/Program.cs `anim`),
# so nothing is re-converted here. Retargeting is by bone name: the mesh and anims share UMA_Master_Skeleton,
# so names match 1:1; a BoneMap pair (class_bonemap: find_profile_bone_name / get_skeleton_bone_name) renames
# bones when the target skeleton is different.
# Clip timing differs between the two on purpose: the glb keys sit at `mdx anim`'s frame times f / (NumFrames /
# SequenceLength) and reduced key tracks were resampled with CUE4Parse's spacing (NumFrames intervals), while the pak
# clip keeps the engine's own keys at k / (NumKeys - 1) * SequenceLength (AEFPerTrackCompressionCodec::
# GetBoneAtomRotation VA 0x142e6b330; UeAnimSequence.key_time). AnimRig rescales either to UE time.
class_name UeAnim

const CLIP_MESH := "Mordhau/Content/UMA/UMA/Master/UMA_Master"	# the mesh `mdx anim` exported every clip onto

static var _clips := {}		# package -> Animation (or null)
static var _meshes := {}	# package -> {mesh: ArrayMesh, bones: [[name, parent, rest]], skin: Skin} (or {})
static var _clip_ref := {}	# CLIP_MESH's FReferenceSkeleton, read once

# "Mordhau/Content/.../X" -> res:// path of the imported glb (godot/data -> extract/gltf).
static func res(ue_path: String) -> String:
	return "res://data/" + ue_path + ".glb"

static func skeleton_of(n: Node) -> Skeleton3D:
	if n is Skeleton3D:
		return n
	for c in n.get_children():
		var s := skeleton_of(c)
		if s != null:
			return s
	return null

static func player_of(n: Node) -> AnimationPlayer:
	if n is AnimationPlayer:
		return n
	for c in n.get_children():
		var p := player_of(c)
		if p != null:
			return p
	return null

# Is the mesh / clip there to load: in the paks (pak backend) or exported and imported (json backend)
static func has_asset(ue_path: String) -> bool:
	if ue_path == "":
		return false
	if UePkgPak.enabled():
		return UePkgPak.exists(UePkg.strip(ue_path))
	return ResourceLoader.exists(res(ue_path))

# Instance a mesh as a Node3D tree. Pak backend: built from the paks (mesh packages; clips come from clip()); json: the
# imported glb (mesh or anim). null when absent.
static func instance(ue_path: String) -> Node3D:
	if UePkgPak.enabled():
		return _pak_instance(UePkg.strip(ue_path))
	var ps: PackedScene = load(res(ue_path))
	return ps.instantiate() if ps != null else null

# A mesh node by UE package path, from the backend's source: the paks (UeAnim.instance), or the exported glb `glb`
# (json backend; "" or not imported -> null). Lets game code ask for a mesh without knowing the backend.
static func mesh_node(ue_path: String, glb := "") -> Node3D:
	if UePkgPak.enabled():
		return instance(ue_path) if has_asset(ue_path) else null
	if glb == "" or not ResourceLoader.exists(glb):
		return null
	return (load(glb) as PackedScene).instantiate()

# The Animation of a clip (tracks "<CLIP_MESH name>_ao/Skeleton3D:<bone>"), cached; null when absent
static func clip(ue_path: String) -> Animation:
	var p := UePkg.strip(ue_path)
	if _clips.has(p):
		return _clips[p]
	var a: Animation = null
	if UePkgPak.enabled():
		a = _pak_clip(p)
	elif ResourceLoader.exists(res(p)):
		var sc := instance(p)
		if sc != null:
			var ap := player_of(sc)
			if ap != null and not ap.get_animation_list().is_empty():
				a = ap.get_animation(ap.get_animation_list()[0])
			sc.free()
	_clips[p] = a
	return a

static func clear_cache() -> void:
	_clips.clear()
	_meshes.clear()
	_clip_ref.clear()

# Build one AnimationLibrary from clips. Track paths are rewritten to "<skel_path>:<bone>" so the library plays
# on any skeleton reached by skel_path from the player's root_node. Animations are keyed by the UE asset name.
# src_map/dst_map (optional): BoneMaps for the anim skeleton and the target skeleton, sharing one SkeletonProfile.
static func library(ue_paths: Array, skel_path: NodePath, src_map: BoneMap = null, dst_map: BoneMap = null) -> AnimationLibrary:
	var lib := AnimationLibrary.new()
	for p in ue_paths:
		var src := clip(p)
		if src == null:
			push_error("UeAnim: cannot load clip " + String(p))
			continue
		var a: Animation = src.duplicate(true)
		for t in range(a.get_track_count() - 1, -1, -1):
			var tp := a.track_get_path(t)
			var bone := String(tp.get_concatenated_subnames())
			if bone == "":
				a.remove_track(t)
				continue
			if src_map != null and dst_map != null:
				bone = String(dst_map.get_skeleton_bone_name(src_map.find_profile_bone_name(bone)))
				if bone == "":
					a.remove_track(t)
					continue
			a.track_set_path(t, NodePath(String(skel_path) + ":" + bone))
		lib.add_animation(String(p).get_file(), a)
	return lib

# Give a character (instance of the mesh glb) an AnimationPlayer playing lib under library name "ue".
static func attach(character: Node3D, lib: AnimationLibrary) -> AnimationPlayer:
	var ap := AnimationPlayer.new()
	ap.name = "UeAnimPlayer"
	character.add_child(ap)
	ap.root_node = NodePath("..")
	ap.add_animation_library("ue", lib)
	return ap

# skel_path as library() wants it, for a skeleton under character.
static func skel_path(character: Node3D) -> NodePath:
	return character.get_path_to(skeleton_of(character))

# ---- pak backend -----------------------------------------------------------------------------------------------------

static func _pak_mesh(pkg: String) -> Dictionary:
	if _meshes.has(pkg):
		return _meshes[pkg]
	var out := {}
	var mat_names := []
	var mesh: ArrayMesh = null
	var m := UeSkeletalMesh.lod0(pkg)
	if not m.is_empty():
		mesh = UeSkeletalMesh.array_mesh(m)
		for s in m.Sections:
			mat_names.append(m.Materials[s.MaterialIndex].Material if s.MaterialIndex < m.Materials.size() else "")
		var sk := UeSkeletalMesh.skeleton(m)
		var bones := []
		for i in sk.get_bone_count():
			bones.append([sk.get_bone_name(i), sk.get_bone_parent(i), sk.get_bone_rest(i)])
		out = {"bones": bones, "skin": sk.create_skin_from_rest_transforms()}
		sk.free()
	else:
		var sm := UeStaticMesh.lod0(pkg)
		if not sm.is_empty():
			mesh = UeStaticMesh.array_mesh(pkg)
			var slots: Array = sm.Properties.get("StaticMaterials", [])
			for s in sm.Sections:
				var mi = slots[s.MaterialIndex].get("MaterialInterface") if s.MaterialIndex < slots.size() else null
				mat_names.append(String(mi.ObjectName).get_slice("'", 1) if mi is Dictionary else "")
			out = {"bones": []}
	if mesh != null:
		# materials: the same UE material the glTF import hook gives each slot (UeImport._resolve -> UeMaterial)
		var dir := UeImport.DATA + pkg.get_base_dir()
		while not DirAccess.dir_exists_absolute(dir) and dir.length() > UeImport.DATA.length():
			dir = dir.get_base_dir()		# a mesh never exported has no folder under res://data
		var built := {}
		for i in mat_names.size():
			var nm: String = mat_names[i]
			if nm == "":
				continue
			if not built.has(nm):
				var json := UeMaterial.json_for(pkg, nm, dir)
				built[nm] = UeMaterial.build(json) if json != "" else null
			if built[nm] != null:
				mesh.surface_set_material(i, built[nm])
			mesh.surface_set_name(i, nm)
		out.mesh = mesh
	UeAsset.clear_cache()		# the package bytes are not needed once the mesh is built
	_meshes[pkg] = out
	return out

static func _pak_instance(pkg: String) -> Node3D:
	var d := _pak_mesh(pkg)
	if d.is_empty():
		return null
	var root := Node3D.new()
	root.name = pkg.get_file()
	var ao := Node3D.new()
	ao.name = pkg.get_file() + "_ao"
	root.add_child(ao)
	var mi := MeshInstance3D.new()
	mi.name = "Mesh"
	mi.mesh = d.mesh
	if d.bones.is_empty():
		ao.add_child(mi)
		return root
	var sk := Skeleton3D.new()
	sk.name = "Skeleton3D"
	ao.add_child(sk)
	for b in d.bones:
		sk.add_bone(b[0])
	for i in d.bones.size():
		if d.bones[i][1] >= 0:
			sk.set_bone_parent(i, d.bones[i][1])
		sk.set_bone_rest(i, d.bones[i][2])
		sk.set_bone_pose(i, d.bones[i][2])
	sk.add_child(mi)
	mi.skin = d.skin
	mi.skeleton = NodePath("..")
	return root

static func _pak_clip(pkg: String) -> Animation:
	var d := UeAnimSequence.decode(pkg)
	if d.is_empty():
		return null
	var skp := String(d.Properties.get("Skeleton", {}).get("ObjectPath", ""))
	var skel := UePkg.export_of(UePkg.load_pkg(UePkg.strip(skp)), "Skeleton")
	if skel.is_empty():
		return null
	var modes := []
	for bt in skel.get("Properties", {}).get("BoneTree", []):
		modes.append(String(bt.get("TranslationRetargetingMode", "EBoneTranslationRetargetingMode::Animation")))
	if _clip_ref.is_empty():
		var cm := UeSkeletalMesh.lod0(CLIP_MESH)
		if not cm.is_empty():
			_clip_ref = cm.RefSkeleton
	# the mesh's own spelling of each bone ("Head" where the skeleton says "head"; FNames compare case-insensitively)
	# and its reference pose, the target `mdx anim` retargeted "Skeleton"-mode bones to
	var spell := {}
	var tpose := {}
	for i in _clip_ref.get("FinalRefBoneInfo", []).size():
		var nm := String(_clip_ref.FinalRefBoneInfo[i].Name)
		spell[nm.to_lower()] = nm
		tpose[nm.to_lower()] = _clip_ref.FinalRefBonePose[i]
	var tgt := {"FinalRefBonePose": []}
	for b in skel.ReferenceSkeleton.FinalRefBoneInfo:
		tgt.FinalRefBonePose.append(tpose.get(String(b.Name).to_lower(), {"Translation": {"X": 0.0, "Y": 0.0, "Z": 0.0}}))
	var a := UeAnimSequence.to_animation(d, skel.ReferenceSkeleton, "Skeleton3D", modes, tgt)
	var prefix := CLIP_MESH.get_file() + "_ao/Skeleton3D:"
	for t in range(a.get_track_count() - 1, -1, -1):
		var bone := String(a.track_get_path(t).get_concatenated_subnames()).to_lower()
		if not spell.has(bone):
			a.remove_track(t)			# a bone the mesh does not have (`mdx anim` skips those too)
			continue
		a.track_set_path(t, NodePath(prefix + spell[bone]))
	a.resource_name = pkg.get_file()
	UeAsset.clear_cache()
	return a

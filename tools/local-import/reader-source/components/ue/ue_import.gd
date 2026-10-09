# ue_import.gd - glTF import hook: gives every mesh surface of a .glb under res://data/ its real UE material.
# CUE4Parse writes the UE material name into the glTF material slot ("M_Fallen_longsword") but no textures. The
# parameters are in M_Fallen_longsword.json next to the mesh, and this hook swaps the empty material for UeMaterial.build().
#
# Why a GLTFDocumentExtension and not an EditorScenePostImport / EditorScenePostImportPlugin:
#  - EditorScenePostImport is per file. It is set in each .glb's .import file (import_script/path), and those files sit
#    under extract/.
#  - EditorScenePostImportPlugin is project wide, but its _pre_process/_post_process(scene) are not told the source
#    file path (https://docs.godotengine.org/en/stable/classes/class_editorscenepostimportplugin.html), and the path is
#    needed because two different materials are both named M_Fallen_longsword (FallenSkin/ and Teutonic/).
#  - GLTFDocumentExtension is project wide once registered (GLTFDocument.register_gltf_document_extension,
#    https://docs.godotengine.org/en/stable/classes/class_gltfdocument.html), and it gets GLTFState.base_path, "the folder
#    path associated with this glTF data" (https://docs.godotengine.org/en/stable/classes/class_gltfstate.html).
# Which JSON: UeMaterial.json_for() - the slot's real package from the mesh's package JSON, else the nearest folder.
# addons/ue_import/plugin.gd registers it while the editor runs, and the editor import of each .glb then runs it.
#
# Hooks (https://docs.godotengine.org/en/stable/classes/class_gltfdocumentextension.html):
#  _import_preflight: "run first ... returning OK" opts this extension in for the file.
#  _import_post_parse: runs "after parsing ... but before generating the scene". Meshes are already ImporterMesh with
#    materials, so the surfaces are rewritten here (ImporterMesh.set_surface_material,
#    https://docs.godotengine.org/en/stable/classes/class_importermesh.html).
# The extension holds no state, as GLTFDocument requires ("all GLTFDocumentExtension classes must be stateless").
@tool
class_name UeImport
extends GLTFDocumentExtension

const DATA := "res://data/"

# ProjectSettings.localize_path: https://docs.godotengine.org/en/stable/classes/class_projectsettings.html#class-projectsettings-method-localize-path
static func dir_of(state: GLTFState) -> String:
	return ProjectSettings.localize_path(state.base_path)

func _import_preflight(state: GLTFState, _extensions: PackedStringArray) -> Error:
	return OK if dir_of(state).begins_with(DATA) else ERR_SKIP

func _import_post_parse(state: GLTFState) -> Error:
	apply(state)
	return OK

# Returns the number of surfaces that got a UE material. Built materials are shared per name within one file.
# GLTFState.get_materials/set_materials, get_meshes; GLTFMesh.mesh, instance_materials:
#   https://docs.godotengine.org/en/stable/classes/class_gltfstate.html, https://docs.godotengine.org/en/stable/classes/class_gltfmesh.html
static func apply(state: GLTFState) -> int:
	var dir := dir_of(state)
	# Mesh package: res://data/<pkg>.glb -> <pkg>, whose package JSON lists each slot's real material.
	var mesh_pkg := dir.trim_prefix(DATA).path_join(state.filename.get_basename())
	var built := {}
	var n := 0
	var mats := state.get_materials()
	for i in mats.size():
		var r = _resolve(mats[i], dir, mesh_pkg, built)
		if r:
			mats[i] = r
	state.set_materials(mats)
	for gm in state.get_meshes():
		var im: ImporterMesh = gm.mesh
		for s in im.get_surface_count():
			var r = _resolve(im.get_surface_material(s), dir, mesh_pkg, built)
			if r:
				im.set_surface_material(s, r)
				n += 1
		var inst := gm.instance_materials
		for i in inst.size():
			var r = _resolve(inst[i], dir, mesh_pkg, built)
			if r:
				inst[i] = r
		gm.instance_materials = inst
	return n

static func _resolve(m: Material, dir: String, mesh_pkg: String, built: Dictionary) -> Material:
	if m == null or m.resource_name == "":
		return null
	var name := m.resource_name
	if not built.has(name):
		var json := UeMaterial.json_for(mesh_pkg, name, dir)
		built[name] = UeMaterial.build(json) if json != "" else null
		if json == "":
			push_warning("UeImport: no %s.json near %s" % [name, dir])
	return built[name]

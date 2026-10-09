# ue_pkg_pak.gd - the "pak" data backend of UePkg: packages, config files and raw files read from the game's own .pak
# files (UePakVfs + UeAsset) instead of the extract/ copies `mdx` writes. See docs/PAK_FORMAT.md "Equivalence".
class_name UePkgPak

# "pak" since layout r2 (2026-10-04): equal to extract/json over every package the readers load, the maps the modes
# load and all of Blueprints/ + UI/ (test_pak_equiv), and the readers' tests pass on it
const DEFAULT := "pak"

static var forced := ""		# tests compare both backends in one run: "pak"/"json" wins over everything below

# "pak" or "json": `forced`, else $MORDHAU_DATA_BACKEND (run the suite on either backend without editing the project),
# else the project setting mordhau/data_backend, else DEFAULT
static func backend() -> String:
	if forced != "":
		return forced
	var e := OS.get_environment("MORDHAU_DATA_BACKEND")
	if e == "pak" or e == "json":
		return e
	return ProjectSettings.get_setting("mordhau/data_backend", DEFAULT)

static func enabled() -> bool:
	return backend() == "pak"

# Exports of "Mordhau/Content/.../BP_X" in mdx json's shape; [] when the package is not in the paks
static func read(pkg_path: String) -> Array:
	var a := UeAsset.open(pkg_path)
	if a == null:
		return []
	return a.exports_json()

static func exists(pkg_path: String) -> bool:
	return UePakVfs.has(pkg_path + ".uasset") or UePakVfs.has(pkg_path + ".umap")

static var _cls := {}		# package -> class of its first export (UeAsset.first_export_class), filled on demand

# Packages (no extension) under `dir` whose first export's class is one of `classes`, sorted: the rows of
# extract/manifest.tsv (mdx list: path, size, class of the first export) that the same filter selects (test_pak).
static func packages_of_class(dir: String, classes: Array) -> PackedStringArray:
	var out := PackedStringArray()
	var pre := dir.to_lower() + "/"
	for pth in UePakVfs.list():
		if not pth.ends_with(".uasset") or not pth.to_lower().begins_with(pre):
			continue
		var p := pth.get_basename()
		if not _cls.has(p):
			_cls[p] = UeAsset.first_export_class(p)
		if classes.has(_cls[p]):
			out.append(p)
	out.sort()
	return out

# SuperStruct of the package's first BlueprintGeneratedClass export (UePkg.super_of), decoding only that export
static func super_of(pkg_path: String) -> Dictionary:
	var a := UeAsset.open(pkg_path)
	if a == null:
		return {}
	for i in a.exports.size():
		var cn = a.node(a.exports[i].cls)
		if cn != null and UeAsset.node_name(cn) == "BlueprintGeneratedClass":
			return a.export_json(i).get("SuperStruct", {})
	return {}

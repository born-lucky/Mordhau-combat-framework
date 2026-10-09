# ue_pkg.gd - read the game's packages (from its .pak files, or the extract/json copy) and resolve Blueprint class defaults.
# UE stores only overridden values on a BP's CDO (Default__X_C); the rest come from the parent class.
# CUE4Parse reads exactly the property tags present, until the "None" tag
# (tools/CUE4Parse-src/CUE4Parse/UE4/Assets/Exports/UObject.cs:416-434; structs: Objects/FStructFallback.cs:30),
# so a key missing from every CDO in the chain holds the native class's value, which no JSON contains.
# defaults() walks the SuperStruct chain from the root down and merges, child wins.
class_name UePkg

static var _cache := {}

static func root() -> String:
	return ProjectSettings.get_setting("mordhau/data_path", "")

# "Mordhau/Content/.../BP_X.0" -> "Mordhau/Content/.../BP_X" (mdx json appends the export index)
static func strip(obj_path: String) -> String:
	return obj_path.get_basename() if obj_path.get_extension().is_valid_int() else obj_path

# Exports of a package, in the shape `mdx json` (CUE4Parse) writes. The data backend (UePkgPak.backend(): setting
# mordhau/data_backend, $MORDHAU_DATA_BACKEND overrides) picks the source: "pak" reads the game's own .pak files in
# place (components/ue/pak, docs/PAK_FORMAT.md), "json" reads extract/json. Both give the same values (test_pak_equiv).
static func read(obj_path: String) -> Array:
	if UePkgPak.enabled():
		return UePkgPak.read(strip(obj_path))
	var f := FileAccess.open(root() + "/json/" + strip(obj_path) + ".json", FileAccess.READ)
	if f == null:
		return []
	var a = JSON.parse_string(f.get_as_text())
	return a if a is Array else []

static func exists(obj_path: String) -> bool:
	if UePkgPak.enabled():
		return UePkgPak.exists(strip(obj_path))
	return FileAccess.file_exists(root() + "/json/" + strip(obj_path) + ".json")

# Bytes of a shipped non-package file by its mounted path ("Mordhau/Config/DefaultInput.ini",
# "Engine/Content/.../Roboto.ufont"): from the paks, or from the copies `mdx raw` wrote (extract/config/<file> for
# config files, extract/raw/<path> otherwise). Empty when missing.
static func file(pth: String) -> PackedByteArray:
	if UePkgPak.enabled():
		return UePakVfs.read(pth)
	var c := root() + "/config/" + pth.get_file()
	if pth.get_base_dir().ends_with("Config") and FileAccess.file_exists(c):
		return FileAccess.get_file_as_bytes(c)
	var r := root() + "/raw/" + pth
	return FileAccess.get_file_as_bytes(r) if FileAccess.file_exists(r) else PackedByteArray()

# Packages under `dir` whose first export's class is in `classes` (e.g. every Blueprint class under Blueprints/
# Wearables), sorted. pak: the class read from each .uasset header; json: extract/manifest.tsv (mdx list).
static func packages_of_class(dir: String, classes: Array) -> PackedStringArray:
	if UePkgPak.enabled():
		return UePkgPak.packages_of_class(dir, classes)
	var out := PackedStringArray()
	var f := FileAccess.open(root() + "/manifest.tsv", FileAccess.READ)
	if f == null:
		push_error("UePkg: no extract/manifest.tsv")
		return out
	while not f.eof_reached():
		var row := f.get_line().split("\t")
		if row.size() >= 3 and row[0].begins_with(dir + "/") and row[0].ends_with(".uasset") and classes.has(row[2]):
			out.append(row[0].get_basename())
	out.sort()
	return out

# parsed export array, cached
static func load_pkg(obj_path: String) -> Array:
	var p := strip(obj_path)
	if _cache.has(p):
		return _cache[p]
	var a := read(p)
	if a.is_empty():
		push_error("UePkg: missing " + p)
		return []
	_cache[p] = a
	return a

static func clear_cache() -> void:
	_cache.clear()

static func export_of(pkg: Array, type: String) -> Dictionary:
	for e in pkg:
		if e.get("Type", "") == type:
			return e
	return {}

static func export_named(pkg: Array, name: String) -> Dictionary:
	for e in pkg:
		if e.get("Name", "") == name:
			return e
	return {}

static func cdo(pkg: Array) -> Dictionary:
	for e in pkg:
		if String(e.get("Name", "")).begins_with("Default__"):
			return e.get("Properties", {})
	return {}

# The class export's SuperStruct ({ObjectName, ObjectPath}). Every BlueprintGeneratedClass export has it; for a
# Blueprint parent it equals "Super", for a native parent it is "Class'X'" @ "/Script/<Module>" (mdx json).
static func super_of(pkg: Array) -> Dictionary:
	return export_of(pkg, "BlueprintGeneratedClass").get("SuperStruct", {})

# super_of(read(obj_path)) without decoding the package's other exports (pak backend) and without UePkg's cache: for
# scans over thousands of packages (UeWeapon.list_weapons)
static func super_path(obj_path: String) -> Dictionary:
	if UePkgPak.enabled():
		return UePkgPak.super_of(strip(obj_path))
	return super_of(read(obj_path))

# Blueprint packages from obj_path up to (not including) the native parent, child first.
static func chain(obj_path: String) -> PackedStringArray:
	var out := PackedStringArray()
	var p := strip(obj_path)
	while p.begins_with("Mordhau/Content"):
		var pkg := load_pkg(p)
		if pkg.is_empty():
			break
		out.append(p)
		p = strip(super_of(pkg).get("ObjectPath", ""))
	return out

# Native class the chain ends at, e.g. "MordhauWeapon" for "Class'MordhauWeapon'" @ /Script/Mordhau. "" if broken.
static func native_root(obj_path: String) -> String:
	var ch := chain(obj_path)
	if ch.is_empty():
		return ""
	var s: Dictionary = super_of(load_pkg(ch[ch.size() - 1]))
	if String(s.get("ObjectPath", "")).begins_with("Mordhau/Content"):
		return ""			# chain broke on a missing package, not at native code
	return String(s.get("ObjectName", "")).get_slice("'", 1)

# Merged class defaults: root ancestor first, so children override. Stops at the native class (no package JSON).
static func defaults(obj_path: String) -> Dictionary:
	var ch := chain(obj_path)
	var out := {}
	for i in range(ch.size() - 1, -1, -1):
		var d := cdo(load_pkg(ch[i]))
		for k in d:
			out[k] = _merge(out.get(k), d[k])
	return out

# Struct values merge per field (a child that sets only Windup keeps the parent's Release).
# Arrays replace whole: every Damage/HeadBonus/LegBonus array in the 279 weapon CDOs has all 4 entries (full scan).
static func _merge(a, b):
	if a is Dictionary and b is Dictionary:
		var r: Dictionary = a.duplicate()
		for k in b:
			r[k] = _merge(r.get(k), b[k])
		return r
	return b

# UE value -> the type of obj's property. Object refs become their ObjectPath, FText its LocalizedString,
# FName structs ({"Name"}) their Name, enums stay "EType::Value" strings. A value of the wrong shape is an error
# (push_error) and leaves the property's current value (the cited class default), never a silent 0.
static func conv(cur, v, what := ""):
	var num: bool = v is float or v is int
	match typeof(cur):
		TYPE_FLOAT:
			if num: return float(v)
		TYPE_INT:
			if num: return int(v)
		TYPE_BOOL:
			if v is bool: return v
			if v is int and (v == 0 or v == 1): return v == 1		# NativeCtor byte of a bitfield bool
		TYPE_VECTOR2:
			if v is Dictionary and v.has("X") and v.has("Y"): return Vector2(v.X, v.Y)
		TYPE_VECTOR3:
			if v is Dictionary and v.has("X") and v.has("Y") and v.has("Z"): return Vector3(v.X, v.Y, v.Z)
		TYPE_PACKED_FLOAT32_ARRAY:
			if v is Array: return PackedFloat32Array(v)
		TYPE_PACKED_STRING_ARRAY:
			if v is Array:
				var s := PackedStringArray()
				for x in v:
					s.append(String(x))
				return s
		TYPE_ARRAY:
			if v is Array: return v
		TYPE_STRING:
			return text(v)
		_:
			return v
	push_error("UePkg.conv %s: %s does not fit %s" % [what, type_string(typeof(v)), type_string(typeof(cur))])
	return cur

static func text(v) -> String:
	if v == null:
		return ""
	if v is Dictionary:
		if v.has("ObjectPath"): return String(v.ObjectPath)
		if v.has("LocalizedString"): return String(v.LocalizedString)
		if v.has("SourceString"): return String(v.SourceString)
		if v.has("Name"): return String(v.Name)
	return String(v) if v is String else str(v)

# Copy d[ue_key] into obj[map[ue_key]] with type conversion. Returns the keys of d that map does not cover.
static func assign(obj: Object, map: Dictionary, d: Dictionary) -> PackedStringArray:
	var extra := PackedStringArray()
	for k in d:
		if map.has(k):
			obj.set(map[k], conv(obj.get(map[k]), d[k], k))
		else:
			extra.append(k)
	return extra

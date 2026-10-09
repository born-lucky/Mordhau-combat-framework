# ue_pak_vfs.gd - the mounted file system of every .pak in the user's Mordhau install, read in place (read-only; nothing
# is ever written under the game folder). Paths are the mounted paths without the "../../../" root, as in
# extract/manifest.tsv: "Mordhau/Content/Mordhau/Blueprints/.../BP_X.uasset". Lookups ignore case like UE's
# FPakPlatformFile (Windows); the stored spelling is kept for package names.
class_name UePakVfs

const ROOT_PREFIX := "../../../"		# mount points are relative to Engine/Binaries/Win64 (FPakFile mount point)

static var _paks: Array[UePak] = []
static var _handles: Array[FileAccess] = []
static var _files := {}				# lower-case path -> Vector2i(pak slot, entry)
static var _spelling := {}			# lower-case path -> path as stored in the pak
static var _mounted := false
static var errors := PackedStringArray()

# The game folder: project setting mordhau/game_dir, else $MORDHAU_DIR, else Steam's default folder (the same default
# as build.py:10 GAME).
static func game_dir() -> String:
	var d: String = ProjectSettings.get_setting("mordhau/game_dir", "")
	if d == "":
		d = OS.get_environment("MORDHAU_DIR")
	if d == "":
		d = "C:/Program Files (x86)/Steam/steamapps/common/Mordhau"
	return d.replace("\\", "/")

static func paks_dir() -> String:
	# the install folder <game>\Mordhau\Content\Paks (an install path, not a package: the paks hold the packages)
	return game_dir().path_join("Mordhau").path_join("Content").path_join("Paks")

# Mount every *.pak once (sorted by name). Returns false when the folder has no readable pak.
static func mount() -> bool:
	if _mounted:
		return not _paks.is_empty()
	_mounted = true
	var dir := paks_dir()
	var names := Array(DirAccess.get_files_at(dir))
	names.sort()
	for n in names:
		if not String(n).ends_with(".pak"):
			continue
		var pk := UePak.new()
		var raw := {}
		if not pk.open(dir + "/" + n, raw, _paks.size()):
			errors.append("%s: %s" % [n, pk.error])
			continue
		var slot := _paks.size()
		_paks.append(pk)
		_handles.append(FileAccess.open(dir + "/" + n, FileAccess.READ))
		for k in raw:
			var pth := String(k).trim_prefix(ROOT_PREFIX)
			var low := pth.to_lower()
			if _files.has(low):
				# UE resolves duplicates by pak read order (_P patch paks win); Mordhau 702625635 has none (test_pak)
				errors.append("duplicate " + pth)
			_files[low] = Vector2i(slot, raw[k].y)
			_spelling[low] = pth
	return not _paks.is_empty()

static func unmount() -> void:
	_paks.clear(); _handles.clear(); _files.clear(); _spelling.clear()
	_mounted = false
	errors.clear()

static func has(pth: String) -> bool:
	mount()
	return _files.has(pth.to_lower())

static func spelling(pth: String) -> String:
	return _spelling.get(pth.to_lower(), pth)

static func file_count() -> int:
	mount()
	return _files.size()

static func paks() -> Array[UePak]:
	mount()
	return _paks

# Every mounted path (stored spelling)
static func list() -> PackedStringArray:
	mount()
	return PackedStringArray(_spelling.values())

# Every mounted entry decoded: {"method <name>": count, "encrypted": count}
static func census() -> Dictionary:
	mount()
	var out := {"encrypted": 0}
	for low in _files:
		var v: Vector2i = _files[low]
		var e := _paks[v.x].entry(v.y)
		var k: String = "method " + (_paks[v.x].compression[e.method] if e.method < _paks[v.x].compression.size() else str(e.method))
		out[k] = out.get(k, 0) + 1
		out.encrypted += int(e.encrypted)
	return out

# Entry info (UePak.entry) of a path, {} when missing
static func entry(pth: String) -> Dictionary:
	mount()
	var v: Vector2i = _files.get(pth.to_lower(), Vector2i(-1, 0))
	if v.x < 0:
		return {}
	var e := _paks[v.x].entry(v.y)
	e.pak = _paks[v.x].path.get_file()
	return e

# Bytes of a file, empty when missing
static func read(pth: String, verify := false) -> PackedByteArray:
	mount()
	var v: Vector2i = _files.get(pth.to_lower(), Vector2i(-1, 0))
	if v.x < 0:
		return PackedByteArray()
	return _paks[v.x].read(_handles[v.x], v.y, verify)

# `size` bytes at `start` of a file (UePak.read_range), empty when missing
static func read_range(pth: String, start: int, size: int) -> PackedByteArray:
	mount()
	var v: Vector2i = _files.get(pth.to_lower(), Vector2i(-1, 0))
	if v.x < 0:
		return PackedByteArray()
	return _paks[v.x].read_range(_handles[v.x], v.y, start, size)

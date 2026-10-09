# mode_kismet.gd - the Blueprint bytecode literals the game-mode / HUD ports use (timers, stage numbers, texts, colours,
# bot-count slider range), read from data_gen/mode/mode_kismet.json. That file is written by scripts/mode_kismet.py
# from the cooked packages (function bodies decoded with scripts/kismet; `mdx json` has no bodies). Every entry is
# {value, src}, src = "<package>:<function>@<in-memory statement index>". A key that is missing is an error, never a
# default (the extractor fails when a pattern does not match).
class_name ModeKismet

const PATH := "res://data_gen/mode/mode_kismet.json"

static var _k := {}
static var _loaded := false

static func _load() -> void:
	if _loaded:
		return
	_loaded = true
	var f := FileAccess.open(PATH, FileAccess.READ)
	if f == null:
		push_error("ModeKismet: %s missing (run python scripts/mode_kismet.py)" % PATH)
		return
	var d = JSON.parse_string(f.get_as_text())
	_k = d if d is Dictionary else {}

static func ok() -> bool:
	_load()
	return not _k.is_empty()

static func has(key: String) -> bool:
	_load()
	return _k.has(key)

static func kv(key: String):
	_load()
	if not _k.has(key):
		push_error("ModeKismet: no bytecode constant %s in %s" % [key, PATH])
		return null
	return _k[key].value

static func src(key: String) -> String:
	_load()
	return String(_k.get(key, {}).get("src", "?"))

# an [R, G, B, A] linear colour constant
static func color(key: String) -> Color:
	var v = kv(key)
	return Color(float(v[0]), float(v[1]), float(v[2]), float(v[3])) if v is Array and v.size() == 4 else Color.MAGENTA

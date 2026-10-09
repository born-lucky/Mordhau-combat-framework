# ue_config.gd - the game's shipped config files (DefaultInput.ini, DefaultEngine.ini, BaseGame.ini ...), read through
# UePkg.file from the paks (Mordhau/Config/Default*.ini, Engine/Config/Base*.ini) or from the extract/config copy, so
# gameplay code asks for "DefaultInput.ini" instead of building a path.
class_name UeConfig

# Mounted path of a config file: the project's Default*.ini under Mordhau/Config, the engine's Base*.ini under
# Engine/Config (UE's config hierarchy, FConfigCacheIni; the paths as listed in extract/manifest.tsv)
static func path(file: String) -> String:
	return ("Engine/Config/" if file.begins_with("Base") else "Mordhau/Config/") + file

static var _text := {}

# Text of a config file; "" (and an error) when it is missing
static func text(file: String) -> String:
	if _text.has(file):
		return _text[file]
	var b := UePkg.file(path(file))
	if b.is_empty():
		push_error("UeConfig: no " + path(file))
	_text[file] = b.get_string_from_utf8()
	return _text[file]

static func lines(file: String) -> PackedStringArray:
	return text(file).split("\n")

# value of `key` in `[section]` of an ini ("" when the file, section or key is missing). Plain key=value lines only
# (the first match wins); array entries (+Key=) are read by the callers that need them.
static func value(file: String, section: String, key: String) -> String:
	var in_sec := false
	for raw in lines(file):
		var l := raw.strip_edges()
		if l.begins_with("["):
			in_sec = l == "[" + section + "]"
		elif in_sec and l.begins_with(key + "="):
			return l.substr(key.length() + 1)
	return ""

# A float that must be in the ini (an error when the file, section or key is missing or not a number)
static func float_value(file: String, section: String, key: String) -> float:
	var v := value(file, section, key)
	if not v.is_valid_float():
		push_error("UeConfig: no number %s in [%s] of %s" % [key, section, path(file)])
		return NAN
	return float(v)

# ---- DefaultInput.ini (UInputSettings) ------------------------------------------------------------------------------
# +ActionMappings=(ActionName="Jump",bShift=False,...,Key=SpaceBar)
class ActionMapping:
	var action := ""
	var key := ""				# EKeys name ("None" = unbound)

# +AxisMappings=(AxisName="Move Forward",Scale=1.000000,Key=W)
class AxisMapping:
	var axis := ""
	var key := ""
	var scale := 0.0

class InputConfig:
	var actions: Array[ActionMapping] = []
	var axes: Array[AxisMapping] = []
	var sensitivity := {}		# +AxisConfig AxisKeyName -> AxisProperties Sensitivity

	# AxisConfig Sensitivity of an axis key; an error when the ini has no AxisConfig for it
	func sensitivity_of(axis_key: String) -> float:
		if not sensitivity.has(axis_key):
			push_error("UeConfig: DefaultInput.ini has no AxisConfig for %s" % axis_key)
			return NAN
		return sensitivity[axis_key]

static var _input: InputConfig

# one field of a mapping line: Name="quoted" or Name=bare up to , or )
static func field(line: String, key: String) -> String:
	var i := line.find(key + "=")
	if i < 0: return ""
	var s := line.substr(i + key.length() + 1)
	if s.begins_with("\""):
		return s.substr(1, s.find("\"", 1) - 1)
	var j := 0
	while j < s.length() and not s[j] in [",", ")"]: j += 1
	return s.substr(0, j)

static func input() -> InputConfig:
	if _input != null:
		return _input
	var out := InputConfig.new()
	for raw in lines("DefaultInput.ini"):
		var l := raw.strip_edges()
		if l.begins_with("+ActionMappings="):
			var a := ActionMapping.new()
			a.action = field(l, "ActionName")
			a.key = field(l, "Key")
			out.actions.append(a)
		elif l.begins_with("+AxisMappings="):
			var x := AxisMapping.new()
			x.axis = field(l, "AxisName")
			x.key = field(l, "Key")
			x.scale = float(field(l, "Scale"))
			out.axes.append(x)
		elif l.begins_with("+AxisConfig="):
			out.sensitivity[field(l, "AxisKeyName")] = float(field(l, "Sensitivity"))
	_input = out
	return out

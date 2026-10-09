# ue_rec.gd - strict field reader behind every typed record in components/ue (MotionDefs, CharacterData.Character,
# BotData.Profile, AnimData.Montage, ...). A record's from_ue() reads each UE property by name exactly once, here,
# and game/** only sees the typed fields (tests/test_boundary.gd test_game_reads_no_ue_keys).
#
# Two kinds of read, and the difference matters:
#   required  f / i / b / v2 / v3 / rot / s / sub / arr / tp_*: the key must be in the source. Used where the source
#             already carries every field: CombatData.class_defaults = NativeCtor.defaults (a value for every field of
#             a decodable type in the PDB layout, zero-filled first: UObject memory is memset to 0 by
#             StaticAllocateObject before the ctor runs, see weapon_data.gd) merged under the Blueprint CDO chain. A
#             missing key there is a wrong name or a class that does not have the field - an error, never 0.
#   optional  *_or(key, default): only for sources that hold just the serialized delta (a CDO alone, a component
#             template, a struct inside a package), where absence legitimately means "the class default". The caller
#             passes that default and cites where it comes from (native ctor / UPROPERTY default / engine ctor).
#   obj       an object reference: absent, null or {} is a null pointer (UObject* members are never written by the
#             NativeCtor replay, so the zero-filled value nullptr is the native default).
# A present value of the wrong type or shape is always an error. Errors are collected; done() push_errors them all
# and returns null, so a malformed record fails loudly where it is read instead of turning into zeros downstream.
class_name UeRec
extends RefCounted

var d: Dictionary = {}
var what := ""
var errors: Array = []		# shared with sub-readers (Array is by reference)

func _init(src = {}, label := "", shared_errors = null) -> void:
	what = label
	errors = shared_errors if shared_errors is Array else []
	if src is Dictionary:
		d = src
	else:
		errors.append("%s: expected a struct, got %s" % [label, type_string(typeof(src))])

func has(k: String) -> bool:
	return d.has(k)

func _bad(k: String, want: String, v) -> void:
	errors.append("%s.%s: expected %s, got %s" % [what, k, want, type_string(typeof(v)) if v != null else "null"])

func _need(k: String):
	if not d.has(k):
		errors.append("%s.%s: missing" % [what, k])
		return null
	if d[k] == null:
		errors.append("%s.%s: null" % [what, k])
	return d[k]

static func _num(v) -> bool:
	return v is float or v is int

# ---- required -----------------------------------------------------------------------------------------------------
func f(k: String) -> float:
	var v = _need(k)
	if v == null: return NAN
	if not _num(v):
		_bad(k, "number", v)
		return NAN
	return float(v)

func i(k: String) -> int:
	var v = _need(k)
	if v == null: return 0
	if not _num(v) or float(v) != floorf(float(v)):
		_bad(k, "integer", v)
		return 0
	return int(v)

func b(k: String) -> bool:
	var v = _need(k)
	if v == null: return false
	if v is int and (v == 0 or v == 1):		# NativeCtor decodes a uint8 bitfield bool as its byte
		return v == 1
	if not (v is bool):
		_bad(k, "bool", v)
		return false
	return v

func s(k: String) -> String:
	var v = _need(k)
	if v == null: return ""
	if not (v is String):
		_bad(k, "string", v)
		return ""
	return v

# FVector2D {X, Y}
func v2(k: String) -> Vector2:
	var r := sub(k)
	return Vector2(r.f("X"), r.f("Y"))

# FVector {X, Y, Z}, UE axes and units as stored
func v3(k: String) -> Vector3:
	var r := sub(k)
	return Vector3(r.f("X"), r.f("Y"), r.f("Z"))

# FRotator {Pitch, Yaw, Roll} -> Vector3(pitch, yaw, roll), degrees
func rot(k: String) -> Vector3:
	var r := sub(k)
	return Vector3(r.f("Pitch"), r.f("Yaw"), r.f("Roll"))

func sub(k: String) -> UeRec:
	var v = _need(k)
	if v != null and not (v is Dictionary): _bad(k, "struct", v)
	return UeRec.new(v if v is Dictionary else {}, what + "." + k, errors)

func arr(k: String) -> Array:
	var v = _need(k)
	if v == null: return []
	if not (v is Array):
		_bad(k, "array", v)
		return []
	return v

# FPerspective* {ThirdPerson, FirstPerson}: the ThirdPerson half (the duel shows third-person bodies; UMordhauMotion::
# GetIsFirstPerson is not ported)
func tp_f(k: String) -> float:
	return sub(k).f("ThirdPerson")

func tp_b(k: String) -> bool:
	return sub(k).b("ThirdPerson")

func tp_v2(k: String) -> Vector2:
	return sub(k).v2("ThirdPerson")

func tp_obj(k: String) -> String:
	return sub(k).obj("ThirdPerson")

# FPerspective* object: ThirdPerson, or FirstPerson when ThirdPerson is null (UParryMotion::OnBegin_Implementation /
# UBlockedMotion::OnBegin_Implementation pick the other half the same way)
func tp_or_fp_obj(k: String) -> String:
	var r := sub(k)
	var t := r.obj("ThirdPerson")
	return t if t != "" else r.obj("FirstPerson")

# ---- optional (absent = the caller's cited class default) --------------------------------------------------------
func f_or(k: String, def: float) -> float:
	return f(k) if d.has(k) else def

func i_or(k: String, def: int) -> int:
	return i(k) if d.has(k) else def

func b_or(k: String, def: bool) -> bool:
	return b(k) if d.has(k) else def

func s_or(k: String, def: String) -> String:
	return s(k) if d.has(k) else def

func v2_or(k: String, def: Vector2) -> Vector2:
	if not d.has(k): return def
	var r := sub(k)
	return Vector2(r.f_or("X", def.x), r.f_or("Y", def.y))

func v3_or(k: String, def: Vector3) -> Vector3:
	if not d.has(k): return def
	var r := sub(k)
	return Vector3(r.f_or("X", def.x), r.f_or("Y", def.y), r.f_or("Z", def.z))

func rot_or(k: String, def: Vector3) -> Vector3:
	if not d.has(k): return def
	var r := sub(k)
	return Vector3(r.f_or("Pitch", def.x), r.f_or("Yaw", def.y), r.f_or("Roll", def.z))

# a struct whose own fields are then read with *_or: absent = every field at its default
func sub_or(k: String) -> UeRec:
	return sub(k) if d.has(k) else UeRec.new({}, what + "." + k, errors)

func arr_or(k: String) -> Array:
	return arr(k) if d.has(k) else []

# UE enum as mdx json writes it ("EType::Value") -> "Value"; absent = `def` (the enum's 0 value, cited by the caller)
func enum_or(k: String, def: String) -> String:
	if not d.has(k): return def
	var v = d[k]
	if v is String: return String(v).get_slice("::", String(v).count("::"))
	_bad(k, "enum string", v)
	return def

# ---- references ---------------------------------------------------------------------------------------------------
# object reference {ObjectName, ObjectPath} -> stripped package path; absent / null / {} -> "" (nullptr)
func obj(k: String) -> String:
	if not d.has(k): return ""
	return ref_path(d[k], k)

# a value that is an object reference (array element, map value)
func ref_path(v, k := "?") -> String:
	if v == null or (v is Dictionary and v.is_empty()): return ""
	if v is Dictionary and v.has("ObjectPath") and v.ObjectPath is String:
		return UePkg.strip(String(v.ObjectPath))
	_bad(k, "object reference", v)
	return ""

# the export index of an in-package reference ("Pkg.12" -> 12), -1 for none
func ref_index(k: String) -> int:
	if not d.has(k) or d[k] == null: return -1
	var v = d[k]
	if v is Dictionary and v.has("ObjectPath"):
		var e := String(v.ObjectPath).get_extension()
		if e.is_valid_int(): return e.to_int()
	_bad(k, "in-package object reference", v)
	return -1

# FText (mdx json: {CultureInvariantString} or {SourceString / LocalizedString}); absent = "" (empty FText)
func text_or(k: String) -> String:
	if not d.has(k) or d[k] == null: return ""
	var v = d[k]
	if v is Dictionary:
		for f in ["LocalizedString", "SourceString", "CultureInvariantString"]:
			if v.has(f) and v[f] is String:
				return v[f]
		if v.has("LocalizedString") or v.has("SourceString") or v.has("CultureInvariantString"):
			return ""		# an FText whose strings are null: empty text
	_bad(k, "FText", v)
	return ""

# ---- finish -------------------------------------------------------------------------------------------------------
func ok() -> bool:
	return errors.is_empty()

func error_text() -> String:
	return "; ".join(PackedStringArray(errors))

# the record, or null after push_error when anything was missing or malformed
func done(rec):
	if errors.is_empty():
		return rec
	push_error("UeRec %s: %s" % [what, error_text()])
	return null

# native_ctor.gd - default values of Mordhau's native classes/structs, read from the game's own constructors.
#
# A Blueprint CDO (extract/json) stores only the values it changed relative to its native parent, so the rest
# of every motion/weapon/attack constant lives in the native C++ constructor. This reader replays the literal
# stores of that constructor, as Ghidra decompiled it (extract/native/decomp_r1/<Class>.cpp, e.g.
# "*(undefined4 *)((longlong)param_1 + 0xb74) = 0x3d4ccccd;"), into a zeroed byte buffer, then names each
# offset with the PDB layout (extract/native/types/<Class>.h, "/* 0x0b74 */ float FeintWindow;").
# Nothing is typed in by hand: every value is the constructor's own immediate operand.
#
# Rules followed (each one is what the decompiled C says):
#   - base class constructor runs first (every ctor starts with "<Base>__<Base>();"); chain from the header's
#     "class X : public Base".
#   - "Struct__Struct(param_1 + N)" runs that struct's own constructor at N (FAttackInfo inside UAttackMotion).
#   - "param_1[N] = V" / "*(T *)(param_1 + N) = V" index by the pointee size of param_1's declared type.
#   - TArray pushes ("*(undefined4 *)(*(longlong *)(param_1 + N) + (longlong)iVar2 * 4) = V") append V to the
#     array whose TArray header is at N (FAttackInfo::Damage, HeadBonus, LegBonus).
#   - A store whose right side is an .rdata constant (_DAT_<va>, directly, through a local, shifted or CONCAT44'd) takes
#     the bytes from extract/native/rdata.tsv; other non-literal right sides (.data globals, runtime values) are
#     skipped and those fields stay 0.
# The zeroed start buffer is UE's: UObject memory is zero-filled before the constructor runs
# (UE 4.26 StaticAllocateObject). For structs every field read here is written by its own ctor.
class_name NativeCtor

static var _layout_cache := {}
static var _ctor_cache := {}

static func _dir(sub: String) -> String:
	return UePkg.root() + "/native/" + sub + "/"

static func _read(path: String) -> String:
	var f := FileAccess.open(path, FileAccess.READ)
	return f.get_as_text() if f else ""

static func has_type(cls: String) -> bool:
	return FileAccess.file_exists(_dir("types") + cls + ".h")

# Header -> {size, base, fields:[{off,type,name}]}
static func layout(cls: String) -> Dictionary:
	if _layout_cache.has(cls):
		return _layout_cache[cls]
	var txt := _read(_dir("types") + cls + ".h")
	var out := {"size": 0, "base": "", "fields": []}
	var m := RegEx.create_from_string("sizeof = 0x([0-9a-f]+)").search(txt)
	if m:
		out.size = m.get_string(1).hex_to_int()
	m = RegEx.create_from_string("(?m)^(?:class|struct) \\w+ : public (\\w+)").search(txt)
	if m:
		out.base = m.get_string(1)
	for f in RegEx.create_from_string("(?m)^\\t/\\* 0x([0-9a-f]+) \\*/ (.+) (\\w+);").search_all(txt):
		out.fields.append({"off": f.get_string(1).hex_to_int(), "type": f.get_string(2), "name": f.get_string(3)})
	_layout_cache[cls] = out
	return out

# One ctor body: the text between "// Cls::Cls  rva=" and the next "// " header.
static func _ctor_body(cls: String) -> String:
	# Constructor stores are replayed from the untyped round-1 output (extract/native/decomp_r1: "param_1 + 0xNN"
	# offsets). Round 2 (extract/native/decomp, PDB types applied) prints the same machine code as "this->Field";
	# the offset form is what this reader names with the headers, so r1 is used when present.
	var dir := _dir("decomp_r1") if DirAccess.dir_exists_absolute(_dir("decomp_r1")) else _dir("decomp")
	var txt := _read(dir + cls + ".cpp")
	var key := "// %s::%s  rva=" % [cls, cls]
	var i := txt.find(key)
	if i < 0:
		return ""
	var j := txt.find("\n// ", i + key.length())
	return txt.substr(i, (j if j > 0 else txt.length()) - i)

static func ctor_rva(cls: String) -> String:
	var m := RegEx.create_from_string("rva=(0x[0-9a-f]+)").search(_ctor_body(cls))
	return m.get_string(1) if m else ""

const _TSIZE := {"undefined1": 1, "byte": 1, "char": 1, "bool": 1, "undefined2": 2, "short": 2, "ushort": 2,
	"undefined4": 4, "int": 4, "uint": 4, "float": 4, "undefined8": 8, "longlong": 8, "ulonglong": 8}

static func _lit(s: String):
	s = s.strip_edges()
	var neg := s.begins_with("-")
	if neg:
		s = s.substr(1)
	if s.begins_with("0x") and s.substr(2).is_valid_hex_number():
		return (-1 if neg else 1) * s.substr(2).hex_to_int()
	if s.is_valid_int():
		return (-1 if neg else 1) * s.to_int()
	return null

# Right-hand side of a constructor store: a literal, a constant the code loads from .rdata (_DAT_<va>, raw bytes from
# extract/native/rdata.tsv through UeRdata, little-endian), a local holding one, "(ulonglong)X << 0x20", or
# "CONCAT44(hi,lo)". Anything else (.data globals such as FVector::ZeroVector, runtime values) -> null (not stored).
static func _rhs(e: String, locals: Dictionary):
	e = e.strip_edges()
	var v = _lit(e)
	if v != null:
		return v
	if locals.has(e):
		return locals[e]
	if e.begins_with("_DAT_") and e.substr(5).is_valid_hex_number():
		var va := e.substr(5).hex_to_int()
		if not UeRdata.has(va):
			return null
		var raw := UeRdata.row(va)[3].hex_decode()
		var out := 0
		for k in mini(raw.size(), 8):
			out |= raw[k] << (8 * k)
		return out
	var m := RegEx.create_from_string("^\\(\\w+\\)(\\w+) << 0x20$").search(e)
	if m:
		var x = _rhs(m.get_string(1), locals)
		return (x & 0xffffffff) << 32 if x != null else null
	m = RegEx.create_from_string("^CONCAT44\\((\\w+),(\\w+)\\)$").search(e)
	if m:
		var hi = _rhs(m.get_string(1), locals)
		var lo = _rhs(m.get_string(2), locals)
		return ((hi & 0xffffffff) << 32) | (lo & 0xffffffff) if hi != null and lo != null else null
	return null

static func _put(buf: PackedByteArray, off: int, size: int, v: int) -> void:
	for k in size:
		if off + k < buf.size():
			buf[off + k] = (v >> (8 * k)) & 0xff

# Replays cls's ctor (and its base chain) into buf at base offset `at`; arrays[off] collects TArray pushes.
static func _apply(cls: String, buf: PackedByteArray, at: int, arrays: Dictionary) -> void:
	var base: String = layout(cls).base
	if base != "" and has_type(base):
		_apply(base, buf, at, arrays)
	var body := _ctor_body(cls)
	if body == "":
		return
	# param_1's declared pointee type decides what "param_1 + N" means (Ghidra pointer arithmetic).
	var esz := 1
	var sig := RegEx.create_from_string("\\((\\w+) \\*param_1").search(body)
	if sig:
		esz = _TSIZE.get(sig.get_string(1), 1)
	var elem_t: String = sig.get_string(1) if sig else "undefined1"
	var alias := {}   # plVar1 = (longlong *)(param_1 + 0x20);
	var re_alias := RegEx.create_from_string("^(\\w+) = \\(\\w+ \\*\\)\\(param_1 \\+ (\\w+)\\);$")
	var re_byte := RegEx.create_from_string("^\\*\\((\\w+) \\*\\)\\(\\(longlong\\)param_1 \\+ (\\w+)\\) = (.+);$")
	var re_idx := RegEx.create_from_string("^\\*\\((\\w+) \\*\\)\\(param_1 \\+ (\\w+)\\) = (.+);$")
	var re_arr := RegEx.create_from_string("^param_1\\[(\\w+)\\] = (.+);$")
	var re_local := RegEx.create_from_string("^([a-z]Var\\d+|[a-z]Stack_\\w+) = (.+);$")
	var locals := {}  # uVar7 = _DAT_144349db8; -> value bits
	var re_push := RegEx.create_from_string("^\\*\\((\\w+) \\*\\)\\(\\*(?:\\(longlong \\*\\)\\(param_1 \\+ (\\w+)\\)|(\\w+)) \\+ \\(longlong\\)\\w+ \\* \\d+\\) = (-?\\w+);$")
	var re_sub := RegEx.create_from_string("^(\\w+)__(\\w+)\\((?:\\(longlong\\))?param_1 \\+ (\\w+)\\);$")
	# Whole-object aliases: "puStack_450 = param_1;" then "puVar19 = puStack_450;" (AMordhauGameMode's ctor stores
	# bCanEquipmentDrop / bSpawnDefaultEquipment / bSuicideAllowed through puVar19, decomp_r1 line 1437-1438). A store
	# through such a local scales by the local's own declared pointee ("undefined8 *puVar19;"). Any other assignment
	# to the local ends the alias.
	var decl := {}	# local -> pointee type
	for dm in RegEx.create_from_string("(?m)^\\s+(\\w+) \\*(\\w+);").search_all(body):
		decl[dm.get_string(2)] = dm.get_string(1)
	var whole := {}	# local -> true while it holds param_1
	var re_assign := RegEx.create_from_string("^(\\w+) = (?:\\(\\w+ \\*\\))?(.+);$")
	var re_wbyte := RegEx.create_from_string("^\\*\\((\\w+) \\*\\)\\(\\(longlong\\)(\\w+) \\+ (\\w+)\\) = (.+);$")
	var re_widx := RegEx.create_from_string("^\\*\\((\\w+) \\*\\)\\((\\w+) \\+ (\\w+)\\) = (.+);$")
	for raw in body.split("\n"):
		var ln := raw.strip_edges()
		var m := re_alias.search(ln)
		if m:
			alias[m.get_string(1)] = _lit(m.get_string(2)) * esz
			whole.erase(m.get_string(1))
			continue
		m = re_assign.search(ln)
		if m and decl.has(m.get_string(1)):
			if m.get_string(2) == "param_1" or whole.has(m.get_string(2)):
				whole[m.get_string(1)] = true
				continue
			whole.erase(m.get_string(1))
		elif m:
			whole.erase(m.get_string(1))
		m = re_local.search(ln)
		if m:
			locals[m.get_string(1)] = _rhs(m.get_string(2), locals)
			continue
		if not whole.is_empty():
			m = re_wbyte.search(ln)
			var by_bytes := m != null
			if m == null:
				m = re_widx.search(ln)
			if m and whole.has(m.get_string(2)) and _TSIZE.has(m.get_string(1)):
				var wo = _lit(m.get_string(3))
				var wv = _rhs(m.get_string(4), locals)
				if wo != null and wv != null:
					var mul := 1 if by_bytes else int(_TSIZE.get(decl.get(m.get_string(2), "undefined1"), 1))
					_put(buf, at + wo * mul, _TSIZE[m.get_string(1)], wv)
				continue
		m = re_sub.search(ln)
		if m and m.get_string(1) == m.get_string(2) and has_type(m.get_string(1)):
			var mul := 1 if ln.contains("(longlong)param_1") else esz
			_apply(m.get_string(1), buf, at + _lit(m.get_string(3)) * mul, arrays)
			continue
		m = re_push.search(ln)
		if m:
			var v = _lit(m.get_string(4))
			var o = _lit(m.get_string(2)) * esz if m.get_string(2) != "" else alias.get(m.get_string(3))
			if v != null and o != null:
				if not arrays.has(at + o):
					arrays[at + o] = []
				arrays[at + o].append(v)
			continue
		var off = null
		var t := ""
		var val = null
		m = re_byte.search(ln)
		if m:
			t = m.get_string(1); off = _lit(m.get_string(2)); val = _rhs(m.get_string(3), locals)
		else:
			m = re_idx.search(ln)
			if m:
				t = m.get_string(1); off = _lit(m.get_string(2)); val = _rhs(m.get_string(3), locals)
				if off != null:
					off *= esz
			else:
				m = re_arr.search(ln)
				if m:
					t = elem_t; off = _lit(m.get_string(1)); val = _rhs(m.get_string(2), locals)
					if off != null:
						off *= esz
				elif ln.begins_with("*param_1 = "):   # store to offset 0 (FAttackInfo: *param_1 = 0x101)
					t = elem_t; off = 0; val = _lit(ln.trim_prefix("*param_1 = ").trim_suffix(";"))
		if off != null and val != null and _TSIZE.has(t):
			_put(buf, at + off, _TSIZE[t], val)

static func _f32(buf: PackedByteArray, o: int) -> float:
	return buf.decode_float(o)

# Decode one class/struct from buf at `at`, base-class fields included. Unknown field types are skipped.
static func _decode(cls: String, buf: PackedByteArray, at: int, arrays: Dictionary) -> Dictionary:
	var L := layout(cls)
	var out := {}
	if L.base != "" and has_type(L.base):
		out = _decode(L.base, buf, at, arrays)
	for f in L.fields:
		var o: int = at + f.off
		var t: String = f.type
		if o >= buf.size():
			continue
		if t == "float":
			out[f.name] = _f32(buf, o)
		elif t == "int32":
			out[f.name] = buf.decode_s32(o)
		elif t == "bool":
			out[f.name] = buf[o] != 0
		elif t == "uint8" or (t.begins_with("E") and not t.contains("<") and not t.contains("*")):
			out[f.name] = buf[o]
		elif t == "FVector2D":
			out[f.name] = {"X": _f32(buf, o), "Y": _f32(buf, o + 4)}
		elif t == "FVector":
			out[f.name] = {"X": _f32(buf, o), "Y": _f32(buf, o + 4), "Z": _f32(buf, o + 8)}
		elif t == "FRotator":
			out[f.name] = {"Pitch": _f32(buf, o), "Yaw": _f32(buf, o + 4), "Roll": _f32(buf, o + 8)}
		elif t.begins_with("TArray<float"):
			out[f.name] = arrays.get(o, []).map(func(v): return PackedByteArray([v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff]).decode_float(0))
		elif t.begins_with("F") and not t.contains("*") and not t.contains("<") and has_type(t):
			out[f.name] = _decode(t, buf, o, arrays)
	return out

# Native defaults of `cls` as a Dictionary keyed by UE property name, in the same shape mdx json writes
# (FVector2D -> {X,Y}, FPerspectiveFloat -> {ThirdPerson,FirstPerson}), so a Blueprint CDO merges over it.
static func defaults(cls: String) -> Dictionary:
	if _ctor_cache.has(cls):
		return _ctor_cache[cls]
	var buf := PackedByteArray()
	buf.resize(max(layout(cls).size, 8))
	buf.fill(0)
	var arrays := {}
	_apply(cls, buf, 0, arrays)
	var d := _decode(cls, buf, 0, arrays)
	_ctor_cache[cls] = d
	return d

# ue_rdata.gd - constants the game's own functions load from the exe's .rdata (extract/native/rdata.tsv,
# written by scripts/rdata_consts.py from the hash-checked Mordhau-Win64-Shipping.exe).
# Ghidra shows such a load as _DAT_<va>; UeRdata.f32(0x143fe4e04) returns the value stored there (0.5), so code
# reads the binary instead of a hand-copied literal.
# (Named f32/f64/i32, not get(): every GDScript class inherits Object.get, which a static func cannot redefine.)
class_name UeRdata

static var _rows := {}		# va (int) -> PackedStringArray row

static func _load() -> void:
	if not _rows.is_empty():
		return
	var f := FileAccess.open(UePkg.root() + "/native/rdata.tsv", FileAccess.READ)
	if f == null:
		push_error("UeRdata: no extract/native/rdata.tsv (run scripts/rdata_consts.py)")
		return
	f.get_line()	# header: va rdata_off width raw f32 f64 i32 i64 funcs
	while not f.eof_reached():
		var r := f.get_line().split("\t")
		if r.size() >= 9:
			_rows[r[0].hex_to_int()] = r

static func has(va: int) -> bool:
	_load()
	return _rows.has(va)

# full row: [va, rdata_off, width, raw, f32, f64, i32, i64, funcs]
static func row(va: int) -> PackedStringArray:
	_load()
	if not _rows.has(va):
		push_error("UeRdata: 0x%x is not an .rdata constant referenced by game code" % va)
		return PackedStringArray()
	return _rows[va]

# little-endian IEEE-754 from the raw bytes (not from the printed decimal columns)
static func _bytes(va: int) -> PackedByteArray:
	var r := row(va)
	return r[3].hex_decode() if r.size() > 3 else PackedByteArray()

static func f32(va: int) -> float:
	var b := _bytes(va)
	return b.decode_float(0) if b.size() >= 4 else NAN

static func f64(va: int) -> float:
	var b := _bytes(va)
	if b.size() >= 8:
		return b.decode_double(0)
	var r := row(va)		# a dword-loaded constant: f64 column holds the 8 bytes at va
	return r[5].to_float() if r.size() > 5 else NAN

static func i32(va: int) -> int:
	var b := _bytes(va)
	return b.decode_s32(0) if b.size() >= 4 else 0

static func funcs(va: int) -> PackedStringArray:
	var r := row(va)
	return r[8].split(";") if r.size() > 8 else PackedStringArray()

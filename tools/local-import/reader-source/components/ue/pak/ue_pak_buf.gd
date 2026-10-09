# ue_pak_buf.gd - little-endian cursor over a PackedByteArray: the FArchive reads the pak and package readers need
# (UE 4.26 Runtime/Core/Public/Serialization/Archive.h; CUE4Parse UE4/Readers/FArchive.cs).
# A read past the end returns 0 / "" and sets `bad` instead of raising an engine error, so a value the property
# reader mis-decodes is caught by its caller (UeAsset.tagged) and skipped by the tag's size.
class_name UePakBuf

var b: PackedByteArray
var p := 0
var bad := false

func _init(bytes: PackedByteArray) -> void:
	b = bytes

func _has(n: int) -> bool:
	if p < 0 or p + n > b.size():
		bad = true
		p += n
		return false
	p += n
	return true

func skip(n: int) -> void: p += n
func u8() -> int: return b[p - 1] if _has(1) else 0
func s8() -> int: return b.decode_s8(p - 1) if _has(1) else 0
func u16() -> int: return b.decode_u16(p - 2) if _has(2) else 0
func s16() -> int: return b.decode_s16(p - 2) if _has(2) else 0
func u32() -> int: return b.decode_u32(p - 4) if _has(4) else 0
func s32() -> int: return b.decode_s32(p - 4) if _has(4) else 0
func s64() -> int: return b.decode_s64(p - 8) if _has(8) else 0
func u64() -> int: return b.decode_u64(p - 8) if _has(8) else 0
func f32() -> float: return short32(b.decode_float(p - 4)) if _has(4) else 0.0
func f32raw() -> float: return b.decode_float(p - 4) if _has(4) else 0.0	# widened as is (bulk vertex data)
func f16() -> float: return b.decode_half(p - 2) if _has(2) else 0.0
func f64() -> float: return b.decode_double(p - 8) if _has(8) else 0.0
func bytes(n: int) -> PackedByteArray: return b.slice(p - n, p) if _has(n) else PackedByteArray()
func left() -> int: return b.size() - p

# FString: int32 length including the terminator; > 0 = Latin-1 bytes, < 0 = UTF-16LE code units (FArchive.cs ReadFString;
# UE FString operator<< in Containers/String.cpp)
func fstring() -> String:
	var n := s32()
	if n == 0:
		return ""
	if n > 0:
		if not _has(n):
			return ""
		var raw := b.slice(p - n, p - 1)
		for c in raw:
			if c >= 0x80:
				return _latin1(raw)
		return raw.get_string_from_ascii()
	if n < -(1 << 24) or not _has(-n * 2):
		bad = true
		return ""
	return b.slice(p + n * 2, p - 2).get_string_from_utf16()

static func _latin1(raw: PackedByteArray) -> String:
	var s := ""
	for c in raw:
		s += String.chr(c)
	return s

# A float32 as the shortest decimal that reads back as the same float32 (1-9 significant digits), as a double: 0.56f
# is 0.5600000023841858 widened, but CUE4Parse writes "0.56" to extract/json (Newtonsoft's shortest round-trip form),
# so both backends hand the readers the same double. f32(result) == the stored float32 always holds.
const _POW10 := [1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17,
	1e18, 1e19, 1e20, 1e21, 1e22]		# exact in binary64 up to 1e22, so round(x * 10^k) / 10^k is correctly rounded

static func short32(d: float) -> float:
	if d == 0.0 or is_nan(d) or is_inf(d):
		return d
	var want := PackedFloat32Array([d])
	var e := floori(log(absf(d)) / log(10.0))
	for sig in range(1, 10):
		var k := sig - 1 - e			# decimal places that keep `sig` significant digits
		var c := d
		if k >= 0 and k <= 22:
			c = _round_even(d * _POW10[k]) / _POW10[k]
		elif k < 0 and -k <= 22:
			c = _round_even(d / _POW10[-k]) * _POW10[-k]
		else:
			return d
		if PackedFloat32Array([c]) == want:
			return c
	return d

# Round half to even: when both neighbours of an exact tie read back as the same float32 (2689.40625 -> 2689.4062 or
# 2689.4063), .NET's shortest form keeps the even last digit (extract/json has 2689.4062)
static func _round_even(x: float) -> float:
	var r := roundf(x)
	if x - floorf(x) == 0.5:
		r = 2.0 * roundf(x / 2.0)
	return r

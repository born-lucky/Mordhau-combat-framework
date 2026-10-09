# ue_anim_sequence.gd - a cooked UE 4.26 AnimSequence read from the paks: its compressed bone tracks decoded to keys
# (UE units), and a Godot Animation built from them, without the glb that `mdx anim` writes to extract/gltf.
#
# Every shipped clip the port exports (147 under Animations/) is cooked with DefaultAnimBoneCompressionSettings into
# AKF_PerTrackCompression (extract/json CompressedDataStructure.KeyEncodingFormat, checked by test_pak_assets), so that
# is the codec decoded here; any other key encoding is reported, not guessed.
#
# Layout (CUE4Parse at the pinned commit, tools/CUE4Parse-src/CUE4Parse/UE4/Assets/Exports/Animation/...):
#   UAnimSequence.Deserialize (UAnimSequence.cs:47-130): tagged properties, UObject Guid, FGuid SkeletonGuid
#     (UAnimationAsset.cs:20-23), FStripDataFlags, bool
#     bSerializeCompressedData, then SerializeCompressedData3 (UAnimSequence.cs:310-336): int32 CompressedRawDataSize,
#     TArray<int32> CompressedTrackToSkeletonMapTable, TArray<FSmartName = FName> CompressedCurveNames, the serialized
#     byte stream (int32 NumBytes, bool bUseBulkDataForLoad, the bytes or an FByteBulkData), FString BoneCodecDDCHandle,
#     FString CurveCodecPath, int32 + curve bytes, then FUECompressedAnimData::SerializeCompressedData (base first for
#     4.25+, AnimCompressionTypes.cs:78-140): int32 CompressedNumberOfFrames, uint8 KeyEncodingFormat, Translation /
#     Rotation / Scale formats, int32 CompressedByteStream size, TrackOffsets count, ScaleOffsets count, StripSize.
#     The byte stream holds int32 TrackOffsets[] (translation, rotation per track), int32 ScaleOffsets[], then the keys
#     (InitViewsFromBuffer, AnimCompressionTypes.cs:100-106).
#   Per-track keys (CUE4Parse-Conversion/Legacy/AnimConverter.cs:332-540, AnimationCompressionUtils.cs; the same
#     decoders as scripts/anim_additive.py): uint32 header = format << 28 | component mask << 24 | key count; for
#     IntervalFixed32NoW the per-component (min, range) floats; the keys; align 4; with mask bit 3 the key frame numbers
#     (uint8 if NumFrames < 256, else uint16), align 4. Offset -1 = no track: rotation identity, translation zero, scale
#     one (AnimConverter.cs:508-537).
class_name UeAnimSequence

const AKF_PER_TRACK_COMPRESSION := 2		# AnimationKeyFormat (AnimationKeyFormat.cs)
# AnimationCompressionFormat (AnimationCompressionFormat.cs)
const ACF_NONE := 0
const ACF_FLOAT96_NO_W := 1
const ACF_FIXED48_NO_W := 2
const ACF_INTERVAL_FIXED32_NO_W := 3
const ACF_FIXED32_NO_W := 4
const ACF_FLOAT32_NO_W := 5
const ACF_IDENTITY := 6

# {NumFrames, SequenceLength, RateScale, AdditiveAnimType, Properties, Tracks: [{Bone (skeleton bone index),
# Pos: {Keys: PackedVector3Array, Frames: PackedFloat32Array} or null, Rot: {Keys: Array[Quaternion (UE x,y,z,w)],
# Frames} or null, Scale: ... or null}]}, or {} with an error
static func decode(pkg_path: String) -> Dictionary:
	var a := UeAsset.open(pkg_path)
	if a == null:
		return {}
	for e in a.exports:
		var cn = a.node(e.cls)
		if cn == null or UeAsset.node_name(cn) != "AnimSequence":
			continue
		var r := UePakBuf.new(a.data())
		r.p = e.off
		var props := a.tagged(r, e.off + e.size)
		if (e.flags & UeAsset.RF_CLASS_DEFAULT_OBJECT) == 0 and r.s32() != 0:
			r.skip(16)					# UObject Guid
		r.skip(16)						# UAnimationAsset SkeletonGuid (UAnimationAsset.cs:20-23)
		r.skip(2)						# FStripDataFlags
		if r.s32() == 0:				# bSerializeCompressedData
			return {}
		r.s32()							# CompressedRawDataSize
		var table := PackedInt32Array()
		table.resize(r.s32())
		for i in table.size():
			table[i] = r.s32()
		r.skip(8 * r.s32())				# CompressedCurveNames
		var nbytes := r.s32()
		var stream := PackedByteArray()
		if r.s32() != 0:				# bUseBulkDataForLoad
			stream = UeBulk.bytes(pkg_path, UeBulk.header(a, r)).slice(0, nbytes)
		else:
			stream = r.bytes(nbytes)
		var codec := r.fstring()		# BoneCodecDDCHandle
		r.fstring()						# CurveCodecPath
		r.skip(r.s32())					# curve bytes
		var frames := r.s32()
		var kef := r.u8()
		r.skip(3)						# translation / rotation / scale formats (per track for AKF_PerTrackCompression)
		var bs_size := r.s32()
		var n_off := r.s32()
		var n_soff := r.s32()
		r.s32()							# StripSize
		if r.bad:
			push_error("UeAnimSequence: %s does not decode" % pkg_path)
			return {}
		if kef != AKF_PER_TRACK_COMPRESSION:
			push_error("UeAnimSequence: %s key encoding %d (%s) not implemented" % [pkg_path, kef, codec])
			return {}
		var keys := stream.slice(4 * (n_off + n_soff))
		if keys.size() != bs_size:
			push_error("UeAnimSequence: %s byte stream %d, expected %d" % [pkg_path, keys.size(), bs_size])
			return {}
		var tracks := []
		for ti in table.size():
			var to := stream.decode_s32(8 * ti)
			var ro := stream.decode_s32(8 * ti + 4)
			var so := stream.decode_s32(4 * n_off + 4 * ti) if ti < n_soff else -1
			tracks.append({"Bone": table[ti],
				"Pos": _track(keys, to, false, frames) if to != -1 else null,
				"Rot": _track(keys, ro, true, frames) if ro != -1 else null,
				"Scale": _track(keys, so, false, frames) if so != -1 else null})
		return {"NumFrames": frames, "SequenceLength": float(props.get("SequenceLength", 0.0)),
			"RateScale": float(props.get("RateScale", 1.0)), "AdditiveAnimType": String(props.get("AdditiveAnimType", "AAT_None")),
			"Codec": codec, "Properties": props, "Tracks": tracks}
	return {}

static func _quat_w(x: float, y: float, z: float) -> Quaternion:
	var w2 := 1.0 - x * x - y * y - z * z
	return Quaternion(x, y, z, sqrt(w2) if w2 > 0.0 else 0.0)

static func _bits(v: int) -> float:
	var b := PackedByteArray()
	b.resize(4)
	b.encode_u32(0, v & 0xffffffff)
	return b.decode_float(0)

# FAnimationCompression_PerTrackUtils fixed-48 per-component translation (DecodeFixed48_PerTrackComponent<7>)
static func _fixed48c(v: int) -> float:
	var off := (1 << (15 - 7)) - 1
	return (v - off) * (1.0 / (off >> 7))

static func _track(b: PackedByteArray, o: int, rot: bool, num_frames: int) -> Dictionary:
	var info := b.decode_u32(o)
	o += 4
	var fmt := info >> 28
	var mask := (info >> 24) & 0xf
	var n := info & 0xffffff
	var mins := [0.0, 0.0, 0.0]
	var rng := [0.0, 0.0, 0.0]
	if fmt == ACF_INTERVAL_FIXED32_NO_W:
		for c in 3:
			if mask & (1 << c):
				mins[c] = b.decode_float(o)
				rng[c] = b.decode_float(o + 4)
				o += 8
	var keys := []
	for k in n:
		var x := 0.0
		var y := 0.0
		var z := 0.0
		if fmt == ACF_NONE or fmt == ACF_FLOAT96_NO_W:
			if rot or (mask & 7) == 0:
				x = b.decode_float(o); y = b.decode_float(o + 4); z = b.decode_float(o + 8)
				o += 12
			else:
				var v := [0.0, 0.0, 0.0]
				for c in 3:
					if mask & (1 << c):
						v[c] = b.decode_float(o)
						o += 4
				x = v[0]; y = v[1]; z = v[2]
		elif fmt == ACF_FIXED48_NO_W:
			var v := [0.0, 0.0, 0.0]
			for c in 3:
				if mask & (1 << c):
					var u := b.decode_u16(o)
					o += 2
					v[c] = (u - 32767) / 32767.0 if rot else _fixed48c(u)
			x = v[0]; y = v[1]; z = v[2]
		elif fmt == ACF_INTERVAL_FIXED32_NO_W:
			var p := b.decode_u32(o)
			o += 4
			if rot:
				x = (((p >> 21) - 1023) / 1023.0) * rng[0] + mins[0]
				y = ((((p & 0x001ffc00) >> 10) - 1023) / 1023.0) * rng[1] + mins[1]
				z = (((p & 0x3ff) - 511) / 511.0) * rng[2] + mins[2]
			else:
				x = (((p & 0x3ff) - 511) / 511.0) * rng[0] + mins[0]
				y = ((((p & 0x001ffc00) >> 10) - 1023) / 1023.0) * rng[1] + mins[1]
				z = (((p >> 21) - 1023) / 1023.0) * rng[2] + mins[2]
		elif fmt == ACF_FIXED32_NO_W and rot:
			var p := b.decode_u32(o)
			o += 4
			x = ((p >> 21) - 1023) / 1023.0
			y = (((p & 0x001ffc00) >> 10) - 1023) / 1023.0
			z = ((p & 0x3ff) - 511) / 511.0
		elif fmt == ACF_FLOAT32_NO_W and rot:
			var p := b.decode_u32(o)
			o += 4
			var ux := p >> 21
			var uy := (p & 0x001ffc00) >> 10
			var uz := p & 0x3ff
			x = _bits(((((ux >> 7) & 7) + 123) << 23) | ((ux & 0x7f | 32 * (ux & 0xfffffc00)) << 16))
			y = _bits(((((uy >> 7) & 7) + 123) << 23) | ((uy & 0x7f | 32 * (uy & 0xfffffc00)) << 16))
			z = _bits(((((uz >> 6) & 7) + 123) << 23) | ((uz & 0x3f | 32 * (uz & 0xfffffe00)) << 17))
		elif fmt == ACF_IDENTITY:
			pass
		else:
			push_error("UeAnimSequence: %s format %d not implemented" % ["rotation" if rot else "vector", fmt])
			return {}
		keys.append(_quat_w(x, y, z) if rot else Vector3(x, y, z))
	o = (o + 3) & ~3
	var times := PackedFloat32Array()
	if mask & 8 and n > 1:
		for k in n:
			times.append(b[o + k] if num_frames < 256 else b.decode_u16(o + 2 * k))
	return {"Keys": keys, "Frames": times}

# Time in seconds of key k of a track: the shipped engine spreads NumKeys over SequenceLength (KeyPos = RelativePos *
# (NumKeys - 1), AEFPerTrackCompressionCodec::GetBoneAtomRotation VA 0x142e6b330, see game/anim/additive_clip.gd);
# time-keyed tracks hold frame numbers over NumFrames - 1 intervals
static func key_time(t: Dictionary, k: int, num_frames: int, length: float) -> float:
	var n: int = t.Keys.size()
	if not t.Frames.is_empty():
		return t.Frames[k] / maxf(num_frames - 1, 1) * length
	return 0.0 if n <= 1 else float(k) / (n - 1) * length

# UE local bone transform parts -> Godot, as the glTF writer and `mdx anim` convert them (Gltf.cs:129-131, 244-250)
static func godot_rot(q: Quaternion) -> Quaternion:
	return Quaternion(q.x, q.z, q.y, -q.w).normalized()

static func godot_pos(v: Vector3) -> Vector3:
	return Vector3(v.x, v.z, v.y) * UeStaticMesh.UNIT_SCALE

# A Godot Animation of the decoded clip for a skeleton node at `skeleton_path` whose bones are named as the USkeleton's
# reference skeleton `ref` (UeAsset.ref_skeleton of the Skeleton export; tracks index its bones). `retarget` = the
# Skeleton's BoneTree TranslationRetargetingMode per bone: "Skeleton" bones take `target_ref`'s translation (the
# mesh's reference pose; FAnimationRuntime::RetargetBoneTransform as CUE4Parse ports it, CAnimSequence.cs:111-160).
# Other modes than Animation / Skeleton are reported (the UMA skeleton uses only those two).
static func to_animation(d: Dictionary, ref: Dictionary, skeleton_path: String, retarget: Array = [],
		target_ref: Dictionary = {}) -> Animation:
	if d.is_empty():
		return null
	var anim := Animation.new()
	var length: float = d.SequenceLength
	anim.length = length
	var info: Array = ref.FinalRefBoneInfo
	var tpose: Array = target_ref.get("FinalRefBonePose", ref.FinalRefBonePose)
	for t in d.Tracks:
		var b: int = t.Bone
		if b >= info.size():
			continue
		var path := NodePath("%s:%s" % [skeleton_path, info[b].Name])
		var mode := String(retarget[b]) if b < retarget.size() else "EBoneTranslationRetargetingMode::Animation"
		var ti := anim.add_track(Animation.TYPE_ROTATION_3D)
		anim.track_set_path(ti, path)
		var rk: Dictionary = t.Rot if t.Rot != null else {"Keys": [Quaternion.IDENTITY], "Frames": PackedFloat32Array()}
		for k in rk.Keys.size():
			anim.rotation_track_insert_key(ti, key_time(rk, k, d.NumFrames, length), godot_rot(rk.Keys[k]))
		ti = anim.add_track(Animation.TYPE_POSITION_3D)
		anim.track_set_path(ti, path)
		if mode.ends_with("::Skeleton"):
			var tr: Dictionary = tpose[b].Translation
			anim.position_track_insert_key(ti, 0.0, godot_pos(Vector3(tr.X, tr.Y, tr.Z)))
		else:
			if not mode.ends_with("::Animation"):
				push_error("UeAnimSequence: retargeting mode %s not implemented (bone %s)" % [mode, info[b].Name])
			var pk: Dictionary = t.Pos if t.Pos != null else {"Keys": [Vector3.ZERO], "Frames": PackedFloat32Array()}
			for k in pk.Keys.size():
				anim.position_track_insert_key(ti, key_time(pk, k, d.NumFrames, length), godot_pos(pk.Keys[k]))
		if t.Scale != null:
			ti = anim.add_track(Animation.TYPE_SCALE_3D)
			anim.track_set_path(ti, path)
			for k in t.Scale.Keys.size():
				var s: Vector3 = t.Scale.Keys[k]
				anim.scale_track_insert_key(ti, key_time(t.Scale, k, d.NumFrames, length), Vector3(s.x, s.z, s.y))
	return anim

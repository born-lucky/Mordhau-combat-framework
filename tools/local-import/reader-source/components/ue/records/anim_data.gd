# anim_data.gd - the animation code's (game/anim/*) data adapter: package JSON (mdx json) for AnimSequence /
# AnimMontage / BlendSpace / AnimBlueprint / Skeleton exports and the decoded additive clips (scripts/anim_additive.py),
# so game/anim never calls the extract readers (UePkg) itself; it reads the typed records at the end of this file
# (Montage, Segment, BlendSpace, BranchFilter, RefSkeleton) instead of UE property names.
class_name AnimData

# "Mordhau/Content/.../X.0" -> "Mordhau/Content/.../X" (mdx json appends the export index)
static func strip(obj_path: String) -> String:
	return UePkg.strip(obj_path)

# stripped package path of an object reference {ObjectName, ObjectPath}; "" for anything else
static func obj(v) -> String:
	return UePkg.strip(String(v.get("ObjectPath", ""))) if v is Dictionary else ""

# every export of a package, as CUE4Parse wrote them
static func exports(path: String) -> Array:
	return UePkg.load_pkg(path)

# the first export of `type` in a package ({} if none)
static func export_of(path: String, type: String) -> Dictionary:
	return UePkg.export_of(UePkg.load_pkg(path), type)

# the class default object of a Blueprint package (Default__X_C properties)
static func cdo(path: String) -> Dictionary:
	return UePkg.cdo(UePkg.load_pkg(path))

# merged class defaults down the Blueprint Super chain (UePkg.defaults)
static func class_defaults(path: String) -> Dictionary:
	return UePkg.defaults(UePkg.strip(path))

# extract/gltf/<package>.additive.json: an additive clip decoded by scripts/anim_additive.py
static func additive_file(ue_path: String) -> String:
	return UePkg.root() + "/gltf/" + UePkg.strip(ue_path) + ".additive.json"

# ---- typed records ------------------------------------------------------------------------------------------------
# Asset exports hold only serialized (non-default) properties, so absent fields take the engine class default; each
# default below names where it comes from. Wrong shapes and missing required fields fail loudly (UeRec).

# FAnimSegment (AnimTrack.AnimSegments[]). Defaults: the FAnimSegment constructor in UE 4.26 AnimCompositeBase.h
# (StartPos 0, AnimStartTime 0, AnimEndTime 0, AnimPlayRate 1, LoopingCount 1). UNCONFIRMED: engine header, the
# shipped constructor is not disassembled; every segment the duel plays serializes all five but AnimPlayRate.
class Segment:
	var seq := ""				# AnimReference
	var start_pos := 0.0
	var anim_start := 0.0
	var anim_end := 0.0
	var rate := 1.0
	var loops := 1

	static func read(r: UeRec) -> Segment:
		var s := Segment.new()
		s.seq = r.obj("AnimReference")
		s.start_pos = r.f_or("StartPos", 0.0)
		s.anim_start = r.f_or("AnimStartTime", 0.0)
		s.anim_end = r.f_or("AnimEndTime", 0.0)
		s.rate = r.f_or("AnimPlayRate", 1.0)
		s.loops = r.i_or("LoopingCount", 1)
		return s

	static func make(seq_path: String, length: float) -> Segment:
		var s := Segment.new()
		s.seq = seq_path
		s.anim_end = length
		return s

# UAnimMontage. Defaults: UAnimMontage::UAnimMontage (VA 0x142e7f430, anim_montage.gd header): BlendIn / BlendOut
# BlendTime 0.25, BlendOption Linear (FAlphaBlend::FAlphaBlend 0x142e42410), BlendOutTriggerTime -1. SequenceLength
# is required. SlotName absent = FAnimSlotGroup::DefaultSlotName "DefaultSlot" (FSlotAnimationTrack ctor, UE 4.26
# AnimMontage.h; UNCONFIRMED: not disassembled).
class Montage:
	var path := ""
	var length := 0.0
	var slot := "DefaultSlot"
	var segments: Array[Segment] = []		# first slot track's segments
	var blend_in_time := 0.25
	var blend_in_option := "Linear"			# EAlphaBlendOption name
	var blend_out_time := 0.25
	var blend_out_option := "Linear"
	var blend_out_trigger_time := -1.0

static var _montages := {}

static func is_montage(path: String) -> bool:
	return export_of(path, "AnimMontage").size() > 0

static func montage(path: String) -> Montage:
	if _montages.has(path):
		return _montages[path]
	var r := UeRec.new(export_of(path, "AnimMontage").get("Properties", null), path)
	var m := Montage.new()
	m.path = path
	m.length = r.f("SequenceLength")
	var bi := r.sub_or("BlendIn")
	var bo := r.sub_or("BlendOut")
	m.blend_in_time = bi.f_or("BlendTime", 0.25)
	m.blend_in_option = bi.enum_or("BlendOption", "Linear")
	m.blend_out_time = bo.f_or("BlendTime", 0.25)
	m.blend_out_option = bo.enum_or("BlendOption", "Linear")
	m.blend_out_trigger_time = r.f_or("BlendOutTriggerTime", -1.0)
	var tracks := r.arr_or("SlotAnimTracks")
	if not tracks.is_empty():
		var tr := UeRec.new(tracks[0], path + ".SlotAnimTracks[0]", r.errors)
		m.slot = tr.s_or("SlotName", "DefaultSlot")
		for s in tr.sub_or("AnimTrack").arr_or("AnimSegments"):
			m.segments.append(Segment.read(UeRec.new(s, path + ".AnimSegments[]", r.errors)))
	_montages[path] = r.done(m)
	return _montages[path]

# UAnimSequenceBase SequenceLength of an AnimSequence (required: every cooked sequence serializes it)
static func sequence_length(path: String) -> float:
	var r := UeRec.new(export_of(path, "AnimSequence").get("Properties", null), path)
	var v := r.f("SequenceLength")
	return v if r.done(true) != null else 0.0

# UAnimSequence AdditiveAnimType != AAT_None. Absent = AAT_None, the enum's 0 (UObject zero fill; the
# UAnimSequence ctor does not change it - UNCONFIRMED: engine ctor not disassembled).
static func is_additive(path: String) -> bool:
	var r := UeRec.new(export_of(path, "AnimSequence").get("Properties", {}), path)
	return r.enum_or("AdditiveAnimType", "AAT_None") != "AAT_None"

# what kind of animation asset a package is ("" when none of these)
static func asset_kind(path: String) -> String:
	for e in exports(path):
		var t := String(e.get("Type", ""))
		if t in ["AnimMontage", "AnimSequence", "BlendSpace", "BlendSpace1D", "AimOffsetBlendSpace"]:
			return t
	return ""

# 2D blend space (UBlendSpace / UAimOffsetBlendSpace / UBlendSpace1D export). FBlendParameter defaults: Min 0,
# Max 100, GridNum 4; FBlendSample SampleValue zero; FEditorElement Indices INDEX_NONE (-1), Weights 0 (UE 4.26
# BlendSpaceBase.h constructors; UNCONFIRMED: not disassembled, and the cooked BS_* packages serialize Min/Max/GridNum).
class Sample:
	var seq := ""
	var value := Vector2.ZERO

class GridElement:
	var indices := PackedInt32Array()		# samples with weight > 0
	var weights := PackedFloat32Array()

class BlendSpace:
	var path := ""
	var samples: Array[Sample] = []
	var min_v := Vector2.ZERO
	var max_v := Vector2(100, 100)
	var grid_n := Vector2i(4, 4)
	var grid: Array[GridElement] = []

static var _blend_spaces := {}

static func blend_space(path: String) -> BlendSpace:
	if _blend_spaces.has(path):
		return _blend_spaces[path]
	var e := {}
	for x in exports(path):
		if String(x.get("Type", "")) in ["BlendSpace", "BlendSpace1D", "AimOffsetBlendSpace"]:
			e = x
			break
	var r := UeRec.new(e.get("Properties", null), path)
	var b := BlendSpace.new()
	b.path = path
	for s in r.arr_or("SampleData"):
		var sr := UeRec.new(s, path + ".SampleData[]", r.errors)
		var smp := Sample.new()
		smp.seq = sr.obj("Animation")
		var sv := sr.v3_or("SampleValue", Vector3.ZERO)
		smp.value = Vector2(sv.x, sv.y)
		b.samples.append(smp)
	var p0 := r.sub_or("BlendParameters")
	var p1 := r.sub_or("BlendParameters[1]")
	b.min_v = Vector2(p0.f_or("Min", 0.0), p1.f_or("Min", 0.0))
	b.max_v = Vector2(p0.f_or("Max", 100.0), p1.f_or("Max", 100.0))
	b.grid_n = Vector2i(p0.i_or("GridNum", 4), p1.i_or("GridNum", 4))
	for g in r.arr_or("GridSamples"):
		var gr := UeRec.new(g, path + ".GridSamples[]", r.errors)
		var el := GridElement.new()
		for k in ["", "[1]", "[2]"]:
			var i := gr.i_or("Indices" + k, -1)
			var w := gr.f_or("Weights" + k, 0.0)
			if i >= 0 and w > 0.0:
				el.indices.append(i)
				el.weights.append(w)
		b.grid.append(el)
	_blend_spaces[path] = r.done(b)
	return _blend_spaces[path]

# Every object reference under .../Animations/ inside a value (class defaults, struct), FirstPerson halves and keys
# naming first person ("1P", "FirstPerson") skipped: the asset paths an item or motion can make a third-person body
# play (AnimRefs). A generic walk over the reference graph, not a field read.
static func animation_refs(v, out: Dictionary) -> void:
	if v is Dictionary:
		if v.has("ObjectPath") and v.ObjectPath is String and String(v.ObjectPath).contains("/Animations/"):
			out[UePkg.strip(String(v.ObjectPath))] = true
		for k in v:
			var ks := String(k)
			if ks == "FirstPerson" or ks.contains("1P") or ks.contains("FirstPerson"):
				continue
			animation_refs(v[k], out)
	elif v is Array:
		for x in v:
			animation_refs(x, out)

# ---- AnimBlueprint graph nodes (AB_MordhauCharacterAnimation CDO; see game/anim/anim_graph_data.gd) -----------------
# FBranchFilter {BoneName, BlendDepth}: both serialized when set; BlendDepth absent = 0 (FBranchFilter ctor, UE 4.26
# AnimNode_LayeredBoneBlend.h; UNCONFIRMED)
class BranchFilter:
	var bone := ""
	var depth := 0

static func layer_filters(node: Dictionary, i: int, label: String) -> Array[BranchFilter]:
	var r := UeRec.new(node, label)
	var out: Array[BranchFilter] = []
	var ls := r.arr("LayerSetup")
	if i < ls.size():
		for f in UeRec.new(ls[i], label + ".LayerSetup[%d]" % i, r.errors).arr("BranchFilters"):
			var fr := UeRec.new(f, label + ".BranchFilters[]", r.errors)
			var bf := BranchFilter.new()
			bf.bone = fr.s("BoneName")
			bf.depth = fr.i_or("BlendDepth", 0)
			out.append(bf)
	r.done(true)
	return out

# the baked FPerBoneBlendWeight array of a LayeredBoneBlend node (BlendWeight absent = 0: FPerBoneBlendWeight ctor)
static func per_bone_weights(node: Dictionary, label: String) -> PackedFloat32Array:
	var r := UeRec.new(node, label)
	var out := PackedFloat32Array()
	for w in r.arr("PerBoneBlendWeights"):
		out.append(UeRec.new(w, label + ".PerBoneBlendWeights[]", r.errors).f_or("BlendWeight", 0.0))
	r.done(true)
	return out

static func node_slot_name(node: Dictionary, label: String) -> String:
	var r := UeRec.new(node, label)
	var s := r.s("SlotName")
	r.done(true)
	return s

# FBoneReference BoneName of each named field of a node (AttackAngling: Head, Neck, Spine, LeftShoulder, ...)
static func bone_refs(node: Dictionary, fields: Array, label: String) -> Dictionary:
	var r := UeRec.new(node, label)
	var out := {}
	for f in fields:
		out[f] = r.sub(f).s("BoneName")
	r.done(true)
	return out

# Reference skeleton in the order PerBoneBlendWeights is indexed in: the FinalRefBoneInfo bones, then the
# VirtualBones (each parented to its SourceBoneName)
class RefSkeleton:
	var names := PackedStringArray()
	var parents := PackedInt32Array()

static func ref_skeleton(skeleton_path: String) -> RefSkeleton:
	var e := export_of(skeleton_path, "Skeleton")
	var r := UeRec.new(e, skeleton_path)
	var out := RefSkeleton.new()
	for b in r.sub("ReferenceSkeleton").arr("FinalRefBoneInfo"):
		var br := UeRec.new(b, skeleton_path + ".FinalRefBoneInfo[]", r.errors)
		out.names.append(br.s("Name"))
		out.parents.append(br.i("ParentIndex"))
	for v in r.sub("Properties").arr_or("VirtualBones"):
		var vr := UeRec.new(v, skeleton_path + ".VirtualBones[]", r.errors)
		out.parents.append(out.names.find(vr.s("SourceBoneName")))
		out.names.append(vr.s("VirtualBoneName"))
	r.done(true)
	return out

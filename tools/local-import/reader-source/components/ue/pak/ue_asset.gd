# ue_asset.gd - one cooked UE 4.26 package (.uasset header + .uexp export data) read straight from the paks, and its
# exports turned into the same Dictionary shape `mdx json` (CUE4Parse) writes to extract/json, so UePkg's readers can
# use either source (UePkgPak). Format sources per step are cited inline and collected in docs/PAK_FORMAT.md.
#
# UE 4.26 names: FPackageFileSummary operator<< (CoreUObject/Private/UObject/PackageFileSummary.cpp), FObjectImport /
# FObjectExport operator<< (ObjectResource.cpp), FPropertyTag operator<< (PropertyTag.cpp), UStruct::SerializeTaggedProperties
# (Class.cpp), UObject::Serialize (Obj.cpp), UDataTable::LoadStructData (Engine/Private/DataTable.cpp).
# Cross-checked byte for byte against CUE4Parse at the pinned commit (tools/CUE4Parse-src/CUE4Parse/...).
class_name UeAsset

const PACKAGE_FILE_TAG := 0x9E2A83C1		# PACKAGE_FILE_TAG (FPackageFileSummary.cs:123-163)
const LEGACY_FILE_VERSION_426 := -7		# UE 4.26 writes -7 (FPackageFileSummary.cs:145-188: -7 = UE4 versions, no UE5 field)
# EUnrealEngineObjectUE4Version values for versioned headers (CUE4Parse UE4/Versions/ObjectVersion.cs: the enum counts
# up from OLDEST_LOADABLE_PACKAGE = 214 at line 298, one per enumerator; these are lines 797, 887, 911. Check: the
# count gives CORRECT_LICENSEE_FLAG = 522 = UE 4.26's VER_UE4_AUTOMATIC_VERSION)
const VER_SERIALIZE_TEXT_IN_PACKAGES := 459
const VER_NAME_HASHES_SERIALIZED := 504
const VER_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID := 516
const PKG_FILTER_EDITOR_ONLY := 0x80000000	# EPackageFlags::PKG_FilterEditorOnly: no LocalizationId (FPackageFileSummary.cs:277-282)
const RF_CLASS_DEFAULT_OBJECT := 0x10		# EObjectFlags::RF_ClassDefaultObject: no object Guid after the properties (UObject.cs:226)
const CACHE_MAX := 512					# packages kept open (headers are small; see CACHE_MAX_BYTES for export data)
const CACHE_MAX_BYTES := 128 << 20		# export bytes kept in memory before the cache is dropped

# UStruct exports: after UObject's tagged properties (+ Guid) comes SuperStruct (UStruct.cs:18-37)
const STRUCT_TYPES := ["BlueprintGeneratedClass", "WidgetBlueprintGeneratedClass", "AnimBlueprintGeneratedClass",
	"Function", "DelegateFunction", "SparseDelegateFunction", "UserDefinedStruct", "ScriptStruct", "Class"]
# Structs that UE serializes natively (not as tagged properties) and that this reader decodes. Field order from the
# UE 4.26 structs (Core/Public/Math/*.h; CoreUObject NoExportTypes.h) as read by CUE4Parse FScriptStruct.cs:78-198.
const SOFT_PATH_STRUCTS := ["SoftObjectPath", "SoftClassPath", "StringAssetReference", "StringClassReference"]
# Natively serialized structs this reader does not decode (skipped by tag size, listed in `unsupported`): sequencer,
# curve and material-input internals no game reader uses (CUE4Parse FScriptStruct.cs:78-198 lists their layouts)
const BINARY_STRUCTS := ["Plane", "PerPlatformFloat", "PerPlatformInt", "PerPlatformBool", "SimpleCurveKey", "Sphere",
	"TwoVectors", "MaterialAttributesInput", "ExpressionInput", "ColorMaterialInput", "ScalarMaterialInput",
	"VectorMaterialInput", "Vector2MaterialInput", "SmartName", "FontCharacter", "UniqueNetIdRepl",
	"MovieSceneFloatChannel", "MovieSceneEvaluationKey", "SectionEvaluationDataTree",
	"LevelSequenceObjectReferenceMap", "MovieSceneSegment", "MovieSceneTrackIdentifier", "MovieSceneSequenceID",
	"MovieSceneEvalTemplatePtr", "MovieSceneTrackImplementationPtr", "NameCurveKey", "StringCurveKey",
	"SkeletalMeshSamplingLODBuiltData", "CompressedRichCurve", "MovieSceneSegmentIdentifier", "MovieSceneSubSequenceTree",
	"MovieSceneEventParameters", "MovieSceneSequenceInstanceDataPtr", "MovieSceneEvaluationFieldEntityTree",
	"MovieSceneFloatValue", "ClothLODData", "ClothTetherData", "NiagaraVariable", "NiagaraVariableBase",
	"NiagaraVariableWithOffset", "Transform3f", "MovieSceneTrackFieldData", "MovieSceneSubSectionFieldData",
	"SkeletalMeshSamplingRegionBuiltData", "PerPlatformFrameRate", "PerQualityLevelInt", "PerQualityLevelFloat"]
# UEnum exports: after the tagged properties come the enum's own Names and CppForm (UEnum.cs)
const ENUM_TYPES := ["UserDefinedEnum", "Enum"]
const CPP_FORM := ["Regular", "Namespaced", "EnumClass"]	# UEnum::ECppForm (UEnum.cs)
const NET_QUANTIZE := ["Vector_NetQuantize", "Vector_NetQuantize10", "Vector_NetQuantize100", "Vector_NetQuantizeNormal"]
# ERichCurveInterpMode / ERichCurveTangentMode / ERichCurveTangentWeightMode (Engine/Classes/Curves/RichCurve.h)
const RCIM := ["RCIM_Linear", "RCIM_Constant", "RCIM_Cubic", "RCIM_None"]
const RCTM := ["RCTM_Auto", "RCTM_User", "RCTM_Break", "RCTM_None"]
const RCTWM := ["RCTWM_WeightedNone", "RCTWM_WeightedArrive", "RCTWM_WeightedLeave", "RCTWM_WeightedBoth"]

var name := ""				# "Mordhau/Content/.../BP_X" (stored spelling, no extension)
var ext := ".uasset"
var header_size := 0		# FPackageFileSummary::TotalHeaderSize = size of the .uasset; export offsets continue into .uexp
var package_flags := 0
var bulk_data_start := 0		# FPackageFileSummary::BulkDataStartOffset (header + export data; .ubulk offsets count from it)
var names := PackedStringArray()
var imports: Array[Dictionary] = []	# {class_package, class_name, outer, name}
var exports: Array[Dictionary] = []	# {cls, sup, tmpl, outer, name, flags, size, off}
var unsupported := PackedStringArray()	# property values this reader could not decode ("<type>: <why>"), skipped by tag size
var _data := PackedByteArray()		# .uasset + .uexp, loaded on the first export read
var _ok := false

static var _cache := {}
static var _cache_bytes := 0
static var _plugin_roots := {}

# Cached package by path ("Mordhau/Content/.../BP_X", any case; a trailing ".N" export index is dropped). null if missing.
static func open(pkg_path: String) -> UeAsset:
	var p := pkg_path.get_basename() if pkg_path.get_extension().is_valid_int() else pkg_path
	var k := p.to_lower()
	if _cache.has(k):
		return _cache[k]
	if _cache.size() >= CACHE_MAX or _cache_bytes > CACHE_MAX_BYTES:
		clear_cache()
	var a := UeAsset.new()
	if not a._open(p):
		a = null
	_cache[k] = a
	return a

static func clear_cache() -> void:
	_cache.clear()
	_cache_bytes = 0

func _open(p: String) -> bool:
	if not UePakVfs.has(p + ext):
		ext = ".umap"
		if not UePakVfs.has(p + ext):
			return false
	name = UePakVfs.spelling(p + ext).trim_suffix(ext)
	var ua := UePakVfs.read(p + ext)
	var r := UePakBuf.new(ua)
	var sm := summary(r, p)
	if sm.is_empty():
		return false
	header_size = sm.header_size
	package_flags = sm.package_flags
	bulk_data_start = sm.get("bulk_data_start", 0)
	var name_count: int = sm.name_count
	var name_offset: int = sm.name_offset
	var export_count: int = sm.export_count
	var export_offset: int = sm.export_offset
	var import_count: int = sm.import_count
	var import_offset: int = sm.import_offset
	# Name map: FNameEntrySerialized = FString + uint16 NonCasePreservingHash + uint16 CasePreservingHash
	# (FNameEntrySerialized.cs:27; CUE4Parse also Trim()s the string there, UE does not: names are kept as stored)
	r.p = name_offset
	names.resize(name_count)
	for i in name_count:
		names[i] = r.fstring()
		r.skip(4)
	# Import map: ClassPackage, ClassName (FName), OuterIndex (FPackageIndex), ObjectName (ObjectResource.cs:377-382)
	r.p = import_offset
	for i in import_count:
		var im := {}
		im["class_package"] = fname(r)
		im["class_name"] = fname(r)
		im.outer = r.s32()
		im.name = fname(r)
		imports.append(im)
	# Export map, 4.26 (ObjectResource.cs:214-304): ClassIndex, SuperIndex, TemplateIndex, OuterIndex, ObjectName,
	# ObjectFlags u32, SerialSize i64, SerialOffset i64, bForcedExport, bNotForClient, bNotForServer (i32 x3),
	# PackageGuid (16), PackageFlags u32, bNotAlwaysLoadedForEditorGame, bIsAsset (i32 x2), FirstExportDependency and
	# the 4 dependency counts (i32 x5) = 104 bytes
	r.p = export_offset
	for i in export_count:
		var ex := {}
		ex.cls = r.s32()
		ex.sup = r.s32()
		ex.tmpl = r.s32()
		ex.outer = r.s32()
		ex.name = fname(r)
		ex.flags = r.u32()
		ex.size = r.s64()
		ex.off = r.s64()
		r.skip(12 + 16 + 4 + 8 + 20)
		exports.append(ex)
	_ok = true
	return true

# FPackageFileSummary, 4.26 cooked (FPackageFileSummary.cs:123-532), up to the import map offset; {} (and an error) for
# anything but an unversioned 4.26 cook or when TotalHeaderSize is not the .uasset's size. `versioned_ok` also accepts
# a versioned header (one package in Mordhau, Slate/Pointer, FileVersionUE4 514, an editor asset that shipped, so no
# LocalizationId): its
# optional fields follow the version (FPackageFileSummary.cs:276-288), for readers of the maps alone.
static func summary(r: UePakBuf, p: String, versioned_ok := false) -> Dictionary:
	if r.u32() != PACKAGE_FILE_TAG:
		push_error("UeAsset: not a package " + p)
		return {}
	var legacy := r.s32()
	if legacy != LEGACY_FILE_VERSION_426:
		push_error("UeAsset: LegacyFileVersion %d in %s (only 4.26's -7 is implemented)" % [legacy, p])
		return {}
	r.skip(4)					# LegacyUE3Version
	var ue4 := r.s32()			# FileVersionUE4: 0 = unversioned cook (all of Mordhau): read as the engine's latest, 4.26
	r.skip(4)					# FileVersionLicenseeUE4
	if ue4 != 0 and not versioned_ok:
		push_error("UeAsset: versioned package (FileVersionUE4 %d) in %s: only unversioned 4.26 cooks are implemented" % [ue4, p])
		return {}
	r.skip(20 * r.s32())		# CustomVersions: FCustomVersion {FGuid Key; int32 Version}
	var sm := {}
	sm.header_size = r.s32()	# TotalHeaderSize
	r.fstring()					# FolderName
	sm.package_flags = r.u32()
	sm.name_count = r.s32()
	sm.name_offset = r.s32()
	sm.name_hashes = ue4 == 0 or ue4 >= VER_NAME_HASHES_SERIALIZED
	if (sm.package_flags & PKG_FILTER_EDITOR_ONLY) == 0 and (ue4 == 0 or ue4 >= VER_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID):
		r.fstring()				# LocalizationId
	if ue4 == 0 or ue4 >= VER_SERIALIZE_TEXT_IN_PACKAGES:
		r.skip(8)				# GatherableTextDataCount, GatherableTextDataOffset
	sm.export_count = r.s32()
	sm.export_offset = r.s32()
	sm.import_count = r.s32()
	sm.import_offset = r.s32()
	if ue4 == 0:
		# the rest up to BulkDataStartOffset, 4.26 cooked (FPackageFileSummary.cs:320-507)
		r.skip(4 + 8 + 4 + 4)		# DependsOffset, SoftPackageReferences count + offset, SearchableNamesOffset, ThumbnailTableOffset
		r.skip(16)					# Guid (no PersistentGuid: PKG_FilterEditorOnly)
		r.skip(8 * r.s32())			# Generations: FGenerationInfo {int32 ExportCount, NameCount}
		for _i in 2:				# SavedByEngineVersion, CompatibleWithEngineVersion: FEngineVersion = uint16 x3,
			r.skip(10)				# uint32 Changelist, FString Branch (FEngineVersion.cs)
			r.fstring()
		r.skip(4)					# CompressionFlags
		r.skip(16 * r.s32())		# CompressedChunks: FCompressedChunk = 4 x int32
		r.skip(4)					# PackageSource
		for _i in r.s32():			# AdditionalPackagesToCook: TArray<FString>
			r.fstring()
		r.skip(4)					# AssetRegistryDataOffset
		sm.bulk_data_start = r.s64()	# BulkDataStartOffset: bulk data offsets are relative to it unless NoOffsetFixUp
	# a split cook's .uasset is exactly the header; an uncooked (versioned) package holds its exports after it
	if (sm.header_size != r.b.size() and not (ue4 != 0 and sm.header_size < r.b.size())) or r.bad:
		push_error("UeAsset: TotalHeaderSize %d != .uasset size %d in %s" % [sm.header_size, r.b.size(), p])
		return {}
	return sm

# Class name of a package's first export, the column `mdx list` writes to extract/manifest.tsv, read from the .uasset
# header alone: export 0's ClassIndex (an import, FPackageIndex < 0) -> that import's ObjectName -> the name map entry,
# walking the name map only up to that entry. "" for a missing package or a class that is not an import.
static func first_export_class(pkg_path: String) -> String:
	var ua := UePakVfs.read(pkg_path + ".uasset")
	if ua.is_empty():
		return ""
	var r := UePakBuf.new(ua)
	var sm := summary(r, pkg_path, true)
	if sm.is_empty() or sm.export_count == 0:
		return ""
	var cls := ua.decode_s32(sm.export_offset)
	if cls >= 0 or -cls > sm.import_count:
		return ""
	# FObjectImport = ClassPackage FName (8) + ClassName FName (8) + OuterIndex (4) + ObjectName FName (8) = 28 bytes
	var im: int = sm.import_offset + (-cls - 1) * 28
	var ni := ua.decode_s32(im + 20)
	var num := ua.decode_s32(im + 24)
	if ni < 0 or ni >= sm.name_count:
		return ""
	r.p = sm.name_offset
	for _i in ni:				# FNameEntrySerialized: FString (int32 length; < 0 = UTF-16 units) + 2 uint16 hashes
		var n := r.s32()
		r.skip((n if n >= 0 else -n * 2) + (4 if sm.name_hashes else 0))
	var s := r.fstring()
	return s if num == 0 else "%s_%d" % [s, num - 1]

# FName in a package = int32 name-map index + int32 Number; Number > 0 prints as "<name>_<Number-1>" (FName.cs Text)
func fname(r: UePakBuf) -> String:
	var i := r.s32()
	var n := r.s32()
	var s: String = names[i] if i >= 0 and i < names.size() else "None"
	return s if n == 0 else "%s_%d" % [s, n - 1]

func data() -> PackedByteArray:
	if _data.is_empty():
		_data = UePakVfs.read(name + ext)
		_data.append_array(UePakVfs.read(name + ".uexp"))
		_cache_bytes += _data.size()
	return _data

# ---- object references (CUE4Parse Package.ResolvePackageIndex, Package.cs:332-426; ResolvedObject, AbstractUePackage.cs) ----
# A node is [kind, asset, index]: kind 0 = this package itself, 1 = export, 2 = unresolved import.

func node(idx: int):
	if idx == 0:
		return null
	if idx > exports.size() or -idx > imports.size():
		unsupported.append("package index %d out of range (%d exports, %d imports)" % [idx, exports.size(), imports.size()])
		return null
	if idx > 0:
		return [1, self, idx - 1]
	return _resolve_import(-idx - 1)

# An import whose outermost package is a content package resolves to that package's export (matched by name and outer
# path), so ObjectPath carries the target's export index; /Script imports stay imports (Package.cs:343-426).
func _resolve_import(ii: int):
	var im: Dictionary = imports[ii]
	var o := -ii - 1
	while true:
		if o > 0:
			return [2, self, ii]
		var om: Dictionary = imports[-o - 1]
		if om.outer == 0:
			break
		o = om.outer
	var top: String = imports[-o - 1].name
	if top.begins_with("/Script/"):
		return [2, self, ii]
	var target := UeAsset.package_path(top)
	var ip: UeAsset = UeAsset.open(target) if target != "" else null
	if ip == null:
		return [2, self, ii]
	var outer = null
	if o != im.outer and im.outer < 0:
		outer = UeAsset.path_name(_resolve_import(-im.outer - 1), true)
	for i in ip.exports.size():
		var e: Dictionary = ip.exports[i]
		if e.name != im.name:
			continue
		var to = ip.node(e.outer)
		if (UeAsset.path_name(to, true) if to != null else null) == outer:
			return [1, ip, i]
	return [2, self, ii]

# "/Game/X" -> "Mordhau/Content/X", "/Engine/X" -> "Engine/Content/X", "/<Plugin>/X" -> "<plugin dir>/Content/X"
# (the mount table UE builds from the project and its .uplugin files; CUE4Parse FixPath does the same)
static func package_path(game_path: String) -> String:
	if game_path.begins_with("/Game/"):
		return "Mordhau/Content/" + game_path.substr(6)
	if game_path.begins_with("/Engine/"):
		return "Engine/Content/" + game_path.substr(8)
	if _plugin_roots.is_empty():
		for f in UePakVfs.list():
			var c := f.find("/Content/")
			if c > 0 and f.find("/Plugins/") >= 0:
				var root := f.substr(0, c)
				_plugin_roots[root.get_file().to_lower()] = root + "/Content/"
	var seg := game_path.trim_prefix("/").get_slice("/", 0).to_lower()
	if _plugin_roots.has(seg):
		return _plugin_roots[seg] + game_path.trim_prefix("/").substr(seg.length() + 1)
	return ""

static func node_name(n) -> String:
	match n[0]:
		1: return n[1].exports[n[2]].name
		2: return n[1].imports[n[2]].name
	return n[1].name

static func node_outer(n):
	match n[0]:
		1:
			var o = n[1].node(n[1].exports[n[2]].outer)
			return o if o != null else [0, n[1], 0]
		2:
			return n[1].node(n[1].imports[n[2]].outer)
	return null

static func node_class_name(n) -> String:
	match n[0]:
		1:
			var c = n[1].node(n[1].exports[n[2]].cls)
			return node_name(c) if c != null else ""
		2:
			return n[1].imports[n[2]]["class_name"]
	return ""

# ResolvedObject.GetPathName (AbstractUePackage.cs:181-202): outer chain joined by "." with ":" after a top-level object
static func path_name(n, incl_outermost: bool) -> String:
	var o = node_outer(n)
	var s := ""
	if o != null:
		var oo = node_outer(o)
		if oo != null or incl_outermost:
			s = path_name(o, incl_outermost) + (":" if oo != null and node_outer(oo) == null else ".")
	return s + node_name(n)

static func full_name(n, incl_outermost := false) -> String:
	return "%s'%s'" % [node_class_name(n), path_name(n, incl_outermost)]

# {ObjectName, ObjectPath} as CUE4Parse's ResolvedObjectConverter writes it (JsonConverters.cs:2874-2902)
static func obj_json(n):
	if n == null:
		return null
	var top = n
	while node_outer(top) != null:
		top = node_outer(top)
	var on := node_name(top)
	return {"ObjectName": full_name(n), "ObjectPath": "%s.%d" % [on, n[2]] if n[0] == 1 else on}

# ---- exports -------------------------------------------------------------------------------------------------------

func exports_json() -> Array:
	var out := []
	for i in exports.size():
		out.append(export_json(i))
	return out

# One export as mdx json writes it (UObject.WriteJson, UObject.cs:485-535): Type, Name, Class, Outer|Package, Super,
# Template, Properties; plus SuperStruct for UStruct exports and Rows for DataTables.
func export_json(i: int) -> Dictionary:
	var e: Dictionary = exports[i]
	var cn = node(e.cls)
	var d := {"Type": node_name(cn) if cn != null else "", "Name": e.name}
	if cn != null:
		if cn[0] == 1:
			d.Class = full_name(cn, true)
		elif cn[0] == 2 and cn[1].imports[cn[2]]["class_name"] in ["Class", "ScriptStruct"]:
			d.Class = "UScriptClass'%s'" % node_name(cn)
	if e.outer != 0:
		d.Outer = obj_json(node(e.outer))
	else:
		d.Package = name
	for k in [["Super", e.sup], ["Template", e.tmpl]]:
		var sn = node(k[1])
		if sn != null and sn[0] == 1:
			d[k[0]] = obj_json(sn)
	var r := UePakBuf.new(data())
	r.p = e.off
	var props := tagged(r, e.off + e.size)
	if not props.is_empty():
		d.Properties = props
	# UObject::Serialize: non-CDO objects then store bool bHasGuid (+ FGuid) (UObject.cs:226-236)
	if d.Type in STRUCT_TYPES or d.Type == "DataTable":
		if (e.flags & RF_CLASS_DEFAULT_OBJECT) == 0 and r.s32() != 0:
			r.skip(16)
		if d.Type == "DataTable":
			# UDataTable::LoadStructData: int32 NumRows, then FName RowName + the row struct's tagged properties
			# (UDataTable.cs:50-56)
			var rows := {}
			var n := _count(r, e.off + e.size)
			for j in n:
				var rn := fname(r)
				rows[rn] = tagged(r, e.off + e.size)
			d.Rows = rows
		else:
			# UField (no Next since FFrameworkObjectVersion::RemoveUField_Next, 4.25) then UStruct::SuperStruct (UStruct.cs:22)
			var sup = obj_json(node(r.s32()))
			if sup != null:
				d.SuperStruct = sup
	elif d.Type in ENUM_TYPES:
		# UObject Guid, UField (no Next, 4.25+), then UEnum::Serialize 4.26: TArray<TPair<FName, int64>> Names,
		# uint8 CppForm (UEnum.cs: Names via ReadArray(ReadFName, Read<long>), CppForm Read<ECppForm>)
		if (e.flags & RF_CLASS_DEFAULT_OBJECT) == 0 and r.s32() != 0:
			r.skip(16)
		var names_ := {}
		for j in _count(r, e.off + e.size):
			var nm := fname(r)
			names_[nm] = r.s64()
		d.Names = names_
		var form := r.u8()
		d.CppForm = CPP_FORM[form] if form < CPP_FORM.size() else str(form)
	elif d.Type == "Skeleton":
		# UObject Guid, then USkeleton::Serialize (USkeleton.cs:44-47): FReferenceSkeleton = TArray<FMeshBoneInfo
		# {FName Name; int32 ParentIndex}> FinalRefBoneInfo, TArray<FTransform> FinalRefBonePose (FQuat Rotation, FVector
		# Translation, FVector Scale3D), TMap<FName, int32> FinalNameToIndexMap (FReferenceSkeleton.cs:15-31,
		# FMeshBoneInfo.cs:17-30). The fields after it (AnimRetargetSources, Guid, NameMappings) are not read.
		if (e.flags & RF_CLASS_DEFAULT_OBJECT) == 0 and r.s32() != 0:
			r.skip(16)
		d.ReferenceSkeleton = ref_skeleton(r, e.off + e.size)
	if r.p > e.off + e.size:
		push_error("UeAsset: export %s read past its end in %s" % [e.name, name])
	return d

# FReferenceSkeleton (FReferenceSkeleton.cs:15-31, FMeshBoneInfo.cs:17-30): TArray<{FName Name; int32 ParentIndex}>
# FinalRefBoneInfo, TArray<FTransform {FQuat, FVector, FVector}> FinalRefBonePose, TMap<FName, int32>
# FinalNameToIndexMap; in a Skeleton export and in a SkeletalMesh's header
func ref_skeleton(r: UePakBuf, end: int) -> Dictionary:
	var info := []
	for j in _count(r, end):
		info.append({"Name": fname(r), "ParentIndex": r.s32()})
	var pose := []
	for j in _count(r, end):
		pose.append({"Rotation": _struct(r, "Quat", 16, 0, end), "Translation": _struct(r, "Vector", 12, 0, end),
			"Scale3D": _struct(r, "Vector", 12, 0, end)})
	var idx := {}
	for j in _count(r, end):
		var nm := fname(r)
		idx[nm] = r.s32()
	return {"FinalRefBoneInfo": info, "FinalRefBonePose": pose, "FinalNameToIndexMap": idx}

# ---- tagged properties (FPropertyTag, UE4 branch: FPropertyTag.cs:186-205; FPropertyTagData.cs:28-62) --------------

# Read tags until "None". Each value is decoded by type; a value this reader cannot decode is recorded in `unsupported`
# and skipped by the tag's Size, so one unknown struct never desyncs the rest (CUE4Parse does the same, FPropertyTag.cs:207-238).
# `end` bounds a nested struct (its tag Size, or the enclosing property's end): a tag that does not fit there means the
# bytes are not tagged properties (a native struct the tag did not name), reported as "__unsupported__".
func tagged(r: UePakBuf, end := -1) -> Dictionary:
	var out := {}
	var limit := r.b.size() if end < 0 else mini(end, r.b.size())
	while r.p + 8 <= limit:
		var pname := fname(r)
		if pname == "None":
			break
		var typ := fname(r)
		var size := r.s32()
		var aidx := r.s32()
		var td := {}
		match typ:
			"StructProperty":
				td.struct = fname(r)
				r.skip(16)					# StructGuid
			"BoolProperty":
				td.bool = r.u8()			# the value lives in the tag (PROPERTYTAG_BOOL_OPTIMIZATION)
			"ByteProperty", "EnumProperty":
				td.enum = fname(r)
			"ArrayProperty", "SetProperty":
				td.inner = fname(r)
			"MapProperty":
				td.inner = fname(r)
				td.value = fname(r)
		if r.u8() != 0:						# HasPropertyGuid
			r.skip(16)
		var start := r.p
		if r.bad or size < 0 or start + size > limit:
			r.bad = false
			out["__unsupported__"] = "no tag at %d" % start
			return out
		var key := pname if aidx == 0 else "%s[%d]" % [pname, aidx]
		var v = _value(r, typ, td, size, 0, start + size)
		if r.bad:
			r.bad = false
			v = {"__unsupported__": "read past the data"}
		if v is Dictionary and v.has("__unsupported__"):
			unsupported.append("%s %s: %s" % [typ, key, v.__unsupported__])
		elif r.p != start + size:
			unsupported.append("%s %s: read %d of %d bytes" % [typ, key, r.p - start, size])
			v = {"__unsupported__": "size"}
		out[key] = v
		r.p = start + size
	return out

# mode 0 = a tagged property, 1 = an array element, 2 = a set/map element (CUE4Parse ReadType NORMAL/ARRAY/MAP)
func _value(r: UePakBuf, typ: String, td: Dictionary, size: int, mode: int, end: int):
	match typ:
		"BoolProperty": return td.get("bool", 0) != 0 if mode == 0 else r.u8() != 0
		"IntProperty": return r.s32()
		"Int8Property": return r.s8()
		"Int16Property": return r.s16()
		"Int64Property": return r.s64()
		"UInt16Property": return r.u16()
		"UInt32Property": return r.u32()
		"UInt64Property": return r.u64()
		"FloatProperty": return r.f32()
		"DoubleProperty": return r.f64()
		"NameProperty", "EnumProperty": return fname(r)
		"StrProperty": return r.fstring()
		"TextProperty": return _text(r)
		"ObjectProperty", "ClassProperty", "WeakObjectProperty", "InterfaceProperty":
			return obj_json(node(r.s32()))
		"LazyObjectProperty":
			# TLazyObjectPtr is serialized as FUniqueObjectGuid (one FGuid), written {"Guid": ...} (LazyObjectProperty.cs:12-19,
			# FUniqueObjectGuid.cs:7-10, JsonConverters.cs:559-564); read before as a package index (4 of 16 bytes: "size")
			return {"Guid": _struct(r, "Guid", 16, mode, end)}
		"SoftObjectProperty", "SoftClassProperty", "AssetObjectProperty", "AssetClassProperty":
			return _soft_path(r)
		"ByteProperty":
			if mode == 0:
				return r.u8() if td.get("enum", "None") == "None" else fname(r)
			# A map's byte key/value has no enum name in the tag; UE knows it from the FByteProperty. Without the class
			# schema, read an FName when the next 8 bytes are a valid name reference, as CUE4Parse does
			# (FPropertyTagType.cs:155, FAssetArchive.TestReadFName FAssetArchive.cs:61-70)
			if mode == 2 and r.p + 8 < end:
				var ni := r.b.decode_s32(r.p)
				var nn := r.b.decode_s32(r.p + 4)
				if ni >= 0 and ni < names.size() and nn >= 0 and nn < 256:
					return fname(r)
			return r.u8()
		"StructProperty":
			return _struct(r, td.get("struct", ""), size, mode, end)
		"ArrayProperty":
			var n := r.s32()
			if n < 0 or n > end - r.p:
				return {"__unsupported__": "array count %d" % n}
			var inner: String = td.inner
			var itd := {}
			if inner == "StructProperty":
				# INNER_ARRAY_TAG_INFO: struct arrays carry one inner FPropertyTag (UScriptArray.cs:51-56)
				fname(r); fname(r); r.skip(8)
				itd.struct = fname(r)
				r.skip(16)
				if r.u8() != 0:
					r.skip(16)
			var a := []
			if inner == "ByteProperty" and n > 0:
				# enum arrays store FNames: element size > 1 byte (UScriptArray.cs:72-84)
				var as_name := size - 4 >= 2 * n
				for i in n:
					a.append(fname(r) if as_name else r.u8())
				return a
			for i in n:
				var v = _value(r, inner, itd, 0, 1, end)
				if v is Dictionary and v.has("__unsupported__"):
					return v
				a.append(v)
			return a
		"SetProperty":
			for i in _count(r, end):		# NumElementsToRemove
				_value(r, td.inner, {}, 0, 2, end)
			var a := []
			for i in _count(r, end):
				var v = _value(r, td.inner, {}, 0, 2, end)
				if r.bad or (v is Dictionary and v.has("__unsupported__")):
					return {"__unsupported__": "set element"}
				a.append(v)
			return a
		"MapProperty":
			# NumKeysToRemove (+ keys), NumEntries, then key/value pairs (UScriptMap.cs:54-82). Struct keys/values carry no
			# struct name in UE4 tags; they are read as tagged properties (UScriptStruct::SerializeItem default)
			for i in _count(r, end):
				_value(r, td.inner, {}, 0, 2, end)
			var a := []
			for i in _count(r, end):
				var k = _map_key(r, td.inner, end)
				var v = _value(r, td.value, {}, 0, 2, end)
				if r.bad or (k is Dictionary and k.has("__unsupported__")) or (v is Dictionary and v.has("__unsupported__")):
					return {"__unsupported__": "map entry"}
				a.append({"Key": k, "Value": v})
			return a
		"MulticastDelegateProperty", "MulticastInlineDelegateProperty", "MulticastSparseDelegateProperty":
			var inv := []
			for i in _count(r, end):
				inv.append({"Object": obj_json(node(r.s32())), "FunctionName": fname(r)})
			return {"InvocationList": inv}
		"FieldPathProperty":
			# FFieldPath: TArray<FName> Path (["None"] = empty) + FPackageIndex ResolvedOwner (FFieldPath.cs:18-30;
			# UE FieldPath.h, owner serialized since FReleaseObjectVersion::FFieldPathOwnerSerialization)
			var path := []
			for i in _count(r, end):
				path.append(fname(r))
			if path == ["None"]:
				path = []
			return {"Path": path, "ResolvedOwner": obj_json(node(r.s32()))}
		"DelegateProperty":
			return {"Object": obj_json(node(r.s32())), "FunctionName": fname(r)}
	return {"__unsupported__": "type " + typ}

# An element count that cannot be right (negative, or more elements than bytes left) reads as 0 and marks the buffer bad
func _count(r: UePakBuf, end: int) -> int:
	var n := r.s32()
	if n < 0 or n > end - r.p:
		r.bad = true
		return 0
	return n

# A map key that is not a struct is written by CUE4Parse as the key's ToString() up to the first "(" (JsonConverters.cs:
# 1058-1088): an object as GetFullName() including its package ("Class'Pkg.Obj'"), "0" for null.
func _map_key(r: UePakBuf, inner: String, end: int):
	if inner in ["ObjectProperty", "ClassProperty", "WeakObjectProperty", "InterfaceProperty"]:
		var n = node(r.s32())
		return "0" if n == null else full_name(n, true).get_slice("(", 0).strip_edges()
	var k = _value(r, inner, {}, 0, 2, end)
	if inner == "StructProperty" or (k is Dictionary and k.has("__unsupported__")):
		return k
	var s := ""
	if k is Dictionary and k.has("AssetPathName"):
		s = (k.AssetPathName if k.AssetPathName != "None" else "") if k.SubPathString == "" else "%s:%s" % [k.AssetPathName, k.SubPathString]
	elif k is bool:
		s = "True" if k else "False"
	else:
		s = str(k)
	return s.get_slice("(", 0).strip_edges()

func _soft_path(r: UePakBuf) -> Dictionary:
	# FSoftObjectPath: FName AssetPathName + FString SubPathString (FSoftObjectPath.cs; UE SoftObjectPath.cpp SerializePath)
	var a := fname(r)
	return {"AssetPathName": a, "SubPathString": r.fstring()}

func _struct(r: UePakBuf, st: String, size: int, mode: int, end: int):
	match st:
		"Vector": return {"X": r.f32(), "Y": r.f32(), "Z": r.f32()}
		"Vector2D": return {"X": r.f32(), "Y": r.f32()}
		"Vector4": return {"X": r.f32(), "Y": r.f32(), "Z": r.f32(), "W": r.f32()}
		"Rotator": return {"Pitch": r.f32(), "Yaw": r.f32(), "Roll": r.f32()}
		"Quat": return {"X": r.f32(), "Y": r.f32(), "Z": r.f32(), "W": r.f32()}
		"LinearColor": return {"R": r.f32(), "G": r.f32(), "B": r.f32(), "A": r.f32()}
		"Color": return {"B": r.u8(), "G": r.u8(), "R": r.u8(), "A": r.u8()}		# FColor is stored BGRA
		"IntPoint": return {"X": r.s32(), "Y": r.s32()}
		"IntVector": return {"X": r.s32(), "Y": r.s32(), "Z": r.s32()}
		"Guid":
			# FGuid A,B,C,D (uint32); printed EGuidFormats::UniqueObjectGuid, as the JSON has it (JsonConverters.cs:2961-2966)
			return "%08X-%08X-%08X-%08X" % [r.u32(), r.u32(), r.u32(), r.u32()]
		"Box":
			return {"Min": _struct(r, "Vector", 12, mode, end), "Max": _struct(r, "Vector", 12, mode, end), "IsValid": r.u8()}
		"Box2D":
			return {"Min": _struct(r, "Vector2D", 8, mode, end), "Max": _struct(r, "Vector2D", 8, mode, end), "bIsValid": r.u8()}
		"FrameNumber": return {"Value": r.s32()}
		"MovieSceneFrameRange":
			# TRange<FFrameNumber>: LowerBound then UpperBound, each TRangeBound {uint8 Type (ERangeBoundTypes); FFrameNumber
			# Value} (FMovieSceneFrameRange.cs, TRangeBound.cs:24-36); 5 bytes per bound as UE serializes it, 8 when the
			# tag's size says the bound was written with its in-memory padding
			var pad := 3 if size == 16 else 0
			var lo := {"Type": r.u8()}
			r.skip(pad)
			lo.Value = {"Value": r.s32()}
			var hi := {"Type": r.u8()}
			r.skip(pad)
			hi.Value = {"Value": r.s32()}
			return {"Value": {"LowerBound": lo, "UpperBound": hi}}
		"FontData":
			# FFontData::Serialize, cooked (FFontData.cs:27-40): bool bIsCooked, then FPackageIndex LocalFontFaceAsset;
			# a null asset (font by file name) is not decoded; then int32 SubFaceIndex
			if r.s32() == 0:
				return {"__unsupported__": "uncooked FontData"}
			var face = obj_json(node(r.s32()))
			if face == null:
				return {"__unsupported__": "FontData by file name"}
			return {"LocalFontFaceAsset": face, "SubFaceIndex": r.s32()}
		"NavAgentSelector": return {"PackedBits": r.u32()}	# FNavAgentSelector: uint32 PackedBits (AI/Navigation/NavigationTypes.h)
		"DateTime", "Timespan": return {"Ticks": r.s64()}
		"GameplayTagContainer":
			var a := []
			for i in _count(r, end):
				a.append(fname(r))
			return a
		"RichCurveKey":
			var im := r.u8(); var tm := r.u8(); var tw := r.u8()
			return {"InterpMode": RCIM[im], "TangentMode": RCTM[tm], "TangentWeightMode": RCTWM[tw], "Time": r.f32(),
				"Value": r.f32(), "ArriveTangent": r.f32(), "ArriveTangentWeight": r.f32(), "LeaveTangent": r.f32(),
				"LeaveTangentWeight": r.f32()}
	if st in SOFT_PATH_STRUCTS:
		return _soft_path(r)
	if st in NET_QUANTIZE:
		# FVector_NetQuantize* derive from FVector, but these packages store them as tagged structs (Size != 12);
		# CUE4Parse reads 12 raw bytes regardless (FScriptStruct.cs:192-195) and so misreads them.
		if mode == 0 and size != 12:
			return tagged(r, end)
		return _struct(r, "Vector", 12, mode, end)
	if st in BINARY_STRUCTS:
		return {"__unsupported__": "native struct " + st}
	return tagged(r, end)

# FText (FText.cs:116-160; UE TextHistory.cpp): uint32 Flags, int8 HistoryType, then the history
func _text(r: UePakBuf):
	r.skip(4)
	var ht := r.s8()
	match ht:
		-1:		# None: bHasCultureInvariantString + string (FText.cs:188-198)
			return {"CultureInvariantString": r.fstring() if r.s32() != 0 else null}
		0:		# Base: Namespace, Key, SourceString (FText.cs:208-213). LocalizedString = SourceString: no .locres is
				# applied (mdx json does not load one either)
			var ns := r.fstring()
			var key := r.fstring()
			var src := r.fstring()
			return {"Namespace": ns, "Key": key, "SourceString": src, "LocalizedString": src}
		11:		# StringTableEntry: FName TableId, FString Key
			var tid := fname(r)
			return {"TableId": tid, "Key": r.fstring()}
	return {"__unsupported__": "text history %d" % ht}

# ue_wearable.gd - reader for Mordhau's character customization data (mdx json):
#   wearable Blueprints (UMordhauWearable and its slot subclasses) -> WearableData,
#   the customization catalogue on BP_MordhauSingleton (HeadWearables / UpperChestWearables / LegsWearables, Equipment,
#   Perks, Archetypes, ColorTables, faces, DefaultProfiles, BotCharacterProfiles),
#   FCharacterProfile structs (as mdx json writes them) -> Loadout.
# Every value comes from extract/json (UePkg.defaults, child wins) over the native constructor (NativeCtor.defaults).
# The slot -> class lookup is FCharacterGearCustomization::GetWearableClass rva=0x1547860 (port in class_for()).
class_name UeWearable

const SINGLETON := "Mordhau/Content/Mordhau/Blueprints/BP_MordhauSingleton"
const WEARABLES_DIR := "Mordhau/Content/Mordhau/Blueprints/Wearables"

# EWearableSlot (extract/native/types/EWearableSlot.h, MordhauCustomizationTypes.h:38)
enum Slot { HEAD, COIF, UPPER_CHEST, LOWER_CHEST, SHOULDERS, ARMS, HANDS, LEGS, FEET, TOTAL, INVALID }
const SLOT_NAMES := ["Head", "Coif", "UpperChest", "LowerChest", "Shoulders", "Arms", "Hands", "Legs", "Feet"]

# Native class of each Blueprint root (the Blueprint's SuperStruct chain ends at /Script/Mordhau.<Class>) -> the PDB
# class whose constructor holds the native defaults.
const NATIVE_TYPES := {"MordhauWearable": "UMordhauWearable", "HeadWearable": "UHeadWearable",
	"UpperChestWearable": "UUpperChestWearable", "ArmsWearable": "UArmsWearable", "LegsWearable": "ULegsWearable"}

static var _wearables := {}		# class path -> WearableData
static var _single := {}

# "/Game/Mordhau/Blueprints/X/BP_Y.BP_Y_C" (FSoftObjectPath AssetPathName) -> "Mordhau/Content/Mordhau/Blueprints/X/BP_Y"
# (/Game is the project's Content folder; the pak path prefix is "<Project>/Content", manifest.tsv). Also takes
# ObjectPath dicts ("Mordhau/Content/.../BP_Y.0") and {"AssetPathName": ...} dicts. "" for None.
static func pkg_path(ref) -> String:
	if ref is Dictionary:
		ref = ref.get("AssetPathName", ref.get("ObjectPath", ""))
	var s := String(ref) if ref != null else ""
	if s == "" or s == "None":
		return ""
	if s.begins_with("/Game/"):
		return "Mordhau/Content/" + s.substr(6).get_slice(".", 0)
	if s.begins_with("/"):			# other mount points are not used by wearables
		return s.substr(1).get_slice(".", 0)
	return UePkg.strip(s)

# BP_MordhauSingleton CDO over UMordhauSingleton::UMordhauSingleton rva=0x15b7510 (DefaultHead is only native: 0).
static func singleton() -> Dictionary:
	if _single.is_empty():
		_single = NativeCtor.defaults("UMordhauSingleton").duplicate(true)
		var d := UePkg.defaults(SINGLETON)
		for k in d:
			_single[k] = d[k]
	return _single

# Merged class defaults of any Blueprint: its native root's constructor values, then the CDO chain.
static func class_defaults(path: String) -> Dictionary:
	var out := {}
	var nat := UePkg.native_root(path)
	var t: String = NATIVE_TYPES.get(nat, "U" + nat)
	if NativeCtor.has_type(t):
		out = NativeCtor.defaults(t).duplicate(true)
	elif NativeCtor.has_type("A" + nat):
		out = NativeCtor.defaults("A" + nat).duplicate(true)
	var d := UePkg.defaults(path)
	for k in d:
		out[k] = d[k]
	return out

static func load_wearable(path: String) -> WearableData:
	path = pkg_path(path)
	if path == "":
		return null
	if _wearables.has(path):
		return _wearables[path]
	var nat := UePkg.native_root(path)
	if not NATIVE_TYPES.has(nat):
		push_error("UeWearable: %s is not a wearable (native root '%s')" % [path, nat])
		return null
	var d := class_defaults(path)
	var w := WearableData.new()
	w.id = path.get_file()
	w.class_path = path
	w.native_class = nat
	w.display_name = UePkg.text(d.get("ItemName"))
	UePkg.assign(w, WearableData.MAP, d)
	w.mesh = pkg_path(d.get("Mesh"))
	w.mesh_1p = pkg_path(d.get("Mesh1POverride"))
	w.aux_mesh = pkg_path(d.get("AuxiliaryMesh"))
	w.aux_mesh_1p = pkg_path(d.get("AuxiliaryMesh1POverride"))
	var fr := UeRec.new(d, path)
	# UseColorsFromSlot: EWearableSlot, the ctor byte (10 = Invalid) or a Blueprint's "EWearableSlot::Name"
	var uc = d.get("UseColorsFromSlot", 10)
	if uc is String:
		var nm := String(uc).get_slice("::", 1)
		w.use_colors_from_slot = SLOT_NAMES.find(nm) if SLOT_NAMES.has(nm) else (Slot.INVALID if nm == "Invalid" else -1)
		if w.use_colors_from_slot < 0:
			fr.errors.append("%s.UseColorsFromSlot: unknown EWearableSlot %s" % [path, uc])
	else:
		w.use_colors_from_slot = fr.i("UseColorsFromSlot")
	for f in WearableData.HIDE:
		if fr.b(f):
			w.hide[f] = true
	for f in WearableData.REQUIRES:
		if fr.b(f):
			w.requires_aux[f] = true
	w.flags = WearableData.Flags.from_sets(w.hide, w.requires_aux)
	fr.done(true)
	w.albedo_map = pkg_path(d.get("AlbedoMap"))
	w.normal_map = pkg_path(d.get("NormalMap"))
	w.roughness_map = pkg_path(d.get("RoughnessMap"))
	var a = d.get("Albedo")
	if a is Dictionary and a.has("R"):
		w.albedo = Color8(int(a.R), int(a.G), int(a.B), int(a.get("A", 255)))
	for p in d.get("Patterns", []):
		w.patterns.append(pkg_path(p.get("Texture")) if p is Dictionary else "")
	# Not in the json = the native default: UMordhauWearable ctor (rva=0x1612dc0) appends two ColorTables entries of 0
	# (and two Colors of 0), so e.g. BP_MailOverGambeson (no ColorTables property) colours from table 0 twice.
	for c in d.get("ColorTables", [0, 0]):
		w.color_tables.append(int(c))
	for k in WearableData.CHILD_LISTS:
		if d.has(k):
			var arr := []
			for r in d[k]:
				arr.append(pkg_path(r))
			w.children[k] = arr
			w.default_child[WearableData.CHILD_LISTS[k]] = int(d.get(WearableData.CHILD_LISTS[k], 0))
	_wearables[path] = w
	return w

static func clear_cache() -> void:
	_wearables.clear()
	_single.clear()

# Element `i` of a soft class array, "" when out of range: UMordhauSingleton::GetHeadWearable rva=0x15d6c60 and the
# ICF-folded UHeadWearable::GetCoifWearable / UUpperChestWearable::GetLowerChestWearable / UArmsWearable::GetHandsWearable
# / ULegsWearable::GetFeetWearable rva=0x14841f0 all return null unless 0 <= Index < ArrayNum.
static func _at(arr, i: int) -> String:
	if not (arr is Array) or i < 0 or i >= arr.size():
		return ""
	return pkg_path(arr[i])

# FCharacterGearCustomization::GetWearableClass(Slot) rva=0x1547860, ported. ids = FWearableCustomization.Id per slot
# (Wearables[slot], 9 entries). Coif comes from the head's CoifWearables, LowerChest/Shoulders/Arms from the upper
# chest's lists, Hands from the arms' HandsWearables (arms picked by Wearables[Arms].Id inside the upper chest), Feet from
# the legs' FeetWearables. Head / UpperChest / Legs / Arms results must be of their native class (the IsA checks).
static func class_for(ids: PackedInt32Array, slot: int) -> String:
	if slot < 0 or slot > Slot.FEET or ids.size() <= slot:
		return ""
	var s := singleton()
	var parent := slot
	if slot == Slot.COIF:
		parent = Slot.HEAD
	elif slot >= Slot.LOWER_CHEST and slot <= Slot.ARMS:
		parent = Slot.UPPER_CHEST
	elif slot == Slot.HANDS:
		parent = Slot.ARMS
	elif slot == Slot.FEET:
		parent = Slot.LEGS
	var own := ids[slot]
	var pid := ids[parent]
	match parent:
		Slot.UPPER_CHEST:
			var uc := load_wearable(_at(s.get("UpperChestWearables"), pid))
			if uc == null:
				return ""
			if slot == Slot.UPPER_CHEST:
				var c := _at(s.get("UpperChestWearables"), own)
				return c if UePkg.native_root(c) == "UpperChestWearable" else ""
			if slot == Slot.SHOULDERS:
				return _at(uc.children.get("ShouldersWearables"), own)
			if slot == Slot.ARMS:
				var c := _at(uc.children.get("ArmsWearables"), own)
				return c if UePkg.native_root(c) == "ArmsWearable" else ""
			return _at(uc.children.get("LowerChestWearables"), own)
		Slot.ARMS:	# Hands: upper chest from Wearables[UpperChest] (pFVar1 + 0x80 = element 2), arms = its ArmsWearables[Index]
			var uc := load_wearable(_at(s.get("UpperChestWearables"), ids[Slot.UPPER_CHEST]))
			if uc == null:
				return ""
			var arms := load_wearable(_at(uc.children.get("ArmsWearables"), pid))
			return _at(arms.children.get("HandsWearables"), own) if arms != null else ""
		Slot.HEAD:
			var hd := load_wearable(_at(s.get("HeadWearables"), pid))
			if hd == null:
				return ""
			if slot == Slot.HEAD:
				var c := _at(s.get("HeadWearables"), own)
				return c if UePkg.native_root(c) == "HeadWearable" else ""
			return _at(hd.children.get("CoifWearables"), own)
		Slot.LEGS:
			var lg := load_wearable(_at(s.get("LegsWearables"), pid))
			if lg == null:
				return ""
			if slot == Slot.LEGS:
				var c := _at(s.get("LegsWearables"), own)
				return c if UePkg.native_root(c) == "LegsWearable" else ""
			return _at(lg.children.get("FeetWearables"), own)
	return ""

# Fill l.wearables: AMordhauCharacter::BuildCharacter rva=0x15319e0 loops slot 0..8, GetWearableClass(slot) and
# NewObject<UMordhauWearable> of it into WearableObjectInstances[slot]. (Its other branch, UHumanMeshComponent
# SingleSlotMode +0x11c0 / SingleSlotModeWearableToUse +0x11c8, is the one-slot preview mesh, not a playing character.)
static func resolve(l: Loadout) -> Loadout:
	l.wearables = []
	for slot in Loadout.SLOTS:
		var c := class_for(l.ids, slot)
		l.wearables.append(load_wearable(c) if c != "" else null)
	return l

static func loadout(profile_name: String, field := "DefaultProfiles") -> Loadout:
	var p := profile(profile_name, field)
	return resolve(Loadout.from_profile(p)) if p != null else null

# UMordhauSingleton::GetEquipmentDefaultObject(Id)->CharacterPointCost [+0x6c0]; AMordhauEquipment ctor stores 1.
static func equipment_path(id: int) -> String:
	return _at(singleton().get("Equipment"), id)

static func equipment_point_cost(id: int) -> int:
	var p := equipment_path(id)
	if p == "":
		return 0
	return int(class_defaults(p).get("CharacterPointCost", 0))

# Singleton.Perks[bit] (FSkillsCustomization::GetPerksCost rva=0x1545790 indexes Perks by bit number) -> {path, cost,
# enum, name}. UPerk ctor rva=0x16488f0 leaves Cost 0.
static func perks() -> Array:
	var out := []
	for r in singleton().get("Perks", []):
		var p := pkg_path(r)
		var d := class_defaults(p)
		out.append({"path": p, "cost": int(d.get("Cost", 0)), "enum": str(d.get("Enum", "")),
			"name": UePkg.text(d.get("Name"))})
	return out

# FSkillsCustomization::GetArchetypeObject -> UArchetype.CharacterPoints [+0x28]. The singleton lists one archetype
# (BP_Archetype, CharacterPoints 48); the profile's archetype index is not in the mdx json of FSkillsCustomization
# (only Perks is written), so index 0 is used.
static func character_points(archetype := 0) -> int:
	var p := _at(singleton().get("Archetypes"), archetype)
	return int(class_defaults(p).get("CharacterPoints", 0)) if p != "" else 0

# FMordhauColorItemTable entry -> UMordhauColor.Color (FLinearColor [+0x60]). table = index in Singleton.ColorTables.
static func color(table: int, entry: int) -> Color:
	var t = singleton().get("ColorTables", [])
	if table < 0 or table >= t.size():
		return Color(1, 1, 1)
	var p := _at(t[table].get("Entries", []), entry)
	if p == "":
		return Color(1, 1, 1)
	var c = class_defaults(p).get("Color", {})
	return Color(float(c.get("R", 1)), float(c.get("G", 1)), float(c.get("B", 1)), float(c.get("A", 1))) if c is Dictionary else Color(1, 1, 1)

# Named FColorItemTable fields on the singleton (SkinColorTable, EyeColorTable, HairColorTable, MetalTintsColorTable).
static func named_color(table_field: String, entry: int) -> Color:
	var t = singleton().get(table_field, {})
	var p := _at(t.get("Entries", []) if t is Dictionary else [], entry)
	if p == "":
		return Color(1, 1, 1)
	var c = class_defaults(p).get("Color", {})
	return Color(float(c.get("R", 1)), float(c.get("G", 1)), float(c.get("B", 1)), float(c.get("A", 1))) if c is Dictionary else Color(1, 1, 1)

# Singleton.MaleFaces / FemaleFaces [Face] -> UCharacterFace defaults (Mesh, Torso, arm/leg/hand/foot parts, Hair[]...).
static func face(female: bool, index: int) -> Dictionary:
	var p := _at(singleton().get("FemaleFaces" if female else "MaleFaces"), index)
	return class_defaults(p) if p != "" else {}

# Profiles as mdx json writes FCharacterProfile: DefaultProfiles (the 9 presets: Knight, Footman, ...) or
# BotCharacterProfiles. Returns {name: profile dict}.
static func profiles(field := "DefaultProfiles") -> Dictionary:
	var out := {}
	for p in singleton().get(field, []):
		var n = p.get("Name", {})
		var nm := UePkg.text(n)
		if nm == "" and n is Dictionary:
			nm = String(n.get("CultureInvariantString", ""))
		out[nm] = p
	return out

# Every wearable Blueprint under Blueprints/Wearables, sorted: packages (UePkg.packages_of_class) whose first export
# class is BlueprintGeneratedClass (689) or Function (4: Blueprints whose first export is a function), filtered to
# native wearable roots. Abstract bases (BP_*Wearable, BP_Tier*Wearable) are included: they are classes too.
static func list_wearables() -> PackedStringArray:
	var out := PackedStringArray()
	for p in UePkg.packages_of_class(WEARABLES_DIR, ["BlueprintGeneratedClass", "Function"]):
		if NATIVE_TYPES.has(UePkg.native_root(p)):
			out.append(p)
	out.sort()
	return out

# glb of a mesh path under extract/gltf, "" when not exported yet.
static func glb(mesh_path: String) -> String:
	if mesh_path == "":
		return ""
	return "res://data/" + mesh_path + ".glb" if FileAccess.file_exists(UePkg.root() + "/gltf/" + mesh_path + ".glb") else ""

static func png(tex_path: String) -> String:
	if tex_path == "":
		return ""
	return "res://data/" + tex_path + ".png" if FileAccess.file_exists(UePkg.root() + "/gltf/" + tex_path + ".png") else ""

# ---- typed records -------------------------------------------------------------------------------------------------
# FCharacterProfile (BP_MordhauSingleton DefaultProfiles / BotCharacterProfiles elements). Array elements of a CDO
# carry every field (all 42 profiles, 378 FWearableCustomization, 126 FEquipmentCustomization serialize each field
# this reads: full scan of BP_MordhauSingleton.json), so every read is required. For reference the struct ctors are
# FWearableCustomization rva=0x1528610 (Id 0, Pattern 0, Colors / Team1Colors / Team2Colors = two 0 entries each)
# and FAppearanceCustomization rva=0x144c360.
class WearableCustomization:
	var id := 0
	var colors := PackedInt32Array()
	var team1_colors := PackedInt32Array()
	var team2_colors := PackedInt32Array()
	var pattern := 0

class Appearance:
	var b_is_female := false	# +0x1d
	var face := 0				# +0x22
	var hair := 0				# +0x25
	var facial_hair := 0		# +0x26
	var skin_color := 0			# +0x21
	var eye_color := 0			# +0x23
	var hair_color := 0			# +0x24

class Profile:
	var name := ""
	var wearables: Array[WearableCustomization] = []
	var equipment := PackedInt32Array()		# FEquipmentCustomization.Id per entry
	var perks := 0							# FSkillsCustomization.Perks
	var appearance := Appearance.new()

static func _bytes(r: UeRec, k: String) -> PackedInt32Array:
	var out := PackedInt32Array()
	for v in r.arr(k):
		if not (v is float or v is int):
			r.errors.append("%s.%s: expected byte array" % [r.what, k])
			break
		out.append(int(v))
	return out

static func profile_of(p: Dictionary) -> Profile:
	var r := UeRec.new(p, "FCharacterProfile")
	var out := Profile.new()
	out.name = r.text_or("Name")
	r.what = "FCharacterProfile(%s)" % out.name
	var gear := r.sub("GearCustomization")
	for w in gear.arr("Wearables"):
		var wr := UeRec.new(w, r.what + ".Wearables[]", r.errors)
		var wc := WearableCustomization.new()
		wc.id = wr.i("Id")
		wc.colors = _bytes(wr, "Colors")
		wc.team1_colors = _bytes(wr, "Team1Colors")
		wc.team2_colors = _bytes(wr, "Team2Colors")
		wc.pattern = wr.i("Pattern")
		out.wearables.append(wc)
	for e in gear.arr("Equipment"):
		out.equipment.append(UeRec.new(e, r.what + ".Equipment[]", r.errors).i("Id"))
	out.perks = r.sub("SkillsCustomization").i("Perks")
	var a := r.sub("AppearanceCustomization")
	out.appearance.b_is_female = a.b("bIsFemale")
	out.appearance.face = a.i("Face")
	out.appearance.hair = a.i("Hair")
	out.appearance.facial_hair = a.i("FacialHair")
	out.appearance.skin_color = a.i("SkinColor")
	out.appearance.eye_color = a.i("EyeColor")
	out.appearance.hair_color = a.i("HairColor")
	return r.done(out)

# UCharacterFace class defaults (native ctor + Blueprint chain, class_defaults): the body part meshes (soft object
# paths; absent = null) and the hair / facial hair class lists. Offsets per types/UCharacterFace.h.
class Face:
	var mesh := ""
	var torso := ""
	var eyes := ""
	var left_arm := ""
	var right_arm := ""
	var left_hand := ""
	var right_hand := ""
	var left_leg := ""
	var right_leg := ""
	var left_foot := ""
	var right_foot := ""
	var fore_arm_aux := ""
	var full_arm_aux := ""
	var upper_chest_aux := ""
	var ankle_aux := ""
	var hair := PackedStringArray()			# class per Appearance.hair index
	var facial_hair := PackedStringArray()

# a soft / hard object reference field: absent or None = "" (null)
static func _soft(r: UeRec, k: String) -> String:
	if not r.has(k) or r.d[k] == null:
		return ""
	var v = r.d[k]
	if not (v is String or (v is Dictionary and (v.has("AssetPathName") or v.has("ObjectPath")))):
		r.errors.append("%s.%s: expected an object path" % [r.what, k])
		return ""
	return pkg_path(v)

static func face_def(female: bool, index: int) -> Face:
	var p := _at(singleton().get("FemaleFaces" if female else "MaleFaces"), index)
	if p == "":
		return null
	var r := UeRec.new(class_defaults(p), p)
	var f := Face.new()
	f.mesh = _soft(r, "Mesh")
	f.torso = _soft(r, "Torso")
	f.eyes = _soft(r, "Eyes")
	f.left_arm = _soft(r, "LeftArm")
	f.right_arm = _soft(r, "RightArm")
	f.left_hand = _soft(r, "LeftHand")
	f.right_hand = _soft(r, "RightHand")
	f.left_leg = _soft(r, "LeftLeg")
	f.right_leg = _soft(r, "RightLeg")
	f.left_foot = _soft(r, "LeftFoot")
	f.right_foot = _soft(r, "RightFoot")
	f.fore_arm_aux = _soft(r, "ForeArmAuxiliaryMesh")
	f.full_arm_aux = _soft(r, "FullArmAuxiliaryMesh")
	f.upper_chest_aux = _soft(r, "UpperChestAuxiliaryMesh")
	f.ankle_aux = _soft(r, "AnkleAuxiliaryMesh")
	for h in r.arr_or("Hair"):
		f.hair.append(pkg_path(h))
	for h in r.arr_or("FacialHair"):
		f.facial_hair.append(pkg_path(h))
	return r.done(f)

# a hair class's Mesh (class defaults), "" for none
static func hair_mesh(hair_class: String) -> String:
	if hair_class == "":
		return ""
	return _soft(UeRec.new(class_defaults(hair_class), hair_class), "Mesh")

static func profile(profile_name: String, field := "DefaultProfiles") -> Profile:
	var p = profiles(field).get(profile_name)
	if p == null:
		push_error("UeWearable: no profile '%s' in %s" % [profile_name, field])
		return null
	return profile_of(p)

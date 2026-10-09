# customization_data.gd - the typed reads the loadout / customization screen's validation needs
# (game/ui/customization_model.gd ports FCharacterProfile::Validate rva=0x1577070 and FCharacterGearCustomization::
# Validate rva=0x15764c0 over these). Every value is BP_MordhauSingleton data (UeWearable.singleton / class_defaults:
# the CDO chain over the native constructor); game/** sees only the typed fields below.
class_name CustomizationData

# One AMordhauEquipment class default object (field offsets per extract/native/types/AMordhauEquipment.h).
class EquipmentRules:
	var id := 0
	var path := ""					# Blueprint package
	var cost := 0					# CharacterPointCost [+0x6c0] (AMordhauEquipment ctor stores 1)
	var max_per_loadout := 0		# MaxAmountPerLoadout [+0x660]; 0 = no limit (the ctor leaves it 0)
	var only_peasants := false		# bOnlyPeasants [+0x68d]
	var allowed_for_peasants := false	# bIsAllowedForPeasants [+0x68e]

static var _eq := {}

# UMordhauSingleton::GetEquipmentNum rva=0x15d6830: Equipment.ArrayNum.
static func equipment_count() -> int:
	var a = UeWearable.singleton().get("Equipment", [])
	return a.size() if a is Array else 0

# UMordhauSingleton::GetEquipmentDefaultObject rva=0x15d66c0: GetEquipment(Index) (null out of range), IsA
# AMordhauEquipment, its class default object. null when there is none.
static func equipment(id: int) -> EquipmentRules:
	if _eq.has(id):
		return _eq[id]
	var p := UeWearable.equipment_path(id)
	var r: EquipmentRules = null
	if p != "":
		var d := UeWearable.class_defaults(p)
		r = EquipmentRules.new()
		r.id = id
		r.path = p
		r.cost = int(d.get("CharacterPointCost", 0))
		r.max_per_loadout = int(d.get("MaxAmountPerLoadout", 0))
		r.only_peasants = bool(d.get("bOnlyPeasants", false))
		r.allowed_for_peasants = bool(d.get("bIsAllowedForPeasants", false))
	_eq[id] = r
	return r

# The DefaultValid out parameter of FCharacterGearCustomization::GetWearableClass rva=0x1547860 for `slot` given the
# profile's Wearables[].Id: Head / UpperChest / Legs -> the singleton's DefaultHead / DefaultUpperChest / DefaultLegs
# (+0x488 / +0x4b0 / +0x4d8); Coif -> the head's DefaultCoif, LowerChest / Arms / Shoulders -> the upper chest's
# DefaultLowerChest / DefaultArms / DefaultShoulders, Hands -> the arms' DefaultHands, Feet -> the legs' DefaultFeet
# (each +0x228 / +0x240 / +0x258 on its wearable class, extract/native/types/U*Wearable.h). 0 when the parent is null
# (the out byte keeps the caller's 0).
static func default_id(ids: PackedInt32Array, slot: int) -> int:
	var s := UeWearable.singleton()
	match slot:
		UeWearable.Slot.HEAD:
			return int(s.get("DefaultHead", 0))
		UeWearable.Slot.UPPER_CHEST:
			return int(s.get("DefaultUpperChest", 0))
		UeWearable.Slot.LEGS:
			return int(s.get("DefaultLegs", 0))
	var parent := {UeWearable.Slot.COIF: UeWearable.Slot.HEAD, UeWearable.Slot.LOWER_CHEST: UeWearable.Slot.UPPER_CHEST,
		UeWearable.Slot.SHOULDERS: UeWearable.Slot.UPPER_CHEST, UeWearable.Slot.ARMS: UeWearable.Slot.UPPER_CHEST,
		UeWearable.Slot.HANDS: UeWearable.Slot.ARMS, UeWearable.Slot.FEET: UeWearable.Slot.LEGS}
	if not parent.has(slot):
		return 0
	var pc := UeWearable.class_for(ids, parent[slot])
	var pw := UeWearable.load_wearable(pc) if pc != "" else null
	if pw == null:
		return 0
	var key := {UeWearable.Slot.COIF: "DefaultCoif", UeWearable.Slot.LOWER_CHEST: "DefaultLowerChest",
		UeWearable.Slot.SHOULDERS: "DefaultShoulders", UeWearable.Slot.ARMS: "DefaultArms",
		UeWearable.Slot.HANDS: "DefaultHands", UeWearable.Slot.FEET: "DefaultFeet"}
	return int(pw.default_child.get(key[slot], 0))

# FSkillsCustomization::GetPerksCost rva=0x1545790 reads Singleton.Perks[n]->Cost: the cost of each perk index.
static func perk_costs() -> Array:
	var out := []
	for p in UeWearable.perks():
		out.append(int(p["cost"]))
	return out

# FSkillsCustomization::GetArchetypeObject rva=0x153eda0 -> UArchetype.CharacterPoints (UeWearable.character_points).
static func character_points(archetype := 0) -> int:
	return UeWearable.character_points(archetype)

static func clear_cache() -> void:
	_eq.clear()

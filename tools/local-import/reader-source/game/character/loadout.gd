# loadout.gd - one character's customization (FCharacterProfile, MordhauCustomizationTypes.h:741) as plain data:
#   gear: 9 FWearableCustomization (Id, Colors, Team1Colors, Team2Colors, Pattern) indexed by EWearableSlot, plus
#         FEquipmentCustomization Ids; skills: FSkillsCustomization.Perks bit mask; appearance: FAppearanceCustomization.
#   wearables: the resolved classes per slot (WearableData or null), filled by UeWearable.resolve() with the port of
#   FCharacterGearCustomization::GetWearableClass. The rules in armor.gd read only this object.
# Engine-agnostic: RefCounted + packed arrays; the profile and wearables are typed records (UeWearable.Profile, WearableData).
class_name Loadout
extends RefCounted

const SLOTS := 9	# EWearableSlot::Total

var name := ""
var ids := PackedInt32Array([0, 0, 0, 0, 0, 0, 0, 0, 0])	# Wearables[slot].Id
var colors := []			# Wearables[slot].Colors (Array of PackedInt32Array): entry index per colour slot
var team_colors := [[], []]	# Team1Colors / Team2Colors per slot
var patterns := PackedInt32Array([0, 0, 0, 0, 0, 0, 0, 0, 0])
var equipment := PackedInt32Array()	# Equipment[].Id (UMordhauSingleton.Equipment index)
var perks := 0				# FSkillsCustomization.Perks (bit n = Singleton.Perks[n], EPerk n)
var appearance: UeWearable.Appearance	# FAppearanceCustomization (face, hair, bIsFemale, ...)
var wearables := []			# resolved: WearableData or null, per slot (size 9)

# FCharacterProfile (UeWearable.Profile, e.g. BP_MordhauSingleton DefaultProfiles[i]) -> Loadout (unresolved).
static func from_profile(p: UeWearable.Profile) -> Loadout:
	var l := Loadout.new()
	l.name = p.name
	l.colors = []
	l.team_colors = [[], []]
	for i in SLOTS:
		var w: UeWearable.WearableCustomization = p.wearables[i] if i < p.wearables.size() else UeWearable.WearableCustomization.new()
		l.ids[i] = w.id
		l.colors.append(w.colors)
		l.team_colors[0].append(w.team1_colors)
		l.team_colors[1].append(w.team2_colors)
		l.patterns[i] = w.pattern
	l.equipment = p.equipment
	l.perks = p.perks
	l.appearance = p.appearance
	return l

# AMordhauCharacter::HasPerk rva=0x1548120 forwards to its UPerkSystemComponent (+0x690; none -> false) ->
# UPerkSystemComponent::HasPerk rva=0x14c0920 (owner IsA AMordhauCharacter, else false) -> Skills (+0x11b8):
func has_perk(bit: int) -> bool:		# FSkillsCustomization::HasPerk rva=0x1548140: Perks & (1 << id), `shl eax, cl` (id & 31)
	return (perks & (1 << (bit & 31))) != 0

func wearable(slot: int) -> WearableData:
	return wearables[slot] if slot < wearables.size() else null

# wearable_data.gd - one Mordhau wearable (UMordhauWearable Blueprint: armour or clothing piece) as plain data.
# Built by UeWearable.load_wearable() from the native constructor defaults (NativeCtor, UMordhauWearable::UMordhauWearable
# rva=0x1612dc0 and the slot subclass ctors) merged with the Blueprint CDO chain (UePkg.defaults), child wins.
# Field offsets: extract/native/types/UMordhauWearable.h (+ UHeadWearable.h, UUpperChestWearable.h, UArmsWearable.h,
# ULegsWearable.h). Only the gameplay / construction fields are kept; blood, emblem and icon fields are not.
class_name WearableData
extends Resource

@export var id := ""					# Blueprint package name, e.g. "BP_Houndskull"
@export var class_path := ""			# "Mordhau/Content/Mordhau/Blueprints/Wearables/Head/Tier3/BP_Houndskull"
@export var native_class := ""			# "HeadWearable" / "UpperChestWearable" / "ArmsWearable" / "LegsWearable" / "MordhauWearable"
@export var display_name := ""			# ItemName (FText)

# rules (AMordhauCharacter::BuildCharacter, ComputePointsLeft, UpdateArmorSpeedAndAcceleration)
@export var armor_class := 0			# [0x1bc] uint8 ArmorClass
@export var point_cost := 0				# [0x1b8] int32 CharacterPointCost
@export var speed_factor := 0.0			# [0x1c0] SpeedFactor
@export var accel_factor := 0.0			# [0x1c4] AccelerationFactor
@export var b_is_allowed_for_peasants := false	# [0x1bd]
@export var b_muffle_voice := false		# [0x0ed]
@export var b_uses_reduced_body_poses := false	# [0x0f0]

# mesh construction (UHumanMeshComponent::SetupWearableConstruction_Internal rva=0x14d5fd0)
@export var mesh := ""					# [0x110] Mesh (UE object path, no extension)
@export var mesh_1p := ""				# [0x138] Mesh1POverride
@export var aux_mesh := ""				# [0x160] AuxiliaryMesh
@export var aux_mesh_1p := ""			# [0x188] AuxiliaryMesh1POverride
@export var b_treat_as_master := false	# [0x0ee]
@export var b_hide_in_1p := false		# [0x0ef]
@export var hide := {}					# bHide* flags [0x0f1..0x108] that are true: {"bHideChest": true, ...}
@export var requires_aux := {}			# bRequires*Auxiliary [0x1b0..0x1b3] that are true
var flags := Flags.new()				# the same two sets as typed fields (Armor.construction reads these)

# material (UHumanMeshComponent::UpdateSkeletalMeshComponentMaterials rva per game_functions.tsv)
@export var albedo_map := ""			# [0x068] UTexture2D (object path)
@export var normal_map := ""			# [0x070]
@export var roughness_map := ""			# [0x078] RMA texture
@export var albedo := Color(0, 0, 0, 0)	# [0x080] FColor Albedo (sRGB bytes / 255)
@export var metallic := 0.0				# [0x084]
@export var roughness := 0.8			# [0x088] ctor 0x3f4ccccd
@export var patterns := PackedStringArray()	# [0x1c8] Patterns[].Texture (colour masks)
@export var color_tables := PackedInt32Array()	# [0x1f0] ColorTables: index into UMordhauSingleton.ColorTables per colour slot
@export var b_ignore_team_color1 := false	# [0x200]
@export var b_ignore_team_color2 := false	# [0x201]
@export var b_uses_masked_material := false	# [0x224]
@export var use_colors_from_slot := 10		# [0x060] EWearableSlot (10 = Invalid, ctor value)

# child slot lists (indexes in FWearableCustomization.Id pick from these): FCharacterGearCustomization::GetWearableClass
@export var children := {}				# {"CoifWearables": [class paths], "LowerChestWearables": [...], ...}
@export var default_child := {}			# {"DefaultCoif": 0, ...}

# UE key -> field, for the scalar fields (UePkg.assign)
const MAP := {
	"ArmorClass": "armor_class", "CharacterPointCost": "point_cost", "SpeedFactor": "speed_factor",
	"AccelerationFactor": "accel_factor", "bIsAllowedForPeasants": "b_is_allowed_for_peasants",
	"bMuffleVoice": "b_muffle_voice", "bUsesReducedBodyPoses": "b_uses_reduced_body_poses",
	"bTreatAsMaster": "b_treat_as_master", "bHideIn1P": "b_hide_in_1p", "Metallic": "metallic",
	"Roughness": "roughness", "bIgnoreTeamColor1": "b_ignore_team_color1", "bIgnoreTeamColor2": "b_ignore_team_color2",
	"bUsesMaskedMaterial": "b_uses_masked_material",
}
const HIDE := ["bHideEars", "bHideHair", "bHideBeard", "bHideNose", "bHideLeftHand", "bHideRightHand", "bHideLeftFoot",
	"bHideRightFoot", "bHideLeftLeg", "bHideRightLeg", "bHideChest", "bHideLeftArm", "bHideRightArm"]
const REQUIRES := ["bRequiresFullArmAuxiliary", "bRequiresForearmAuxiliary", "bRequiresUpperChestAuxiliary",
	"bRequiresAnkleAuxiliary"]
const CHILD_LISTS := {"CoifWearables": "DefaultCoif", "LowerChestWearables": "DefaultLowerChest",
	"ArmsWearables": "DefaultArms", "ShouldersWearables": "DefaultShoulders", "HandsWearables": "DefaultHands",
	"FeetWearables": "DefaultFeet"}

func hides(flag: String) -> bool:
	return bool(hide.get(flag, false))

func requires(flag: String) -> bool:
	return bool(requires_aux.get(flag, false))

# bHide* / bRequires*Auxiliary as typed fields; merge() ORs another wearable's in (construction hides a part when any
# worn wearable hides it)
class Flags:
	var hide_ears := false
	var hide_hair := false
	var hide_beard := false
	var hide_nose := false
	var hide_left_hand := false
	var hide_right_hand := false
	var hide_left_foot := false
	var hide_right_foot := false
	var hide_left_leg := false
	var hide_right_leg := false
	var hide_chest := false
	var hide_left_arm := false
	var hide_right_arm := false
	var requires_full_arm_aux := false
	var requires_forearm_aux := false
	var requires_upper_chest_aux := false
	var requires_ankle_aux := false

	const _FIELDS := {"bHideEars": "hide_ears", "bHideHair": "hide_hair", "bHideBeard": "hide_beard",
		"bHideNose": "hide_nose", "bHideLeftHand": "hide_left_hand", "bHideRightHand": "hide_right_hand",
		"bHideLeftFoot": "hide_left_foot", "bHideRightFoot": "hide_right_foot", "bHideLeftLeg": "hide_left_leg",
		"bHideRightLeg": "hide_right_leg", "bHideChest": "hide_chest", "bHideLeftArm": "hide_left_arm",
		"bHideRightArm": "hide_right_arm", "bRequiresFullArmAuxiliary": "requires_full_arm_aux",
		"bRequiresForearmAuxiliary": "requires_forearm_aux", "bRequiresUpperChestAuxiliary": "requires_upper_chest_aux",
		"bRequiresAnkleAuxiliary": "requires_ankle_aux"}

	static func from_sets(hide_set: Dictionary, req_set: Dictionary) -> Flags:
		var f := Flags.new()
		for k in _FIELDS:
			if hide_set.has(k) or req_set.has(k):
				f.set(_FIELDS[k], true)
		return f

	func merge(o: Flags) -> void:
		for k in _FIELDS:
			if o.get(_FIELDS[k]):
				set(_FIELDS[k], true)

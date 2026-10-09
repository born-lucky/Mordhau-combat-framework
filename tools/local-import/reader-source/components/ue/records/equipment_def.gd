# equipment_def.gd - AMordhauEquipment (and the AMordhauWeapon / AMordhauShield fields beside WeaponData's combat
# numbers) as a typed record: what the hands, the grip, the animation layer and the mode switch read from an item.
# Source: CombatData.class_defaults(path) = NativeCtor (AMordhauEquipment::AMordhauEquipment rva=0x1526480,
# AMordhauWeapon::AMordhauWeapon rva=0x1610e30, AMordhauShield::AMordhauShield rva=0x15b61a0, zero-filled first) with
# the Blueprint CDO chain merged over it, so every native field is present and read as required (UeRec).
# Field offsets: extract/native/types/AMordhauEquipment.h, AMordhauWeapon.h, AMordhauShield.h.
class_name EquipmentDef
extends RefCounted

var path := ""						# Blueprint package
var native := ""					# native class the chain ends at, C++ name ("AMordhauShield", "AFistsWeapon", ...)
var is_weapon := false				# IsA AMordhauWeapon (CombatData.is_weapon_class)
var is_shield := false				# IsA AMordhauShield
var b_is_right_handed := true		# +0x569
var b_second_is_right_handed := true	# +0x56a
var b_is_two_handed := false		# +0x56b
var b_second_is_two_handed := false	# +0x56c
var b_can_holster := false			# +0xc70
var b_quickthrow_only := false		# +0xce2
var b_has_alternate_mode := false	# +0xd23
var right_hand_equip_offset := Vector3.ZERO	# +0xb4c FVector, UE cm
var rotation_offset := Vector3.ZERO			# +0xc20 FRotator (pitch, yaw, roll)
var grip_location_local := Vector3.ZERO		# +0xc38 FVector
var lower_animation := ""			# +0x840 UAnimSequence*
var upper_blend_space := ""			# +0x820 UBlendSpaceBase*
var kick_animation := ""			# +0x7c8
var kick_riposte_animation := ""	# +0x7d8
var kick_combo_animation := ""		# +0x7e8
# AMordhauWeapon only ("" / 0 otherwise)
var weapon_animation_profile := ""			# +0x1a60 TSubclassOf<UMeleeWeaponAnimationProfile>
var second_weapon_animation_profile := ""	# +0x1a68
var length := 0.0					# +0x1bec
var second_length := 0.0			# +0x1bf0
# AMordhauShield only
var b_allow_shield_wall := false	# +0x1c98

static var _cache := {}

static func load_def(bp_path: String) -> EquipmentDef:
	var p := UePkg.strip(bp_path)
	if not _cache.has(p):
		_cache[p] = from_ue(CombatData.class_defaults(p))
	return _cache[p]

static func from_ue(d: Dictionary) -> EquipmentDef:
	var r := UeRec.new(d, "EquipmentDef(%s)" % d.get("__path", "?"))
	var e := EquipmentDef.new()
	e.read(r)
	return r.done(e)

func read(r: UeRec) -> void:
	path = r.s("__path")
	native = r.s("__native")
	if not CombatData.is_class_of(native, "AMordhauEquipment"):
		r.errors.append("%s: %s is not an AMordhauEquipment" % [r.what, native])
		return
	is_weapon = CombatData.is_weapon_class(native)
	is_shield = CombatData.is_class_of(native, "AMordhauShield")
	b_is_right_handed = r.b("bIsRightHanded")
	b_second_is_right_handed = r.b("bSecondIsRightHanded")
	b_is_two_handed = r.b("bIsTwoHanded")
	b_second_is_two_handed = r.b("bSecondIsTwoHanded")
	b_can_holster = r.b("bCanHolster")
	b_quickthrow_only = r.b("bQuickthrowOnly")
	b_has_alternate_mode = r.b("bHasAlternateMode")
	right_hand_equip_offset = r.v3("RightHandEquipOffset")
	rotation_offset = r.rot("RotationOffset")
	grip_location_local = r.v3("GripLocationLocal")
	lower_animation = r.obj("LowerAnimation")
	upper_blend_space = r.obj("UpperBlendSpace")
	kick_animation = r.obj("KickAnimation")
	kick_riposte_animation = r.obj("KickRiposteAnimation")
	kick_combo_animation = r.obj("KickComboAnimation")
	if is_weapon:
		weapon_animation_profile = r.obj("WeaponAnimationProfileClass")
		second_weapon_animation_profile = r.obj("SecondWeaponAnimationProfileClass")
		length = r.f("Length")
		second_length = r.f("SecondLength")
	if is_shield:
		b_allow_shield_wall = r.b("bAllowShieldWall")

func is_a(cpp_class: String) -> bool:
	return CombatData.is_class_of(native, cpp_class)

# AMordhauWeapon::SwitchMode_Implementation rva=0x1640a00, the fields swapped here: Length <-> SecondLength,
# WeaponAnimationProfile <-> SecondWeaponAnimationProfile (motion_system.gd switch_mode)
func switched() -> EquipmentDef:
	var e: EquipmentDef = EquipmentDef.new()
	for p in get_property_list():
		if p.usage & PROPERTY_USAGE_SCRIPT_VARIABLE:
			e.set(p.name, get(p.name))
	e.length = second_length
	e.second_length = length
	e.weapon_animation_profile = second_weapon_animation_profile
	e.second_weapon_animation_profile = weapon_animation_profile
	return e

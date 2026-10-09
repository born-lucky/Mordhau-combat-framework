# weapon_data.gd - one Mordhau melee weapon (AMordhauWeapon / AMordhauShield / AFistsWeapon / AKickWeapon Blueprint)
# as a Godot Resource. Built by UeWeapon.load_weapon() from the merged CDO chain (UePkg.defaults).
# Fields = the gameplay keys found on the CDOs of the 279 Blueprints rooted at those native classes (mdx json, full scan).
# Values the chain never writes keep the native constructor's value. Defaults below marked [0x..] are the stores of
# AMordhauEquipment::AMordhauEquipment (rva 0x1526480) and AMordhauWeapon::AMordhauWeapon (rva 0x1610e30, runs after it),
# extract/native/decomp/*.cpp, mapped to fields by the PDB layouts in extract/native/types/AMordhauEquipment.h and
# AMordhauWeapon.h. Fields marked ZERO are not stored by either constructor, so they keep the
# allocation's zeroes: StaticAllocateObject (exe 0x141b88130) memsets every new object to 0 for the class's
# PropertiesSize before any constructor runs (`call memset` at 0x141b886e3, size = the same Class+0x58 value it passed
# to FUObjectAllocator::AllocateUObject at 0x141b8849e). Subclass
# constructors (AFistsWeapon, AKickWeapon) are applied by UeWeapon.NATIVE_OVERRIDES. UE names the Blueprint chain never
# writes are listed in `unset`.
class_name WeaponData
extends Resource

@export var id := ""						# Blueprint package name, e.g. "BP_Longsword"
@export var class_path := ""				# "Mordhau/Content/.../BP_Longsword"
@export var native_class := ""				# native parent the Blueprint chain ends at, e.g. "MordhauWeapon"
@export var chain := PackedStringArray()	# Blueprint ancestors, child first

# identity / loadout
@export var display_name := ""				# EquipmentName (FText, LocalizedString)
@export var point_cost := 1					# [0x6c0] equipment ctor
@export var equipment_ui_type := ""			# "EEquipmentType::..."
@export var equipment_ui_category := ""		# "EEquipmentCategory::..."
@export var b_only_peasants := false			# ZERO 0x68d (no constructor store)
@export var b_is_allowed_for_peasants := false	# ZERO 0x68e (no constructor store)

# attacks (FAttackInfo). The 9 NATIVE_ATTACKS are AMordhauWeapon members (types/AMordhauWeapon.h 0xf40..0x1880), and
# all 4 native roots derive from AMordhauWeapon (AMordhauShield.h, AVirtualWeapon.h headers), so an attack no CDO
# writes is still present with FAttackInfo ctor defaults. Base* are Blueprint-declared: null when absent.
@export var strike: AttackInfo
@export var second_strike: AttackInfo
@export var stab: AttackInfo
@export var second_stab: AttackInfo
@export var couch: AttackInfo
@export var second_couch: AttackInfo
@export var kick: AttackInfo
@export var second_kick: AttackInfo
@export var bash: AttackInfo
@export var base_strike: AttackInfo			# Horde "super" weapons keep an unmodified copy here
@export var base_second_strike: AttackInfo
@export var base_stab: AttackInfo

# block / parry
@export var block_stamina_negation := 7.0			# [0x1a38] 0x40e00000
@export var second_block_stamina_negation := 7.0	# [0x1a44] 0x40e00000
@export var block_stamina_clamp := Vector2(6, 20)		# [0x1a3c] 0x40c00000, [0x1a40] 0x41a00000
@export var second_block_stamina_clamp := Vector2(6, 20)	# [0x1a48], [0x1a4c]
@export var parry_turn_cap := Vector2(450, 450)		# [0x19f0] 0x43e10000, [0x19f4] 0x43e10000
@export var parry_backpedal_speed_factor := 1.0	# [0x1a08] 0x3f800000
@export var b_is_parry_held := false			# [0x1a0c] byte store 0
# ParryHeldStaminaDrain [0x1a10] and ParryWindowOffset [0x19b0]: written by NO native code. The AMordhauWeapon ctor
# disassembly (scripts/ue_dis.py on .text+0x160fe30, 1764 bytes) stores 0x1a08 (4 B), 0x1a0c (1 B), 0x1a14 (4 B) and
# 0x19a8/0x19ac but nothing at 0x1a10..0x1a13 or 0x19b0..0x19b3; its calls are AMordhauEquipment's ctor (sizeof 0xd30,
# cannot reach them), FAttackInfo's ctor x9 (members end at 0x19a8), and TSparseArray/TArray/TBitArray/FMemory::Free
# on stack locals. No decompile in extract/native/decomp writes either offset (UParryMotion.cpp:787-789 only reads
# them). So the native value is the allocation's initial memory: 0, IF UE zero-fills new UObjects
# (UE 4.26 StaticAllocateObject; engine source not in this repo, UNCONFIRMED here). Consistent with the data: the 5
# CDOs that write ParryHeldStaminaDrain store 1.0/2.0 and the 8 that write ParryWindowOffset store 0.05/0.1/0.3,
# never 0 (a delta never stores the archetype value).
@export var parry_held_stamina_drain := 0.0		# [0x1a10] not stored: zero-fill (see above)
@export var parry_window_offset := 0.0			# [0x19b0] not stored: zero-fill (see above)
@export var parry_mask := 5						# [0x19ac] (fists 4, kick 2: UeWeapon.NATIVE_OVERRIDES)
@export var shield_wall_turn_cap := Vector2(80, 80)		# [0x19f8] 0x42a00000, [0x19fc] 0x42a00000
@export var parry_success_turn_cap := Vector2(-1, -1)	# [0x1a00] 0xbf800000, [0x1a04] 0xbf800000
@export var b_can_block := true					# [0xef8] byte store 1
@export var slide_radius := 70.0					# [0xf2c] 0x428c0000
@export var block_movement_restriction := ""	# "EMovementRestriction::...". [0x1a34] AMordhauWeapon ctor byte store 0
											# (AMordhauWeapon.cpp:2059) = None; "" here means that value. Read by
											# UParryMotion::GetMovementRestriction rva=0x165cc30 (`movzx eax, byte [rax+0x1a34]`)

# handling
@export var stab_release_modifier := 1.0			# [0xf38] 0x3f800000
@export var attack_supersprint_duration := 0.35		# [0xf30] 0x3eb33333
@export var second_attack_supersprint_duration := 0.35	# [0xf34] 0x3eb33333
@export var equip_time_modifier := 1.0			# [0x570] equipment ctor 0x3f800000
@export var b_is_two_handed := false			# [0x56b] equipment ctor byte 0
@export var b_second_is_two_handed := false		# ZERO 0x56c (no constructor store)
@export var b_has_alternate_mode := false		# ZERO 0xd23 (no constructor store)
@export var b_can_couch_on_horseback := true		# [0xefb] u16 store 0x101 at 0xefa
@export var b_can_equip_on_horse := false		# ZERO 0xc71 (no constructor store)
@export var b_can_equip_on_ladder := false		# ZERO 0xc72 (no constructor store)
@export var b_can_attack_on_foot := true			# [0xcba] u16 store 0x101
@export var b_can_attack_on_horseback := true		# [0xcbb]
@export var b_can_block_on_foot := true			# [0xef9] byte store 1
@export var b_can_block_on_horseback := true		# [0xefa] u16 store 0x101
@export var b_allow_fire := false				# ZERO 0xcbc (no constructor store)
@export var b_fire_throws_equipment := true		# [0xcbd] byte store 1
@export var b_allow_drop := true				# [0xcb8] equipment ctor byte store 1 (AMordhauEquipment.cpp:1666); read by
											# UAttackMotion::CheckChamber rva=0x1617140 / UParryMotion::ReceiveBlock rva=0x166bf90
@export var movement_restriction := ""		# "EMovementRestriction::...". ctor stores 0 at [0x6f4] = EMovementRestriction::None (PDB LF_ENUM, combat_enums.gd); "" here means that value
# The next three are Blueprint-declared variables (absent from the native headers), only on the BPs that declare them
@export var damage_multiplier := 0.0
@export var attack_speed_modifier := 0.0
@export var max_combo_count := 0
@export var attack_mask := 1					# [0x19a8] (fists 4, kick 2: UeWeapon.NATIVE_OVERRIDES)
@export var b_can_attack := true				# [0xcb9] equipment field, weapon ctor byte 1 (shield ctor 0: NATIVE_OVERRIDES)
@export var strike_dismemberment := ""		# "EDismembermentType::...". ZERO 0xf28..0xf2b (no constructor store: enum value 0; the EDismembermentType name of 0 is not mapped here)
@export var second_strike_dismemberment := ""
@export var stab_dismemberment := ""
@export var second_stab_dismemberment := ""
@export var b_has_wooden_handle := false		# ZERO 0x1b3a (no constructor store)

# alt-mode throw
@export var ranged_draw_time := 0.5				# [0xcec] equipment ctor 0x3f000000
@export var ranged_release_time := 0.5			# [0xcf4] equipment ctor 0x3f000000
@export var ranged_cancel_time := 0.4			# [0xcf0] equipment ctor 0x3ecccccd
@export var ranged_reload_time := 0.6			# [0xd00] equipment ctor 0x3f19999a
@export var projectile_class := ""			# UE object path of the thrown projectile Blueprint
@export var kick_bounce := ""				# AMordhauEquipment KickBounce (UAnimMontage path, "" = none): a kick's BounceMontage

# look
@export var mesh := ""						# SkeletalMeshComponent.SkeletalMesh (UE object path), "" if parts-only
@export var mesh_glb := ""					# res://data/... .glb if that mesh was exported
@export var skins := []						# Skins[] as dictionaries (see UeWeapon._skin); typed view UeWeapon.skins(w)

# UE names never written by the chain (native value, unknown), and keys present but not mapped here
@export var unset := PackedStringArray()

const ATTACKS := {
	"StrikeAttack": "strike", "SecondStrikeAttack": "second_strike", "StabAttack": "stab",
	"SecondStabAttack": "second_stab", "CouchAttack": "couch", "SecondCouchAttack": "second_couch",
	"KickAttack": "kick", "SecondKickAttack": "second_kick", "BashAttack": "bash",
	"BaseStrikeAttack": "base_strike", "BaseSecondStrikeAttack": "base_second_strike", "BaseStabAttack": "base_stab",
}

const NATIVE_ATTACKS := ["StabAttack", "SecondStabAttack", "CouchAttack", "SecondCouchAttack", "StrikeAttack",
	"SecondStrikeAttack", "KickAttack", "SecondKickAttack", "BashAttack"]

# UE property name -> field, for the plain-valued fields
const MAP := {
	"EquipmentName": "display_name", "CharacterPointCost": "point_cost",
	"EquipmentUIType": "equipment_ui_type", "EquipmentUICategory": "equipment_ui_category",
	"bOnlyPeasants": "b_only_peasants", "bIsAllowedForPeasants": "b_is_allowed_for_peasants",
	"BlockStaminaNegation": "block_stamina_negation", "SecondBlockStaminaNegation": "second_block_stamina_negation",
	"BlockStaminaClamp": "block_stamina_clamp", "SecondBlockStaminaClamp": "second_block_stamina_clamp",
	"ParryTurnCap": "parry_turn_cap", "ParryBackpedalSpeedFactor": "parry_backpedal_speed_factor",
	"bIsParryHeld": "b_is_parry_held", "ParryHeldStaminaDrain": "parry_held_stamina_drain",
	"ParryWindowOffset": "parry_window_offset", "ParryMask": "parry_mask", "ShieldWallTurnCap": "shield_wall_turn_cap",
	"ParrySuccessTurnCap": "parry_success_turn_cap", "bCanBlock": "b_can_block", "SlideRadius": "slide_radius",
	"BlockMovementRestriction": "block_movement_restriction",
	"bCanAttack": "b_can_attack",
	"StabReleaseModifier": "stab_release_modifier", "AttackSupersprintDuration": "attack_supersprint_duration",
	"SecondAttackSupersprintDuration": "second_attack_supersprint_duration",
	"EquipTimeModifier": "equip_time_modifier", "bIsTwoHanded": "b_is_two_handed",
	"bSecondIsTwoHanded": "b_second_is_two_handed", "bHasAlternateMode": "b_has_alternate_mode",
	"bCanCouchOnHorseback": "b_can_couch_on_horseback", "bCanEquipOnHorse": "b_can_equip_on_horse",
	"bCanEquipOnLadder": "b_can_equip_on_ladder", "bCanAttackOnFoot": "b_can_attack_on_foot",
	"bCanAttackOnHorseback": "b_can_attack_on_horseback", "bCanBlockOnFoot": "b_can_block_on_foot",
	"bCanBlockOnHorseback": "b_can_block_on_horseback", "bAllowFire": "b_allow_fire",
	"bFireThrowsEquipment": "b_fire_throws_equipment", "bAllowDrop": "b_allow_drop", "MovementRestriction": "movement_restriction",
	"DamageMultiplier": "damage_multiplier", "AttackSpeedModifier": "attack_speed_modifier",
	"MaxComboCount": "max_combo_count", "AttackMask": "attack_mask",
	"StrikeDismembermentType": "strike_dismemberment", "SecondStrikeDismembermentType": "second_strike_dismemberment",
	"StabDismembermentType": "stab_dismemberment", "SecondStabDismembermentType": "second_stab_dismemberment",
	"bHasWoodenHandle": "b_has_wooden_handle",
	"RangedDrawTime": "ranged_draw_time", "RangedReleaseTime": "ranged_release_time",
	"RangedCancelTime": "ranged_cancel_time", "RangedReloadTime": "ranged_reload_time",
	"ProjectileClass": "projectile_class", "KickBounce": "kick_bounce",
}

func attack(ue_name: String) -> AttackInfo:
	return get(ATTACKS[ue_name]) if ATTACKS.has(ue_name) else null

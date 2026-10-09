# armor.gd - Mordhau's armour / loadout rules as pure functions over a resolved Loadout (loadout.gd).
# Engine-agnostic: no Node, no scene. Each function names the native function it ports.
#
#  coverage(l)     AMordhauCharacter::BuildCharacter rva=0x15319e0 fills WearableProtectionCoverageMap (+0xb18,
#                  TMap<FName, UMordhauWearable*>) after creating WearableObjectInstances[0..8]:
#                    Head wearable  (WearableObjectInstances.Data + 0x00, slot 0): head, Neck
#                    UpperChest     (Data + 0x10, slot 2): Hips, LowerBack, Spine, Spine1, LeftHand, LeftForearm, LeftArm,
#                                                         RightHand, RightForearm, RightArm
#                    Legs           (Data + 0x38, slot 7): LeftUpLeg, LeftLeg, LeftFoot, RightUpLeg, RightLeg, RightFoot
#                  (same order as the Emplace calls). No other slot (coif, shoulders, arms, gloves, lower chest, feet)
#                  protects any bone. The same bone groups are AMordhauCharacter::AppendHeadSet rva=0x15303b0,
#                  AppendBodySet 0x1530330, AppendLeftArmSet 0x1530400, AppendRightArmSet 0x1530500,
#                  AppendLeftLegSet 0x1530480, AppendRightLegSet 0x1530580 (body + both arm sets = the upper chest's).
#  tier_for_bone   AMordhauCharacter::GetArmorTierForBone rva=0x153ee30: map lookup by FName (case-insensitive), the
#                  wearable's ArmorClass (+0x1bc); 0 when the bone is absent or the wearable null.
#  footstep_tier   BuildCharacter: FootstepArmorTier (+0x768) = max(GetArmorTierForBone(Spine1), GetArmorTierForBone(RightLeg))
#                  (vtable +0x958 = GetArmorTierForBone, AMordhauCharacter.h slot 299).
#  speed_factors   UMordhauMovementComponent::UpdateArmorSpeedAndAcceleration rva=0x14dc0c0 (called from BuildCharacter).
#  points_left     UMordhauUtilityLibrary::ComputePointsLeft rva=0x16189a0 with FSkillsCustomization::GetPerksCost 0x1545790.
#
# Bone FNames: the globals _DAT_145721940.. are module FName constants; the string each is built from was paired by
# scanning .text for "lea rdx, <str>; lea rcx, <global>" (the dynamic initializers; MeleeDamage documents the same
# technique), and the string is read back from extract/native/rdata.tsv (column funcs = "dynamic initializer for
# 'NAME_<x>'") by CombatConstants.fname, so no bone name is typed in here:
#   _DAT_145721940 head      0x144317d48   _DAT_145721948 Neck        0x144317ac8
#   _DAT_145721950 Spine1    0x144317ad0   _DAT_145721958 Spine       0x144317ad8
#   _DAT_145721960 LowerBack 0x144317ae0   _DAT_145721968 Hips        0x144317aec
#   _DAT_145721978 RightArm  0x144317b08   _DAT_145721980 RightForearm 0x144317b18  _DAT_145721988 RightHand 0x144317b28
#   _DAT_1457219a0 LeftArm   0x144317b58   _DAT_1457219a8 LeftForearm 0x144317b60   _DAT_1457219b0 LeftHand  0x144317b70
#   _DAT_1457219c0 RightUpLeg 0x144317b90  _DAT_1457219c8 RightLeg    0x144317ba0   _DAT_1457219d0 RightFoot 0x144317bb0
#   _DAT_1457219d8 LeftUpLeg 0x144317bc0   _DAT_1457219e0 LeftLeg     0x144317bd0   _DAT_1457219e8 LeftFoot  0x144317bd8
class_name Armor

const SLOT_HEAD := 0
const SLOT_UPPER_CHEST := 2
const SLOT_LEGS := 7

# [slot, bone FNames in BuildCharacter's Emplace order] (the strings are read by CombatConstants from rdata.tsv)
static func _coverage_sets() -> Array:
	return [[SLOT_HEAD, CombatConstants.armor_head_bones], [SLOT_UPPER_CHEST, CombatConstants.armor_upper_chest_bones],
		[SLOT_LEGS, CombatConstants.armor_leg_bones]]

const PERK_TANK := 6	# EPerk::Tank (BP_TankPerk Enum; Singleton.Perks[6]); UpdateArmorSpeedAndAcceleration HasPerk(6)
const PERK_RAT := 7		# EPerk::Rat; HasPerk(7) -> MaxWalkSpeedCrouched = MaxWalkSpeedCrouchedWithRatPerk

# Bones covered by a slot's wearable (empty for slots that cover nothing).
static func bones_of_slot(slot: int) -> PackedStringArray:
	var out := PackedStringArray()
	for c in _coverage_sets():
		if c[0] == slot:
			out.append_array(c[1])
	return out

# WearableProtectionCoverageMap as {bone: ArmorClass}. Every bone of the three sets is present (BuildCharacter emplaces
# all 18 keys unconditionally); a null wearable reads as tier 0 (GetArmorTierForBone returns 0 for a null value).
static func coverage(l: Loadout) -> Dictionary:
	var out := {}
	for c in _coverage_sets():
		var w: WearableData = l.wearable(c[0])
		for bone in c[1]:
			out[bone] = w.armor_class if w != null else 0
	return out

# GetArmorTierForBone, with AAdvancedCharacter's DamageArmorTierOverride (+0x7f4, ctor 0xffffffff = -1) first, as
# ComputeMeleeDamage rva=0x1536fb0 reads it.
static func tier_for_bone(cov: Dictionary, bone: String, override := -1) -> int:
	if override != -1:
		return override
	for k in cov:
		if String(k).nocasecmp_to(bone) == 0:
			return int(cov[k])
	return 0

static func footstep_tier(cov: Dictionary) -> int:
	return maxi(tier_for_bone(cov, CombatConstants.footstep_spine1_bone),
		tier_for_bone(cov, CombatConstants.footstep_right_leg_bone))

# UpdateArmorSpeedAndAcceleration -> SpeedFactors (UMordhauMovementComponent ArmorSpeedFactor +0xd04,
# ArmorAccelerationFactor +0xd08). Both start at 1.0, then:
#   game state bOverrideArmorSpeedAndAccelerationFactor (+0x54d): 1 - OverrideArmorSpeedFactor (+0x550),
#                                                                 1 - OverrideArmorAccelerationFactor (+0x554)
#   else Tank perk (HasPerk 6): 1 - TankArmorSpeedFactor, 1 - TankArmorAccelerationFactor (UPerkSystemComponent +0x104/+0x108;
#                               ctor rva per UPerkSystemComponent.cpp: 0.33, -0.8)
#   else head (slot 0), upper chest (slot 2), legs (slot 7):
#        accel = 1 - sum(AccelerationFactor)
#        speed = 1 - (sum(SpeedFactor) - min(SpeedFactor))      (the smallest of the three is not charged)
# game_override: null, or the game state's Override* factors; tank: the UPerkSystemComponent defaults (null = no
# Tank values known).
class SpeedFactors:
	var speed := 1.0		# ArmorSpeedFactor, or OverrideArmorSpeedFactor / TankArmorSpeedFactor as an input
	var accel := 1.0		# ArmorAccelerationFactor, or the Override / Tank acceleration factor as an input
	func _init(s := 1.0, a := 1.0) -> void:
		speed = s
		accel = a

static func speed_factors(l: Loadout, tank: CharacterData.Perks = null, game_override: SpeedFactors = null) -> SpeedFactors:
	if game_override != null:
		return SpeedFactors.new(1.0 - game_override.speed, 1.0 - game_override.accel)
	if l.has_perk(PERK_TANK) and tank != null:
		return SpeedFactors.new(1.0 - tank.tank_armor_speed_factor, 1.0 - tank.tank_armor_acceleration_factor)
	var accel := 1.0
	var sf := [0.0, 0.0, 0.0]
	var slots := [SLOT_HEAD, SLOT_UPPER_CHEST, SLOT_LEGS]
	for i in 3:
		var w: WearableData = l.wearable(slots[i])
		if w != null:
			sf[i] = w.speed_factor
			accel -= w.accel_factor
	# fVar5 = chest; if head <= chest: head; if legs <= that: legs  -> min of the three. The game sums in float32
	# as (chest + head) + legs; GDScript floats are doubles (difference < 1e-7, below any test tolerance used).
	var mn: float = minf(minf(float(sf[0]), float(sf[1])), float(sf[2]))
	var speed: float = 1.0 - ((float(sf[1]) + float(sf[0]) + float(sf[2])) - mn)
	return SpeedFactors.new(speed, accel)

# FSkillsCustomization::GetPerksCost: for each set bit n of Perks, Singleton.Perks[n]->Cost (+0x40), bits out of range
# skipped. perk_costs: Array of int per Singleton.Perks index (UeWearable.perks() costs).
static func perks_cost(perks: int, perk_costs: Array) -> int:
	var c := 0
	for n in 32:
		if perks & (1 << n) and n < perk_costs.size():
			c += int(perk_costs[n])
	return c

# ComputePointsLeft = Archetype.CharacterPoints - perks cost - sum(equipment CharacterPointCost)
#                     - sum over the 9 slots of the wearable class's CharacterPointCost (+0x1b8).
# equipment_costs: CharacterPointCost of each Equipment Id (0 for an empty Id: GetEquipmentDefaultObject returns null).
static func points_left(l: Loadout, character_points: int, perk_costs: Array, equipment_costs: Array) -> int:
	var left := character_points - perks_cost(l.perks, perk_costs)
	for c in equipment_costs:
		left -= int(c)
	for slot in Loadout.SLOTS:
		var w: WearableData = l.wearable(slot)
		if w != null:
			left -= w.point_cost
	return left

static func wearables_cost(l: Loadout) -> int:
	var c := 0
	for slot in Loadout.SLOTS:
		var w: WearableData = l.wearable(slot)
		if w != null:
			c += w.point_cost
	return c

# --- mesh construction --------------------------------------------------------------------------------------------
# UHumanMeshComponent::SetupWearableConstruction_Internal rva=0x14d5fd0, third-person full body (bAddHead, bAddTorso,
# no ParamSet, bIs1PMesh false, bHasInvisibleBody false) -> ordered list of skeletal mesh paths to put on the master
# pose. face = UeWearable.face_def(appearance.b_is_female, appearance.face) (Singleton Male/FemaleFaces, offsets per
# UCharacterFace.h); hair / facial_hair = the Mesh of Face.Hair[appearance.hair] / FacialHair[appearance.facial_hair]
# (UCharacterHair +0x60 Mesh). The hide flags are OR-ed over all 9 slot wearables (WearableData.Flags):
#   Torso      [face +0x360] unless any bHideChest (+0x106)
#   face Mesh  [+0x060]
#   hair       unless bHideHair (+0xf8);   facial hair unless bHideBeard (+0xf9)
#   Eyes       [+0x158]
#   LeftArm [+0x180] / RightArm [+0x1d0] unless bHideLeftArm (+0x107) / bHideRightArm (+0x108)
#   LeftHand [+0x220] / RightHand [+0x270] unless bHideLeftHand (+0x100) / bHideRightHand (+0x101)
#   LeftLeg [+0x2c0] / RightLeg [+0x2e8] unless bHideLeftLeg (+0x104) / bHideRightLeg (+0x105)
#   LeftFoot [+0x310] / RightFoot [+0x338] unless bHideLeftFoot (+0x102) / bHideRightFoot (+0x103)
#   each slot's wearable: Mesh [+0x110] then AuxiliaryMesh [+0x160] (bTreatAsMaster ones only with bAddMasterMeshes)
#   auxiliaries if any wearable requires them: ForeArm [face +0x3d8] (bRequiresForearmAuxiliary +0x1b1),
#   FullArm [+0x388] (+0x1b0), UpperChest [+0x428] (+0x1b2), Ankle [+0x450] (+0x1b3)
# Where each part lands in the decompiled function's output order between the left/right pair is not resolved
# (UNCONFIRMED); the set of meshes is what matters for rendering.
static func construction(l: Loadout, face: UeWearable.Face, hair := "", facial_hair := "", add_master := true) -> PackedStringArray:
	var fl := WearableData.Flags.new()
	for slot in Loadout.SLOTS:
		var w: WearableData = l.wearable(slot)
		if w != null:
			fl.merge(w.flags)
	var out := PackedStringArray()
	var add := func(p: String) -> void:
		if p != "":
			out.append(p)
	if face == null:
		face = UeWearable.Face.new()
	if not fl.hide_chest: add.call(face.torso)
	add.call(face.mesh)
	if not fl.hide_hair: add.call(hair)
	if not fl.hide_beard: add.call(facial_hair)
	add.call(face.eyes)
	if not fl.hide_left_arm: add.call(face.left_arm)
	if not fl.hide_right_arm: add.call(face.right_arm)
	if not fl.hide_left_hand: add.call(face.left_hand)
	if not fl.hide_right_hand: add.call(face.right_hand)
	if not fl.hide_left_leg: add.call(face.left_leg)
	if not fl.hide_right_leg: add.call(face.right_leg)
	if not fl.hide_left_foot: add.call(face.left_foot)
	if not fl.hide_right_foot: add.call(face.right_foot)
	for slot in Loadout.SLOTS:
		var w: WearableData = l.wearable(slot)
		if w == null or (w.b_treat_as_master and not add_master):
			continue
		add.call(w.mesh)
		add.call(w.aux_mesh)
	if fl.requires_forearm_aux: add.call(face.fore_arm_aux)
	if fl.requires_full_arm_aux: add.call(face.full_arm_aux)
	if fl.requires_upper_chest_aux: add.call(face.upper_chest_aux)
	if fl.requires_ankle_aux: add.call(face.ankle_aux)
	return out

# melee_damage.gd - how much a melee hit on a bone does: AMordhauCharacter::ComputeMeleeDamage rva=0x1536fb0,
# GetArmorTierForBone rva=0x153ee30, IsHead rva=0x154aa40, IsLeftLeg rva=0x154ad60, IsRightLeg rva=0x154b880.
#   tier = DamageArmorTierOverride (+0x7f4) unless it is -1 (0xffffffff), else
#          WearableProtectionCoverageMap[bone] (+0xb18, FName -> UMordhauWearable*)->ArmorClass (+0x1bc), 0 if absent
#   damage = Damage[min(tier, Damage.Num - 1)] (0 if the index is out of range)
#          + HeadBonus[min(tier, ...)] if bone == NAME_Head
#          + LegBonus[min(tier, ...)] if bone is a left or right leg bone
#              (x LegDamageBonusModifierAirborne, +0xe58, when ReplicatedCharacterFlags +0x839 bit 0 is set)
# The bone FNames are module-static FName globals; their text comes from the exe through the reader layer
# (CombatConstants.head_bones / left_leg_bones / right_leg_bones, which cite the strings and initializers).
# FName comparison ignores case.
class_name MeleeDamage

static func _is(bone: String, names: PackedStringArray) -> bool:
	for n in names:
		if bone.nocasecmp_to(n) == 0:
			return true
	return false

static func is_head(bone: String) -> bool:
	return _is(bone, CombatConstants.head_bones)

static func is_leg(bone: String) -> bool:		# AMordhauCharacter::IsLeg rva=0x154aed0
	return _is(bone, CombatConstants.left_leg_bones) or _is(bone, CombatConstants.right_leg_bones)

static func _at(a: PackedFloat32Array, tier: int) -> float:
	var i := mini(tier, a.size() - 1)
	return a[i] if i >= 0 and i < a.size() else 0.0

# AMordhauCharacter::GetArmorTierForBone rva=0x153ee30 (header): `who` is a MotionSystem
static func armor_tier(who, bone: String) -> int:
	var tier: int = who.armor_tier_override
	if tier == -1:
		tier = 0
		for k in who.wearable_coverage:
			if String(k).nocasecmp_to(bone) == 0:
				tier = int(who.wearable_coverage[k])
	return tier

# victim: MotionSystem (armor_tier_override, wearable_coverage, character defaults)
static func compute(victim, ai: AttackInfo, bone: String) -> float:
	var tier := armor_tier(victim, bone)
	var d := _at(ai.damage, tier)
	if is_head(bone):
		d += _at(ai.head_bonus, tier)
	elif is_leg(bone):
		var lb := _at(ai.leg_bonus, tier)
		if victim.airborne:
			lb *= victim.character.leg_damage_bonus_modifier_airborne
		d += lb
	return d

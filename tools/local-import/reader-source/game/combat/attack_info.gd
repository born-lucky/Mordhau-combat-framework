# attack_info.gd - FAttackInfo (Mordhau native struct) as a Godot Resource.
# Fields = every key seen on any FAttackInfo property (Strike/Stab/Couch/Kick/Bash/Base*/Second*) across the CDOs of
# all 279 Blueprints rooted at native MordhauWeapon/MordhauShield/FistsWeapon/KickWeapon (mdx json, full scan).
# Times are seconds, turn caps degrees/s, damage per armor tier [0..3].
#
# Defaults = the native constructor FAttackInfo::FAttackInfo, rva 0x16117e0
# (extract/native/decomp/FAttackInfo.cpp). Each store's byte offset is mapped to a field by the PDB layout in
# extract/native/types/FAttackInfo.h (sizeof 0x128); the [0x..] after each default below is that offset. Floats are
# the stored IEEE-754 bit patterns (x64 is little-endian). AMordhauWeapon::AMordhauWeapon (rva 0x1610e30) runs this
# constructor for all 9 attack members (StabAttack 0xf40 .. BashAttack 0x1880) and writes nothing inside them after,
# and the Shield/Fists/Kick/VirtualWeapon constructors write nothing there either, so these are the archetype values
# a Blueprint chain falls back to. Blueprints serialize only fields that differ from this (see components/ue/pkg/ue_pkg.gd).
class_name AttackInfo
extends Resource

@export var b_can_combo := true						# [0x00] *param_1 = 0x101 -> bytes 01 01 00 00
@export var b_can_miss_combo := true				# [0x01]
@export var b_forces_rearing_from_front := false	# [0x02]
@export var b_no_flinch := false					# [0x03]
@export var b_no_release_flinch := false			# [0x04] byte store 0
@export var flinch_speed_modifier := 1.0			# [0x08] 0x3f800000
@export var flinch_duration_modifier := 1.0			# [0x0c] 0x3f800000
@export var windup := 0.4							# [0x10] 0x3ecccccd
@export var combo_windup_increase := 0.2			# [0x14] 0x3e4ccccd
@export var miss_combo_extra_windup_increase := 0.2	# [0x18] 0x3e4ccccd
@export var release := 0.4							# [0x1c] 0x3ecccccd
@export var feint_lock_out := 0.38					# [0x20] 0x3ec28f5c
@export var feint_cost := 7							# [0x24]
@export var chamber_feint_cost := 5					# [0x28]
@export var chamber_cost := 20						# [0x2c] 0x14
@export var morph_cost := 7							# [0x30]
@export var turn_caps := Vector2(250, 90)			# [0x34] 0x437a0000, [0x38] 0x42b40000
@export var turn_cap_curve := ""					# [0x40] 0 (null UCurveFloat*)
@export var hit_effect_ik_weight_curve := ""		# [0x48] 0
@export var hit_effect_speed_up_exponent := 0.0		# [0x50] 0
@export var stamina_drain := 14.0					# [0x54] 0x41600000
@export var extra_stamina_drain_vs_held_block := 0.0	# [0x58] 0
@export var stamina_damage := 0.0					# [0x5c] 0
@export var damage := PackedFloat32Array([75, 50, 40, 30])		# [0x60] TArray, 4 Adds: 0x42960000 0x42480000 0x42200000 0x41f00000
@export var head_bonus := PackedFloat32Array([15, 15, 15, 15])	# [0x70] 4 Adds of 0x41700000
@export var leg_bonus := PackedFloat32Array([-10, -10, -10, -10])	# [0x80] 4 Adds of 0xc1200000
@export var wood_damage := 20.0						# [0x90] 0x41a00000
@export var stone_damage := 2.0						# [0x94] 0x40000000
@export var b_stop_on_hit := false					# [0x98] u16 store 0
@export var b_drain_all_stam_on_block := false		# [0x99]
@export var b_ragdoll_on_block := false				# [0x9a] byte store 0
@export var chip_damage_percentage_on_block := 0.0	# [0x9c] u64 store 0 (also zeroes 0xa0..0xa3)
@export var b_will_clash_when_parried := false		# [0xa0]
@export var b_ragdoll_on_hit := false				# [0xa1]
@export var b_dismounts_horse_rider := false		# [0xa2]
@export var b_dismounts_ladder_user := false		# [0xa3]
@export var miss_stamina_cost := 15.0				# [0xa4] u64 store 0x41700000 (low dword)
@export var hit_stamina_reward := 0.0				# [0xa8] (high dword of that store = 0)
@export var miss_recovery := 0.8					# [0xac] 0x3f4ccccd
@export var hit_knockback_factor := 1.0				# [0xb0] 0x3f800000
@export var follow_attack_direction_factor := 0.0	# [0xb4] 0
@export var wound_info := []						# [0xb8] empty TArray. [{WoundType: {X,Y}, WoundSize: {X,Y,Z}}] as UE dicts
@export var hit_shake := ""							# [0xc8] 0 (UE object path when set)
@export var hit_stop_shake := ""					# [0xd0] 0
@export var ignore_bones := PackedStringArray()		# [0xd8] empty TSet
# UE field names this attack never received from any CDO in the chain: they hold the native default above
@export var unset := PackedStringArray()

# UE property name -> our field. UE's own typo "HitKockbackFactor" is kept on the UE side only.
const MAP := {
	"Windup": "windup", "ComboWindupIncrease": "combo_windup_increase",
	"MissComboExtraWindupIncrease": "miss_combo_extra_windup_increase", "Release": "release",
	"MissRecovery": "miss_recovery", "FeintLockOut": "feint_lock_out", "FeintCost": "feint_cost",
	"ChamberCost": "chamber_cost", "ChamberFeintCost": "chamber_feint_cost", "MorphCost": "morph_cost",
	"TurnCaps": "turn_caps", "StaminaDrain": "stamina_drain", "StaminaDamage": "stamina_damage",
	"ExtraStaminaDrainVsHeldBlock": "extra_stamina_drain_vs_held_block",
	"MissStaminaCost": "miss_stamina_cost", "HitStaminaReward": "hit_stamina_reward",
	"Damage": "damage", "HeadBonus": "head_bonus", "LegBonus": "leg_bonus",
	"WoodDamage": "wood_damage", "StoneDamage": "stone_damage", "HitKockbackFactor": "hit_knockback_factor",
	"FlinchSpeedModifier": "flinch_speed_modifier", "FlinchDurationModifier": "flinch_duration_modifier",
	"FollowAttackDirectionFactor": "follow_attack_direction_factor",
	"ChipDamagePercentageOnBlock": "chip_damage_percentage_on_block",
	"HitEffectSpeedUpExponent": "hit_effect_speed_up_exponent",
	"bCanCombo": "b_can_combo", "bCanMissCombo": "b_can_miss_combo", "bStopOnHit": "b_stop_on_hit",
	"bNoFlinch": "b_no_flinch", "bNoReleaseFlinch": "b_no_release_flinch",
	"bWillClashWhenParried": "b_will_clash_when_parried", "bDrainAllStamOnBlock": "b_drain_all_stam_on_block",
	"bDismountsHorseRider": "b_dismounts_horse_rider", "bDismountsLadderUser": "b_dismounts_ladder_user",
	"bForcesRearingFromFront": "b_forces_rearing_from_front",
	"bRagdollOnBlock": "b_ragdoll_on_block", "bRagdollOnHit": "b_ragdoll_on_hit",
	"IgnoreBones": "ignore_bones", "WoundInfoArray": "wound_info",
	"HitShake": "hit_shake", "HitStopShake": "hit_stop_shake",
	"HitEffectIKWeightCurve": "hit_effect_ik_weight_curve", "TurnCapCurve": "turn_cap_curve",
}

# UE keys in the source dict that MAP does not cover: a new field shows up here instead of being dropped silently
var unmapped := PackedStringArray()

# Built from a CDO FAttackInfo struct by the reader layer: CombatData.attack_info_from_ue(d).

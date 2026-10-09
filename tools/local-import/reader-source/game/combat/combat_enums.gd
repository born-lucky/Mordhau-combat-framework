# combat_enums.gd - Mordhau's combat enums, values as the game defines them.
# EAttackMove, EAttackType, EBlockType, EMovementRestriction, EParryRecoveryType: LF_ENUM records of the shipped PDB
#   (llvm-pdbutil dump -types Mordhau-Win64-Shipping.pdb, build 702625635).
# EAttackStage, EFeintType, EParryStage: no LF_ENUM record; values are the order of the UHT enumerator name table in
#   the exe .rdata ("EAttackStage::Windup" @file 0x43882c0, "::Release" 0x43882d8, "::Recovery" 0x43882f0;
#   "EFeintType::Regular" 0x43a0240, "::Combo" 0x43a0258, "::Chamber" 0x43a0270; "EParryStage::Parry", "::Recovery"),
#   and they agree with every use in the decompiled code (UAttackMotion::OnTick_Implementation compares Stage to
#   0/1/2 and calls OnTickWindUp only at 0; UAttackMotion::ProcessFeint_Implementation writes EFeintType 1 for a
#   combo, 2 when chambered).
class_name CombatEnums

enum Move { RIGHT_STRIKE = 0, LEFT_STRIKE = 1, STAB = 2, ALT_STAB = 3, KICK = 4, BASH = 5, COUCH = 6, RANGED = 7 }
enum AttackType { REGULAR = 0, RIPOSTE = 1, COMBO = 2, POST_CLASH = 3, MORPH = 4, MISS_COMBO = 5, POST_CLASH_SLOW = 6 }
enum Stage { WINDUP = 0, RELEASE = 1, RECOVERY = 2 }
enum BlockType { REGULAR = 0, ALT_REGULAR = 1, SHIELD_WALL = 2 }
enum FeintType { REGULAR = 0, COMBO = 1, CHAMBER = 2 }
enum ParryStage { PARRY = 0, RECOVERY = 1 }
enum ParryRecovery { SUCCESS = 0, FAIL = 1, MISS = 2 }
enum MovementRestriction { NONE = 0, PARTIAL_SPRINT = 1, WALK = 2, NO_MOVEMENT = 3 }	# EMovementRestriction LF_ENUM

const MOVE_NAMES := ["RightStrike", "LeftStrike", "Stab", "AltStab", "Kick", "Bash", "Couch", "Ranged"]
const STAGE_NAMES := ["Windup", "Release", "Recovery"]
const PARRY_STAGE_NAMES := ["Parry", "Recovery"]
const ATTACK_TYPE_NAMES := ["Regular", "Riposte", "Combo", "PostClash", "Morph", "MissCombo", "PostClashSlow"]
const MOVEMENT_RESTRICTION_NAMES := ["None", "PartialSprint", "Walk", "NoMovement"]

# "EMovementRestriction::Walk" -> 2; "" (a WeaponData field the Blueprint chain never wrote) -> 0 = None, the ctor value
static func movement_restriction_of(s: String) -> int:
	return maxi(MOVEMENT_RESTRICTION_NAMES.find(s.get_slice("::", 1)), 0) if s != "" else 0

# "EAttackMove::LeftStrike" -> 1
static func move_from_ue(s: String) -> int:
	return MOVE_NAMES.find(s.get_slice("::", 1))

# UAttackMotion::IsStrike rva=0x162c570: move < 2
static func is_strike(m: int) -> bool:
	return m < 2

# UAttackMotion::IsStab rva=0x162c560: (move - 2) < 2 unsigned
static func is_stab(m: int) -> bool:
	return m == 2 or m == 3

# UAttackMotion::IsLeft rva=0x162c140: ((move - 1) & 0xfd) == 0, i.e. LeftStrike or AltStab
static func is_left(m: int) -> bool:
	return m == 1 or m == 3

# UAttackMotion::FlipSide rva=0x161dc80
static func flip_side(m: int) -> int:
	match m:
		0: return 1
		1: return 0
		2: return 3
		3: return 2
	return m

# ---- FNetMotion byte packing (types/FNetMotion.h: Id, MotionType, MotionParam0..2, MotionDynamicParam; the PDB
# declares static GetHI/GetLO/SetHI/SetLO(uint8) helpers, inlined in the game code as >> 4 / & 0xf / << 4).
static func hi(b: int) -> int:		# FNetMotion::GetHI
	return (b >> 4) & 0xf

static func lo(b: int) -> int:		# FNetMotion::GetLO
	return b & 0xf

static func pack(hi_: int, lo_: int) -> int:	# FNetMotion::SetHI + SetLO
	return ((hi_ & 0xf) << 4) | (lo_ & 0xf)

# MotionType byte of FNetMotion (the literals the builders write: Attack 0x100 -> 1, Parry 0x200 -> 2,
# Flinched 0x300 -> 3, Feinted 0x500 -> 5, Blocked 0x600 -> 6; FNetMotion::Flinched rva=0x14bb820, ::Blocked
# rva=0x1615f50, AssignNetAttackMotion rva=0x14b3210, UMordhauMotion::ProcessBlock_Implementation rva=0x166bb20,
# UAttackMotion::ProcessFeint_Implementation rva=0x1638220)
enum NetType { ATTACK = 1, PARRY = 2, FLINCHED = 3, STUNNED = 4, FEINTED = 5, BLOCKED = 6, DISARMED = 7 }	# FNetMotion::Stunned 4, ::Disarmed 7

# Attack motion MotionDynamicParam bits, named after what UAttackMotion::OnDynamicParamChanged_Implementation
# rva=0x1631060 does with each (no PDB names exist: the field is a plain uint8):
const DYN_HIT := 1					# -> bHasHit (+0x10ea); set by ProcessHitForDamage rva=0x1638a60 (last line)
const DYN_CHAMBERED := 2			# -> bHasChambered (+0x10f4); set by CheckChamber rva=0x1617140
const DYN_PARENT_HIT_LATE := 4		# miss-combo whose parent attack hit: drop MissComboExtraWindupIncrease
# Parry motion MotionDynamicParam: GetLO = recovery request (1 Fail, 2 Miss), GetHI = TotalBlocks
# (UParryMotion::OnDynamicParamChanged_Implementation rva=0x1663210)

enum BlockedReason { PARRY = 0, CHAMBER = 1, WORLD = 2, CLASH = 3, HIT = 4 }	# types/EBlockedReason.h

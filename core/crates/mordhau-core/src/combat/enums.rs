//! Mordhau's combat enums, values as the game defines them (godot/game/combat/combat_enums.gd).
//! EAttackMove, EAttackType, EBlockType, EMovementRestriction, EParryRecoveryType: LF_ENUM records of the shipped PDB
//! (llvm-pdbutil dump -types Mordhau-Win64-Shipping.pdb, build 702625635).
//! EAttackStage, EFeintType, EParryStage: no LF_ENUM record; values are the order of the UHT enumerator name table in
//! the exe .rdata ("EAttackStage::Windup" @file 0x43882c0, "::Release" 0x43882d8, "::Recovery" 0x43882f0;
//! "EFeintType::Regular" 0x43a0240, "::Combo" 0x43a0258, "::Chamber" 0x43a0270), and they agree with every use in
//! the decompiled code (UAttackMotion::OnTick_Implementation compares Stage to 0/1/2).
//! Values stay plain i64 (they travel through FNetMotion bytes and the golden traces as numbers).

pub mod mv {
    //! EAttackMove
    pub const RIGHT_STRIKE: i64 = 0;
    pub const LEFT_STRIKE: i64 = 1;
    pub const STAB: i64 = 2;
    pub const ALT_STAB: i64 = 3;
    pub const KICK: i64 = 4;
    pub const BASH: i64 = 5;
    pub const COUCH: i64 = 6;
    pub const RANGED: i64 = 7;
}
pub mod at {
    //! EAttackType
    pub const REGULAR: i64 = 0;
    pub const RIPOSTE: i64 = 1;
    pub const COMBO: i64 = 2;
    pub const POST_CLASH: i64 = 3;
    pub const MORPH: i64 = 4;
    pub const MISS_COMBO: i64 = 5;
    pub const POST_CLASH_SLOW: i64 = 6;
}
pub mod stage {
    //! EAttackStage
    pub const WINDUP: i64 = 0;
    pub const RELEASE: i64 = 1;
    pub const RECOVERY: i64 = 2;
}
pub mod bt {
    //! EBlockType
    pub const REGULAR: i64 = 0;
    pub const ALT_REGULAR: i64 = 1;
    pub const SHIELD_WALL: i64 = 2;
}
pub mod ft {
    //! EFeintType
    pub const REGULAR: i64 = 0;
    pub const COMBO: i64 = 1;
    pub const CHAMBER: i64 = 2;
}
pub mod ps {
    //! EParryStage
    pub const PARRY: i64 = 0;
    pub const RECOVERY: i64 = 1;
}
pub mod pr {
    //! EParryRecoveryType
    pub const SUCCESS: i64 = 0;
    pub const FAIL: i64 = 1;
    pub const MISS: i64 = 2;
}
pub mod mr {
    //! EMovementRestriction (PDB LF_ENUM)
    pub const NONE: i64 = 0;
    pub const PARTIAL_SPRINT: i64 = 1;
    pub const WALK: i64 = 2;
    pub const NO_MOVEMENT: i64 = 3;
}
pub mod net {
    //! MotionType byte of FNetMotion (Attack 0x100 -> 1, Parry 0x200 -> 2, Flinched 0x300 -> 3, Feinted 0x500 -> 5,
    //! Blocked 0x600 -> 6; FNetMotion::Flinched rva=0x14bb820, ::Blocked rva=0x1615f50, AssignNetAttackMotion
    //! rva=0x14b3210, UMordhauMotion::ProcessBlock_Implementation rva=0x166bb20, UAttackMotion::ProcessFeint_Implementation
    //! rva=0x1638220; FNetMotion::Stunned 4 rva=0x14d86a0, ::Disarmed 7 rva=0x14ba4a0)
    pub const ATTACK: i64 = 1;
    pub const PARRY: i64 = 2;
    pub const FLINCHED: i64 = 3;
    pub const STUNNED: i64 = 4;
    pub const FEINTED: i64 = 5;
    pub const BLOCKED: i64 = 6;
    pub const DISARMED: i64 = 7;
    /// FNetMotion::Climbing rva=0x14b5380 (`mov byte [rsi+1], 0x1b` at 0x1414b5416); added by rust-net r5 so a
    /// networked RequestClimb goes through AssignNetMotion (climb.rs)
    pub const CLIMBING: i64 = 0x1b;
    /// fp-anim r3: BP_MordhauCharacter Motions[11] = EquipmentModeSwitchMotion (the MotionType indexes AMordhauCharacter::
    /// Motions +0x10d0, begin_net_motion); Param0 = bIsSwitchingToAlt (UNCONFIRMED which FNetMotion byte OnBegin reads)
    pub const EQUIPMENT_MODE_SWITCH: i64 = 11;
}
pub mod br {
    //! EBlockedReason (types/EBlockedReason.h)
    pub const PARRY: i64 = 0;
    pub const CHAMBER: i64 = 1;
    pub const WORLD: i64 = 2;
    pub const CLASH: i64 = 3;
    pub const HIT: i64 = 4;
}

/// Attack motion MotionDynamicParam bits (UAttackMotion::OnDynamicParamChanged_Implementation rva=0x1631060)
pub const DYN_HIT: i64 = 1; // -> bHasHit (+0x10ea); set by ProcessHitForDamage rva=0x1638a60
pub const DYN_CHAMBERED: i64 = 2; // -> bHasChambered (+0x10f4); set by CheckChamber rva=0x1617140
pub const DYN_PARENT_HIT_LATE: i64 = 4; // miss-combo whose parent attack hit: drop MissComboExtraWindupIncrease

pub const MOVE_NAMES: [&str; 8] = ["RightStrike", "LeftStrike", "Stab", "AltStab", "Kick", "Bash", "Couch", "Ranged"];
pub const ATTACK_TYPE_NAMES: [&str; 7] = ["Regular", "Riposte", "Combo", "PostClash", "Morph", "MissCombo", "PostClashSlow"];
pub const MOVEMENT_RESTRICTION_NAMES: [&str; 4] = ["None", "PartialSprint", "Walk", "NoMovement"];

/// "EMovementRestriction::Walk" -> 2; "" (a WeaponData field the Blueprint chain never wrote) -> 0 = None
pub fn movement_restriction_of(s: &str) -> i64 {
    if s.is_empty() {
        return 0;
    }
    let tail = s.split("::").nth(1).unwrap_or("");
    MOVEMENT_RESTRICTION_NAMES.iter().position(|n| *n == tail).map(|i| i as i64).unwrap_or(0)
}

/// UAttackMotion::IsStrike rva=0x162c570: move < 2
pub fn is_strike(m: i64) -> bool {
    m < 2
}
/// UAttackMotion::IsStab rva=0x162c560: (move - 2) < 2 unsigned
pub fn is_stab(m: i64) -> bool {
    m == 2 || m == 3
}
/// UAttackMotion::IsLeft rva=0x162c140: ((move - 1) & 0xfd) == 0, i.e. LeftStrike or AltStab
pub fn is_left(m: i64) -> bool {
    m == 1 || m == 3
}
/// UAttackMotion::FlipSide rva=0x161dc80
pub fn flip_side(m: i64) -> i64 {
    match m {
        0 => 1,
        1 => 0,
        2 => 3,
        3 => 2,
        _ => m,
    }
}
/// FNetMotion::GetHI (inlined `>> 4`)
pub fn hi(b: i64) -> i64 {
    (b >> 4) & 0xf
}
/// FNetMotion::GetLO (inlined `& 0xf`)
pub fn lo(b: i64) -> i64 {
    b & 0xf
}
/// FNetMotion::SetHI + SetLO
pub fn pack(h: i64, l: i64) -> i64 {
    ((h & 0xf) << 4) | (l & 0xf)
}

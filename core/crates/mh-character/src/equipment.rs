//! The movement side of held equipment (rust-character r9): what any AMordhauEquipment class (melee, ranged, tools:
//! BP_Longbow, BP_ToolBox, ...) feeds UMordhauMovementComponent, read from the class chain's package JSONs over the
//! AMordhauEquipment ctor rva=0x1526480, and the two movement functions that consume it:
//!  - UMordhauMovementComponent::UpdateEquipmentSpeedAndAcceleration rva=0x14dcd30 (on equipment changes: the hands'
//!    SpeedOverride / SpeedBonusPercentage / AccelerationBonusPercentage / SubSprintSpeedBonus, the holstered items'
//!    Holstered bonuses) -> `ExeMovement::update_equipment_speed_and_acceleration`;
//!  - UMordhauMovementComponent::OnCharacterLODTick rva=0x14c9ec0 (each actor tick: the current motion's speed
//!    factors, the hands' BackpedalSpeedFactorEquipped) -> `ExeMovement::on_character_lod_tick` with
//!    `ExeMovement::current_motion_factors` / `left_equipment_backpedal` / `right_equipment_backpedal`.
//! The ranged motions (combat side) take their movement values from here: URangedDrawMotion::OnBegin_Implementation
//! rva=0x1661e50 sets the motion's SpeedFactor = RangedDrawSpeedFactor (RangedDrawSpeedFactorWithRangerPerk with the
//! Ranger perk, 0x12) and MovementRestriction = RangedDrawMovementRestriction; the release / reload motions use
//! RangedReleaseMovementRestriction / ReloadMovementRestriction.

use crate::exe::ExeMovement;
use serde_json::Value;

/// One equipment class's movement data (AMordhauEquipment fields, offsets of this build; ctor defaults cited)
#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentMovement {
    pub class: String,
    pub movement_restriction: u8,             // +0x6f4 (ctor 0)
    pub sub_sprint_speed_bonus_equipped: f32, // +0x6f8 (0)
    pub speed_override_equipped: f32,         // +0x700 (0 = none)
    pub backpedal_speed_factor_equipped: f32, // +0x704 (ctor 1)
    pub speed_bonus_percentage_equipped: f32, // +0x708 (0)
    pub acceleration_bonus_percentage_equipped: f32, // +0x70c (0)
    pub speed_bonus_percentage_holstered: f32, // +0x710 (0)
    pub acceleration_bonus_percentage_holstered: f32, // +0x714 (0)
    pub reload_movement_restriction: u8,      // +0xccc (ctor 1)
    pub ranged_draw_movement_restriction: u8, // +0xccd (ctor 2)
    pub ranged_draw_speed_factor: f32,        // +0xcd0 (ctor 1)
    pub ranged_draw_speed_factor_with_ranger_perk: f32, // +0xcd4 (ctor 1)
    pub ranged_draw_turn_caps: (f32, f32),    // +0xcd8 (ctor (-1, -1))
    pub ranged_release_movement_restriction: u8, // +0xce0 (ctor 1)
    pub ranged_reload_turn_caps: (f32, f32),  // +0xd04 (ctor (-1, -1))
}

impl Default for EquipmentMovement {
    /// the AMordhauEquipment ctor rva=0x1526480 values (AMordhauEquipment.cpp decomp lines 1730..1835)
    fn default() -> Self {
        EquipmentMovement {
            class: String::new(),
            movement_restriction: 0,
            sub_sprint_speed_bonus_equipped: 0.0,
            speed_override_equipped: 0.0,
            backpedal_speed_factor_equipped: 1.0,
            speed_bonus_percentage_equipped: 0.0,
            acceleration_bonus_percentage_equipped: 0.0,
            speed_bonus_percentage_holstered: 0.0,
            acceleration_bonus_percentage_holstered: 0.0,
            reload_movement_restriction: 1,
            ranged_draw_movement_restriction: 2,
            ranged_draw_speed_factor: 1.0,
            ranged_draw_speed_factor_with_ranger_perk: 1.0,
            ranged_draw_turn_caps: (-1.0, -1.0),
            ranged_release_movement_restriction: 1,
            ranged_reload_turn_caps: (-1.0, -1.0),
        }
    }
}

/// EMovementRestriction by name ("EMovementRestriction::Walk" / "Walk")
fn restriction(v: &Value) -> Option<u8> {
    let s = v.as_str()?;
    let n = s.rsplit("::").next()?;
    Some(match n {
        "None" => 0,
        "PartialSprint" => 1,
        "Walk" => 2,
        "NoMovement" => 3,
        _ => return None,
    })
}

impl EquipmentMovement {
    /// The class chain's package JSONs, most derived first (e.g. BP_Longbow, BP_Bow, BP_RangedWeapon, ...); each
    /// property takes the most derived value that sets it, over the ctor defaults
    pub fn from_json_chain(chain: &[&str]) -> Result<EquipmentMovement, String> {
        let mut m = EquipmentMovement::default();
        for (i, j) in chain.iter().rev().enumerate() {
            let ex: Value = serde_json::from_str(j).map_err(|e| format!("equipment json {i}: {e}"))?;
            let Some(cdo) = ex.as_array().and_then(|a| a.iter().find(|e| e["Name"].as_str().is_some_and(|n| n.starts_with("Default__")))) else { continue };
            if i == chain.len() - 1 {
                m.class = cdo["Name"].as_str().unwrap_or("").trim_start_matches("Default__").to_string();
            }
            m.apply_props(&cdo["Properties"]);
        }
        Ok(m)
    }

    /// A class's merged class-default properties (mh_pak Reader::defaults: the class chain's CDOs, most derived last)
    pub fn from_defaults(class: &str, p: &Value) -> EquipmentMovement {
        let mut m = EquipmentMovement { class: class.to_string(), ..Default::default() };
        m.apply_props(p);
        m
    }

    fn apply_props(&mut self, p: &Value) {
        let m = self;
            let f = |k: &str| p.get(k).and_then(Value::as_f64).map(|x| x as f32);
            let set = |k: &str, d: &mut f32| {
                if let Some(x) = f(k) {
                    *d = x;
                }
            };
            set("SubSprintSpeedBonusEquipped", &mut m.sub_sprint_speed_bonus_equipped);
            set("SpeedOverrideEquipped", &mut m.speed_override_equipped);
            set("BackpedalSpeedFactorEquipped", &mut m.backpedal_speed_factor_equipped);
            set("SpeedBonusPercentageEquipped", &mut m.speed_bonus_percentage_equipped);
            set("AccelerationBonusPercentageEquipped", &mut m.acceleration_bonus_percentage_equipped);
            set("SpeedBonusPercentageHolstered", &mut m.speed_bonus_percentage_holstered);
            set("AccelerationBonusPercentageHolstered", &mut m.acceleration_bonus_percentage_holstered);
            set("RangedDrawSpeedFactor", &mut m.ranged_draw_speed_factor);
            set("RangedDrawSpeedFactorWithRangerPerk", &mut m.ranged_draw_speed_factor_with_ranger_perk);
            for (k, d) in [
                ("MovementRestriction", &mut m.movement_restriction),
                ("ReloadMovementRestriction", &mut m.reload_movement_restriction),
                ("RangedDrawMovementRestriction", &mut m.ranged_draw_movement_restriction),
                ("RangedReleaseMovementRestriction", &mut m.ranged_release_movement_restriction),
            ] {
                if let Some(x) = p.get(k).and_then(restriction) {
                    *d = x;
                }
            }
            for (k, d) in [("RangedDrawTurnCaps", &mut m.ranged_draw_turn_caps), ("RangedReloadTurnCaps", &mut m.ranged_reload_turn_caps)] {
                if let Some(v) = p.get(k) {
                    let x = v.get("X").and_then(Value::as_f64).map_or(d.0, |x| x as f32);
                    let y = v.get("Y").and_then(Value::as_f64).map_or(d.1, |x| x as f32);
                    *d = (x, y);
                }
            }
    }

    /// The draw motion's (SpeedFactor, MovementRestriction) as URangedDrawMotion::OnBegin_Implementation rva=0x1661e50
    /// sets them (the Ranger perk picks RangedDrawSpeedFactorWithRangerPerk); BackpedalSpeedFactor stays the
    /// UMordhauMotion default
    pub fn draw_motion(&self, ranger_perk: bool) -> (f32, u8) {
        (if ranger_perk { self.ranged_draw_speed_factor_with_ranger_perk } else { self.ranged_draw_speed_factor }, self.ranged_draw_movement_restriction)
    }
}

impl ExeMovement {
    /// UMordhauMovementComponent::UpdateEquipmentSpeedAndAcceleration rva=0x14dcd30 (read off the disassembly): the
    /// override / bonuses zeroed; the left hand (+0x1200): SpeedOverrideEquipped when > 0, SpeedBonusPercentage,
    /// AccelerationBonusPercentage, SubSprintSpeedBonus; the right hand (+0x11f8): its override when > 0 and (no
    /// override yet or smaller), its bonuses added; every other carried item (+0x11e8 array, not a hand): the Holstered
    /// bonuses added. (UpdateArmorSpeedAndAcceleration rva=0x14dc0c0 follows in the exe: the armor fields are the
    /// host's, `armor_speed` / `armor_accel`.)
    pub fn update_equipment_speed_and_acceleration(&mut self, left: Option<&EquipmentMovement>, right: Option<&EquipmentMovement>, others: &[&EquipmentMovement]) {
        self.equip_speed_cap = 0.0;
        self.equip_speed_add = 0.0;
        self.equip_sprint_penalty = 0.0;
        self.equip_accel_add = 0.0;
        let mut ov = 0.0f32;
        let (mut sp, mut ac, mut ss) = (0.0f32, 0.0f32, 0.0f32);
        if let Some(l) = left {
            if l.speed_override_equipped > 0.0 {
                self.equip_speed_cap = l.speed_override_equipped;
                ov = l.speed_override_equipped;
            }
            sp = l.speed_bonus_percentage_equipped;
            ac = l.acceleration_bonus_percentage_equipped;
            ss = l.sub_sprint_speed_bonus_equipped;
            self.equip_speed_add = sp;
            self.equip_accel_add = ac;
            self.equip_sprint_penalty = ss;
        }
        if let Some(r) = right {
            let o = r.speed_override_equipped;
            if o > 0.0 && (ov == 0.0 || o < ov) {
                self.equip_speed_cap = o;
            }
            self.equip_speed_add = sp + r.speed_bonus_percentage_equipped;
            self.equip_accel_add = ac + r.acceleration_bonus_percentage_equipped;
            self.equip_sprint_penalty = ss + r.sub_sprint_speed_bonus_equipped;
        }
        for o in others {
            self.equip_speed_add = o.speed_bonus_percentage_holstered + self.equip_speed_add;
            self.equip_accel_add = o.acceleration_bonus_percentage_holstered + self.equip_accel_add;
        }
        self.left_equipment_backpedal = left.map(|l| l.backpedal_speed_factor_equipped);
        self.right_equipment_backpedal = right.map(|r| r.backpedal_speed_factor_equipped);
    }
}

/// A worn armor piece's movement data (UMordhauWearable SpeedFactor +0x1c0 / AccelerationFactor +0x1c4, ArmorClass
/// +0x1bc; the UMordhauWearable ctor rva=0x1612dc0 leaves them 0): BP_Tier0..3{Head,UpperChest,Legs}Wearable hold the
/// tier values every armor class inherits
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ArmorWearable {
    pub speed_factor: f32,
    pub acceleration_factor: f32,
    pub armor_class: u8,
}

impl ArmorWearable {
    /// from a wearable class's merged class-default properties
    pub fn from_defaults(p: &Value) -> ArmorWearable {
        let f = |k: &str| p.get(k).and_then(Value::as_f64).map_or(0.0, |x| x as f32);
        ArmorWearable { speed_factor: f("SpeedFactor"), acceleration_factor: f("AccelerationFactor"), armor_class: p.get("ArmorClass").and_then(Value::as_u64).unwrap_or(0) as u8 }
    }
}

/// What UpdateArmorSpeedAndAcceleration reads besides the worn pieces
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ArmorRules {
    /// AMordhauGameState bOverrideArmorSpeedAndAccelerationFactor (+0x54d) with OverrideArmorSpeedFactor (+0x550) /
    /// OverrideArmorAccelerationFactor (+0x554)
    pub game_override: Option<(f32, f32)>,
    /// the Tank perk (6): UPerkSystemComponent TankArmorSpeedFactor (+0x104) / TankArmorAccelerationFactor (+0x108)
    pub tank: Option<(f32, f32)>,
    /// the Rat perk (7): MaxWalkSpeedCrouched = MaxWalkSpeedCrouchedWithRatPerk (+0xcbc)
    pub rat_crouched_speed: Option<f32>,
}

impl ExeMovement {
    /// UMordhauMovementComponent::UpdateArmorSpeedAndAcceleration rva=0x14dc0c0 (read off the disassembly; called at
    /// the end of UpdateEquipmentSpeedAndAcceleration): ArmorSpeedFactor = ArmorAccelerationFactor = 1; a game state
    /// override subtracts its factors; else the Tank perk subtracts its; else the worn pieces of WearableObjectInstances
    /// [Head 0], [UpperChest 2], [Legs 7]: ArmorAccelerationFactor -= each AccelerationFactor in that order, and
    /// ArmorSpeedFactor -= ((UpperChest + Head) + Legs) - min(min(UpperChest, Head), Legs) of their SpeedFactors (the
    /// two heaviest pieces count); then the Rat perk's crouched speed
    pub fn update_armor_speed_and_acceleration(&mut self, head: Option<&ArmorWearable>, upper_chest: Option<&ArmorWearable>, legs: Option<&ArmorWearable>, rules: &ArmorRules) {
        self.armor_speed = 1.0;
        self.armor_accel = 1.0;
        if let Some((sp, ac)) = rules.game_override {
            self.armor_speed = self.armor_speed - sp;
            self.armor_accel = self.armor_accel - ac;
        } else if let Some((sp, ac)) = rules.tank {
            self.armor_speed = self.armor_speed - sp;
            self.armor_accel = self.armor_accel - ac;
        } else {
            let (mut h, mut u, mut l) = (0.0f32, 0.0f32, 0.0f32);
            if let Some(w) = head {
                self.armor_accel = self.armor_accel - w.acceleration_factor;
                h = w.speed_factor;
            }
            if let Some(w) = upper_chest {
                self.armor_accel = self.armor_accel - w.acceleration_factor;
                u = w.speed_factor;
            }
            if let Some(w) = legs {
                self.armor_accel = self.armor_accel - w.acceleration_factor;
                l = w.speed_factor;
            }
            let minss = |a: f32, b: f32| if a < b { a } else { b };
            let sum = (u + h) + l;
            let lo = minss(minss(u, h), l);
            self.armor_speed = self.armor_speed - (sum - lo);
        }
        if let Some(c) = rules.rat_crouched_speed {
            self.c.max_walk_speed_crouched = c;
        }
    }
}

//! The data the character rules read, as typed records, loaded through one trait (`CharacterSource`).
//!
//! Data is read, never re-authored (docs/DESIGN.md §1): no gameplay value is typed in here. The records mirror the
//! GDScript record layer godot/components/ue/records/character_data.gd (CharacterData.Movement / MoveExtra /
//! Character) field for field, with the same snake_case names; UE offsets and sources are cited there and repeated on
//! the fields the rules depend on. Two sources ship in r1:
//!  - `RecordsJson`: the record dump of godot/tools/export_golden_character.gd (core/tests/golden/character/
//!    records.json, git-ignored Triternion data), i.e. the exact values the reference read (native ctor defaults +
//!    Blueprint CDO chain + DefaultEngine.ini). Floats are the 16 hex digits of their little-endian f64 bytes.
//!  - `ExtractJson`: BP_MordhauCharacter.json (extract/json, read-only) + DefaultEngine.ini text, over the native
//!    ctor defaults taken from another source (the native constructor replay, NativeCtor in the reference, is not
//!    ported: mh-spec will supply it). Cross-checked against the dump in tests/golden.rs.
//! When mh-spec (docs/SPEC_SHEETS.md) lands, swapping it in is one new `impl CharacterSource`.

use serde_json::Value;

/// A record field type readable from the dump (hex f64 / bool / int) and from package JSON (number / bool).
pub trait Field: Sized {
    fn dump(v: &Value) -> Option<Self>;
    fn ue(v: &Value) -> Option<Self>;
}

/// f64 from the dump's 16-hex-digit little-endian bytes (PackedByteArray.encode_double + hex_encode)
pub fn hex_f64(s: &str) -> Option<f64> {
    if s.len() != 16 {
        return None;
    }
    let mut b = [0u8; 8];
    for (i, byte) in b.iter_mut().enumerate() {
        *byte = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(f64::from_le_bytes(b))
}

impl Field for f64 {
    fn dump(v: &Value) -> Option<f64> {
        hex_f64(v.as_str()?)
    }
    fn ue(v: &Value) -> Option<f64> {
        v.as_f64()
    }
}
impl Field for i64 {
    fn dump(v: &Value) -> Option<i64> {
        v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
    }
    fn ue(v: &Value) -> Option<i64> {
        v.as_i64()
    }
}
impl Field for String {
    fn dump(v: &Value) -> Option<String> {
        v.as_str().map(str::to_string)
    }
    /// an object reference: {ObjectName, ObjectPath "<package>.<index>"} -> the package path (UeRec.ref_path)
    fn ue(v: &Value) -> Option<String> {
        let p = v.get("ObjectPath")?.as_str()?;
        Some(p.rsplit_once('.').map(|(a, _)| a).unwrap_or(p).to_string())
    }
}
impl Field for [f64; 3] {
    fn dump(v: &Value) -> Option<[f64; 3]> {
        let a = v.as_array()?;
        Some([hex_f64(a.first()?.as_str()?)?, hex_f64(a.get(1)?.as_str()?)?, hex_f64(a.get(2)?.as_str()?)?])
    }
    /// FVector {X, Y, Z} (UE axes)
    fn ue(v: &Value) -> Option<[f64; 3]> {
        Some([v.get("X")?.as_f64()?, v.get("Y")?.as_f64()?, v.get("Z")?.as_f64()?])
    }
}
impl Field for bool {
    fn dump(v: &Value) -> Option<bool> {
        v.as_bool()
    }
    fn ue(v: &Value) -> Option<bool> {
        v.as_bool()
    }
}

/// struct + dump reader + UE-property overlay from one field list (snake_case name, UE property name)
macro_rules! record {
    ($(#[$m:meta])* $name:ident { $( $(#[$fm:meta])* $f:ident : $t:ty = $ue:literal ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name { $( $(#[$fm])* pub $f: $t ),* }
        impl $name {
            /// every field from a record dump object (a missing or mistyped field is an error, like UeRec)
            pub fn from_dump(o: &Value) -> Result<Self, String> {
                Ok($name { $( $f: <$t as Field>::dump(&o[stringify!($f)])
                    .ok_or_else(|| format!("{}.{} missing in the record dump", stringify!($name), stringify!($f)))? ),* })
            }
            /// fields whose UE property is present in `props` (package JSON "Properties") replace the current value;
            /// returns the UE names that were absent
            pub fn overlay(&mut self, props: &Value) -> Vec<&'static str> {
                let mut missing = Vec::new();
                $( match props.get($ue).and_then(<$t as Field>::ue) {
                    Some(x) => self.$f = x,
                    None => if !$ue.is_empty() { missing.push($ue) },
                } )*
                missing
            }
        }
    };
}

record! {
    /// CharMoveComp (UMordhauMovementComponent template of BP_MordhauCharacter): every value the movement port reads.
    /// Engine fields (UCharacterMovementComponent: MaxWalkSpeed, GroundFriction, ...) and the Mordhau ones (ctor
    /// UMordhauMovementComponent::UMordhauMovementComponent rva=0x14af4f0) are all serialized by the template.
    Movement {
        max_walk_speed: f64 = "MaxWalkSpeed",
        max_walk_speed_crouched: f64 = "MaxWalkSpeedCrouched",            // +0x190 (the Rat perk replaces it)
        max_walk_speed_crouched_with_rat_perk: f64 = "MaxWalkSpeedCrouchedWithRatPerk", // +0xcbc
        max_speed_falling: f64 = "MaxSpeedFalling",                      // +0xd2c
        max_acceleration: f64 = "MaxAcceleration",
        walk_acceleration: f64 = "WalkAcceleration",                    // +0xcd8
        sprint_acceleration: f64 = "SprintAcceleration",
        partial_sprint_acceleration: f64 = "PartialSprintAcceleration",
        sprint_modifier: f64 = "SprintModifier",                        // +0xcc0
        partial_sprint_modifier: f64 = "PartialSprintModifier",
        backpedal_modifier: f64 = "BackpedalModifier",
        supersprint_modifier: f64 = "SupersprintModifier",
        chasing_modifier: f64 = "ChasingModifier",
        sprint_time_to_reach_max_sprint: f64 = "SprintTimeToReachMaxSprint",
        braking_deceleration_falling: f64 = "BrakingDecelerationFalling",
        braking_deceleration_falling_too_fast: f64 = "BrakingDecelerationFallingTooFast",
        braking_friction_factor: f64 = "BrakingFrictionFactor",
        ground_friction: f64 = "GroundFriction",
        falling_lateral_friction: f64 = "FallingLateralFriction",
        air_control: f64 = "AirControl",
        air_control_boost_multiplier: f64 = "AirControlBoostMultiplier",
        air_control_boost_velocity_threshold: f64 = "AirControlBoostVelocityThreshold",
        jump_z_velocity: f64 = "JumpZVelocity",
        gravity_scale: f64 = "GravityScale",
        crouched_half_height: f64 = "CrouchedHalfHeight",
        walkable_floor_z: f64 = "WalkableFloorZ",
        max_step_height: f64 = "MaxStepHeight",
        perch_radius_threshold: f64 = "PerchRadiusThreshold",            // +0x1dc (engine ctor 0)
        perch_additional_height: f64 = "PerchAdditionalHeight",          // +0x1e0 (engine ctor 40)
        b_can_walk_off_ledges_when_crouching: bool = "bCanWalkOffLedgesWhenCrouching", // +0x1f1 bit 6 (ctor clear)
    }
}

record! {
    /// Movement-component fields CharMoveComp may leave to the native constructors: the UMordhauMovementComponent ctor
    /// (base UAdvancedCharacterMovement ctor rva=0x144da20 first) with CharMoveComp merged over it. Offsets:
    /// types/UAdvancedCharacterMovement.h, UMordhauMovementComponent.h. `b_can_crouch` is NavAgentProps.bCanCrouch
    /// (UNavMovementComponent), a nested struct: `ExtractJson` reads it by hand.
    MoveExtra {
        min_velocity_for_fall_damage: f64 = "MinVelocityForFallDamage",   // +0xb98
        fall_damage_offset: f64 = "FallDamageOffset",                     // +0xb9c
        fall_damage_factor: f64 = "FallDamageFactor",                     // +0xba0
        ragdoll_min_velocity_for_fall_damage: f64 = "RagdollMinVelocityForFallDamage", // +0xba4
        ragdoll_fall_damage_offset: f64 = "RagdollFallDamageOffset",      // +0xba8
        ragdoll_fall_damage_factor: f64 = "RagdollFallDamageFactor",      // +0xbac
        knockback_ground_friction: f64 = "KnockbackGroundFriction",       // +0xd20
        knockback_falling_lateral_friction: f64 = "KnockbackFallingLateralFriction", // +0xd24
        knockback_up_impulse: f64 = "KnockbackUpImpulse",                 // +0xd28
        knockback_duration: f64 = "KnockbackDuration",                    // +0xd38
        b_can_crouch: bool = "",                                          // NavAgentProps.bCanCrouch
        // UMordhauMovementComponent::LODTick rva=0x14c4d60 terms (ctor rva=0x14af4f0 + CharMoveComp)
        turn_sprint_prevention_max_accumulated_angle: f64 = "TurnSprintPreventionMaxAccumulatedAngle", // +0xc70
        turn_sprint_prevention_decay_curve: String = "TurnSprintPreventionDecayCurve",       // +0xc60
        turn_sprint_prevention_slowdown_curve: String = "TurnSprintPreventionSlowdownCurve", // +0xc68
        rush_sprint_time_start: f64 = "RushSprintTimeStart",              // +0xc74
        chasing_sprint_time_start: f64 = "ChasingSprintTimeStart",        // +0xc78
        max_angle_to_chase: f64 = "MaxAngleToChase",                      // +0xc80
        max_angle_to_stop_chasing: f64 = "MaxAngleToStopChasing",         // +0xc84
        chasing_max_distance: [f64; 3] = "ChasingMaxDistance",            // +0xc88
        stop_chasing_max_distance: [f64; 3] = "StopChasingMaxDistance",   // +0xc94
        time_to_break_us_chasing: f64 = "TimeToBreakUsChasing",           // +0xcac
        time_to_break_us_being_chased: f64 = "TimeToBreakUsBeingChased",  // +0xcb0
        min_time_to_start_chasing: f64 = "MinTimeToStartChasing",         // +0xcb4
        min_time_to_start_being_chased: f64 = "MinTimeToStartBeingChased", // +0xcb8
        spawn_max_sprint_duration: f64 = "SpawnMaxSprintDuration",        // +0xc4c
    }
}

record! {
    /// AMordhauCharacter (+ AAdvancedCharacter) class defaults of BP_MordhauCharacter that the movement rules and their
    /// combat hooks read (CharacterData.Character; offsets types/AMordhauCharacter.h, AAdvancedCharacter.h).
    Character {
        jump_cooldown: f64 = "JumpCooldown",                    // AAdvancedCharacter +0x864 (CanJumpInternal_Implementation)
        jump_stamina_cost: f64 = "JumpStaminaCost",             // +0xe7c (AMordhauCharacter::OnJumped_Implementation)
        received_fall_damage_modifier: f64 = "ReceivedFallDamageModifier", // +0x9fc (UDamageableComponent Fall branch)
        falling_time_to_ragdoll: f64 = "FallingTimeToRagdoll",  // +0xe9c (AMordhauCharacter::LODTick)
        crouch_cooldown: f64 = "CrouchCooldown",                // +0xde8 (AMordhauCharacter::LODTick crouch toggle)
        ragdoll_falling_get_up_duration: f64 = "RagdollFallingGetUpDuration",  // +0x840 (AAdvancedCharacter ctor)
        ragdoll_falling_min_time: f64 = "RagdollFallingMinTime",               // +0x84c
        ragdoll_falling_min_velocity_to_get_up: f64 = "RagdollFallingMinVelocityToGetUp", // +0x850
        ragdoll_falling_time_at_min_velocity_to_get_up: f64 = "RagdollFallingTimeAtMinVelocityToGetUp", // +0x854
        b_disable_ragdoll_falling: bool = "bDisableRagdollFalling", // +0x7f8
        dodge_duration: f64 = "DodgeDuration",                  // AMordhauCharacter +0xea4 (ctor 0x3eb33333; CanJumpInternal)
        dodge_cooldown: f64 = "DodgeCooldown",                  // +0xea8 (ctor 0.5)
        dodge_stamina_cost: i64 = "DodgeStaminaCost",           // +0xeac (ctor 10)
        b_can_dodge: bool = "bCanDodge",                        // +0xe66 (not written by the ctor: false)
        ellipse_bubble_length: f64 = "EllipseBubbleLength",     // +0xf48 (ctor 20)
        ellipse_bubble_radius: f64 = "EllipseBubbleRadius",     // +0xf4c
        ellipse_bubble_max_height_diff: f64 = "EllipseBubbleMaxHeightDiff", // +0xf50 (ctor 150)
    }
}

/// Mordhau/Config/DefaultEngine.ini [/Script/Engine.PhysicsSettings] (MordhauMovement.load_data).
#[derive(Clone, Debug, PartialEq)]
pub struct Physics {
    pub gravity_z: f64,         // DefaultGravityZ
    pub terminal_velocity: f64, // DefaultTerminalVelocity
}

impl Default for Physics {
    /// the engine defaults when the ini has no value: the UPhysicsSettingsCore ctor (unnamed code at 0x1426fcba0 called
    /// from `UPhysicsSettings::UPhysicsSettings` at 0x14332ffa9) stores DefaultGravityZ -980 (+0x38, 0x1426fcbbd) and
    /// DefaultTerminalVelocity 4000 (+0x3c, 0x1426fcbc4); the reference (mordhau_movement.gd) uses the same
    fn default() -> Self {
        Physics { gravity_z: -980.0, terminal_velocity: 4000.0 }
    }
}

/// A UCurveFloat's FRichCurve (FloatCurve): keys (time, value, interp mode) and the extrapolation modes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurveRec {
    pub keys: Vec<(f64, f64, String)>,
    pub pre: String,
    pub post: String,
}

/// Everything the movement model loads (MordhauMovement.load_data).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterRecords {
    pub movement: Movement,
    pub move_extra: MoveExtra,
    pub character: Character,
    pub physics: Physics,
    /// curve package path -> curve (the LODTick turn-sprint prevention curves)
    pub curves: std::collections::BTreeMap<String, CurveRec>,
}

/// Anything that can produce the character records (one change to swap the data source).
pub trait CharacterSource {
    fn load(&self) -> Result<CharacterRecords, String>;
}

/// The record dump of godot/tools/export_golden_character.gd (core/tests/golden/character/records.json).
pub struct RecordsJson<'a>(pub &'a str);

impl CharacterSource for RecordsJson<'_> {
    fn load(&self) -> Result<CharacterRecords, String> {
        let root: Value = serde_json::from_str(self.0).map_err(|e| format!("records json: {e}"))?;
        let f = |k: &str| f64::dump(&root[k]).ok_or_else(|| format!("records.{k} missing"));
        Ok(CharacterRecords {
            movement: Movement::from_dump(&root["movement"])?,
            move_extra: MoveExtra::from_dump(&root["move_extra"])?,
            character: Character::from_dump(&root["character"])?,
            physics: Physics { gravity_z: f("gravity_z")?, terminal_velocity: f("terminal_velocity")? },
            curves: root["curves"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, c)| {
                    let keys = c["keys"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|kk| {
                            Some((hex_f64(kk["time"].as_str()?)?, hex_f64(kk["value"].as_str()?)?, kk["interp"].as_str().unwrap_or("RCIM_Linear").to_string()))
                        })
                        .collect();
                    let ex = |i: usize| c["extrap"][i].as_str().unwrap_or("RCCE_Constant").to_string();
                    (k.clone(), CurveRec { keys, pre: ex(0), post: ex(1) })
                })
                .collect(),
        })
    }
}

/// BP_MordhauCharacter.json (CUE4Parse export list, extract/json/Mordhau/Content/Mordhau/Blueprints/Characters/) +
/// DefaultEngine.ini text (extract/config, `repak get`), over `native` (the native ctor defaults: MoveExtra's base
/// fall-damage terms, the Character fields the CDO leaves to AAdvancedCharacter / AMordhauCharacter ctors).
/// Movement must be fully serialized by CharMoveComp (CharacterData.movement reads every field as required).
pub struct ExtractJson<'a> {
    pub bp_json: &'a str,
    pub engine_ini: Option<&'a str>,
    pub native: CharacterRecords,
}

/// `[section]` `key=value` from ini text (UeConfig.value: last assignment wins; UNCONFIRMED for +/- array keys,
/// which the physics keys are not)
pub fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut cur = "";
    let mut out = None;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            cur = &l[1..l.len() - 1];
        } else if cur == section {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim() == key {
                    out = Some(v.trim().to_string());
                }
            }
        }
    }
    out
}

impl CharacterSource for ExtractJson<'_> {
    fn load(&self) -> Result<CharacterRecords, String> {
        let exports: Value = serde_json::from_str(self.bp_json).map_err(|e| format!("BP json: {e}"))?;
        let named = |n: &str| -> Result<&Value, String> {
            exports
                .as_array()
                .and_then(|a| a.iter().find(|e| e["Name"] == n))
                .map(|e| &e["Properties"])
                .ok_or_else(|| format!("BP_MordhauCharacter has no export {n}"))
        };
        let cmc = named("CharMoveComp")?;
        let cdo = named("Default__BP_MordhauCharacter_C")?;
        let mut r = self.native.clone();
        let mut movement = Movement::default();
        let missing = movement.overlay(cmc);
        if !missing.is_empty() {
            return Err(format!("CharMoveComp does not serialize {missing:?}"));
        }
        r.movement = movement;
        r.move_extra.overlay(cmc);
        if let Some(b) = cmc["NavAgentProps"]["bCanCrouch"].as_bool() {
            r.move_extra.b_can_crouch = b;
        }
        r.character.overlay(cdo);
        if let Some(ini) = self.engine_ini {
            let g = |k| ini_value(ini, "/Script/Engine.PhysicsSettings", k).and_then(|s| s.parse::<f64>().ok());
            if let Some(v) = g("DefaultGravityZ") {
                r.physics.gravity_z = v;
            }
            if let Some(v) = g("DefaultTerminalVelocity") {
                r.physics.terminal_velocity = v;
            }
        }
        Ok(r)
    }
}

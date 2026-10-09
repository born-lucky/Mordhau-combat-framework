//! The data layer: every record the combat rules read, as typed Rust structs, loaded through one trait.
//!
//! Data is read, never re-authored (docs/DESIGN.md §1): no gameplay value is typed in here. Values come from a
//! `SpecSource`. Round r1 ships `RecordsJson`, the record dump written by godot/tools/export_golden.gd from the
//! GDScript record layer (godot/components/ue/records: CombatData / MotionDefs / CharacterData / UeWeapon /
//! EquipmentDef / CombatConstants), i.e. the exact values the reference reads (native ctor defaults + Blueprint CDO
//! chain from the user's own extract/ and paks, .rdata literals). It is Triternion data: core/tests/golden/ is
//! git-ignored. When the sheets builder's spec JSON (mh-spec, docs/SPEC_SHEETS.md) lands, swapping it in is one new
//! `impl SpecSource` that fills the same `Spec`.
//!
//! Field names are the reference's snake_case names; their UE offsets/sources are cited on the GDScript records
//! (godot/components/ue/records/*.gd) and repeated here where the combat rules depend on them.

use crate::ue::Vec2;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;

/// Anything that can produce the combat `Spec` (one change to swap the data source).
pub trait SpecSource {
    fn load_spec(&self) -> Result<Spec, String>;
}

/// FAttackInfo (godot/game/combat/attack_info.gd; ctor FAttackInfo::FAttackInfo rva=0x16117e0, layout
/// extract/native/types/FAttackInfo.h). Only the fields the combat rules read.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AttackInfo {
    pub b_can_combo: bool,                       // [0x00]
    pub b_can_miss_combo: bool,                  // [0x01]
    pub b_no_flinch: bool,                       // [0x03]
    pub flinch_speed_modifier: f64,              // [0x08]
    pub flinch_duration_modifier: f64,           // [0x0c]
    pub windup: f64,                             // [0x10]
    pub combo_windup_increase: f64,              // [0x14]
    pub miss_combo_extra_windup_increase: f64,   // [0x18]
    pub release: f64,                            // [0x1c]
    pub feint_lock_out: f64,                     // [0x20]
    pub feint_cost: i64,                         // [0x24]
    pub chamber_feint_cost: i64,                 // [0x28]
    pub chamber_cost: i64,                       // [0x2c]
    pub morph_cost: i64,                         // [0x30]
    pub stamina_drain: f64,                      // [0x54]
    pub extra_stamina_drain_vs_held_block: f64,  // [0x58]
    pub turn_caps: Vec<f64>,                     // [0x34] FVector2D TurnCaps (turncap.rs; empty = none)
    pub hit_effect_speed_up_exponent: f64,       // [0x50] native ctor 0; cooked weapon AttackInfo
    pub turn_cap_curve: String,                  // [0x40] TurnCapCurve
    pub stamina_damage: f64,                     // [0x5c]
    pub damage: Vec<f32>,                        // [0x60] TArray<float> (PackedFloat32Array in the reference)
    pub head_bonus: Vec<f32>,                    // [0x70]
    pub leg_bonus: Vec<f32>,                     // [0x80]
    pub wood_damage: f64,                        // [0x90] (UAttackMotion::HandleBlockingHit: SurfaceType 2)
    pub stone_damage: f64,                       // [0x94] (HandleBlockingHit: any other surface but 1)
    pub b_stop_on_hit: bool,                     // [0x98]
    pub b_drain_all_stam_on_block: bool,         // [0x99]
    pub chip_damage_percentage_on_block: f64,    // [0x9c]
    pub b_will_clash_when_parried: bool,         // [0xa0]
    pub miss_stamina_cost: f64,                  // [0xa4]
    pub hit_stamina_reward: f64,                 // [0xa8]
    pub miss_recovery: f64,                      // [0xac]
}

fn yes() -> bool {
    true
}

/// WeaponData (godot/game/combat/weapon_data.gd: AMordhauWeapon ctor rva=0x1610e30 + Blueprint CDO chain).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct WeaponData {
    pub id: String,
    pub class_path: String,
    pub native_class: String,
    pub strike: AttackInfo,
    pub second_strike: AttackInfo,
    pub stab: AttackInfo,
    pub second_stab: AttackInfo,
    pub couch: AttackInfo,
    pub second_couch: AttackInfo,
    pub kick: AttackInfo,
    pub second_kick: AttackInfo,
    pub bash: AttackInfo,
    #[serde(deserialize_with = "de_vec2")]
    pub block_stamina_clamp: Vec2,          // [0x1a3c]
    #[serde(deserialize_with = "de_vec2")]
    pub second_block_stamina_clamp: Vec2,   // [0x1a48]
    pub block_stamina_negation: f64,        // [0x1a38]
    pub second_block_stamina_negation: f64, // [0x1a44]
    pub parry_backpedal_speed_factor: f64,  // [0x1a08]
    // parry_angle proof (first-person r3): the turn caps UParryMotion::OnBegin applies (ctor (450, 450) / (80, 80),
    // decomp AMordhauWeapon.cpp 2067-2070)
    #[serde(deserialize_with = "de_vec2")]
    pub parry_turn_cap: Vec2,               // [0x19f0]
    #[serde(deserialize_with = "de_vec2")]
    pub shield_wall_turn_cap: Vec2,         // [0x19f8]
    pub b_is_parry_held: bool,              // [0x1a0c]
    pub parry_held_stamina_drain: f64,      // [0x1a10]
    pub parry_window_offset: f64,           // [0x19b0]
    pub parry_mask: i64,                    // [0x19ac]
    pub attack_mask: i64,                   // [0x19a8]
    pub b_can_block: bool,                  // [0xef8]
    pub b_can_block_on_foot: bool,          // [0xef9]
    pub b_can_attack: bool,                 // [0xcb9]
    pub b_can_attack_on_foot: bool,         // [0xcba]
    /// AMordhauEquipment bCanAttackOnHorseback / AMordhauWeapon bCanBlockOnHorseback (AMordhauWeapon ctor rva=0x1610e30
    /// sets both true: decomp AMordhauWeapon.cpp 2063-2064); FLD_WPN_B_CAN_*_ON_HORSEBACK in the spec matrix
    #[serde(default = "yes")]
    pub b_can_attack_on_horseback: bool,    // [0xcbb]
    #[serde(default = "yes")]
    pub b_can_block_on_horseback: bool,     // [0xefa]
    pub b_allow_drop: bool,                 // [0xcb8]
    pub stab_release_modifier: f64,         // [0xf38]
    pub block_movement_restriction: String, // [0x1a34] "EMovementRestriction::..."
    pub kick_bounce: String,                // AMordhauEquipment KickBounce
}

impl WeaponData {
    /// AMordhauWeapon::SwitchMode_Implementation rva=0x1640a00, the fields combat reads (motion_system.gd
    /// _ATTACK_SWAPS): Strike/SecondStrike, Stab/SecondStab, Kick/SecondKick, Couch/SecondCouch,
    /// BlockStaminaNegation, BlockStaminaClamp.
    pub fn switched(&self) -> WeaponData {
        let mut w = self.clone();
        std::mem::swap(&mut w.strike, &mut w.second_strike);
        std::mem::swap(&mut w.stab, &mut w.second_stab);
        std::mem::swap(&mut w.kick, &mut w.second_kick);
        std::mem::swap(&mut w.couch, &mut w.second_couch);
        std::mem::swap(&mut w.block_stamina_negation, &mut w.second_block_stamina_negation);
        std::mem::swap(&mut w.block_stamina_clamp, &mut w.second_block_stamina_clamp);
        w
    }
}

/// EquipmentDef (godot/components/ue/records/equipment_def.gd): AMordhauEquipment fields combat reads.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct EquipmentDef {
    pub path: String,
    pub native: String,
    pub is_weapon: bool,
    pub is_shield: bool,
    pub b_second_is_right_handed: bool,       // +0x56a
    pub b_second_is_two_handed: bool,         // +0x56c
    /// fp-anim r3: bIsTwoHanded +0x56b (UEquipmentModeSwitchMotion::OnBegin SwitchType; spec.json equip already
    /// carries it)
    pub b_is_two_handed: bool,                // +0x56b
    pub b_has_alternate_mode: bool,           // +0xd23
    pub weapon_animation_profile: String,     // +0x1a60
    pub second_weapon_animation_profile: String, // +0x1a68
    pub length: f64,                          // +0x1bec
    pub second_length: f64,                   // +0x1bf0
    pub b_allow_shield_wall: bool,            // +0x1c98
    pub kick_animation: String,               // +0x7c8
    pub kick_riposte_animation: String,       // +0x7d8
    pub kick_combo_animation: String,         // +0x7e8
    pub lower_animation: String,              // +0x840
    pub b_is_right_handed: bool,              // +0x569
}

impl EquipmentDef {
    /// EquipmentDef.switched (AMordhauWeapon::SwitchMode_Implementation rva=0x1640a00): Length and the animation
    /// profile swap with their Second* fields.
    pub fn switched(&self) -> EquipmentDef {
        let mut e = self.clone();
        std::mem::swap(&mut e.length, &mut e.second_length);
        std::mem::swap(&mut e.weapon_animation_profile, &mut e.second_weapon_animation_profile);
        e
    }
}

/// The tracer sockets of a weapon mesh (UePhysics.socket "TraceStart"/"TraceEnd", bone space metres) and the
/// weapon Length AMordhauWeapon::RecalculateTracerPoints rva=0x163a940 derives from them.
#[derive(Clone, Debug, Default)]
pub struct TracerDef {
    pub start: Option<crate::ue::FVector>,
    pub end: Option<crate::ue::FVector>,
}

/// One animation profile's motion classes (CombatData.ProfileMotions; UMeleeWeaponAnimationProfile Attacks map,
/// UMotionSystemComponent::GetAttackMotionClass rva=0x14bc010, ParryMotion: GetParryMotionClass rva=0x14be9b0)
#[derive(Clone, Debug, Default)]
pub struct ProfileMotions {
    pub profile: String,
    pub motions: HashMap<i64, String>, // EAttackMove -> motion Blueprint path
    pub parry_motion: String,          // "" = none set
}

/// CombatData.weapon_setup
#[derive(Clone, Debug)]
pub struct WeaponSetup {
    pub weapon: Rc<WeaponData>,
    pub equip: Option<Rc<EquipmentDef>>,
    pub motions: HashMap<i64, String>,
    pub tracer: TracerDef,
}

/// MotionDefs.Base (godot/components/ue/records/motion_defs.gd): UMordhauMotion class defaults.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MotionBaseDef {
    pub rec: String,     // record class: Base / Attack / Kick / Parry / Feinted / Blocked / Flinch / Stun / Disarmed
    pub path: String,    // Blueprint class ("" for a native-only motion)
    pub native: String,  // C++ class
    pub b_is_flinchable: bool, // +0x60
    pub b_can_attack: bool,    // +0x6d
    pub b_can_block: bool,     // +0x6e
    pub b_can_emote: bool,     // +0x6c
    pub b_blocks_regen: bool,  // +0x88
    pub speed_factor: f64,     // +0x64
    /// +0x68 BackpedalSpeedFactor (UMordhauMotion ctor 1.0; the Blueprint CDO's value, e.g. BP_FlinchMotion 1.25)
    #[serde(default = "one_f64")]
    pub backpedal_speed_factor: f64,
    /// UParryMotion +ShieldWallSpeedFactor (ctor 1.0, decomp UParryMotion.cpp 2068; BP_ParryMotion 0.95)
    #[serde(default = "one_f64")]
    pub shield_wall_speed_factor: f64,
    pub movement_restriction: i64, // +0x61
}

fn one_f64() -> f64 {
    1.0
}

/// FSpineSpaceAdditive (types/FSpineSpaceAdditive.h: 11 FRotators): bone field -> [Pitch, Yaw, Roll] degrees
pub type SpineAdd = std::collections::BTreeMap<String, [f64; 3]>;

/// FHighMidLowSpineSpaceAdditive / the ThirdPerson half of FPerspectiveHighMidLowSpineSpaceAdditive
/// (rva=0x165c440 Get(bIsFirstPerson = false))
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Hml {
    pub high: SpineAdd,
    pub mid: SpineAdd,
    pub low: SpineAdd,
}

/// MotionDefs.Attack: UAttackMotion class defaults (field offsets in motion_defs.gd).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AttackDef {
    #[serde(deserialize_with = "de_vec2")]
    pub angling_limits: Vec2, // +0xf0
    pub clash_on_parry_follow_up_windup: f64, // +0xaa8
    pub morph_windup_modifier: f64,           // +0xba4
    pub riposte_windup_modifier: f64,         // +0xaa4
    pub chamber_stamina_recover: i64,         // +0xa7c
    pub miss_twice_stamina_cost_multiplier: f64,
    pub min_windup_time_before_morphing: f64,
    pub morph_window: f64,
    pub max_morph_total_time: f64,
    pub morph_kick_extra_time: f64,
    pub recovery_queue_window: f64,
    pub feint_window: f64,
    pub riposte_windup_can_parry_window: f64,
    pub hit_recovery: f64,
    pub clashed_recovery: f64,  // +0xabc
    pub hit_stop_recovery: f64, // +0xab8
    pub chamber_window: f64,
    pub trace_memory_stay_duration: f64, // +0xa58
    pub strike_animation_normalized_recovery_offset: f64,
    pub riposte_trade_damage_factor: f64, // +0xa9c
    pub post_friendly_hit_modifier: f64,  // +0xa8c
    pub b_do_not_make_recovery_flinchable: bool,
    pub b_is_riposte_feintable: bool,
    pub b_use_seamless_cftp_in_recovery: bool,
    pub b_can_block_from_release_after_hit: bool,
    pub b_can_auto_feint_to_attack: bool,    // +0xb98
    pub b_can_attack_from_feint_lockout: bool, // +0xb7d
    pub b_stop_on_hit_on_kills: bool,
    pub b_can_be_parried_in_early_release: bool, // +0xa6c
    pub clash_on_parry_can_parry_window: f64,    // +0xaac
    pub to_chamber_attack_angle_tolerance: f64,  // +0xa80
    pub bounce_montage: String,            // +0xae0
    pub world_bounce_curve: String,        // +0xaf8
    pub world_bounce_scale_curve: String,  // +0xb08
    pub parry_bounce_curve: String,        // +0xb18
    pub parry_late_bounce_curve: String,   // +0xb28
    pub parry_bounce_scale_curve: String,  // +0xb38
    pub extra_early_release_for_look_up_non_undercuts: f64,
    pub extra_early_release_for_look_up_overheads: f64,
    pub windup_curve: String,          // +0xdf8
    pub release_curve: String,         // +0xe18
    pub combo_windup_curve: String,    // +0xe00
    pub morph_windup_curve: String,    // +0xba8
    pub riposte_release_curve: String, // +0xe20
    pub early_release: f64,                     // +0xb48
    pub early_release_time_factor: f64,         // +0xb4c
    pub riposte_early_release: f64,             // +0xb50
    pub riposte_early_release_time_factor: f64, // +0xb54
    pub auto_blend_optimize_forward_steps: i64, // +0xbf0
    pub riposte_auto_blend_optimize_forward_steps: i64,
    pub auto_blend_optimize_forward_step_size: f64, // +0xbf8
    pub auto_blend_consider_up_vector_if_larger_than_angle: f64,
    pub regular_attacks_use_auto_blend_in: bool,
    pub combo_attacks_use_auto_blend_in: bool,
    pub post_clash_attacks_use_auto_blend_in: bool,
    pub morph_attacks_use_auto_blend_in: bool,
    pub riposte_attacks_use_auto_blend_in: bool,
    pub enable_windup_smoothing: bool,
    #[serde(deserialize_with = "de_vec2")]
    pub windup_smoothing_exponent_clamp: Vec2,
    pub auto_blend_in_spine_curve: String,
    pub auto_blend_in_weapon_curve: String,
    pub windup_animation_start_time_offset: f64, // +0x10a4 class default
    // CheckChamber / CheckAttackParry / CheckClash geometry (types/UAttackMotion.h; values: native ctor rva=0x1612540
    // + Blueprint CDO chain; not in the spec matrix yet, requested from the sheets builder)
    pub max_parry_angle_for_chamber_and_active_parry: f64,        // +0xa5c
    pub max_parry_weapon_angle_for_chamber_and_active_parry: f64, // +0xa60
    pub active_parry_stamina_cost: i64,                           // +0xa64
    pub active_parry_window: f64,                                 // +0xa68
    pub b_can_be_parried_by_forward_collider: bool,               // +0xa6e
    pub b_can_be_parried_by_forward_collider_in_early_release: bool, // +0xa6f
    pub clash_angle: f64,                                         // +0xa70
    pub early_release_is_clashable_after: f64,                    // +0xa74
    pub b_can_trace_hit_using_shield_block_collider: bool,        // +0xa91
    // world collision (UAttackMotion::ExecuteAttackTracingAndLogic rva=0x161c390 / ProcessHitForBlocking
    // rva=0x1638380; types/UAttackMotion.h). The native ctor rva=0x1612540 stores none of them (0 / null); the motion
    // Blueprint CDO chain sets the curves (BP_StrikeMotion / BP_StabMotion) and the flag (Horde ogre). Filled by mh-sim
    // spec.rs from the CDOs (not in the spec matrix).
    pub b_disable_world_collision: bool,             // +0xa46
    pub world_collision_percentage_trigger_curve: String, // +0xa48 UCurveFloat*
    pub world_collision_absolute_trigger_curve: String,   // +0xa50 UCurveFloat*
    pub b_no_damage_in_early_release: bool,          // +0xa6d
    pub animation: String,
    pub bounce_additive: String,
    // animation-side fields (mh-sim's MotionAnim; motion_defs.gd Attack, read on the ThirdPerson half)
    pub clash_animation: String,
    pub riposte_animation: String,
    pub alt_riposte_animation: String,
    pub normal_blend_in: f64,
    pub normal_parry_slow_blend_in: f64,
    pub normal_slow_blend_in: f64,
    pub combo_blend_in: f64,
    pub post_clash_blend_in: f64,
    pub morph_blend_in: f64,
    pub riposte_blend_in: f64,
    pub blend_in_curve: String,
    pub combo_blend_in_curve: String,
    pub morph_blend_in_curve: String,
    pub riposte_blend_in_curve: String,
    pub blend_out: f64,
    pub feint_anim_duration_offset: f64,
    pub feint_anim_minimum_duration: f64,
    pub feint_anim_rate: f64,
    #[serde(deserialize_with = "de_vec2")]
    pub miss_recovery_play_rate_clamp: Vec2,
    pub miss_recovery_to_play_rate: f64,
    pub successful_hit_play_rate: f64,
    pub successful_hit_blend_out_anim_time: f64,
    // SpineSpaceAdditive targets (UAttackMotion::UpdateSpineSpaceAdditiveTargets rva=0x16417a0)
    pub angling_windup: Hml,
    pub angling_release: Hml,
    pub riposte_angling_windup: Hml,
    pub riposte_angling_release: Hml,
}

/// MotionDefs.Kick (UKickMotion)
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct KickDef {
    pub kick_damage_modifier_tier3_legs: f64,   // +0x1100
    pub jump_kick_stamina_drain: f64,           // +0x1104
    pub jump_kick_extra_windup: f64,            // +0x1108
    pub jump_kick_air_movement_restriction: i64, // +0x110c
    pub max_airborne_time_for_jump_kick_anim: f64, // +0x1110
}

/// MotionDefs.Parry (UParryMotion)
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ParryDef {
    pub parry_up_time: f64, // +0x514
    pub successful_parry_recovery_movement_restriction: i64, // +0x4a4
    pub failed_parry_recovery_movement_restriction: i64,     // +0x4a5
    pub parry_recovery_time: f64,          // +0x500
    pub minimum_held_parry_time: f64,      // +0x4f8
    pub non_held_parry_extension_time: f64, // +0x4b0
    pub minimum_held_riposte_parry_time: f64,
    pub shield_wall_raise_time: f64,
    pub miss_parry_recovery_time: f64,
    pub shield_wall_recovery_time: f64,
    pub held_parry_recovery_time: f64,
    pub held_parry_success_recovery_time: f64,
    pub parry_success_recovery_time: f64,
    pub parry_in_flinch_duration_max: f64, // +0x490
    pub non_held_parry_extension_and_riposte_window_extra: f64,
    pub riposte_window_base: f64,
    pub held_riposte_window_extra: f64,
    pub easy_parry_stamina_cost: f64,
    pub shield_wall_stamina_drain_factor: f64,
    pub easy_parry_duration: f64,
    pub block_stamina_recover: i64,          // +0x518
    pub chamber_ftp_extra_stamina_drain: f64, // +0x46c
    // CheckParry geometry (types/UParryMotion.h; native ctor rva=0x16486b0 + CDO chain)
    // animation-side fields (mh-sim's MotionAnim; motion_defs.gd Parry)
    pub animation: String,
    pub alt_animation: String,
    pub parry_up_time_delay_expected_delay: f64,
    pub shield_wall_raise_time_anim_offset: f64,
    pub b_legacy_animation_playing_method: bool,
    pub parry_fail_play_rate: f64,
    pub held_parry_fail_play_rate: f64,
    pub parry_miss_fade_out: f64,
    pub parry_fail_fade_out: f64,
    pub held_parry_fail_fade_out: f64,
    // FAnglingSpineSpaceAdditive (UParryMotion::OnTick_Implementation rva=0x1668980)
    pub angle_additive_left: Hml,
    pub angle_additive_right: Hml,
    pub max_parry_angle: f64,             // +0x480
    pub max_parry_weapon_angle: f64,      // +0x484
    pub held_block_memory_duration: f64,  // +0x488
    pub timed_block_memory_duration: f64, // +0x48c
}

/// MotionDefs.Feinted (UFeintedMotion)
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FeintedDef {
    #[serde(deserialize_with = "de_vec2")]
    pub strike_and_stab_lockout_in: Vec2,
    #[serde(deserialize_with = "de_vec2")]
    pub strike_and_stab_lockout_out: Vec2,
    pub strike_and_stab_late_feint_adjustment_curve: String,
    pub extra_stab_lockout: f64,
    pub extra_strike_lockout: f64,
    pub slow_kick_duration: f64,
    pub queue_window: f64,
    pub spine_space_additive_blend_out_time: f64,
}

/// MotionDefs.Blocked (UBlockedMotion; offsets in motion_defs.gd, third-person members)
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct BlockedDef {
    #[serde(deserialize_with = "de_vec2")]
    pub parried_recovery_time_limits: Vec2, // +0x120
    pub parried_recovery_time_offset: f64,  // +0x11c
    pub world_miss_stamina_factor: f64,     // +0x194
    pub world_recovery_time: f64,           // +0x190
    #[serde(deserialize_with = "de_vec2")]
    pub chambered_recovery_time_limits: Vec2, // +0x12c
    pub chambered_recovery_time_offset: f64,  // +0x128
    pub queue_window: f64,     // +0xc0
    pub queue_window_hit: f64, // +0xc4
    pub movement_restriction_hit: i64,   // +0xc8
    pub movement_restriction_world: i64, // +0xc9
    pub clash_fade_out_time: f64,        // +0xd0
    pub stab_world_fade_out_time: f64,   // +0xd8
    #[serde(deserialize_with = "de_vec2")]
    pub stab_parry_min_max_range: Vec2, // +0xec
    #[serde(deserialize_with = "de_vec2")]
    pub stab_parry_fade_out_time: Vec2, // +0xf4
    #[serde(deserialize_with = "de_vec2")]
    pub stab_chambered_min_max_range: Vec2, // +0x10c
    #[serde(deserialize_with = "de_vec2")]
    pub stab_chambered_fade_out_time: Vec2, // +0x114
    pub stab_hit_stop_fade_out_time: f64,   // +0x184
    pub kick_hit_stop_blend_out_time: f64,  // +0x188
    pub kick_hit_stop_anim_rate: f64,       // +0x18c
    pub hit_stop_time_until_fade: f64,      // +0x174
    pub hit_stop_bounce_duration: f64,      // +0x178
    pub hit_stop_fade_out_time: f64,        // +0x17c
    pub hit_stop_bounce_curve: String,      // +0x150
    pub hit_stop_bounce_scale_curve: String, // +0x158
    pub hit_stop_release_scale_curve: String, // +0x160
    pub release_scale_curve: String,        // +0x1a0
    pub world_time_until_fade: f64,         // +0x234
    pub world_bounce_duration: f64,         // +0x238
    pub world_fade_out_time: f64,           // +0x23c
    #[serde(deserialize_with = "de_vec2")]
    pub parry_min_max_range: Vec2, // +0x1f4
    #[serde(deserialize_with = "de_vec2")]
    pub parry_time_until_fade: Vec2, // +0x1fc
    #[serde(deserialize_with = "de_vec2")]
    pub parry_bounce_duration: Vec2, // +0x204
    #[serde(deserialize_with = "de_vec2")]
    pub parry_fade_out_time: Vec2, // +0x20c
    #[serde(deserialize_with = "de_vec2")]
    pub chamber_min_max_range: Vec2, // +0x214
    #[serde(deserialize_with = "de_vec2")]
    pub chamber_time_until_fade: Vec2, // +0x21c
    #[serde(deserialize_with = "de_vec2")]
    pub chamber_bounce_duration: Vec2, // +0x224
    #[serde(deserialize_with = "de_vec2")]
    pub chamber_fade_out_time: Vec2, // +0x22c
}

/// MotionDefs.Flinch / Stun / Disarmed
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct HitReactDef {
    pub flinch_duration: f64,              // UFlinchMotion +0xa4
    pub parry_lock_out_time: f64,          // UFlinchMotion +0xa8
    pub stun_duration: f64,                // UStunMotion +0xa8
    pub stun_grace_period_extra_time: f64, // UStunMotion +0xa4
    pub recovery_time: f64,                // UDisarmedMotion +0xa0
}

/// One motion class's defaults: the base + the record of its native class
#[derive(Clone, Debug, Default)]
pub struct MotionDef {
    pub base: MotionBaseDef,
    pub attack: Option<AttackDef>,
    pub kick: Option<KickDef>,
    pub parry: Option<ParryDef>,
    pub feinted: Option<FeintedDef>,
    pub blocked: Option<BlockedDef>,
    pub react: HitReactDef,
}

impl MotionDef {
    pub fn attack(&self) -> &AttackDef {
        self.attack.as_ref().expect("not an attack motion def")
    }
    pub fn parry(&self) -> &ParryDef {
        self.parry.as_ref().expect("not a parry motion def")
    }
}

/// CharacterData.Character: AMordhauCharacter (+ AAdvancedCharacter) class defaults of BP_MordhauCharacter.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Character {
    pub melee_windup_modifier: f64, // +0xe48
    pub melee_combo_extra_windup_modifier: f64,
    pub melee_release_modifier: f64,
    pub melee_miss_recovery_modifier: f64, // ..+0xe54
    pub look_up_limit: f64,                // +0x8a0
    /// AAdvancedCharacter LookDownLimit (CharacterData.look_down_limit; mh-net ReplicatedLookUpValue)
    pub look_down_limit: f64,
    /// +0xe84 KnockbackParry (UParryMotion::ReceiveBlock's knockback; mh-net)
    pub knockback_parry: f64,
    /// +0xe88 KnockbackWorld (UAttackMotion::HandleBlockingHit rva=0x162afa0: ApplyBackwardsKnockbackIfNotInKnockback
    /// on a World block; BP_MordhauCharacter CDO 400; not in the spec matrix, mh-sim spec.rs reads the CDO chain)
    pub knockback_world: f64,
    /// +0xeac DodgeStaminaCost (AMordhauCharacter::OnDodged; mh-net)
    pub dodge_stamina_cost: i64,
    pub stamina_cost_modifier: f64,        // +0xe44
    pub stamina_regen_delay: f64,          // +0xe6c
    pub stamina_regen_per_tick: i64,       // +0xe69
    pub stamina_regen_tick_rate: f64,      // +0xe78
    pub received_ranged_damage_modifier: f64,
    pub received_damage_modifier: f64,      // +0x9f8
    pub received_team_damage_modifier: f64, // +0xa00
    pub received_damage_absorption: f64,    // +0xa0c
    pub received_damage_max: f64,           // +0xa10
    pub received_fall_damage_modifier: f64, // +0x9fc
    pub b_has_last_chance: bool,            // +0x9f0
    pub last_chance_heal_amount: i64,       // +0x9f4
    pub damage_armor_tier_override: i64,    // +0x7f4
    pub b_can_jump_kick: bool,
    pub leg_damage_bonus_modifier_airborne: f64,
    pub extra_stamina_on_hit: i64, // +0xe40
    pub b_is_hit_stop_on_team_hits_disabled: bool,
    pub b_will_stop_melee: bool,
    pub b_cannot_chamber: bool,              // +0xb8d
    pub b_destroy_equipment_on_death: bool,  // +0x11e0
    pub b_always_stun_instead_of_disarm: bool, // +0x1219
    /// +0xf54 BlockColliderForwardParryDistance (ctor 25, 10: AMordhauCharacter.cpp 3015-3016)
    #[serde(deserialize_with = "de_vec2")]
    pub block_collider_forward_parry_distance: Vec2,
}

/// CharacterData.Stat: UStatComponent fields of a stat component's native ctor defaults
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Stat {
    pub min_value: i64,
    pub max_value: i64,
    pub initial_value: i64,
    pub b_is_regenerable: bool,
}

/// CombatData.ModeRules (AMordhauGameMode / AMordhauGameState / UDamageableComponent fields)
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ModeRules {
    pub game_mode: String,
    pub game_state: String,
    pub damage_factor: f64,      // AMordhauGameMode +0x40c
    pub team_damage_factor: f64, // +0x410
    pub team_damage_flinch: i64, // +0x414
    pub spawn_protection_duration: f64, // +0x418
    pub b_disable_damage: bool,  // +0x41c
    pub b_is_hit_stop_on_team_hits_disabled: bool, // +0x390
    pub b_is_team_mode: bool,    // AMordhauGameState +0x6b0
    pub damageable_spawn_protection_duration: f64, // UDamageableComponent +0xe4
}

/// CombatConstants (godot/components/ue/rdata/combat_constants.gd): every exe .rdata literal the combat rules use,
/// by name. The va + citing functions of each are in that file (and checked by test_combat there).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Constants {
    pub small_number: f64,              // 0x144014a88
    pub byte_to_unit: f64,              // 0x1440a4394
    pub byte_max: f64,                  // 0x144014b78
    pub net_lag_unit_s: f64,            // 0x1440e9f84
    pub easy_parry_window: f64,         // 0x143fe4e04
    pub morph_stab_angle_scale: f64,    // 0x143fe4e38
    pub no_weapon_fastest_windup: f64,  // 0x1441231f0
    pub auto_blend_deg_to_rad: f64,     // 0x144022360
    pub auto_blend_angle_to_cm: f64,    // 0x14402cb98
    pub auto_blend_min_blend: f64,      // 0x1442efa58
    pub auto_blend_half: f64,           // 0x143fe4e04
    pub auto_blend_no_blend_yet: f64,   // 0x144014b94
    pub blocked_time_unit_s: f64,       // 0x14402c9a0
    pub blocked_kick_delay_chamber: f64, // 0x1441231e4
    pub blocked_kick_delay_parry: f64,  // 0x1442898a4
    pub blocked_bounce_window: f64,     // 0x1441231f0
    pub blocked_late_bounce_release: f64, // 0x143fe4e08
    pub blocked_bounce_half: f64,       // 0x143fe4e04
    pub blocked_unit_scale: f64,        // 0x143fe4e0c
    pub blocked_no_blend_out: f64,      // 0x144014bb8
    pub blocked_leave_fade: f64,        // 0x143fe4e04
    pub idle_coming_from_timeout: f64,  // 0x143fe4e04
    pub flinch_kick_delay: f64,         // 0x143fe4e00
    pub disarmed_kick_delay: f64,       // 0x1441231e4
    pub flinch_recover_cosmetics: f64,  // 0x1443145ac
    pub parry_open_end: f64,            // 0x144349db0
    pub shield_wall_riposte_window: f64, // 0x143fe4e00
    pub shield_wall_drain_round_bias: f64, // 0x144022404
    pub kick_lag_window: f64,           // 0x1441231e4
    pub kick_windup_extra: f64,         // 0x1441231e4
    pub max_ping_compensation: f64,     // 0x144331154
    pub net_lag_units_per_s: f64,       // 0x1442efaa4
    pub attack_angle_min: f64,          // 0x1443247d4
    pub attack_angle_max: f64,          // 0x143fe4e38
    pub attack_angle_to_unit: f64,      // 0x14428988c
    pub blocked_time_to_byte: f64,      // 0x144331164
    pub flinch_angle_to_unit: f64,      // 0x144014a90
    pub min_damage: f64,                // 0x143fe4e0c
    pub spawn_damage_scale: f64,        // 0x14432475c
    pub friendly_spawn_window: f64,     // 0x143fe4e20
    pub spawn_protection_max_damage: f64, // 0x144349dac
    pub tracer_length_per_cm: f64,      // 0x1440701e0
    pub tracer_count_scale: f64,        // 0x144362ca8
    pub tracer_round_bias: f64,         // 0x143fe4e04
    pub tracer_spacing_cm: f64,         // 0x144362cc0
    pub tracer_step: f64,               // 0x144014bb8
    pub curve_third: f64,               // 0x14406a414 (FRichCurve Bezier 1/3, engine curve code)
    pub head_bones: Vec<String>,        // NAME_Head (string va 0x144317d48)
    pub right_leg_bones: Vec<String>,   // 0x144317ba0, 0x144317b90, 0x144317bb0
    pub left_leg_bones: Vec<String>,    // 0x144317bd0, 0x144317bc0, 0x144317bd8
    pub kick_tier_bone: String,         // NAME_RightLeg (0x144317ba0)
}

/// A UCurveFloat (FloatCurve keys + extrapolation, CombatData.curve_keys / curve_extrap)
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Curve {
    pub keys: Vec<CurveKey>,
    pub pre: String,
    pub post: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct CurveKey {
    #[serde(rename = "InterpMode")]
    pub interp_mode: Option<String>,
    #[serde(rename = "TangentWeightMode")]
    pub tangent_weight_mode: Option<String>,
    #[serde(rename = "TangentMode")]
    pub tangent_mode: Option<String>,
    #[serde(rename = "Time")]
    pub time: f64,
    #[serde(rename = "Value")]
    pub value: f64,
    #[serde(rename = "ArriveTangent")]
    pub arrive_tangent: f64,
    #[serde(rename = "ArriveTangentWeight")]
    pub arrive_tangent_weight: f64,
    #[serde(rename = "LeaveTangent")]
    pub leave_tangent: f64,
    #[serde(rename = "LeaveTangentWeight")]
    pub leave_tangent_weight: f64,
}

/// BP_MordhauCharacter BlockCollider (CombatData.block_collider), Godot metres, actor frame
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockCollider {
    pub half: crate::ue::FVector,
    pub centre: crate::ue::FVector,
}

/// Everything the combat core reads, resolved.
#[derive(Clone, Debug, Default)]
pub struct Spec {
    pub constants: Constants,
    pub character: Rc<Character>,
    pub stats: HashMap<String, Stat>,
    pub kick_weapon_path: String,
    pub weapons: HashMap<String, WeaponSetup>,
    pub profiles: HashMap<String, ProfileMotions>,
    pub motion_defs: HashMap<String, Rc<MotionDef>>,
    pub curves: HashMap<String, Curve>,
    /// Explicit mod-authored keys; absent overrides preserve the original asset lookup/evaluator.
    pub curve_overrides: HashMap<String, Curve>,
    pub class_chains: HashMap<String, Vec<String>>,
    pub block_collider: BlockCollider,
    pub mode_rules: HashMap<String, ModeRules>,
    /// UePhysics.body_boxes(): the character physics asset's boxes (bone, bone-space transform, half extent).
    /// r1: not exported (golden scenarios use scripted contacts); empty = traces hit no body. TODO r2: export.
    pub body_boxes: Vec<BodyBox>,
}

#[derive(Clone, Debug)]
pub struct BodyBox {
    pub bone: String,
    pub xf: crate::ue::Xform,
    pub half: crate::ue::FVector,
}

/// CombatData.FEINTED_MOTION etc.: the motion classes of BP_MordhauCharacter's Motions table (CDO entries)
pub const MOTIONS: &str = "Mordhau/Content/Mordhau/Blueprints/Motions/";
pub fn feinted_motion() -> String {
    format!("{MOTIONS}BP_FeintedMotion")
}
pub fn blocked_motion() -> String {
    format!("{MOTIONS}BP_BlockedMotion")
}
pub fn flinch_motion() -> String {
    format!("{MOTIONS}BP_FlinchMotion")
}
pub fn stun_motion() -> String {
    format!("{MOTIONS}BP_StunMotion")
}
pub fn disarmed_motion() -> String {
    format!("{MOTIONS}BP_DisarmedMotion")
}

impl Spec {
    pub fn weapon_setup(&self, path: &str) -> &WeaponSetup {
        self.weapons.get(path).unwrap_or_else(|| panic!("spec: no weapon {path}"))
    }
    pub fn weapon(&self, path: &str) -> Option<Rc<WeaponData>> {
        self.weapons.get(path).map(|s| s.weapon.clone())
    }
    pub fn equip(&self, path: &str) -> Option<Rc<EquipmentDef>> {
        self.weapons.get(path).and_then(|s| s.equip.clone())
    }
    pub fn motion_def(&self, key: &str) -> Rc<MotionDef> {
        self.motion_defs.get(key).cloned().unwrap_or_else(|| panic!("spec: no motion def {key}"))
    }
    pub fn native_motion_def(&self, cls: &str) -> Rc<MotionDef> {
        self.motion_def(&format!("native:{cls}"))
    }
    pub fn profile_motions(&self, profile: &str) -> ProfileMotions {
        if profile.is_empty() {
            return ProfileMotions::default();
        }
        self.profiles.get(profile).cloned().unwrap_or_else(|| panic!("spec: no profile {profile}"))
    }
    pub fn stat(&self, cls: &str) -> &Stat {
        self.stats.get(cls).unwrap_or_else(|| panic!("spec: no stat {cls}"))
    }
    /// CombatData.is_class_of: walks the PDB base chain (NativeCtor.layout(cls).base)
    pub fn is_class_of(&self, cls: &str, base: &str) -> bool {
        if cls == base {
            return true;
        }
        self.class_chains.get(cls).map(|c| c.iter().any(|x| x == base)).unwrap_or(false)
    }

    /// UCurveFloat::GetFloatValue (rva 0x3026f70) on a curve package: CombatData.curve_value
    pub fn curve_value(&self, path: &str, t: f64) -> f64 {
        let c = self.curve_overrides.get(path).or_else(|| self.curves.get(path)).unwrap_or_else(|| panic!("spec: no curve {path}"));
        self.curve_eval(&c.keys, t, &c.pre, &c.post)
    }

    /// FRichCurve::Eval (rva 0x3024860) + EvalForTwoKeys (rva 0x3024fa0), as CombatData.curve_eval ports them:
    /// pre/post linear extrapolation through the two end keys, binary search, constant/linear/cubic (unweighted
    /// Bezier with the 1/3 of _DAT_14406a414) interpolation.
    pub fn curve_eval(&self, keys: &[CurveKey], t: f64, pre: &str, post: &str) -> f64 {
        let n = keys.len();
        if n == 0 {
            return 0.0;
        }
        let small = self.constants.small_number; // UeRdata.f32(0x144014a88)
        let first = &keys[0];
        if n < 2 || t <= first.time {
            if pre == "RCCE_Linear" && n > 1 {
                let d = keys[1].time - first.time;
                if d.abs() > small {
                    return (keys[1].value - first.value) / d * (t - first.time) + first.value;
                }
            }
            return first.value;
        }
        let last = &keys[n - 1];
        if t >= last.time {
            if post == "RCCE_Linear" {
                let d2 = keys[n - 2].time - last.time;
                if d2.abs() > small {
                    return (keys[n - 2].value - last.value) / d2 * (t - last.time) + last.value;
                }
            }
            return last.value;
        }
        let mut lo = 1usize;
        let mut cnt = n as i64 - 2;
        while cnt > 0 {
            let half = cnt >> 1;
            if t >= keys[lo + half as usize].time {
                lo = lo + half as usize + 1;
                cnt -= half + 1;
            } else {
                cnt = half;
            }
        }
        self.two_keys(&keys[lo - 1], &keys[lo], t)
    }

    fn two_keys(&self, a: &CurveKey, b: &CurveKey, t: f64) -> f64 {
        use crate::ue::lerpf;
        let dt = b.time - a.time;
        // CombatData._INTERP: the raw InterpMode string -> RCIM_Linear 0 / Constant 1 / Cubic 2 (absent = Linear)
        let mode = match a.interp_mode.as_deref().unwrap_or("RCIM_Linear") {
            "RCIM_Constant" => 1,
            "RCIM_Cubic" => 2,
            _ => 0,
        };
        if dt <= 0.0 || mode == 1 {
            return a.value;
        }
        let alpha = (t - a.time) / dt;
        if mode == 0 {
            return (b.value - a.value) * alpha + a.value;
        }
        let wa = a.tangent_weight_mode.as_deref().unwrap_or("RCTWM_WeightedNone");
        let wb = b.tangent_weight_mode.as_deref().unwrap_or("RCTWM_WeightedNone");
        if !(matches!(wa, "RCTWM_WeightedNone" | "RCTWM_WeightedArrive") && matches!(wb, "RCTWM_WeightedNone" | "RCTWM_WeightedLeave")) {
            return f64::NAN; // weighted tangents not ported (reference push_error + NAN)
        }
        let third = self.constants.curve_third;
        let p0 = a.value;
        let p1 = p0 + a.leave_tangent * dt * third;
        let p3 = b.value;
        let p2 = p3 - b.arrive_tangent * dt * third;
        let p01 = lerpf(p0, p1, alpha);
        let p12 = lerpf(p1, p2, alpha);
        let p23 = lerpf(p2, p3, alpha);
        lerpf(lerpf(p01, p12, alpha), lerpf(p12, p23, alpha), alpha)
    }
}

fn de_vec2<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec2, D::Error> {
    let v: Vec<f64> = Vec::deserialize(d)?;
    Ok(Vec2::new(v.first().copied().unwrap_or(0.0) as f32, v.get(1).copied().unwrap_or(0.0) as f32))
}

fn vec3(v: &Value) -> Option<crate::ue::FVector> {
    let a = v.as_array()?;
    let g = |i: usize| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    Some(crate::ue::FVector::new(g(0), g(1), g(2)))
}

fn from<T: for<'de> Deserialize<'de>>(v: &Value, what: &str) -> Result<T, String> {
    T::deserialize(v).map_err(|e| format!("spec {what}: {e}"))
}

/// The record dump of godot/tools/export_golden.gd (core/tests/golden/spec.json; schema in docs/RUST_CORE.md), with
/// the reference's values (f64: package floats as the decimal JSON double, ctor floats as their f32).
pub struct RecordsJson<'a>(pub &'a str);

/// The same dump with every number rounded to binary32: the values the exe holds (every UE float field is an f32;
/// round(f64 of the package decimal) = the package's f32, so this equals the mh-spec matrix, which stores each UE
/// float as its f32 - cross-checked by mh-sim's spec test). Use with `World::new` (exe-exact).
pub struct RecordsJsonExe<'a>(pub &'a str);

impl SpecSource for RecordsJsonExe<'_> {
    fn load_spec(&self) -> Result<Spec, String> {
        let mut root: Value = serde_json::from_str(self.0).map_err(|e| format!("spec json: {e}"))?;
        decode_exact(&mut root);
        round_f32(&mut root);
        load_records(root)
    }
}

/// every JSON float -> its binary32 value (integers stay integers)
pub fn round_f32(v: &mut Value) {
    match v {
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap() as f32 as f64;
            *v = serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null);
        }
        Value::Array(a) => a.iter_mut().for_each(round_f32),
        Value::Object(o) => o.values_mut().for_each(round_f32),
        _ => {}
    }
}

impl SpecSource for RecordsJson<'_> {
    fn load_spec(&self) -> Result<Spec, String> {
        let mut root: Value = serde_json::from_str(self.0).map_err(|e| format!("spec json: {e}"))?;
        decode_exact(&mut root);
        load_records(root)
    }
}

/// Spec from a decoded records tree (RecordsJson / RecordsJsonExe; also the mh-sim adapter's output format)
pub fn load_records(root: Value) -> Result<Spec, String> {
    {
        let mut s = Spec {
            constants: from(&root["constants"], "constants")?,
            character: Rc::new(from(&root["character"], "character")?),
            kick_weapon_path: root["kick_weapon_path"].as_str().unwrap_or("").to_string(),
            ..Default::default()
        };
        for (k, v) in root["stats"].as_object().into_iter().flatten() {
            s.stats.insert(k.clone(), from(v, k)?);
        }
        for (k, v) in root["profiles"].as_object().into_iter().flatten() {
            s.profiles.insert(k.clone(), profile(k, v));
        }
        for (k, v) in root["weapons"].as_object().into_iter().flatten() {
            let weapon: WeaponData = from(&v["weapon"], k)?;
            let equip: Option<EquipmentDef> = if v["equip"].is_null() { None } else { Some(from(&v["equip"], k)?) };
            let tr = &v["tracer"];
            s.weapons.insert(
                k.clone(),
                WeaponSetup {
                    weapon: Rc::new(weapon),
                    equip: equip.map(Rc::new),
                    motions: profile("", &v["profile_motions"]).motions,
                    tracer: TracerDef { start: vec3(&tr["start"]), end: vec3(&tr["end"]) },
                },
            );
        }
        for (k, v) in root["motion_defs"].as_object().into_iter().flatten() {
            let base: MotionBaseDef = from(v, k)?;
            let rec = base.rec.clone();
            let mut d = MotionDef { base, react: from(v, k)?, ..Default::default() };
            match rec.as_str() {
                "Attack" => d.attack = Some(from(v, k)?),
                "Kick" => {
                    d.attack = Some(from(v, k)?);
                    d.kick = Some(from(v, k)?);
                }
                "Parry" => d.parry = Some(from(v, k)?),
                "Feinted" => d.feinted = Some(from(v, k)?),
                "Blocked" => d.blocked = Some(from(v, k)?),
                _ => {}
            }
            s.motion_defs.insert(k.clone(), Rc::new(d));
        }
        for (k, v) in root["curves"].as_object().into_iter().flatten() {
            s.curves.insert(
                k.clone(),
                Curve {
                    keys: from(&v["keys"], k)?,
                    pre: v["pre"].as_str().unwrap_or("RCCE_Constant").into(),
                    post: v["post"].as_str().unwrap_or("RCCE_Constant").into(),
                },
            );
        }
        for (k, v) in root["class_chains"].as_object().into_iter().flatten() {
            s.class_chains.insert(k.clone(), from(v, k)?);
        }
        let bc = &root["block_collider"];
        s.block_collider = BlockCollider { half: vec3(&bc["half"]).unwrap_or_default(), centre: vec3(&bc["centre"]).unwrap_or_default() };
        for (k, v) in root["mode_rules"].as_object().into_iter().flatten() {
            s.mode_rules.insert(k.clone(), from(v, k)?);
        }
        Ok(s)
    }
}

/// export_golden.gd writes every float as "f64:<16 hex>" (its 8 IEEE-754 bytes, little-endian) because Godot's JSON
/// keeps only ~15 significant digits; this turns them back into numbers, bit for bit.
pub fn decode_exact(v: &mut Value) {
    match v {
        Value::String(s) if s.starts_with("f64:") && s.len() == 20 => {
            let mut b = [0u8; 8];
            for (i, x) in b.iter_mut().enumerate() {
                *x = u8::from_str_radix(&s[4 + 2 * i..6 + 2 * i], 16).unwrap_or(0);
            }
            let f = f64::from_le_bytes(b);
            *v = serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null);
        }
        Value::Array(a) => a.iter_mut().for_each(decode_exact),
        Value::Object(o) => o.values_mut().for_each(decode_exact),
        _ => {}
    }
}

fn profile(name: &str, v: &Value) -> ProfileMotions {
    let mut p = ProfileMotions { profile: name.to_string(), ..Default::default() };
    for (mk, bp) in v["motions"].as_object().into_iter().flatten() {
        if let (Ok(m), Some(b)) = (mk.parse::<i64>(), bp.as_str()) {
            p.motions.insert(m, b.to_string());
        }
    }
    p.parry_motion = v["parry_motion"].as_str().unwrap_or("").to_string();
    p
}

//! Horse, projectile and ladder-mover records (sheets r10): mh-character's exe-mode loaders as the spec's readers,
//! and the same records built back from the spec matrix for the hosts.
//!
//!   dump(json_root)        the spec_src dump godot/data_gen/spec_src/character_exe.json (bin mh-spec-src): mh-character's
//!                          own readers over extract/json (the game's packages, CUE4Parse export lists):
//!                            horse      HorseCfg::from_json (exe_horse.rs; BP_Horse + BP_VehicleHorse + FC_ curves), the
//!                                       horse_character_records overrides (UCharacterMovementComponent rva=0x2f6cc10 /
//!                                       UAdvancedCharacterMovement rva=0x144da20 ctors under BP_Horse CharMoveComp) and
//!                                       ExeMovement::new_horse's capsule / BrakingDecelerationWalking
//!                            projectiles ProjectileCfg::from_json_chain (projectile.rs) for every Blueprint class whose
//!                                       Super chain reaches BP_MordhauProjectile
//!                            ladder     the BP_LadderMover mover ExeMovement::new_ladder_mover sets (exe_ladder.rs):
//!                                       CharMoveComp / CollisionCylinder over the UOneDimensionalMovementComponent ctor
//!                                       rva=0x14afe70 and the UCharacterMovementComponent ctor rva=0x2f6cc10, plus
//!                                       BP_VehicleLadderMover's CDO
//!   horse_cfg / horse_movement / projectile_cfg / projectile_classes / ladder_mover
//!                          the records from the matrix (ENT_HORSE_*, ENT_PROJ_*, ENT_LADDER_*); tests/exe_records.rs proves
//!                          each equal (f32 bits) to the loader over extract/json
//! Field names are mh-character's struct field names (HorseCfg, GearInfo, ProjectileCfg, LadderState / ExeMovement);
//! the CharacterRecords overrides carry their record group as a prefix (movement_ / move_extra_ / character_).

use mh_character::exe_horse::{horse_character_records, GearInfo, HorseCfg};
use mh_character::equipment::EquipmentMovement;
use mh_character::projectile::ProjectileCfg;
use mh_character::uequat::RichKey;
use mh_character::ue::FVector;
use mh_character::CharacterRecords;
use serde_json::{json, Map, Value};
use std::path::Path;

use crate::Result;

pub const BP_HORSE: &str = "Mordhau/Content/Mordhau/Blueprints/Interactables/Animals/BP_Horse";
pub const BP_VEHICLE_HORSE: &str = "Mordhau/Content/Mordhau/Blueprints/VehicleComponents/BP_VehicleHorse";
pub const BP_LADDER_MOVER: &str = "Mordhau/Content/Mordhau/Blueprints/Interactables/SiegeEngines/BP_LadderMover";
pub const BP_VEHICLE_LADDER_MOVER: &str = "Mordhau/Content/Mordhau/Blueprints/VehicleComponents/BP_VehicleLadderMover";
pub const BP_MORDHAU_PROJECTILE: &str = "Mordhau/Content/Mordhau/Blueprints/BP_MordhauProjectile";

/// HorseCfg's curve fields and the property each is read from (exe_horse.rs HorseCfg::from_json)
pub const HORSE_CURVES: [(&str, &str, &str); 4] = [
    ("turning_brake_curve", "CharMoveComp", "TurningBrakeAccelerationByVelocity"),
    ("turning_factor_curve", "CharMoveComp", "TurningFactorByVelocity"),
    ("turning_accel_curve", "CharMoveComp", "TurningAccelerationByVelocity"),
    ("bump_damage_curve", "Default__BP_Horse_C", "BumpDamageBySpeedModifierCurve"),
];

/// The ladder mover's values (what ExeMovement::new_ladder_mover sets; exe_ladder.rs)
#[derive(Clone, Debug, PartialEq)]
pub struct LadderMoverCfg {
    pub max_walk_speed: f32,               // BP_LadderMover CharMoveComp MaxWalkSpeed
    pub max_acceleration: f32,             // UOneDimensionalMovementComponent ctor rva=0x14afe70 (10000)
    pub braking_deceleration_walking: f32, // same ctor (10000)
    pub gravity_scale: f32,                // CharMoveComp GravityScale
    pub ground_friction: f32,              // UCharacterMovementComponent ctor 0x142f6cfff (8)
    pub braking_friction_factor: f32,      // UCharacterMovementComponent ctor 0x142f6d1ba (2)
    pub capsule_radius: f32,               // CollisionCylinder CapsuleRadius
    pub capsule_half_height: f32,          // CollisionCylinder CapsuleHalfHeight
    pub in_steps: bool,                    // CharMoveComp bIsMovementInSteps (+0xc48)
    pub use_driver_speed: bool,            // +0xc49 bUseDriverSpeedFactor (ctor true; BP does not set it)
    pub step_size: f32,                    // CharMoveComp StepSize (+0xc50)
    pub secondary_turn_limit: f32,         // BP_VehicleLadderMover SecondaryTurnLimit
    pub use_driver_turn_caps: bool,        // BP_VehicleLadderMover bUseDriverTurnCaps
    pub is_ladder: bool,                   // BP_VehicleLadderMover bIsLadder
}

// ---- reading the packages ------------------------------------------------------------------------------------------

fn read(root: &Path, pkg: &str) -> Result<String> {
    std::fs::read_to_string(root.join(format!("{pkg}.json"))).map_err(|e| format!("{pkg}.json: {e}"))
}
fn parse(text: &str, what: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|e| format!("{what}: {e}"))
}
fn named<'a>(exports: &'a Value, name: &str) -> &'a Value {
    exports.as_array().and_then(|a| a.iter().find(|e| e["Name"] == name)).map_or(&Value::Null, |e| &e["Properties"])
}
fn cdo(exports: &Value) -> &Value {
    exports
        .as_array()
        .and_then(|a| a.iter().find(|e| e["Name"].as_str().is_some_and(|n| n.starts_with("Default__"))))
        .map_or(&Value::Null, |e| &e["Properties"])
}
/// the package path of an object reference ({ObjectPath: "<pkg>.<index>"}), as exe_horse.rs ref_path reads it
fn ref_path(v: &Value, k: &str) -> Option<String> {
    let p = v.get(k)?.get("ObjectPath")?.as_str()?;
    Some(p.rsplit_once('.').map_or(p, |(a, _)| a).to_string())
}
fn f(v: &Value, k: &str, d: f32) -> f32 {
    v.get(k).and_then(Value::as_f64).map_or(d, |x| x as f32)
}
fn b(v: &Value, k: &str, d: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn n(x: f32) -> Value {
    json!(x as f64)
}
fn vec3(x: FVector) -> Value {
    json!([x.x as f64, x.y as f64, x.z as f64])
}

/// the class's Super package (BlueprintGeneratedClass export's Super.ObjectPath)
fn super_pkg(exports: &Value) -> Option<String> {
    let c = exports.as_array()?.iter().find(|e| e["Type"] == "BlueprintGeneratedClass")?;
    ref_path(c, "Super")
}

// ---- the dump (godot/data_gen/spec_src/character_exe.json) --------------------------------------------------------

fn horse_cfg_json(c: &HorseCfg) -> Value {
    let gears: Vec<Value> = c
        .gears
        .iter()
        .map(|g| {
            json!({"max_speed": n(g.max_speed), "max_acceleration": n(g.max_acceleration), "allow_jump": g.allow_jump,
                   "can_rider_regen_health": g.can_rider_regen_health, "can_rider_regen_stamina": g.can_rider_regen_stamina,
                   "can_horse_regen": g.can_horse_regen})
        })
        .collect();
    json!({
        "gears": gears,
        "turning_factor_scale_airborne": n(c.turning_factor_scale_airborne),
        "head_on_min_speed_to_rear": n(c.head_on_min_speed_to_rear),
        "soft_bubble_rel": vec3(c.soft_bubble_rel),
        "soft_bubble_length": n(c.soft_bubble_length),
        "soft_bubble_radius": n(c.soft_bubble_radius),
        "soft_bubble_max_height": n(c.soft_bubble_max_height),
        "front_rear_half_height": n(c.front_rear_half_height),
        "front_rear_radius": n(c.front_rear_radius),
        "front_rel": vec3(c.front_rel),
        "rear_rel": vec3(c.rear_rel),
        "avoidance_turning_acceleration": n(c.avoidance_turning_acceleration),
        "speed_multiplier_on_bump": n(c.speed_multiplier_on_bump),
        "speed_multiplier_on_melee": n(c.speed_multiplier_on_melee),
        "knockback_force": n(c.knockback_force),
        "knockback_force_velocity_factor": n(c.knockback_force_velocity_factor),
        "knockback_damage": n(c.knockback_damage),
        "rearing_duration": n(c.rearing_duration),
        "uncontrolled_gear": c.uncontrolled_gear,
        "attach_offset": vec3(c.attach_offset),
        "min_xy_distance_to_enter": n(c.min_xy_distance_to_enter),
        "min_z_distance_to_enter": [c.min_z_distance_to_enter.0 as f64, c.min_z_distance_to_enter.1 as f64],
        "minimum_interactable_velocity": n(c.minimum_interactable_velocity),
        "mesh_relative": vec3(c.mesh_relative),
    })
}

/// the CharacterRecords fields horse_character_records overrides (exe_horse.rs), group-prefixed
fn horse_movement_json(r: &CharacterRecords) -> Value {
    let m = &r.movement;
    json!({
        "movement_gravity_scale": m.gravity_scale, "movement_ground_friction": m.ground_friction,
        "movement_jump_z_velocity": m.jump_z_velocity, "movement_walkable_floor_z": m.walkable_floor_z,
        "movement_max_step_height": m.max_step_height, "movement_perch_radius_threshold": m.perch_radius_threshold,
        "movement_perch_additional_height": m.perch_additional_height, "movement_max_walk_speed": m.max_walk_speed,
        "movement_max_walk_speed_crouched": m.max_walk_speed_crouched, "movement_air_control": m.air_control,
        "movement_air_control_boost_multiplier": m.air_control_boost_multiplier,
        "movement_air_control_boost_velocity_threshold": m.air_control_boost_velocity_threshold,
        "movement_max_acceleration": m.max_acceleration, "movement_braking_friction_factor": m.braking_friction_factor,
        "movement_braking_deceleration_falling": m.braking_deceleration_falling,
        "movement_falling_lateral_friction": m.falling_lateral_friction,
        "movement_crouched_half_height": m.crouched_half_height,
        "movement_b_can_walk_off_ledges_when_crouching": m.b_can_walk_off_ledges_when_crouching,
        "move_extra_min_velocity_for_fall_damage": r.move_extra.min_velocity_for_fall_damage,
        "move_extra_fall_damage_offset": r.move_extra.fall_damage_offset,
        "move_extra_fall_damage_factor": r.move_extra.fall_damage_factor,
        "character_jump_cooldown": r.character.jump_cooldown,
    })
}

fn projectile_json(c: &ProjectileCfg) -> Value {
    let fl = |v: &[f32]| v.iter().map(|x| *x as f64).collect::<Vec<_>>();
    json!({
        "class": c.class, "initial_speed": n(c.initial_speed), "max_speed": n(c.max_speed), "gravity_scale": n(c.gravity_scale),
        "rotation_follows_velocity": c.rotation_follows_velocity, "should_bounce": c.should_bounce,
        "sweep_collision": c.sweep_collision, "force_sub_stepping": c.force_sub_stepping, "bounciness": n(c.bounciness),
        "friction": n(c.friction), "bounce_stop_threshold": n(c.bounce_stop_threshold),
        "max_simulation_time_step": n(c.max_simulation_time_step), "max_simulation_iterations": c.max_simulation_iterations,
        "bounce_additional_iterations": c.bounce_additional_iterations, "drag_deceleration": n(c.drag_deceleration),
        "box_extent": vec3(c.box_extent), "rotation_spin": vec3(c.rotation_spin), "will_sticky_on": c.will_sticky_on,
        "will_pass_through_on": c.will_pass_through_on, "sticky_surface_pitch_blend": n(c.sticky_surface_pitch_blend),
        "sticky_surface_yaw_blend": n(c.sticky_surface_yaw_blend), "destroy_when_terminated": c.destroy_when_terminated,
        "path_blend_duration": n(c.path_blend_duration), "damage": fl(&c.damage), "head_bonus": fl(&c.head_bonus),
        "leg_bonus": fl(&c.leg_bonus), "initial_velocity": vec3(c.initial_velocity),
        "box_responses": c.box_responses.iter().map(|(ch, r)| json!([ch, r])).collect::<Vec<_>>(),
    })
}

fn ladder_json(c: &LadderMoverCfg) -> Value {
    json!({
        "max_walk_speed": n(c.max_walk_speed), "max_acceleration": n(c.max_acceleration),
        "braking_deceleration_walking": n(c.braking_deceleration_walking), "gravity_scale": n(c.gravity_scale),
        "ground_friction": n(c.ground_friction), "braking_friction_factor": n(c.braking_friction_factor),
        "capsule_radius": n(c.capsule_radius), "capsule_half_height": n(c.capsule_half_height), "in_steps": c.in_steps,
        "use_driver_speed": c.use_driver_speed, "step_size": n(c.step_size), "secondary_turn_limit": n(c.secondary_turn_limit),
        "use_driver_turn_caps": c.use_driver_turn_caps, "is_ladder": c.is_ladder,
    })
}

/// HorseCfg::from_json over the packages under `root` (extract/json)
pub fn horse_cfg_json_root(root: &Path) -> Result<HorseCfg> {
    let bp = read(root, BP_HORSE)?;
    let veh = read(root, BP_VEHICLE_HORSE).ok();
    HorseCfg::from_json(&bp, veh.as_deref(), &|p: &str| read(root, p).ok())
}

/// horse_character_records' overrides over a default base (only the overridden fields are dumped)
pub fn horse_records_json_root(root: &Path) -> Result<CharacterRecords> {
    horse_character_records(&CharacterRecords::default(), &read(root, BP_HORSE)?)
}

/// ExeMovement::new_horse's pawn values: BP_Horse CollisionCylinder (defaults 34 / 88: the ACharacter ctor, as
/// exe_horse.rs reads them) and CharMoveComp BrakingDecelerationWalking (UCMC ctor 2048)
pub fn horse_pawn_json_root(root: &Path) -> Result<(f32, f32, f32)> {
    let ex = parse(&read(root, BP_HORSE)?, "BP_Horse")?;
    let cyl = named(&ex, "CollisionCylinder");
    let cmc = named(&ex, "CharMoveComp");
    Ok((f(cyl, "CapsuleRadius", 34.0), f(cyl, "CapsuleHalfHeight", 88.0), f(cmc, "BrakingDecelerationWalking", 2048.0)))
}

/// the Blueprint chain (most derived first) of a projectile package, if it reaches BP_MordhauProjectile
pub fn projectile_chain(root: &Path, pkg: &str) -> Option<Vec<String>> {
    let mut chain = vec![pkg.to_string()];
    let mut cur = pkg.to_string();
    while cur != BP_MORDHAU_PROJECTILE {
        let ex = parse(&read(root, &cur).ok()?, &cur).ok()?;
        cur = super_pkg(&ex)?;
        if chain.len() > 16 {
            return None;
        }
        chain.push(cur.clone());
    }
    Some(chain)
}

/// every projectile class package under `root` (a Blueprint whose name holds "Projectile" and whose chain reaches
/// BP_MordhauProjectile), sorted, with its chain
pub fn projectile_classes_json_root(root: &Path) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("Mordhau/Content/Mordhau/Blueprints")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "json") && p.file_stem().is_some_and(|s| s.to_string_lossy().contains("Projectile")) {
                let rel = p.strip_prefix(root).unwrap().with_extension("").to_string_lossy().replace('\\', "/");
                if let Some(c) = projectile_chain(root, &rel) {
                    out.push((rel, c));
                }
            }
        }
    }
    out.sort();
    out
}

pub fn projectile_cfg_json_root(root: &Path, chain: &[String]) -> Result<ProjectileCfg> {
    let texts: Vec<String> = chain.iter().map(|p| read(root, p)).collect::<Result<_>>()?;
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    ProjectileCfg::from_json_chain(&refs)
}

/// BP_LadderMover / BP_VehicleLadderMover over the ctors (exe_ladder.rs new_ladder_mover cites them)
pub fn ladder_mover_json_root(root: &Path) -> Result<LadderMoverCfg> {
    let ex = parse(&read(root, BP_LADDER_MOVER)?, "BP_LadderMover")?;
    let veh = parse(&read(root, BP_VEHICLE_LADDER_MOVER)?, "BP_VehicleLadderMover")?;
    let cmc = named(&ex, "CharMoveComp");
    let cyl = named(&ex, "CollisionCylinder");
    let vc = cdo(&veh);
    Ok(LadderMoverCfg {
        max_walk_speed: f(cmc, "MaxWalkSpeed", 600.0), // UCMC ctor 600
        max_acceleration: f(cmc, "MaxAcceleration", 10000.0),
        braking_deceleration_walking: f(cmc, "BrakingDecelerationWalking", 10000.0),
        gravity_scale: f(cmc, "GravityScale", 1.0),
        ground_friction: f(cmc, "GroundFriction", 8.0),
        braking_friction_factor: f(cmc, "BrakingFrictionFactor", 2.0),
        capsule_radius: f(cyl, "CapsuleRadius", 34.0),
        capsule_half_height: f(cyl, "CapsuleHalfHeight", 88.0),
        in_steps: b(cmc, "bIsMovementInSteps", false),
        use_driver_speed: b(cmc, "bUseDriverSpeedFactor", true),
        step_size: f(cmc, "StepSize", 0.0), // UNCONFIRMED ctor default (BP_LadderMover sets it)
        secondary_turn_limit: f(vc, "SecondaryTurnLimit", 0.0), // UNCONFIRMED ctor default (BP sets it)
        use_driver_turn_caps: b(vc, "bUseDriverTurnCaps", false),
        is_ladder: b(vc, "bIsLadder", false),
    })
}

// ---- equipment movement (mh-character equipment.rs) ------------------------------------------------------------------

/// the native class a Blueprint chain's root derives from (SuperStruct of its BlueprintGeneratedClass: Class'X' under
/// /Script/..), e.g. "MordhauEquipment"
fn native_super(exports: &Value) -> Option<String> {
    let c = exports.as_array()?.iter().find(|e| e["Type"] == "BlueprintGeneratedClass")?;
    let s = c.get("SuperStruct")?;
    if !s.get("ObjectPath")?.as_str()?.starts_with("/Script/") {
        return None;
    }
    let n = s.get("ObjectName")?.as_str()?;
    Some(n.split('\'').nth(1)?.to_string())
}

/// does native class `name` (no A prefix) derive from AMordhauEquipment? Read off extract/native/types/A<name>.h
/// (`class A<name> : public <Base>`, the PDB class records)
fn derives_from_equipment(types: &Path, name: &str) -> bool {
    let mut cur = format!("A{name}");
    for _ in 0..16 {
        if cur == "AMordhauEquipment" {
            return true;
        }
        let Ok(t) = std::fs::read_to_string(types.join(format!("{cur}.h"))) else { return false };
        let head = format!("class {cur} : public ");
        let Some(base) = t.lines().find_map(|l| l.strip_prefix(head.as_str()).map(|r| r.split_whitespace().next().unwrap_or("").to_string())) else {
            return false;
        };
        cur = base;
    }
    false
}

/// every Blueprint class under Blueprints/Equipment whose chain's native root derives from AMordhauEquipment, with its
/// Blueprint chain (most derived first) and that native root; sorted
pub fn equipment_classes_json_root(root: &Path) -> Vec<(String, Vec<String>, String)> {
    let types = root.join("../native/types");
    let mut out = Vec::new();
    let mut stack = vec![root.join("Mordhau/Content/Mordhau/Blueprints/Equipment")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if !p.extension().is_some_and(|x| x == "json") {
                continue;
            }
            let rel = p.strip_prefix(root).unwrap().with_extension("").to_string_lossy().replace('\\', "/");
            let mut chain = vec![rel.clone()];
            let mut native = None;
            for _ in 0..16 {
                let Some(ex) = read(root, chain.last().unwrap()).ok().and_then(|t| parse(&t, "equipment").ok()) else { break };
                match super_pkg(&ex) {
                    Some(sp) if sp.starts_with("Mordhau/Content") => chain.push(sp),
                    _ => {
                        native = native_super(&ex);
                        break;
                    }
                }
            }
            if let Some(n) = native {
                if derives_from_equipment(&types, &n) {
                    out.push((rel, chain, n));
                }
            }
        }
    }
    out.sort();
    out
}

pub fn equipment_movement_json_root(root: &Path, chain: &[String]) -> Result<EquipmentMovement> {
    let texts: Vec<String> = chain.iter().map(|p| read(root, p)).collect::<Result<_>>()?;
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    EquipmentMovement::from_json_chain(&refs)
}

fn equipment_json(c: &EquipmentMovement) -> Value {
    json!({
        "class": c.class, "movement_restriction": c.movement_restriction,
        "sub_sprint_speed_bonus_equipped": n(c.sub_sprint_speed_bonus_equipped),
        "speed_override_equipped": n(c.speed_override_equipped),
        "backpedal_speed_factor_equipped": n(c.backpedal_speed_factor_equipped),
        "speed_bonus_percentage_equipped": n(c.speed_bonus_percentage_equipped),
        "acceleration_bonus_percentage_equipped": n(c.acceleration_bonus_percentage_equipped),
        "speed_bonus_percentage_holstered": n(c.speed_bonus_percentage_holstered),
        "acceleration_bonus_percentage_holstered": n(c.acceleration_bonus_percentage_holstered),
        "reload_movement_restriction": c.reload_movement_restriction,
        "ranged_draw_movement_restriction": c.ranged_draw_movement_restriction,
        "ranged_draw_speed_factor": n(c.ranged_draw_speed_factor),
        "ranged_draw_speed_factor_with_ranger_perk": n(c.ranged_draw_speed_factor_with_ranger_perk),
        "ranged_draw_turn_caps": [c.ranged_draw_turn_caps.0 as f64, c.ranged_draw_turn_caps.1 as f64],
        "ranged_release_movement_restriction": c.ranged_release_movement_restriction,
        "ranged_reload_turn_caps": [c.ranged_reload_turn_caps.0 as f64, c.ranged_reload_turn_caps.1 as f64],
    })
}

/// The whole spec_src dump (`root` = extract/json)
pub fn dump(root: &Path) -> Result<Value> {
    let ex = parse(&read(root, BP_HORSE)?, "BP_Horse")?;
    let mut curves = Map::new();
    for (field, export, prop) in HORSE_CURVES {
        let o = if export.starts_with("Default__") { cdo(&ex) } else { named(&ex, export) };
        curves.insert(field.into(), ref_path(o, prop).map_or(Value::Null, Value::String));
    }
    let (cr, chh, bdw) = horse_pawn_json_root(root)?;
    let mut horse = horse_cfg_json(&horse_cfg_json_root(root)?);
    let hm = horse_records_json_root(root)?;
    let o = horse.as_object_mut().unwrap();
    o.extend(horse_movement_json(&hm).as_object().unwrap().clone());
    o.insert("capsule_radius".into(), n(cr));
    o.insert("capsule_half_height".into(), n(chh));
    o.insert("braking_deceleration_walking".into(), n(bdw));
    let mut projectiles = Vec::new();
    for (pkg, chain) in projectile_classes_json_root(root) {
        let c = projectile_cfg_json_root(root, &chain)?;
        projectiles.push(json!({"package": pkg, "chain": chain, "cfg": projectile_json(&c)}));
    }
    let mut equipment = Vec::new();
    for (pkg, chain, native) in equipment_classes_json_root(root) {
        let c = equipment_movement_json_root(root, &chain)?;
        equipment.push(json!({"package": pkg, "chain": chain, "native_root": native, "cfg": equipment_json(&c)}));
    }
    Ok(json!({
        "_reader": "core/crates/mh-host/src/exe_records.rs dump() over extract/json: mh-character HorseCfg::from_json + horse_character_records + new_horse (exe_horse.rs), ProjectileCfg::from_json_chain (projectile.rs), EquipmentMovement::from_json_chain (equipment.rs), the BP_LadderMover mover of new_ladder_mover (exe_ladder.rs)",
        "horse": {"package": BP_HORSE, "vehicle_package": BP_VEHICLE_HORSE, "cfg": horse, "curves": curves},
        "projectiles": projectiles,
        "equipment_movement": equipment,
        "ladder": {"package": BP_LADDER_MOVER, "vehicle_package": BP_VEHICLE_LADDER_MOVER, "cfg": ladder_json(&ladder_mover_json_root(root)?)},
    }))
}

// ---- from the matrix -------------------------------------------------------------------------------------------------

fn e<E: std::fmt::Debug>(what: &str) -> impl Fn(E) -> String + '_ {
    move |x| format!("spec {what}: {x:?}")
}
fn fv(a: [f32; 3]) -> FVector {
    mh_character::uemath::v(a[0], a[1], a[2])
}

/// a curve entity's raw FRichCurve keys through mh-character's own key reader (curve_keys_from_json)
fn rich_keys(s: &mh_spec::Spec, curve_id: &str) -> Result<Vec<RichKey>> {
    let keys = s.json(curve_id, "FLD_CURVE_KEYS").map_err(e(curve_id))?;
    let wrapped = json!([{"Properties": {"FloatCurve": {"Keys": keys}}}]).to_string();
    mh_character::exe_horse::curve_keys_from_json(&wrapped).ok_or_else(|| format!("{curve_id}: bad keys"))
}

/// the horse entity id (one horse class: BP_Horse)
pub fn horse_id(s: &mh_spec::Spec) -> Result<String> {
    s.entities_of("horse").next().map(str::to_string).ok_or_else(|| "spec: no horse entity".into())
}

/// HorseCfg from the matrix (== HorseCfg::from_json, tests/exe_records.rs)
pub fn horse_cfg(s: &mh_spec::Spec) -> Result<HorseCfg> {
    let id = horse_id(s)?;
    let r = s.record(&id).map_err(e(&id))?;
    let g = |k: &str| r.f32(k).map_err(e(k));
    let v3 = |k: &str| r.vec3(k).map(fv).map_err(e(k));
    let curve = |k: &str| -> Result<Option<Vec<RichKey>>> {
        match r.reference(k).map_err(e(k))? {
            Some(c) => rich_keys(s, c).map(Some),
            None => Ok(None),
        }
    };
    let gears = r
        .json("gears")
        .map_err(e("gears"))?
        .as_array()
        .ok_or("gears: not a list")?
        .iter()
        .map(|x| GearInfo {
            max_speed: f(x, "max_speed", 0.0),
            max_acceleration: f(x, "max_acceleration", 0.0),
            allow_jump: b(x, "allow_jump", false),
            can_rider_regen_health: b(x, "can_rider_regen_health", false),
            can_rider_regen_stamina: b(x, "can_rider_regen_stamina", false),
            can_horse_regen: b(x, "can_horse_regen", false),
        })
        .collect();
    let mz = r.vec2("min_z_distance_to_enter").map_err(e("min_z_distance_to_enter"))?;
    Ok(HorseCfg {
        gears,
        turning_brake_curve: curve("turning_brake_curve")?,
        turning_factor_curve: curve("turning_factor_curve")?,
        turning_accel_curve: curve("turning_accel_curve")?,
        turning_factor_scale_airborne: g("turning_factor_scale_airborne")?,
        head_on_min_speed_to_rear: g("head_on_min_speed_to_rear")?,
        soft_bubble_rel: v3("soft_bubble_rel")?,
        soft_bubble_length: g("soft_bubble_length")?,
        soft_bubble_radius: g("soft_bubble_radius")?,
        soft_bubble_max_height: g("soft_bubble_max_height")?,
        front_rear_half_height: g("front_rear_half_height")?,
        front_rear_radius: g("front_rear_radius")?,
        front_rel: v3("front_rel")?,
        rear_rel: v3("rear_rel")?,
        avoidance_turning_acceleration: g("avoidance_turning_acceleration")?,
        speed_multiplier_on_bump: g("speed_multiplier_on_bump")?,
        speed_multiplier_on_melee: g("speed_multiplier_on_melee")?,
        bump_damage_curve: curve("bump_damage_curve")?,
        knockback_force: g("knockback_force")?,
        knockback_force_velocity_factor: g("knockback_force_velocity_factor")?,
        knockback_damage: g("knockback_damage")?,
        rearing_duration: g("rearing_duration")?,
        uncontrolled_gear: r.i64("uncontrolled_gear").map_err(e("uncontrolled_gear"))? as i32,
        attach_offset: v3("attach_offset")?,
        min_xy_distance_to_enter: g("min_xy_distance_to_enter")?,
        min_z_distance_to_enter: (mz[0], mz[1]),
        minimum_interactable_velocity: g("minimum_interactable_velocity")?,
        mesh_relative: v3("mesh_relative")?,
    })
}

/// the horse's CharacterRecords: `base` with horse_character_records' overrides from the matrix (exe mode: f32 values)
pub fn horse_records(s: &mh_spec::Spec, base: &CharacterRecords) -> Result<CharacterRecords> {
    let id = horse_id(s)?;
    let r = s.record(&id).map_err(e(&id))?;
    let g = |k: &str| r.f32(k).map(|x| x as f64).map_err(e(k));
    let mut o = base.clone();
    let m = &mut o.movement;
    m.gravity_scale = g("movement_gravity_scale")?;
    m.ground_friction = g("movement_ground_friction")?;
    m.jump_z_velocity = g("movement_jump_z_velocity")?;
    m.walkable_floor_z = g("movement_walkable_floor_z")?;
    m.max_step_height = g("movement_max_step_height")?;
    m.perch_radius_threshold = g("movement_perch_radius_threshold")?;
    m.perch_additional_height = g("movement_perch_additional_height")?;
    m.max_walk_speed = g("movement_max_walk_speed")?;
    m.max_walk_speed_crouched = g("movement_max_walk_speed_crouched")?;
    m.air_control = g("movement_air_control")?;
    m.air_control_boost_multiplier = g("movement_air_control_boost_multiplier")?;
    m.air_control_boost_velocity_threshold = g("movement_air_control_boost_velocity_threshold")?;
    m.max_acceleration = g("movement_max_acceleration")?;
    m.braking_friction_factor = g("movement_braking_friction_factor")?;
    m.braking_deceleration_falling = g("movement_braking_deceleration_falling")?;
    m.falling_lateral_friction = g("movement_falling_lateral_friction")?;
    m.crouched_half_height = g("movement_crouched_half_height")?;
    m.b_can_walk_off_ledges_when_crouching =
        r.bool("movement_b_can_walk_off_ledges_when_crouching").map_err(e("movement_b_can_walk_off_ledges_when_crouching"))?;
    o.move_extra.min_velocity_for_fall_damage = g("move_extra_min_velocity_for_fall_damage")?;
    o.move_extra.fall_damage_offset = g("move_extra_fall_damage_offset")?;
    o.move_extra.fall_damage_factor = g("move_extra_fall_damage_factor")?;
    o.character.jump_cooldown = g("character_jump_cooldown")?;
    Ok(o)
}

/// the horse pawn's (capsule radius, capsule half height, BrakingDecelerationWalking) from the matrix
pub fn horse_pawn(s: &mh_spec::Spec) -> Result<(f32, f32, f32)> {
    let id = horse_id(s)?;
    let r = s.record(&id).map_err(e(&id))?;
    let g = |k: &str| r.f32(k).map_err(e(k));
    Ok((g("capsule_radius")?, g("capsule_half_height")?, g("braking_deceleration_walking")?))
}

/// every projectile class in the matrix: (entity id, package path)
pub fn projectile_classes(s: &mh_spec::Spec) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = s
        .entities_of("projectile")
        .filter_map(|id| s.str(id, "FLD_PROJ_PACKAGE").ok().map(|p| (id.to_string(), p.to_string())))
        .collect();
    v.sort();
    v
}

/// ProjectileCfg from the matrix by entity id or package path (== ProjectileCfg::from_json_chain)
pub fn projectile_cfg(s: &mh_spec::Spec, id_or_pkg: &str) -> Result<ProjectileCfg> {
    let id = if id_or_pkg.starts_with("ENT_") {
        id_or_pkg.to_string()
    } else {
        projectile_classes(s).into_iter().find(|(_, p)| p == id_or_pkg).map(|(i, _)| i).ok_or_else(|| format!("spec: no projectile {id_or_pkg}"))?
    };
    let r = s.record(&id).map_err(e(&id))?;
    let g = |k: &str| r.f32(k).map_err(e(k));
    let bl = |k: &str| r.bool(k).map_err(e(k));
    let i = |k: &str| r.i64(k).map_err(e(k));
    let v3 = |k: &str| r.vec3(k).map(fv).map_err(e(k));
    let u8s = |k: &str| -> Result<Vec<u8>> {
        let fid = s.field_id("projectile", k).map_err(e(k))?;
        Ok(s.i64_list(&id, &fid).map_err(e(k))?.into_iter().map(|x| x as u8).collect())
    };
    Ok(ProjectileCfg {
        class: r.str("class").map_err(e("class"))?.to_string(),
        initial_speed: g("initial_speed")?,
        max_speed: g("max_speed")?,
        gravity_scale: g("gravity_scale")?,
        rotation_follows_velocity: bl("rotation_follows_velocity")?,
        should_bounce: bl("should_bounce")?,
        sweep_collision: bl("sweep_collision")?,
        force_sub_stepping: bl("force_sub_stepping")?,
        bounciness: g("bounciness")?,
        friction: g("friction")?,
        bounce_stop_threshold: g("bounce_stop_threshold")?,
        max_simulation_time_step: g("max_simulation_time_step")?,
        max_simulation_iterations: i("max_simulation_iterations")? as i32,
        bounce_additional_iterations: i("bounce_additional_iterations")? as i32,
        drag_deceleration: g("drag_deceleration")?,
        box_extent: v3("box_extent")?,
        rotation_spin: v3("rotation_spin")?,
        will_sticky_on: u8s("will_sticky_on")?,
        will_pass_through_on: u8s("will_pass_through_on")?,
        sticky_surface_pitch_blend: g("sticky_surface_pitch_blend")?,
        sticky_surface_yaw_blend: g("sticky_surface_yaw_blend")?,
        destroy_when_terminated: bl("destroy_when_terminated")?,
        path_blend_duration: g("path_blend_duration")?,
        damage: r.f32_list("damage").map_err(e("damage"))?,
        head_bonus: r.f32_list("head_bonus").map_err(e("head_bonus"))?,
        leg_bonus: r.f32_list("leg_bonus").map_err(e("leg_bonus"))?,
        initial_velocity: v3("initial_velocity")?,
        // [[channel, response 0 Ignore / 1 Overlap / 2 Block], ...]
        box_responses: r
            .json("box_responses")
            .map_err(e("box_responses"))?
            .as_array()
            .ok_or("box_responses: not a list")?
            .iter()
            .map(|p| Ok((p[0].as_str().ok_or("box_responses: channel")?.to_string(), p[1].as_u64().ok_or("box_responses: response")? as u8)))
            .collect::<Result<Vec<_>>>()?,
    })
}

/// every equipment class in the matrix: (entity id, package path)
pub fn equipment_classes(s: &mh_spec::Spec) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = s
        .entities_of("equipment_movement")
        .filter_map(|id| s.str(id, "FLD_EQMV_PACKAGE").ok().map(|p| (id.to_string(), p.to_string())))
        .collect();
    v.sort();
    v
}

/// EquipmentMovement from the matrix by entity id, package path or class stem (e.g. "BP_Longbow")
/// (== EquipmentMovement::from_json_chain, tests/exe_records.rs)
pub fn equipment_movement(s: &mh_spec::Spec, key: &str) -> Result<EquipmentMovement> {
    let id = if key.starts_with("ENT_") {
        key.to_string()
    } else {
        equipment_classes(s)
            .into_iter()
            .find(|(_, p)| p == key || p.rsplit('/').next() == Some(key))
            .map(|(i, _)| i)
            .ok_or_else(|| format!("spec: no equipment {key}"))?
    };
    let r = s.record(&id).map_err(e(&id))?;
    let g = |k: &str| r.f32(k).map_err(e(k));
    let u = |k: &str| r.i64(k).map(|x| x as u8).map_err(e(k));
    let v2 = |k: &str| r.vec2(k).map(|a| (a[0], a[1])).map_err(e(k));
    Ok(EquipmentMovement {
        class: r.str("class").map_err(e("class"))?.to_string(),
        movement_restriction: u("movement_restriction")?,
        sub_sprint_speed_bonus_equipped: g("sub_sprint_speed_bonus_equipped")?,
        speed_override_equipped: g("speed_override_equipped")?,
        backpedal_speed_factor_equipped: g("backpedal_speed_factor_equipped")?,
        speed_bonus_percentage_equipped: g("speed_bonus_percentage_equipped")?,
        acceleration_bonus_percentage_equipped: g("acceleration_bonus_percentage_equipped")?,
        speed_bonus_percentage_holstered: g("speed_bonus_percentage_holstered")?,
        acceleration_bonus_percentage_holstered: g("acceleration_bonus_percentage_holstered")?,
        reload_movement_restriction: u("reload_movement_restriction")?,
        ranged_draw_movement_restriction: u("ranged_draw_movement_restriction")?,
        ranged_draw_speed_factor: g("ranged_draw_speed_factor")?,
        ranged_draw_speed_factor_with_ranger_perk: g("ranged_draw_speed_factor_with_ranger_perk")?,
        ranged_draw_turn_caps: v2("ranged_draw_turn_caps")?,
        ranged_release_movement_restriction: u("ranged_release_movement_restriction")?,
        ranged_reload_turn_caps: v2("ranged_reload_turn_caps")?,
    })
}

/// the ladder mover from the matrix (ENT_LADDER_BP_LadderMover)
pub fn ladder_mover(s: &mh_spec::Spec) -> Result<LadderMoverCfg> {
    let id = s.entities_of("ladder").next().map(str::to_string).ok_or("spec: no ladder entity")?;
    let r = s.record(&id).map_err(e(&id))?;
    let g = |k: &str| r.f32(k).map_err(e(k));
    let bl = |k: &str| r.bool(k).map_err(e(k));
    Ok(LadderMoverCfg {
        max_walk_speed: g("max_walk_speed")?,
        max_acceleration: g("max_acceleration")?,
        braking_deceleration_walking: g("braking_deceleration_walking")?,
        gravity_scale: g("gravity_scale")?,
        ground_friction: g("ground_friction")?,
        braking_friction_factor: g("braking_friction_factor")?,
        capsule_radius: g("capsule_radius")?,
        capsule_half_height: g("capsule_half_height")?,
        in_steps: bl("in_steps")?,
        use_driver_speed: bl("use_driver_speed")?,
        step_size: g("step_size")?,
        secondary_turn_limit: g("secondary_turn_limit")?,
        use_driver_turn_caps: bl("use_driver_turn_caps")?,
        is_ladder: bl("is_ladder")?,
    })
}

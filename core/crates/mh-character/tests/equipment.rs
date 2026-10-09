//! The movement side of equipment (equipment.rs): BP_Longbow's and BP_ToolBox's data over the AMordhauEquipment ctor,
//! the longbow draw slowing the walk through the motion speed factor (URangedDrawMotion::OnBegin -> SpeedFactor ->
//! OnCharacterLODTick -> GetSpeedFactor), the equipment speed bonuses (UpdateEquipmentSpeedAndAcceleration), and the
//! backpedal factor. Data: extract/json (SKIP without it).

use mh_character::equipment::EquipmentMovement;
use mh_character::exe::{ExeInput, ExeMovement};
use mh_character::uemath::v;
use mh_character::world::BoxWorld;
use mh_character::{CharacterRecords, CharacterSource, RecordsJson};
use std::path::PathBuf;

fn root() -> PathBuf {
    let dir = std::env::var("MH_CHARACTER_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    dir.join("../../..")
}

fn bp(rel: &str) -> Option<String> {
    std::fs::read_to_string(root().join("extract/json/Mordhau/Content/Mordhau/Blueprints").join(rel)).ok()
}

fn chain(rels: &[&str]) -> Option<EquipmentMovement> {
    let j: Option<Vec<String>> = rels.iter().map(|r| bp(r)).collect();
    let j = j?;
    let r: Vec<&str> = j.iter().map(|s| s.as_str()).collect();
    Some(EquipmentMovement::from_json_chain(&r).unwrap())
}

fn longbow() -> Option<EquipmentMovement> {
    chain(&["Equipment/Ranged/BP_Longbow.json", "Equipment/Ranged/BP_Bow.json", "Equipment/Ranged/BP_MissileEquipment.json"])
}

fn toolbox() -> Option<EquipmentMovement> {
    chain(&["Equipment/Misc/BP_ToolBox.json", "Equipment/Misc/BP_2HThrowableBase.json"])
}

fn rec() -> Option<CharacterRecords> {
    let p = root().join("core/tests/golden/character/records.json");
    Some(RecordsJson(&std::fs::read_to_string(p).ok()?).load().unwrap())
}

#[test]
fn longbow_and_toolbox_data() {
    let (Some(l), Some(t)) = (longbow(), toolbox()) else { return eprintln!("SKIP: no extract/json") };
    assert_eq!(l.class, "BP_Longbow_C");
    assert_eq!(l.ranged_draw_speed_factor, 0.7);
    assert_eq!(l.ranged_draw_movement_restriction, 2, "ctor Walk");
    assert_eq!((l.reload_movement_restriction, l.ranged_release_movement_restriction), (2, 2), "BP_MissileEquipment Walk");
    assert_eq!(l.draw_motion(false), (0.7, 2));
    assert_eq!(l.draw_motion(true), (1.0, 2), "RangedDrawSpeedFactorWithRangerPerk stays the ctor 1");
    assert_eq!(t.class, "BP_ToolBox_C");
    assert_eq!(t.ranged_draw_movement_restriction, 0, "BP_2HThrowableBase None");
    assert_eq!(t.backpedal_speed_factor_equipped, 1.0);
}

/// drawing the longbow: the draw motion's SpeedFactor 0.7 reaches GetSpeedFactor through OnCharacterLODTick (one
/// actor tick, then the next frame's movement): the walk speed settles at 0.7x
#[test]
fn longbow_draw_slows_the_walk() {
    let (Some(l), Some(r)) = (longbow(), rec()) else { return eprintln!("SKIP: no data") };
    let mut w = BoxWorld::new();
    w.add_box(v(-50000.0, -50000.0, -200.0), v(50000.0, 50000.0, 0.0));
    let mk = || {
        let mut m = ExeMovement::new(&r, v(0.0, 0.0, 96.0 + mh_character::exe_cmc::AVG_FLOOR_DIST));
        m.world_time = 10.0;
        m.frame(&w, 1.0 / 60.0, &ExeInput::default());
        m
    };
    let mut free = mk();
    let mut draw = mk();
    draw.update_equipment_speed_and_acceleration(None, Some(&l), &[]);
    free.update_equipment_speed_and_acceleration(None, Some(&l), &[]);
    let (sf, restr) = l.draw_motion(false);
    draw.current_motion_factors = Some((sf, 1.0));
    draw.motion_restriction = restr as i64;
    for _ in 0..240 {
        free.frame(&w, 1.0 / 60.0, &ExeInput { fwd: 1.0, ..Default::default() });
        draw.frame(&w, 1.0 / 60.0, &ExeInput { fwd: 1.0, ..Default::default() });
    }
    let s = |m: &ExeMovement| (m.velocity.x * m.velocity.x + m.velocity.y * m.velocity.y).sqrt();
    println!("free {} drawing {}", s(&free), s(&draw));
    assert_eq!(draw.motion_speed_factor, 0.7);
    assert!((s(&draw) / s(&free) - 0.7).abs() < 0.01, "{} / {}", s(&draw), s(&free));
}

/// UpdateEquipmentSpeedAndAcceleration: the left hand's override, the right's only when smaller, the bonuses summed,
/// holstered items' bonuses added; the left hand's backpedal factor wins in OnCharacterLODTick
#[test]
fn equipment_speed_bonuses() {
    let Some(r) = rec() else { return eprintln!("SKIP: no golden records") };
    let mut m = ExeMovement::new(&r, v(0.0, 0.0, 100.0));
    let mut a = EquipmentMovement { speed_override_equipped: 0.9, speed_bonus_percentage_equipped: 0.05, backpedal_speed_factor_equipped: 0.8, ..Default::default() };
    let b = EquipmentMovement { speed_override_equipped: 0.95, speed_bonus_percentage_equipped: -0.1, acceleration_bonus_percentage_equipped: 0.2, backpedal_speed_factor_equipped: 0.6, ..Default::default() };
    let h = EquipmentMovement { speed_bonus_percentage_holstered: -0.02, ..Default::default() };
    m.update_equipment_speed_and_acceleration(Some(&a), Some(&b), &[&h]);
    assert_eq!(m.equip_speed_cap, 0.9, "the right's 0.95 is not smaller");
    assert_eq!(m.equip_speed_add, -0.02 + (0.05 + -0.1));
    assert_eq!(m.equip_accel_add, 0.2);
    m.on_character_lod_tick();
    assert_eq!(m.equipment_backpedal_speed_factor, 0.8);
    a.speed_override_equipped = 0.0;
    m.update_equipment_speed_and_acceleration(Some(&a), Some(&b), &[]);
    assert_eq!(m.equip_speed_cap, 0.95);
}

/// The Knight's worn pieces (BP_Houndskull tier 3 head, BP_BurgundianCuirass tier 3 chest, BP_BrigandineLegs tier 2)
/// through the paks' class defaults: ArmorSpeedFactor = 1 - (2 * 0.09332 + 0.063075 - 0.063075) and the walk slows
/// by it
#[test]
fn knight_armor_slows_the_walk() {
    use mh_character::equipment::{ArmorRules, ArmorWearable};
    let Some(r) = rec() else { return eprintln!("SKIP: no golden records") };
    let Ok(vfs) = mh_pak::Vfs::mount_default() else { return eprintln!("SKIP: no install") };
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(std::sync::Arc::new(vfs)));
    let piece = |c: &str| ArmorWearable::from_defaults(&serde_json::Value::Object((*pk.defaults(c)).clone()));
    let w = "Mordhau/Content/Mordhau/Blueprints/Wearables/";
    let (h, u, l) = (piece(&format!("{w}Head/Tier3/BP_Houndskull")), piece(&format!("{w}UpperChest/Tier3/BP_BurgundianCuirass")), piece(&format!("{w}Legs/Tier2/BP_BrigandineLegs")));
    println!("{h:?} {u:?} {l:?}");
    assert_eq!((h.armor_class, u.armor_class, l.armor_class), (3, 3, 2));
    let mut world = BoxWorld::new();
    world.add_box(v(-50000.0, -50000.0, -200.0), v(50000.0, 50000.0, 0.0));
    let mk = |armor: bool| {
        let mut m = ExeMovement::new(&r, v(0.0, 0.0, 96.0 + mh_character::exe_cmc::AVG_FLOOR_DIST));
        m.world_time = 10.0;
        if armor {
            m.update_armor_speed_and_acceleration(Some(&h), Some(&u), Some(&l), &ArmorRules::default());
        }
        for _ in 0..240 {
            m.frame(&world, 1.0 / 60.0, &ExeInput { fwd: 1.0, ..Default::default() });
        }
        (m.velocity.x * m.velocity.x + m.velocity.y * m.velocity.y).sqrt()
    };
    let (naked, knight) = (mk(false), mk(true));
    println!("naked {naked} knight {knight}");
    assert!(knight < naked * 0.85, "{knight} vs {naked}");
}

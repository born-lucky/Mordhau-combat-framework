//! The horse / projectile / ladder-mover records from the spec matrix (mh_host::exe_records) == mh-character's own
//! loaders over extract/json (HorseCfg::from_json, horse_character_records, ProjectileCfg::from_json_chain) and ==
//! what ExeMovement::new_ladder_mover / new_horse set, compared as f32 (PartialEq on the f32 structs). Skipped (pass)
//! without data_gen/spec or extract/json.
use mh_character::exe_ladder::LadderInfo;
use mh_character::uemath::v;
use mh_character::{CharacterRecords, ExeMovement};
use mh_host::exe_records as xr;
use std::path::PathBuf;

fn repo() -> PathBuf {
    std::env::var("MORDHAU_REPO").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

fn inputs() -> Option<(mh_spec::Spec, PathBuf)> {
    let root = repo().join("extract/json");
    if !root.join(format!("{}.json", xr::BP_HORSE)).exists() {
        eprintln!("SKIP: no extract/json");
        return None;
    }
    match mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) {
        Ok(s) if s.entities_of("horse").next().is_some() => Some((s, root)),
        Ok(_) => {
            eprintln!("SKIP: the matrix has no horse records yet (run scripts/sheets_refresh.sh)");
            None
        }
        Err(e) => {
            eprintln!("SKIP: {e:?}");
            None
        }
    }
}

#[test]
fn horse_from_spec_equals_the_loader() {
    let Some((s, root)) = inputs() else { return };
    let want = xr::horse_cfg_json_root(&root).unwrap();
    let got = xr::horse_cfg(&s).unwrap();
    assert_eq!(got, want, "HorseCfg from the matrix != HorseCfg::from_json");
    assert!(got.turning_factor_curve.is_some() && !got.gears.is_empty(), "BP_Horse has gears and turning curves");
    // horse_character_records over a real base: every field equal (the overrides as f32, the rest the base's)
    let base = CharacterRecords::default();
    let want_r = mh_character::exe_horse::horse_character_records(&base, &std::fs::read_to_string(root.join(format!("{}.json", xr::BP_HORSE))).unwrap()).unwrap();
    let got_r = xr::horse_records(&s, &base).unwrap();
    let f = |x: f64| x as f32;
    let (a, b) = (&got_r.movement, &want_r.movement);
    for (n, x, y) in [
        ("gravity_scale", a.gravity_scale, b.gravity_scale),
        ("ground_friction", a.ground_friction, b.ground_friction),
        ("jump_z_velocity", a.jump_z_velocity, b.jump_z_velocity),
        ("walkable_floor_z", a.walkable_floor_z, b.walkable_floor_z),
        ("max_step_height", a.max_step_height, b.max_step_height),
        ("max_walk_speed", a.max_walk_speed, b.max_walk_speed),
        ("air_control", a.air_control, b.air_control),
        ("max_acceleration", a.max_acceleration, b.max_acceleration),
        ("crouched_half_height", a.crouched_half_height, b.crouched_half_height),
        ("fall_damage_factor", got_r.move_extra.fall_damage_factor, want_r.move_extra.fall_damage_factor),
        ("jump_cooldown", got_r.character.jump_cooldown, want_r.character.jump_cooldown),
    ] {
        assert_eq!(f(x).to_bits(), f(y).to_bits(), "horse {n}: {x} vs {y}");
    }
    let (cr, chh, bdw) = xr::horse_pawn(&s).unwrap();
    let m = ExeMovement::new_horse(&want_r, &std::fs::read_to_string(root.join(format!("{}.json", xr::BP_HORSE))).unwrap(), want, v(0.0, 0.0, 0.0)).unwrap();
    assert_eq!((cr, chh, bdw), (m.e.capsule_radius, m.e.capsule_half_height, m.e.braking_deceleration_walking), "new_horse pawn values");
    eprintln!("horse: HorseCfg ({} gears, 4 curves) + records + pawn == the loaders", got.gears.len());
}

#[test]
fn projectiles_from_spec_equal_the_loader() {
    let Some((s, root)) = inputs() else { return };
    let classes = xr::projectile_classes_json_root(&root);
    let in_spec = xr::projectile_classes(&s);
    assert_eq!(in_spec.len(), classes.len(), "every projectile class is in the matrix");
    for (pkg, chain) in &classes {
        let want = xr::projectile_cfg_json_root(&root, chain).unwrap();
        let got = xr::projectile_cfg(&s, pkg).unwrap();
        assert_eq!(got, want, "{pkg}");
    }
    for k in ["Equipment/Ranged/BP_ArrowProjectile", "Equipment/Ranged/BP_BoltProjectile", "Equipment/Weapons/BP_ThrownJavelinProjectile"] {
        let p = format!("Mordhau/Content/Mordhau/Blueprints/{k}");
        assert!(classes.iter().any(|(c, _)| *c == p), "{p} listed");
    }
    eprintln!("projectiles: {} classes == ProjectileCfg::from_json_chain", classes.len());
}

#[test]
fn ladder_mover_from_spec_equals_the_port() {
    let Some((s, root)) = inputs() else { return };
    let got = xr::ladder_mover(&s).unwrap();
    assert_eq!(got, xr::ladder_mover_json_root(&root).unwrap(), "ladder mover from the matrix != the package reader");
    // == what exe_ladder.rs new_ladder_mover hard-codes (its constants read off the same packages / ctors)
    let info = LadderInfo { start: v(0.0, 0.0, 0.0), end: v(0.0, 0.0, 400.0), exit: v(0.0, 0.0, 500.0), yaw: 0.0 };
    let m = ExeMovement::new_ladder_mover(&CharacterRecords::default(), info, v(0.0, 0.0, 0.0));
    let l = m.ladder.as_ref().unwrap();
    assert_eq!(got.max_walk_speed, m.c.max_walk_speed);
    assert_eq!(got.max_acceleration, m.c.max_acceleration);
    assert_eq!(got.braking_deceleration_walking, m.e.braking_deceleration_walking);
    assert_eq!(got.gravity_scale, m.c.gravity_scale);
    assert_eq!(got.ground_friction, m.c.ground_friction);
    assert_eq!(got.braking_friction_factor, m.c.braking_friction_factor);
    assert_eq!(got.capsule_radius, m.e.capsule_radius);
    assert_eq!(got.capsule_half_height, m.e.capsule_half_height);
    assert_eq!((got.in_steps, got.use_driver_speed, got.step_size), (l.in_steps, l.use_driver_speed, l.step_size));
    eprintln!("ladder: BP_LadderMover from the matrix == new_ladder_mover");
}

#[test]
fn equipment_movement_from_spec_equals_the_loader() {
    let Some((s, root)) = inputs() else { return };
    let classes = xr::equipment_classes_json_root(&root);
    assert_eq!(xr::equipment_classes(&s).len(), classes.len(), "every equipment class is in the matrix");
    for (pkg, chain, _) in &classes {
        let want = xr::equipment_movement_json_root(&root, chain).unwrap();
        assert_eq!(xr::equipment_movement(&s, pkg).unwrap(), want, "{pkg}");
    }
    // the ranged / tool loadout items rust-character asked for (r9), by class stem
    for k in ["BP_Longbow", "BP_ToolBox"] {
        let m = xr::equipment_movement(&s, k).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert!(m.class.starts_with(k), "{k}: class {}", m.class);
    }
    eprintln!("equipment movement: {} classes == EquipmentMovement::from_json_chain", classes.len());
}

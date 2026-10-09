use mordhau_core::combat::world::World;
use mordhau_core::combat::enums::bt;
use mordhau_core::data::{RecordsJsonExe, SpecSource};
use std::rc::Rc;

/// The local fixture remains outside Git. Set the env path to a generated combat records JSON.
/// This test fails when the fixture is absent rather than silently reporting unexecuted coverage.
#[test]
fn buffered_player_parry_reselects_current_mouse_side_after_cooldown() {
    let path = std::env::var("MORDHAU_PARRY_TEST_SPEC").expect("set MORDHAU_PARRY_TEST_SPEC to local combat records");
    let text = std::fs::read_to_string(path).expect("read local combat records");
    let spec = Rc::new(RecordsJsonExe(&text).load_spec().expect("combat records"));
    const WEAPON: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
    for (requested, current_x, expected) in [
        (bt::ALT_REGULAR, Some(-0.000001), bt::ALT_REGULAR),
        (bt::ALT_REGULAR, Some(0.000001), bt::REGULAR),
        (bt::REGULAR, Some(-0.000001), bt::ALT_REGULAR),
        (bt::ALT_REGULAR, None, bt::REGULAR),
    ] {
        let mut world = World::new(spec.clone(), 1.0 / 120.0);
        let fighter = world.add_fighter("Player", WEAPON, "");
        world.block_pressed(fighter, bt::REGULAR);
        let first = world.fighters[fighter].last_parry_motion.expect("first parry");
        // Re-press during the active parry: native bWantsBlock buffers it until the motion allows blocking.
        world.release_block(fighter);
        world.block_pressed(fighter, requested);
        assert!(world.fighters[fighter].wants_block, "test must actually enter the buffered path");
        world.fighters[fighter].controller_angling_x = current_x;
        for _ in 0..400 {
            world.step();
            if world.fighters[fighter].last_parry_motion != Some(first) { break; }
        }
        let f = &world.fighters[fighter];
        let next = f.last_parry_motion.expect("buffered parry should begin");
        assert_ne!(next, first, "buffered parry never began");
        let block_type = f.motions[next.0 as usize].as_ref().unwrap().parry().unwrap().block_type;
        assert_eq!(block_type, expected, "requested {requested}, current angling {current_x:?}");
        assert!(!f.wants_block, "successful retry consumes the buffer");
    }
}

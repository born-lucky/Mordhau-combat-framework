//! EVD_MOV_020 (state/proofs/pawn_collision.md): characters block each other. Every character's CollisionCylinder
//! is a Pawn-profile body (ACharacter::ACharacter rva=0x2f27200, 0x142f27386) that blocks the Pawn channel the
//! movement sweeps on, 50 / 96 (AMordhauCharacter ctor rva=0x1524d90, decomp 2254-2255): a fighter walking into a
//! standing one stops with the capsule centres radius + radius = 100 cm apart. A dead one collides with nothing
//! (AAdvancedCharacter::OnTookDamage rva=0x14926a0, SetCollisionResponseToAllChannels(ECR_Ignore)).
use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
use mh_sim::{FighterDesc, Sim, SimInput};
use mordhau_core::ue::FVector;
use std::rc::Rc;
use std::sync::Arc;

const GS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";

fn sim() -> Option<(Sim, usize, usize)> {
    let m = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).ok()?;
    let vfs = Arc::new(mh_pak::Vfs::mount_default().ok()?);
    let rec = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json")).ok()?;
    let ld = mh_sim::load::load(&m, vfs.clone(), &[GS]).ok()?;
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: GS.into(), team: 1, location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "Bot".into(), weapon: GS.into(), team: 2, location: FVector::new(300.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    // settle on the floor, past RagdollFallingGetUpDuration (IsRagdollFallingOrGettingUp rva=0x1487ca0 ignores move
    // input in a younger world, EVD_MOV_015)
    for _ in 0..240 {
        s.step(&[(a, SimInput { yaw: Some(0.0), ..Default::default() })]);
    }
    Some((s, a, b))
}

/// A walks forward (+X) for 3 s toward the bot standing 300 cm ahead; the bot's location before and A's / the bot's after
fn walk_into(s: &mut Sim, a: usize, b: usize) -> (FVector, FVector, FVector) {
    let b0 = s.movers[b].location;
    for _ in 0..180 {
        s.step(&[(a, SimInput { yaw: Some(0.0), fwd: 1.0, ..Default::default() })]);
    }
    (b0, s.movers[a].location, s.movers[b].location)
}

#[test]
fn walking_into_a_standing_bot_stops_at_radius_plus_radius() {
    let Some((mut s, a, b)) = sim() else { return };
    let r = s.movers[a].e.capsule_radius + s.movers[b].e.capsule_radius;
    assert_eq!(r, 100.0);
    let (b0, pa, pb) = walk_into(&mut s, a, b);
    let gap = ((pb.x - pa.x).powi(2) + (pb.y - pa.y).powi(2)).sqrt();
    println!("A {:?} bot {:?} gap {gap}", pa, pb);
    assert!(gap >= r - 0.01 && gap < r + 1.0, "A stops at the bot's capsule: gap {gap}, expected {r}");
    assert!((pb.x - b0.x).abs() < 0.01 && (pb.y - b0.y).abs() < 0.01, "the bot is not pushed: {:?} -> {:?}", b0, pb);
    assert!((pa.z - pb.z).abs() < 0.5, "A stays on the floor, not on the bot: {:?}", pa);
}

#[test]
fn a_dead_character_does_not_block() {
    let Some((mut s, a, b)) = sim() else { return };
    s.combat.fighters[b].dead = true;
    let (_, pa, pb) = walk_into(&mut s, a, b);
    assert!(pa.x > pb.x + 100.0, "A walks through the dead bot: A {:?} bot {:?}", pa, pb);
}

//! r5 AI gaps: GetAllyClearanceSides' box test (rva=0x14fa080) and LODTick's turning (rva=0x14fe7c0).
use mh_mode::ai::{BotBody, BotHost, Bots, PawnView, Profile};
use mordhau_core::ue::FVector;

struct Host;
impl BotHost for Host {
    fn request_attack(&mut self, _b: usize, _mv: i64, _a: f64) -> PawnView {
        PawnView::default()
    }
    fn request_parry(&mut self, _b: usize, _bt: i64) -> PawnView {
        PawnView::default()
    }
    fn request_feint(&mut self, _b: usize) -> PawnView {
        PawnView::default()
    }
}

fn body(name: &str, at: FVector, yaw: f64, team: i64) -> BotBody {
    let mut b = BotBody::new(name);
    b.location = at;
    b.yaw = yaw;
    b.team = team;
    b
}

fn bots(bodies: Vec<BotBody>, team_mode: bool) -> Bots {
    let mut w = Bots { team_mode, ..Default::default() };
    for b in bodies {
        w.add_body(b);
    }
    let mut p = Profile::default();
    p.max_turn_rate = 360.0;
    p.max_look_up_rate = 180.0;
    w.add_bot(0, p, None, 0.0, &mut Host);
    w
}

#[test]
fn ally_clearance_box() {
    // me at the origin facing +X; an ally 100 cm ahead, 100 cm to the right (UE +Y = right): the right side is blocked
    let mut w = bots(vec![body("me", FVector::ZERO, 0.0, 1), body("ally", FVector::new(100.0, 100.0, 0.0), 0.0, 1)], true);
    let calls = w.rng.calls;
    assert_eq!(w.with_bot(0, 0.0, &mut Host, |c, cx| c.ally_clearance_sides(cx)), 1, "only the left is clear");
    assert_eq!(w.rng.calls, calls, "no rand() draw when one side is decided");
    // ally on the left (Y < 0)
    w.bodies[1].location = FVector::new(100.0, -100.0, 0.0);
    assert_eq!(w.with_bot(0, 0.0, &mut Host, |c, cx| c.ally_clearance_sides(cx)), 0, "only the right is clear");
    // facing -Y (yaw -90): the world offset (100, -100) is ahead (local X 100) and right (local Y 100)
    w.bodies[0].yaw = -90.0;
    assert_eq!(w.with_bot(0, 0.0, &mut Host, |c, cx| c.ally_clearance_sides(cx)), 1);
    // facing +Y (yaw 90): (100, 100) is ahead and left (local Y -100)
    w.bodies[0].yaw = 90.0;
    w.bodies[1].location = FVector::new(100.0, 100.0, 0.0);
    assert_eq!(w.with_bot(0, 0.0, &mut Host, |c, cx| c.ally_clearance_sides(cx)), 0);
    // outside the box (behind me by more than 75 cm) or too high: both clear -> a rand() coin
    w.bodies[0].yaw = 0.0;
    w.bodies[1].location = FVector::new(-100.0, 50.0, 0.0);
    let calls = w.rng.calls;
    let r = w.with_bot(0, 0.0, &mut Host, |c, cx| c.ally_clearance_sides(cx));
    assert!(r == 0 || r == 1);
    assert_eq!(w.rng.calls, calls + 1);
    w.bodies[1].location = FVector::new(100.0, 50.0, 150.0);
    let calls = w.rng.calls;
    w.with_bot(0, 0.0, &mut Host, |c, cx| c.ally_clearance_sides(cx));
    assert_eq!(w.rng.calls, calls + 1, "|Z| > 100: not in the box");
    // allies on both sides: neither clear -> the coin
    w.bodies.push(body("ally2", FVector::new(100.0, -50.0, 0.0), 0.0, 1));
    w.bodies[1].location = FVector::new(100.0, 50.0, 0.0);
    w.with_bot(0, 0.0, &mut Host, |c, cx| c.update_perception(cx));
    let calls = w.rng.calls;
    w.with_bot(0, 0.0, &mut Host, |c, cx| c.ally_clearance_sides(cx));
    assert_eq!(w.rng.calls, calls + 1);
}

#[test]
fn lod_turn_toward_facing() {
    let mut w = bots(vec![body("me", FVector::ZERO, 0.0, 1), body("enemy", FVector::new(0.0, 500.0, 0.0), 0.0, 2)], false);
    // face the enemy at +Y (90 degrees right): RotationTargetInterpolated eases there with alpha = dt / 0.1
    let dt = 1.0 / 60.0;
    w.with_bot(0, 0.0, &mut Host, |c, _| c.start_facing_actor(1, 0.0, [0.0, 0.0], 0.0));
    let (y, p) = w.with_bot(0, 0.0, &mut Host, |c, cx| c.lod_turn(cx, dt, None)).unwrap();
    // RTI yaw after one frame = 90 * (1/6) = 15 degrees; the yaw input = 15 clamped to dt * 360 = 6
    assert!((y - 6.0).abs() < 1e-4, "yaw input {y}");
    assert!(p.abs() < 1e-4, "pitch input {p}");
    assert!((w.bot(0).rotation_target_interpolated[1] - 15.0).abs() < 1e-3);
    // the pawn turns by the inputs; after a second it faces the enemy and the input fades out
    for _ in 0..60 {
        let (y, _) = w.with_bot(0, 0.0, &mut Host, |c, cx| c.lod_turn(cx, dt, None)).unwrap();
        w.bodies[0].yaw += y as f64;
    }
    assert!((w.bodies[0].yaw - 90.0).abs() < 1.0, "turned to {}", w.bodies[0].yaw);
    // a facing actor above: pitch input up (elevation), clamped by MaxLookUpRate
    w.bodies[1].location = FVector::new(0.0, 500.0, 500.0);
    let (_, p) = w.with_bot(0, 0.0, &mut Host, |c, cx| c.lod_turn(cx, dt, None)).unwrap();
    assert!(p > 0.0 && p <= dt * 180.0 + 1e-5, "pitch {p}");
    // facing movement with no path direction: no input
    w.with_bot(0, 0.0, &mut Host, |c, _| c.start_facing_movement(0.0));
    assert!(w.with_bot(0, 0.0, &mut Host, |c, cx| c.lod_turn(cx, dt, None)).is_none());
    assert!(w.with_bot(0, 0.0, &mut Host, |c, cx| c.lod_turn(cx, dt, Some(FVector::new(1.0, 0.0, 0.0)))).is_some());
}

#[test]
fn make_from_xz() {
    use mh_mode::ai::controller::make_from_xz_rotator;
    let r = make_from_xz_rotator(FVector::new(0.0, 1.0, 0.0));
    assert!((r[1] - 90.0).abs() < 1e-3 && r[0].abs() < 1e-3 && r[2].abs() < 1e-3, "{r:?}");
    let r = make_from_xz_rotator(FVector::new(1.0, 0.0, 1.0));
    assert!((r[0] - 45.0).abs() < 1e-2 && r[1].abs() < 1e-3, "{r:?}");
}

fn one_task(ty: &str, params: serde_json::Value) -> mh_mode::ai::TreeDef {
    serde_json::from_value(serde_json::json!({
        "asset": "test",
        "root": {"kind": "sequence", "type": "BTComposite_Sequence", "children": [{"node": {"kind": "task", "type": ty, "name": ty, "params": params}}]}
    }))
    .unwrap()
}

fn weapon(id: &str, can_attack: bool) -> mh_mode::ai::WeaponView {
    mh_mode::ai::WeaponView { id: id.into(), chain: vec![id.into()], native_class: "MordhauWeapon".into(), b_can_attack: can_attack, stab_damage0: 0.0, strike_damage0: 0.0 }
}

/// UBTTask_SwitchEquipment::PerformSwitchingTask rva=0x1497830: the first melee item that is not in a hand ->
/// an equip request (InProgress); in the right or the left hand -> Succeeded
#[test]
fn switch_equipment_requests() {
    let mut w = Bots::default();
    let mut b = body("me", FVector::ZERO, 0.0, 1);
    b.inventory = vec![
        mh_mode::ai::InventoryItem { weapon: Some(weapon("Bow", false)), is_fists: false, ranged: true },
        mh_mode::ai::InventoryItem { weapon: Some(weapon("Longsword", true)), is_fists: false, ranged: false },
    ];
    b.right_hand = 0;
    w.add_body(b);
    let t = one_task("BTTask_SwitchEquipment", serde_json::json!({"b_melee": true}));
    w.add_bot(0, Profile::default(), Some(&t), 0.0, &mut Host);
    w.bots[0].as_mut().unwrap().record_trace = true;
    w.tick(1.0 / 60.0, 0.0, &mut Host);
    let ev: Vec<(&str, i64)> = w.bot(0).events.iter().map(|e| (e.kind, e.id)).collect();
    // ExecuteTask and the same frame's TickTask (rva=0x14a5260) both run PerformSwitchingTask
    assert!(!ev.is_empty() && ev.iter().all(|e| *e == ("equip", 1)), "equip slot 1 (the longsword): {ev:?}");
    // the sword in the left hand counts as equipped: no new request
    let mut w2 = Bots::default();
    let mut b = w.bodies[0].clone();
    b.ai = None;
    b.left_hand = Some(1);
    w2.add_body(b);
    w2.add_bot(0, Profile::default(), Some(&t), 0.0, &mut Host);
    w2.tick(1.0 / 60.0, 0.0, &mut Host);
    assert!(w2.bot(0).events.is_empty());
}

/// UBTTask_VoiceOrEmote::ExecuteTask rva=0x147fed0: Chance roll, then a voice line and an emote under the cooldowns
#[test]
fn voice_or_emote_events() {
    let mut w = Bots::default();
    w.add_body(body("me", FVector::ZERO, 0.0, 1));
    let t = one_task("BTTask_VoiceOrEmote", serde_json::json!({"chance": 1.0, "global_cooldown": 10.0, "voice_commands": [7, 9], "emotes": [3], "b_force_emote": true}));
    w.add_bot(0, Profile::default(), Some(&t), 0.0, &mut Host);
    w.tick(1.0 / 60.0, 20.0, &mut Host);
    let ev: Vec<(&str, bool)> = w.bot(0).events.iter().map(|e| (e.kind, e.forced)).collect();
    assert_eq!(ev, vec![("voice", false), ("emote", true)]);
    let id = w.bot(0).events[0].id;
    assert!(id == 7 || id == 9);
    // within the global cooldown: no new event
    w.tick(1.0 / 60.0, 21.0, &mut Host);
    assert_eq!(w.bot(0).events.len(), 2);
    w.tick(1.0 / 60.0, 31.0, &mut Host);
    assert_eq!(w.bot(0).events.len(), 4, "after the 10 s cooldown");
}

/// UAISense_Hearing (Bots::report_noise): heard within HearingRange 2000 x Loudness, then a hearing stimulus perceives
/// the instigator even outside the sight cone
#[test]
fn hearing_noise_perceives_behind() {
    // the enemy 1500 cm behind the bot (outside the 65 degree cone): not seen
    let mut w = bots(vec![body("me", FVector::ZERO, 0.0, 1), body("enemy", FVector::new(-1500.0, 0.0, 0.0), 0.0, 2)], false);
    w.with_bot(0, 2.0, &mut Host, |c, cx| c.update_perception(cx));
    assert!(w.with_bot(0, 2.0, &mut Host, |c, cx| c.perceived_enemies(cx)).is_empty(), "behind: not seen");
    // a quiet sound (0.5 -> 1000 cm): not heard
    w.report_noise(1, FVector::new(-1500.0, 0.0, 0.0), 0.5, 0.0, 3.0);
    assert!(w.bot(0).heard.is_empty());
    // a 0.75 sound (1500 cm range): heard; the next perception update perceives the enemy
    w.report_noise(1, FVector::new(-1500.0, 0.0, 0.0), 0.75, 0.0, 3.0);
    assert_eq!(w.bot(0).heard.len(), 1);
    w.with_bot(0, 4.5, &mut Host, |c, cx| c.update_perception(cx));
    assert_eq!(w.with_bot(0, 4.5, &mut Host, |c, cx| c.perceived_enemies(cx)), vec![1]);
    // own sounds are not heard
    w.report_noise(0, FVector::ZERO, 1.0, 0.0, 5.0);
    assert_eq!(w.bot(0).heard.len(), 1);
}

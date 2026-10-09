//! The exe's parry / chamber / clash geometry (combat/geometry.rs; exe.rs differences 13-14): unit tests of the angle
//! and box tests, and the component hit pipeline driven by a scripted TraceHost (BlockCollider vs body hits, the
//! BlockedAttacks memory). Needs the git-ignored golden spec for the integration part.

use mordhau_core::combat::geometry::{check_simple_block, check_simple_block_directional, line_box_intersection, test_forward_parry};
use mordhau_core::combat::world::{FighterGeom, HitComp, Input, RawHit, TraceHost};
use mordhau_core::combat::World;
use mordhau_core::data::{RecordsJsonExe, SpecSource};
use mordhau_core::ue::{FQuat, FTransform, FVector};
use std::path::PathBuf;
use std::rc::Rc;

fn spec() -> Option<Rc<mordhau_core::data::Spec>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/spec.json");
    let txt = std::fs::read_to_string(p).ok()?;
    Some(Rc::new(RecordsJsonExe(&txt).load_spec().expect("spec")))
}

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

/// a character at `root` (capsule centre) looking along `yaw`, its BlockCollider `ahead` cm in front at chest height
fn geom(root: FVector, yaw: f32, ahead: f32, ext: FVector) -> FighterGeom {
    let r = yaw.to_radians();
    let rot = FQuat::from_rotator(0.0, yaw, 0.0);
    FighterGeom {
        root,
        camera_loc: FVector::new(root.x, root.y, root.z + 70.0),
        camera_rot: (0.0, yaw),
        block_collider: FTransform::new(rot, FVector::new(root.x + ahead * r.cos(), root.y + ahead * r.sin(), root.z + 30.0)),
        block_extent: ext,
    }
}

#[test]
fn simple_block_angles() {
    let d = geom(FVector::new(0.0, 0.0, 100.0), 0.0, 40.0, FVector::new(30.0, 30.0, 60.0));
    // CheckSimpleBlockDirectional: the direction points from the threat to the camera (camera - attacker), so a threat
    // straight ahead (+X) has direction -X: angle 0
    assert!(check_simple_block_directional(&d, FVector::new(-1.0, 0.0, 0.0), 1.0));
    assert!(!check_simple_block_directional(&d, FVector::new(1.0, 0.0, 0.0), 90.0), "from behind");
    // 45 degrees off: inside 60, outside 30
    let dir = FVector::new(-1.0, 1.0, 0.0);
    assert!(check_simple_block_directional(&d, dir, 60.0));
    assert!(!check_simple_block_directional(&d, dir, 30.0));
    // CheckSimpleBlock: a point in front, and a point on the camera (2D) always blocks
    assert!(check_simple_block(&d, FVector::new(200.0, 10.0, 100.0), 60.0));
    assert!(!check_simple_block(&d, FVector::new(-200.0, 0.0, 100.0), 60.0));
    assert!(check_simple_block(&d, FVector::new(0.0, 0.0, 0.0), 1.0));
}

#[test]
fn line_box_and_forward_parry() {
    let (mn, mx) = (FVector::new(-1.0, -1.0, -1.0), FVector::new(1.0, 1.0, 1.0));
    let inv = |d: FVector| FVector::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
    let (s, e) = (FVector::new(-5.0, 0.2, 0.3), FVector::new(5.0, 0.2, 0.3));
    let d = e - s;
    assert!(line_box_intersection(mn, mx, s, e, d, inv(d)));
    let (s2, e2) = (FVector::new(-5.0, 3.0, 0.3), FVector::new(5.0, 3.0, 0.3));
    let d2 = e2 - s2;
    assert!(!line_box_intersection(mn, mx, s2, e2, d2, inv(d2)), "misses beside the box");
    // TestForwardParry: the BP's (50, 10) box ahead of a defender at the origin facing +X, extent X 30: local X in
    // [-30, 20], Y in [-10, 10]
    let g = geom(FVector::new(0.0, 0.0, 100.0), 0.0, 0.0, FVector::new(30.0, 30.0, 60.0));
    let c = g.block_collider.loc;
    let across = |x: f32, y0: f32, y1: f32| test_forward_parry(&g, (50.0, 10.0), FVector::new(c.x + x, c.y + y0, c.z), FVector::new(c.x + x, c.y + y1, c.z));
    // the exe passes Direction = Start - End (decomp UParryMotion.cpp TestForwardParry: FStack_cc = start - end) to
    // FMath::LineBoxIntersection, which expects End - Start: a slab entered from outside gets a negative time, so a
    // segment crossing the box from outside fails and only a segment STARTING inside the box passes
    assert!(!across(0.0, -50.0, 50.0), "a sweep entering the box from outside (reversed Direction)");
    assert!(across(0.0, 0.0, 50.0), "a segment starting inside the box");
    assert!(across(0.0, 5.0, -50.0), "a segment starting inside the box, either way");
    assert!(!across(25.0, 0.0, 50.0), "past the forward distance (fwd.x - ExtX = 20)");
    assert!(!test_forward_parry(&g, (0.0, 10.0), FVector::new(c.x, c.y - 50.0, c.z), FVector::new(c.x, c.y + 50.0, c.z)), "a zero distance disables it");
}

/// Emits, every sample of fighter A, one swept segment across B's BlockCollider (only while it is enabled) and,
/// optionally, B's "Spine1" body
struct Script {
    a: usize,
    b: usize,
    body: bool,
    block: bool,
}

impl TraceHost for Script {
    fn prepare(&self, _w: &World, _fi: usize) {}
    fn sample(&self, w: &World, fi: usize) -> (Vec<Vec<RawHit>>, FVector, FVector) {
        let g = w.fighters[self.b].geom.unwrap();
        let c = g.block_collider.loc;
        let (s, e) = (FVector::new(c.x, c.y - 50.0, c.z), FVector::new(c.x, c.y + 50.0, c.z));
        if fi != self.a {
            return (Vec::new(), s, e);
        }
        let mut hits = Vec::new();
        if self.block && w.fighters[self.b].block_collider_enabled {
            hits.push(RawHit { t: 0.3, victim: self.b, comp: HitComp::BlockCollider, trace_start: s, trace_end: e });
        }
        if self.body {
            hits.push(RawHit { t: 0.6, victim: self.b, comp: HitComp::Body("Spine1".into()), trace_start: s, trace_end: e });
        }
        (vec![hits], s, e)
    }
}

/// returns (parried, damaged)
fn duel(sp: &Rc<mordhau_core::data::Spec>, parry: bool, block: bool, body: bool) -> (bool, bool) {
    let mut w = World::new(sp.clone(), 1.0 / 120.0);
    let a = w.add_fighter("A", LS, "");
    let b = w.add_fighter("B", LS, "");
    w.fighters[a].geom = Some(geom(FVector::new(0.0, 0.0, 100.0), 0.0, 40.0, FVector::new(30.0, 30.0, 60.0)));
    w.fighters[b].geom = Some(geom(FVector::new(150.0, 0.0, 100.0), 180.0, 40.0, FVector::new(30.0, 30.0, 60.0)));
    w.set_trace_host(Some(Rc::new(Script { a, b, body, block })));
    w.at(0.1, Input::Attack { who: "A".into(), mv: 0, angle: 0.0 });
    w.run_until(0.11);
    let we = w.cur_m(a).unwrap().attack().unwrap().windup_end;
    if parry {
        w.at(we - 0.1, Input::Parry { who: "B".into(), bt: 0 });
    }
    w.run_until(we + 0.3);
    let parried = w.hits.iter().any(|e| e["kind"] == "parry");
    let damaged = w.hits.iter().any(|e| e["kind"] == "hit");
    (parried, damaged)
}

#[test]
fn block_collider_parries_and_memory() {
    let Some(sp) = spec() else { return eprintln!("SKIP: no golden spec") };
    // a timed parry: the sweep crosses the enabled BlockCollider (recorded in BlockedAttacks; TestForwardParry fails for
    // a segment entering from outside), then the body hit of the same sweep is parried from that memory + the angles
    assert_eq!(duel(&sp, true, true, true), (true, false), "BlockCollider hit while parrying");
    // no parry: the BlockCollider is disabled, the body is hit
    assert_eq!(duel(&sp, false, true, true), (false, true), "no parry");
    // parrying, but only the body is swept: no BlockedAttacks memory -> the exe does not parry a body hit
    assert_eq!(duel(&sp, true, false, true), (false, true), "body-only hit with no BlockedAttacks entry");
}

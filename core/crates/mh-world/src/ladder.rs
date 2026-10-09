//! BP_Ladder (Mordhau/Content/Mordhau/Blueprints/Interactables/SiegeEngines/BP_Ladder; sizes BP_Ladder_4m..11m):
//! climbing hand-off (a vehicle: BP_LadderMover_C, ported by mh-character) and the held-use drop / raise with its
//! fall damage. Bytecode: state/world_kismet/BP_Ladder.txt.
//!
//! LadderState: 0 raised, 1 dropped (ExecuteUbergraph@153 toggles 0 <-> 1). While it animates the ladder is not
//! interactable and climbing is prevented (BeginAnimatingLadder @6310..@6438); at normalized time 1 it is again
//! (@3040..@15).

use crate::door::component_rel_location;
use crate::props::Props;
use crate::world::{ActorId, CharView, Queries, WorldEvent};
use crate::V;
use mh_level::xf::{rot_quat, Xf};
use serde_json::json;
use std::collections::BTreeSet;

/// FRichCurve keys (Time, Value, interp, arrive / leave tangents) of a CurveFloat asset
#[derive(Clone, Debug, Default)]
pub struct Curve {
    pub keys: Vec<(f64, f64, String, f64, f64)>,
}

impl Curve {
    pub fn load(pk: &mh_level::Pkgs, path: &str) -> Curve {
        let mut c = Curve::default();
        if path.is_empty() {
            return c;
        }
        for e in pk.load_pkg(path).iter() {
            let keys = e.get("Properties").and_then(|p| p.get("FloatCurve")).and_then(|f| f.get("Keys")).and_then(|k| k.as_array());
            for k in keys.into_iter().flatten() {
                let f = |n: &str| k.get(n).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let m = k.get("InterpMode").and_then(|v| v.as_str()).unwrap_or("RCIM_Linear").to_string();
                c.keys.push((f("Time"), f("Value"), m, f("ArriveTangent"), f("LeaveTangent")));
            }
        }
        c
    }

    /// FRichCurve::Eval: constant before / after; between keys linear, constant, or cubic Bezier with the tangents
    /// scaled by the key spacing (P1 = v1 + leave1 * dt / 3, P2 = v2 - arrive2 * dt / 3)
    pub fn eval(&self, t: f64) -> f64 {
        let k = &self.keys;
        if k.is_empty() {
            return t;
        }
        if t <= k[0].0 {
            return k[0].1;
        }
        let last = k.last().unwrap();
        if t >= last.0 {
            return last.1;
        }
        let i = k.iter().position(|x| x.0 > t).unwrap() - 1;
        let (a, b) = (&k[i], &k[i + 1]);
        let d = b.0 - a.0;
        let u = if d > 0.0 { (t - a.0) / d } else { 0.0 };
        match a.2.as_str() {
            "RCIM_Constant" => a.1,
            "RCIM_Linear" => a.1 + (b.1 - a.1) * u,
            _ => {
                let (p0, p1, p2, p3) = (a.1, a.1 + a.4 * d / 3.0, b.1 - b.3 * d / 3.0, b.1);
                let v = 1.0 - u;
                v * v * v * p0 + 3.0 * v * v * u * p1 + 3.0 * v * u * u * p2 + u * u * u * p3
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct LadderInfo {
    pub state: u8,
    pub prevent_climbing: bool,
    pub start: V,
    pub end: V,
    pub exit: V,
}

#[derive(Clone, Debug)]
pub struct Ladder {
    pub state: u8,
    pub change_time: f64,
    pub animating: bool,
    pub interactable: bool,
    pub prevent_climbing: bool,
    pub can_drop_and_raise: bool,
    pub raise_duration: f64,
    pub drop_duration: f64,
    pub drop_damage: f64,
    pub raise_curve: Curve,
    pub drop_curve: Curve,
    /// LadderStart / LadderEnd / LadderExit relative locations (component templates, ladder-local cm)
    pub start: V,
    pub end: V,
    pub exit: V,
    /// DroppedMesh's relative transform (ReceiveBeginPlay @3183) and the raised one (MakeTransform(0, (0, -90, 0)))
    pub dropped_rel: Xf,
    pub raised_rel: Xf,
    pub fall_ignore: BTreeSet<u32>,
    /// HasAuthority (the fall damage, @3917)
    pub authority: bool,
}

pub fn comp_rel_xf(pk: &mh_level::Pkgs, chain: &[String], comp: &str) -> Option<Xf> {
    for cls in chain {
        for e in pk.load_pkg(cls).iter() {
            if e.get("Name").and_then(|v| v.as_str()) == Some(&format!("{comp}_GEN_VARIABLE")) {
                let p = e.get("Properties").and_then(|p| p.as_object()).cloned().unwrap_or_default();
                return Some(mh_level::xf::rel_xf(&p));
            }
        }
    }
    None
}

impl Ladder {
    pub fn new(p: &Props, pk: &mh_level::Pkgs, chain: &[String]) -> Ladder {
        let start_raised = p.b("StartAsRaised");
        let can = p.b("CanDropAndRaise");
        Ladder {
            // ReceiveBeginPlay @3312..@3357: with CanDropAndRaise, a ladder that does not StartAsRaised is set to
            // state 1, dropped (@280)
            state: if can && !start_raised { 1 } else { 0 },
            change_time: -1000.0, // OnRep_LadderState@112: changes in the first 5 s skip the animation
            animating: false,
            interactable: p.get("bIsInteractable").and_then(|v| v.as_bool()).unwrap_or(true),
            prevent_climbing: p.b("bPreventClimbing"),
            can_drop_and_raise: can,
            raise_duration: p.f("RaiseDuration"),
            drop_duration: p.f("DropDuration"),
            drop_damage: p.f("DropDamage"),
            raise_curve: Curve::load(pk, mh_level::level::pkg_of(&p.obj("RaiseCurve"))),
            drop_curve: Curve::load(pk, mh_level::level::pkg_of(&p.obj("DropCurve"))),
            start: component_rel_location(pk, chain, "LadderStart").unwrap_or([0.0; 3]),
            end: component_rel_location(pk, chain, "LadderEnd").unwrap_or([0.0; 3]),
            exit: component_rel_location(pk, chain, "LadderExit").unwrap_or([0.0; 3]),
            dropped_rel: comp_rel_xf(pk, chain, "DroppedMesh").unwrap_or(mh_level::xf::IDENTITY),
            raised_rel: Xf::trs([0.0; 3], rot_quat(Some(&json!({"Pitch": 0.0, "Yaw": -90.0, "Roll": 0.0}))), [1.0; 3]),
            fall_ignore: BTreeSet::new(),
            authority: true,
        }
    }

    pub fn info(&self, xf: &Xf) -> LadderInfo {
        LadderInfo { state: self.state, prevent_climbing: self.prevent_climbing, start: xf.apply(self.start), end: xf.apply(self.end), exit: xf.apply(self.exit) }
    }

    /// CanInteract: interactable, LadderState 0 and the character allows vehicles; then (locally controlled) the
    /// character is above LadderEnd - 75 or in front of the ladder (actor-local X < 10)
    pub fn can_interact(&self, xf: &Xf, ch: &CharView) -> bool {
        if !self.interactable || self.state != 0 || !ch.allow_vehicles {
            return false;
        }
        let end = xf.apply(self.end);
        if ch.location[2] > end[2] - 75.0 {
            return true;
        }
        match xf.inverse() {
            Some(inv) => inv.apply(ch.location)[0] < 10.0,
            None => false,
        }
    }

    /// The ladder as mh-character's ladder mover reads it (exe_ladder::LadderInfo): LadderStart / LadderEnd /
    /// LadderExit world locations (K2_GetComponentLocation, f32) and the actor's yaw (K2_GetActorRotation)
    pub fn character_info(&self, xf: &Xf) -> mh_character::exe_ladder::LadderInfo {
        let f = |p: V| mh_character::uemath::v(p[0] as f32, p[1] as f32, p[2] as f32);
        mh_character::exe_ladder::LadderInfo {
            start: f(xf.apply(self.start)),
            end: f(xf.apply(self.end)),
            exit: f(xf.apply(self.exit)),
            yaw: actor_yaw(xf) as f32,
        }
    }

    /// OnInteractionStart @7232..@8034: a BP_LadderMover_C at ClosestPointOnLine(LadderStart, LadderEnd, character
    /// location) (@7410; FMath::ClosestPointOnLine rva=0x18a2a80, the segment-clamped one, mh-character's
    /// `closest_point_on_line`) with the ladder's rotation (@7465), Ladder = self + OnRep_Ladder (@7928..@7998: the
    /// mover's SetMovementLine), then BP_VehicleLadderMover.StartDriving(character) (@8034). The host builds the mover
    /// with `mh_character::ExeMovement::new_ladder_mover(records, info, character location)` from the event's `info`
    pub fn mount(&mut self, id: ActorId, xf: &Xf, ch: &CharView, ev: &mut Vec<WorldEvent>) {
        let info = self.character_info(xf);
        let loc = mh_character::uemath::v(ch.location[0] as f32, ch.location[1] as f32, ch.location[2] as f32);
        let p = mh_character::exe_ladder::closest_point_on_line(info.start, info.end, loc);
        let mut m = *xf;
        for (r, c) in [p.x, p.y, p.z].iter().enumerate() {
            m.m[r][3] = *c as f64;
        }
        ev.push(WorldEvent::LadderMount {
            char: ch.id,
            ladder: id,
            mover_xf: m,
            start: xf.apply(self.start),
            end: xf.apply(self.end),
            exit: xf.apply(self.exit),
            mover_class: "BP_LadderMover_C",
            info,
        });
    }

    /// OnRep_LadderState on a client: the replicated state, then BeginAnimatingLadder as `toggle` does
    pub fn on_rep_state(&mut self, s: u8, now: f64) {
        if s == self.state {
            return;
        }
        self.state = s;
        self.change_time = now;
        self.animating = true;
        self.interactable = false;
        self.prevent_climbing = true;
    }

    /// OnHeldInteractionStart -> ToggleLadderState: LadderState 0 <-> 1, then (OnRep) BeginAnimatingLadder
    pub fn toggle(&mut self, now: f64) {
        if !self.can_drop_and_raise || self.animating {
            return;
        }
        self.state = if self.state == 0 { 1 } else { 0 };
        self.change_time = now;
        self.animating = true;
        self.interactable = false;
        self.prevent_climbing = true;
    }

    /// ReceiveTick @8145 -> @3396: normalized time over RaiseDuration (state 0) / DropDuration (state 1); the mesh at
    /// TLerp(dropped, raised, RaiseCurve(t)) or TLerp(raised, dropped, DropCurve(t)); while dropping past t 0.333,
    /// characters under it (ladder-local |Y| < 50, X < -10; alive, not ragdolling) take DropDamage and Trip, once per
    /// drop (FallIgnoreActors); at t 1 the animation ends
    pub fn tick(&mut self, id: ActorId, xf: &Xf, now: f64, q: &dyn Queries, chars: &[CharView], ev: &mut Vec<WorldEvent>) {
        if !self.animating {
            return;
        }
        let dur = if self.state == 0 { self.raise_duration } else { self.drop_duration };
        let t = if dur > 0.0 { ((now - self.change_time) / dur).clamp(0.0, 1.0) } else { 1.0 };
        let rel = if self.state == 0 {
            lerp_xf(&self.dropped_rel, &self.raised_rel, self.raise_curve.eval(t))
        } else {
            lerp_xf(&self.raised_rel, &self.dropped_rel, self.drop_curve.eval(t))
        };
        ev.push(WorldEvent::ComponentTransform { actor: id, component: "LadderMesh", rel });
        if self.authority && self.state == 1 && t > 0.333 {
            if let Some(inv) = xf.inverse() {
                for c in q.overlapping_chars(id) {
                    let Some(v) = chars.iter().find(|v| v.id == c) else { continue };
                    if v.dead || v.ragdoll_falling || self.fall_ignore.contains(&c) {
                        continue;
                    }
                    let l = inv.apply(v.location);
                    if l[1].abs() < 50.0 && l[0] < -10.0 {
                        ev.push(WorldEvent::DamageCharacter { char: c, amount: self.drop_damage });
                        ev.push(WorldEvent::Trip { char: c });
                        self.fall_ignore.insert(c);
                    }
                }
            }
        }
        if t >= 1.0 {
            self.fall_ignore.clear();
            self.animating = false;
            self.interactable = true;
            self.prevent_climbing = false;
        }
    }
}

/// The actor's yaw in degrees (FRotator from the X axis: atan2(X.y, X.x))
pub fn actor_yaw(xf: &Xf) -> f64 {
    xf.m[1][0].atan2(xf.m[0][0]).to_degrees()
}

/// UKismetMathLibrary::TLerp (EInterpolationMode QuatInterp = 0): translation / scale lerped, rotation slerped; done
/// on the matrices' decomposition [rotation via normalized lerp of the basis, UNCONFIRMED vs slerp: equal at 0 / 1]
fn lerp_xf(a: &Xf, b: &Xf, t: f64) -> Xf {
    let mut m = [[0.0; 4]; 3];
    for r in 0..3 {
        for c in 0..4 {
            m[r][c] = a.m[r][c] + (b.m[r][c] - a.m[r][c]) * t;
        }
    }
    Xf { m }
}

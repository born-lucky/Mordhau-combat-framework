//! Gates and other driven movers: BP_SwitchInteractable -> BP_ProgressDriver (levers, wall cranks, wheels; and their
//! BP_SlaveProgressDriver copies) driving BP_BaseProgressActor subclasses (BP_StaticMeshProgressActor: BP_CastleGate,
//! BP_Porticulis; BP_SceneProgressActor: trap doors, iron maiden doors, spike / circle-saw actors). Natives:
//! AProgressDriver (ctor rva 0x1642fc0, AMordhauActor + vtables only) and AProgressActor (ctor 0x1642f80; BeginPlay
//! 0x164ae30 / Tick 0x166e8b0 forward to the parent). Bytecode: state/world_kismet/BP_SwitchInteractable.txt,
//! BP_ProgressDriver.txt, BP_SlaveProgressDriver.txt, BP_BaseProgressActor.txt, BP_StaticMeshProgressActor.txt,
//! BP_SceneProgressActor.txt.
//!
//! The switch holds a bool Value (toggled by use, ToggleValue@83); the driver eases SmoothedValue towards Value at
//! RaiseSpeed / LowerSpeed (UMordhauUtilityLibrary::FInterpConstantToSeparate rva 0x161c990) every tick while they
//! differ and hands it to every TargetProgressActor (ProgressUpdatedInternal) and slave crank (UpdateProgress); the
//! progress actor maps it through its raise / lower curve and lerps its moving component (TLerp) from the
//! BeginPlay relative transform to its target transform. Moving that component moves its collision.
//! Not ported: BP_SpikeProgressActor's Box overlap (ProcessHit: 1000 damage via MordhauTakeDamage + random dismember)
//! is the character side's; it moves as a scene progress actor here.

use crate::ladder::Curve;
use crate::props::Props;
use crate::world::{ActorId, WorldEvent};
use crate::V;
use mh_level::xf::Xf;
use serde_json::Value;

/// UMordhauUtilityLibrary::FInterpConstantToSeparate rva 0x161c990: the speed is DecreaseSpeed when Target < Current;
/// then FMath::FInterpConstantTo (dist^2 < 1e-8 -> Target; step clamped to +-dt*speed)
pub fn finterp_constant_to_separate(current: f64, target: f64, dt: f64, inc: f64, dec: f64) -> f64 {
    let speed = if target < current { dec } else { inc };
    let d = target - current;
    if d * d < 1e-8 {
        return target;
    }
    let step = dt * speed;
    current + d.clamp(-step, step)
}

/// An FTransform as the Kismet nodes see it (translation, rotation quaternion x y z w, scale)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tr {
    pub t: V,
    pub q: [f64; 4],
    pub s: V,
}

impl Default for Tr {
    fn default() -> Tr {
        Tr { t: [0.0; 3], q: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3] }
    }
}

fn n(v: Option<&Value>, k: &str, d: f64) -> f64 {
    v.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).unwrap_or(d)
}

impl Tr {
    /// a serialized FTransform {Rotation{X,Y,Z,W}, Translation, Scale3D}
    pub fn from_json(v: Option<&Value>) -> Tr {
        let (r, t, s) = (v.and_then(|v| v.get("Rotation")), v.and_then(|v| v.get("Translation")), v.and_then(|v| v.get("Scale3D")));
        Tr { t: [n(t, "X", 0.0), n(t, "Y", 0.0), n(t, "Z", 0.0)], q: [n(r, "X", 0.0), n(r, "Y", 0.0), n(r, "Z", 0.0), n(r, "W", 1.0)], s: [n(s, "X", 1.0), n(s, "Y", 1.0), n(s, "Z", 1.0)] }
    }
    /// a component's relative transform (RelativeLocation / RelativeRotation / RelativeScale3D)
    pub fn from_relative(p: &serde_json::Map<String, Value>) -> Tr {
        let v = |k: &str, d: f64| mh_level::xf::vec3(p.get(k), d);
        Tr { t: v("RelativeLocation", 0.0), q: mh_level::xf::rot_quat(p.get("RelativeRotation")), s: v("RelativeScale3D", 1.0) }
    }
    pub fn xf(&self) -> Xf {
        Xf::trs(self.t, self.q, self.s)
    }
    /// UKismetMathLibrary::TLerp(A, B, Alpha, ELerpInterpolationMode::QuatInterp = 0): FTransform::Blend -> translation
    /// / scale lerped, rotation FQuat::FastLerp (sign-corrected lerp) then normalized (UE 4.26 TransformVectorized.h
    /// Blend / BlendWith)
    pub fn lerp(&self, b: &Tr, a: f64) -> Tr {
        let l = |x: V, y: V| [x[0] + (y[0] - x[0]) * a, x[1] + (y[1] - x[1]) * a, x[2] + (y[2] - x[2]) * a];
        let dot: f64 = (0..4).map(|i| self.q[i] * b.q[i]).sum();
        let bias = if dot >= 0.0 { 1.0 } else { -1.0 };
        let mut q = [0.0; 4];
        for i in 0..4 {
            q[i] = b.q[i] * a * bias + self.q[i] * (1.0 - a);
        }
        let len = q.iter().map(|c| c * c).sum::<f64>().sqrt();
        if len > 0.0 {
            q = q.map(|c| c / len);
        }
        Tr { t: l(self.t, b.t), q, s: l(self.s, b.s) }
    }
    /// BreakRotator of the rotation (UE FQuat::Rotator): (roll, pitch, yaw) degrees
    pub fn rotator(&self) -> V {
        let [x, y, z, w] = self.q;
        let sing = z * x - w * y;
        let yy = 2.0 * (w * z + x * y);
        let yx = 1.0 - 2.0 * (y * y + z * z);
        let yaw = yy.atan2(yx).to_degrees();
        let th = 0.4999995;
        if sing < -th {
            let pitch = -90.0;
            let roll = norm_axis(-yaw - 2.0 * x.atan2(w).to_degrees());
            [roll, pitch, yaw]
        } else if sing > th {
            let pitch = 90.0;
            let roll = norm_axis(yaw - 2.0 * x.atan2(w).to_degrees());
            [roll, pitch, yaw]
        } else {
            let pitch = (2.0 * sing).asin().to_degrees();
            let roll = (-2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y)).to_degrees();
            [roll, pitch, yaw]
        }
    }
}

fn norm_axis(a: f64) -> f64 {
    let mut a = a % 360.0;
    if a > 180.0 {
        a -= 360.0;
    } else if a < -180.0 {
        a += 360.0;
    }
    a
}

/// BP_SwitchInteractable + BP_ProgressDriver state (also used for a slave: only the master toggles)
#[derive(Clone, Debug)]
pub struct Driver {
    pub value: bool,
    pub smoothed: f64,
    pub tick_enabled: bool,
    pub interactable: bool,
    /// RetriggerableDelay(MinDelayBetweenUses) -> bIsInteractable = true (@244 -> @146)
    pub interactable_at: Option<f64>,
    pub targets: Vec<ActorId>,
    pub slaves: Vec<ActorId>,
}

impl Driver {
    pub fn new(p: &Props) -> Driver {
        Driver {
            value: p.b("Value"),
            smoothed: p.f("SmoothedValue"),
            tick_enabled: true,
            interactable: p.get("bIsInteractable").and_then(|v| v.as_bool()).unwrap_or(true),
            interactable_at: None,
            targets: vec![],
            slaves: vec![],
        }
    }

    fn v(&self) -> f64 {
        self.value as u8 as f64
    }

    /// OnRep_Value on a client: the replicated Value, OnValueToggled (tick on) and PreventInteraction
    pub fn on_rep_value(&mut self, p: &Props, v: bool, now: f64) {
        if v != self.value {
            self.value = !v;
            self.toggle(p, now);
        }
    }

    /// BP_SwitchInteractable ReceiveBeginPlay (@213 -> @158): authority with StartInverted -> ToggleValue
    pub fn begin_play(&mut self, p: &Props, now: f64) {
        if p.b("StartInverted") {
            self.toggle(p, now);
        }
    }

    /// ToggleValue (@83): Value = !Value; OnRep_Value: OnValueToggled (BP_ProgressDriver @3560: SetActorTickEnabled
    /// (true)), and with MinDelayBetweenUses not ~0 PreventInteraction (@233: bIsInteractable false until the
    /// retriggerable delay ends)
    pub fn toggle(&mut self, p: &Props, now: f64) {
        self.value = !self.value;
        self.tick_enabled = true;
        let d = p.f("MinDelayBetweenUses");
        if (d - 0.0).abs() > 9.999999974752427e-07 {
            self.interactable = false;
            self.interactable_at = Some(now + d);
        }
    }

    /// BP_ProgressDriver:CanInteract: at rest (SmoothedValue == Value) not with PreventInteractRaised (raised) /
    /// PreventInteractLowered (lowered); moving only with CanInterruptRaising / CanInterruptLowering; then the
    /// parent's (bIsInteractable) [AMordhauActor CanInteract = bIsInteractable, UNCONFIRMED as in BP_Door]
    pub fn can_interact(&self, p: &Props) -> bool {
        let ok = if self.smoothed == self.v() {
            !((p.b("PreventInteractRaised") && self.value) || (p.b("PreventInteractLowered") && !self.value))
        } else {
            (self.value && p.b("CanInterruptRaising")) || (!self.value && p.b("CanInterruptLowering"))
        };
        ok && self.interactable
    }

    /// ReceiveTick (@2655..@3581): at rest -> tick off; moving -> SmoothedValue eased towards Value (@3174); either
    /// way SmoothedValue goes to every TargetProgressActor and slave (@1224 / @15); then the auto interact
    /// (AutoInteractRaised / AutoInteractLowered at rest -> ToggleValue, @3279..@3545). Returns the SmoothedValue
    /// handed out
    pub fn tick(&mut self, p: &Props, dt: f64, now: f64) -> Option<f64> {
        if let Some(t) = self.interactable_at {
            if now >= t {
                self.interactable_at = None;
                self.interactable = true;
            }
        }
        if !self.tick_enabled {
            return None;
        }
        if self.smoothed == self.v() {
            self.tick_enabled = false;
        } else {
            self.smoothed = finterp_constant_to_separate(self.smoothed, self.v(), dt, p.f("RaiseSpeed"), p.f("LowerSpeed"));
        }
        let out = self.smoothed;
        let auto = (self.value && p.b("AutoInteractRaised")) || (!self.value && p.b("AutoInteractLowered"));
        if self.smoothed == self.v() && auto {
            self.toggle(p, now);
        }
        Some(out)
    }
}

/// BP_BaseProgressActor with its moving component
#[derive(Clone, Debug)]
pub struct ProgressActor {
    pub last_progress: f64,
    /// "StaticMesh" (BP_StaticMeshProgressActor) or "Holder" (BP_SceneProgressActor)
    pub component: &'static str,
    /// the component's relative transform at BeginPlay (StartStaticMeshTransform / StartTransform, @10..@60)
    pub start: Tr,
    /// TargetStaticMeshTransform / TargetTransform
    pub target: Tr,
    pub use_absolute_rotation: bool,
    pub absolute_rotation: V,
    pub raise_curve: Option<Curve>,
    pub lower_curve: Option<Curve>,
    pub progress: f64,
    pub rel: Tr,
}

impl ProgressActor {
    pub fn new(pk: &mh_level::Pkgs, p: &Props, chain: &[String], component: &'static str) -> ProgressActor {
        // the placed component's relative transform (the actor's instance component export over its template), else
        // the class template's
        let start = pk
            .obj(p.get(component))
            .map(|o| Tr::from_relative(&pk.props(&o)))
            .or_else(|| {
                chain.iter().find_map(|cls| {
                    pk.load_pkg(cls)
                        .iter()
                        .find(|e| e.get("Name").and_then(|v| v.as_str()) == Some(&format!("{component}_GEN_VARIABLE")))
                        .map(|e| Tr::from_relative(e.get("Properties").and_then(|p| p.as_object()).unwrap_or(&serde_json::Map::new())))
                })
            })
            .unwrap_or_default();
        let target = Tr::from_json(p.get(if component == "StaticMesh" { "TargetStaticMeshTransform" } else { "TargetTransform" }));
        let curve = |k: &str| {
            let path = p.obj(k);
            (!path.is_empty()).then(|| Curve::load(pk, mh_level::level::pkg_of(&path)))
        };
        let ar = p.get("AbsoluteRotation");
        ProgressActor {
            last_progress: p.f("LastProgress"),
            component,
            start,
            target,
            use_absolute_rotation: p.b("UseAbsoluteRotation"),
            absolute_rotation: [n(ar, "X", 0.0), n(ar, "Y", 0.0), n(ar, "Z", 0.0)],
            raise_curve: curve("ProgressCurveRaise"),
            lower_curve: curve("ProgressCurveLower"),
            progress: 0.0,
            rel: start,
        }
    }

    /// ProgressUpdatedInternal (BP_BaseProgressActor): nothing when Progress ~ LastProgress (1e-6); rising -> the
    /// raise curve's value (or Progress without one), falling -> the lower curve's; ProgressUpdated; LastProgress
    pub fn progress_updated_internal(&mut self, id: ActorId, progress: f64, ev: &mut Vec<WorldEvent>) {
        if (progress - self.last_progress).abs() <= 9.999999974752427e-07 {
            return;
        }
        let c = if progress >= self.last_progress { &self.raise_curve } else { &self.lower_curve };
        let v = c.as_ref().map(|c| c.eval(progress)).unwrap_or(progress);
        self.progress_updated(id, v, ev);
        self.last_progress = progress;
    }

    /// ProgressUpdated: BP_StaticMeshProgressActor (@92..@681) StaticMesh = TLerp(start, target, p), with
    /// UseAbsoluteRotation the rotation instead MakeRotator(VLerp((roll, pitch, yaw) of start, AbsoluteRotation, p));
    /// BP_SceneProgressActor (@92..@149) Holder = TLerp(start, target, p)
    pub fn progress_updated(&mut self, id: ActorId, p: f64, ev: &mut Vec<WorldEvent>) {
        self.progress = p;
        let mut r = self.start.lerp(&self.target, p);
        if self.component == "StaticMesh" && self.use_absolute_rotation {
            let s = self.start.rotator();
            let a = self.absolute_rotation;
            let v = [s[0] + (a[0] - s[0]) * p, s[1] + (a[1] - s[1]) * p, s[2] + (a[2] - s[2]) * p];
            // MakeRotator(Roll = X, Pitch = Y, Yaw = Z)
            r.q = mh_level::xf::rot_quat(Some(&serde_json::json!({"Roll": v[0], "Pitch": v[1], "Yaw": v[2]})));
        }
        self.rel = r;
        ev.push(WorldEvent::ComponentTransform { actor: id, component: self.component, rel: r.xf() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FInterpConstantToSeparate rva 0x161c990: raise / lower speeds, snap within 1e-4
    #[test]
    fn separate_speeds() {
        assert!((finterp_constant_to_separate(0.0, 1.0, 0.5, 0.2, 2.0) - 0.1).abs() < 1e-12);
        assert!((finterp_constant_to_separate(1.0, 0.0, 0.1, 0.2, 2.0) - 0.8).abs() < 1e-12);
        assert_eq!(finterp_constant_to_separate(0.99995, 1.0, 0.0, 0.0, 0.0), 1.0);
    }

    /// TLerp halfway: translation midpoint, 90 deg yaw -> 45
    #[test]
    fn tlerp() {
        let a = Tr::default();
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let b = Tr { t: [0.0, 0.0, 400.0], q: [0.0, 0.0, h, h], s: [1.0; 3] };
        let m = a.lerp(&b, 0.5);
        assert!((m.t[2] - 200.0).abs() < 1e-9);
        assert!((m.rotator()[2] - 45.0).abs() < 1e-9, "{:?}", m.rotator());
    }
}

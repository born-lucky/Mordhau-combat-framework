//! The airborne states of AB_MordhauCharacterAnimation (fidelity-audit r6): the "UpperBody" state machine
//! (BakedStateMachines[9]: Idle / Airborne / Jump / Land) and the "LowerBody" machine's Jump / Falling states
//! (BakedStateMachines[1]: Ground / Jump / Falling). The Ground / Idle poses stay lower.rs / the blend space path.
//!
//! Anim instance inputs, UAdvancedCharacterAnimInstance::NativeUpdateAnimation rva=0x148aa10 (decomp 258-276): bJumped =
//! the character's jumped flag, consumed (cleared) by the update (a one-update pulse; mh-sim: ExeMovement::jumps of the
//! frame, the "jump" event); bIsAirborne = the character's IsAirborne vcall; AirborneTime = 0 on the update that becomes
//! airborne, else AirborneTime + DeltaSeconds.
//!
//! UpperBody transitions (state transition lists in evaluation order; rules from the AnimBlueprint's
//! EvaluateGraphExposedInputs bytecode, state/world_kismet/AB_MordhauCharacterAnimation.txt):
//!   Idle -> Jump      0.15 s  bJumped (TransitionResult_134 bound to bJumped)
//!   Idle -> Airborne  0.5 s   bIsAirborne && !bJumped && AirborneTime > 0.4 (@14748-14887)
//!   Jump -> Idle      0.2 s   !bIsAirborne && !bJumped && GetInstanceStateWeight(UpperBody, Jump) < 0.95 (@19488-19694)
//!   Jump -> Land      0.2 s   !bIsAirborne && !bJumped (@15187-15283)
//!   Jump -> Airborne  0.4 s   elapsed in Jump > 0.8 && bIsAirborne (@14920-15025)
//!   Airborne -> Land  0.15 s  !bIsAirborne && !bJumped (@15058-15154)
//!   Land -> Idle      0.8 s   automatic rule (remaining time of the Land player <= the crossfade)
//! State poses: Airborne SequencePlayer_54 FallingAnimation (loop, rate 1); Jump SequencePlayer_55 JumpAnimation (rate 1.3,
//! no loop); Land SequencePlayer_56 LandAnimation (rate 1.3, no loop); bAlwaysResetOnEntry on Jump / Land.
//! LowerBody transitions:
//!   Ground -> Jump    0.1 s   bJumped (TransitionResult_1)
//!   Ground -> Falling 0.2 s   bIsAirborne && !bJumped (@20594-20661)
//!   Jump -> Ground    0.3 s   !bIsAirborne && !bJumped (@20694-20790)
//!   Jump -> Falling   0.2 s   elapsed in Jump > 0.7 && bIsAirborne (@20898-21003)
//!   Falling -> Ground 0.3 s   !bIsAirborne (TransitionResult_5 bound to bIsAirborne; the negation: UNCONFIRMED, the
//!                             fast-path compile keeps no explicit Not)
//! Falling pose (StateResult_4): SequencePlayer_10 Locomotion/Falling (loop) -> ModifyBone_38 Hips translation (0, 0, -15)
//! additive (component space) -> CopyBone_6 / _7 (LeftFoot / RightFoot translation -> VB Position_LeftFoot / _RightFoot).
//! Jump pose (StateResult_3, fidelity-audit r7): TwoWayBlend_1 (Alpha = Velocity mapped 20..60 -> 0..1, clamped) of
//!   A: SequencePlayer_8 Locomotion/Jump (once) -> ModifyBone_36 RightUpLeg roll -30 -> ModifyBone_37 RightLeg roll 30
//!   B: SequencePlayer_9 Locomotion/JumpForward (once) -> ModifyBone_34 Hips roll = MapRangeClamped(AirborneTime, 0, 0.6,
//!      -0.75, 1) x 20 (@19727-19873) -> ModifyBone_35 LeftUpLeg roll 20
//! (all rotation BMM_Additive, BCS_ComponentSpace), then ModifyBone_33 Hips (@19914-20478): R = Yaw(Direction) x
//! Roll(MapRangeClamped(AirborneTime, 0, 0.6, -1, 1) x -30 x MapRangeClamped(Velocity, 0, 90, 0, 1)) x Yaw(-Direction)
//! (ComposeRotators(A, B) = B.Quat * A.Quat), broken to (Roll, Pitch, Yaw) and remade as (min(Roll, 0), Pitch, 0); then
//! CopyBone_4 / _5 as in Falling. Crossfades: HermiteCubic (the transitions' BlendMode), FAnimNode_StateMachine engine rules
//! (one transition per update, a new transition interrupts with the current weights: UNCONFIRMED as compiled).

use crate::animgraph::{alpha_blend, blend_transforms, AnimAssets};
use crate::pose::Skeleton;
use crate::procedural::CsPose;
use mordhau_core::combat::geometry::quat_rotator;
use mordhau_core::data::Spec;
use mordhau_core::ue::{FQuat, FTransform, FVector};

pub const LB_JUMP_CLIP: &str = "Mordhau/Content/Mordhau/Animations/RawClips/Locomotion/Jump";
pub const LB_JUMP_FORWARD_CLIP: &str = "Mordhau/Content/Mordhau/Animations/RawClips/Locomotion/JumpForward";

/// UKismetMathLibrary::MapRangeClamped
fn map_clamped(v: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    let t = if b != a { ((v - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
    c + (d - c) * t
}

/// FRotator from MakeRotator(Roll, Pitch, Yaw) as a quaternion
fn rot(roll: f64, pitch: f64, yaw: f64) -> FQuat {
    FQuat::from_rotator(pitch as f32, yaw as f32, roll as f32)
}

/// CopyBone (translation only, component space) SourceBone -> TargetBone
fn copy_translation(p: &mut CsPose, src: &str, dst: &str) {
    if let (Some(s), Some(d)) = (p.sk.find(src), p.sk.find(dst)) {
        let mut n = p.cs(d);
        n.loc = p.cs(s).loc;
        p.blend_cs(d, n, 1.0);
    }
}

fn roll_add(p: &mut CsPose, bone: &str, roll: f32) {
    if let Some(b) = p.sk.find(bone) {
        p.modify_add(b, (0.0, 0.0, roll), None, false, 1.0);
    }
}

/// HermiteCubic in mordhau-core's alpha_blend numbering (animgraph.rs / additive.rs BLEND_OPTION)
const HERMITE: i32 = 2;

#[derive(Clone, Debug, Default)]
struct Fade {
    from: usize,
    to: usize,
    start: f64,
    dur: f64,
}

/// A small baked state machine: current state, its entry time, the running crossfades
#[derive(Clone, Debug, Default)]
pub struct Machine {
    pub current: usize,
    pub entered: f64,
    /// entry time of every state (players restart on entry for bAlwaysResetOnEntry states)
    pub entries: Vec<f64>,
    fades: Vec<Fade>,
}

impl Machine {
    fn new(n: usize) -> Machine {
        Machine { current: 0, entered: 0.0, entries: vec![0.0; n], fades: Vec::new() }
    }
    fn go(&mut self, to: usize, dur: f64, t: f64) {
        self.fades.push(Fade { from: self.current, to, start: t, dur });
        self.current = to;
        self.entered = t;
        self.entries[to] = t;
    }
    fn alpha(spec: &Spec, f: &Fade, t: f64) -> f64 {
        if f.dur <= 0.0 {
            return 1.0;
        }
        alpha_blend(spec, HERMITE, ((t - f.start) / f.dur).clamp(0.0, 1.0), "")
    }
    fn tidy(&mut self, spec: &Spec, t: f64) {
        for i in (0..self.fades.len()).rev() {
            if Self::alpha(spec, &self.fades[i], t) >= 1.0 {
                self.fades.drain(0..=i);
                return;
            }
        }
    }
    /// every state's weight now (sums to 1)
    pub fn weights(&self, spec: &Spec, n: usize, t: f64) -> Vec<f64> {
        let mut w = vec![0.0; n];
        if self.fades.is_empty() {
            w[self.current] = 1.0;
            return w;
        }
        w[self.fades[0].from] = 1.0;
        for f in &self.fades {
            let a = Self::alpha(spec, f, t);
            for x in w.iter_mut() {
                *x *= 1.0 - a;
            }
            w[f.to] += a;
        }
        w
    }
    pub fn elapsed(&self, t: f64) -> f64 {
        t - self.entered
    }
}

/// UpperBody states
pub const UB_IDLE: usize = 0;
pub const UB_AIRBORNE: usize = 1;
pub const UB_JUMP: usize = 2;
pub const UB_LAND: usize = 3;
/// LowerBody states
pub const LB_GROUND: usize = 0;
pub const LB_JUMP: usize = 1;
pub const LB_FALLING: usize = 2;

/// the equipment's airborne animations (UMordhauAnimInstance::UpdateEquipmentData rva=0x151c4d0 decomp 6570-6720)
#[derive(Clone, Debug, Default)]
pub struct AirAnims {
    pub jump: String,
    pub falling: String,
    pub land: String,
}

pub const LB_FALLING_CLIP: &str = "Mordhau/Content/Mordhau/Animations/RawClips/Locomotion/Falling";

#[derive(Clone, Debug, Default)]
pub struct AirState {
    pub ub: Machine,
    pub lb: Machine,
    pub airborne: bool,
    pub airborne_time: f64,
    pub anims: AirAnims,
    initialised: bool,
    last_t: f64,
}

impl AirState {
    pub fn new() -> AirState {
        AirState { ub: Machine::new(4), lb: Machine::new(3), ..Default::default() }
    }

    /// one anim update: the inputs, then one transition per machine
    pub fn update(&mut self, spec: &Spec, a: &AnimAssets, t: f64, jumped: bool, airborne_now: bool) {
        if !self.initialised {
            *self = AirState { anims: self.anims.clone(), last_t: t, ..AirState::new() };
            for e in self.ub.entries.iter_mut().chain(self.lb.entries.iter_mut()) {
                *e = t;
            }
            self.ub.entered = t;
            self.lb.entered = t;
            self.initialised = true;
        }
        let dt = (t - self.last_t).max(0.0);
        self.last_t = t;
        if !self.airborne && airborne_now {
            self.airborne_time = 0.0;
        } else {
            self.airborne_time += dt;
        }
        self.airborne = airborne_now;
        let air = self.airborne;
        // UpperBody
        self.ub.tidy(spec, t);
        let jump_w = self.ub.weights(spec, 4, t)[UB_JUMP];
        let next: Option<(usize, f64)> = match self.ub.current {
            UB_IDLE => {
                if jumped {
                    Some((UB_JUMP, 0.15))
                } else if air && self.airborne_time > 0.4 {
                    Some((UB_AIRBORNE, 0.5))
                } else {
                    None
                }
            }
            UB_JUMP => {
                if !air && !jumped && jump_w < 0.95 {
                    Some((UB_IDLE, 0.2))
                } else if !air && !jumped {
                    Some((UB_LAND, 0.2))
                } else if self.ub.elapsed(t) > 0.8 && air {
                    Some((UB_AIRBORNE, 0.4))
                } else {
                    None
                }
            }
            UB_AIRBORNE => (!air && !jumped).then_some((UB_LAND, 0.15)),
            UB_LAND => {
                // the automatic rule: the Land player's remaining time <= the 0.8 s crossfade
                let len = a.clip(&self.anims.land).map(|c| c.sequence_length as f64 / 1.3).unwrap_or(0.0);
                (len - self.ub.elapsed(t) <= 0.8).then_some((UB_IDLE, 0.8))
            }
            _ => None,
        };
        if let Some((s, d)) = next {
            self.ub.go(s, d, t);
        }
        // LowerBody
        self.lb.tidy(spec, t);
        let next: Option<(usize, f64)> = match self.lb.current {
            LB_GROUND => {
                if jumped {
                    Some((LB_JUMP, 0.1))
                } else if air && !jumped {
                    Some((LB_FALLING, 0.2))
                } else {
                    None
                }
            }
            LB_JUMP => {
                if !air && !jumped {
                    Some((LB_GROUND, 0.3))
                } else if self.lb.elapsed(t) > 0.7 && air {
                    Some((LB_FALLING, 0.2))
                } else {
                    None
                }
            }
            LB_FALLING => (!air).then_some((LB_GROUND, 0.3)),
            _ => None,
        };
        if let Some((s, d)) = next {
            self.lb.go(s, d, t);
        }
    }

    fn sample(&self, a: &AnimAssets, sk: &Skeleton, path: &str, since: f64, rate: f64, looping: bool) -> Vec<FTransform> {
        let Some(c) = a.clip(path) else { return sk.ref_local.clone() };
        let len = c.sequence_length as f64;
        let mut tt = since.max(0.0) * rate;
        tt = if looping && len > 0.0 { tt.rem_euclid(len) } else { tt.min(len) };
        a.sample(sk, path, tt, looping)
    }

    /// the UpperBody machine's pose given the Idle state's pose
    /// fp-anim r2: the LowerBody machine's Ground state weight now
    pub fn ground_weight(&self, spec: &Spec, t: f64) -> f64 {
        self.lb.weights(spec, 3, t)[LB_GROUND]
    }

    pub fn upper(&self, spec: &Spec, a: &AnimAssets, sk: &Skeleton, idle: Vec<FTransform>, t: f64) -> Vec<FTransform> {
        let w = self.ub.weights(spec, 4, t);
        if w[UB_IDLE] >= 1.0 - 1e-6 {
            return idle;
        }
        let mut poses: Vec<(Vec<FTransform>, f64)> = Vec::new();
        for (s, &ws) in w.iter().enumerate() {
            if ws <= 1e-6 {
                continue;
            }
            let since = t - self.ub.entries[s];
            let (path, rate, looping) = match s {
                UB_AIRBORNE => (&self.anims.falling, 1.0, true),
                UB_JUMP => (&self.anims.jump, 1.3, false),
                _ => (&self.anims.land, 1.3, false),
            };
            // a missing animation (no equipment: the Unarmed* fields, not ported) keeps the Idle pose (UNCONFIRMED)
            let p = if s == UB_IDLE || a.clip(path).is_none() { idle.clone() } else { self.sample(a, sk, path, since, rate, looping) };
            poses.push((p, ws));
        }
        blend(sk, &poses)
    }

    /// the LowerBody Jump state's pose (module doc); `velocity` / `direction` = the anim instance's Velocity / Direction
    pub fn lower_jump(&self, a: &AnimAssets, sk: &Skeleton, t: f64, velocity: f64, direction: f64) -> Vec<FTransform> {
        let since = t - self.lb.entries[LB_JUMP];
        let mut pa = CsPose { sk, local: self.sample(a, sk, LB_JUMP_CLIP, since, 1.0, false) };
        roll_add(&mut pa, "RightUpLeg", -30.0);
        roll_add(&mut pa, "RightLeg", 30.0);
        let mut pb = CsPose { sk, local: self.sample(a, sk, LB_JUMP_FORWARD_CLIP, since, 1.0, false) };
        roll_add(&mut pb, "Hips", (map_clamped(self.airborne_time, 0.0, 0.6, -0.75, 1.0) * 20.0) as f32);
        roll_add(&mut pb, "LeftUpLeg", 20.0);
        // TwoWayBlend_1: AlphaScaleBiasClamp maps Velocity 20..60 -> 0..1, then the node clamps to [0, 1]
        let alpha = ((velocity - 20.0) / 40.0).clamp(0.0, 1.0);
        let mixed = blend(sk, &[(pa.local, 1.0 - alpha), (pb.local, alpha)]);
        let mut p = CsPose { sk, local: mixed };
        // ModifyBone_33
        let yaw = rot(0.0, 0.0, direction);
        let roll = map_clamped(self.airborne_time, 0.0, 0.6, -1.0, 1.0) * -30.0 * map_clamped(velocity, 0.0, 90.0, 0.0, 1.0);
        let q = yaw.mul(rot(roll, 0.0, 0.0)).mul(rot(0.0, 0.0, -direction));
        let (pitch, _, r) = quat_rotator(q);
        roll_add_full(&mut p, "Hips", (pitch, 0.0, r.min(0.0)));
        copy_translation(&mut p, "LeftFoot", "VB Position_LeftFoot");
        copy_translation(&mut p, "RightFoot", "VB Position_RightFoot");
        p.local
    }

    /// the LowerBody machine's pose given the Ground state's pose; `velocity` / `direction` as lower_jump
    pub fn lower(&self, spec: &Spec, a: &AnimAssets, sk: &Skeleton, ground: Vec<FTransform>, t: f64, velocity: f64, direction: f64) -> Vec<FTransform> {
        let w = self.lb.weights(spec, 3, t);
        if w[LB_GROUND] >= 1.0 - 1e-6 {
            return ground;
        }
        let mut poses = vec![(ground, w[LB_GROUND])];
        if w[LB_JUMP] > 1e-6 {
            poses.push((self.lower_jump(a, sk, t, velocity, direction), w[LB_JUMP]));
        }
        if w[LB_FALLING] > 1e-6 {
            let mut p = CsPose { sk, local: self.sample(a, sk, LB_FALLING_CLIP, t - self.lb.entries[LB_FALLING], 1.0, true) };
            // ModifyBone_38: Hips translation (0, 0, -15) BMM_Additive (component space; children follow)
            if let Some(h) = sk.find("Hips") {
                p.modify_add(h, (0.0, 0.0, 0.0), Some(FVector::new(0.0, 0.0, -15.0)), false, 1.0);
            }
            copy_translation(&mut p, "LeftFoot", "VB Position_LeftFoot");
            copy_translation(&mut p, "RightFoot", "VB Position_RightFoot");
            poses.push((p.local, w[LB_FALLING]));
        }
        blend(sk, &poses)
    }
}

fn roll_add_full(p: &mut CsPose, bone: &str, pyr: (f32, f32, f32)) {
    if let Some(b) = p.sk.find(bone) {
        p.modify_add(b, pyr, None, false, 1.0);
    }
}

fn blend(sk: &Skeleton, poses: &[(Vec<FTransform>, f64)]) -> Vec<FTransform> {
    let ws: Vec<f64> = poses.iter().map(|p| p.1).collect();
    (0..sk.names.len())
        .map(|i| {
            let xs: Vec<FTransform> = poses.iter().map(|p| p.0[i]).collect();
            blend_transforms(&xs, &ws)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the machine weights crossfade and settle (no assets needed)
    #[test]
    fn crossfade_weights() {
        let spec = Spec::default();
        let mut m = Machine::new(4);
        m.go(UB_JUMP, 0.2, 0.0);
        let w = m.weights(&spec, 4, 0.1);
        assert!(w[UB_IDLE] > 0.0 && w[UB_JUMP] > 0.0 && (w.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        m.tidy(&spec, 0.3);
        assert_eq!(m.weights(&spec, 4, 0.3)[UB_JUMP], 1.0);
    }
}

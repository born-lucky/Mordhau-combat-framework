//! The "Additive" state machine of AB_MordhauCharacterAnimation and its ApplyAdditive_2 (parry / parry push / bounce
//! additives over the upper body): the Rust port of godot/game/anim/{additive_override, additive_machine,
//! additive_pose, additive_clip}.gd (rust-combat r4). Every rule keeps the reference's citation; see those files'
//! headers for the full derivations.
//!
//! - AdditiveOverride: AAdvancedCharacter AdditiveOverrideType (+0xacc) / EndTime (+0xad4) / StartTime (+0xad8):
//!   SetAdditiveOverrideType rva=0x14a0320, ResetAdditiveOverrideType rva=0x149ede0 (Type None, End = now + 3600),
//!   AAdvancedCharacter::Tick (End < now -> None); UAdvancedCharacterAnimInstance::NativeUpdateAnimation (decomp
//!   UAdvancedCharacterAnimInstance.cpp 289-323): the anim copy, AdditiveOverrideWeight (0 after 1 s of None).
//! - AdditiveMachine: BakedStateMachines[2] (states, transitions in priority order, CrossfadeDuration, HermiteCubic);
//!   rules = AdditiveOverrideType ==/!= one name (ExecuteUbergraph statements cited per transition). Runtime
//!   (FAnimNode_StateMachine, engine) UNCONFIRMED as in the reference: max 3 transitions per update, first update after
//!   (re)initialisation takes none, crossfades HermiteCubic, a state entered without weight restarts its players.
//! - State poses: Bounce = AttackBounce (rate 1), Parry = ParryAdditive's single clip (rate 1, loop), ParryPush /
//!   AltParryPush = ParryPushAdditive / AltParryPushAdditive (rate 2), HitBounce(Left) = Misc Bounce_(Left_)Additive
//!   (rate 2.5), GenericSway = Interact_Additive (loop), Flinch1P / FlinchTwo1P = Flinch_1p; the flinch blend spaces (BS_FlinchBlend: only
//!   AMordhauVehicle triggers "Flinch" / "FlinchTwo"; a 3P player flinch is OnTookDamage, flinch.rs) and RangedDrawn
//!   are not ported; Disarm plays ToLeft / ToRight by DisarmDirection (MachineInputs::disarm_direction).
//! - Apply: FAnimationRuntime::AccumulateLocalSpaceAdditivePoseInternal VA 0x142e46c20 (w <= 1e-5 nothing; w >=
//!   0.99999: R = w * Add.R, R.w^2 < 1 -> Base.R = R * Base.R; else R = normalize(sign * (1 - w) * I + w * Add.R));
//!   Base.T += w * Add.T (scale not modelled: FTransform here has none). Crossfade blend of two additive poses:
//!   (1 - a) A + a B per bone (sign against the running sum, normalized).

use crate::animgraph::{alpha_blend, AnimAssets};
use crate::pose::Skeleton;
use mordhau_core::data::Spec;
use mordhau_core::ue::{FQuat, FTransform, FVector};

pub const NONE: &str = "None";

/// AdditiveOverride (additive_override.gd)
#[derive(Clone, Debug)]
pub struct AdditiveOverride {
    pub ty: String,
    pub start_time: f64,
    pub end_time: f64,
    pub anim_type: String,
    pub none_time: f64,
    pub weight: f64,
}

impl Default for AdditiveOverride {
    fn default() -> Self {
        AdditiveOverride { ty: NONE.into(), start_time: 0.0, end_time: 0.0, anim_type: NONE.into(), none_time: 0.0, weight: 1.0 }
    }
}

impl AdditiveOverride {
    /// AAdvancedCharacter::SetAdditiveOverrideType rva=0x14a0320
    pub fn set_type(&mut self, t: &str, duration: f64, now: f64) {
        self.ty = t.to_string();
        self.start_time = now;
        self.end_time = now + duration;
    }
    /// AAdvancedCharacter::ResetAdditiveOverrideType rva=0x149ede0
    pub fn reset(&mut self, now: f64) {
        self.ty = NONE.into();
        self.start_time = now;
        self.end_time = now + 3600.0;
    }
    /// AAdvancedCharacter::Tick (decomp AAdvancedCharacter.cpp 3357-3361)
    pub fn tick(&mut self, now: f64) {
        if self.end_time < now {
            self.ty = NONE.into();
        }
    }
    /// UAdvancedCharacterAnimInstance::NativeUpdateAnimation part
    pub fn update_anim(&mut self, dt: f64) {
        self.weight = 1.0;
        if self.ty == self.anim_type {
            if self.anim_type == NONE {
                self.none_time += dt;
                if self.none_time > 1.0 {
                    self.weight = 0.0;
                }
            }
        } else {
            self.anim_type = self.ty.clone();
            if self.anim_type == NONE {
                self.none_time = 0.0;
            }
        }
    }
}

pub const STATES: [&str; 15] = [
    "Identity", "Flinch", "Bounce", "Parry", "ParryPush", "Disarm", "HitBounce", "HitBounceLeft", "RangedDrawn", "FlinchTwo", "Flinch1P", "FlinchTwo1P", "AltParryPush", "ReducedFlinch",
    "GenericSway",
];
const MAX_TRANSITIONS: usize = 3;

/// (from, to, rule ==, AdditiveOverrideType name, CrossfadeDuration) per state in priority order
/// (BakedStateMachines[2].States[i].Transitions; rule at the ubergraph statement in additive_machine.gd)
pub const TRANSITIONS: [(usize, usize, bool, &str, f64); 34] = [
    (0, 8, true, "RangedDrawn", 0.0),     // TransitionResult_6 @21190
    (0, 2, true, "Bounce", 0.0),          // TransitionResult_7 @21265
    (0, 3, true, "Parry", 0.2),           // TransitionResult_8 @21340
    (0, 4, true, "ParryPush", 0.0),       // TransitionResult_9 @21803
    (0, 5, true, "Disarm", 0.0),          // TransitionResult_10 @22100
    (0, 6, true, "HitBounce", 0.0),       // TransitionResult_11 @22429
    (0, 7, true, "HitBounceLeft", 0.0),   // TransitionResult_12 @22712
    (0, 1, true, "Flinch", 0.0),          // TransitionResult_13 @22862
    (0, 9, true, "FlinchTwo", 0.0),       // TransitionResult_14 @23116
    (0, 10, true, "Flinch1P", 0.2),       // TransitionResult_15 @23445
    (0, 11, true, "FlinchTwo1P", 0.2),    // TransitionResult_16 @23728
    (0, 12, true, "AltParryPush", 0.0),   // TransitionResult_17 @23878
    (0, 13, true, "ReducedFlinch", 0.0),  // TransitionResult_18 @24236
    (0, 14, true, "GenericSway", 0.4),    // TransitionResult_19 @24386
    (1, 9, true, "FlinchTwo", 0.1),       // TransitionResult_20 @24640
    (1, 0, false, "Flinch", 0.2),         // TransitionResult_21 @24565
    (2, 0, false, "Bounce", 0.3),         // TransitionResult_22 @24490
    (3, 4, true, "ParryPush", 0.0),       // TransitionResult_23 @24311
    (3, 12, true, "AltParryPush", 0.0),   // TransitionResult_24 @24057
    (3, 0, false, "Parry", 0.2),          // TransitionResult_25 @24161
    (4, 0, false, "ParryPush", 0.4),      // TransitionResult_26 @23982
    (5, 0, false, "Disarm", 0.1),         // TransitionResult_27 @23803
    (6, 0, false, "HitBounce", 0.3),      // TransitionResult_28 @23653
    (7, 0, false, "HitBounceLeft", 0.3),  // TransitionResult_29 @23549
    (8, 0, false, "RangedDrawn", 0.2),    // TransitionResult_30 @23370
    (9, 1, true, "Flinch", 0.1),          // TransitionResult_31 @23295
    (9, 0, false, "FlinchTwo", 0.2),      // TransitionResult_32 @23220
    (10, 11, true, "FlinchTwo1P", 0.2),   // TransitionResult_33 @23041
    (10, 0, false, "Flinch1P", 0.2),      // TransitionResult_34 @22966
    (11, 10, true, "Flinch1P", 0.2),      // TransitionResult_35 @22787
    (11, 0, false, "FlinchTwo1P", 0.2),   // TransitionResult_36 @22637
    (12, 0, false, "AltParryPush", 0.4),  // TransitionResult_37 @22533
    (13, 0, false, "ReducedFlinch", 0.2), // TransitionResult_38 @22354
    (14, 0, false, "GenericSway", 0.2),   // TransitionResult_39 @22279
];
/// EAlphaBlendOption::HermiteCubic on every transition
const BLEND_OPTION: i32 = 2;

/// per state: (player rate, loop)
fn player(s: usize) -> Option<(f64, bool)> {
    Some(match s {
        1 => (1.1, false),
        2 => (1.0, false),
        3 => (1.0, true),
        4 => (2.0, false),
        5 => (1.0, false),
        6 | 7 => (2.5, false),
        8 => (1.0, true),
        9 => (1.1, false),
        10 | 11 => (1.0, false),
        12 => (2.0, false),
        13 => (1.1, false),
        14 => (1.0, true),
        _ => return None,
    })
}
const MISC: &str = "Mordhau/Content/Mordhau/Animations/RawClips/Misc/";
const FLINCH_1P: &str = "Mordhau/Content/Mordhau/Animations/RawClips/Flinch/Flinch_1p";

#[derive(Clone, Debug)]
struct Active {
    from: usize,
    to: usize,
    start: f64,
    dur: f64,
}

/// The dynamic inputs (UMordhauAnimInstance AttackBounce / ParryAdditive / ParryPushAdditive / AltParryPushAdditive)
#[derive(Clone, Debug, Default)]
pub struct MachineInputs {
    pub attack_bounce: String,
    pub parry_additive: String,
    pub parry_push_additive: String,
    pub alt_parry_push_additive: String,
    /// UMordhauAnimInstance BlockDirection: the X of the ParryPushAdditive / AltParryPushAdditive BlendSpacePlayers
    /// (AB_MordhauCharacterAnimation PropertyAccess: BlockDirection -> AnimGraphNode_BlendSpacePlayer_6/_9/_19/_22/_25/_28.X)
    pub block_direction: f64,
    /// UMordhauAnimInstance DisarmDirection (+0x964), set by UDisarmedMotion::OnBegin_Implementation rva=0x165ee70
    /// (decomp 76-93: replicated byte / 255 x 360 - 180): the Disarm state's TwoWayBlend_2 (node 605) picks
    /// A = Disarm_RH_ToRight_Additive / B = Disarm_RH_ToLeft_Additive at Alpha = DisarmDirection < 0 (Kismet
    /// EvaluateGraphExposedInputs for 605, sheets/combat/07)
    pub disarm_direction: f64,
}

/// AdditiveMachine (additive_machine.gd)
#[derive(Clone, Debug, Default)]
pub struct AdditiveMachine {
    pub current: usize,
    state_start: Vec<(usize, f64)>,
    active: Vec<Active>,
    initialised: bool,
    skip_next: bool,
    pub inputs: MachineInputs,
}

/// one bone of an additive pose: (rotation delta, translation delta); absent = identity
pub type AddPose = Vec<Option<(FQuat, FVector)>>;

fn qn(q: FQuat) -> FQuat {
    let l = q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w;
    if l >= 1e-8 {
        let r = 1.0 / l.sqrt();
        FQuat::new(q.x * r, q.y * r, q.z * r, q.w * r)
    } else {
        FQuat::IDENTITY
    }
}

impl AdditiveMachine {
    pub fn reinit(&mut self, t: f64) {
        self.current = 0;
        self.active.clear();
        self.state_start = vec![(0, t)];
        self.initialised = true;
        self.skip_next = true;
    }
    fn weighted(&self, s: usize) -> bool {
        s == self.current || self.active.iter().any(|a| a.from == s || a.to == s)
    }
    fn set_start(&mut self, s: usize, t: f64) {
        self.state_start.retain(|x| x.0 != s);
        self.state_start.push((s, t));
    }
    fn start_of(&self, s: usize, t: f64) -> f64 {
        self.state_start.iter().find(|x| x.0 == s).map(|x| x.1).unwrap_or(t)
    }
    pub fn update(&mut self, spec: &Spec, ty: &str, t: f64) {
        if !self.initialised {
            self.reinit(t);
        }
        self.drop_finished(spec, t);
        if self.skip_next {
            self.skip_next = false;
            return;
        }
        for _ in 0..MAX_TRANSITIONS {
            let Some(&(_, to, _, _, dur)) = TRANSITIONS.iter().find(|tr| tr.0 == self.current && ((ty == tr.3) == tr.2)) else { break };
            if !self.weighted(to) {
                self.set_start(to, t);
            }
            self.active.push(Active { from: self.current, to, start: t, dur });
            self.current = to;
        }
        self.drop_finished(spec, t);
    }
    fn alpha_of(spec: &Spec, a: &Active, t: f64) -> f64 {
        if a.dur <= 0.0 {
            return 1.0;
        }
        alpha_blend(spec, BLEND_OPTION, ((t - a.start) / a.dur).clamp(0.0, 1.0), "")
    }
    fn drop_finished(&mut self, spec: &Spec, t: f64) {
        for i in (0..self.active.len()).rev() {
            if Self::alpha_of(spec, &self.active[i], t) >= 1.0 {
                self.active.drain(0..=i);
                return;
            }
        }
    }
    /// state -> (clip, time) or None (Identity / not ported)
    fn clip_of(&self, a: &AnimAssets, s: usize, t: f64) -> Option<(String, f64)> {
        let p = match s {
            2 => self.inputs.attack_bounce.clone(),
            3 => a.single_clip(&self.inputs.parry_additive),
            4 => a.single_clip(&self.inputs.parry_push_additive),
            12 => a.single_clip(&self.inputs.alt_parry_push_additive),
            6 => format!("{MISC}Bounce_Additive"),
            7 => format!("{MISC}Bounce_Left_Additive"),
            14 => format!("{MISC}Interact_Additive"),
            5 => format!("{MISC}{}", if self.inputs.disarm_direction < 0.0 { "Disarm_RH_ToLeft_Additive" } else { "Disarm_RH_ToRight_Additive" }),
            10 | 11 => FLINCH_1P.to_string(),
            _ => String::new(),
        };
        let (rate, looping) = player(s)?;
        if p.is_empty() {
            return None;
        }
        let c = a.clip(&p)?;
        let len = c.sequence_length as f64;
        let mut tt = (t - self.start_of(s, t)) * rate;
        tt = if looping && len > 0.0 { tt.rem_euclid(len) } else { tt.clamp(0.0, len) };
        Some((p, tt))
    }
    fn state_pose(&self, a: &AnimAssets, sk: &Skeleton, s: usize, t: f64) -> AddPose {
        // the parry-push states play their blend space at X = BlockDirection (reader method 2026-10-06: the real game's
        // BlockDirection is -90 / 0 / 90 after a parry). The single-clip path below returned nothing for these
        // directional blend spaces (samples on different clips), so the push never played.
        if s == 4 || s == 12 {
            if let Some(p) = self.directional_push(a, sk, s, t) {
                return p;
            }
        }
        match self.clip_of(a, s, t) {
            Some((p, tt)) => additive_sample(a, sk, &p, tt),
            None => vec![None; sk.names.len()],
        }
    }
    /// a parry-push blend space sampled at (BlockDirection, 0): each weighted sample's additive clip at the state's
    /// time (non-looping, clamped to that clip), blended by weight. None when the blend space has a single clip (the
    /// clip_of path handles it) or is missing.
    fn directional_push(&self, a: &AnimAssets, sk: &Skeleton, s: usize, t: f64) -> Option<AddPose> {
        let path = if s == 4 { &self.inputs.parry_push_additive } else { &self.inputs.alt_parry_push_additive };
        if path.is_empty() {
            return None;
        }
        let bs = a.blend_space(path)?;
        if !bs.single_clip().is_empty() {
            return None;
        }
        let (rate, _looping) = player(s)?;
        let tt = ((t - self.start_of(s, t)) * rate).max(0.0);
        let mut pose: Option<AddPose> = None;
        let mut acc = 0.0;
        for (i, w) in bs.weights(self.inputs.block_direction, 0.0) {
            if w <= 0.0 {
                continue;
            }
            let clip = &bs.samples[i].0;
            let len = a.clip(clip).map(|c| c.sequence_length as f64).unwrap_or(0.0);
            let sp = additive_sample(a, sk, clip, tt.min(len));
            acc += w;
            pose = Some(match pose {
                None => sp,
                Some(prev) => blend_add(&prev, &sp, w / acc),
            });
        }
        pose
    }
    /// the machine's output pose at t
    pub fn evaluate(&self, spec: &Spec, a: &AnimAssets, sk: &Skeleton, t: f64) -> AddPose {
        if self.active.is_empty() {
            return self.state_pose(a, sk, self.current, t);
        }
        let mut pose = self.state_pose(a, sk, self.active[0].from, t);
        for x in &self.active {
            pose = blend_add(&pose, &self.state_pose(a, sk, x.to, t), Self::alpha_of(spec, x, t));
        }
        pose
    }
}

/// an additive clip as deltas (AdditiveClip.sample): rotation / translation of every tracked bone; untracked bones
/// and absent channels are the additive identity
pub fn additive_sample(a: &AnimAssets, sk: &Skeleton, path: &str, t: f64) -> AddPose {
    let mut out: AddPose = vec![None; sk.names.len()];
    let Some(c) = a.clip(path) else { return out };
    for tr in &c.tracks {
        let b = tr.bone as usize;
        if b >= out.len() {
            continue;
        }
        let (r, p, _) = c.sample(tr, t as f32);
        let q = r.map(FQuat::from_array).map(qn).unwrap_or(FQuat::IDENTITY);
        let v = p.map(|p| FVector::new(p[0], p[1], p[2])).unwrap_or(FVector::ZERO);
        // This reader samples cooked compressed additive deltas (not editor/raw data).
        // RetargetBoneTransform's baked-additive mode-1 branch preserves additive identity.
        let v = sk.retarget_translation(b, v, true);
        out[b] = Some((q, v));
    }
    out
}

/// AdditivePose.blend: (1 - alpha) a + alpha b, bone by bone
pub fn blend_add(a: &AddPose, b: &AddPose, alpha: f64) -> AddPose {
    if alpha <= 0.0 {
        return a.clone();
    }
    if alpha >= 1.0 {
        return b.clone();
    }
    let al = alpha as f32;
    let wa = 1.0 - al;
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            if x.is_none() && y.is_none() {
                return None;
            }
            let (qa, ta) = x.unwrap_or((FQuat::IDENTITY, FVector::ZERO));
            let (qb, tb) = y.unwrap_or((FQuat::IDENTITY, FVector::ZERO));
            let acc = FQuat::new(qa.x * wa, qa.y * wa, qa.z * wa, qa.w * wa);
            let mut rb = FQuat::new(qb.x * al, qb.y * al, qb.z * al, qb.w * al);
            if rb.x * acc.x + rb.y * acc.y + rb.z * acc.z + rb.w * acc.w < 0.0 {
                rb = FQuat::new(-rb.x, -rb.y, -rb.z, -rb.w);
            }
            let q = qn(FQuat::new(acc.x + rb.x, acc.y + rb.y, acc.z + rb.z, acc.w + rb.w));
            Some((q, FVector::new(ta.x * wa + tb.x * al, ta.y * wa + tb.y * al, ta.z * wa + tb.z * al)))
        })
        .collect()
}

/// AccumulateLocalSpaceAdditivePoseInternal on a local pose at weight w (AdditivePose.apply_ue)
pub fn apply_additive(base: &mut [FTransform], add: &AddPose, w: f64) {
    if w <= 1e-5 {
        return;
    }
    let w = w as f32;
    for (b, x) in base.iter_mut().zip(add.iter()) {
        let Some((q, t)) = x else { continue };
        let r = if w >= 0.99999 {
            let r = FQuat::new(q.x * w, q.y * w, q.z * w, q.w * w);
            if r.w * r.w >= 1.0 { None } else { Some(r) }
        } else {
            let sg = if q.w >= 0.0 { 1.0 } else { -1.0 };
            let r = FQuat::new(q.x * w, q.y * w, q.z * w, sg * (1.0 - w) + q.w * w);
            let l = r.x * r.x + r.y * r.y + r.z * r.z + r.w * r.w;
            Some(if l >= 1e-8 { let k = 1.0 / l.sqrt(); FQuat::new(r.x * k, r.y * k, r.z * k, r.w * k) } else { FQuat::new(0.0, 0.0, 0.0, sg) })
        };
        if let Some(r) = r {
            b.rot = qn(r.mul(b.rot));
        }
        b.loc = b.loc + FVector::new(t.x * w, t.y * w, t.z * w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_weight_drops_after_a_second_of_none() {
        let mut o = AdditiveOverride::default();
        o.update_anim(0.5);
        assert_eq!(o.weight, 1.0);
        o.update_anim(0.6);
        assert_eq!(o.weight, 0.0);
        o.set_type("Parry", 0.3, 0.0);
        o.update_anim(0.01);
        assert_eq!((o.weight, o.anim_type.as_str()), (1.0, "Parry"));
        o.tick(0.31);
        assert_eq!(o.ty, NONE);
    }

    #[test]
    fn apply_full_weight_premultiplies() {
        let q = FQuat::from_rotator(0.0, 30.0, 0.0);
        let mut base = vec![FTransform::new(FQuat::from_rotator(0.0, 10.0, 0.0), FVector::new(1.0, 0.0, 0.0))];
        apply_additive(&mut base, &vec![Some((q, FVector::new(0.0, 2.0, 0.0)))], 1.0);
        let (_, yaw, _) = mordhau_core::combat::geometry::quat_rotator(base[0].rot);
        assert!((yaw - 40.0).abs() < 1e-3 && base[0].loc.y == 2.0);
    }
}

//! The character's animation, engine-neutral and in UE space: the Rust port of godot/game/actor/fighter_anim.gd +
//! game/anim/{anim_montage, montage_set, motion_anim, layered_blend, anim_math, anim_rig} (each rule keeps the
//! reference's citation). It produces the per-bone local pose that BOTH the weapon traces (trace.rs) and the renderer
//! (Sim::snapshot "pose") use, so what is traced is what is shown.
//!
//! Ported: motion -> montage (PlayAttackAnim dynamic montages / AnimMontage assets), blend in / out (FAlphaBlend,
//! every EAlphaBlendOption incl. Custom curves), montage positions (SetAnimPosition from the attack / parry timing),
//! play rates (SetAnimRate: recovery, feint, leave, hit), StopAnim fades, auto blend-out, the slot node weights
//! (DefaultSlot, FullBodySlot), LayeredBoneBlend_1 (LowerBack depth 1, mesh-space rotations), the AutoBlend windup
//! offset search (UAttackMotion::OnBegin_Implementation rva=0x162eda0).
//! r4 adds: SpineSpaceAdditive (spine_additive.gd) + AnimGraphNode_AttackAngling, the upper blend space
//! (blendspace.rs, BlendSpacePlayer_29 at Direction / Helper_UBVelocity from the Sim), the Additive state machine and
//! ApplyAdditive_2 (additive.rs: parry / parry push / bounce additives), and the procedural skeletal controls around
//! AttackAngling that move the weapon (procedural.rs: ModifyBone_56 RightWeapon cosmetic base, the look-up spine bend
//! chain 700..463).
//! Not ported (UNCONFIRMED / left for later): locomotion (LowerBody machine; the reference pose stands in), the
//! turn-in-place LowerBodyRotationOffset pivots, the HitEffect / weapon-slide IK (TwoBoneIK_4, TwoBoneIKOffset_3), the
//! springs / sway, the feet grounding IK and the body-shape controls (see procedural.rs for the node list).

use mh_assets::anim::AnimSequence;
use mh_pak::Reader;
use mordhau_core::combat::motion::{MotionId, MotionKind};
use mordhau_core::combat::World;
use mordhau_core::data::Spec;
use mordhau_core::ue::{FQuat, FTransform, FVector};
use serde_json::Value;
use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;

use crate::pose::Skeleton;

// ---- FAlphaBlend ------------------------------------------------------------------------------------------------

/// FAlphaBlend::AlphaToBlendOption (engine VA 0x142e47870; anim_math.gd alpha_blend): every option clamps to [0, 1];
/// Custom (14) evaluates the curve over its key time range
pub fn alpha_blend(spec: &Spec, option: i32, a: f64, curve: &str) -> f64 {
    let pi = std::f64::consts::PI;
    let v = match option {
        1 => 3.0 * a * a - 2.0 * a * a * a,
        2 => {
            if a < 0.0 {
                0.0
            } else if a >= 1.0 {
                1.0
            } else {
                (3.0 - 2.0 * a) * a * a
            }
        }
        3 => ((a * pi - pi * 0.5).sin() + 1.0) * 0.5,
        4..=7 => {
            let n = (option - 2) as f64;
            let x = a + a;
            if a < 0.5 { x.powf(n) * 0.5 } else { (2.0 - (2.0 - x).powf(n)) * 0.5 }
        }
        8 => 1.0 - (1.0 - a * a).max(0.0).sqrt(),
        9 => (1.0 - (a - 1.0) * (a - 1.0)).max(0.0).sqrt(),
        10 => {
            let x = a + a;
            if a < 0.5 { (1.0 - (1.0 - x * x).max(0.0).sqrt()) * 0.5 } else { ((1.0 - (x - 2.0) * (x - 2.0)).max(0.0).sqrt() + 1.0) * 0.5 }
        }
        11 => {
            if a == 0.0 { 0.0 } else { 2f64.powf((a - 1.0) * 10.0) }
        }
        12 => {
            if a == 1.0 { 1.0 } else { 1.0 - 2f64.powf(a * -10.0) }
        }
        13 => {
            let x = a + a;
            if a < 0.5 {
                (if x == 0.0 { 0.0 } else { 2f64.powf((x - 1.0) * 10.0) }) * 0.5
            } else {
                let x4 = x - 1.0;
                ((if x4 == 1.0 { 1.0 } else { 1.0 - 2f64.powf(x4 * -10.0) }) + 1.0) * 0.5
            }
        }
        14 if !curve.is_empty() => match spec.curves.get(curve) {
            Some(c) if !c.keys.is_empty() => {
                let (t0, t1) = (c.keys[0].time, c.keys[c.keys.len() - 1].time);
                spec.curve_eval(&c.keys, t0 + (t1 - t0) * a, &c.pre, &c.post)
            }
            _ => a,
        },
        _ => a,
    };
    v.clamp(0.0, 1.0)
}

const BLEND_OPTIONS: [&str; 15] = ["Linear", "Cubic", "HermiteCubic", "Sinusoidal", "QuadraticInOut", "CubicInOut", "QuarticInOut",
    "QuinticInOut", "CircularIn", "CircularOut", "CircularInOut", "ExpIn", "ExpOut", "ExpInOut", "Custom"];

fn blend_option(v: &Value) -> i32 {
    let s = v.as_str().unwrap_or("Linear");
    let s = s.rsplit("::").next().unwrap_or(s);
    BLEND_OPTIONS.iter().position(|x| *x == s).unwrap_or(0) as i32
}

// ---- montages (anim_montage.gd / montage_set.gd) -------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Segment {
    pub seq: String,
    pub start_pos: f64,
    pub anim_start: f64,
    pub anim_end: f64,
    pub rate: f64,
    pub loops: i64,
}

/// UAnimMontage data (absent fields: the UAnimMontage ctor values, AnimData.montage) or a dynamic montage
#[derive(Clone, Debug)]
pub struct Montage {
    pub path: String,
    pub slot: String,
    pub length: f64,
    pub segments: Vec<Segment>,
    pub blend_in: f64,
    pub blend_in_option: i32,
    pub blend_in_curve: String,
    pub blend_out: f64,
    pub blend_out_option: i32,
    pub trigger: f64,
    /// MetaData[0] is a UMordhauAnimMetaData with bDisablesSpineBending (+0x2c) (procedural.rs SpineBendBlendWeight)
    pub disables_spine_bending: bool,
}

impl Montage {
    /// FAnimSegment::ConvertTrackPosToAnimPos: (sequence, sequence time) at a montage position
    pub fn sample_at(&self, pos: f64) -> (String, f64) {
        let Some(mut seg) = self.segments.first() else { return (String::new(), 0.0) };
        for s in &self.segments {
            if pos >= s.start_pos {
                seg = s;
            }
        }
        let seg_len = (seg.anim_end - seg.anim_start) / seg.rate.abs().max(1e-8);
        let mut local = (pos - seg.start_pos).clamp(0.0, seg_len * seg.loops.max(1) as f64);
        if seg.loops > 1 && seg_len > 0.0 {
            local %= seg_len;
        }
        (seg.seq.clone(), seg.anim_start + local * seg.rate)
    }
}

/// One FAnimMontageInstance (positions and weights in closed form at absolute times)
#[derive(Clone, Debug)]
pub struct Inst {
    pub asset: Rc<Montage>,
    pub start: f64,
    pub bin: f64,
    pub bin_option: i32,
    pub bin_curve: String,
    pub key_t: f64,
    pub key_pos: f64,
    pub rate: f64,
    pub stopped: bool,
    pub stop_t: f64,
    pub stop_w: f64,
    pub bout: f64,
    pub bout_option: i32,
    pub bout_curve: String,
}

impl Inst {
    pub fn position(&self, t: f64) -> f64 {
        (self.key_pos + (t - self.key_t) * self.rate).clamp(0.0, self.asset.length)
    }
    pub fn set_position(&mut self, t: f64, p: f64) {
        self.key_t = t;
        self.key_pos = p.clamp(0.0, self.asset.length);
    }
    /// SetAnimRate rva=0x14a0460: a rate of 0 becomes 1e-5
    pub fn set_rate(&mut self, t: f64, r: f64) {
        self.key_pos = self.position(t);
        self.key_t = t;
        self.rate = if r != 0.0 { r } else { 1e-5 };
    }
    fn in_weight(&self, spec: &Spec, t: f64) -> f64 {
        if self.bin <= 0.0 {
            return 1.0;
        }
        alpha_blend(spec, self.bin_option, (t - self.start) / self.bin, &self.bin_curve)
    }
    pub fn weight(&self, spec: &Spec, t: f64) -> f64 {
        if t < self.start {
            return 0.0;
        }
        if !self.stopped || t < self.stop_t {
            return self.in_weight(spec, t);
        }
        if self.bout <= 0.0 {
            return 0.0;
        }
        self.stop_w * (1.0 - alpha_blend(spec, self.bout_option, (t - self.stop_t) / self.bout, &self.bout_curve))
    }
    /// FAnimMontageInstance::Stop (0x142e9b4f0)
    pub fn stop(&mut self, spec: &Spec, t: f64, fade: f64, option: i32, curve: &str) {
        if self.stopped {
            if fade < self.bout {
                self.stop_w = self.weight(spec, t);
                self.stop_t = t;
                self.bout = fade;
            }
            return;
        }
        self.stop_w = self.in_weight(spec, t);
        self.stopped = true;
        self.stop_t = t;
        self.bout = fade;
        self.bout_option = option;
        self.bout_curve = curve.to_string();
    }
}

#[derive(Clone, Debug, Default)]
pub struct MontageSet {
    pub insts: Vec<Inst>,
    next_id: u64,
    pub ids: Vec<u64>,
}

impl MontageSet {
    /// UAnimInstance::Montage_Play with bStopAllMontages (0x142e759e0): returns the new instance's id
    pub fn play(&mut self, spec: &Spec, asset: Rc<Montage>, t: f64, rate: f64) -> u64 {
        // Montage_Play stops the montages of the new one's slot group; "DefaultLeftTorso" is taken as its own group
        // (FP-6b, UNCONFIRMED: the skeleton's slot groups are not dumped), so a DefaultSlot / FullBodySlot play leaves a
        // left-torso montage running (the exe keeps CurrentLeftTorsoAnimMontage playing through the 1P -> 3P hand-over)
        let lt = "DefaultLeftTorso";
        for i in self.insts.iter_mut().filter(|i| i.asset.slot != lt || asset.slot == lt) {
            let (bi, bo, bc) = (asset.blend_in, asset.blend_in_option, asset.blend_in_curve.clone());
            i.stop(spec, t, bi, bo, &bc);
        }
        self.next_id += 1;
        self.insts.push(Inst {
            start: t,
            bin: asset.blend_in,
            bin_option: asset.blend_in_option,
            bin_curve: asset.blend_in_curve.clone(),
            key_t: t,
            key_pos: 0.0,
            rate: if rate != 0.0 { rate } else { 1e-5 },
            stopped: false,
            stop_t: 0.0,
            stop_w: 1.0,
            bout: 0.0,
            bout_option: 2,
            bout_curve: String::new(),
            asset,
        });
        self.ids.push(self.next_id);
        self.next_id
    }
    /// Montage_Play of a montage in another slot group (FP-6b: the LeftTorsoMontage in "DefaultLeftTorso"): only
    /// instances in the same slot are stopped (UAnimInstance::Montage_Play stops the montages of the new one's slot
    /// group; UNCONFIRMED: DefaultLeftTorso's group, taken as its own since the exe plays it beside the attack montage)
    pub fn play_extra(&mut self, spec: &Spec, asset: Rc<Montage>, t: f64, rate: f64) -> u64 {
        for i in self.insts.iter_mut().filter(|i| i.asset.slot == asset.slot) {
            let (bi, bo, bc) = (asset.blend_in, asset.blend_in_option, asset.blend_in_curve.clone());
            i.stop(spec, t, bi, bo, &bc);
        }
        self.next_id += 1;
        self.insts.push(Inst {
            start: t,
            bin: asset.blend_in,
            bin_option: asset.blend_in_option,
            bin_curve: asset.blend_in_curve.clone(),
            key_t: t,
            key_pos: 0.0,
            rate: if rate != 0.0 { rate } else { 1e-5 },
            stopped: false,
            stop_t: 0.0,
            stop_w: 1.0,
            bout: 0.0,
            bout_option: 2,
            bout_curve: String::new(),
            asset,
        });
        self.ids.push(self.next_id);
        self.next_id
    }
    pub fn get(&mut self, id: u64) -> Option<&mut Inst> {
        let k = self.ids.iter().position(|x| *x == id)?;
        self.insts.get_mut(k)
    }
    pub fn weight_of(&self, spec: &Spec, id: Option<u64>, t: f64) -> f64 {
        id.and_then(|id| self.ids.iter().position(|x| *x == id)).map(|k| self.insts[k].weight(spec, t)).unwrap_or(0.0)
    }
    /// AAdvancedCharacter::StopAnim rva=0x14a3090 -> Montage_Stop(FadeOut, nullptr): every instance not stopping
    pub fn stop_all(&mut self, spec: &Spec, t: f64, fade: f64) {
        for m in self.insts.iter_mut() {
            if !m.stopped {
                let o = m.asset.blend_out_option;
                m.stop(spec, t, fade, o, "");
            }
        }
    }
    /// auto blend-out (FAnimMontageInstance::Advance block 0x142e81106..0x142e8119f) + removal of finished instances
    pub fn update(&mut self, spec: &Spec, t: f64) {
        for m in self.insts.iter_mut() {
            if m.stopped || m.rate.abs() <= 1e-8 {
                continue;
            }
            let custom = m.asset.trigger >= 0.0;
            let trig = (if custom { m.asset.trigger } else { m.asset.blend_out }).max(1e-4);
            let remaining = ((m.asset.length - m.position(t)) / m.rate).abs();
            if remaining <= trig {
                let (f, o) = (if custom { m.asset.blend_out } else { remaining }, m.asset.blend_out_option);
                m.stop(spec, t, f, o, "");
            }
        }
        let mut k = 0;
        while k < self.insts.len() {
            let m = &self.insts[k];
            if m.stopped && t >= m.stop_t + m.bout {
                self.insts.remove(k);
                self.ids.remove(k);
            } else {
                k += 1;
            }
        }
    }
    /// NativeUpdateAnimation1501930 selects the last genuine montage with nonzero DesiredValue
    /// and nonempty metadata. Existing Play/Stop lifecycle maps desired1/0 to !stopped.
    /// A qualifying null/wrong first entry shadows earlier montages rather than falling back.
    fn last_desired_metadata<T>(&self, mut metadata: impl FnMut(&str) -> Option<Option<T>>) -> Option<(&str, Option<T>)> {
        for instance in self.insts.iter().rev().filter(|i| !i.stopped) {
            if let Some(first) = metadata(&instance.asset.path) {
                return Some((&instance.asset.path, first));
            }
        }
        None
    }

    /// UMordhauAnimInstance::NativeUpdateAnimation rva=0x1501930 (decomp 3056-3101): SpineBendBlendWeight = clamp(1 -
    /// sum of the weights of the instances whose montage bDisablesSpineBending, 0, 1)
    pub fn spine_bend_blend_weight(&self, spec: &Spec, t: f64) -> f64 {
        let s: f64 = self.insts.iter().filter(|m| m.asset.disables_spine_bending).map(|m| m.weight(spec, t)).sum();
        (1.0 - s).clamp(0.0, 1.0)
    }
    /// [(sequence, time, weight)] of the montages on a slot (UpdateMontageEvaluationData 0x142e7cbb0: weight > 1e-5)
    pub fn slot_layers(&self, spec: &Spec, slot: &str, t: f64) -> Vec<(String, f64, f64)> {
        let mut out = Vec::new();
        for m in &self.insts {
            if m.asset.slot != slot {
                continue;
            }
            let w = m.weight(spec, t);
            if w <= 1e-5 {
                continue;
            }
            let (s, st) = m.asset.sample_at(m.position(t));
            out.push((s, st, w));
        }
        out
    }
}

/// FAnimInstanceProxy::GetSlotWeight (0x142e8d060) + SlotEvaluatePose (0x142e9a460): None = the source alone; else
/// (per-montage weights, source weight)
pub fn slot_weights(ws: &[f64]) -> Option<(Vec<f64>, f64)> {
    let tot: f64 = ws.iter().sum();
    let slot_node = if tot > 1.0 { 1.0 } else { tot };
    if ws.is_empty() || slot_node <= 1e-5 {
        return None;
    }
    let non_add = if tot > 1.0 { 1.0 } else { tot };
    Some((ws.iter().map(|w| if tot > 1.00001 { w / tot } else { *w }).collect(), (1.0 - non_add).clamp(0.0, 1.0)))
}

// ---- pose math (anim_math.gd blend_transforms, layered_blend.gd) --------------------------------------------------

fn qdot(a: FQuat, b: FQuat) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w
}
fn qnorm(q: FQuat) -> FQuat {
    let l = qdot(q, q);
    if l < 1e-8 {
        return FQuat::IDENTITY;
    }
    let r = 1.0 / l.sqrt();
    FQuat::new(q.x * r, q.y * r, q.z * r, q.w * r)
}

/// FAnimationRuntime::BlendPosesTogether (0x142e4a250): weighted sum, rotation sign against the running sum,
/// normalized rotations
pub fn blend_transforms(xs: &[FTransform], ws: &[f64]) -> FTransform {
    let mut q = FQuat::new(0.0, 0.0, 0.0, 0.0);
    let mut t = FVector::ZERO;
    for (i, x) in xs.iter().enumerate() {
        let w = ws[i] as f32;
        let mut r = FQuat::new(x.rot.x * w, x.rot.y * w, x.rot.z * w, x.rot.w * w);
        if i > 0 && qdot(r, q) < 0.0 {
            r = FQuat::new(-r.x, -r.y, -r.z, -r.w);
        }
        q = FQuat::new(q.x + r.x, q.y + r.y, q.z + r.z, q.w + r.w);
        t = t + x.loc.scale(w as f64);
    }
    FTransform::new(qnorm(q), t)
}

/// FAnimationRuntime::CreateMaskWeights (LayeredBlend.mask_weights): per filter bone, w += IncreaseWeightPerDepth *
/// (depth + 1) below it, clamped
pub fn mask_weights(sk: &Skeleton, filters: &[(String, i64)]) -> Vec<f32> {
    let mut w = vec![0f32; sk.names.len()];
    for (bone, depth) in filters {
        let Some(mb) = sk.find(bone) else { continue };
        let inc = if *depth != 0 { 1.0 / *depth as f32 } else { 1.0 };
        for (i, wi) in w.iter_mut().enumerate() {
            let mut d = 0;
            let mut b = i as i32;
            let mut found = -1;
            while b >= 0 {
                if b as usize == mb {
                    found = d;
                    break;
                }
                b = sk.parents[b as usize];
                d += 1;
            }
            if found >= 0 {
                *wi = (*wi + inc * (found + 1) as f32).clamp(0.0, 1.0);
            }
        }
    }
    w
}

/// FAnimationRuntime::BlendPosesPerBoneFilter, mesh-space rotation path (0x142e48840; LayeredBlend.blend_mesh_space)
pub fn blend_mesh_space(sk: &Skeleton, base: &[FTransform], blend: &[FTransform], w: &[f32]) -> Vec<FTransform> {
    let ms = |p: &[FTransform]| {
        let mut r: Vec<FQuat> = Vec::with_capacity(p.len());
        for i in 0..p.len() {
            let pr = sk.parents[i];
            r.push(if pr >= 0 { r[pr as usize].mul(p[i].rot) } else { p[i].rot });
        }
        r
    };
    let (rb, rl) = (ms(base), ms(blend));
    let mut out_ms: Vec<FQuat> = Vec::with_capacity(base.len());
    let mut out = Vec::with_capacity(base.len());
    for i in 0..base.len() {
        let a = w[i].min(1.0);
        let (q, loc) = if a <= 1e-5 {
            (rb[i], base[i].loc)
        } else if a >= 0.99999 {
            (rl[i], blend[i].loc)
        } else {
            let (qa, mut qc) = (rb[i], rl[i]);
            if qdot(qa, qc) < 0.0 {
                qc = FQuat::new(-qc.x, -qc.y, -qc.z, -qc.w);
            }
            let q = qnorm(FQuat::new(qa.x * (1.0 - a) + qc.x * a, qa.y * (1.0 - a) + qc.y * a, qa.z * (1.0 - a) + qc.z * a, qa.w * (1.0 - a) + qc.w * a));
            (q, blend_transforms(&[base[i], blend[i]], &[(1.0 - a) as f64, a as f64]).loc)
        };
        out_ms.push(q);
        let p = sk.parents[i];
        let lq = if p >= 0 { qnorm(out_ms[p as usize].inverse().mul(q)) } else { q };
        out.push(FTransform::new(lq, loc));
    }
    out
}

// ---- assets ------------------------------------------------------------------------------------------------------

/// Animation assets read from the paks (AnimData): montages, decoded clips, the AnimBP's node data. Caches are
/// interior (RefCell) so the AutoBlend pose sampler and the montage lookups share one handle.
pub struct AnimAssets {
    pub rd: Reader,
    pub src: mh_assets::pak_source::PakSource,
    montages: RefCell<HashMap<String, Option<Rc<Montage>>>>,
    clips: RefCell<HashMap<String, Option<Rc<AnimSequence>>>>,
    blend_spaces: RefCell<HashMap<String, Option<Rc<crate::blendspace::BlendSpace>>>>,
    notify_cache: RefCell<HashMap<String, Rc<Vec<(String, f64)>>>>,
    fp_attack_defs: RefCell<HashMap<String, Rc<mordhau_core::data::AttackDef>>>,
    /// merged class defaults (Reader::defaults: the Blueprint CDO chain, ~0.5 ms a call) and weapon mesh sockets, cached:
    /// a respawn rebuilds the fighter's anim from them (first-person r3: the test level's round-restart hitch)
    cdos: RefCell<HashMap<String, Rc<serde_json::Map<String, Value>>>>,
    weapon_sockets: RefCell<HashMap<String, Rc<Vec<(String, crate::physics::Socket)>>>>,
    curves: RefCell<HashMap<String, Option<Rc<(Vec<mordhau_core::data::CurveKey>, String, String)>>>>,
    blocked_1p: RefCell<HashMap<String, Rc<Blocked1P>>>,
    pub slot_default: String,
    pub slot_full_body: String,
    pub upper_filters: Vec<(String, i64)>,
    /// AnimGraphNode_AttackAngling's FBoneReference fields: (node field, bone name)
    pub angling: Vec<(String, String)>,
}

pub const ABP: &str = "Mordhau/Content/Mordhau/Animations/Blueprints/AB_MordhauCharacterAnimation";

impl AnimAssets {
    pub fn new(vfs: std::sync::Arc<mh_pak::Vfs>) -> Result<AnimAssets, String> {
        let rd = Reader::new(vfs.clone());
        // AB_MordhauCharacterAnimation CDO (AnimGraphData: AnimGraphNode_Slot / _Slot_1 / LayeredBoneBlend_1)
        let ex = rd.read(ABP).ok_or("pak: no AB_MordhauCharacterAnimation")?;
        let cdo = ex
            .iter()
            .find(|e| e["Name"].as_str().map(|n| n.starts_with("Default__")).unwrap_or(false))
            .ok_or("AB_MordhauCharacterAnimation: no CDO")?;
        let p = &cdo["Properties"];
        let slot = |n: &str| p[n]["SlotName"].as_str().unwrap_or("").to_string();
        let mut filters = Vec::new();
        for f in p["AnimGraphNode_LayeredBoneBlend_1"]["LayerSetup"][0]["BranchFilters"].as_array().into_iter().flatten() {
            filters.push((f["BoneName"].as_str().unwrap_or("").to_string(), f["BlendDepth"].as_i64().unwrap_or(0)));
        }
        let (slot_default, slot_full_body) = (slot("AnimGraphNode_Slot"), slot("AnimGraphNode_Slot_1"));
        let angling = ANGLING_ORDER
            .iter()
            .filter_map(|(f, _, _)| p["AnimGraphNode_AttackAngling"][*f]["BoneName"].as_str().map(|b| (f.to_string(), b.to_string())))
            .collect();
        Ok(AnimAssets {
            src: mh_assets::pak_source::PakSource::new(vfs),
            rd,
            montages: RefCell::new(HashMap::new()),
            clips: RefCell::new(HashMap::new()),
            blend_spaces: RefCell::new(HashMap::new()),
            notify_cache: RefCell::new(HashMap::new()),
            fp_attack_defs: RefCell::new(HashMap::new()),
            cdos: RefCell::new(HashMap::new()),
            weapon_sockets: RefCell::new(HashMap::new()),
            curves: RefCell::new(HashMap::new()),
            blocked_1p: RefCell::new(HashMap::new()),
            slot_default,
            slot_full_body,
            upper_filters: filters,
            angling,
        })
    }

    /// AMordhauEquipment RightWeaponBoneCosmeticTransform (+0xb60, third person) from the class defaults chain:
    /// (FRotator of its rotation, translation); identity when absent (AMordhauEquipment ctor)
    /// the right-hand equipment's offhand-IK data (procedural::OffhandWeapon): AMordhauEquipment bUsesOffhandIK /
    /// bInvertOffhandUp / OffhandIKUpOffset(1P) over the ctor values (OffhandIKUpOffset -10, AMordhauEquipment.cpp 1672),
    /// the Second* fields in the alternate mode (SwitchMode_Implementation swaps them, decomp 3206-3247), and the merged
    /// mesh's "GripEnd" / "OffhandIK" / "OffhandIK1P" sockets (OnPartsChanged rva=0x1556340; the sockets of the weapon
    /// skeleton relative to its root bone, whose ref pose is taken as identity: UNCONFIRMED) (fidelity-audit r4)
    /// Reader::defaults(cls), cached
    pub fn cdo(&self, cls: &str) -> Rc<serde_json::Map<String, Value>> {
        if let Some(d) = self.cdos.borrow().get(cls) {
            return d.clone();
        }
        let d = Rc::new(self.rd.defaults(cls));
        self.cdos.borrow_mut().insert(cls.to_string(), d.clone());
        d
    }

    /// the weapon mesh's sockets (physics::weapon_mesh + sockets), cached
    fn weapon_sockets(&self, weapon: &str) -> Rc<Vec<(String, crate::physics::Socket)>> {
        if let Some(s) = self.weapon_sockets.borrow().get(weapon) {
            return s.clone();
        }
        let s = Rc::new(crate::physics::weapon_mesh(&self.rd, weapon).and_then(|m| crate::physics::sockets(&self.rd, &m).ok()).unwrap_or_default());
        self.weapon_sockets.borrow_mut().insert(weapon.to_string(), s.clone());
        s
    }

    pub fn offhand_weapon(&self, weapon: &str, alternate: bool, grip_rel: FTransform) -> crate::procedural::OffhandWeapon {
        let d = self.cdo(weapon);
        let b = |k: &str| d.get(k).and_then(|v| v.as_bool());
        let f = |k: &str| d.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
        let pre = if alternate { "Second" } else { "" };
        let uses = if alternate { b("bSecondUsesOffhandIK") } else { b("bUsesOffhandIK") }.unwrap_or(false);
        let invert = if alternate { b("bSecondInvertOffhandUp") } else { b("bInvertOffhandUp") }.unwrap_or(false);
        let up = f(&format!("{pre}OffhandIKUpOffset")).unwrap_or(if alternate { 0.0 } else { -10.0 });
        let up1p = f(&format!("{pre}OffhandIKUpOffset1P")).unwrap_or(0.0);
        let socks = self.weapon_sockets(weapon);
        let get = |n: &str| socks.iter().find(|(k, _)| k == n).map(|(_, s)| s.xf.loc);
        crate::procedural::OffhandWeapon {
            uses_offhand_ik: uses,
            invert_up: invert,
            up_offset: up,
            up_offset_1p: up1p,
            grip_end_z: get("GripEnd").map(|v| v.z),
            fixed: get("OffhandIK"),
            fixed_1p: get("OffhandIK1P"),
            grip_rel,
        }
    }

    /// Native montage overlay: None means no qualifying metadata array; Some(None) means
    /// an eligible montage whose FIRST entry contributes no supported Mordhau metadata.
    /// Dynamic Wrapped1499390 montages have empty metadata, even when their sequence has metadata.
    fn montage_first_offhand_meta(&self, path: &str) -> Option<Option<(bool, bool, Option<f32>, f32, f32)>> {
        // Reader strips signed/overflow suffixes too; validate exact export identity first.
        let index = if mh_pak::reader::strip_index(path) != path { Some(indexed_export_path(path)?.1) } else { None };
        let exports = self.rd.read(path)?;
        let asset = if let Some(index) = index {
            let asset = exports.get(index)?;
            if asset["Type"].as_str() != Some("AnimMontage") { return None; }
            asset
        } else {
            let mut montages = exports.iter().filter(|e| e["Type"].as_str() == Some("AnimMontage"));
            let asset = montages.next()?;
            if montages.next().is_some() { return None; }
            asset
        };
        first_montage_offhand_metadata(asset, |package, index| {
            let pkg = self.rd.open(package)?;
            if index >= pkg.exports.len() { return None; }
            Some(self.rd.export_json(&pkg, index))
        })
    }

    /// Referenced UMordhauAnimMetaData fields: (disables, forces, optional speed override, maximum, minimum).
    /// UAttackMotion::PlayAttackAnim rva=0x1636d90, decomp 7195-7232 visits the sequence's MetaData array in order:
    /// the last matching flags/distances and last requested speed override win. None means no matching metadata
    /// or unavailable/unsupported reference data, preserving this caller's Option contract. This only decodes;
    /// persistent motion writes and NativeUpdateAnimation overlays outside authored switch montages remain unported
    /// (state/proofs/attack_offhand_speed-20261008.md). Bare package paths select their unique animation export;
    /// indexed paths and metadata references always select the exact export index.
    pub fn offhand_meta(&self, seq: &str) -> Option<(bool, bool, Option<f32>, f32, f32)> {
        if seq.is_empty() {
            return None;
        }
        // Reader::open strips numeric suffixes (including signed/oversized ones). Reject unsupported indices
        // before it does so, instead of accidentally interpreting such a path as an unindexed asset package.
        let index = if mh_pak::reader::strip_index(seq) != seq { Some(indexed_export_path(seq)?.1) } else { None };
        let ex = self.rd.read(seq)?;
        let is_animation = |e: &&Value| matches!(e["Type"].as_str(), Some("AnimSequence" | "AnimMontage"));
        let asset = if let Some(index) = index {
            let e = ex.get(index)?;
            if !is_animation(&e) {
                return None;
            }
            e
        } else {
            let mut animations = ex.iter().filter(is_animation);
            let e = animations.next()?;
            if animations.next().is_some() {
                return None;
            }
            e
        };
        offhand_metadata_refs(asset, |package, index| {
            let pkg = self.rd.open(package)?;
            if index >= pkg.exports.len() {
                return None;
            }
            Some(self.rd.export_json(&pkg, index))
        })
    }

    /// AMordhauEquipment RightShoulderOffset1P (+0xac0) / LeftShoulderOffset1P (+0xacc) of the right-hand equipment
    /// (UMordhauAnimInstance::NativeUpdateAnimation decomp 3900-3926; the ctor leaves them zero, only throwables /
    /// tools set them in their Blueprints) (fidelity-audit r4)
    pub fn shoulder_offsets_1p(&self, weapon: &str) -> (FVector, FVector) {
        let d = self.cdo(weapon);
        let v = |k: &str| -> FVector {
            let o = d.get(k).cloned().unwrap_or(Value::Null);
            let g = |c: &str| o[c].as_f64().unwrap_or(0.0) as f32;
            FVector::new(g("X"), g("Y"), g("Z"))
        };
        (v("RightShoulderOffset1P"), v("LeftShoulderOffset1P"))
    }

    pub fn weapon_cosmetic(&self, weapon: &str) -> ((f32, f32, f32), FVector) {
        let d = self.cdo(weapon);
        let Some(v) = d.get("RightWeaponBoneCosmeticTransform") else { return ((0.0, 0.0, 0.0), FVector::ZERO) };
        let g = |o: &Value, k: &str, z: f64| o[k].as_f64().unwrap_or(z) as f32;
        let r = &v["Rotation"];
        let q = if r.is_null() { FQuat::IDENTITY } else { FQuat::new(g(r, "X", 0.0), g(r, "Y", 0.0), g(r, "Z", 0.0), g(r, "W", 1.0)) };
        let t = &v["Translation"];
        (mordhau_core::combat::geometry::quat_rotator(q), FVector::new(g(t, "X", 0.0), g(t, "Y", 0.0), g(t, "Z", 0.0)))
    }

    /// a BlendSpace / AimOffsetBlendSpace asset (blendspace.rs), cached
    pub fn blend_space(&self, path: &str) -> Option<Rc<crate::blendspace::BlendSpace>> {
        if path.is_empty() {
            return None;
        }
        if let Some(b) = self.blend_spaces.borrow().get(path) {
            return b.clone();
        }
        let b = self.rd.read(path).and_then(|ex| {
            let e = ex.iter().find(|e| e["Type"].as_str().map(|t| t.contains("BlendSpace")).unwrap_or(false))?;
            Some(Rc::new(crate::blendspace::BlendSpace::from_props(path, &e["Properties"])))
        });
        self.blend_spaces.borrow_mut().insert(path.to_string(), b.clone());
        b
    }

    /// the clip of a blend space whose samples all use one sequence ("" otherwise; additive_machine.gd _single_clip)
    pub fn single_clip(&self, bs: &str) -> String {
        self.blend_space(bs).map(|b| b.single_clip()).unwrap_or_default()
    }

    /// FPerspectiveAnimMontage::Get / FPerspectiveAnimSequenceBase::Get / FPerspectiveCurveFloat::Get rva=0x165c400
    /// (one ICF body, decomp FPerspectiveAnimMontage.cpp 4-17): bIsFirstPerson ? (FirstPerson ?: ThirdPerson) :
    /// (ThirdPerson ?: FirstPerson), on a class's defaults (fidelity-audit r3)
    pub fn persp(&self, cls: &str, key: &str, first_person: bool) -> String {
        if cls.is_empty() {
            return String::new();
        }
        let d = self.cdo(cls);
        let Some(v) = d.get(key) else { return String::new() };
        let (t, f) = (obj_path(&v["ThirdPerson"]), obj_path(&v["FirstPerson"]));
        if first_person {
            if !f.is_empty() { f } else { t }
        } else if !t.is_empty() {
            t
        } else {
            f
        }
    }

    /// An attack motion's class defaults as a first-person character reads them: every FPerspective* field of
    /// UAttackMotion (types/UAttackMotion.h) through FPerspectiveFloat::Get / FPerspectiveBool::Get (decomp
    /// FPerspectiveFloat.cpp 6-13, FPerspectiveBool.cpp 6-13: FirstPerson, no fallback) and the object Gets rva=0x165c400
    /// (FirstPerson, else ThirdPerson). A FirstPerson value the Blueprint chain does not set is the native ctor's:
    /// UAttackMotion ctor rva=0x1612540 (decomp UAttackMotion.cpp 3500-3562), UKickMotion ctor NormalBlendIn.FirstPerson
    /// 0.15 (src/Mordhau/Private/Motions/KickMotion.cpp, byte-matched); UStrikeMotion's ctor sets only
    /// EnableWindUpSmoothing.ThirdPerson (decomp UStrikeMotion.cpp 1028), so the 1P smoothing is off.
    pub fn attack_def_1p(&self, d: &mordhau_core::data::AttackDef, bp: &str, native: &str) -> Rc<mordhau_core::data::AttackDef> {
        let key = format!("{bp}|{native}");
        if let Some(x) = self.fp_attack_defs.borrow().get(&key) {
            return x.clone();
        }
        let cdo = if bp.is_empty() { Rc::new(serde_json::Map::new()) } else { self.cdo(bp) };
        let fp = |k: &str| cdo.get(k).and_then(|v| v.get("FirstPerson")).cloned();
        let f = |k: &str, ctor: f64| fp(k).and_then(|v| v.as_f64()).map(|x| x as f32 as f64).unwrap_or(ctor);
        let b = |k: &str, ctor: bool| fp(k).and_then(|v| v.as_bool()).unwrap_or(ctor);
        let o = |k: &str, tp: &String| {
            let x = fp(k).map(|v| obj_path(&v)).unwrap_or_default();
            if x.is_empty() { tp.clone() } else { x }
        };
        let mut x = d.clone();
        let kick = native == "UKickMotion";
        x.normal_blend_in = f("NormalBlendIn", 0.15);
        let _ = kick; // UKickMotion's FirstPerson NormalBlendIn equals UAttackMotion's (0.15)
        x.normal_slow_blend_in = f("NormalSlowBlendIn", 0.31);
        x.normal_parry_slow_blend_in = f("NormalParrySlowBlendIn", 0.35);
        x.combo_blend_in = f("ComboBlendIn", 1.0);
        x.post_clash_blend_in = f("PostClashBlendIn", 0.5);
        x.morph_blend_in = f("MorphBlendIn", 0.35);
        x.riposte_blend_in = f("RiposteBlendIn", 0.125);
        x.blend_out = f("BlendOut", 0.8);
        x.feint_anim_rate = f("FeintAnimRate", 1.0);
        x.feint_anim_minimum_duration = f("FeintAnimMinimumDuration", 0.2);
        x.feint_anim_duration_offset = f("FeintAnimDurationOffset", 0.0);
        x.successful_hit_blend_out_anim_time = f("SuccessfulHitBlendOutAnimTime", 0.6);
        x.successful_hit_play_rate = f("SuccessfulHitPlayRate", 1.1);
        x.miss_recovery_to_play_rate = f("MissRecoveryToPlayRate", 0.63);
        // MissRecoveryPlayRateClamp.FirstPerson (ctor (0.8, 1.4), decomp UAttackMotion.cpp 3557-3560; fp_anim_items #7)
        x.miss_recovery_play_rate_clamp = match fp("MissRecoveryPlayRateClamp") {
            Some(v) => mordhau_core::ue::Vec2::new(v["X"].as_f64().unwrap_or(0.8) as f32, v["Y"].as_f64().unwrap_or(1.4) as f32),
            None => mordhau_core::ue::Vec2::new(0.8, 1.4),
        };
        x.regular_attacks_use_auto_blend_in = b("RegularAttacksUseAutoBlendIn", false);
        x.riposte_attacks_use_auto_blend_in = b("RiposteAttacksUseAutoBlendIn", false);
        x.combo_attacks_use_auto_blend_in = b("ComboAttacksUseAutoBlendIn", false);
        x.post_clash_attacks_use_auto_blend_in = b("PostClashAttacksUseAutoBlendIn", false);
        x.morph_attacks_use_auto_blend_in = b("MorphAttacksUseAutoBlendIn", false);
        x.enable_windup_smoothing = b("EnableWindUpSmoothing", false);
        x.combo_blend_in_curve = o("ComboBlendInCurve", &d.combo_blend_in_curve);
        x.combo_windup_curve = o("ComboWindUpCurve", &d.combo_windup_curve);
        x.auto_blend_in_weapon_curve = o("AutoBlendInWeaponCurve", &d.auto_blend_in_weapon_curve);
        x.auto_blend_in_spine_curve = o("AutoBlendInSpineCurve", &d.auto_blend_in_spine_curve);
        // FPerspectiveHighMidLowSpineSpaceAdditive AnglingAdditiveWindUp / RiposteAnglingAdditiveWindUp: the FirstPerson half
        let hml = |k: &str, tp: &mordhau_core::data::Hml| -> mordhau_core::data::Hml {
            let Some(v) = fp(k) else { return tp.clone() };
            let lvl = |n: &str| -> mordhau_core::data::SpineAdd {
                v.get(n)
                    .and_then(|m| m.as_object())
                    .map(|m| {
                        m.iter()
                            .map(|(bone, r)| (bone.clone(), [r["Pitch"].as_f64().unwrap_or(0.0), r["Yaw"].as_f64().unwrap_or(0.0), r["Roll"].as_f64().unwrap_or(0.0)]))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            mordhau_core::data::Hml { high: lvl("High"), mid: lvl("Mid"), low: lvl("Low") }
        };
        x.angling_windup = hml("AnglingAdditiveWindUp", &d.angling_windup);
        x.riposte_angling_windup = hml("RiposteAnglingAdditiveWindUp", &d.riposte_angling_windup);
        let x = Rc::new(x);
        self.fp_attack_defs.borrow_mut().insert(key, x.clone());
        x
    }

    /// UeRec.tp_or_fp_obj on a class's defaults: Key.ThirdPerson, else Key.FirstPerson (FPerspective* structs)
    pub fn tp_or_fp(&self, cls: &str, key: &str) -> String {
        if cls.is_empty() {
            return String::new();
        }
        let d = self.cdo(cls);
        let Some(v) = d.get(key) else { return String::new() };
        let t = obj_path(&v["ThirdPerson"]);
        if !t.is_empty() { t } else { obj_path(&v["FirstPerson"]) }
    }

    /// an AnimSequence's float curve `name` at time t (mh-assets decoded the CompressedRichCurve codec; FRichCurve::Eval
    /// rva=0x3024860 through data::Spec::curve_eval); None when the clip has no such curve
    pub fn curve_value(&self, spec: &Spec, seq: &str, name: &str, t: f64) -> Option<f64> {
        let c = self.clip(seq)?;
        let fc = c.curves.iter().find(|x| x.name == name)?;
        if fc.keys.is_empty() {
            return Some(fc.default_value as f64);
        }
        let keys: Vec<mordhau_core::data::CurveKey> = fc
            .keys
            .iter()
            .map(|k| mordhau_core::data::CurveKey {
                interp_mode: Some(match k.interp { 1 => "RCIM_Constant", 2 => "RCIM_Cubic", _ => "RCIM_Linear" }.to_string()),
                tangent_weight_mode: None,
                time: k.time as f64,
                value: k.value as f64,
                arrive_tangent: k.arrive_tangent as f64,
                leave_tangent: k.leave_tangent as f64,
                ..Default::default()
            })
            .collect();
        let ext = |e: u8| match e { 0 => "RCCE_Cycle", 1 => "RCCE_CycleWithOffset", 2 => "RCCE_Oscillate", 3 => "RCCE_Linear", 5 => "RCCE_None", _ => "RCCE_Constant" };
        Some(spec.curve_eval(&keys, t, ext(fc.pre), ext(fc.post)))
    }

    /// an AnimSequence's notifies (FAnimNotifyEvent NotifyName, time = LinkValue with EAnimLinkMethod::Absolute), cached
    pub fn notifies(&self, seq: &str) -> Rc<Vec<(String, f64)>> {
        if let Some(n) = self.notify_cache.borrow().get(seq) {
            return n.clone();
        }
        let v: Vec<(String, f64)> = self
            .rd
            .read(seq)
            .and_then(|ex| ex.iter().find(|e| e["Type"].as_str() == Some("AnimSequence")).cloned())
            .map(|e| {
                e.get("Notifies").or_else(||e["Properties"].get("Notifies")).unwrap_or(&Value::Null)
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|n| (n["NotifyName"].as_str().unwrap_or("").to_string(), n["LinkValue"].as_f64().unwrap_or(0.0) + n["TriggerTimeOffset"].as_f64().unwrap_or(0.0)))
                    .collect()
            })
            .unwrap_or_default();
        let v = Rc::new(v);
        self.notify_cache.borrow_mut().insert(seq.to_string(), v.clone());
        v
    }

    /// a UCurveFloat's FloatCurve (keys, pre / post extrapolation) from the paks (SpecBuilder::curve's reading)
    pub fn curve(&self, path: &str) -> Option<(Vec<mordhau_core::data::CurveKey>, String, String)> {
        if path.is_empty() {
            return None;
        }
        let ex = self.rd.read(path)?;
        let e = mh_pak::pkg::export_of(&ex, "CurveFloat")?;
        let fc = &e["Properties"]["FloatCurve"];
        let keys: Vec<mordhau_core::data::CurveKey> = serde_json::from_value(fc["Keys"].clone()).ok()?;
        let tail = |v: &Value| v.as_str().map(|s| s.rsplit("::").next().unwrap_or(s).to_string()).unwrap_or_else(|| "RCCE_Constant".into());
        Some((keys, tail(&fc["PreInfinityExtrap"]), tail(&fc["PostInfinityExtrap"])))
    }

    /// a UCurveFloat evaluated (FRichCurve::Eval through Spec::curve_eval), the curve read once (AnimAssets::curve);
    /// None for an empty path or a package without a CurveFloat
    pub fn curve_at(&self, spec: &Spec, path: &str, t: f64) -> Option<f64> {
        if path.is_empty() {
            return None;
        }
        // Explicit authoring overrides also reach the selected FirstPerson combo asset.
        // With no override, preserve the original perspective selection and cached pak curve.
        if let Some(c) = spec.curve_overrides.get(path) {
            return Some(spec.curve_eval(&c.keys, t, &c.pre, &c.post));
        }
        let c = {
            let mut m = self.curves.borrow_mut();
            m.entry(path.to_string()).or_insert_with(|| self.curve(path).map(Rc::new)).clone()
        }?;
        Some(spec.curve_eval(&c.0, t, &c.1, &c.2))
    }

    /// ParryRates of a UParryMotion Blueprint for one perspective. Third person: the spec's ParryDef (ParryFailPlayRate,
    /// ParryFailFadeOut, HeldParryFail*, ParryMissFadeOut) and the CDO's ShieldWallPlayRate / ShieldWallFadeOut. First
    /// person: the *1P fields from the CDO chain over the native ctor rva=0x16486b0 (decomp UParryMotion.cpp 2037-2045:
    /// ParryFailPlayRate1P / HeldParryFailPlayRate1P 0.6, ParryFailFadeOut1P / HeldParryFailFadeOut1P 0.6; fields the ctor
    /// does not set stay 0, UObject zero-init)
    pub fn parry_rates(&self, bp: &str, pd: &mordhau_core::data::ParryDef, first_person: bool) -> ParryRates {
        let d = if bp.is_empty() { Rc::new(serde_json::Map::new()) } else { self.cdo(bp) };
        let f = |k: &str, ctor: f64| d.get(k).and_then(|v| v.as_f64()).map(|x| x as f32 as f64).unwrap_or(ctor);
        if first_person {
            ParryRates {
                fail_rate: f("ParryFailPlayRate1P", 0.6),
                fail_fade: f("ParryFailFadeOut1P", 0.6),
                held_fail_rate: f("HeldParryFailPlayRate1P", 0.6),
                held_fail_fade: f("HeldParryFailFadeOut1P", 0.6),
                miss_fade: f("ParryMissFadeOut1P", 0.0),
                wall_rate: f("ShieldWallPlayRate1P", 0.0),
                wall_fade: f("ShieldWallFadeOut1P", 0.0),
            }
        } else {
            ParryRates {
                fail_rate: pd.parry_fail_play_rate,
                fail_fade: pd.parry_fail_fade_out,
                held_fail_rate: pd.held_parry_fail_play_rate,
                held_fail_fade: pd.held_parry_fail_fade_out,
                miss_fade: pd.parry_miss_fade_out,
                wall_rate: f("ShieldWallPlayRate", 0.0),
                wall_fade: f("ShieldWallFadeOut", 0.0),
            }
        }
    }

    /// Blocked1P of a UBlockedMotion Blueprint (cached): ctor rva=0x1644780 values, then the class defaults
    pub fn blocked_1p(&self, bp: &str) -> Rc<Blocked1P> {
        if let Some(x) = self.blocked_1p.borrow().get(bp) {
            return x.clone();
        }
        let d = if bp.is_empty() { Rc::new(serde_json::Map::new()) } else { self.cdo(bp) };
        let f = |k: &str, ctor: f64| d.get(k).and_then(|v| v.as_f64()).map(|x| x as f32 as f64).unwrap_or(ctor);
        let v2 = |k: &str, ctor: (f64, f64)| match d.get(k) {
            Some(v) => (v["X"].as_f64().map(|x| x as f32 as f64).unwrap_or(ctor.0), v["Y"].as_f64().map(|x| x as f32 as f64).unwrap_or(ctor.1)),
            None => ctor,
        };
        let o = |k: &str| d.get(k).map(obj_path).unwrap_or_default();
        // the ctor's values for the non-3P fields (UnitVector * k, ZeroVector, scalars)
        let x = Blocked1P {
            clash_fade: f("ClashFadeOutTime", 0.7),
            stab_world_fade: f("StabWorldFadeOutTime", 1.0),
            stab_parry_range: v2("StabParryMinMaxRange", (0.4, 0.4)),
            stab_parry_fade: v2("StabParryFadeOutTime", (0.7, 0.7)),
            stab_chambered_range: v2("StabChamberedMinMaxRange", (0.4, 0.4)),
            stab_chambered_fade: v2("StabChamberedFadeOutTime", (0.7, 0.7)),
            stab_hit_stop_fade: f("StabHitStopFadeOutTime", 0.5),
            hit_stop_curve: o("ProceduralHitStopBounceCurve"),
            hit_stop_scale_curve: o("ProceduralHitStopBounceScaleCurve"),
            hit_stop_release_scale_curve: o("ProceduralHitStopReleaseScaleCurve"),
            hit_stop_until_fade: f("ProceduralHitStopTimeUntilFade", 0.3),
            hit_stop_duration: f("ProceduralHitStopBounceDuration", 0.5),
            hit_stop_fade: f("ProceduralHitStopFadeOutTime", 0.8),
            release_scale_curve: o("ReleaseScaleCurve"),
            parry_range: v2("ProceduralParryMinMaxRange", (0.4, 0.4)),
            parry_until_fade: v2("ProceduralParryTimeUntilFade", (0.0, 0.0)),
            parry_duration: v2("ProceduralParryBounceDuration", (0.35, 0.35)),
            parry_fade: v2("ProceduralParryFadeOutTime", (0.8, 0.8)),
            chamber_range: v2("ProceduralChamberMinMaxRange", (0.4, 0.4)),
            chamber_until_fade: v2("ProceduralChamberTimeUntilFade", (0.0, 0.0)),
            chamber_duration: v2("ProceduralChamberBounceDuration", (0.35, 0.35)),
            chamber_fade: v2("ProceduralChamberFadeOutTime", (0.8, 0.8)),
            world_until_fade: f("ProceduralWorldTimeUntilFade", 0.3),
            world_duration: f("ProceduralWorldBounceDuration", 0.5),
            world_fade: f("ProceduralWorldFadeOutTime", 0.8),
        };
        let x = Rc::new(x);
        self.blocked_1p.borrow_mut().insert(bp.to_string(), x.clone());
        x
    }

    /// UStabMotion AnimAngleCurve / AnimAngleCueAmount from the motion's class defaults (UStabMotion::OnTickWindUp)
    pub fn stab_cue(&self, motion_bp: &str) -> Option<StabCue> {
        if motion_bp.is_empty() {
            return None;
        }
        let d = self.cdo(motion_bp);
        let c = obj_path(d.get("AnimAngleCurve")?);
        let (keys, pre, post) = self.curve(&c)?;
        let r = d.get("AnimAngleCueAmount").cloned().unwrap_or(Value::Null);
        let g = |k: &str| r[k].as_f64().unwrap_or(0.0) as f32;
        Some(StabCue { keys, pre, post, amount: (g("Pitch"), g("Yaw"), g("Roll")) })
    }

    /// AMordhauEquipment UpperBlendSpace (+0x820; equipment_def.gd upper_blend_space)
    pub fn upper_blend_space(&self, weapon: &str) -> String {
        self.cdo(weapon).get("UpperBlendSpace").map(obj_path).unwrap_or_default()
    }

    /// an object property of an equipment class's defaults (Blueprint chain), "" when unset
    pub fn equipment_obj(&self, cls: &str, key: &str) -> String {
        if cls.is_empty() {
            return String::new();
        }
        self.cdo(cls).get(key).map(obj_path).unwrap_or_default()
    }

    /// UMordhauAnimInstance::UpdateEquipmentData rva=0x151c4d0 (decomp UMordhauAnimInstance.cpp 6610-6665):
    /// UpperBlendSpaceA = bIsFirstPerson ? UpperBlendSpace1P (+0x800) : UpperBlendSpace (+0x820), no fallback
    /// (fidelity-audit r3; shield / horse / alternate-mode variants not modelled here)
    pub fn upper_blend_space_for(&self, weapon: &str, first_person: bool) -> String {
        if !first_person {
            return self.upper_blend_space(weapon);
        }
        self.cdo(weapon).get("UpperBlendSpace1P").map(obj_path).unwrap_or_default()
    }

    /// fp-anim r1: a clip's play length SequenceLength / RateScale (UAnimSequenceBase::GetPlayLength with the sequence's
    /// RateScale, as UBlendSpaceBase::GetAnimationLengthFromSampleData divides it; e.g. 2H_Sword_Run_1P RateScale 2.0)
    pub fn play_length(&self, seq: &str) -> f64 {
        self.clip(seq).map(|c| {
            let r = if c.rate_scale.abs() > 1e-6 { c.rate_scale.abs() as f64 } else { 1.0 };
            c.sequence_length as f64 / r
        }).unwrap_or(0.0)
    }

    /// fp-anim r1: UBlendSpaceBase::GetAnimationLengthFromSampleData (engine source, UNCONFIRMED as compiled): the
    /// weighted sum of the samples' SequenceLength / (RateScale x sample RateScale; every sample RateScale here is 1)
    pub fn blend_space_length(&self, bs: &crate::blendspace::BlendSpace, weights: &[(usize, f64)]) -> f64 {
        weights.iter().filter_map(|(k, w)| bs.samples.get(*k).map(|s| self.play_length(&s.0) * w)).sum()
    }

    /// fp-anim r1: a blend space's pose from given sample weights (BsPlayer) with every clip at `norm` x its length
    /// (sync-group follower / leader: FAnimTickRecord normalized time; looping)
    pub fn blend_space_pose_at(&self, sk: &Skeleton, bs: &crate::blendspace::BlendSpace, weights: &[(usize, f64)], norm: f64) -> Vec<FTransform> {
        let mut by_seq: Vec<(String, f64)> = Vec::new();
        for (k, w) in weights {
            let Some(s) = bs.samples.get(*k) else { continue };
            if self.clip(&s.0).is_none() {
                continue;
            }
            match by_seq.iter_mut().find(|e| e.0 == s.0) {
                Some(e) => e.1 += w,
                None => by_seq.push((s.0.clone(), *w)),
            }
        }
        if by_seq.is_empty() {
            return sk.ref_local.clone();
        }
        let poses: Vec<Vec<FTransform>> = by_seq
            .iter()
            .map(|(s, _)| {
                let len = self.clip(s).map(|c| c.sequence_length as f64).unwrap_or(0.0);
                self.sample(sk, s, norm.rem_euclid(1.0) * len, true)
            })
            .collect();
        if poses.len() == 1 {
            return poses[0].clone();
        }
        let tot: f64 = by_seq.iter().map(|e| e.1).sum();
        let ws: Vec<f64> = by_seq.iter().map(|e| e.1 / tot).collect();
        (0..sk.ref_local.len()).map(|i| blend_transforms(&poses.iter().map(|p| p[i]).collect::<Vec<_>>(), &ws)).collect()
    }

    /// a blend space's pose at (x, y): each weighted sample's clip at its own looping time t (rig.blend_space_pose)
    pub fn blend_space_pose(&self, sk: &Skeleton, bs: &crate::blendspace::BlendSpace, x: f64, y: f64, t: f64) -> Vec<FTransform> {
        let mut by_seq: Vec<(String, f64)> = Vec::new();
        for (k, w) in bs.weights(x, y) {
            let Some(s) = bs.samples.get(k) else { continue };
            if self.clip(&s.0).is_none() {
                continue;
            }
            match by_seq.iter_mut().find(|e| e.0 == s.0) {
                Some(e) => e.1 += w,
                None => by_seq.push((s.0.clone(), w)),
            }
        }
        if by_seq.is_empty() {
            return sk.ref_local.clone();
        }
        let poses: Vec<Vec<FTransform>> = by_seq.iter().map(|(s, _)| self.sample(sk, s, t, true)).collect();
        if poses.len() == 1 {
            return poses[0].clone();
        }
        let tot: f64 = by_seq.iter().map(|e| e.1).sum();
        let ws: Vec<f64> = by_seq.iter().map(|e| e.1 / tot).collect();
        (0..sk.ref_local.len()).map(|i| blend_transforms(&poses.iter().map(|p| p[i]).collect::<Vec<_>>(), &ws)).collect()
    }

    pub fn clip(&self, path: &str) -> Option<Rc<AnimSequence>> {
        if path.is_empty() {
            return None;
        }
        if let Some(c) = self.clips.borrow().get(path) {
            return c.clone();
        }
        let c = mh_assets::anim::decode(&self.src, path).ok().map(Rc::new);
        self.clips.borrow_mut().insert(path.to_string(), c.clone());
        c
    }

    /// AnimData.montage (absent BlendIn / BlendOut: 0.25 Linear; BlendOutTriggerTime -1)
    pub fn montage(&self, path: &str) -> Option<Rc<Montage>> {
        if path.is_empty() {
            return None;
        }
        if let Some(m) = self.montages.borrow().get(path) {
            return m.clone();
        }
        let m = self.rd.read(path).and_then(|ex| {
            let e = ex.iter().find(|e| e["Type"].as_str() == Some("AnimMontage"))?;
            let p = &e["Properties"];
            let tr = &p["SlotAnimTracks"][0];
            let segs = tr["AnimTrack"]["AnimSegments"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| Segment {
                    seq: obj_path(&s["AnimReference"]),
                    start_pos: s["StartPos"].as_f64().unwrap_or(0.0),
                    anim_start: s["AnimStartTime"].as_f64().unwrap_or(0.0),
                    anim_end: s["AnimEndTime"].as_f64().unwrap_or(0.0),
                    rate: s["AnimPlayRate"].as_f64().unwrap_or(1.0),
                    loops: s["LoopingCount"].as_i64().unwrap_or(1),
                })
                .collect();
            Some(Rc::new(Montage {
                path: path.to_string(),
                slot: tr["SlotName"].as_str().unwrap_or("DefaultSlot").to_string(),
                length: p["SequenceLength"].as_f64().unwrap_or(0.0),
                segments: segs,
                blend_in: p["BlendIn"]["BlendTime"].as_f64().unwrap_or(0.25),
                blend_in_option: blend_option(&p["BlendIn"]["BlendOption"]),
                blend_in_curve: String::new(),
                blend_out: p["BlendOut"]["BlendTime"].as_f64().unwrap_or(0.25),
                blend_out_option: blend_option(&p["BlendOut"]["BlendOption"]),
                trigger: p["BlendOutTriggerTime"].as_f64().unwrap_or(-1.0),
                disables_spine_bending: p["MetaData"][0]["ObjectName"]
                    .as_str()
                    .and_then(|on| {
                        let name = on.rsplit(':').next()?.trim_end_matches('\'');
                        ex.iter().find(|x| x["Name"].as_str() == Some(name) && x["Type"].as_str() == Some("MordhauAnimMetaData"))
                    })
                    .map(|x| x["Properties"]["bDisablesSpineBending"].as_bool().unwrap_or(false))
                    .unwrap_or(false),
            }))
        });
        self.montages.borrow_mut().insert(path.to_string(), m.clone());
        m
    }

    /// PlaySlotAnimationAsDynamicMontage (rva=0x1499390; AnimMontage.dynamic)
    #[allow(clippy::too_many_arguments)]
    pub fn dynamic(&self, seq: &str, slot: &str, bin: f64, bin_option: i32, bin_curve: &str, bout: f64, bout_option: i32) -> Option<Rc<Montage>> {
        let len = self.clip(seq)?.sequence_length as f64;
        Some(Rc::new(Montage {
            path: seq.to_string(),
            slot: slot.to_string(),
            length: len,
            segments: vec![Segment { seq: seq.to_string(), start_pos: 0.0, anim_start: 0.0, anim_end: len, rate: 1.0, loops: 1 }],
            blend_in: bin,
            blend_in_option: bin_option,
            blend_in_curve: bin_curve.to_string(),
            blend_out: bout,
            blend_out_option: bout_option,
            trigger: -1.0,
            // a dynamic montage carries no MetaData (UNCONFIRMED: PlaySlotAnimationAsDynamicMontage copies none)
            disables_spine_bending: false,
        }))
    }

    /// a clip's local pose at t (clamped, or wrapped when looping); the reference pose when the clip is missing
    pub fn sample(&self, sk: &Skeleton, seq: &str, t: f64, looping: bool) -> Vec<FTransform> {
        match self.clip(seq) {
            Some(c) => {
                let len = c.sequence_length as f64;
                let tt = if looping && len > 0.0 { t.rem_euclid(len) } else { t.clamp(0.0, len) };
                sk.sample_local(&c, tt as f32)
            }
            None => sk.ref_local.clone(),
        }
    }
}

/// Reader::obj_json (reader.rs 293-298) writes package.N, where N is the zero-based export index.
/// Metadata references must keep it; obj_path/physics::strip intentionally lose that information.
fn indexed_export_path(path: &str) -> Option<(&str, usize)> {
    let (package, index) = path.rsplit_once('.')?;
    if package.is_empty() || mh_pak::reader::strip_index(package) != package || index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((package, index.parse().ok()?))
}

/// NativeUpdateAnimation examines only MetaData[0], unlike PlayAttackAnim's array loop.
fn first_montage_offhand_metadata(asset: &Value, resolve: impl FnMut(&str, usize) -> Option<Value>) -> Option<Option<(bool, bool, Option<f32>, f32, f32)>> {
    let first = asset.get("Properties")?.get("MetaData")?.as_array()?.first()?;
    let one = serde_json::json!({"Properties":{"MetaData":[first]}});
    Some(offhand_metadata_refs(&one, resolve))
}

fn offhand_metadata_refs(asset: &Value, mut resolve: impl FnMut(&str, usize) -> Option<Value>) -> Option<(bool, bool, Option<f32>, f32, f32)> {
    let refs = asset.get("Properties")?.get("MetaData")?.as_array()?;
    let mut result: Option<(bool, bool, Option<f32>, f32, f32)> = None;
    for reference in refs {
        if reference.is_null() {
            continue;
        }
        let (package, index) = indexed_export_path(reference.get("ObjectPath")?.as_str()?)?;
        // A nonnull unresolved/malformed reference invalidates this lookup; never substitute a local metadata
        // export or return a partial prefix. Successfully resolved null/wrong-class entries are skipped below.
        let metadata = resolve(package, index)?;
        if metadata["Type"].as_str() != Some("MordhauAnimMetaData") {
            continue;
        }
        // All 185 original cooked animation metadata exports use this native class without a Template. Do not
        // invent inherited instance defaults for a template this bounded decoder has not proved.
        if metadata.get("Template").is_some_and(|t| !t.is_null()) {
            return None;
        }
        let p = match metadata.get("Properties") {
            None => &Value::Null,
            Some(p) if p.is_object() => p,
            _ => return None,
        };
        // UMordhauAnimMetaData ctor rva=0x14e6a40 changes only OverrideIdleChangeBlendTime; these absent fields
        // retain false/zero. Explicit false/zero are values, while incorrectly typed fields are unsupported.
        let b = |key: &str| match p.get(key) { None => Some(false), Some(v) => v.as_bool() };
        let f = |key: &str| match p.get(key) {
            None => Some(0.0),
            Some(v) => v.as_f64().map(|v| v as f32).filter(|v| v.is_finite()),
        };
        let speed = if b("bOverridesOffhandIKChangeSpeed")? {
            Some(f("OffhandIKChangeSpeedOverride")?)
        } else {
            result.as_ref().and_then(|r| r.2)
        };
        result = Some((b("bDisablesOffhandIK")?, b("bForcesOffhandIK")?, speed, f("MaxOffhandIKDistance")?, f("MinOffhandIKDistance")?));
    }
    result
}

/// an object reference as the reader dumps it ({"ObjectPath": ...} or a plain string) -> package path
fn obj_path(v: &Value) -> String {
    let s = v["ObjectPath"].as_str().or_else(|| v.as_str()).unwrap_or("");
    crate::physics::strip(s)
}

/// The selected FPerspectiveAnimSequenceBase before its Get() fallback. OnBegin tests the raw FirstPerson
/// member, so a ThirdPerson fallback must never be mistaken for a FirstPerson override (attack_selection_reset).
#[derive(Default)]
struct AttackAnimationPair {
    third_person: String,
    first_person: String,
}

impl AttackAnimationPair {
    fn get(&self, first_person: bool) -> &str {
        if (first_person && !self.first_person.is_empty()) || self.third_person.is_empty() {
            &self.first_person
        } else {
            &self.third_person
        }
    }

    fn release_source(&self, first_person: bool) -> Option<&str> {
        (first_person && !self.first_person.is_empty() && !self.third_person.is_empty()).then_some(self.third_person.as_str())
    }
}

// ---- MotionAnim (motion_anim.gd) -----------------------------------------------------------------------------------

/// The montage side effects of one fighter's motions, replayed from the combat state at the motion's own times
#[derive(Default)]
pub struct MotionAnim {
    /// AAdvancedCharacter LastFlinchAdditiveTime / LastFlinchAdditiveName (BeginFlinchAdditiveOverride, fidelity-audit r8)
    pub last_flinch_additive: Option<(f64, String)>,
    pub montages: MontageSet,
    motion: Option<(MotionId, f64)>,
    inst: Option<u64>,
    inst_of: Vec<((MotionId, f64), u64)>,
    pub seq: String,
    selected_attack_animation: AttackAnimationPair,
    pub blend_in: f64,
    pub offset: f64,
    pub auto_used: bool,
    pub early_release: f64,
    early_release_tf: f64,
    stage: i64,
    recovered: bool,
    hit_blend: bool,
    pub position: f64,
    /// AMordhauCharacter's SpineSpaceAdditive blend (AttackAngling input)
    pub spine: SpineBlend,
    /// WindUpAdditive / TargetAdditive of the current attack
    pub targets: (SpineAdd, SpineAdd),
    /// AAdvancedCharacter::GetLookUpValue for the parry additive
    pub look_up: f64,
    /// the current stab's AnimAngleCurve / AnimAngleCueAmount (UStabMotion::OnTickWindUp)
    pub stab_cue: Option<StabCue>,
    /// Whether this update actually wrote the stab cue target (capture evidence, independent of curve availability).
    pub stab_cue_written: bool,
    /// the additive override + the Additive state machine (additive.rs)
    pub additive: crate::additive::AdditiveOverride,
    pub machine: crate::additive::AdditiveMachine,
    /// ApplyAdditive_2 Alpha this update (AdditiveOverrideWeight)
    pub additive_alpha: f64,
    last_now: Option<f64>,
    /// (last parry motion, its TotalBlocks seen) for OnRep_NetBlock
    blocks_seen: Option<((MotionId, f64), i64)>,
    /// anim instance BlockDirection
    pub block_direction: f64,
    /// AMordhauCharacter::bIsFirstPerson (CameraStyle == 1 && IsViewTarget, CameraStyleChanged rva=0x1532190): the
    /// FPerspective* assets the motions pick (fidelity-audit r3)
    pub first_person: bool,
    /// AAdvancedCharacter::bIsViewTarget: the local camera's target in either perspective.
    pub is_view_target: bool,
    /// Evidence-only fly1p camera keeps the viewed 1P pose while observing from outside.
    pub view_target_debug_override: bool,
    /// UAttackMotion QueuedAnimFor3PRelease (+0x10c0) as the dynamic montage PlayAttackAnim would build
    queued_3p: Option<Rc<Montage>>,
    /// the current attack's class defaults as this perspective reads them (attack_def_1p in first person)
    adef: Option<Rc<mordhau_core::data::AttackDef>>,
    /// the current UBlockedMotion's anim side (first-person r3, fp_anim_audit #1)
    blk: Option<BlockedAnim>,
    /// the current parry's EnterParryRecovery rates / fades for this perspective (fp_anim_items #8)
    parry_rates: Option<ParryRates>,
    /// fp_anim_items FP-6b: UAttackMotion CurrentLeftTorsoAnimMontage's instance, and the current motion's
    /// bUsesLeftTorsoBlend / LeftTorsoBlendSpeed (UMordhauMotion +0x8a / +0x8c; ctor speed 4.0, UMordhauMotion.cpp 200)
    lt_inst: Option<u64>,
    pub uses_left_torso: bool,
    pub left_torso_speed: f64,
}

/// UParryMotion's recovery rates and fades as EnterParryRecovery rva=0x1650540 picks them: the *1P fields when
/// bIsFirstPerson, else the plain ones (decomp UParryMotion.cpp 1478-1560)
#[derive(Clone, Debug, Default)]
pub struct ParryRates {
    pub fail_rate: f64,
    pub fail_fade: f64,
    pub held_fail_rate: f64,
    pub held_fail_fade: f64,
    pub miss_fade: f64,
    pub wall_rate: f64,
    pub wall_fade: f64,
}

/// UBlockedMotion's montage side effects on the blocked attack's montage (FromAttackMontage): OnBegin rva=0x165dc20
/// (decomp UBlockedMotion.cpp 196-290: StopAnim for a stab / clash, bDoReleaseBounceProcedural for a strike, the kick
/// hit-stop rate, the BounceMontage) and OnTick rva=0x16671b0 (decomp 454-728: SetAnimPosition from the bounce curves,
/// StopAnim at TimeUntilFade). A third-person fighter applies what mordhau-core's blocked.rs computed (the *3P fields,
/// ThirdPerson curves); a first-person one recomputes them from the non-3P fields and the FirstPerson curves (the
/// exe's IsFirstPerson branches).
#[derive(Default)]
struct BlockedAnim {
    atk: Option<(MotionId, f64)>,
    inst: Option<u64>,
    fp: Option<Rc<Blocked1P>>,
    procedural: bool,
    faded: bool,
    rate_done: bool,
    montage_done: bool,
}

/// The first-person (non-3P) fields of a UBlockedMotion class: the native ctor rva=0x1644780 values under the
/// Blueprint CDO chain (BP_BlockedMotion overrides most of them)
#[derive(Clone, Debug, Default)]
pub struct Blocked1P {
    pub clash_fade: f64,
    pub stab_world_fade: f64,
    pub stab_parry_range: (f64, f64),
    pub stab_parry_fade: (f64, f64),
    pub stab_chambered_range: (f64, f64),
    pub stab_chambered_fade: (f64, f64),
    pub stab_hit_stop_fade: f64,
    pub hit_stop_curve: String,
    pub hit_stop_scale_curve: String,
    pub hit_stop_release_scale_curve: String,
    pub hit_stop_until_fade: f64,
    pub hit_stop_duration: f64,
    pub hit_stop_fade: f64,
    pub release_scale_curve: String,
    pub parry_range: (f64, f64),
    pub parry_until_fade: (f64, f64),
    pub parry_duration: (f64, f64),
    pub parry_fade: (f64, f64),
    pub chamber_range: (f64, f64),
    pub chamber_until_fade: (f64, f64),
    pub chamber_duration: (f64, f64),
    pub chamber_fade: (f64, f64),
    pub world_until_fade: f64,
    pub world_duration: f64,
    pub world_fade: f64,
}

fn mkey(w: &World, fi: usize, id: MotionId) -> (MotionId, f64) {
    (id, w.m(fi, id).start_time)
}

impl MotionAnim {
    /// Read-only evidence for a runtime attack capture; expose the actual montage as well as selected raw fields.
    pub fn attack_debug(&self) -> Value {
        let active = self.inst.and_then(|id| self.montages.ids.iter().position(|i| *i == id)).map(|i| self.montages.insts[i].asset.path.as_str());
        serde_json::json!({
            "first_person": self.first_person,
            "is_view_target": self.is_view_target,
            "view_target_debug_override": self.view_target_debug_override,
            "stab_cue_available": self.stab_cue.is_some(),
            "stab_cue_written": self.stab_cue_written,
            "sequence": self.seq,
            "active_montage": active,
            "position": self.position,
            "selected_third_person": self.selected_attack_animation.third_person,
            "selected_first_person": self.selected_attack_animation.first_person,
            "queued_release": self.queued_3p.as_ref().map(|m| m.path.as_str()),
        })
    }

    fn inst_for(&self, k: (MotionId, f64)) -> Option<u64> {
        self.inst_of.iter().find(|(m, _)| *m == k).map(|x| x.1)
    }

    /// MotionAnim.update: a motion change replays its begin at its StartTime, then the tick at `now`
    pub fn update(&mut self, w: &World, fi: usize, now: f64, a: &AnimAssets, poses: &mut dyn FnMut(&str, f64) -> (FTransform, FTransform), pose_now: (FTransform, FTransform)) {
        self.stab_cue_written = false;
        let spec = w.spec.clone();
        let Some(cur) = w.cur(fi) else { return };
        self.look_up = w.fighters[fi].look_up_value;
        let k = mkey(w, fi, cur);
        if self.motion != Some(k) {
            let t = w.m(fi, cur).start_time;
            if let Some(old) = self.motion {
                self.leave(w, fi, old, cur, t, &spec);
            }
            self.motion = Some(k);
            self.inst = None;
            // FP-6b: a new motion object starts with bUsesLeftTorsoBlend false and LeftTorsoBlendSpeed 4.0 (UMordhauMotion
            // ctor, UMordhauMotion.cpp 200); a UBlockedMotion copies them from the blocked attack (UBlockedMotion.cpp
            // 133-134), so they are kept for it
            if !matches!(w.m(fi, cur).k, MotionKind::Blocked(_)) {
                self.uses_left_torso = false;
                self.left_torso_speed = 4.0;
                self.lt_inst = None;
            }
            self.stage = -1;
            self.recovered = false;
            self.hit_blend = false;
            self.begin(w, fi, cur, t, a, poses, pose_now);
        }
        self.tick(w, fi, cur, now, &spec, a);
        self.net_block_check(w, fi, now);
        self.additive.tick(now);
        self.montages.update(&spec, now);
        self.update_additive(&spec, now);
        if self.inst_of.len() > 8 {
            let live = self.montages.ids.clone();
            self.inst_of.retain(|(_, i)| live.contains(i));
        }
    }

    fn begin(&mut self, w: &World, fi: usize, id: MotionId, t: f64, a: &AnimAssets, poses: &mut dyn FnMut(&str, f64) -> (FTransform, FTransform), pose_now: (FTransform, FTransform)) {
        let spec = w.spec.clone();
        let m = w.m(fi, id);
        self.stab_cue = None;
        match &m.k {
            MotionKind::Attack(at) => {
                // Keep the asset available across camera handoffs. OnTickWindUp checks IsViewTarget each tick;
                // it does not choose this cue by first/third-person perspective at motion begin.
                self.stab_cue = if at.native == "UStabMotion" { a.stab_cue(&m.bp) } else { None };
                self.begin_attack(w, fi, id, t, a, poses, pose_now);
                // UAttackMotion::OnBegin_Implementation: UpdateSpineSpaceAdditiveTargets rva=0x16417a0, then
                // SetSpineSpaceAdditiveTarget(WindUpAdditive, max(BlendIn, 0.25))
                // the FPerspective AnglingAdditiveWindUp: its FirstPerson half for a first-person character (attack_def_1p)
                let fpd = if self.first_person { Some(a.attack_def_1p(m.def.attack(), &m.bp, &at.native)) } else { None };
                self.targets = attack_spine_targets(fpd.as_deref().unwrap_or(m.def.attack()), at.ty, at.angle_target);
                let bi = self.blend_in;
                self.spine.set_target(self.targets.0.clone(), bi.max(0.25), t);
                // UAttackMotion::OnBegin_Implementation (decomp UAttackMotion.cpp 3934-3943): a ParryPush /
                // AltParryPush override is reset
                if self.additive.ty == "ParryPush" || self.additive.ty == "AltParryPush" {
                    self.additive.reset(t);
                }
            }
            MotionKind::Blocked(b) => {
                // fp_anim_audit #1: the montage side (BlockedAnim): the blocked attack's montage instance, and in first
                // person the OnBegin animation half with the non-3P fields (decomp 196-290)
                let atk = b.from_attack.and_then(|id| w.fighters[fi].motions.get(id.0 as usize).and_then(|x| x.as_ref()).map(|m| (id, m.start_time)));
                // the attack's latest montage instance (after the 1P -> 3P hand-over at release 0.6 the attack owns two; the
                // exe's FromAttackMontage is UAttackMotion::Montage, the one playing)
                let latest = atk.and_then(|k| self.inst_of.iter().rev().find(|(m, _)| *m == k).map(|x| x.1));
                let mut ba = BlockedAnim { atk, inst: latest, ..Default::default() };
                if self.first_person && ba.inst.is_some() && b.bounce_montage.is_empty() {
                    use mordhau_core::combat::enums::{self as en, br};
                    let fp = a.blocked_1p(&m.bp);
                    if b.reason == br::CLASH || en::is_stab(b.from_move) {
                        let fade = match b.reason {
                            br::HIT => fp.stab_hit_stop_fade,
                            br::WORLD => fp.stab_world_fade,
                            br::CLASH => fp.clash_fade,
                            r => {
                                let (rng, fade) = if r == br::PARRY { (fp.stab_parry_range, fp.stab_parry_fade) } else { (fp.stab_chambered_range, fp.stab_chambered_fade) };
                                let k = mordhau_core::ue::range_fraction(rng.0, rng.1, m.end_time - m.start_time, spec.constants.small_number);
                                (fade.1 - fade.0) * k + fade.0
                            }
                        };
                        if let Some(i) = ba.inst.and_then(|i| self.montages.get(i)) {
                            let o = i.asset.blend_out_option;
                            i.stop(&spec, t, fade, o, "");
                        }
                        ba.faded = true;
                    } else {
                        ba.procedural = true;
                    }
                    ba.fp = Some(fp);
                }
                self.blk = Some(ba);
                // UBlockedMotion::OnBegin_Implementation rva=0x165dc20 (decomp 365-440): the core keeps the attack's
                // BounceAdditive when it bounces -> AttackBounce = it, SetAdditiveOverrideType("Bounce", 0.2)
                if !b.bounce_additive.is_empty() {
                    // 1P: the attack's BounceAdditive FirstPerson (UBlockedMotion::OnBegin picks +0xaf0 when
                    // bIsFirstPerson, decomp UBlockedMotion.cpp 336-347; FPerspective Get fallback to ThirdPerson)
                    let fp = if self.first_person {
                        b.from_attack.and_then(|id| w.fighters[fi].motions.get(id.0 as usize).and_then(|x| x.as_ref()).map(|m| m.bp.clone())).map(|bp| a.persp(&bp, "BounceAdditive", true)).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    self.machine.inputs.attack_bounce = if fp.is_empty() { b.bounce_additive.clone() } else { fp };
                    self.additive.set_type("Bounce", 0.2, t);
                }
            }
            MotionKind::Feinted(_) => {
                // UFeintedMotion::OnBegin: SetSpineSpaceAdditiveTarget(zero, SpineSpaceAdditiveBlendOutTime)
                let d = m.def.feinted.as_ref().map(|f| f.spine_space_additive_blend_out_time).unwrap_or(0.0);
                self.spine.set_target(SpineAdd::new(), d, t);
            }
            MotionKind::Parry(p) => {
                // UParryMotion::OnBegin_Implementation rva=0x1660f70 (decomp UParryMotion.cpp 718-765, 904-914): the anim
                // instance's ParryPushAdditive / AltParryPushAdditive / ParryAdditive = the motion's ParriedAdditive /
                // AltParriedAdditive / AnimationAdditive (ThirdPerson, else FirstPerson); no override ->
                // SetAdditiveOverrideType("Parry" (string 0x144341d1c), ParryEnd)
                let fp = self.first_person;
                self.machine.inputs.parry_push_additive = a.persp(&m.bp, "ParriedAdditive", fp);
                self.machine.inputs.alt_parry_push_additive = a.persp(&m.bp, "AltParriedAdditive", fp);
                self.machine.inputs.parry_additive = a.persp(&m.bp, "AnimationAdditive", fp);
                if self.additive.ty == crate::additive::NONE {
                    self.additive.set_type("Parry", p.parry_end, t);
                }
                // UParryMotion::OnBegin_Implementation: SetSpineSpaceAdditiveTarget(zero, 0.3)
                self.spine.set_target(SpineAdd::new(), 0.3, t);
                let pd = m.def.parry();
                self.parry_rates = Some(a.parry_rates(&m.bp, pd, self.first_person));
                // UParryMotion::OnBegin_Implementation rva=0x1660f70: PlayAnim(Animation or AltAnimation, rate 0)
                let alt = p.block_type == mordhau_core::combat::enums::bt::ALT_REGULAR;
                let path = if alt { &pd.alt_animation } else { &pd.animation };
                // 1P: the motion's FPerspectiveAnimMontage Animation / AltAnimation FirstPerson (Get rva=0x165c400)
                let fp_path = if self.first_person { a.persp(&m.bp, if alt { "AltAnimation" } else { "Animation" }, true) } else { String::new() };
                let path = if fp_path.is_empty() { path } else { &fp_path };
                if let Some(asset) = a.montage(path) {
                    let i = self.montages.play(&spec, asset, t, 0.0);
                    self.inst = Some(i);
                    self.inst_of.push((mkey(w, fi, id), i));
                }
            }
            // UFlinchMotion::OnBegin_Implementation rva=0x16606a0: StopAnim(0.4) (no equipment switch motion here)
            MotionKind::Flinch(_) => {
                self.montages.stop_all(&spec, t, 0.4);
                // first person: AAdvancedCharacter::BeginFlinchAdditiveOverride rva=0x1458fb0 ("Flinch1P", "FlinchTwo1P",
                // Duration 1, SnapDegreeToSteps 45; UFlinchMotion::OnBegin decomp 136-140): at most once per 0.3 s,
                // alternating the two names; third person runs OnTookDamage's procedural flinch instead (flinch.rs)
                if self.first_person {
                    let ok = self.last_flinch_additive.as_ref().map(|(lt, _)| lt + 0.3 <= t).unwrap_or(true);
                    if ok {
                        let alt = self.last_flinch_additive.as_ref().map(|(_, n)| n == "Flinch1P").unwrap_or(false);
                        let name = if alt { "FlinchTwo1P" } else { "Flinch1P" };
                        self.additive.set_type(name, 1.0, t);
                        self.last_flinch_additive = Some((t, name.to_string()));
                    }
                }
            }
            // UDisarmedMotion::OnBegin_Implementation rva=0x165ee70 (decomp 88-93): StopAnim(0.4), SetAdditiveOverrideType
            // ("Disarm", 0.6), AnimInstance DisarmDirection = the motion's direction
            MotionKind::Disarmed(d) => {
                self.montages.stop_all(&spec, t, 0.4);
                self.additive.set_type("Disarm", 0.6, t);
                self.machine.inputs.disarm_direction = d.direction;
            }
            // UClimbingMotion::OnBegin_Implementation rva=0x165e550: StopAnim(0.5)
            MotionKind::Climbing(_) => self.montages.stop_all(&spec, t, 0.5),
            _ => {}
        }
    }

    fn leave(&mut self, w: &World, fi: usize, old: (MotionId, f64), new: MotionId, t: f64, spec: &Spec) {
        // the old motion object may already be freed from the slab: only its recorded instance is touched
        let nw = w.m(fi, new);
        let old_m = w.fighters[fi].motions.get(old.0 .0 as usize).and_then(|x| x.as_ref()).filter(|m| m.start_time == old.1);
        let Some(om) = old_m else { return };
        // UBlockedMotion::OnLeave_Implementation rva=0x1665730: a procedural bounce not faded out yet stops the
        // montage (blocked_leave_fade); fp_anim_audit #1
        if matches!(om.k, MotionKind::Blocked(_)) {
            if let Some(ba) = self.blk.take() {
                if ba.fp.is_some() && ba.procedural && !ba.faded {
                    if let Some(i) = ba.inst.and_then(|i| self.montages.get(i)) {
                        let o = i.asset.blend_out_option;
                        i.stop(spec, t, spec.constants.blocked_leave_fade, o, "");
                    }
                }
            }
        }
        // UParryMotion::OnLeave_Implementation rva=0x1665b10 (decomp 1181-1184)
        if matches!(om.k, MotionKind::Parry(_)) && self.additive.ty == "Parry" {
            self.additive.reset(t);
        }
        if let MotionKind::Attack(at) = &om.k {
            // UAttackMotion::OnLeave_Implementation rva=0x16323e0 (0.5 at 0x14163249b)
            if let Some(i) = self.inst.and_then(|i| self.montages.get(i)) {
                if at.stage == mordhau_core::combat::enums::stage::WINDUP {
                    i.set_rate(t, 0.5 / (at.windup_end - om.start_time).max(1e-6));
                } else if at.stage == mordhau_core::combat::enums::stage::RELEASE {
                    i.set_rate(t, 0.5 / (at.release_end - at.windup_end).max(1e-6));
                }
            }
            // OnLeave (0.6 at 0x141632656): out of Windup / Release into anything but a feint or parry
            if at.stage != mordhau_core::combat::enums::stage::RECOVERY && !matches!(nw.k, MotionKind::Feinted(_) | MotionKind::Parry(_)) {
                self.spine.set_target(SpineAdd::new(), 0.6, t);
            }
            if let MotionKind::Feinted(fd) = &nw.k {
                // UAttackMotion::OnFeinted rva=0x16312d0: FeintAnimRate / FeintAnimDurationOffset /
                // FeintAnimMinimumDuration through FPerspectiveFloat::Get(IsFirstPerson) (fp_anim_items #5): the
                // attack's attack_def_1p in first person
                let ad = self.adef.as_deref().filter(|_| self.first_person).unwrap_or(om.def.attack());
                let fade = (ad.feint_anim_duration_offset + fd.lock_out_time).max(ad.feint_anim_minimum_duration);
                if let Some(i) = self.inst.and_then(|i| self.montages.get(i)) {
                    i.set_rate(t, ad.feint_anim_rate);
                }
                self.montages.stop_all(spec, t, fade);
            }
        }
    }

    /// the sequence or montage an attack plays: PrepareAnimationData rva=0x1637140 + UKickMotion rva=0x166b200
    /// AMordhauCharacter::OnRep_NetBlock rva=0x155a4e0 (decomp AMordhauCharacter.cpp 12452-12650), run on the defender
    /// when an attack is parried: a new block on the fighter's last parry motion (TotalBlocks went up; its
    /// RiposteWindowStart is the block time for a non-held parry); BlockDirection = the blocked move RightStrike ? 90 :
    /// LeftStrike ? -90 : 0; not ParryPush / AltParryPush already, current motion neither an attack nor a flinch ->
    /// SetAdditiveOverrideType(BlockType AltRegular ? "AltParryPush" : "ParryPush" (0x144341f00 / 0x144341ef0), 0.25)
    fn net_block_check(&mut self, w: &World, fi: usize, now: f64) {
        use mordhau_core::combat::enums::{br, mv};
        let Some(pid) = w.fighters[fi].last_parry_motion else { return };
        let Some(pm) = w.fighters[fi].motions.get(pid.0 as usize).and_then(|x| x.as_ref()) else { return };
        let Some(p) = pm.parry() else { return };
        let key = (pid, pm.start_time);
        let seen = match self.blocks_seen {
            Some((k, n)) if k == key => n,
            _ => 0,
        };
        if p.total_blocks <= seen {
            return;
        }
        self.blocks_seen = Some((key, p.total_blocks));
        let t = if !p.b_is_block_holdable && p.riposte_window_start > 0.0 && p.riposte_window_start <= now { p.riposte_window_start } else { now };
        let mut blocked_move = -1;
        for (oi, _) in w.fighters.iter().enumerate() {
            if oi == fi {
                continue;
            }
            if let Some(MotionKind::Blocked(b)) = w.cur_m(oi).map(|m| &m.k) {
                if b.reason == br::PARRY {
                    blocked_move = b.from_move;
                }
            }
        }
        self.block_direction = if blocked_move == mv::RIGHT_STRIKE { 90.0 } else if blocked_move == mv::LEFT_STRIKE { -90.0 } else { 0.0 };
        self.machine.inputs.block_direction = self.block_direction;
        let cur_ok = !matches!(w.cur_m(fi).map(|m| &m.k), Some(MotionKind::Attack(_)) | Some(MotionKind::Flinch(_)));
        if self.additive.ty != "ParryPush" && self.additive.ty != "AltParryPush" && cur_ok {
            let alt = p.block_type == mordhau_core::combat::enums::bt::ALT_REGULAR;
            self.additive.set_type(if alt { "AltParryPush" } else { "ParryPush" }, 0.25, t);
        }
    }

    /// UBlockedMotion::OnTick_Implementation rva=0x16671b0 (decomp UBlockedMotion.cpp 454-728) on the blocked attack's
    /// montage (fp_anim_audit #1). Third person: mordhau-core's per-tick results (bounce_anim_position, stop_anim_fade,
    /// kick_hit_stop_anim_rate, bounce_montage; the *3P fields and ThirdPerson curves). First person: the same
    /// procedure with the non-3P fields (Blocked1P) and the attack's FirstPerson bounce curves (FPerspective Get
    /// rva=0x165c400: FirstPerson, else ThirdPerson), as the exe's IsFirstPerson branches pick them.
    #[allow(clippy::too_many_arguments)]
    fn tick_blocked(&mut self, w: &World, fi: usize, id: MotionId, b: &mordhau_core::combat::blocked::BlockedMotion, now: f64, spec: &Spec, a: &AnimAssets) {
        use mordhau_core::combat::enums::{self as en, br};
        let Some(mut ba) = self.blk.take() else { return };
        // evidence only (first-person r3): MH_NO_BOUNCE skips this, for before / after captures of the bounce
        if std::env::var_os("MH_NO_BOUNCE").is_some() {
            self.blk = Some(ba);
            return;
        }
        let m = w.m(fi, id);
        let inst = ba.inst;
        // OnBegin's kick hit stop: SetAnimRate(FromAttackMontage, KickHitStopAnimRate) (no 3P variant)
        if !ba.rate_done && b.kick_hit_stop_anim_rate >= 0.0 {
            ba.rate_done = true;
            if let Some(i) = inst.and_then(|i| self.montages.get(i)) {
                i.set_rate(m.start_time, b.kick_hit_stop_anim_rate);
            }
        }
        // OnBegin: the attack's BounceMontage (PlayAnim)
        if !ba.montage_done && !b.bounce_montage.is_empty() {
            ba.montage_done = true;
            if let Some(asset) = a.montage(&b.bounce_montage) {
                let i = self.montages.play(spec, asset, m.start_time, 1.0);
                self.inst_of.push((mkey(w, fi, id), i));
            }
        }
        match ba.fp.clone() {
            None => {
                // third person: the core's StopAnim / SetAnimPosition of this tick
                if !ba.faded && b.stop_anim_fade >= 0.0 {
                    ba.faded = true;
                    if let Some(i) = inst.and_then(|i| self.montages.get(i)) {
                        let o = i.asset.blend_out_option;
                        i.stop(spec, now, b.stop_anim_fade, o, "");
                    }
                }
                if b.bounce_anim_position >= 0.0 {
                    if let Some(i) = inst.and_then(|i| self.montages.get(i)) {
                        i.set_position(now, b.bounce_anim_position);
                    }
                }
            }
            Some(fp) if ba.procedural => {
                let c = &spec.constants;
                let (start, end) = (m.start_time, m.end_time);
                let (until_fade, fade_out, duration) = match b.reason {
                    br::HIT => (fp.hit_stop_until_fade, fp.hit_stop_fade, fp.hit_stop_duration),
                    br::WORLD => (fp.world_until_fade, fp.world_fade, fp.world_duration),
                    r => {
                        let (rng, tuf, fo, bd) = if r == br::PARRY {
                            (fp.parry_range, fp.parry_until_fade, fp.parry_fade, fp.parry_duration)
                        } else {
                            (fp.chamber_range, fp.chamber_until_fade, fp.chamber_fade, fp.chamber_duration)
                        };
                        let k = mordhau_core::ue::range_fraction(rng.0, rng.1, end - start, c.small_number);
                        ((tuf.1 - tuf.0) * k + tuf.0, (fo.1 - fo.0) * k + fo.0, (bd.1 - bd.0) * k + bd.0)
                    }
                };
                if until_fade + start < now && !ba.faded {
                    ba.faded = true;
                    if let Some(i) = inst.and_then(|i| self.montages.get(i)) {
                        let o = i.asset.blend_out_option;
                        i.stop(spec, until_fade + start, fade_out, o, "");
                    }
                }
                let atk = ba.atk.and_then(|(aid, st)| w.fighters[fi].motions.get(aid.0 as usize).and_then(|x| x.as_ref()).filter(|x| x.start_time == st));
                if now < start + c.blocked_bounce_window {
                    if let Some(am) = atk {
                        if let Some(at) = am.attack() {
                            let nt = mordhau_core::ue::normalized_time(start, start + duration, now);
                            let lrn = at.last_release_norm;
                            let fpc = |k: &str| a.persp(&am.bp, k, true);
                            let (curve, scale) = if b.reason == br::HIT {
                                (fp.hit_stop_curve.clone(), fp.hit_stop_scale_curve.clone())
                            } else if !en::is_strike(at.mv) {
                                (String::new(), String::new())
                            } else if b.reason == br::WORLD {
                                (fpc("WorldBounceCurve"), fpc("WorldBounceScaleCurve"))
                            } else {
                                (if lrn <= c.blocked_late_bounce_release { fpc("ParryBounceCurve") } else { fpc("ParryLateBounceCurve") }, fpc("ParryBounceScaleCurve"))
                            };
                            let rs_curve = if b.reason == br::HIT { &fp.hit_stop_release_scale_curve } else { &fp.release_scale_curve };
                            let length = w.fighters[fi].tracer.length;
                            let release_scale = a.curve_at(spec, rs_curve, length).unwrap_or(c.blocked_unit_scale);
                            if let (Some(cv), Some(sv)) = (a.curve_at(spec, &curve, nt), a.curve_at(spec, &scale, lrn)) {
                                let pos = (cv * release_scale * sv + lrn * c.blocked_bounce_half + c.blocked_bounce_half).clamp(0.0, 1.0);
                                if let Some(i) = inst.and_then(|i| self.montages.get(i)) {
                                    i.set_position(now, pos);
                                }
                            }
                        }
                    }
                }
            }
            Some(_) => {}
        }
        self.blk = Some(ba);
    }

    /// the anim-instance side: AdditiveOverrideWeight, then the machine update (re-initialised when it becomes
    /// relevant again)
    fn update_additive(&mut self, spec: &Spec, now: f64) {
        let dt = self.last_now.map(|l| now - l).unwrap_or(0.0);
        self.last_now = Some(now);
        let was = self.additive_alpha;
        self.additive.update_anim(dt);
        self.additive_alpha = self.additive.weight;
        if self.additive_alpha > 1e-5 {
            if was <= 1e-5 {
                self.machine.reinit(now);
            }
            let ty = self.additive.anim_type.clone();
            self.machine.update(spec, &ty, now);
        }
    }

    fn attack_animation(&self, w: &World, fi: usize, id: MotionId, a: &AnimAssets) -> AttackAnimationPair {
        let m = w.m(fi, id);
        let at = m.attack().unwrap();
        let d = m.def.attack();
        use mordhau_core::combat::enums::at as ty;
        if at.native == "UKickMotion" {
            let Some(e) = &w.fighters[fi].weapon_equip else { return AttackAnimationPair::default() };
            // UKickMotion::PrepareAnimationData_Implementation rva=0x166b200 (decomp UKickMotion.cpp): bIsAirKick picks
            // the equipment's JumpKick / JumpKickCombo / JumpKickRiposteAnimation (+0x7d0 / +0x7f0 / +0x7e0), as set
            // (a null field plays nothing) (fp_anim_items #10)
            if at.b_is_air_kick {
                let key = match at.ty {
                    ty::COMBO => "JumpKickComboAnimation",
                    ty::RIPOSTE => "JumpKickRiposteAnimation",
                    _ => "JumpKickAnimation",
                };
                return AttackAnimationPair { third_person: a.equipment_obj(&e.path, key), ..Default::default() };
            }
            let third_person = match at.ty {
                ty::COMBO => e.kick_combo_animation.clone(),
                ty::RIPOSTE => e.kick_riposte_animation.clone(),
                _ => e.kick_animation.clone(),
            };
            return AttackAnimationPair { third_person, ..Default::default() };
        }
        // PrepareAnimationData rva=0x1637140 copies BOTH members of the selected pair, including nulls.
        // Preserve raw member presence for OnBegin's release queue; Get's fallback is only for playback.
        let (key, fallback) = match at.ty {
            ty::POST_CLASH => ("ClashAnimation", &d.clash_animation),
            ty::RIPOSTE => {
                // Original also excludes a shield in ComingFromAsParry->WeaponPtr; the core does not yet preserve
                // that originating pointer. Kept as a separate pending issue in attack_selection_reset.md.
                let alt = m.coming_from.and_then(|c| w.m(fi, c).parry()).map(|p| p.block_type == mordhau_core::combat::enums::bt::ALT_REGULAR).unwrap_or(false);
                if alt { ("AltRiposteAnimation", &d.alt_riposte_animation) } else { ("RiposteAnimation", &d.riposte_animation) }
            }
            _ => ("Animation", &d.animation),
        };
        let cdo = a.cdo(&m.bp);
        if let Some(pair) = cdo.get(key) {
            AttackAnimationPair { third_person: obj_path(&pair["ThirdPerson"]), first_person: obj_path(&pair["FirstPerson"]) }
        } else {
            // Records-only/native fixtures have no Blueprint pair. Keep their established TP asset fallback.
            AttackAnimationPair { third_person: fallback.clone(), ..Default::default() }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_attack(&mut self, w: &World, fi: usize, id: MotionId, t: f64, a: &AnimAssets, poses: &mut dyn FnMut(&str, f64) -> (FTransform, FTransform), pose_now: (FTransform, FTransform)) {
        use mordhau_core::combat::enums::at as ty;
        let spec = w.spec.clone();
        let m = w.m(fi, id);
        let at = m.attack().unwrap();
        let d: mordhau_core::data::AttackDef = if self.first_person { (*a.attack_def_1p(m.def.attack(), &m.bp, &at.native)).clone() } else { m.def.attack().clone() };
        self.adef = if self.first_person { Some(a.attack_def_1p(m.def.attack(), &m.bp, &at.native)) } else { None };
        let animation = self.attack_animation(w, fi, id, a);
        self.seq = animation.get(self.first_person).to_string();
        // UAttackMotion::OnBegin_Implementation rva=0x162eda0 (decomp UAttackMotion.cpp 1623-1660): in first person with
        // a FirstPerson Animation, the ThirdPerson one (an AnimSequence) is queued for the release
        self.queued_3p = None;
        if let Some(tpa) = animation.release_source(self.first_person) {
            // (decomp 1626-1660) Animation.FirstPerson set: QueuedAnimFor3PRelease = Animation.ThirdPerson when it is an
            // UAnimSequence; when it is an UAnimMontage, its SlotAnimTracks[0].AnimTrack.AnimSegments[0].AnimReference
            // if that is an UAnimSequence (first-person r3: the montage case was dropped, so a 1P strike whose
            // ThirdPerson is a montage kept the 1P clip through the release and recovery)
            let queued = if let Some(mt) = a.montage(tpa) {
                mt.segments.first().map(|s| s.seq.clone()).filter(|q| !q.is_empty() && a.montage(q).is_none() && a.clip(q).is_some())
            } else {
                a.clip(tpa).is_some().then(|| tpa.to_string())
            };
            if let Some(q) = queued {
                // PlayAttackAnim(Queued, PlayRate 0, FadeIn 0, bStopExistingAnims false, no curve): HermiteCubic, BlendOut
                self.queued_3p = a.dynamic(&q, &a.slot_default, 0.0, 2, "", d.blend_out, 2);
            }
        }
        self.selected_attack_animation = animation;
        let normal = d.normal_blend_in;
        let mut bi = normal;
        if let Some(pm) = w.fighters[fi].last_parry_motion {
            let wgt = self.montages.weight_of(&spec, self.inst_for(mkey(w, fi, pm)), t);
            bi = wgt * (d.normal_parry_slow_blend_in - normal) + normal;
        }
        if let Some(pl) = at.previous_last_attack {
            let wgt = self.montages.weight_of(&spec, self.inst_for(mkey(w, fi, pl)), t);
            bi = bi.max(wgt * (d.normal_slow_blend_in - normal) + normal);
        }
        let mut curve = d.blend_in_curve.clone();
        let windup = at.windup_end - m.start_time;
        self.early_release = at.early_release;
        self.early_release_tf = at.early_release_tf;
        // AttackAutoBlend.settings: the auto flag / steps per type
        let (mut auto, mut steps) = (d.regular_attacks_use_auto_blend_in, d.auto_blend_optimize_forward_steps);
        match at.ty {
            ty::COMBO => {
                curve = d.combo_blend_in_curve.clone();
                bi = d.combo_blend_in * windup;
                auto = d.combo_attacks_use_auto_blend_in;
            }
            ty::POST_CLASH => {
                bi = d.post_clash_blend_in;
                auto = d.post_clash_attacks_use_auto_blend_in;
            }
            ty::MORPH => {
                bi = d.morph_blend_in;
                curve = d.morph_blend_in_curve.clone();
                auto = d.morph_attacks_use_auto_blend_in;
            }
            ty::RIPOSTE => {
                curve = d.riposte_blend_in_curve.clone();
                bi = d.riposte_blend_in;
                auto = d.riposte_attacks_use_auto_blend_in;
                steps = d.riposte_auto_blend_optimize_forward_steps;
            }
            _ => {}
        }
        if at.native == "UKickMotion" {
            bi = normal;
        }
        self.offset = d.windup_animation_start_time_offset;
        self.auto_used = false;
        let montage = a.montage(&self.seq);
        if let Some(mt) = &montage {
            bi = mt.blend_in;
        } else if auto && windup > 0.0 && !self.seq.is_empty() && a.clip(&self.seq).is_some() {
            let seq = self.seq.clone();
            let length = w.fighters[fi].tracer.length;
            let r = auto_blend_search(&spec, &d, steps, m.start_time, at.windup_end, at.release_end, length, pose_now, &mut |tt| poses(&seq, tt));
            bi = r.0;
            self.offset = r.1;
            self.auto_used = true;
        }
        bi = bi.min(windup);
        self.blend_in = bi;
        let asset = match montage {
            Some(mt) => Some(mt),
            None => a.dynamic(&self.seq, &a.slot_default, bi, if curve.is_empty() { 2 } else { 14 }, &curve, d.blend_out, 2),
        };
        if let Some(asset) = asset {
            let i = self.montages.play(&spec, asset, t, 0.0);
            self.inst = Some(i);
            self.inst_of.push((mkey(w, fi, id), i));
        }
        // fp_anim_items FP-6b. UAttackMotion::OnBegin (decomp UAttackMotion.cpp 1973-1981):
        // CurrentLeftTorsoAnimMontage = FPerspectiveAnimMontage::Get(LeftTorsoMontage, IsFirstPerson); when set,
        // bUsesLeftTorsoBlend = true, LeftTorsoBlendSpeed = 1 / its BlendIn (+0xb0; 1e9 for ~0), PlayAnim(it, rate 0).
        // PrepareAnimationData's riposte branch (decomp 4011-4017) first copies RiposteLeftTorsoMontage over it.
        self.lt_inst = None;
        let lt_key = if at.ty == ty::RIPOSTE { "RiposteLeftTorsoMontage" } else { "LeftTorsoMontage" };
        let lt = a.persp(&m.bp, lt_key, self.first_person);
        if let Some(mt) = a.montage(&lt) {
            self.uses_left_torso = true;
            self.left_torso_speed = if mt.blend_in.abs() > 1e-8 { 1.0 / mt.blend_in } else { 1e9 };
            let i = self.montages.play_extra(&spec, mt, t, 0.0);
            self.lt_inst = Some(i);
        }
        self.stage = 0;
    }

    /// UAttackMotion::OnTick_Implementation's SetAnimPosition (disasm 0x141633066..0x141633571; MotionAnim.attack_position)
    /// (position, release spine alpha: -1 in Windup, smoothstep of the early-release fraction, then 1)
    fn attack_position(&self, w: &World, fi: usize, id: MotionId, now: f64, assets: &AnimAssets) -> (f64, f64) {
        use mordhau_core::combat::enums::at as ty;
        let spec = &w.spec;
        let m = w.m(fi, id);
        let at = m.attack().unwrap();
        let d = self.adef.as_deref().unwrap_or(m.def.attack());
        let (we, re) = (at.windup_end, at.release_end);
        if at.stage == 0 {
            let mut x = mordhau_core::ue::normalized_time(m.start_time, we, now);
            x = mordhau_core::combat::attack::smoothed_windup(d, m.start_time, we, re, self.offset, x, spec.constants.small_number);
            // PrepareAnimationData rva=0x1637140 selects ComboWindUpCurve for the captured perspective.
            // The core keeps its TP gameplay curve; the local FP montage uses the original FP curve.
            let curve = if at.ty == ty::COMBO { &d.combo_windup_curve } else { &at.windup_curve };
            if !curve.is_empty() {
                x = if at.ty == ty::COMBO {
                    assets.curve_at(spec, curve, x).unwrap_or_else(|| panic!("original combo windup curve unavailable: {curve}"))
                } else {
                    spec.curve_value(curve, x)
                };
            }
            return ((0.5 - self.offset) * x + self.offset, -1.0);
        }
        let (r, sa) = w.attack_release_state(fi, id, now);
        let q = w.qf();
        (q(q(r + 1.0) * 0.5), sa)
    }

    fn tick(&mut self, w: &World, fi: usize, id: MotionId, now: f64, spec: &Spec, a: &AnimAssets) {
        self.stab_cue_written = false;
        let m = w.m(fi, id);
        match &m.k {
            MotionKind::Blocked(b) => self.tick_blocked(w, fi, id, b, now, spec, a),
            MotionKind::Attack(at) => {
                let s = at.stage;
                if s == 0 || s == 1 {
                    let (p, sa) = self.attack_position(w, fi, id, now, a);
                    self.position = p;
                    // OnTick_Implementation rva=0x16328c0 (decomp 2837-2845): in Release, NewPosition >=
                    // AnimationTimeFor3PTransition (0.6, ctor decomp 3582; no Blueprint override) with an anim queued ->
                    // PlayAttackAnim(QueuedAnimFor3PRelease), then SetAnimPosition on the new montage
                    if s == 1 && p >= 0.6 {
                        if let Some(q) = self.queued_3p.take() {
                            // PlayAttackAnim rva=0x1636d90 also reads the newly played sequence's offhand metadata
                            // (UAttackMotion.cpp 7194-7230); keep offhand_motion's source aligned with playback.
                            self.seq = q.path.clone();
                            let i = self.montages.play(spec, q, now, 0.0);
                            self.inst = Some(i);
                            self.inst_of.push((mkey(w, fi, id), i));
                        }
                    }
                    if let Some(i) = self.inst.and_then(|i| self.montages.get(i)) {
                        i.set_position(now, p);
                    }
                    // FP-6b: SetAnimPosition(CurrentLeftTorsoAnimMontage, NewPosition) (decomp 2849-2851)
                    if let Some(i) = self.lt_inst.and_then(|i| self.montages.get(i)) {
                        i.set_position(now, p);
                    }
                    if s == 0 && !self.is_view_target {
                        if let Some(c) = &self.stab_cue {
                            // UStabMotion::OnTickWindUp rva=0x1666710 calls IsViewTarget (0x8e1fb0) now:
                            // suppress the cue for the locally viewed character in both 1P and 3P.
                            // Target = Lerp(WindUpAdditive, cue, AnimAngleCurve(LastWindupNormalizedTime))
                            let x = spec.curve_eval(&c.keys, at.last_windup_norm, &c.pre, &c.post);
                            let t = spine_lerp(&self.targets.0, &stab_cue_additive(&self.targets.0, c.amount), x);
                            self.spine.write_target(t);
                            self.stab_cue_written = true;
                        }
                    }
                    if s == 1 {
                        // OnTick release: SpineSpaceAdditiveTarget = Lerp(WindUp, Target, alpha) (Target at alpha 1)
                        let t = if sa < 1.0 { spine_lerp(&self.targets.0, &self.targets.1, sa) } else { self.targets.1.clone() };
                        self.spine.write_target(t);
                    }
                } else if s == 2 && !self.recovered {
                    // UAttackMotion::EnterRecovery rva=0x161b7b0: position 1, rate = clamp(MissRecoveryToPlayRate /
                    // max(EndTime - now, 0.01), MissRecoveryPlayRateClamp)
                    self.recovered = true;
                    let t = at.release_end;
                    self.position = 1.0;
                    // FPerspective Get(IsFirstPerson) of MissRecoveryToPlayRate / MissRecoveryPlayRateClamp (fp_anim_items #7)
                    let d = self.adef.as_deref().filter(|_| self.first_person).unwrap_or(m.def.attack());
                    let c = d.miss_recovery_play_rate_clamp;
                    let rate = (d.miss_recovery_to_play_rate / (m.end_time - t).max(0.01)).clamp(c.xf(), c.yf());
                    if let Some(i) = self.inst.and_then(|i| self.montages.get(i)) {
                        i.set_position(t, 1.0);
                        i.set_rate(t, rate);
                    }
                    // FP-6b: EnterRecovery sets the same rate on CurrentLeftTorsoAnimMontage (decomp 3178-3180)
                    if let Some(i) = self.lt_inst.and_then(|i| self.montages.get(i)) {
                        i.set_rate(t, rate);
                    }
                    self.spine.set_target(SpineAdd::new(), 0.6, t);
                }
                if s == 2 && at.b_has_hit && !self.hit_blend && at.release_end + 0.1 < now {
                    self.hit_blend = true;
                    let t2 = at.release_end + 0.1;
                    // SuccessfulHitPlayRate / SuccessfulHitBlendOutAnimTime through FPerspective Get (fp_anim_items #7)
                    let d = self.adef.as_deref().filter(|_| self.first_person).unwrap_or(m.def.attack());
                    if d.successful_hit_play_rate != -1.0 {
                        if let Some(i) = self.inst.and_then(|i| self.montages.get(i)) {
                            i.set_rate(t2, d.successful_hit_play_rate);
                        }
                        // FP-6b: and on CurrentLeftTorsoAnimMontage (decomp 2866-2870)
                        if let Some(i) = self.lt_inst.and_then(|i| self.montages.get(i)) {
                            i.set_rate(t2, d.successful_hit_play_rate);
                        }
                    }
                    if d.successful_hit_blend_out_anim_time != -1.0 {
                        self.montages.stop_all(spec, t2, d.successful_hit_blend_out_anim_time);
                    }
                }
                self.stage = s;
            }
            MotionKind::Parry(p) => {
                let pd = m.def.parry();
                if p.stage == 0 {
                    // UParryMotion::OnTick_Implementation rva=0x1668980: SetAnimPosition(n * 0.5)
                    let mut end = m.start_time + p.parry_up_time + pd.parry_up_time_delay_expected_delay;
                    if p.b_is_shield_wall {
                        end += pd.shield_wall_raise_time_anim_offset;
                    }
                    let mut n = mordhau_core::ue::normalized_time(m.start_time, end, now);
                    if pd.b_legacy_animation_playing_method {
                        n = if p.b_is_shield_wall { let c = n.clamp(0.0, 1.0); c * c * (3.0 - 2.0 * c) } else { 1.0 - (1.0 - n).powf(1.75) };
                    }
                    self.position = n * 0.5;
                    let pos = self.position;
                    if let Some(i) = self.inst.and_then(|i| self.montages.get(i)) {
                        i.set_position(now, pos);
                    }
                    self.spine.write_target(parry_spine_target(pd, p.block_type, self.look_up));
                } else if !self.recovered {
                    // UParryMotion::EnterParryRecovery rva=0x1650540
                    self.recovered = true;
                    let t = p.recovery_start_time;
                    use mordhau_core::combat::enums::pr;
                    // UParryMotion::EnterParryRecovery rva=0x1650540 (decomp UParryMotion.cpp 1478-1560): the 1P fields
                    // when bIsFirstPerson, the shield-wall branch (fp_anim_items #8)
                    let r = self.parry_rates.clone().unwrap_or_default();
                    let wall = p.b_is_shield_wall;
                    let rate = if wall { r.wall_rate } else if p.recovery_type == pr::FAIL { if p.b_is_block_holdable { r.held_fail_rate } else { r.fail_rate } } else { 1.0 };
                    if let Some(i) = self.inst.and_then(|i| self.montages.get(i)) {
                        i.set_rate(t, rate);
                    }
                    if p.recovery_type == pr::MISS {
                        self.montages.stop_all(spec, t, r.miss_fade);
                    } else if wall {
                        self.montages.stop_all(spec, t, r.wall_fade);
                    } else if p.recovery_type == pr::FAIL {
                        self.montages.stop_all(spec, t, if p.b_is_block_holdable { r.held_fail_fade } else { r.fail_fade });
                    }
                    self.spine.set_target(SpineAdd::new(), 0.5, t);
                }
            }
            _ => {}
        }
    }
}

// ---- AutoBlend (attack_auto_blend.gd) ------------------------------------------------------------------------------

fn quat_yaw(q: FQuat) -> f64 {
    let (x, y, z, w) = (q.x as f64, q.y as f64, q.z as f64, q.w as f64);
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z)).to_degrees()
}
fn normalize_axis(a: f64) -> f64 {
    let mut r = a % 360.0;
    if r < 0.0 {
        r += 360.0;
    }
    if r > 180.0 {
        r -= 360.0;
    }
    r
}

/// UAttackMotion::AutoBlendCalculateWeaponDistance rva=0x1615930 (UE component space: forward -X, up +Z)
fn weapon_distance(d: &mordhau_core::data::AttackDef, deg_to_rad: f64, to_cm: f64, length: f64, hand: FTransform, anim: FTransform) -> f64 {
    let ang = |v: FVector| {
        let (a, b) = (hand.rot.rotate(v), anim.rot.rotate(v));
        (a.dot(b) as f64).clamp(-1.0, 1.0).acos()
    };
    let fwd = ang(FVector::new(-1.0, 0.0, 0.0));
    let up = ang(FVector::new(0.0, 0.0, 1.0));
    let th = d.auto_blend_consider_up_vector_if_larger_than_angle * deg_to_rad;
    let angle = if up > th && up > fwd { up } else { fwd };
    angle * length * to_cm + (hand.loc - anim.loc).length() as f64
}

/// AutoBlendGetBlendTime rva=0x1615c60: max(SpineCurve(angle), WeaponCurve(distance))
fn blend_time(spec: &Spec, d: &mordhau_core::data::AttackDef, dist: f64, spine: f64) -> f64 {
    let st = if d.auto_blend_in_spine_curve.is_empty() { 0.0 } else { spec.curve_value(&d.auto_blend_in_spine_curve, spine) };
    let mut t = st;
    if !d.auto_blend_in_weapon_curve.is_empty() {
        t = spec.curve_value(&d.auto_blend_in_weapon_curve, dist);
        if t <= st {
            t = st;
        }
    }
    t
}

/// The AutoBlend search of UAttackMotion::OnBegin_Implementation rva=0x162eda0 (exe 0x14162fb19..0x14162ff7b):
/// (BlendIn, WindUpAnimationStartTimeOffset). pose = (RightWeapon, Spine1) component transforms.
#[allow(clippy::too_many_arguments)]
pub fn auto_blend_search(spec: &Spec, d: &mordhau_core::data::AttackDef, steps: i64, start: f64, we: f64, re: f64, length: f64, now: (FTransform, FTransform), clip: &mut dyn FnMut(f64) -> (FTransform, FTransform)) -> (f64, f64) {
    let c = &spec.constants;
    let windup = we - start;
    let class_offset = d.windup_animation_start_time_offset;
    let gap = |p: (FTransform, FTransform)| -> (f64, f64) {
        let yaw = normalize_axis(quat_yaw(p.1.rot) - quat_yaw(now.1.rot)).abs();
        (yaw, weapon_distance(d, c.auto_blend_deg_to_rad, c.auto_blend_angle_to_cm, length, now.0, p.0))
    };
    let (mut best_blend, mut best_offset) = (c.auto_blend_no_blend_yet, 0.0);
    for i in 0..=steps {
        let t = i as f64 * d.auto_blend_optimize_forward_step_size;
        let first = gap(clip(t));
        let mut blend = blend_time(spec, d, first.1, first.0);
        if blend >= c.auto_blend_min_blend {
            let sm = mordhau_core::combat::attack::smoothed_windup(d, start, we, re, class_offset, blend / windup, c.small_number);
            let t2 = (sm * c.auto_blend_half + t).min(c.auto_blend_half);
            let second = gap(clip(t2));
            blend = blend_time(spec, d, (first.1 + second.1) * c.auto_blend_half, (first.0 + second.0) * c.auto_blend_half);
        }
        if blend < best_blend {
            best_blend = blend;
            best_offset = t;
        }
    }
    (best_blend, best_offset)
}

// ---- the per-fighter poser (fighter_anim.gd) ------------------------------------------------------------------------

/// One fighter's animation: MotionAnim + the graph path of FighterAnim.update
#[derive(Default)]
pub struct FighterAnim {
    pub ma: MotionAnim,
    pub upper_w: Vec<f32>,
    pub last_cs: Vec<FTransform>,
    /// the final local pose (component = Skeleton::to_component)
    pub local: Vec<FTransform>,
    pub idle: String,
    /// AttackAngling node field -> bone index
    pub angling: Vec<(String, usize)>,
    /// the procedural nodes around AttackAngling (procedural.rs)
    pub proc_bones: crate::procedural::ProcBones,
    /// bIsFirstPerson of this fighter (Sim::set_first_person, fidelity-audit r3)
    pub first_person: bool,
    /// Local camera ownership, independent of first/third-person asset selection.
    pub is_view_target: bool,
    /// The external fly1p observer explicitly retains the local view-target pose for diagnostics.
    pub view_target_debug_override: bool,
    /// fp_anim_items FP-6b: the anim instance's LeftTorsoBlendWeight (+0x9ec) and the last update time
    pub left_torso_w: f64,
    lt_last: Option<f64>,
    pub proc: crate::procedural::ProcState,
    /// the strike look counter-compensation (procedural.rs CounterCompensation; node 457 ModifyBone_90)
    pub counter_comp: crate::procedural::CounterCompensation,
    /// the right hand equipment's RightWeaponBoneCosmeticTransform (FRotator, translation)
    pub cosmetic: ((f32, f32, f32), FVector),
    /// the right-hand equipment's RightShoulderOffset1P / LeftShoulderOffset1P (AnimAssets::shoulder_offsets_1p)
    pub shoulder_1p: (FVector, FVector),
    /// UpperAdditiveA (UpdateEquipmentData: the main equipment's UpperAdditive(1P), Shield*Additive with a shield)
    pub upper_additive: String,
    /// the offhand grip IK (procedural::OffhandState) and its weapon data (normal / alternate mode)
    pub offhand: crate::procedural::OffhandState,
    pub offhand_weapon: Option<(crate::procedural::OffhandWeapon, crate::procedural::OffhandWeapon)>,
    /// the weapon's UpperBlendSpace (BlendSpacePlayer_29 = UpperBlendSpaceA) and its input this frame
    /// (Direction, Helper_UBVelocity; Sim::anim_input)
    pub upper_bs: Option<Rc<crate::blendspace::BlendSpace>>,
    pub upper_input: (f64, f64),
    /// the LowerBody state machine (lower.rs); None = the reference pose (r4 behaviour)
    pub lower: Option<crate::lower::LowerBody>,
    /// this frame's turn-in-place inputs (Sim) and the dedicated-server flag
    pub turn: crate::procedural::TurnInput,
    pub dedicated_server: bool,
    /// the airborne states of the UpperBody / LowerBody machines (air.rs) and this update's inputs: bJumped (a jump
    /// made since the last update; cleared here) and the character's IsAirborne
    pub air: crate::air::AirState,
    pub jumped: bool,
    pub airborne: bool,
    /// the procedural (cosmetic) flinch of a victim (flinch.rs, fidelity-audit r8)
    pub flinch: crate::flinch::Flinch,
    /// fp-anim r1: MovementSpeedScale (NativeUpdateAnimation decomp 2443-2522; the 1P sprint factor, Sim::upper_bs_input)
    pub movement_speed_scale: f64,
    /// fp-anim r1: BlendSpacePlayer_29 / _30's smoothed sample weights, and the outgoing player of BlendListByBool_7
    /// (bIsCurrentUpperA; UPPER_AB_BLEND_TIME, HermiteCubic) after an UpperBlendSpace change: (space, weights, when)
    pub upper_player: crate::blendspace::BsPlayer,
    pub upper_prev: Option<(Rc<crate::blendspace::BlendSpace>, crate::blendspace::BsPlayer, f64)>,
    upper_seen: Option<Rc<crate::blendspace::BlendSpace>>,
    upper_last_now: Option<f64>,
    /// fp-anim r1: the alternate weapon mode this instance last read its equipment assets for (Sim::sync_alternate_mode)
    pub alt_seen: bool,
    pub alt_init: bool,
    /// fp-anim r2: a mode switch in progress (target mode, the motion's StartTime) until FinishSwitch
    pub alt_pending: Option<(bool, f64)>,
    /// Original Type0 regrip target-mode gripped transforms, independent of the already-swapped core mode.
    pub switch_hand_grips: Option<crate::procedural::SwitchHandGrips>,
    upper_fp_seen: Option<bool>,
    /// fp-anim r2: the left-hand equipment's LeftShoulderIdleOffset1P (NativeUpdateAnimation decomp 3887-3902; zero
    /// without one), ModifyBone_188's translation
    pub left_shoulder_idle_1p: FVector,
    /// EVD_CAM_010: the mesh's ComponentToWorld rotation this update (Sim::refresh_pose) and the camera's last
    /// CameraCollisionLocationOffset (world; Sim::set_camera_collision_offset)
    pub mesh_rot: FQuat,
    pub camera_collision_location_offset: FVector,
    /// EVD_SWG_003: the hit-effect IK (procedural.rs HitEffect; nodes 488 / 491 / 492)
    pub hit_effect: crate::procedural::HitEffect,
    /// EVD_MOV_003: the foot grounding (grounding.rs; UCreatureAnimInstance::UpdateGrounding)
    pub grounding: crate::grounding::Grounding,
}

/// fp-anim r2: UpperBlendSpaceABlendTime / BBlendTime when not bPerformInstantAnimSwitching (UpdateEquipmentData
/// rva=0x151c4d0 decomp 6562-6566), bound to BlendListByBool_7 BlendTime[0] / [1] (PropertyAccessLibrary copies)
pub const UPPER_AB_BLEND_TIME: f64 = 0.5;

impl FighterAnim {
    pub fn new(sk: &Skeleton, a: &AnimAssets, idle: &str) -> FighterAnim {
        let angling = a.angling.iter().filter_map(|(f, b)| sk.find(b).map(|i| (f.clone(), i))).collect();
        FighterAnim { upper_w: mask_weights(sk, &a.upper_filters), idle: idle.to_string(), angling, proc_bones: crate::procedural::ProcBones::new(sk), movement_speed_scale: 1.0, ..Default::default() }
    }

    /// the current attack's AttackInfo.HitEffectIKWeightCurve (AMordhauWeapon GetAttackInfo vcall +0x7a0, UAttackMotion
    /// OnBegin decomp 1424-1444): the weapon CDO's StrikeAttack / StabAttack (Second* in the alternate mode) by move
    /// (attack.rs attack_info_for); kick / couch / bash: none (UNCONFIRMED: their infos carry no curve in the paks read)
    fn hit_effect_curve(w: &World, fi: usize, a: &AnimAssets) -> Option<String> {
        use mordhau_core::combat::enums::mv;
        let f = &w.fighters[fi];
        let at = w.cur_m(fi)?.attack()?;
        let base = match at.mv {
            mv::STAB | mv::ALT_STAB => "StabAttack",
            mv::KICK | mv::COUCH | mv::BASH => return None,
            _ => "StrikeAttack",
        };
        let key = if f.alternate_mode { format!("Second{base}") } else { base.to_string() };
        let cdo = a.cdo(&f.weapon_path);
        let p = cdo.get(&key)?.get("HitEffectIKWeightCurve")?.get("ObjectPath")?.as_str()?;
        Some(p.rsplit_once('.').map(|x| x.0).unwrap_or(p).to_string())
    }

    /// AnimNode_Slot over `source` (AnimRig.slot_blend)
    fn slot_blend(sk: &Skeleton, a: &AnimAssets, source: Vec<FTransform>, layers: &[(String, f64, f64)]) -> Vec<FTransform> {
        let raw: Vec<f64> = layers.iter().map(|l| l.2).collect();
        let Some((mut ws, src)) = slot_weights(&raw) else { return source };
        let with_src = src > 1e-5;
        if with_src {
            ws.push(src);
        }
        let poses: Vec<Vec<FTransform>> = layers.iter().map(|l| a.sample(sk, &l.0, l.1, false)).collect();
        (0..source.len())
            .map(|i| {
                let mut xs: Vec<FTransform> = poses.iter().map(|p| p[i]).collect();
                if with_src {
                    xs.push(source[i]);
                }
                blend_transforms(&xs, &ws)
            })
            .collect()
    }

    /// the current motion's offhand fields (procedural::OffhandMotion): UAttackMotion OnBegin (decomp 1662-1667)
    /// OffhandIKChangeSpeed = 0.75 / max(BlendIn, 0.25), then the attack sequence's UMordhauAnimMetaData (decomp
    /// 7210-7230: bDisablesOffhandIK, bForcesOffhandIK, the speed override, Max / MinOffhandIKDistance); UFlinchMotion
    /// (react.rs fields), UDisarmedMotion ctor (true, 5.0, decomp 245-246); every other motion the UMordhauMotion ctor
    /// (2.5). UNCONFIRMED: the writes of the other motions (block / feint copy the attack flag, equipment switches)
    fn offhand_motion(&self, w: &World, fi: usize, a: &AnimAssets, sk: &Skeleton) -> crate::procedural::OffhandMotion {
        use mordhau_core::combat::motion::MotionKind;
        let mut m = crate::procedural::OffhandMotion::default();
        if let Some(cur) = w.cur_m(fi) {
            match &cur.k {
                MotionKind::Attack(_) => {
                    m.change_speed = 0.75 / self.ma.blend_in.max(0.25) as f32;
                    if let Some((dis, forces, over, max, min)) = a.offhand_meta(&self.ma.seq) {
                        m.disables = dis;
                        m.forces = forces;
                        if let Some(s) = over {
                            m.change_speed = s;
                        }
                        m.distance_max = max;
                        m.distance_min = min;
                    }
                }
                MotionKind::Flinch(f) => {
                    m.disables = f.b_disables_offhand_ik;
                    m.change_speed = f.offhand_ik_change_speed as f32;
                }
                MotionKind::ModeSwitch(s) => {
                    let (disables, speed, right) = crate::procedural::mode_switch_offhand(s.switch_type, s.stage);
                    m.disables = disables;
                    m.change_speed = speed;
                    m.right_hand = right;
                    // ComputeRightHandIKPosition164f9d0: only Type0, valid equipment, Stage<2.
                    if s.switch_type == 0 && s.stage < 2 {
                        if let Some(g) = self.switch_hand_grips.as_ref() {
                            if let (Some(rw), Some(rh), Some(lh)) = (sk.find("RightWeapon"), sk.find("RightHand"), sk.find("LeftHand")) {
                                if let (Some(rw_cs), Some(rh_cs), Some(lh_cs)) = (self.last_cs.get(rw), self.last_cs.get(rh), self.last_cs.get(lh)) {
                                    let parent = sk.parents[rw];
                                    if parent >= 0 {
                                        if let Some(p) = self.last_cs.get(parent as usize) {
                                            m.right_target = Some(crate::procedural::switch_hand_target(*rw_cs, *rh_cs, *lh_cs, rw_cs.then(&p.inverse()), g));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                MotionKind::Disarmed(_) => {
                    m.disables = true;
                    m.change_speed = 5.0;
                }
                _ => {}
            }
        }
        // Select globally before checking the weapon's two authored switch assets.
        // A later unrelated/null/wrong-class winner must prevent fallback to an earlier switch.
        if let Some((path, fields)) = self.ma.montages.last_desired_metadata(|p| a.montage_first_offhand_meta(p)) {
            let weapon = &w.fighters[fi].weapon_path;
            let primary = a.equipment_obj(weapon, "ModeSwitchAnimation");
            let alternate = a.equipment_obj(weapon, "SecondModeSwitchAnimation");
            if path == primary || path == alternate {
                if let Some((disables, forces, speed, maximum, minimum)) = fields {
                    m.has_switch_montage_overlay = true;
                    m.disables |= disables;
                    m.forces |= forces;
                    if let Some(speed) = speed { m.change_speed = speed; }
                    m.distance_max = maximum;
                    m.distance_min = minimum;
                }
            }
        }
        m
    }

    /// FighterAnim.update: LowerBody (reference pose; locomotion UNCONFIRMED / not ported) | DefaultSlot over the upper
    /// pose, LayeredBoneBlend_1 mesh-space from LowerBack, FullBodySlot on top
    pub fn update(&mut self, w: &World, fi: usize, now: f64, sk: &Skeleton, a: &AnimAssets) {
        let (hand, spine) = (sk.find("RightWeapon"), sk.find("Spine1"));
        let pick = |cs: &[FTransform]| (hand.map(|h| cs[h]).unwrap_or_default(), spine.map(|s| cs[s]).unwrap_or_default());
        let pose_now = if self.last_cs.is_empty() { pick(&sk.ref_pose()) } else { pick(&self.last_cs) };
        let mut poses = |seq: &str, t: f64| pick(&sk.to_component(&a.sample(sk, seq, t, false)));
        self.ma.first_person = self.first_person;
        self.ma.is_view_target = self.is_view_target;
        self.ma.view_target_debug_override = self.view_target_debug_override;
        self.ma.update(w, fi, now, a, &mut poses, pose_now);
        let spec = &w.spec;
        // LowerBody (lower.rs): the Ground state's pose, or the reference pose without it
        let base = match self.lower.as_mut() {
            Some(l) => l.pose(spec, sk, a, now),
            None => sk.ref_local.clone(),
        };
        // the Jump / Falling / Airborne / Land states (air.rs): bJumped is a one-update pulse (consumed here as
        // UAdvancedCharacterAnimInstance::NativeUpdateAnimation rva=0x148aa10 decomp 258-259 clears the character flag)
        let jumped = std::mem::take(&mut self.jumped);
        self.air.update(spec, a, now, jumped, self.airborne);
        let base = self.air.lower(spec, a, sk, base, now, self.upper_input.1, self.upper_input.0);
        // UpperbodyPose: the UpperBody machine's Idle state = BlendSpacePlayer_29 (the weapon's UpperBlendSpace) at
        // (Direction, Helper_UBVelocity); the weapon's LowerAnimation loop without one (the reference's tree-less path)
        // fp-anim r1: BlendSpacePlayer_29 is an AlwaysFollower of the Locomotion sync group the LowerBody machine leads
        // (lower.rs), so every sample plays at the group's normalized time; its weights smooth at the space's
        // TargetWeightInterpolationSpeedPerSec (blendspace.rs BsPlayer); fp-anim r2: an UpperBlendSpace change
        // crossfades through BlendListByBool_7 (node 67 above BlendSpacePlayer_29 / ApplyAdditive_8 in UpperBody Idle,
        // StateResult_64; HermiteCubic, BlendTime = UpperBlendSpaceA/BBlendTime = 0.5, UpdateEquipmentData decomp
        // 6562-6566) from the outgoing player; BlendListByBool_6 (0.3 s) is the horse branch's
        let udt = self.upper_last_now.map(|l| (now - l).max(0.0)).unwrap_or(0.0);
        self.upper_last_now = Some(now);
        let changed = match (&self.upper_seen, &self.upper_bs) {
            (Some(o), Some(n)) => !Rc::ptr_eq(o, n) && o.path != n.path,
            (None, None) => false,
            _ => true,
        };
        // fp-anim r2: bPerformInstantAnimSwitching (NativeUpdateAnimation decomp 1713-1720) is set on the frame the
        // perspective changes: UpdateEquipmentData then gives the A/B list blend time 0 (no crossfade)
        let instant = self.upper_fp_seen.is_some_and(|f| f != self.first_person);
        self.upper_fp_seen = Some(self.first_person);
        if changed {
            if let Some(o) = self.upper_seen.take() {
                if self.upper_player.started && !instant {
                    self.upper_prev = Some((o, std::mem::take(&mut self.upper_player), now));
                }
            }
            self.upper_player = Default::default();
            self.upper_seen = self.upper_bs.clone();
        }
        let norm = self.lower.as_ref().map(|l| l.sync.norm);
        let (ux, uy) = self.upper_input;
        let upper_bs_pose = |bs: &Rc<crate::blendspace::BlendSpace>, pl: &mut crate::blendspace::BsPlayer| match norm {
            Some(n) => {
                let w = pl.update(bs, ux, uy, udt);
                a.blend_space_pose_at(sk, bs, &w, n)
            }
            None => a.blend_space_pose(sk, bs, ux, uy, now),
        };
        let upper_src = match self.upper_bs.clone() {
            Some(bs) => upper_bs_pose(&bs, &mut self.upper_player),
            None if !self.idle.is_empty() => a.sample(sk, &self.idle, now, true),
            None => sk.ref_local.clone(),
        };
        let upper_src = match self.upper_prev.take() {
            Some((obs, mut opl, t0)) if now - t0 < UPPER_AB_BLEND_TIME => {
                let w_new = alpha_blend(spec, 2, ((now - t0) / UPPER_AB_BLEND_TIME).clamp(0.0, 1.0), "");
                let old = upper_bs_pose(&obs, &mut opl);
                self.upper_prev = Some((obs, opl, t0));
                (0..upper_src.len()).map(|i| blend_transforms(&[old[i], upper_src[i]], &[1.0 - w_new, w_new])).collect()
            }
            _ => upper_src,
        };
        // ApplyAdditive_3 (367) in the UpperBody Idle state: SequencePlayer_25 (UpperAdditiveA, looping, rate 1) over the
        // blend space at Alpha = AnimLOD1 (PropertyAccessLibrary copy; 0 on a dedicated server, NativeUpdateAnimation
        // decomp 500-506; a standalone client at LOD 0: 1) (fidelity-audit r6). UNCONFIRMED: the A/B switch
        // BlendListByBool_6 (0.3 s crossfade on an equipment change) is not modelled
        let mut upper_src = upper_src;
        if !self.dedicated_server && !self.upper_additive.is_empty() {
            if let Some(c) = a.clip(&self.upper_additive) {
                let len = c.sequence_length as f64;
                // MH_UPPER_ADDITIVE_PHASE=<s>: the clip's time offset. The real player's SequencePlayer_25 accumulates
                // from its anim instance's initialisation (spawn), so a recording made N s after spawning sees the loop
                // at phase N mod 17; a replay against such a recording sets the same phase (camera1p gauntlet
                // 2026-10-07: the record motion1's phase is fitted in state/proofs/upper_additive_phase.md)
                let t = if len > 0.0 { (now + upper_additive_phase()).rem_euclid(len) } else { 0.0 };
                let add = crate::additive::additive_sample(a, sk, &self.upper_additive, t);
                crate::additive::apply_additive(&mut upper_src, &add, 1.0);
            }
        }
        // the UpperBody machine: Idle (the pose above) or Airborne / Jump / Land (air.rs); DefaultSlot after the machine
        // (UNCONFIRMED order: the slot node's link was not traced)
        // fp-anim r2: ModifyBone_188 (node 47, UpperBody Idle state StateResult_64, after BlendListByBool_7):
        // LeftShoulder translation + LeftShoulderIdleOffset1P, BMM_Additive in component space, Alpha IsFirstPersonFloat
        let upper_src = if self.first_person && self.left_shoulder_idle_1p != FVector::ZERO {
            match sk.find("LeftShoulder") {
                Some(ls) => {
                    let mut cp = crate::procedural::CsPose { sk, local: upper_src };
                    cp.modify_add(ls, (0.0, 0.0, 0.0), Some(self.left_shoulder_idle_1p), false, 1.0);
                    cp.local
                }
                None => upper_src,
            }
        } else {
            upper_src
        };
        let upper_src = self.air.upper(spec, a, sk, upper_src, now);
        let def_layers = self.ma.montages.slot_layers(spec, &a.slot_default, now);
        let mut upper = Self::slot_blend(sk, a, upper_src, &def_layers);
        // fp_anim_items FP-6b: LeftTorsoBlendWeight = FInterpTo(weight, bUsesLeftTorsoBlend, dt, LeftTorsoBlendSpeed)
        // (NativeUpdateAnimation, UMordhauAnimInstance.cpp 2982-2985); LayeredBoneBlend_2 takes the "DefaultLeftTorso"
        // slot from LeftShoulder (depth 0) at that weight (fp_anim_audit 1.3). UNCONFIRMED: the slot's own source pose
        // (taken as the DefaultSlot output) and local-space blending (LayeredBoneBlend default).
        {
            let dt = self.lt_last.map(|l| now - l).unwrap_or(0.0).max(0.0);
            self.lt_last = Some(now);
            let target = if self.ma.uses_left_torso { 1.0 } else { 0.0 };
            let speed = self.ma.left_torso_speed;
            let d = target - self.left_torso_w;
            self.left_torso_w = if speed <= 0.0 || d * d < 1e-8 { target } else { self.left_torso_w + d * (dt * speed).clamp(0.0, 1.0) };
            let lt_layers = self.ma.montages.slot_layers(spec, "DefaultLeftTorso", now);
            if self.left_torso_w > 1e-5 && !lt_layers.is_empty() {
                if let Some(ls) = sk.find("LeftShoulder") {
                    let left = Self::slot_blend(sk, a, upper.clone(), &lt_layers);
                    let wgt = self.left_torso_w;
                    for i in 0..upper.len() {
                        let mut j = i as i32;
                        while j >= 0 && j as usize != ls {
                            j = sk.parents[j as usize];
                        }
                        if j >= 0 {
                            upper[i] = blend_transforms(&[upper[i], left[i]], &[1.0 - wgt, wgt]);
                        }
                    }
                }
            }
        }
        // ApplyAdditive_2: the Additive machine over the slot output at AdditiveOverrideWeight
        if self.ma.additive_alpha > 1e-5 {
            let add = self.ma.machine.evaluate(spec, a, sk, now);
            crate::additive::apply_additive(&mut upper, &add, self.ma.additive_alpha);
        }
        // EVD_CAM_021: the third-person crouch lean on the UpperbodyPose branch, nodes 56 / 48 / 49 / 55 / 54 / 51 / 50 /
        // 53 / 52 (ModifyBone_179 Spine1, _187 RightForeArm, _186 LeftForeArm, _180 LeftShoulder, _181 RightShoulder,
        // _184 LeftArm, _185 RightArm, _182 Neck, _183 head; rotation BMM_Additive, the forearms in bone space, the rest
        // in component space) at Alpha Helper_UBCrouchAlpha: f = (1 - clamp((LookUp + 30) x -0.025, 0, 1)) x
        // UnclampedFastSmoothedIsCrouching x IsNotFirstPersonFloat; Spine1 roll 40 f, forearms 20 f, shoulders -10 f,
        // arms -15 f, neck / head -20 f (UpdateBlueprintHelpers rva=0x151a2b0 decomp 4474-4500)
        if let Some(l) = self.lower.as_ref().filter(|_| !self.first_person) {
            let look = w.fighters[fi].look_up_value as f32;
            let f = (1.0 - ((look + 30.0) * -0.025).clamp(0.0, 1.0)) * l.crouch_unclamped as f32;
            if f.abs() > 0.001 {
                let mut cp = crate::procedural::CsPose { sk, local: upper };
                for (b, roll, bone_space) in [
                    ("Spine1", 40.0, false),
                    ("RightForeArm", 20.0, true),
                    ("LeftForeArm", 20.0, true),
                    ("LeftShoulder", -10.0, false),
                    ("RightShoulder", -10.0, false),
                    ("LeftArm", -15.0, false),
                    ("RightArm", -15.0, false),
                    ("Neck", -20.0, false),
                    ("head", -20.0, false),
                ] {
                    if let Some(i) = sk.find(b) {
                        cp.modify_add(i, (0.0, 0.0, roll * f), None, bone_space, 1.0);
                    }
                }
                upper = cp.local;
            }
        }
        let pose = blend_mesh_space(sk, &base, &upper, &self.upper_w);
        let tr = |stage: &str, local: &[FTransform]| {
            if let Some(i) = trace_bone(sk) {
                let q = local[i].rot;
                // the component-space rotation too (camera1p gauntlet: Spine1's yaw relative to the mesh)
                let c = sk.to_component(local)[i].rot;
                eprintln!("TRACE {now:.4} {stage} {} {} {} {} CS {} {} {} {}", q.x, q.y, q.z, q.w, c.x, c.y, c.z, c.w);
            }
        };
        tr("base", &base);
        tr("upper", &upper);
        tr("layered", &pose);
        let fb_layers = self.ma.montages.slot_layers(spec, &a.slot_full_body, now);
        // FullBodySlot also blends the Position->Foot virtual-bone tracks. Our skeleton stores real bones only;
        // carry those targets through the same slot instead of retaining the standing LowerBody targets.
        // AB graph: Slot_1 (545) precedes TwoBoneIK_7/_8 (465/464), which consume VB Position_*Foot.
        let slot_feet = self.lower.as_ref().and_then(|l| l.vb_feet).map(|feet| {
            let position = sk.find("Position").map(|i| sk.to_component(&pose)[i]).unwrap_or_default();
            let raw: Vec<f64> = fb_layers.iter().map(|l| l.2).collect();
            let Some((weights, source_weight)) = slot_weights(&raw) else { return feet };
            let mut targets = (position.inverse().apply(feet.0).scale(source_weight), position.inverse().apply(feet.1).scale(source_weight));
            for (layer, weight) in fb_layers.iter().zip(weights) {
                let cs = sk.to_component(&a.sample(sk, &layer.0, layer.1, false));
                let inv = sk.find("Position").map(|i| cs[i]).unwrap_or_default().inverse();
                if let (Some(left), Some(right)) = (sk.find("LeftFoot"), sk.find("RightFoot")) {
                    targets.0 = targets.0 + inv.apply(cs[left].loc).scale(weight);
                    targets.1 = targets.1 + inv.apply(cs[right].loc).scale(weight);
                }
            }
            // Position is itself blended by the slot; convert the virtual targets back to component space.
            let slot_pose = Self::slot_blend(sk, a, pose.clone(), &fb_layers);
            let position = sk.find("Position").map(|i| sk.to_component(&slot_pose)[i]).unwrap_or_default();
            (position.apply(targets.0), position.apply(targets.1))
        });
        let pose = Self::slot_blend(sk, a, pose, &fb_layers);
        tr("fb_slot", &pose);
        // UAnimInstance::GetCurveValue of the montage-driven curves (WeaponSlideAmount, SlideCompensationWeight): the
        // weight-blended values of the slot layers' sequences (UNCONFIRMED: the upper / lower pose curves are not added)
        let curve = |n: &str| -> f64 {
            def_layers.iter().chain(fb_layers.iter()).map(|l| a.curve_value(spec, &l.0, n, l.1).unwrap_or(0.0) * l.2).sum()
        };
        let slide = (curve("WeaponSlideAmount") as f32, curve("SlideCompensationWeight") as f32);
        // the anim instance's per-frame values, then ModifyBone_84 + TwoBoneIKOffset_3 (weapon slide) and
        // ModifyBone_56 (RightWeapon base) before AttackAngling
        let dt = if self.proc.initialised { (now - self.proc.last_now) as f32 } else { 0.0 };
        self.proc.update(now, crate::procedural::MotionFlags::of(w, fi), self.cosmetic);
        // EVD_SWG_003: the hit-effect IK's anim-instance update (UMordhauAnimInstance.cpp 4280-4364). IsViewTarget is
        // taken as the first-person flag (UNCONFIRMED: a third-person view target also qualifies)
        {
            let curve = Self::hit_effect_curve(w, fi, a);
            let active = self.first_person
                && curve.is_some()
                && w.cur_m(fi).and_then(|m| m.attack()).is_some_and(|at| at.b_has_hit_including_cosmetic_hit && now < at.release_end + 0.1);
            let path = curve.unwrap_or_default();
            let spec = &w.spec;
            self.hit_effect.update(dt, active, &|d| a.curve_at(spec, &path, d as f64).unwrap_or(0.0) as f32);
        }
        self.proc.update_turn(dt, &self.turn);
        // AAdvancedCharacter lag induction (procedural.rs LagInduction): observed every frame for the local player
        {
            use mordhau_core::combat::motion::MotionKind;
            let attack = w.cur_m(fi).and_then(|m| match &m.k {
                MotionKind::Attack(at) => Some((m.start_time, at.stage, at.lag_induction, at.ai.windup, at.windup_end, at.release_end, at.early_release, at.early_release_tf)),
                _ => None,
            });
            self.proc.observe_lag(dt, self.turn.loc, self.turn.yaw, w.fighters[fi].look_up_value as f32, self.first_person, attack);
        }
        let mut cp = crate::procedural::CsPose { sk, local: pose };
        crate::procedural::weapon_slide(&mut cp, &self.proc_bones, slide.0, slide.1);
        tr("weapon_slide", &cp.local);
        // ModifyBone_82 LeftShoulder (477) then ModifyBone_83 RightShoulder (476): the 1P shoulder offsets
        // Native3919–3944 shares post-montage disable/speed across shoulder and hand weight.
        // Resolve once; retain ordinary attack/default shoulder behavior outside this switch slice.
        let om = self.offhand_motion(w, fi, a, sk);
        let shoulder_flags = om.switch_shoulder_flags(crate::procedural::MotionFlags::of(w, fi));
        self.proc.update_shoulder_1p(dt, shoulder_flags, self.first_person);
        self.proc.shoulder_offsets_1p(&mut cp, &self.proc_bones, self.shoulder_1p);
        tr("shoulder1p", &cp.local);
        // EVD_SWG_003: CopyBone_14 (488) RightHand -> VB Global_RightHand (CS), then (550 ModifyBone_40 Position: not
        // ported) TwoBoneIK_4 (491) + ModifyBone_74 (492), the hit-effect IK, before ModifyBone_56 (517). The VB is
        // what the next NativeUpdateAnimation reads (GetSocketTransform of the last evaluated pose).
        let vb = self.proc_bones.right_hand.map(|i| cp.cs(i));
        self.hit_effect.apply(&mut cp, &self.proc_bones);
        self.hit_effect.vb = vb;
        tr("hiteffect", &cp.local);
        self.proc.right_weapon_base(&mut cp, &self.proc_bones);
        tr("weapon_base", &cp.local);
        // RotateAroundPivot_1 / ModifyBone_69 (turn in place), after ModifyBone_56 and before AttackAngling
        self.proc.turn_in_place(&mut cp, &self.proc_bones, self.dedicated_server, self.first_person);
        tr("turn", &cp.local);
        // fp-anim r1: 494 ModifyBone_72 (Spine1), 490 ModifyBone_75 (RightHand), 489 ModifyBone_76 (LeftHand): the
        // pitch / yaw springs (procedural.rs), after the turn-in-place nodes and before 493 / AttackAngling
        self.proc.update_springs(dt, w.fighters[fi].look_up_value as f32, self.dedicated_server);
        self.proc.springs(&mut cp, &self.proc_bones, self.dedicated_server);
        tr("springs", &cp.local);
        let pose = cp.local;
        // AnimGraphNode_AttackAngling with the character's current SpineSpaceAdditive
        let mut cs = sk.to_component(&pose);
        let angling_additive = spine_bake(&self.ma.spine.current(now));
        attack_angling(&mut cs, &self.angling, &angling_additive);
        let ov: Vec<(usize, FTransform)> = self.angling.iter().map(|(_, b)| (*b, cs[*b])).collect();
        let pose = with_component_overrides(sk, &pose, &ov);
        tr("angling", &pose);
        // the look-up spine bend after AttackAngling (procedural.rs spine_bend)
        let inp = crate::procedural::ProcInput {
            look_up: w.fighters[fi].look_up_value as f32,
            spine_bend_w: self.ma.montages.spine_bend_blend_weight(spec, now) as f32,
            // EVD_CAM_021: FastSmoothedIsCrouching (lower.rs crouch_step)
            crouch: self.lower.as_ref().map(|l| l.crouch_fast as f32).unwrap_or(0.0),
        };
        let mut cp = crate::procedural::CsPose { sk, local: pose };
        self.proc.spine_bend(&mut cp, &self.proc_bones, &inp);
        tr("spine_bend", &cp.local);
        // 457 ModifyBone_90 (Spine1): the strike's look counter-compensation, right after the spine bend (463)
        {
            let strike = w.fighters[fi].motion.and_then(|id| {
                let m = w.cur_m(fi)?;
                match &m.k {
                    MotionKind::Attack(at) if at.is_strike_class() => Some(crate::procedural::StrikeView {
                        key: (id.0 as u64, m.start_time),
                        mv: at.mv,
                        stage: at.stage,
                        windup_end: at.windup_end,
                        lag_induction: at.lag_induction,
                        angle_target: at.angle_target,
                    }),
                    _ => None,
                }
            });
            self.counter_comp.update(now, dt, strike.as_ref(), w.fighters[fi].look_up_value as f32, self.turn.yaw);
            self.counter_comp.apply(&mut cp, &self.proc_bones, self.offhand.weight);
            tr("counter", &cp.local);
        }
        // fp-anim r2: TwoBoneIK_7 / _8 (first person: the feet to their offset virtual bones) before the offhand IK.
        // EVD_CAM_023 (state/proofs/foot_shuffle.md): the record motion1 (rest turns in 1P) keeps the feet planted in
        // the WORLD while LowerBodyRotationOffset builds (LeftFoot world xy constant to 0.1 cm, its component-space xy
        // rotating with the Hips), so the virtual-bone targets turn with RotateAroundPivot_1 (503: the hips yawed by the
        // offset about the component origin) - the LowerBody machine's feet (CopyBone 660 / 659 + ModifyBone_31 / _32)
        // yawed the same way. Before this the targets stayed in mesh space and the legs snapped with the camera.
        if self.first_person {
            if let Some(vb) = slot_feet {
                let yaw = FQuat::from_rotator(0.0, self.proc.lower_rot_offset, 0.0);
                let vb = (yaw.rotate(vb.0), yaw.rotate(vb.1));
                let gw = self.air.ground_weight(spec, now) as f32;
                crate::procedural::first_person_feet_ik(&mut cp, vb, gw);
            }
        }
        // ModifyBone_97 / _96 (445 / 449): the 1P grounding, after the feet IK / ModifyBone_87 and before the offhand IK
        self.proc.grounding_1p(&mut cp, &self.proc_bones, self.mesh_rot);
        tr("grounding", &cp.local);
        // TwoBoneIK_2 (548): the offhand on the grip (fidelity-audit r4), its inputs from the previous frame's pose as
        // NativeUpdateAnimation reads them on the game thread
        if let Some((nw, aw)) = self.offhand_weapon.clone() {
            let wd = if w.fighters[fi].alternate_mode { aw } else { nw };
            if !self.last_cs.is_empty() {
                let prev = self.last_cs.clone();
                self.offhand.update(sk, &prev, &wd, &om, self.first_person, slide, dt);
            }
            self.offhand.apply(&mut cp);
            tr("offhand", &cp.local);
        }
        // the procedural flinch's root chain (flinch.rs): UpdateProceduralFlinch rva=0x151ea30 suppresses it during an
        // attack not in recovery (UAttackMotion Stage +0x10e9 != 2) or a UFeintedMotion
        {
            use mordhau_core::combat::motion::MotionKind;
            let suppress = match w.cur_m(fi).map(|m| &m.k) {
                Some(MotionKind::Attack(at)) => at.stage != 2,
                Some(MotionKind::Feinted(_)) => true,
                _ => false,
            };
            self.flinch.update(now, dt, suppress);
            self.flinch.apply(&mut cp);
            tr("flinch", &cp.local);
        }
        // EVD_CAM_010: the outer chain's 189-191 / 206-204 camera-collision offsets (procedural.rs camera_collision),
        // after the flinch's 194 .. 192 nodes; CameraCollisionOffset = mesh rotation^-1 * the camera's offset (decomp
        // 1482-1504)
        self.proc.camera_collision_offset = self.mesh_rot.inverse().rotate(self.camera_collision_location_offset);
        self.proc.camera_collision(&mut cp, &self.proc_bones, self.first_person && !w.fighters[fi].dead);
        // ModifyBone_207 Position (AB node 0): the lag induction, the last node before the Root
        self.proc.lag_node(&mut cp, &self.proc_bones, self.mesh_rot, self.dedicated_server);
        tr("camcol", &cp.local);
        let pose = cp.local;
        self.last_cs = sk.to_component(&pose);
        self.local = pose;
    }
}

/// replay knob: MH_UPPER_ADDITIVE_PHASE (seconds, default 0) added to the upper additive clip's time (see its use)
fn upper_additive_phase() -> f64 {
    static V: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MH_UPPER_ADDITIVE_PHASE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0))
}

/// debug: MH_TRACE_BONE=<bone name> prints that bone's local rotation after every stage of the pose evaluation
/// (stderr "TRACE <now> <stage> x y z w"; camera1p gauntlet 2026-10-07, finding which stage moves the wrist)
fn trace_bone(sk: &Skeleton) -> Option<usize> {
    static NAME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    NAME.get_or_init(|| std::env::var("MH_TRACE_BONE").ok()).as_deref().and_then(|n| sk.find(n))
}

// ---- SpineSpaceAdditive + AttackAngling (spine_additive.gd, attack_angling.gd) ------------------------------------

use mordhau_core::data::{Hml, SpineAdd};

/// the 11 FSpineSpaceAdditive fields (types/FSpineSpaceAdditive.h)
pub const SPINE_FIELDS: [&str; 11] = ["Spine1", "LeftShoulder", "LeftArm", "LeftForearm", "LeftHand", "RightShoulder", "RightArm", "RightForearm", "RightHand", "Neck", "Head"];

fn spine_get(a: &SpineAdd, f: &str) -> [f64; 3] {
    a.get(f).copied().unwrap_or([0.0; 3])
}

/// FSpineSpaceAdditive::Bake RVA 0x14ef580: change each authored additive rotation into spine space.
/// NativeUpdateAnimation calls this after interpolating/copying all 132 bytes, before the graph consumes them.
/// Keep the character's interpolation inputs unbaked; bake the animation instance's per-frame copy once.
pub fn spine_bake(a: &SpineAdd) -> SpineAdd {
    // Original forward-axis quaternion: .rdata 0x433f6e0 and immediate 0x3f7e697c, not fitted angles.
    let basis = FQuat::new(f32::from_bits(0x3de3_c16b), 0.0, 0.0, f32::from_bits(0x3f7e_697c));
    let mut out = SpineAdd::new();
    for field in SPINE_FIELDS {
        let r = spine_get(a, field);
        let q = basis.mul(FQuat::from_rotator(r[0] as f32, r[1] as f32, r[2] as f32)).mul(basis.inverse());
        let (pitch, yaw, roll) = mordhau_core::combat::geometry::quat_rotator(q);
        out.insert(field.to_string(), [pitch as f64, yaw as f64, roll as f64]);
    }
    out
}

/// FSpineSpaceAdditive::Lerp rva=0x154e140: Alpha <= 0 -> A, Alpha >= 1 -> B, else per rotator
/// A + (B - A).GetNormalized() * Alpha
pub fn spine_lerp(a: &SpineAdd, b: &SpineAdd, alpha: f64) -> SpineAdd {
    if !(0.0 < alpha) {
        return a.clone();
    }
    if !(alpha < 1.0) {
        return b.clone();
    }
    let mut d = SpineAdd::new();
    for f in SPINE_FIELDS {
        let (x, y) = (spine_get(a, f), spine_get(b, f));
        let mut r = [0.0; 3];
        for i in 0..3 {
            r[i] = x[i] + normalize_axis(y[i] - x[i]) * alpha;
        }
        d.insert(f.to_string(), r);
    }
    d
}

/// UStabMotion's AnimAngleCurve keys and AnimAngleCueAmount (FRotator)
#[derive(Clone, Debug)]
pub struct StabCue {
    pub keys: Vec<mordhau_core::data::CurveKey>,
    pub pre: String,
    pub post: String,
    pub amount: (f32, f32, f32),
}

/// UStabMotion::OnTickWindUp rva=0x1666710 (decomp UStabMotion.cpp 30-250): the WindUpAdditive with Spine1,
/// RightShoulder, RightArm, RightForearm and RightHand each replaced by FQuat::Rotator(Q(AnimAngleCueAmount) * Q(field))
pub fn stab_cue_additive(w: &SpineAdd, amount: (f32, f32, f32)) -> SpineAdd {
    let mut out = w.clone();
    for f in ["Spine1", "RightShoulder", "RightArm", "RightForearm", "RightHand"] {
        let r = spine_get(w, f);
        let q = FQuat::from_rotator(amount.0, amount.1, amount.2).mul(FQuat::from_rotator(r[0] as f32, r[1] as f32, r[2] as f32));
        let (p, y, rl) = mordhau_core::combat::geometry::quat_rotator(q);
        out.insert(f.to_string(), [p as f64, y as f64, rl as f64]);
    }
    out
}

/// UAttackMotion::UpdateSpineSpaceAdditiveTargets rva=0x16417a0: (WindUpAdditive, TargetAdditive); Riposte reads the
/// RiposteAngling pair; AngleTarget <= 0 -> Lerp(Mid, High, |a|) else Lerp(Mid, Low, a)
pub fn attack_spine_targets(d: &mordhau_core::data::AttackDef, ty: i64, angle: f64) -> (SpineAdd, SpineAdd) {
    let rip = ty == mordhau_core::combat::enums::at::RIPOSTE;
    let (w, r) = if rip { (&d.riposte_angling_windup, &d.riposte_angling_release) } else { (&d.angling_windup, &d.angling_release) };
    let pick = |h: &Hml| spine_lerp(&h.mid, if angle <= 0.0 { &h.high } else { &h.low }, angle.abs());
    (pick(w), pick(r))
}

/// UParryMotion::OnTick_Implementation rva=0x1668980: AngleAdditive (Left for AltRegular) by look-up
pub fn parry_spine_target(d: &mordhau_core::data::ParryDef, block_type: i64, look_up: f64) -> SpineAdd {
    let h = if block_type == 1 { &d.angle_additive_left } else { &d.angle_additive_right };
    let mut f = if look_up >= -5.0 { (look_up * (1.0 / 60.0) + 1.0 / 12.0).clamp(0.0, 1.0) + 1.0 } else { ((look_up + 70.0) * (1.0 / 65.0)).clamp(0.0, 1.0) };
    f *= 0.5;
    if f > 0.5 {
        spine_lerp(&h.mid, &h.high, (f - 0.5) + f - 0.5)
    } else {
        spine_lerp(&h.low, &h.mid, f + f)
    }
}

/// AMordhauCharacter's SpineSpaceAdditive blend state (From / Target / BlendStart / BlendEnd / FromTangent)
#[derive(Clone, Debug, Default)]
pub struct SpineBlend {
    pub from: SpineAdd,
    pub target: SpineAdd,
    pub start: f64,
    pub end: f64,
    pub tangent: f64,
}

impl SpineBlend {
    fn h(&self, n: f64) -> f64 {
        let (n2, n3) = (n * n, n * n * n);
        ((n3 - (n2 + n2)) + n) * self.tangent + (n2 * 3.0 - (n3 + n3))
    }
    /// AMordhauCharacter::ComputeCurrentSpineSpaceAdditive rva=0x1536db0
    pub fn current(&self, now: f64) -> SpineAdd {
        spine_lerp(&self.from, &self.target, self.h(mordhau_core::ue::normalized_time(self.start, self.end, now)))
    }
    /// AMordhauCharacter::SetSpineSpaceAdditiveTarget rva=0x156c290
    pub fn set_target(&mut self, t: SpineAdd, duration: f64, now: f64) {
        let n = mordhau_core::ue::normalized_time(self.start, self.end, now);
        let t0 = self.tangent;
        self.from = spine_lerp(&self.from, &self.target, self.h(n));
        self.target = t;
        self.start = now;
        self.end = now + duration;
        self.tangent = (t0 * 3.0 - 6.0) * n * n + (6.0 - t0 * 4.0) * n + t0;
    }
    /// a direct SpineSpaceAdditiveTarget write (UAttackMotion::OnTick release, UParryMotion::OnTick)
    pub fn write_target(&mut self, t: SpineAdd) {
        self.target = t;
    }
}

/// FAnimNode_AttackAngling node fields, their FSpineSpaceAdditive field and chain parent, in evaluation order
/// (FAnimNode_AttackAngling::EvaluateSkeletalControl_AnyThread rva=0x14627a0)
pub const ANGLING_ORDER: [(&str, &str, &str); 11] = [
    ("Spine", "Spine1", ""),
    ("LeftShoulder", "LeftShoulder", "Spine"),
    ("LeftArm", "LeftArm", "LeftShoulder"),
    ("LeftForearm", "LeftForearm", "LeftArm"),
    ("LeftHand", "LeftHand", "LeftForearm"),
    ("RightShoulder", "RightShoulder", "Spine"),
    ("RightArm", "RightArm", "RightShoulder"),
    ("RightForearm", "RightForearm", "RightArm"),
    ("RightHand", "RightHand", "RightForearm"),
    ("Neck", "Neck", "Spine"),
    ("Head", "Head", "Neck"),
];

/// FAnimNode_AttackAngling on a component-space pose: Spine's rotation pre-multiplied by Q(Spine1 additive); every
/// other bone = its old transform relative to its chain parent's OLD transform, composed with the parent's NEW one,
/// then Q(additive) * rotation. Alpha 1 (node data). `bones`: node field -> bone index.
pub fn attack_angling(cs: &mut [FTransform], bones: &[(String, usize)], add: &SpineAdd) {
    let idx = |f: &str| bones.iter().find(|b| b.0 == f).map(|b| b.1);
    let old: Vec<FTransform> = cs.to_vec();
    for (field, sf, parent) in ANGLING_ORDER {
        let Some(b) = idx(field) else { continue };
        let mut nw = old[b];
        if !parent.is_empty() {
            if let Some(p) = idx(parent) {
                let rel = old[b].then(&old[p].inverse());
                nw = rel.then(&cs[p]);
            }
        }
        let r = spine_get(add, sf);
        let q = FQuat::from_rotator(r[0] as f32, r[1] as f32, r[2] as f32);
        nw.rot = qnorm(q.mul(nw.rot));
        cs[b] = nw;
    }
}

/// AnimRig.with_component_overrides: bones in `ov` take that component transform, the others keep their local one
pub fn with_component_overrides(sk: &Skeleton, local: &[FTransform], ov: &[(usize, FTransform)]) -> Vec<FTransform> {
    let mut cs: Vec<FTransform> = Vec::with_capacity(local.len());
    let mut out = local.to_vec();
    for i in 0..local.len() {
        let p = sk.parents[i];
        let c = match ov.iter().find(|o| o.0 == i) {
            Some(o) => {
                if p >= 0 {
                    out[i] = o.1.then(&cs[p as usize].inverse());
                } else {
                    out[i] = o.1;
                }
                o.1
            }
            None => {
                if p >= 0 { local[i].then(&cs[p as usize]) } else { local[i] }
            }
        };
        cs.push(c);
    }
    out
}

#[cfg(test)]
mod attack_selection_tests {
    use super::*;
    use mordhau_core::combat::enums::{at, bt, mv, stage};
    use std::sync::Arc;

    const WEAPON: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";
    const CLIPS: &str = "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword";

    fn fixture() -> Option<(Rc<Spec>, AnimAssets)> {
        let result = (|| -> Result<_, String> {
            let matrix = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).map_err(|e| e.to_string())?;
            let vfs = Arc::new(mh_pak::Vfs::mount_default().map_err(|e| e.to_string())?);
            let assets = AnimAssets::new(vfs)?;
            let spec = crate::spec::SpecBuilder::new(&matrix, &assets.rd).build(&[WEAPON])?;
            Ok((Rc::new(spec), assets))
        })();
        match result {
            Ok(f) => Some(f),
            Err(e) => {
                assert!(std::env::var("MORDHAU_GOLDEN_REQUIRED").as_deref() != Ok("1"), "attack selection fixture: {e}");
                eprintln!("SKIP attack selection: original spec/paks unavailable: {e}");
                None
            }
        }
    }

    fn begin(spec: Rc<Spec>, assets: &AnimAssets, ty: i64, attack_move: i64, first_person: bool, alt_parry: bool) -> (World, usize, MotionId, MotionAnim) {
        let mut w = World::new(spec, 1.0 / 60.0);
        let fi = w.add_fighter("selection", WEAPON, "");
        if alt_parry {
            w.assign_net_parry(fi, bt::ALT_REGULAR);
        }
        w.assign_net_attack_motion(fi, ty, attack_move, 0.0);
        let id = w.cur(fi).expect("attack assigned");
        assert_eq!(w.m(fi, id).attack().expect("attack motion").ty, ty);
        let mut anim = MotionAnim { first_person, ..Default::default() };
        let mut poses = |_: &str, _: f64| (FTransform::IDENTITY, FTransform::IDENTITY);
        anim.begin_attack(&w, fi, id, w.now, assets, &mut poses, (FTransform::IDENTITY, FTransform::IDENTITY));
        (w, fi, id, anim)
    }

    fn release(w: &mut World, fi: usize, id: MotionId, anim: &mut MotionAnim, assets: &AnimAssets) {
        // Isolate the animation handover from collision and movement. Native OnTick gets release-stage timing.
        let motion = w.fighters[fi].motions[id.0 as usize].as_mut().unwrap();
        let attack = motion.attack_mut().unwrap();
        attack.stage = stage::RELEASE;
        w.now = attack.release_end - 0.0001;
        anim.tick(w, fi, id, w.now, &w.spec, assets);
        assert!(anim.position > 0.6, "probe must cross the native handover position");
        assert!(anim.queued_3p.is_none());
    }

    fn active_path(anim: &MotionAnim) -> Option<&str> {
        anim.inst.and_then(|id| anim.montages.ids.iter().position(|i| *i == id)).map(|i| anim.montages.insts[i].asset.path.as_str())
    }

    fn update_stab(anim: &mut MotionAnim, w: &World, fi: usize, assets: &AnimAssets) {
        assert_eq!(w.cur_m(fi).and_then(|m| m.attack()).expect("stab attack").stage, stage::WINDUP);
        let mut poses = |_: &str, _: f64| (FTransform::IDENTITY, FTransform::IDENTITY);
        anim.update(w, fi, w.now, assets, &mut poses, (FTransform::IDENTITY, FTransform::IDENTITY));
    }

    fn release_probe(w: &mut World, fi: usize, id: MotionId, hit: bool, exponent: f64, view: bool) {
        w.fighters[fi].is_view_target = view;
        let a = w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap();
        a.stage = stage::RELEASE; a.windup_end = 1.0; a.release_end = 2.0;
        a.early_release = 0.25; a.early_release_tf = 1.0; a.release_curve.clear();
        a.ai.hit_effect_speed_up_exponent = exponent; a.first_hit_release_norm = 0.2_f32 as f64;
        a.b_has_hit_including_cosmetic_hit = hit;
    }

    #[test]
    fn original_post_hit_power_gates_endpoints_and_core_montage_agree() {
        let Some((spec, assets)) = fixture() else { return };
        for (hit,e,view,expected) in [(true,3.0,true,0.9),(false,3.0,true,0.6),
            (true,0.0,true,0.6),(true,1.0,true,0.6),(true,3.0,false,0.6)] {
            for fp in [false,true] {
                let (mut w,fi,id,mut anim)=begin(spec.clone(),&assets,at::REGULAR,mv::STAB,fp,false);
                anim.is_view_target=view;
                release_probe(&mut w,fi,id,hit,e,view);
                let (r,sa)=w.attack_release_state(fi,id,1.6_f32 as f64);
                assert!((r-expected).abs()<3e-7,"native power gate: hit={hit} e={e} view={view} r={r}");
                assert_eq!(sa,1.0);
                assert_eq!(anim.attack_position(&w,fi,id,1.6_f32 as f64,&assets).0,((r as f32+1.0)*0.5)as f64);
                w.now=(1.6_f32-w.dt as f32)as f64;
                w.step_pre(&mut |_|{});
                let stored=w.m(fi,id).attack().unwrap().last_release_norm;
                assert_eq!(stored,w.attack_release_state(fi,id,w.now).0);
                assert!((stored-expected).abs()<3e-7);
            }
        }
        let (mut w,fi,id,_)=begin(spec,&assets,at::REGULAR,mv::STAB,true,false);
        release_probe(&mut w,fi,id,true,3.0,true);
        // Exact binary32 endpoint: decimal .2 and (f32(1.2)-1) are different floats.
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().first_hit_release_norm=0.25;
        assert_eq!(w.attack_release_state(fi,id,1.25).0,0.25);
        assert_eq!(w.attack_release_state(fi,id,2.0).0,1.0);
        let (_,sa)=w.attack_release_state(fi,id,1.25);
        assert_eq!(sa,1.0,"exact early endpoint takes late target-additive branch");
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().first_hit_release_norm=1.0;
        assert!((w.attack_release_state(fi,id,1.6_f32 as f64).0-0.6).abs()<1e-7);
    }

    #[test]
    fn release_curve_precedes_power_and_spine_alpha_remains_raw() {
        use mordhau_core::data::{Curve,CurveKey};
        let Some((mut spec,assets))=fixture()else{return};
        Rc::get_mut(&mut spec).unwrap().curves.insert("probe-half".into(),Curve{
            keys:vec![CurveKey{time:0.0,value:0.0,..Default::default()},CurveKey{time:1.0,value:0.5,..Default::default()}],..Default::default()});
        let(mut w,fi,id,_)=begin(spec,&assets,at::REGULAR,mv::STAB,true,false);
        release_probe(&mut w,fi,id,true,3.0,true);
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().release_curve="probe-half".into();
        // curve(.6)=.3; cubic expansion gives .8*(1-.875^3)+.2=.4640625.
        assert!((w.attack_release_state(fi,id,1.6_f32 as f64).0-0.4640625).abs()<3e-7);
        let a=w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap();
        a.release_curve.clear();a.first_hit_release_norm=0.0;
        let(r,sa)=w.attack_release_state(fi,id,1.125);
        assert!((r-0.330078125).abs()<1e-7);
        assert_eq!(sa,0.5,"early spine smoothstep uses raw fraction .5, not post-hit release norm");
    }

    #[test]
    fn native_dynamic_and_cosmetic_latches_keep_first_stored_normalized_value() {
        use mordhau_core::combat::enums::DYN_HIT;
        use mordhau_core::combat::world::{Input,Call};
        let Some((spec,assets))=fixture()else{return};
        let(mut w,fi,id,_)=begin(spec.clone(),&assets,at::REGULAR,mv::STAB,true,false);
        release_probe(&mut w,fi,id,false,3.0,false);
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().last_release_norm=0.3;
        w.now=99.0;w.assign_net_motion_dynamic_param(fi,DYN_HIT);
        assert_eq!(w.m(fi,id).attack().unwrap().first_hit_release_norm,0.3);
        assert_eq!(w.m(fi,id).attack().unwrap().first_hit_time,99.0);
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().last_release_norm=0.7;
        w.assign_net_motion_dynamic_param(fi,0);w.assign_net_motion_dynamic_param(fi,DYN_HIT);
        assert_eq!(w.m(fi,id).attack().unwrap().first_hit_release_norm,0.3);
        let(mut w,fi,id,_)=begin(spec,&assets,at::REGULAR,mv::STAB,true,false);
        release_probe(&mut w,fi,id,false,3.0,false);
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().last_release_norm=0.4;
        w.input(Input::Call(Call::SetHasHitIncludingCosmeticHit{who:"selection".into()}));w.step_pre(&mut |_|{});
        assert_eq!(w.m(fi,id).attack().unwrap().first_hit_release_norm,0.4);
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().last_release_norm=0.8;
        w.assign_net_motion_dynamic_param(fi,DYN_HIT);
        assert_eq!(w.m(fi,id).attack().unwrap().first_hit_release_norm,0.4);
    }

    #[test]
    fn original_weapon_exponents_survive_typed_spec_loading() {
        let Some((spec,assets))=fixture()else{return};
        for mv in [mv::STAB,mv::ALT_STAB] {
            let(w,fi,id,_)=begin(spec.clone(),&assets,at::REGULAR,mv,true,false);
            assert_eq!(w.m(fi,id).attack().unwrap().ai.hit_effect_speed_up_exponent,3.0);
        }
        let gs=spec.weapon(WEAPON).unwrap();
        assert_eq!(gs.second_stab.hit_effect_speed_up_exponent,3.0);
        assert_eq!(gs.strike.hit_effect_speed_up_exponent,0.0);
        assert_eq!(gs.second_strike.hit_effect_speed_up_exponent,0.0);
        let ogre="Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedBlunt/BP_2HStickOgre";
        let matrix=mh_spec::Spec::load(&mh_spec::Spec::default_dir(),false).unwrap();
        let spec=crate::spec::SpecBuilder::new(&matrix,&assets.rd).build(&[ogre]).unwrap();
        let ogre=spec.weapon(ogre).unwrap();
        assert_eq!(ogre.stab.hit_effect_speed_up_exponent,0.0);
        assert_eq!(ogre.second_stab.hit_effect_speed_up_exponent,0.0);
        assert_eq!(mordhau_core::data::AttackInfo::default().hit_effect_speed_up_exponent,0.0);
    }

    #[test]
    fn spine_bake_changes_all_authored_rotation_axes_in_original_order() {
        let authored = [13.0, -7.0, 3.0];
        let input: SpineAdd = SPINE_FIELDS.iter().map(|f| ((*f).to_string(), authored)).collect();
        let before = input.clone();
        let baked = spine_bake(&input);
        assert_eq!(input, before, "baking must not accumulate in the character's interpolation state");
        let raw = FQuat::from_rotator(authored[0] as f32, authored[1] as f32, authored[2] as f32);
        // Independent axis-change oracle: conjugating by an X quaternion rotates the vector part in Y/Z.
        let (x, w) = (f32::from_bits(0x3de3_c16b), f32::from_bits(0x3f7e_697c));
        let (cos, sin, norm) = (w * w - x * x, 2.0 * x * w, w * w + x * x);
        let expected = [norm * raw.x, cos * raw.y - sin * raw.z, sin * raw.y + cos * raw.z, norm * raw.w];
        assert_eq!(baked.len(), 11);
        for field in SPINE_FIELDS {
            let r = spine_get(&baked, field);
            let q = FQuat::from_rotator(r[0] as f32, r[1] as f32, r[2] as f32);
            for (actual, expected) in [q.x, q.y, q.z, q.w].into_iter().zip(expected) {
                assert!((actual - expected).abs() < 2e-6, "original Bake axis orientation for {field}: {actual} vs {expected}");
            }
        }
    }

    #[test]
    fn spine_bake_preserves_zero_and_forward_axis_roll() {
        let zero = spine_bake(&SpineAdd::new());
        for field in SPINE_FIELDS { assert_eq!(spine_get(&zero, field), [0.0; 3]); }
        let roll: SpineAdd = SPINE_FIELDS.iter().map(|f| ((*f).to_string(), [0.0, 0.0, 17.0])).collect();
        let baked = spine_bake(&roll);
        for field in SPINE_FIELDS {
            let r = spine_get(&baked, field);
            assert!(r[0].abs() < 1e-5 && r[1].abs() < 1e-5 && (r[2] - 17.0).abs() < 2e-4, "roll commutes with native forward-axis basis: {field} {r:?}");
        }
    }

    #[test]
    fn spine_bake_original_greatsword_stab_high_low_produce_spine_space_yaw() {
        let Some((spec, assets)) = fixture() else { return };
        let (w, fi, id, anim) = begin(spec, &assets, at::REGULAR, mv::STAB, true, false);
        let def = anim.adef.as_deref().unwrap_or(w.m(fi, id).def.attack());
        for (raw, pitch) in [(&def.angling_release.high, -12.0), (&def.angling_release.low, 8.0)] {
            let before = raw.clone();
            let baked = spine_bake(raw);
            assert_eq!(*raw, before);
            for field in &SPINE_FIELDS[..9] {
                assert_eq!(spine_get(raw, field), [pitch, 0.0, 0.0], "actual original Greatsword release field {field}");
                let r = spine_get(&baked, field);
                assert!(r[1].abs() > 1.0 && r[1].signum() == -pitch.signum(), "native axis change introduces opposite-sign yaw: {field} {r:?}");
            }
        }
    }

    fn cue_target(anim: &MotionAnim, spec: &Spec, norm: f64) -> SpineAdd {
        let c = anim.stab_cue.as_ref().expect("original stab cue");
        spine_lerp(&anim.targets.0, &stab_cue_additive(&anim.targets.0, c.amount), spec.curve_eval(&c.keys, norm, &c.pre, &c.post))
    }

    #[test]
    fn original_combo_stabs_sample_the_prepared_perspective_curve() {
        let Some((spec, assets)) = fixture() else { return };
        let curves = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/WeaponAnimationProfiles";
        for attack_move in [mv::STAB, mv::ALT_STAB] {
            for first_person in [false, true] {
                let (w, fi, id, mut anim) = begin(spec.clone(), &assets, at::COMBO, attack_move, first_person, false);
                let motion = w.m(fi, id);
                let attack = motion.attack().unwrap();
                let original = (attack.windup_curve.clone(), attack.windup_end, attack.release_end);
                let def = anim.adef.as_deref().unwrap_or(motion.def.attack());
                let path = format!("{curves}/FC_ComboWindUpCurve{}", if first_person { "1p" } else { "" });
                assert_eq!(def.combo_windup_curve, path);
                assert_eq!(attack.windup_curve, format!("{curves}/FC_ComboWindUpCurve"));
                let curve = assets.curve(&path).expect("original perspective combo curve");
                let intercept = if first_person { 0.0 } else { 0.3f32 as f64 };
                assert_eq!(curve.0.len(), 2);
                // Reader's JSON retains decimal f64 spelling; the original curve keys are binary32.
                assert_eq!((curve.0[0].time as f32, curve.0[0].value as f32), (0.0, intercept as f32));
                assert_eq!((curve.0[1].time as f32, curve.0[1].value as f32), (1.0, 1.0));
                assert_eq!(anim.offset, 0.0);
                assert!(def.enable_windup_smoothing);
                // Native GetSmoothedWindUpNormalizedTime (0x1628bf0) precedes the selected curve.
                let exponent = ((attack.windup_end - motion.start_time) / (attack.release_end - attack.windup_end))
                    .clamp(def.windup_smoothing_exponent_clamp.xf(), def.windup_smoothing_exponent_clamp.yf());
                for fraction in [0.0, 0.2, 0.5, 0.9] {
                    let now = motion.start_time + fraction * (attack.windup_end - motion.start_time);
                    let normalized = (now - motion.start_time) / (attack.windup_end - motion.start_time);
                    let expected = 0.5 * (intercept + (1.0 - intercept) * normalized.powf(exponent));
                    let (position, spine_alpha) = anim.attack_position(&w, fi, id, now, &assets);
                    assert!((position - expected).abs() < 1e-7, "original combo {attack_move} FP={first_person} fraction={fraction}: {position} vs {expected}");
                    assert_eq!(spine_alpha, -1.0);
                    anim.tick(&w, fi, id, now, &spec, &assets);
                    assert_eq!(anim.position, position, "actual montage tick must consume the perspective position");
                }
                let attack = w.m(fi, id).attack().unwrap();
                assert_eq!((&attack.windup_curve, attack.windup_end, attack.release_end), (&original.0, original.1, original.2));
                // Preparation captures perspective. A subsequent view flag alone cannot replace its curve.
                anim.first_person = !first_person;
                assert_eq!(anim.attack_position(&w, fi, id, motion.start_time, &assets).0 as f32, (0.5 * intercept) as f32);
            }
        }
    }

    #[test]
    fn original_regular_stab_position_retains_its_prepared_curve() {
        let Some((spec, assets)) = fixture() else { return };
        for attack_move in [mv::STAB, mv::ALT_STAB] {
            for first_person in [false, true] {
                let (w, fi, id, anim) = begin(spec.clone(), &assets, at::REGULAR, attack_move, first_person, false);
                let motion = w.m(fi, id);
                let attack = motion.attack().unwrap();
                assert!(attack.windup_curve.is_empty(), "shipped regular stab has no combo curve override");
                let def = anim.adef.as_deref().unwrap_or(motion.def.attack());
                for fraction in [0.0, 0.25, 0.75] {
                    let now = motion.start_time + fraction * (attack.windup_end - motion.start_time);
                    let x = mordhau_core::ue::normalized_time(motion.start_time, attack.windup_end, now);
                    let x = mordhau_core::combat::attack::smoothed_windup(def, motion.start_time, attack.windup_end, attack.release_end, anim.offset, x, spec.constants.small_number);
                    assert_eq!(anim.attack_position(&w, fi, id, now, &assets), ((0.5 - anim.offset) * x + anim.offset, -1.0));
                }
            }
        }
    }

    #[test]
    fn authored_fp_combo_keys_reach_montage_without_changing_tp_or_phase_times() {
        let Some((stock, assets)) = fixture() else { return };
        let path = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/WeaponAnimationProfiles/FC_ComboWindUpCurve1p";
        let mut changed = (*stock).clone();
        changed.curve_overrides.insert(path.into(), mordhau_core::data::Curve {
            keys: [0.0,1.0].into_iter().map(|time| mordhau_core::data::CurveKey {
                time, value: 0.4f32 as f64, interp_mode: Some("RCIM_Constant".into()), ..Default::default()
            }).collect(), pre:"RCCE_Constant".into(), post:"RCCE_Constant".into(),
        });
        let changed = Rc::new(changed);
        for first_person in [false,true] {
            let (w0,fi0,id0,a0) = begin(stock.clone(),&assets,at::COMBO,mv::STAB,first_person,false);
            let (w,fi,id,mut a) = begin(changed.clone(),&assets,at::COMBO,mv::STAB,first_person,false);
            let (m0,m) = (w0.m(fi0,id0),w.m(fi,id));
            assert_eq!((m.start_time,m.attack().unwrap().windup_end,m.attack().unwrap().release_end),
                (m0.start_time,m0.attack().unwrap().windup_end,m0.attack().unwrap().release_end));
            let now=m.start_time+0.5*(m.attack().unwrap().windup_end-m.start_time);
            let actual=a.attack_position(&w,fi,id,now,&assets);
            if first_person { assert_eq!(actual,(0.5*(0.4f32 as f64),-1.0)); }
            else { assert_eq!(actual,a0.attack_position(&w0,fi0,id0,now,&assets)); }
            a.tick(&w,fi,id,now,&w.spec,&assets);
            assert_eq!(a.position,actual.0);
        }
    }

    #[test]
    fn original_stab_cue_uses_view_target_in_both_perspectives() {
        let Some((spec, assets)) = fixture() else { return };
        for attack_move in [mv::STAB, mv::ALT_STAB] {
            for (first_person, is_view_target) in [(true, true), (false, true), (false, false)] {
                let (mut w, fi, id, selected) = begin(spec.clone(), &assets, at::REGULAR, attack_move, first_person, false);
                let mut anim = MotionAnim { first_person, is_view_target, ..Default::default() };
                update_stab(&mut anim, &w, fi, &assets);
                assert!(anim.stab_cue.is_some(), "camera ownership must not discard the original cue asset");
                // Pick an actual nonzero point of the shipped curve, rather than assuming its shape.
                let norm = (1..=100).map(|i| i as f64 / 100.0)
                    .find(|n| cue_target(&anim, &spec, *n) != anim.targets.0).expect("cue must change the original spine target");
                w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().last_windup_norm = norm;
                update_stab(&mut anim, &w, fi, &assets);
                assert_eq!(anim.stab_cue_written, !is_view_target);
                assert_eq!(anim.spine.target, if is_view_target { anim.targets.0.clone() } else { cue_target(&anim, &spec, norm) });
                assert_eq!(active_path(&anim), active_path(&selected), "view-target gating must leave asset selection alone");
                assert_eq!(anim.attack_debug()["is_view_target"], is_view_target);
                assert_eq!(anim.attack_debug()["stab_cue_written"], !is_view_target);
            }
        }
    }

    #[test]
    fn original_stab_cue_rechecks_view_target_during_one_windup() {
        let Some((spec, assets)) = fixture() else { return };
        let (mut w, fi, id, _) = begin(spec.clone(), &assets, at::REGULAR, mv::STAB, false, false);
        let mut anim = MotionAnim { is_view_target: true, ..Default::default() };
        update_stab(&mut anim, &w, fi, &assets);
        let motion_key = anim.motion;
        let norm = (1..=100).map(|i| i as f64 / 100.0)
            .find(|n| cue_target(&anim, &spec, *n) != anim.targets.0).expect("nonzero original cue");
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().last_windup_norm = norm;
        anim.is_view_target = false;
        update_stab(&mut anim, &w, fi, &assets);
        assert!(anim.stab_cue_written, "losing camera ownership enables the already loaded cue");
        assert_eq!(anim.spine.target, cue_target(&anim, &spec, norm));
        let previous_target = anim.spine.target.clone();
        anim.is_view_target = true;
        w.fighters[fi].motions[id.0 as usize].as_mut().unwrap().attack_mut().unwrap().last_windup_norm = 0.0;
        update_stab(&mut anim, &w, fi, &assets);
        assert!(!anim.stab_cue_written, "regaining camera ownership suppresses the next write");
        assert_eq!(anim.spine.target, previous_target, "native suppression does not reset a previously written target");
        anim.is_view_target = false;
        update_stab(&mut anim, &w, fi, &assets);
        assert!(anim.stab_cue_written);
        assert_eq!(anim.spine.target, cue_target(&anim, &spec, 0.0));
        assert_eq!(anim.motion, motion_key, "camera handoffs must not restart the motion");
    }

    #[test]
    fn original_stabs_and_swings_keep_the_selected_asset_through_release() {
        let Some((spec, assets)) = fixture() else { return };
        // Original CDOs: regular stabs have no FP asset; regular swings do. Riposte/clash have no FP assets.
        // Left clash uses *Riposte, right clash *AltRiposte. Asset names are intentionally not inferred from type.
        for (attack_move, name, clash) in [
            (mv::STAB, "RightStab", "RightStab_AltRiposte"),
            (mv::ALT_STAB, "LeftStab", "LeftStab_Riposte"),
            (mv::RIGHT_STRIKE, "RightStrike", "RightStrike_AltRiposte"),
            (mv::LEFT_STRIKE, "LeftStrike", "LeftStrike_Riposte"),
        ] {
            for first_person in [false, true] {
                for ty in [at::REGULAR, at::MORPH, at::COMBO, at::RIPOSTE, at::POST_CLASH] {
                    let suffix = match ty {
                        at::RIPOSTE => format!("{name}_AltRiposte"),
                        at::POST_CLASH => clash.to_string(),
                        _ => name.to_string(),
                    };
                    let third = format!("{CLIPS}/2H_Sword_{suffix}");
                    let has_fp = attack_move <= mv::LEFT_STRIKE && matches!(ty, at::REGULAR | at::MORPH | at::COMBO);
                    let first = if first_person && has_fp { format!("{third}1P") } else { third.clone() };
                    let (mut w, fi, id, mut anim) = begin(spec.clone(), &assets, ty, attack_move, first_person, false);
                    assert_eq!(anim.seq, first, "{name} type {ty} FP {first_person}");
                    assert_eq!(active_path(&anim), Some(first.as_str()));
                    assert_eq!(anim.queued_3p.as_ref().map(|q| q.path.as_str()), (first_person && has_fp).then_some(third.as_str()));
                    release(&mut w, fi, id, &mut anim, &assets);
                    assert_eq!(active_path(&anim), Some(third.as_str()), "{name} type {ty} changed to the wrong release asset");
                    assert_eq!(anim.seq, third, "metadata source must follow the active release asset");
                }
                let expected = format!("{CLIPS}/2H_Sword_{name}_Riposte");
                let (mut w, fi, id, mut anim) = begin(spec.clone(), &assets, at::RIPOSTE, attack_move, first_person, true);
                assert_eq!(anim.seq, expected, "alternate weapon parry riposte");
                assert!(anim.queued_3p.is_none());
                release(&mut w, fi, id, &mut anim, &assets);
                assert_eq!(active_path(&anim), Some(expected.as_str()));
            }
        }
    }

    #[test]
    fn selected_riposte_and_clash_raw_members_control_playback_and_queue() {
        let Some((spec, assets)) = fixture() else { return };
        let (probe, fi, id, _) = begin(spec.clone(), &assets, at::REGULAR, mv::RIGHT_STRIKE, true, false);
        let bp = probe.m(fi, id).bp.clone();
        let original = assets.cdo(&bp);
        let tp = format!("{CLIPS}/2H_Sword_RightStrike_AltRiposte");
        let fp = format!("{CLIPS}/2H_Sword_RightStrike1P");
        for (ty, key) in [(at::RIPOSTE, "RiposteAnimation"), (at::POST_CLASH, "ClashAnimation")] {
            for (has_tp, has_fp) in [(true, false), (false, true), (true, true), (false, false)] {
                // Synthetic presence combinations use genuine decodable assets. Expectations are the native raw
                // pointer contract; notably FP-only and null pairs must not fall back to regular Animation.
                let mut cdo = (*original).clone();
                cdo.insert(key.to_string(), serde_json::json!({
                    "ThirdPerson": if has_tp { serde_json::json!({"ObjectPath": tp}) } else { Value::Null },
                    "FirstPerson": if has_fp { serde_json::json!({"ObjectPath": fp}) } else { Value::Null },
                }));
                assets.cdos.borrow_mut().insert(bp.clone(), Rc::new(cdo));
                for first_person in [false, true] {
                    let expected_start = if has_fp && (first_person || !has_tp) { fp.as_str() } else if has_tp { tp.as_str() } else { "" };
                    let queued = first_person && has_fp && has_tp;
                    let expected_release = if queued { tp.as_str() } else { expected_start };
                    let (mut w, fi, id, mut anim) = begin(spec.clone(), &assets, ty, mv::RIGHT_STRIKE, first_person, false);
                    assert_eq!(anim.seq, expected_start, "{key} TP {has_tp} FP {has_fp} perspective {first_person}");
                    assert_eq!(active_path(&anim), (!expected_start.is_empty()).then_some(expected_start));
                    assert_eq!(anim.queued_3p.as_ref().map(|q| q.path.as_str()), queued.then_some(tp.as_str()));
                    release(&mut w, fi, id, &mut anim, &assets);
                    assert_eq!(active_path(&anim), (!expected_release.is_empty()).then_some(expected_release));
                    assert_eq!(anim.seq, expected_release);
                }
            }
        }
    }

    #[test]
    fn offhand_metadata_original_stabs_and_fp_strike_resolve_imports() {
        let Some((spec, assets)) = fixture() else { return };
        let (reference, _) = mh_assets::skeletal_mesh::skeleton(&assets.src, "Mordhau/Content/UMA/UMA/Master/UMA_Master_Skeleton").expect("original fixture skeleton");
        let sk = Skeleton::from_ref(&reference);
        let expected = Some((false, true, None, -20.0, 0.0));
        for (attack_move, name) in [(mv::STAB, "RightStab"), (mv::ALT_STAB, "LeftStab")] {
            for first_person in [false, true] {
                for (ty, alt_parry, suffix) in [(at::REGULAR, false, ""), (at::RIPOSTE, false, "_AltRiposte"), (at::RIPOSTE, true, "_Riposte")] {
                    let (w, fi, _, anim) = begin(spec.clone(), &assets, ty, attack_move, first_person, alt_parry);
                    let path = format!("{CLIPS}/2H_Sword_{name}{suffix}");
                    assert_eq!(anim.seq, path, "original selected pair, FP {first_person}");
                    assert_eq!(assets.offhand_meta(&path), expected, "referenced metadata for {path}");
                    assert_eq!(assets.offhand_meta(&format!("{path}.0")), expected, "explicit asset export index");
                    assert_eq!(assets.offhand_meta(&format!("{path}.1")), None, "metadata export is not an animation asset");
                    assert_eq!(assets.offhand_meta(&format!("{path}.+0")), None, "signed index must not fall back to the package asset");
                    assert_eq!(assets.offhand_meta(&format!("{path}.999999999999999999999999999999999")), None, "oversized index must not fall back to the package asset");
                    if name == "LeftStab" || !suffix.is_empty() {
                        let ex = assets.rd.read(&path).expect("original sequence package");
                        assert_eq!(ex[0]["Properties"]["MetaData"][0]["ObjectPath"], format!("{CLIPS}/2H_Sword_RightStab.1"));
                        assert!(!ex.iter().any(|e| e["Type"].as_str() == Some("MordhauAnimMetaData")), "fixture must exercise an imported reference");
                    }
                    // Exercise the real caller, not only a JSON helper. Speed capture/lifetime is deliberately
                    // outside this piece; the decoded force/distance fields must reach OffhandMotion.
                    let fighter = FighterAnim { ma: anim, ..Default::default() };
                    let offhand = fighter.offhand_motion(&w, fi, &assets, &sk);
                    assert!(!offhand.disables);
                    assert!(offhand.forces);
                    assert_eq!((offhand.distance_max, offhand.distance_min), (-20.0, 0.0));
                }
            }
        }
        let (w, fi, _, anim) = begin(spec, &assets, at::REGULAR, mv::RIGHT_STRIKE, true, false);
        assert_eq!(anim.seq, format!("{CLIPS}/2H_Sword_RightStrike1P"));
        assert_eq!(assets.offhand_meta(&anim.seq), expected);
        let ex = assets.rd.read(&anim.seq).expect("original FP strike");
        assert_eq!(ex[0]["Properties"]["MetaData"][0]["ObjectPath"], format!("{CLIPS}/2H_Sword_RightStrike.1"));
        assert!(!ex.iter().any(|e| e["Type"].as_str() == Some("MordhauAnimMetaData")));
        let fighter = FighterAnim { ma: anim, ..Default::default() };
        let offhand = fighter.offhand_motion(&w, fi, &assets, &sk);
        assert!(offhand.forces);
        assert_eq!(offhand.distance_max, -20.0);
    }

    #[test]
    fn offhand_metadata_reference_order_nulls_and_explicit_zero() {
        use serde_json::json;
        // Synthetic parser inputs isolate native pointer-array ordering, not shipped attack behavior. Export 0
        // is an unreferenced local-search trap; reference order differs from export-table order.
        let exports = [
            json!({"Type":"MordhauAnimMetaData", "Properties":{"bOverridesOffhandIKChangeSpeed":true,"OffhandIKChangeSpeedOverride":999.0}}),
            json!({"Type":"MordhauAnimMetaData", "Properties":{"bForcesOffhandIK":true,"MaxOffhandIKDistance":-20.0}}),
            json!({"Type":"AnimMetaData", "Properties":{"bOverridesOffhandIKChangeSpeed":true,"OffhandIKChangeSpeedOverride":888.0}}),
            json!({"Type":"MordhauAnimMetaData", "Properties":{"bDisablesOffhandIK":true,"bOverridesOffhandIKChangeSpeed":true,"OffhandIKChangeSpeedOverride":5.0,"MaxOffhandIKDistance":-10.0,"MinOffhandIKDistance":2.0}}),
            json!({"Type":"MordhauAnimMetaData", "Properties":{"bDisablesOffhandIK":false,"bForcesOffhandIK":false,"bOverridesOffhandIKChangeSpeed":true,"OffhandIKChangeSpeedOverride":0.0,"MaxOffhandIKDistance":0.0,"MinOffhandIKDistance":0.0}}),
            json!({"Type":"MordhauAnimMetaData"}),
        ];
        let asset = json!({"Properties":{"MetaData":[{"ObjectPath":"Imported.Package.3"},null,{"ObjectPath":"Imported.Package.2"},{"ObjectPath":"Imported.Package.1"}]}});
        let mut calls = Vec::new();
        let result = offhand_metadata_refs(&asset, |package, index| {
            calls.push((package.to_string(), index));
            (package == "Imported.Package").then(|| exports.get(index).cloned()).flatten()
        });
        assert_eq!(calls, [("Imported.Package".to_string(), 3), ("Imported.Package".to_string(), 2), ("Imported.Package".to_string(), 1)]);
        assert_eq!(result, Some((false, true, Some(5.0), -20.0, 0.0)), "later no-override metadata keeps earlier speed override");
        let decode = |indices: &[usize]| {
            let references: Vec<_> = indices.iter().map(|i| json!({"ObjectPath":format!("Imported.Package.{i}")})).collect();
            offhand_metadata_refs(&json!({"Properties":{"MetaData":references}}), |package, index| {
                (package == "Imported.Package").then(|| exports.get(index).cloned()).flatten()
            })
        };
        assert_eq!(decode(&[1, 3]), Some((true, false, Some(5.0), -10.0, 2.0)), "metadata array order wins");
        assert_eq!(decode(&[3, 4]), Some((false, false, Some(0.0), 0.0, 0.0)), "explicit zero override must not become absent");
        assert_eq!(decode(&[3, 5]), Some((false, false, Some(5.0), 0.0, 0.0)), "absent native fields default, but no speed override retains prior override");
        assert_eq!(decode(&[5]), Some((false, false, None, 0.0, 0.0)));
        assert_eq!(decode(&[2]), None, "resolved wrong class contributes no metadata");
        assert_eq!(decode(&[]), None);
        assert_eq!(offhand_metadata_refs(&json!({}), |_, _| panic!("missing array must not resolve")), None);
    }

    #[test]
    fn offhand_metadata_unresolved_or_malformed_references_never_fall_back() {
        use serde_json::json;
        let metadata = json!({"Type":"MordhauAnimMetaData", "Properties":{"bForcesOffhandIK":true}});
        for invalid in [json!({}), json!({"ObjectPath":"Package"}), json!({"ObjectPath":"Package.MetadataName"}), json!({"ObjectPath":"Package.-1"}), json!({"ObjectPath":"Package.+1"}), json!({"ObjectPath":".1"}), json!("Package.1")] {
            let asset = json!({"Properties":{"MetaData":[{"ObjectPath":"Package.1"},invalid]}});
            assert_eq!(offhand_metadata_refs(&asset, |_, _| Some(metadata.clone())), None, "malformed reference invalidates lookup rather than returning a partial prefix");
        }
        let asset = json!({"Properties":{"MetaData":[{"ObjectPath":"Package.1"},{"ObjectPath":"Package.999"}]}});
        assert_eq!(offhand_metadata_refs(&asset, |_, index| (index == 1).then(|| metadata.clone())), None, "missing exact export must not use another local export");
        assert_eq!(indexed_export_path("Package.With.Dots.7"), Some(("Package.With.Dots", 7)));
        assert_eq!(indexed_export_path("Package.1.2"), None, "Reader must not strip a second index from the target package");
        assert_eq!(indexed_export_path("Package.999999999999999999999999999999999"), None);
        let asset = json!({"Properties":{"MetaData":[{"ObjectPath":"Package.1"}]}});
        assert_eq!(offhand_metadata_refs(&asset, |_, _| Some(json!({"Type":"MordhauAnimMetaData", "Template":{"ObjectPath":"Other.1"}}))), None, "unproved template defaults remain unsupported");
        assert_eq!(offhand_metadata_refs(&asset, |_, _| Some(json!({"Type":"MordhauAnimMetaData", "Properties":{"bForcesOffhandIK":"true"}}))), None, "malformed explicit fields are not native defaults");
    }
}


#[cfg(test)]
mod switch_montage_metadata_tests {
    use super::*;
    use std::sync::Arc;
    const MESSER: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Messer";
    const GS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";
    const MT: &str = "Mordhau/Content/Mordhau/Animations/Montages";

    fn fixture() -> Option<(Rc<Spec>, AnimAssets, Skeleton)> {
        let result = (|| -> Result<_, String> {
            let matrix = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).map_err(|e| e.to_string())?;
            let vfs = Arc::new(mh_pak::Vfs::mount_default().map_err(|e| e.to_string())?);
            let assets = AnimAssets::new(vfs)?;
            let spec = crate::spec::SpecBuilder::new(&matrix, &assets.rd).build(&[MESSER, GS])?;
            let (reference, _) = mh_assets::skeletal_mesh::skeleton(&assets.src, "Mordhau/Content/UMA/UMA/Master/UMA_Master_Skeleton").map_err(|e| e.0)?;
            Ok((Rc::new(spec), assets, Skeleton::from_ref(&reference)))
        })();
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                assert!(std::env::var("MORDHAU_GOLDEN_REQUIRED").as_deref() != Ok("1"), "switch montage fixture: {e}");
                eprintln!("SKIP switch montage fixture: {e}"); None
            }
        }
    }

    #[test]
    fn native_overlay_reads_first_metadata_only_and_preserves_shadowing() {
        use serde_json::json;
        let exports = [json!({"Type":"AnimMetaData"}), json!({"Type":"MordhauAnimMetaData", "Properties":{"bDisablesOffhandIK":true,"bOverridesOffhandIKChangeSpeed":true,"OffhandIKChangeSpeedOverride":0.0}})];
        let asset = |first: Value| json!({"Properties":{"MetaData":[first,{"ObjectPath":"Pkg.1"}]}});
        assert_eq!(first_montage_offhand_metadata(&asset(Value::Null), |_, _| panic!("null first must not inspect later entry")), Some(None));
        let mut calls = Vec::new();
        let result = first_montage_offhand_metadata(&asset(json!({"ObjectPath":"Pkg.0"})), |_, i| { calls.push(i); exports.get(i).cloned() });
        assert_eq!(result, Some(None));
        assert_eq!(calls, [0]);
        let result = first_montage_offhand_metadata(&asset(json!({"ObjectPath":"Pkg.1"})), |_, i| exports.get(i).cloned());
        assert_eq!(result, Some(Some((true,false,Some(0.0),0.0,0.0))));
        assert_eq!(first_montage_offhand_metadata(&json!({"Properties":{"MetaData":[]}}), |_, _| panic!()), None);
    }

    #[test]
    fn native_desired_selection_includes_zero_weight_and_excludes_stopped_blendout() {
        let spec = Spec::default();
        let make = |path: &str, slot: &str| Rc::new(Montage {
            path:path.into(), slot:slot.into(), length:1.0, segments:vec![], blend_in:0.25,
            blend_in_option:2, blend_in_curve:String::new(), blend_out:1.0, blend_out_option:2,
            trigger:-1.0, disables_spine_bending:false,
        });
        let mut set = MontageSet::default();
        set.play(&spec, make("switch", "A"), 0.0, 1.0);
        assert_eq!(set.insts[0].weight(&spec, 0.0), 0.0);
        assert_eq!(set.last_desired_metadata(|_| Some(Some(1))), Some(("switch",Some(1))));
        set.play_extra(&spec, make("unrelated_wrong_first", "B"), 0.0, 1.0);
        assert_eq!(set.last_desired_metadata(|p| if p == "switch" {Some(Some(1))} else {Some(None)}), Some(("unrelated_wrong_first",None)));
        // Metadata-free later montage does not shadow, unlike an eligible wrong first entry.
        assert_eq!(set.last_desired_metadata(|p| if p == "switch" {Some(Some(1))} else {None}), Some(("switch",Some(1))));
        set.insts[1].stop(&spec, 0.125, 1.0, 2, "");
        assert!(set.insts[1].weight(&spec, 0.125) > 0.0);
        assert_eq!(set.last_desired_metadata(|_| Some(Some(1))), Some(("switch",Some(1))));
        set.insts[0].stop(&spec, 0.125, 1.0, 2, "");
        assert!(set.insts[0].weight(&spec, 0.125) > 0.0);
        assert_eq!(set.last_desired_metadata(|_| Some(Some(1))), None);
    }

    #[test]
    fn native_montage_metadata_resolver_keeps_export_identity_and_dynamic_empty() {
        let Some((_, assets, _)) = fixture() else { return };
        let path = format!("{MT}/MTG_1H_RH_To2H");
        let expected = Some(Some((false,false,Some(2.0),0.0,0.0)));
        assert_eq!(assets.montage_first_offhand_meta(&path), expected);
        assert_eq!(assets.montage_first_offhand_meta(&format!("{path}.0")), expected);
        for suffix in [".1", ".+0", ".-0", ".999999999999999999999999999999999"] {
            assert_eq!(assets.montage_first_offhand_meta(&format!("{path}{suffix}")), None);
        }
        let seq = "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_RightStab";
        assert!(assets.offhand_meta(seq).is_some(), "sequence metadata exists, but Wrapped does not copy it");
        assert_eq!(assets.montage_first_offhand_meta(seq), None);
        assert_eq!(assets.montage_first_offhand_meta(&format!("{MT}/MTG_2H_Polearm_ToShortGrip_Idle")), None);
    }

    #[test]
    fn original_messer_overrides_reach_hand_and_shoulder_until_desired_zero() {
        let Some((spec, assets, sk)) = fixture() else { return };
        for source_alt in [false, true] {
            let mut w = World::new(spec.clone(), 1.0/60.0);
            let fi = w.add_fighter("messer", MESSER, "");
            w.change_to_idle(fi);
            if source_alt { w.switch_mode_and_reattach(fi); }
            w.switch_mode(fi);
            let id = w.cur(fi).unwrap();
            assert_eq!(w.m(fi,id).mode_switch().unwrap().switch_type, 1);
            w.mm(fi,id).mode_switch_mut().unwrap().stage = 1; // isolate native stage1 overlay branch
            let mut fighter = FighterAnim::default();
            let key = if source_alt {"SecondModeSwitchAnimation"} else {"ModeSwitchAnimation"};
            let montage = assets.montage(&assets.equipment_obj(MESSER,key)).unwrap();
            fighter.ma.montages.play(&spec, montage, 0.0, 1.0);
            // Desired1 overlays immediately even when initial blended weight is zero.
            let om = fighter.offhand_motion(&w,fi,&assets,&sk);
            assert!(om.has_switch_montage_overlay);
            assert_eq!(om.disables, !source_alt);
            assert_eq!(om.change_speed, if source_alt {2.0} else {1.25});
            let flags = om.switch_shoulder_flags(crate::procedural::MotionFlags::of(&w,fi));
            let mut proc = crate::procedural::ProcState { shoulder_1p_weight:if source_alt {0.0} else {1.0}, ..Default::default() };
            proc.update_shoulder_1p(0.0625, flags, true);
            assert_eq!(proc.shoulder_1p_weight, if source_alt {0.125} else {0.921875});
            // Core OnLeave and even no current motion do not stop a still-desired switch montage.
            w.change_to_idle(fi);
            w.fighters[fi].motion = None;
            w.now = 0.125;
            let after_leave = fighter.offhand_motion(&w,fi,&assets,&sk);
            assert!(after_leave.has_switch_montage_overlay);
            assert_eq!(after_leave.disables, !source_alt);
            assert_eq!(after_leave.change_speed, if source_alt {2.0} else {2.5});
            fighter.ma.montages.insts[0].stop(&spec,w.now,1.0,2,"");
            assert!(fighter.ma.montages.insts[0].weight(&spec,w.now)>0.0);
            let stopped = fighter.offhand_motion(&w,fi,&assets,&sk);
            assert!(!stopped.has_switch_montage_overlay);
            assert_eq!((stopped.disables,stopped.change_speed),(false,2.5));
        }
        // Original Type0 switch montage has no metadata: retain stage1right/speed1/.15.
        let mut w = World::new(spec,1.0/60.0);
        let fi = w.add_fighter("greatsword",GS,"");
        w.change_to_idle(fi); w.switch_mode(fi);
        let id = w.cur(fi).unwrap();
        w.mm(fi,id).mode_switch_mut().unwrap().stage = 1;
        let mut fighter = FighterAnim::default();
        fighter.ma.montages.play(&w.spec,assets.montage(&assets.equipment_obj(GS,"ModeSwitchAnimation")).unwrap(),0.0,1.0);
        let om = fighter.offhand_motion(&w,fi,&assets,&sk);
        assert!(!om.has_switch_montage_overlay);
        assert_eq!((om.disables,om.right_hand,om.change_speed),(false,true,1.0/0.15_f32));
    }
}

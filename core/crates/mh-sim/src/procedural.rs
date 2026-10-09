//! The procedural part of AB_MordhauCharacterAnimation's ground pose that moves the weapon: the skeletal-control nodes
//! between FullBodySlot and the Root that the anim instance drives every frame (rust-combat r4). Node data and the
//! property bindings (PropertyAccessLibrary copy records) are read off the AnimBlueprintGeneratedClass in
//! extract/json/.../AB_MordhauCharacterAnimation.json (node index = CDO AnimGraphNode_* order, as anim_graph_data.gd).
//!
//! Evaluation order from FullBodySlot (545) toward the Root, with what is ported here:
//!   539 LocalToComponent, 475 ModifyBone_84 / 474 TwoBoneIKOffset_3 (WeaponSlideVector*: NOT ported), 477 / 476
//!   (1P shoulder offsets, alpha Helper_ShoulderOffset1PWith1PWeight: 0 in third person), CopyBone 15 / 14 / 16 (into
//!   virtual bones: no effect on real bones), 550 ModifyBone_40 (Position: NOT ported), 491 TwoBoneIK_4 + 492
//!   ModifyBone_74 (the hit-effect IK after CopyBone_14 488 -> VB Global_RightHand: `HitEffect`, EVD_SWG_003),
//!   517 ModifyBone_56 RightWeapon, translation + rotation BMM_Additive in BCS_BoneSpace, bound to
//!       Helper_RightWeaponBoneBase{Translation,Rotation} (copies 195 / 196)              -> `right_weapon_base`
//!   503 / 471 / 470 RotateAroundPivot (LowerBodyRotationOffset) and 504, 455..452 (its inverse) - turn in place: NOT
//!   ported (UNCONFIRMED: the Sim turns the whole actor), 494 / 490 / 489 springs, 493 sway (ranged draw): NOT ported,
//!   518 AttackAngling (animgraph.rs attack_angling),
//!   700 ModifyBone_18 .. 622 CopyBone_9, 463 RotateAroundPivot_5: the look-up spine bend    -> `spine_bend`
//!   (the nodes after 463 toward the Root are hips / feet / grounding / body-shape controls: NOT ported).
//!
//! Anim instance values (UMordhauAnimInstance, extract/native/decomp/UMordhauAnimInstance.cpp):
//!   NativeUpdateAnimation rva=0x1501930: LookUpValue = AAdvancedCharacter::GetLookUpValue (decomp 2113 / 2859);
//!     SpineBendBlendWeight = clamp(1 - sum of the weights of montage instances whose MetaData[0] is a
//!     UMordhauAnimMetaData with bDisablesSpineBending (+0x2c), 0, 1) (decomp 3056-3101);
//!     AtmosphericsWeight = UMordhauUtilityLibrary::FInterpToSeparate rva=0x161c9a0 (weight, target, dt, 2, 4), target
//!     = !Motion.bDisablesAtmospherics (+0x6f) and not disabled by the top montage's bDisablesAtmospherics (decomp
//!     1600-1668); RightWeaponBoneBaseTransform (decomp 2905-2981): target = the right hand equipment's
//!     RightWeaponBoneCosmeticTransform (+0xb60; identity when the motion bDisablesCosmeticWeaponTransform), rotation
//!     FMath::RInterpTo(speed 2), translation FMath::VInterpTo(speed Motion.CosmeticTransformChangeSpeed), set at once
//!     when the right hand equipment changed. Arms3PSyncWeight = FMath::FInterpTo(W, Motion.bRequires3PArmsSync, dt,
//!     3) (decomp 1637 / 1663-1665; speed from the machine code, see ProcState::update): consumers are the camera
//!     collision offset (189-191, 204-206: ProcState::camera_collision) and the 1P grounding alpha (96 / 97:
//!     ProcState::grounding_1p, RootTranslationOffset not computed yet).
//!   UpdateBlueprintHelpers rva=0x151a2b0 (decomp 5131-5244): Helper_SpineBendBlendWeightHalf / Third = W * 0.5 /
//!     0.3333; Helper_SpineBendRotationAlpha = (1 - Atmo) W 0.05 + W 0.95; Helper_SpineBendRotation.Roll =
//!     -(L 0.85 + L 0.15 (1 - FastSmoothedIsCrouching)); Helper_HipsBendRotation.Roll = Atmo * that * W * 0.05;
//!     Helper_ArmsBendRotation.Roll = (L 0.15 - L 0.15 (1 - crouch)) * -0.28 * W; Helper_RightWeaponBoneBase* = the
//!     transform's translation and FQuat::Rotator.
//! Motion flags (ctors / stage changes): UAttackMotion ctor rva=0x1612540 bDisablesAtmospherics and
//! bDisablesCosmeticWeaponTransform true, CosmeticTransformChangeSpeed 2 (decomp 3528-3530), EnterRecovery rva=0x161b7b0
//! clears bDisablesCosmeticWeaponTransform (3149); UKickMotion ctor clears it (216); UParryMotion ctor rva=0x16486b0
//! true / speed 4 (2052-2053), OnBegin rva=0x1660f70 bDisablesAtmospherics true (935), EnterParryRecovery rva=0x1650540
//! clears both and sets speed 1 (1483-1484, 1570); UFlinchMotion ctor rva=0x1646930 and UDisarmedMotion ctor
//! rva=0x1646630 bDisablesAtmospherics true; UMordhauMotion ctor speed 2 (204). UNCONFIRMED: UFlinchMotion::OnTick's
//! clearing branch (decomp 418) and the motions this Sim does not model keep the class default.
//!
//! Skeletal-control semantics: ModifyBone / CopyBone are UE 4.26 engine nodes (FAnimNode_ModifyBone /
//! FAnimNode_CopyBone, engine source, not disassembled: UNCONFIRMED as compiled): ModifyBone converts the bone's CS
//! transform into the given space (BCS_ComponentSpace: as is; BCS_BoneSpace: relative to the bone's own CS
//! transform), applies rotation (Additive: Q * R) then translation (Additive: + T) and converts back; CopyBone copies
//! the source's CS rotation onto the target. FAnimNode_SkeletalControlBase blends the result in with
//! FCSPose::LocalBlendCSBoneTransforms (Alpha < 1e-5: skip; > 1 - 1e-5: set; else the bone's transform BlendWith the
//! new one: lerp + FQuat::FastLerp + Normalize) and children keep their local transforms (they follow).
//! FAnimNode_BlendBetweenBones::EvaluateSkeletalControl_AnyThread rva=0x146a5e0 and
//! FAnimNode_RotateAroundPivot::EvaluateSkeletalControl_AnyThread rva=0x146c480 are Mordhau's own and ported from the
//! decomp. The virtual bones the chain uses as scratch (VB LeftUpLeg_LeftLeg, VB RightUpLeg_RightLeg; children of
//! LeftUpLeg / RightUpLeg) are kept as stored CS rotations: no node between their write and their reads moves their
//! parents (only LowerBack / Spine / Spine1 change), so their CS rotation is what was copied.

use crate::pose::Skeleton;
use mordhau_core::combat::geometry::quat_rotator;
use mordhau_core::ue::{FQuat, FTransform, FVector};

const ZERO_ANIMWEIGHT_THRESH: f32 = 0.00001;

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

/// FTransform::BlendWith (engine source, UNCONFIRMED as compiled): alpha <= thresh keeps `a`, >= 1 - thresh is `b`,
/// else lerped translation and FQuat::FastLerp(a, b) normalized
pub fn blend_with(a: FTransform, b: FTransform, alpha: f32) -> FTransform {
    if alpha <= ZERO_ANIMWEIGHT_THRESH {
        return a;
    }
    if alpha >= 1.0 - ZERO_ANIMWEIGHT_THRESH {
        return b;
    }
    let bias = if qdot(a.rot, b.rot) >= 0.0 { 1.0 } else { -1.0 };
    let (s, u) = (alpha, bias * (1.0 - alpha));
    let q = FQuat::new(b.rot.x * s + a.rot.x * u, b.rot.y * s + a.rot.y * u, b.rot.z * s + a.rot.z * u, b.rot.w * s + a.rot.w * u);
    let l = |x: f32, y: f32| x + (y - x) * alpha;
    FTransform::new(qnorm(q), FVector::new(l(a.loc.x, b.loc.x), l(a.loc.y, b.loc.y), l(a.loc.z, b.loc.z)))
}

/// A local pose with component-space reads and writes (FCSPose: a CS write keeps the children's local transforms)
pub struct CsPose<'a> {
    pub sk: &'a Skeleton,
    pub local: Vec<FTransform>,
}

impl<'a> CsPose<'a> {
    pub fn cs(&self, i: usize) -> FTransform {
        let p = self.sk.parents[i];
        if p < 0 { self.local[i] } else { self.local[i].then(&self.cs(p as usize)) }
    }
    /// SafeSetCSBoneTransforms / LocalBlendCSBoneTransforms for one bone at `alpha`
    pub fn blend_cs(&mut self, i: usize, new_cs: FTransform, alpha: f32) {
        if alpha < ZERO_ANIMWEIGHT_THRESH {
            return;
        }
        let p = self.sk.parents[i];
        let new_local = if p < 0 { new_cs } else { new_cs.then(&self.cs(p as usize).inverse()) };
        self.local[i] = if alpha > 1.0 - ZERO_ANIMWEIGHT_THRESH { new_local } else { blend_with(self.local[i], new_local, alpha) };
    }
    /// Original FCSPose::LocalBlendCSBoneTransforms (RVA 0x2a95d10): snapshot all old target locals,
    /// apply the full component-space target batch, derive target locals, then blend the locals once.
    /// SafeSetCSBoneTransforms (0x2a9b050) requires unique targets in parent-before-child bone order.
    /// Untargeted intermediate bones keep their local transform under the full target parent.
    pub fn blend_cs_batch(&mut self, targets: &[(usize, FTransform)], alpha: f32) {
        if targets.is_empty() || alpha < ZERO_ANIMWEIGHT_THRESH {
            return;
        }
        assert!(targets.iter().all(|(i, _)| *i < self.local.len()), "component-space target outside skeleton");
        assert!(targets.windows(2).all(|w| w[0].0 < w[1].0), "component-space targets must be unique and ordered");
        let mut full_cs: Vec<FTransform> = Vec::with_capacity(self.local.len());
        let mut target_locals = Vec::with_capacity(targets.len());
        let mut next = 0;
        for (i, old_local) in self.local.iter().enumerate() {
            let parent = self.sk.parents[i];
            let target = targets.get(next).filter(|(bone, _)| *bone == i);
            let cs = match target {
                Some((_, cs)) => *cs,
                None if parent < 0 => *old_local,
                None => old_local.then(&full_cs[parent as usize]),
            };
            if target.is_some() {
                let local = if parent < 0 { cs } else { cs.then(&full_cs[parent as usize].inverse()) };
                target_locals.push((i, local));
                next += 1;
            }
            full_cs.push(cs);
        }
        for (i, target_local) in target_locals {
            self.local[i] = if alpha >= 1.0 - ZERO_ANIMWEIGHT_THRESH {
                target_local
            } else {
                blend_with(self.local[i], target_local, alpha)
            };
        }
    }
    /// FAnimNode_ModifyBone, rotation BMM_Additive (+ optional translation BMM_Additive in the same space);
    /// `bone_space`: BCS_BoneSpace, else BCS_ComponentSpace
    pub fn modify_add(&mut self, i: usize, rot: (f32, f32, f32), trans: Option<FVector>, bone_space: bool, alpha: f32) {
        if alpha < ZERO_ANIMWEIGHT_THRESH {
            return;
        }
        let base = self.cs(i);
        let q = FQuat::from_rotator(rot.0, rot.1, rot.2);
        let mut n = if bone_space { FTransform::new(q, FVector::ZERO).then(&base) } else { FTransform::new(q.mul(base.rot), base.loc) };
        if let Some(t) = trans {
            if bone_space {
                // relative to the bone's original CS transform: rotation kept, translation + T, back to CS
                let rel = n.then(&base.inverse());
                n = FTransform::new(rel.rot, rel.loc + t).then(&base);
            } else {
                n.loc = n.loc + t;
            }
        }
        self.blend_cs(i, n, alpha);
    }
    /// FAnimNode_CopyBone (rotation only, BCS_ComponentSpace) from a CS rotation
    pub fn copy_rot(&mut self, i: usize, rot: FQuat, alpha: f32) {
        let mut n = self.cs(i);
        n.rot = rot;
        self.blend_cs(i, n, alpha);
    }
    /// FAnimNode_BlendBetweenBones::EvaluateSkeletalControl_AnyThread rva=0x146a5e0 (decomp lines 56-300): A / B = the
    /// CS rotations of BlendBoneA / B; with a reference rotation R: s = sign(R.A), k = sign((s R).B), Q = (1 - a) A +
    /// a k B; without: Q = (1 - a) sign(A.B) A + a B; normalized (rsqrtps + 2 Newton steps; |Q|^2 < 1e-8 -> identity);
    /// translation the bone's own
    pub fn blend_between(&mut self, i: usize, a_bone: usize, b_bone: usize, reference: Option<FQuat>, a: f32, alpha: f32) {
        if alpha < ZERO_ANIMWEIGHT_THRESH {
            return;
        }
        let (qa, qb) = (self.cs(a_bone).rot, self.cs(b_bone).rot);
        let q = match reference {
            Some(r) => {
                let s = if qdot(r, qa) < 0.0 { -1.0 } else { 1.0 };
                let k = if 0.0 <= qdot(FQuat::new(r.x * s, r.y * s, r.z * s, r.w * s), qb) { 1.0 } else { -1.0 };
                let u = 1.0 - a;
                FQuat::new(u * qa.x + a * qb.x * k, u * qa.y + a * qb.y * k, u * qa.z + a * qb.z * k, u * qa.w + a * qb.w * k)
            }
            None => {
                let s = (1.0 - a) * if qdot(qa, qb) < 0.0 { -1.0 } else { 1.0 };
                FQuat::new(s * qa.x + a * qb.x, s * qa.y + a * qb.y, s * qa.z + a * qb.z, s * qa.w + a * qb.w)
            }
        };
        let mut n = self.cs(i);
        n.rot = qnorm(q);
        self.blend_cs(i, n, alpha);
    }
    /// FAnimNode_RotateAroundPivot::EvaluateSkeletalControl_AnyThread rva=0x146c480 in BCS_ComponentSpace: rotation
    /// Q * R, translation Q.Rotate(T - Pivot) + Pivot
    pub fn rotate_around_pivot(&mut self, i: usize, rot: (f32, f32, f32), pivot: FVector, alpha: f32) {
        if alpha < ZERO_ANIMWEIGHT_THRESH {
            return;
        }
        let q = FQuat::from_rotator(rot.0, rot.1, rot.2);
        let c = self.cs(i);
        let n = FTransform::new(q.mul(c.rot), q.rotate(c.loc - pivot) + pivot);
        self.blend_cs(i, n, alpha);
    }
}

/// ModifyBone_84 (475) + TwoBoneIKOffset_3 (474), the weapon slide (right after FullBodySlot):
/// UMordhauAnimInstance::NativeUpdateAnimation (decomp 3104-3143): WeaponSlideAmount / SlideCompensationWeight =
/// GetCurveValue; WeaponSlideVector = Amount * (-0.94280905, -0.23570226, 0.23570226), the inverse its negation;
/// Helper_WeaponSlideVectorIsNonzero = Amount > 0 (UpdateBlueprintHelpers decomp 5260).
/// ModifyBone_84: RightWeapon translation BMM_Additive in BCS_BoneSpace by WeaponSlideVector, Alpha IsNonzero.
/// TwoBoneIKOffset_3 (FAnimNode_TwoBoneIKOffset::EvaluateSkeletalControl_AnyThread rva=0x147c830, decomp 300-560):
/// IKBone RightHand (its parent RightForeArm the joint, RightArm the root), EffectorTarget RightWeapon in
/// BCS_BoneSpace with bEffectorLocationIsOffset: effector = the hand's component location + RightWeapon's rotation
/// applied to WeaponSlideVectorInverse; JointTarget RightArm in BCS_BoneSpace at JointTargetLocation
/// (0, -92.38372, -38.27853) (node data); AnimationCore::SolveTwoBoneIK (bAllowStretching false = field 0x1d2 bit 0,
/// node data) -> the root / joint / end CS transforms, blended at Alpha SlideCompensationWeight. The end bone keeps
/// its CS rotation (bTakeRotationFromEffectorSpace / bMaintainEffectorRelRot false). OffsetVector, RotateEndBone* and
/// bUseParentZLimit are zero / off in the node data. Only 2H_Polearm_RightStrike carries these curves.
pub fn weapon_slide(pose: &mut CsPose, b: &ProcBones, amount: f32, weight: f32) {
    let (Some(rw), Some(rh)) = (b.right_weapon, b.right_hand) else { return };
    if !(0.0 < amount) {
        return;
    }
    let v = FVector::new(amount * -0.94280905, amount * -0.23570226, amount * 0.23570226);
    pose.modify_add(rw, (0.0, 0.0, 0.0), Some(v), true, 1.0);
    if !(weight > ZERO_ANIMWEIGHT_THRESH) {
        return;
    }
    let joint = pose.sk.parents[rh];
    if joint < 0 || pose.sk.parents[joint as usize] < 0 {
        return;
    }
    let (joint, root) = (joint as usize, pose.sk.parents[joint as usize] as usize);
    let w = pose.cs(rw);
    let (r0, j0, e0) = (pose.cs(root), pose.cs(joint), pose.cs(rh));
    let eff = e0.loc + w.rot.rotate(FVector::new(-v.x, -v.y, -v.z));
    let jt = FTransform::new(FQuat::IDENTITY, FVector::new(0.0, -92.38372, -38.27853)).then(&r0).loc;
    let (r1, j1, e1) = solve_two_bone_ik(r0, j0, e0, jt, eff);
    pose.blend_cs_batch(&[(root, r1), (joint, j1), (rh, e1)], weight);
}

/// FVector::GetSafeNormal (engine source, tolerance 1e-8 on the squared size)
fn safe_normal(a: FVector) -> FVector {
    let s = a.length_squared();
    if s == 1.0 {
        return a;
    }
    if s < 1e-8 {
        return FVector::ZERO;
    }
    let r = 1.0 / s.sqrt();
    FVector::new(a.x * r, a.y * r, a.z * r)
}

fn cross(a: FVector, b: FVector) -> FVector {
    FVector::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// FQuat::FindBetweenNormals (UE 4.26 UnrealMath.cpp FindBetweenHelper with NormAB 1; engine source, UNCONFIRMED as
/// compiled)
fn find_between_normals(a: FVector, b: FVector) -> FQuat {
    let w = 1.0 + a.dot(b);
    let q = if w >= 1e-6 {
        FQuat::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x, w)
    } else if a.x.abs() > a.y.abs() {
        FQuat::new(-a.z, 0.0, a.x, 0.0)
    } else {
        FQuat::new(0.0, -a.z, a.y, 0.0)
    };
    qnorm(q)
}

/// AnimationCore::SolveTwoBoneIK (UE 4.26 Runtime/AnimationCore/Private/TwoBoneIK.cpp; called by
/// FAnimNode_TwoBoneIKOffset rva=0x147c830 at decomp 506; engine source, UNCONFIRMED as compiled), without stretching:
/// limb lengths from the current CS positions, the joint placed in the plane of (effector, joint target), the root and
/// joint rotated by FindBetweenNormals of their old -> new directions, the end translated (rotation kept)
pub fn solve_two_bone_ik(root: FTransform, joint: FTransform, end: FTransform, joint_target: FVector, effector: FVector) -> (FTransform, FTransform, FTransform) {
    const KINDA_SMALL: f32 = 1e-4;
    let (rp, jp, ep) = (root.loc, joint.loc, end.loc);
    let lower = (ep - jp).length();
    let upper = (jp - rp).length();
    let delta = effector - rp;
    let mut len = delta.length();
    let max_len = lower + upper;
    let dir = if len < KINDA_SMALL {
        len = KINDA_SMALL;
        FVector::new(1.0, 0.0, 0.0)
    } else {
        safe_normal(delta)
    };
    let jt = joint_target - rp;
    let bend = if jt.length_squared() < KINDA_SMALL * KINDA_SMALL {
        FVector::new(0.0, 1.0, 0.0)
    } else {
        let n = cross(dir, jt);
        if n.length_squared() < KINDA_SMALL * KINDA_SMALL {
            // FVector::FindBestAxisVectors: Axis1 = (1,0,0) when |Z| is the largest, else (0,0,1), made perpendicular
            let (nx, ny, nz) = (dir.x.abs(), dir.y.abs(), dir.z.abs());
            let a1 = if nz > nx && nz > ny { FVector::new(1.0, 0.0, 0.0) } else { FVector::new(0.0, 0.0, 1.0) };
            let a1 = safe_normal(a1 - dir.scale(a1.dot(dir) as f64));
            cross(a1, dir)
        } else {
            safe_normal(jt - dir.scale(jt.dot(dir) as f64))
        }
    };
    let (out_end, out_joint) = if len >= max_len {
        (rp + dir.scale(max_len as f64), rp + dir.scale(upper as f64))
    } else {
        let two_ab = 2.0 * upper * len;
        let cos = if two_ab != 0.0 { (upper * upper + len * len - lower * lower) / two_ab } else { 0.0 };
        let reverse = cos < 0.0;
        let ang = cos.clamp(-1.0, 1.0).acos();
        let line = upper * ang.sin();
        let sq = upper * upper - line * line;
        let mut proj = if sq > 0.0 { sq.sqrt() } else { 0.0 };
        if reverse {
            proj = -proj;
        }
        (effector, rp + dir.scale(proj as f64) + bend.scale(line as f64))
    };
    let d_root = find_between_normals(safe_normal(jp - rp), safe_normal(out_joint - rp));
    let r1 = FTransform::new(d_root.mul(root.rot), rp);
    let d_joint = find_between_normals(safe_normal(ep - jp), safe_normal(out_end - out_joint));
    let j1 = FTransform::new(d_joint.mul(joint.rot), out_joint);
    let e1 = FTransform::new(end.rot, out_end);
    (r1, j1, e1)
}

/// Bone indices of the ported nodes (None when the skeleton lacks the bone: that node is skipped)
#[derive(Clone, Debug, Default)]
pub struct ProcBones {
    /// the Position bone (root child): ModifyBone_207, the lag-induction node
    pub position: Option<usize>,
    pub hips: Option<usize>,
    pub lower_back: Option<usize>,
    pub spine: Option<usize>,
    pub spine1: Option<usize>,
    pub left_arm: Option<usize>,
    pub right_arm: Option<usize>,
    pub right_weapon: Option<usize>,
    pub right_hand: Option<usize>,
    /// fp-anim r1: ModifyBone_76's bone
    pub left_hand: Option<usize>,
    pub left_shoulder: Option<usize>,
    pub right_shoulder: Option<usize>,
    /// EVD_CAM_010: the camera-collision / 1P grounding nodes' bones
    pub spine1_adjust: Option<usize>,
    pub lower_back_adjust: Option<usize>,
    pub left_up_leg: Option<usize>,
    pub right_up_leg: Option<usize>,
    pub neck: Option<usize>,
    pub head: Option<usize>,
}

impl ProcBones {
    pub fn new(sk: &Skeleton) -> ProcBones {
        ProcBones {
            hips: sk.find("Hips"),
            position: sk.find("Position"),
            lower_back: sk.find("LowerBack"),
            spine: sk.find("Spine"),
            spine1: sk.find("Spine1"),
            left_arm: sk.find("LeftArm"),
            right_arm: sk.find("RightArm"),
            right_weapon: sk.find("RightWeapon"),
            right_hand: sk.find("RightHand"),
            left_hand: sk.find("LeftHand"),
            left_shoulder: sk.find("LeftShoulder"),
            right_shoulder: sk.find("RightShoulder"),
            spine1_adjust: sk.find("Spine1Adjust"),
            lower_back_adjust: sk.find("LowerBackAdjust"),
            left_up_leg: sk.find("LeftUpLeg"),
            right_up_leg: sk.find("RightUpLeg"),
            neck: sk.find("Neck"),
            head: sk.find("head"),
        }
    }
}

/// The anim-instance inputs of one frame
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcInput {
    /// LookUpValue (degrees)
    pub look_up: f32,
    /// SpineBendBlendWeight (MontageSet::spine_bend_blend_weight)
    pub spine_bend_w: f32,
    /// FastSmoothedIsCrouching (EVD_CAM_021: lower.rs crouch_step; the Sim passes the LowerBody's)
    pub crouch: f32,
}

/// The turn-in-place inputs of one frame (Sim): the actor yaw (the mesh yaw minus its -90 relative yaw: deltas are
/// equal), the 2D velocity, the current motion's bWantsRightLegBending, whether this runs on a dedicated server
#[derive(Clone, Copy, Debug, Default)]
pub struct TurnInput {
    pub yaw: f32,
    /// the mesh world location this frame (the lag induction observes its per-frame delta)
    pub loc: FVector,
    pub vel2: f32,
    pub right_leg_bending: bool,
}

/// AAdvancedCharacter's look / location lag induction (user feel 2026-10-07: the body lags the camera and the
/// movement during an attack). UAttackMotion::OnBegin (decomp UAttackMotion.cpp 1985-2003, autonomous proxy only):
/// SetLookLagInductionTarget(min(LagInduction, 0.03), 2 / AttackInfo.Windup x that) and
/// SetLocationLagInductionTarget(LagInduction, LagInduction / (max(0.001, (ReleaseEnd - WindupEnd) x EarlyRelease x
/// EarlyReleaseTimeFactor) + Windup)); EnterRecovery (rva 0x161b7b0) and OnLeave (0x16323e0) call
/// ResetLagInductionTargets (0x149ee50: targets 0, change speeds 0.1). Each rendered frame (UAdvancedCharacterMovement
/// LODTick decomp 492-493, LODDeltaTime) ObserveLocationLag (0x148b180: the mesh world translation delta) and
/// ObserveLookLag (0x148b3d0: the root yaw delta wrapped to +-180 as X, the LookUpValue delta as Y) feed
/// UpdateLagInduction (0x14a9790) below; Accumulated = sum of the kept deltas / (Counterweight + 1). Counterweights
/// 0.2 (look) / 0.33333334 (location), change speeds 0.5 (AAdvancedCharacter ctor, decomp 1621-1624). Local player
/// controllers only (others: the buffers are cleared and the accumulations zeroed). The anim instance copies them as
/// AccumulatedLocationLag = -(X, Y, Z) and AccumulatedTurnLag = (0, -X_look, 0) (UAdvancedCharacterAnimInstance decomp
/// 205-216) into ModifyBone_207 (AB node 0): the Position bone, rotation + translation BMM_Additive in
/// BCS_WorldSpace, Alpha AnimLOD0. LagInduction is the attack's network lag induction (0 offline), so offline this is
/// zero, as in the record motion1.
#[derive(Clone, Debug, Default)]
pub struct LagInduction {
    /// FFloatAndVector: (DeltaTime, delta), oldest first
    pub delayed: Vec<(f32, FVector)>,
    pub current: f32,
    pub target: f32,
    pub change_speed: f32,
    pub counterweight: f32,
    pub accumulated: FVector,
}

impl LagInduction {
    pub fn new(counterweight: f32, change_speed: f32) -> Self {
        LagInduction { counterweight, change_speed, ..Default::default() }
    }
    /// Set*LagInductionTarget rva=0x14a0af0 / 0x14a0b10
    pub fn set_target(&mut self, amount: f32, change_speed: f32) {
        self.target = amount;
        self.change_speed = change_speed;
    }
    /// Reset*LagInductionTarget rva=0x149ee80 / 0x149eea0
    pub fn reset_target(&mut self) {
        self.target = 0.0;
        self.change_speed = 0.1;
    }
    /// a non-local controller: DelayedDeltas emptied, the accumulation zeroed
    pub fn clear(&mut self) {
        self.delayed.clear();
        self.accumulated = FVector::ZERO;
    }
    /// AAdvancedCharacter::UpdateLagInduction rva=0x14a9790 (decomp AAdvancedCharacter.cpp): CurrentLagInduction
    /// FInterpConstantTo its target at ChangeSpeed; push (DeltaTime, NewDelta); excess = sum(DeltaTime) -
    /// (Counterweight + 1) x Current; while excess > 1e-8 the oldest entry is scaled down by f = min(1, excess / its
    /// DeltaTime) (DeltaTime and delta x (1 - f)); an entry left with DeltaTime <= 1e-8 is removed and the drain goes
    /// on. Then (Observe*Lag) Accumulated = sum of the deltas / (Counterweight + 1).
    pub fn update(&mut self, delta: FVector, dt: f32) {
        self.current = finterp_constant_to_f(self.current, self.target, dt, self.change_speed);
        self.delayed.push((dt, delta));
        let mut excess: f32 = self.delayed.iter().map(|d| d.0).sum::<f32>() - (self.counterweight + 1.0) * self.current;
        if excess > 0.0 {
            while excess.abs() > 1e-8 && !self.delayed.is_empty() {
                let (t0, d0) = self.delayed[0];
                let f = (excess / t0).min(1.0);
                let keep = 1.0 - f;
                excess -= t0 * f;
                self.delayed[0].0 = t0 * keep;
                if self.delayed[0].0.abs() > 1e-8 {
                    self.delayed[0].1 = FVector::new(d0.x * keep, d0.y * keep, d0.z * keep);
                    break;
                }
                self.delayed.remove(0);
            }
        }
        let k = 1.0 / (self.counterweight + 1.0);
        let mut sum = FVector::ZERO;
        for (_, d) in &self.delayed {
            sum = sum + *d;
        }
        self.accumulated = FVector::new(sum.x * k, sum.y * k, sum.z * k);
    }
}

/// The per-character state the anim instance keeps between frames
#[derive(Clone, Debug)]
pub struct ProcState {
    /// LowerBodyRotationOffset.Yaw, TurnValue (last mesh yaw), ZeroingLowerBodySign, InternalScaledTimeSeconds
    pub lower_rot_offset: f32,
    pub turn_value: Option<f32>,
    pub zeroing_sign: f32,
    pub internal_time: f32,
    pub atmospherics: f32,
    /// RightWeaponBoneBaseTransform: rotation as the FRotator RInterpTo works on, translation
    pub weapon_base_rot: (f32, f32, f32),
    pub weapon_base_loc: FVector,
    pub initialised: bool,
    pub last_now: f64,
    /// UMordhauAnimInstance ShoulderOffset1PWeight and IsFirstPersonFloat (fidelity-audit r4)
    pub shoulder_1p_weight: f32,
    pub is_first_person: f32,
    /// fp-anim r1: the last update_turn's FindDeltaAngleDegrees(TurnValue, mesh yaw) (decomp 2027), SpringPitchYawValue
    /// (X pitch, Y yaw), the FFloatSpringState pair (PrevError, Velocity) of each, the last LookUpValue, HandSpringWeight
    pub turn_delta: f32,
    pub spring: (f32, f32),
    pub pitch_spring_state: (f32, f32),
    pub yaw_spring_state: (f32, f32),
    pub last_look_up: Option<f32>,
    pub hand_spring_weight: f32,
    /// fp-anim r2: HandSpringWeightTarget (UpdateEquipmentData decomp 6572 / 6604-6611): 0 without a main equipment or
    /// when its bDisableHandSpringAnimation is set, else 1 (the Sim sets it from the equipment)
    pub hand_spring_target: f32,
    /// AAdvancedCharacter look / location lag induction (LagInduction above): the attack this frame's targets were set
    /// for (its start time), the last observed mesh location and (root yaw, LookUpValue)
    pub look_lag: LagInduction,
    pub loc_lag: LagInduction,
    pub lag_attack: Option<u64>,
    pub last_observed_loc: Option<FVector>,
    pub last_observed_look: Option<(f32, f32)>,
    /// EVD_CAM_014: UMordhauAnimInstance Arms3PSyncWeight (+0x954; zero-initialised, not set by the ctor). Its graph
    /// consumers are UpdateBlueprintHelpers' (decomp 5119-5130, 5134) Helper_CameraCollisionOffsetWithNot3PArmsSync /
    /// Helper_FirstPersonZoomOffsetAndCollision = CameraCollisionOffset * (1 - W) -> Translation of 204 / 205 / 206
    /// ModifyBone (RightUpLeg / LeftUpLeg / LowerBackAdjust) and 189 / 190 / 191 (Spine1Adjust / LeftShoulder /
    /// RightShoulder), and Helper_GroundingWeightWithFirstPerson = GroundingWeight * IsFirstPersonFloat * (1 - W) ->
    /// Alpha of 449 / 445 ModifyBone_96 / _97 (RightUpLeg / LeftUpLeg). Helper_FirstPersonNotDeadWith3PArmsSync
    /// (= FirstPersonNotDead * W) has no binding in the graph. It blends no 3P arm pose (camera_collision /
    /// grounding_1p below).
    pub arms_3p_sync: f32,
    /// EVD_CAM_010: CameraCollisionOffset (+0x694): NativeUpdateAnimation rva=0x1501930 (decomp 1482-1504) =
    /// Mesh.ComponentToWorld.Rotation.Inverse() * MordhauCamera.CameraCollisionLocationOffset (the camera's last
    /// UpdateFirstPersonCamera, run from AMordhauCharacter::LateTick after the previous evaluation; the anim
    /// instance's own call at decomp 720 passes bOnlyUpdateRotation and leaves it). The Sim sets it
    /// (Sim::set_camera_collision_offset); component space.
    pub camera_collision_offset: FVector,
    /// RootTranslationOffset (UAdvancedCharacterAnimInstance; UCreatureAnimInstance::UpdateGrounding, decomp
    /// UCreatureAnimInstance.cpp 8-498, the GroundingLimbs foot traces): NOT ported, stays zero (UNCONFIRMED: the real
    /// record reads (0, 0, -7.7) .. (0, 0, -1.9) standing on the duel floor). World space (ModifyBone_96 / _97).
    pub root_translation_offset: FVector,
    /// GroundingWeight (UAdvancedCharacterAnimInstance +0x320): NOT ported (grounding); the record reads 1 on the ground
    pub grounding_weight: f32,
}

impl ProcState {
    /// Helper_CameraCollisionOffsetWithNot3PArmsSync (UpdateBlueprintHelpers rva=0x151a2b0, decomp 5119-5126)
    pub fn camera_collision_offset_with_not_3p_arms_sync(&self, camera_collision_offset: FVector) -> FVector {
        let k = 1.0 - self.arms_3p_sync;
        FVector::new(camera_collision_offset.x * k, camera_collision_offset.y * k, camera_collision_offset.z * k)
    }

    /// Helper_GroundingWeightWithFirstPerson (decomp 5134): GroundingWeight * IsFirstPersonFloat * (1 - Arms3PSyncWeight)
    pub fn grounding_weight_with_first_person(&self, grounding_weight: f32) -> f32 {
        grounding_weight * self.is_first_person * (1.0 - self.arms_3p_sync)
    }

    /// EVD_CAM_010: the first-person camera-collision nodes of the outer AnimGraph chain (after the Dismemberment node
    /// 1 and ModifyBone_7 / _6 / _5 735-737, before ModifyBone_207 Position 0; walked by ComponentPose.LinkID from the
    /// Root 727): 189 / 190 / 191 ModifyBone (Spine1Adjust, LeftShoulder, RightShoulder; graph nodes 31 / 30 / 29),
    /// then 206 / 205 / 204 (LowerBackAdjust, LeftUpLeg, RightUpLeg; nodes 9 / 10 / 11). Each: Translation BMM_Additive
    /// in BCS_ComponentSpace, rotation / scale BMM_Ignore, Alpha Helper_FirstPersonNotDead (AB CDO; PropertyAccess
    /// copies). Translation = Helper_FirstPersonZoomOffsetAndCollision (189-191) / Helper_CameraCollisionOffsetWithNot3PArmsSync
    /// (204-206), both (1 - Arms3PSyncWeight) * CameraCollisionOffset (UpdateBlueprintHelpers rva=0x151a2b0, machine
    /// code 0x14151abec-0x14151ac38 stores the same vector to +0xd78 and +0xd84). Helper_FirstPersonNotDead =
    /// bIsFirstPerson (+0x688) && !bIsDead (+0x34e) (0x14151abc9-0x14151abe4).
    pub fn camera_collision(&self, pose: &mut CsPose, b: &ProcBones, first_person_not_dead: bool) {
        let a = if first_person_not_dead { 1.0 } else { 0.0 };
        let t = self.camera_collision_offset_with_not_3p_arms_sync(self.camera_collision_offset);
        for i in [b.spine1_adjust, b.left_shoulder, b.right_shoulder, b.lower_back_adjust, b.left_up_leg, b.right_up_leg].into_iter().flatten() {
            pose.modify_add(i, (0.0, 0.0, 0.0), Some(t), false, a);
        }
    }

    /// The 1P grounding: ModifyBone_97 (graph node 445, LeftUpLeg) then ModifyBone_96 (449, RightUpLeg), after
    /// ModifyBone_87 (469) and before RotateAroundPivot_4 (468) toward the offhand TwoBoneIK_2 (548): Translation
    /// BMM_Additive in BCS_WorldSpace = RootTranslationOffset, Alpha Helper_GroundingWeightWithFirstPerson =
    /// GroundingWeight * IsFirstPersonFloat * (1 - Arms3PSyncWeight) (decomp 5134; machine code 0x14151ac77-0x14151ac8e).
    /// `mesh_rot` = the mesh's ComponentToWorld rotation (world -> component). RootTranslationOffset is not computed
    /// (UpdateGrounding not ported): zero, so this does nothing yet.
    pub fn grounding_1p(&self, pose: &mut CsPose, b: &ProcBones, mesh_rot: FQuat) {
        let a = self.grounding_weight_with_first_person(self.grounding_weight);
        let t = mesh_rot.inverse().rotate(self.root_translation_offset);
        for i in [b.left_up_leg, b.right_up_leg].into_iter().flatten() {
            pose.modify_add(i, (0.0, 0.0, 0.0), Some(t), false, a);
        }
    }
}

impl Default for ProcState {
    fn default() -> Self {
        // UMordhauAnimInstance ctor rva=0x14e6480: RightWeaponBoneBaseTransform = identity (decomp 6922-6938)
        ProcState { lower_rot_offset: 0.0, turn_value: None, zeroing_sign: 0.0, internal_time: 0.0, atmospherics: 0.0, weapon_base_rot: (0.0, 0.0, 0.0), weapon_base_loc: FVector::ZERO, initialised: false, last_now: 0.0, shoulder_1p_weight: 0.0, is_first_person: 0.0, turn_delta: 0.0, spring: (0.0, 0.0), pitch_spring_state: (0.0, 0.0), yaw_spring_state: (0.0, 0.0), last_look_up: None, hand_spring_weight: 0.0, hand_spring_target: 1.0, look_lag: LagInduction::new(0.2, 0.5), loc_lag: LagInduction::new(0.33333334, 0.5), lag_attack: None, last_observed_loc: None, last_observed_look: None, arms_3p_sync: 0.0, camera_collision_offset: FVector::ZERO, root_translation_offset: FVector::ZERO, grounding_weight: 1.0 }
    }
}

fn normalize_axis(a: f32) -> f32 {
    let mut r = a % 360.0;
    if r < 0.0 {
        r += 360.0;
    }
    if r > 180.0 {
        r -= 360.0;
    }
    r
}

/// FMath::VInterpTo (engine source, UNCONFIRMED as compiled)
pub fn vinterp_to(cur: FVector, target: FVector, dt: f32, speed: f32) -> FVector {
    if speed <= 0.0 {
        return target;
    }
    let d = target - cur;
    if d.length_squared() < 1e-4 {
        return target;
    }
    let k = (dt * speed).clamp(0.0, 1.0);
    cur + FVector::new(d.x * k, d.y * k, d.z * k)
}

/// FMath::RInterpTo (engine source, UNCONFIRMED as compiled): (Target - Current).GetNormalized() * clamp(dt speed),
/// result normalized
pub fn rinterp_to(cur: (f32, f32, f32), target: (f32, f32, f32), dt: f32, speed: f32) -> (f32, f32, f32) {
    if dt == 0.0 || cur == target {
        return cur;
    }
    if speed <= 0.0 {
        return target;
    }
    let k = (dt * speed).clamp(0.0, 1.0);
    let d = (normalize_axis(target.0 - cur.0), normalize_axis(target.1 - cur.1), normalize_axis(target.2 - cur.2));
    if d.0.abs() <= 1e-4 && d.1.abs() <= 1e-4 && d.2.abs() <= 1e-4 {
        return target;
    }
    (normalize_axis(cur.0 + d.0 * k), normalize_axis(cur.1 + d.1 * k), normalize_axis(cur.2 + d.2 * k))
}

/// The current motion's anim flags (see the module header for the sources)
#[derive(Clone, Copy, Debug)]
pub struct MotionFlags {
    /// UMordhauMotion bDisablesOffhandIK (+0x72) / OffhandIKChangeSpeed (+0x78)
    pub disables_offhand_ik: bool,
    pub offhand_ik_change_speed: f32,
    pub disables_atmospherics: bool,
    pub disables_cosmetic_weapon_transform: bool,
    pub cosmetic_change_speed: f32,
    /// UMordhauMotion bRequires3PArmsSync (+0x70): set by the UAttackMotion ctor rva=0x1612540 (decomp
    /// UAttackMotion.cpp 3527; UKickMotion's ctor keeps it), cleared by UAttackMotion::EnterRecovery rva=0x161b7b0
    /// (decomp 3148); no other native writer and no BP motion CDO overrides it. Reader check
    /// (state/live_rec/everything.json motion_vars): 1 only in strike / stab motions, 0 in Idle, Blocked, Parry, Flinch,
    /// Feinted.
    pub requires_3p_arms_sync: bool,
}

impl MotionFlags {
    pub fn of(w: &mordhau_core::combat::World, fi: usize) -> MotionFlags {
        use mordhau_core::combat::motion::MotionKind;
        // OffhandIKChangeSpeed of a motion that does not set it: the UMordhauMotion ctor's 2.5 (decomp UMordhauMotion.cpp 199)
        let idle = MotionFlags { disables_offhand_ik: false, offhand_ik_change_speed: 2.5, disables_atmospherics: false, disables_cosmetic_weapon_transform: false, cosmetic_change_speed: 2.0, requires_3p_arms_sync: false };
        let Some(m) = w.cur_m(fi) else { return idle };
        match &m.k {
            MotionKind::Attack(a) => {
                let kick = a.native == "UKickMotion";
                MotionFlags { disables_atmospherics: true, disables_cosmetic_weapon_transform: !kick && a.stage != 2, cosmetic_change_speed: 2.0, requires_3p_arms_sync: a.stage != 2, ..idle }
            }
            MotionKind::Parry(p) => {
                let rec = p.stage != 0;
                MotionFlags { disables_atmospherics: !rec, disables_cosmetic_weapon_transform: !rec, cosmetic_change_speed: if rec { 1.0 } else { 4.0 }, ..idle }
            }
            // UFlinchMotion ctor rva=0x1646930 / its OnTick (react.rs), UDisarmedMotion ctor (decomp 245-246: true, 5.0)
            MotionKind::Flinch(f) => MotionFlags { disables_atmospherics: true, disables_offhand_ik: f.b_disables_offhand_ik, offhand_ik_change_speed: f.offhand_ik_change_speed as f32, ..idle },
            MotionKind::Disarmed(_) => MotionFlags { disables_atmospherics: true, disables_offhand_ik: true, offhand_ik_change_speed: 5.0, ..idle },
            // fp-anim r3: UEquipmentModeSwitchMotion ctor bDisablesAtmospherics (decomp 1052); OnBegin bDisablesOffhandIK
            // with OffhandIKChangeSpeed 1 / Stage1Duration until FirstStageEnd (decomp 945-948, 704-712)
            MotionKind::ModeSwitch(x) => {
                let (disables, speed, _) = mode_switch_offhand(x.switch_type, x.stage);
                MotionFlags { disables_atmospherics: true, disables_offhand_ik: disables, offhand_ik_change_speed: speed, ..idle }
            },
            _ => idle,
        }
    }
}

impl ProcState {
    /// the per-frame anim instance update (NativeUpdateAnimation's parts above); `cosmetic` = the right hand
    /// equipment's RightWeaponBoneCosmeticTransform (rotation as FRotator, translation)
    pub fn update(&mut self, now: f64, flags: MotionFlags, cosmetic: ((f32, f32, f32), FVector)) {
        let dt = if self.initialised { (now - self.last_now) as f32 } else { 0.0 };
        self.last_now = now;
        let target = if flags.disables_cosmetic_weapon_transform { ((0.0, 0.0, 0.0), FVector::ZERO) } else { cosmetic };
        if !self.initialised {
            // PreviousRightHandEquipment != RightHandEquipment on the first frame: set at once
            self.weapon_base_rot = target.0;
            self.weapon_base_loc = target.1;
        } else {
            self.weapon_base_rot = rinterp_to(self.weapon_base_rot, target.0, dt, 2.0);
            self.weapon_base_loc = vinterp_to(self.weapon_base_loc, target.1, dt, flags.cosmetic_change_speed);
        }
        let at = if flags.disables_atmospherics { 0.0 } else { 1.0 };
        self.atmospherics = interp_to_separate(self.atmospherics, at, dt, 2.0, 4.0);
        // NativeUpdateAnimation rva=0x1501930 (decomp 1637 / 1663-1665): Arms3PSyncWeight = FMath::FInterpTo(W,
        // Motion.bRequires3PArmsSync, dt, 3). The decompile's speed (auVar125) is lost; the machine code passes xmm7,
        // loaded with 3.0 ([0x143fe4e10]) at 0x141503529 and on the other path at 0x1415022fe, unchanged up to the call
        // at 0x1415039ea (FInterpTo 0x1418aae60, result stored to +0x954). Reader fit of the recording: 2.9 / s.
        self.arms_3p_sync = finterp_to_f(self.arms_3p_sync, if flags.requires_3p_arms_sync { 1.0 } else { 0.0 }, dt, 3.0);
        self.initialised = true;
    }

    /// UMordhauAnimInstance::NativeUpdateAnimation rva=0x1501930, the turn-in-place offset (decomp 2029, 2113-2185):
    /// d = FindDeltaAngleDegrees(TurnValue, mesh yaw); InternalScaledTimeSeconds < 2 (decomp 499 / 2141) or
    /// bIgnoreAngularVelocityAnimation -> Offset FInterpConstantTo 0 at 225; |Velocity2D|^2 > 25 (zeroed while
    /// climbing) or bWantsRightLegBending -> the same; else a running zeroing (ZeroingLowerBodySign == sign(Offset))
    /// FInterpConstantTo 0 at 90 (otherwise the sign clears), then Offset = clamp(Offset - d, -45, 45) and at +-45 the
    /// zeroing sign = sign(Offset). TurnValue = mesh yaw afterwards (decomp 2859).
    pub fn update_turn(&mut self, dt: f32, t: &TurnInput) {
        self.internal_time += dt;
        let d = match self.turn_value {
            Some(prev) => {
                let mut x = (t.yaw - prev) % 360.0;
                if x > 180.0 {
                    x -= 360.0;
                } else if x < -180.0 {
                    x += 360.0;
                }
                x
            }
            None => 0.0,
        };
        self.turn_value = Some(t.yaw);
        self.turn_delta = d;
        let to_zero = |v: f32, speed: f32| {
            let step = speed * dt;
            if v.abs() <= step { 0.0 } else { v - step * v.signum() }
        };
        if self.internal_time < 2.0 || t.vel2 * t.vel2 > 25.0 || t.right_leg_bending {
            self.lower_rot_offset = to_zero(self.lower_rot_offset, 225.0);
            return;
        }
        let s = if self.lower_rot_offset > 0.0 { 1.0 } else if self.lower_rot_offset < 0.0 { -1.0 } else { 0.0 };
        if self.zeroing_sign != 0.0 && s == self.zeroing_sign {
            self.lower_rot_offset = to_zero(self.lower_rot_offset, 90.0);
        } else {
            self.zeroing_sign = 0.0;
        }
        let o = (self.lower_rot_offset - d).clamp(-45.0, 45.0);
        self.lower_rot_offset = o;
        if (o.abs() - 45.0).abs() <= 1e-8 {
            self.zeroing_sign = if o > 0.0 { 1.0 } else if o < 0.0 { -1.0 } else { 0.0 };
        }
    }

    /// RotateAroundPivot_1 (503): Hips rotated by LowerBodyRotationOffset about the component origin, then the inverse
    /// on the upper body: ModifyBone_69 (504) LowerBack, ModifyBone_92 / _93 (455 / 454) LeftShoulder / RightShoulder,
    /// ModifyBone_94 / _95 (453 / 452) Neck / head, all additive in component space by
    /// Helper_LowerBodyRotationOffsetInverse. Their Alpha pin is NotFirstPersonWithAtmosphericsAndAnimLOD1 =
    /// IsNotFirstPerson * AtmosphericsWeight * AnimLOD1 (UpdateBlueprintHelpers decomp 5158-5160), but each node maps it
    /// through its AlphaScaleBiasClamp (CDO: bMapRange, InRange 0..1): LowerBack OutRange 1.0..0.4, shoulders 0..0.25,
    /// neck / head 0..0.3 (state/gauntlet/camera1p/alpha_maps.tsv; FInputScaleBiasClamp::ApplyTo rva 0x2ec3e30 =
    /// GetMappedRangeValueUnclamped). So in first person (pin 0) the LowerBack inverse runs at alpha 1 and the whole
    /// upper body stays on the camera while the legs lag; in third person the torso keeps 40 % of the counter-rotation
    /// and the shoulders / head add theirs. PROVEN live 2026-10-07 (state/proofs/turn_in_place_1p.md: node 504 Alpha 0.000,
    /// ActualAlpha 1.000, LowerBack written back to its rest yaw; record motion1 LowerBack 47.8 / Spine 37.4 at offset 45).
    /// AnimLOD1 is 0 on a dedicated server, which also maps to the full inverse.
    pub fn turn_in_place(&self, pose: &mut CsPose, b: &ProcBones, dedicated_server: bool, first_person: bool) {
        let o = self.lower_rot_offset;
        if o == 0.0 {
            return;
        }
        if let Some(h) = b.hips {
            pose.rotate_around_pivot(h, (0.0, o, 0.0), FVector::ZERO, 1.0);
        }
        let a = if first_person || dedicated_server { 0.0 } else { self.atmospherics.clamp(0.0, 1.0) };
        let inv = (0.0, -o, 0.0);
        // FInputScaleBias::ApplyTo clamps the mapped value to 0..1
        let map = |out_min: f32, out_max: f32| (out_min + a * (out_max - out_min)).clamp(0.0, 1.0);
        if let Some(lb) = b.lower_back {
            pose.modify_add(lb, inv, None, false, map(1.0, 0.4));
        }
        for (bone, hi) in [(b.left_shoulder, 0.25), (b.right_shoulder, 0.25), (b.neck, 0.3), (b.head, 0.3)] {
            if let Some(i) = bone {
                pose.modify_add(i, inv, None, false, map(0.0, hi));
            }
        }
    }

    /// ModifyBone_56 (before AttackAngling) on the local pose
    pub fn right_weapon_base(&self, pose: &mut CsPose, b: &ProcBones) {
        let Some(rw) = b.right_weapon else { return };
        // Helper_RightWeaponBoneBaseRotation = FQuat::Rotator of the stored FRotator::Quaternion
        let r = self.weapon_base_rot;
        let rr = quat_rotator(FQuat::from_rotator(r.0, r.1, r.2));
        pose.modify_add(rw, rr, Some(self.weapon_base_loc), true, 1.0);
    }

    /// nodes 700 .. 463 (after AttackAngling): the look-up spine bend
    pub fn spine_bend(&self, pose: &mut CsPose, b: &ProcBones, inp: &ProcInput) {
        let (Some(hips), Some(lb), Some(sp), Some(s1)) = (b.hips, b.lower_back, b.spine, b.spine1) else { return };
        let (l, w, atmo) = (inp.look_up, inp.spine_bend_w, self.atmospherics);
        let stand = 1.0 - inp.crouch;
        let spine_roll = -(l * 0.85 + l * 0.15 * stand);
        let alpha_s1 = (1.0 - atmo) * w * 0.05 + w * 0.95;
        let arms_roll = (l * 0.15 - l * 0.15 * stand) * -0.28 * w;
        let hips_roll = atmo * spine_roll * w * 0.05;
        // 700 ModifyBone_18: Spine1 rotation additive, component space
        pose.modify_add(s1, (0.0, 0.0, spine_roll), None, false, alpha_s1);
        // 547 / 546 ModifyBone_42 / _43: RightArm / LeftArm, component space, alpha 1
        if let Some(ra) = b.right_arm {
            pose.modify_add(ra, (0.0, 0.0, arms_roll), None, false, 1.0);
        }
        if let Some(la) = b.left_arm {
            pose.modify_add(la, (0.0, 0.0, arms_roll), None, false, 1.0);
        }
        // 446 CopyBone_17: LowerBack -> VB RightUpLeg_RightLeg; 623 CopyBone_8: Spine1 -> VB LeftUpLeg_LeftLeg
        let vb_r = pose.cs(lb).rot;
        let vb_l = pose.cs(s1).rot;
        // 531 CopyBone_10 (W), 529 ModifyBone_50 roll -9 bone space (W), 448 BlendBetweenBones (Hips, LowerBack, ref
        // VB_R, 0.5, W), 532 ModifyBone_49 roll -3 (W / 2)
        pose.copy_rot(lb, vb_l, w);
        pose.modify_add(lb, (0.0, 0.0, -9.0), None, true, w);
        pose.blend_between(lb, hips, lb, Some(vb_r), 0.5, w);
        pose.modify_add(lb, (0.0, 0.0, -3.0), None, true, w * 0.5);
        // 530 CopyBone_11 (W), 621 ModifyBone_39 roll -13 (W), 447 BlendBetweenBones_1 (Hips, Spine, ref VB_R,
        // 0.66666, W), 528 ModifyBone_51 roll -19 (W * 0.3333)
        pose.copy_rot(sp, vb_l, w);
        pose.modify_add(sp, (0.0, 0.0, -13.0), None, true, w);
        pose.blend_between(sp, hips, sp, Some(vb_r), 0.66666, w);
        pose.modify_add(sp, (0.0, 0.0, -19.0), None, true, w * 0.3333);
        // 622 CopyBone_9: VB_L -> Spine1 (alpha 1)
        pose.copy_rot(s1, vb_l, 1.0);
        // 463 RotateAroundPivot_5: Hips, component space, pivot 0, Helper_HipsBendRotation
        pose.rotate_around_pivot(hips, (0.0, 0.0, hips_roll), FVector::ZERO, 1.0);
    }
}

/// fp-anim r1: UKismetMathLibrary::FloatSpringInterp (?FloatSpringInterp@UKismetMathLibrary@@SAMMMAEAUFFloatSpringState@@MMMM@Z,
/// .text 0x14317934 0 disassembled with scripts/ue_dis.py): for DeltaTime > 1e-8 and |Mass| > 1e-8, Error = Target -
/// Current, Damping = 2 sqrt(Stiffness Mass) CriticalDampingFactor, Velocity += (Damping (Error - PrevError) + Error
/// Stiffness DeltaTime) / Mass, PrevError = Error, return Current + Velocity DeltaTime; otherwise Current unchanged.
/// `state` = FFloatSpringState (PrevError, Velocity).
pub fn float_spring_interp(cur: f32, target: f32, state: &mut (f32, f32), stiffness: f32, damping: f32, dt: f32, mass: f32) -> f32 {
    if !(dt > 1e-8) || !(mass.abs() > 1e-8) {
        return cur;
    }
    let err = target - cur;
    let d = 2.0 * (stiffness * mass).sqrt() * damping;
    state.1 += (d * (err - state.0) + err * stiffness * dt) / mass;
    state.0 = err;
    cur + state.1 * dt
}

/// AB_MordhauCharacterAnimation CDO SpringPitchYawStiffness (50, 50), SpringPitchYawDamping (0.55, 0.55),
/// SpringPitchYawMass (0.9, 0.9) (extract/json AB_MordhauCharacterAnimation.json Default__)
pub const SPRING_STIFFNESS: f32 = 50.0;
pub const SPRING_DAMPING: f32 = 0.55;
pub const SPRING_MASS: f32 = 0.9;

impl ProcState {
    /// fp-anim r1: NativeUpdateAnimation rva=0x1501930's pitch / yaw springs (decomp 2113-2139, 2219-2246; not
    /// bIgnoreAngularVelocityAnimation, |dt| > 1e-8): yaw target = clamp(FindDeltaAngleDegrees(TurnValue, mesh yaw) /
    /// dt, -720, 720) / 720, pitch target = clamp(FindDeltaAngleDegrees(LookUpValue, GetLookUpValue) / dt, -360, 360) /
    /// 360, each through FloatSpringInterp with DeltaTime min(dt, 0.1); HandSpringWeight = FInterpConstantTo(weight,
    /// target, dt, 5) (decomp 1869-1871), target 1 with a main equipment whose bDisableHandSpringAnimation is false
    /// (UpdateEquipmentData decomp 6604-6611; every melee weapon here). Call after update_turn.
    /// the per-frame lag observation (ObserveLocationLag / ObserveLookLag, see LagInduction) for the local player;
    /// `attack` = the current attack's (start time, stage, LagInduction, AttackInfo.Windup, WindupEnd, ReleaseEnd,
    /// EarlyRelease, EarlyReleaseTimeFactor) when one is running; its targets are set once per attack (OnBegin) and
    /// reset when it leaves the windup / release (EnterRecovery, OnLeave)
    pub fn observe_lag(&mut self, dt: f32, loc: FVector, root_yaw: f32, look_up: f32, local_player: bool, attack: Option<(f64, i64, f64, f64, f64, f64, f64, f64)>) {
        if !local_player {
            self.look_lag.clear();
            self.loc_lag.clear();
            self.lag_attack = None;
            return;
        }
        match attack {
            Some((start, stage, li, windup, we, re, er, ertf)) if stage != 2 => {
                let key = start.to_bits();
                if self.lag_attack != Some(key) {
                    self.lag_attack = Some(key);
                    let look = (li as f32).min(0.03);
                    self.look_lag.set_target(look, (2.0 / windup as f32) * look);
                    let t = ((re - we) * er * ertf).max(0.001);
                    self.loc_lag.set_target(li as f32, li as f32 / (t + windup) as f32);
                }
            }
            _ => {
                if self.lag_attack.is_some() {
                    self.lag_attack = None;
                    self.look_lag.reset_target();
                    self.loc_lag.reset_target();
                }
            }
        }
        if dt.abs() <= 1e-8 {
            return;
        }
        let dloc = match self.last_observed_loc {
            Some(l) => loc - l,
            None => FVector::ZERO,
        };
        self.last_observed_loc = Some(loc);
        let (dyaw, dlook) = match self.last_observed_look {
            Some((y, l)) => {
                let mut d = root_yaw - y;
                if d > 180.0 {
                    d -= 360.0;
                } else if d < -180.0 {
                    d += 360.0;
                }
                (d, look_up - l)
            }
            None => (0.0, 0.0),
        };
        self.last_observed_look = Some((root_yaw, look_up));
        self.loc_lag.update(dloc, dt);
        self.look_lag.update(FVector::new(dyaw, dlook, 0.0), dt);
    }

    /// ModifyBone_207 (AB node 0, the last node before the Root): the Position bone, rotation BMM_Additive in
    /// BCS_WorldSpace by AccumulatedTurnLag = (0, -AccumulatedLookLag.X, 0) and translation BMM_Additive in
    /// BCS_WorldSpace by -AccumulatedLocationLag, Alpha AnimLOD0 (0 on a dedicated server). The mesh rotation is
    /// yaw-only, so a world yaw is a component-space yaw; the world translation is turned into component space.
    pub fn lag_node(&self, pose: &mut CsPose, b: &ProcBones, mesh_rot: FQuat, dedicated_server: bool) {
        let Some(p) = b.position else { return };
        if dedicated_server {
            return;
        }
        let yaw = -self.look_lag.accumulated.x;
        let l = self.loc_lag.accumulated;
        let t = mesh_rot.inverse().rotate(FVector::new(-l.x, -l.y, -l.z));
        if yaw.abs() < 1e-6 && t.x.abs() < 1e-6 && t.y.abs() < 1e-6 && t.z.abs() < 1e-6 {
            return;
        }
        pose.modify_add(p, (0.0, yaw, 0.0), Some(t), false, 1.0);
    }

    pub fn update_springs(&mut self, dt: f32, look_up: f32, dedicated_server: bool) {
        let _ = dedicated_server;
        let target_hand = self.hand_spring_target;
        let d = target_hand - self.hand_spring_weight;
        self.hand_spring_weight = if d * d < 1e-8 { target_hand } else { self.hand_spring_weight + d.clamp(-5.0 * dt, 5.0 * dt) };
        if !(dt.abs() > 1e-8) {
            self.last_look_up = Some(look_up);
            return;
        }
        let sdt = dt.min(0.1);
        let yaw_t = (self.turn_delta / dt).clamp(-720.0, 720.0) * 0.001_388_888_9;
        self.spring.1 = float_spring_interp(self.spring.1, yaw_t, &mut self.yaw_spring_state, SPRING_STIFFNESS, SPRING_DAMPING, sdt, SPRING_MASS);
        let prev = self.last_look_up.unwrap_or(look_up);
        let mut dl = (look_up - prev) % 360.0;
        if dl > 180.0 {
            dl -= 360.0;
        } else if dl < -180.0 {
            dl += 360.0;
        }
        let pitch_t = (dl / dt).clamp(-360.0, 360.0) * 0.002_777_777_8;
        self.spring.0 = float_spring_interp(self.spring.0, pitch_t, &mut self.pitch_spring_state, SPRING_STIFFNESS, SPRING_DAMPING, sdt, SPRING_MASS);
        self.last_look_up = Some(look_up);
    }

    /// fp-anim r1: 494 ModifyBone_72 Spine1, then 490 ModifyBone_75 RightHand and 489 ModifyBone_76 LeftHand: rotation
    /// BMM_Additive in BCS_ComponentSpace by Helper_SpringPitchYawValueRotator = (Pitch Y*5, Yaw Y*5, Roll -X*5)
    /// (UpdateBlueprintHelpers rva=0x151a2b0, disasm 0x14151b09d..0x14151b0cb), Alpha AtmosphericsWeightWithAnimLOD0 =
    /// AtmosphericsWeight * AnimLOD0 for Spine1 and Helper_HandSpringWeight = that * HandSpringWeight for the hands
    /// (decomp 5225-5227). Not gated by the perspective: it runs in first person. AnimLOD0 is 0 on a dedicated server.
    pub fn springs(&self, pose: &mut CsPose, b: &ProcBones, dedicated_server: bool) {
        let lod0 = if dedicated_server { 0.0 } else { 1.0 };
        let a = self.atmospherics * lod0;
        if a <= 0.0 {
            return;
        }
        let r = (self.spring.1 * 5.0, self.spring.1 * 5.0, self.spring.0 * -5.0);
        if let Some(s1) = b.spine1 {
            pose.modify_add(s1, r, None, false, a);
        }
        let ah = a * self.hand_spring_weight;
        if ah > 0.0 {
            if let Some(rh) = b.right_hand {
                pose.modify_add(rh, r, None, false, ah);
            }
            if let Some(lh) = b.left_hand {
                pose.modify_add(lh, r, None, false, ah);
            }
        }
    }
}

/// UMordhauUtilityLibrary::FInterpToSeparate rva=0x161c9a0 (decomp UMordhauUtilityLibrary.cpp 23261-23285)
pub fn interp_to_separate(cur: f32, target: f32, dt: f32, up: f32, down: f32) -> f32 {
    let speed = if target < cur { down } else { up };
    let d = target - cur;
    if 0.0 < speed && 1e-8 <= d * d {
        let k = (dt * speed).clamp(0.0, 1.0);
        return k * d + cur;
    }
    target
}

#[cfg(test)]
mod tests {
    /// fp-anim r2: a main equipment with bDisableHandSpringAnimation (target 0) keeps the hand springs off
    #[test]
    fn hand_spring_follows_its_target() {
        let mut p = ProcState { hand_spring_target: 0.0, ..ProcState::default() };
        for _ in 0..30 {
            p.update_springs(1.0 / 60.0, 0.0, false);
        }
        assert_eq!(p.hand_spring_weight, 0.0);
        p.hand_spring_target = 1.0;
        p.update_springs(0.1, 0.0, false);
        assert!((p.hand_spring_weight - 0.5).abs() < 1e-6, "FInterpConstantTo at 5 / s");
    }

    /// fp-anim r1: FloatSpringInterp as disassembled (0x143179340): one step from rest toward 1
    #[test]
    fn float_spring_matches_disasm() {
        let mut st = (0.0f32, 0.0f32);
        let dt = 0.016f32;
        let v = float_spring_interp(0.0, 1.0, &mut st, 50.0, 0.55, dt, 0.9);
        let d = 2.0 * (50.0f32 * 0.9).sqrt() * 0.55;
        let vel = (d * 1.0 + 1.0 * 50.0 * dt) / 0.9;
        assert!((st.1 - vel).abs() < 1e-6 && (st.0 - 1.0).abs() < 1e-9);
        assert!((v - vel * dt).abs() < 1e-6);
        // no step for dt 0 or mass 0
        assert_eq!(float_spring_interp(0.3, 1.0, &mut st, 50.0, 0.55, 0.0, 0.9), 0.3);
        assert_eq!(float_spring_interp(0.3, 1.0, &mut st, 50.0, 0.55, dt, 0.0), 0.3);
    }

    /// fp-anim r1: turning drives the yaw spring (target = yaw rate / 720) and the look-up the pitch spring
    #[test]
    fn turning_and_looking_drive_the_springs() {
        let mut p = ProcState::default();
        let dt = 1.0 / 60.0;
        p.update_turn(dt, &TurnInput { yaw: 0.0, loc: FVector::ZERO, vel2: 0.0, right_leg_bending: false });
        p.update_springs(dt, 0.0, false);
        for k in 1..20 {
            p.update_turn(dt, &TurnInput { yaw: k as f32 * 3.0, loc: FVector::ZERO, vel2: 0.0, right_leg_bending: false });
            p.update_springs(dt, k as f32 * 1.0, false);
        }
        // 180 deg/s -> target 0.25; 60 deg/s look -> 0.1667
        assert!(p.spring.1 > 0.05 && p.spring.1 < 0.6, "{:?}", p.spring);
        assert!(p.spring.0 > 0.02 && p.spring.0 < 0.4, "{:?}", p.spring);
        assert!((p.hand_spring_weight - 1.0).abs() < 1e-6);
    }


    use super::*;

    #[test]
    fn batch_ik_blends_all_target_locals_under_full_target_parents() {
        let yaw = |v| FTransform::new(FQuat::from_rotator(0.0, v, 0.0), FVector::ZERO);
        let sk = Skeleton::from_parts(
            vec!["Ancestor".into(), "Root".into(), "Joint".into(), "End".into()],
            vec![-1, 0, 1, 2], vec![yaw(30.0), yaw(0.0), yaw(0.0), yaw(0.0)],
        );
        let targets = [(1, yaw(120.0)), (2, yaw(120.0)), (3, yaw(120.0))];
        let mut p = CsPose { sk: &sk, local: sk.ref_local.clone() };
        p.blend_cs_batch(&targets, 0.5);
        for i in 1..4 {
            let (_, y, _) = quat_rotator(p.cs(i).rot);
            assert!((y - 75.0).abs() < 1e-3, "bone {i}: {y}");
        }
        // Original snapshot-all/full-target-parent local blending differs from the old sequential code.
        let mut sequential = CsPose { sk: &sk, local: sk.ref_local.clone() };
        for (i, target) in targets {
            sequential.blend_cs(i, target, 0.5);
        }
        let (_, old_joint_yaw, _) = quat_rotator(sequential.cs(2).rot);
        assert!((old_joint_yaw - 97.5).abs() < 1e-3);
    }

    #[test]
    fn batch_ik_preserves_untargeted_intermediate_and_descendant_locals() {
        let yaw = |v| FTransform::new(FQuat::from_rotator(0.0, v, 0.0), FVector::ZERO);
        let sk = Skeleton::from_parts(
            vec!["Root".into(), "Middle".into(), "End".into(), "Follower".into()],
            vec![-1, 0, 1, 2], vec![yaw(0.0), yaw(20.0), yaw(0.0), yaw(15.0)],
        );
        let mut p = CsPose { sk: &sk, local: sk.ref_local.clone() };
        p.blend_cs_batch(&[(0, yaw(90.0)), (2, yaw(110.0))], 0.5);
        assert_eq!(p.local[1], sk.ref_local[1]);
        assert_eq!(p.local[3], sk.ref_local[3]);
        let (_, end_yaw, _) = quat_rotator(p.cs(2).rot);
        let (_, follower_yaw, _) = quat_rotator(p.cs(3).rot);
        assert!((end_yaw - 65.0).abs() < 1e-3, "{end_yaw}");
        assert!((follower_yaw - 80.0).abs() < 1e-3, "{follower_yaw}");
    }

    #[test]
    fn batch_ik_zero_keeps_pose_and_full_alpha_reaches_absolute_targets() {
        let sk = Skeleton::from_parts(
            vec!["Root".into(), "Joint".into(), "End".into()], vec![-1, 0, 1],
            vec![FTransform::IDENTITY; 3],
        );
        let targets = [
            (0, FTransform::new(FQuat::from_rotator(10.0, 90.0, 0.0), FVector::new(2.0, 3.0, 4.0))),
            (1, FTransform::new(FQuat::from_rotator(-20.0, 45.0, 30.0), FVector::new(12.0, 7.0, 9.0))),
            (2, FTransform::new(FQuat::from_rotator(5.0, 15.0, -10.0), FVector::new(16.0, 21.0, 13.0))),
        ];
        let mut p = CsPose { sk: &sk, local: sk.ref_local.clone() };
        p.blend_cs_batch(&targets, 0.0);
        assert_eq!(p.local, sk.ref_local);
        p.blend_cs_batch(&targets, 1.0);
        for (i, target) in targets {
            let actual = p.cs(i);
            assert!((actual.loc - target.loc).length() < 1e-4, "bone {i}: {:?}", actual.loc);
            assert!((qdot(actual.rot, target.rot).abs() - 1.0).abs() < 1e-5, "bone {i}: {:?}", actual.rot);
        }
    }

    #[test]
    fn interp_separate_matches_decomp() {
        assert_eq!(interp_to_separate(0.0, 1.0, 0.1, 2.0, 4.0), 0.2);
        assert_eq!(interp_to_separate(1.0, 0.0, 0.1, 2.0, 4.0), 1.0 - 0.4);
        assert_eq!(interp_to_separate(0.5, 0.5, 0.1, 2.0, 4.0), 0.5);
    }

    #[test]
    fn blend_between_without_reference_is_slerp_like() {
        let a = FQuat::IDENTITY;
        let b = FQuat::from_rotator(0.0, 90.0, 0.0);
        let sk = Skeleton::from_parts(vec!["A".into(), "B".into(), "C".into()], vec![-1, 0, 0], vec![FTransform::new(a, FVector::ZERO), FTransform::new(b, FVector::ZERO), FTransform::IDENTITY]);
        let mut p = CsPose { sk: &sk, local: sk.ref_local.clone() };
        p.blend_between(2, 0, 1, None, 0.5, 1.0);
        let (_, yaw, _) = quat_rotator(p.cs(2).rot);
        assert!((yaw - 45.0).abs() < 1e-3, "{yaw}");
    }

    #[test]
    fn modify_add_bone_space_vs_component_space() {
        let r = FQuat::from_rotator(0.0, 90.0, 0.0);
        let sk = Skeleton::from_parts(vec!["A".into()], vec![-1], vec![FTransform::new(r, FVector::new(1.0, 2.0, 3.0))]);
        let mut p = CsPose { sk: &sk, local: sk.ref_local.clone() };
        // bone space: translation along the bone's own X (rotated 90 yaw -> world +Y)
        p.modify_add(0, (0.0, 0.0, 0.0), Some(FVector::new(10.0, 0.0, 0.0)), true, 1.0);
        let c = p.cs(0);
        assert!((c.loc.x - 1.0).abs() < 1e-4 && (c.loc.y - 12.0).abs() < 1e-4, "{:?}", c.loc);
    }

    #[test]
    fn spine_bend_rotates_spine1_by_look_up() {
        // a chain Hips -> LowerBack -> Spine -> Spine1, identity rest: W = 1, Atmo 0 -> Spine1 ends at Q(roll -L)
        let names: Vec<String> = ["Hips", "LowerBack", "Spine", "Spine1"].iter().map(|s| s.to_string()).collect();
        let up = FTransform::new(FQuat::IDENTITY, FVector::new(0.0, 0.0, 10.0));
        let sk = Skeleton::from_parts(names, vec![-1, 0, 1, 2], vec![FTransform::IDENTITY, up, up, up]);
        let b = ProcBones::new(&sk);
        let mut p = CsPose { sk: &sk, local: sk.ref_local.clone() };
        let st = ProcState::default();
        st.spine_bend(&mut p, &b, &ProcInput { look_up: 30.0, spine_bend_w: 1.0, crouch: 0.0 });
        let (_, _, roll) = quat_rotator(p.cs(3).rot);
        assert!((roll + 30.0).abs() < 1e-2, "{roll}");
        // look-up 0: the static -9 / -13 / -19 rolls are compensated away only partially; Spine1 itself is restored
        let mut p0 = CsPose { sk: &sk, local: sk.ref_local.clone() };
        st.spine_bend(&mut p0, &b, &ProcInput { look_up: 0.0, spine_bend_w: 1.0, crouch: 0.0 });
        let (_, _, r0) = quat_rotator(p0.cs(3).rot);
        assert!(r0.abs() < 1e-3, "{r0}");
    }
}

#[cfg(test)]
mod two_bone_tests {
    use super::*;

    /// SolveTwoBoneIK: a reachable effector is met exactly, the limb lengths are kept and the joint bends toward the
    /// joint target; an unreachable one straightens the limb along the effector direction
    #[test]
    fn solve_two_bone_ik_reaches_and_keeps_lengths() {
        let t = |x: f32, y: f32, z: f32| FTransform::new(FQuat::IDENTITY, FVector::new(x, y, z));
        let (r, j, e) = (t(0.0, 0.0, 0.0), t(30.0, 0.0, 0.0), t(60.0, 0.0, 0.0));
        let (r1, j1, e1) = solve_two_bone_ik(r, j, e, FVector::new(30.0, 0.0, 50.0), FVector::new(40.0, 0.0, 0.0));
        assert!((e1.loc - FVector::new(40.0, 0.0, 0.0)).length() < 1e-3);
        assert!(((j1.loc - r1.loc).length() - 30.0).abs() < 1e-3 && ((e1.loc - j1.loc).length() - 30.0).abs() < 1e-3);
        assert!(j1.loc.z > 0.0, "bends toward the joint target");
        let (_, j2, e2) = solve_two_bone_ik(r, j, e, FVector::new(30.0, 0.0, 50.0), FVector::new(0.0, 100.0, 0.0));
        assert!((e2.loc - FVector::new(0.0, 60.0, 0.0)).length() < 1e-3 && (j2.loc - FVector::new(0.0, 30.0, 0.0)).length() < 1e-3);
        // the root's rotation turns the old upper-limb direction onto the new one
        let d = r1.rot.rotate(FVector::new(1.0, 0.0, 0.0));
        assert!((d - (j1.loc - r1.loc).scale(1.0 / 30.0)).length() < 1e-3);
    }
}

impl ProcState {
    /// UMordhauAnimInstance::NativeUpdateAnimation rva=0x1501930 (decomp 2998-3012, 3880-3929):
    /// ShoulderOffset1PWeight = FInterpConstantTo(weight, target, dt, OffhandIKChangeSpeed), target 1 unless the motion
    /// bDisablesOffhandIK (0); UpdateBlueprintHelpers rva=0x151a2b0 (decomp 5258):
    /// Helper_ShoulderOffset1PWith1PWeight = ShoulderOffset1PWeight * IsFirstPersonFloat. UNCONFIRMED: the montage
    /// general MetaData overlays outside selected switch montages and the airborne branch (3880-3887) are not modelled (fidelity-audit r4)
    pub fn update_shoulder_1p(&mut self, dt: f32, f: MotionFlags, first_person: bool) {
        let target = if f.disables_offhand_ik { 0.0 } else { 1.0 };
        let d = target - self.shoulder_1p_weight;
        let step = f.offhand_ik_change_speed * dt;
        self.shoulder_1p_weight = if d * d < 1e-8 { target } else { self.shoulder_1p_weight + d.clamp(-step, step) };
        self.is_first_person = if first_person { 1.0 } else { 0.0 };
    }
    /// AB_MordhauCharacterAnimation ModifyBone_82 (LeftShoulder, LeftShoulderOffset1P) then ModifyBone_83 (RightShoulder,
    /// RightShoulderOffset1P): translation BMM_Additive in BCS_ComponentSpace, Alpha = Helper_ShoulderOffset1PWith1PWeight
    /// (PropertyAccessLibrary copies 138-141; node data AnimGraphNode_ModifyBone_82 / _83)
    pub fn shoulder_offsets_1p(&self, pose: &mut CsPose, b: &ProcBones, off: (FVector, FVector)) {
        let a = self.shoulder_1p_weight * self.is_first_person;
        if let Some(l) = b.left_shoulder {
            pose.modify_add(l, (0.0, 0.0, 0.0), Some(off.1), false, a);
        }
        if let Some(r) = b.right_shoulder {
            pose.modify_add(r, (0.0, 0.0, 0.0), Some(off.0), false, a);
        }
    }
}

// ---- the offhand (left hand) grip IK: AB_MordhauCharacterAnimation TwoBoneIK_2 (548) on LeftHand, effector
// LeftHandGripPosition in RightWeapon bone space, Alpha Helper_LeftHandIKWeight (PropertyAccessLibrary copies 226 / 227)
// (fidelity-audit r4) --------------------------------------------------------------------------------------------

/// The right-hand equipment's offhand-IK fields (AMordhauEquipment, types/AMordhauEquipment.h) and the merged weapon
/// mesh's sockets
#[derive(Clone, Debug, Default)]
pub struct OffhandWeapon {
    /// bUsesOffhandIK (+0xae4) / bInvertOffhandUp (+0xae5)
    pub uses_offhand_ik: bool,
    pub invert_up: bool,
    /// OffhandIKUpOffset (+0xb3c; ctor -10, AMordhauEquipment.cpp 1672) / OffhandIKUpOffset1P (+0xb40; ctor 0)
    pub up_offset: f32,
    pub up_offset_1p: f32,
    /// GripEndLocationLocal (+0xc50) when the mesh has a "GripEnd" socket (bHasFoundGripEndSocket, OnPartsChanged
    /// rva=0x1556340): its Z
    pub grip_end_z: Option<f32>,
    /// the "OffhandIK" / "OffhandIK1P" sockets (weapon-local), the fixed-target path
    pub fixed: Option<FVector>,
    pub fixed_1p: Option<FVector>,
    /// the held weapon's actor transform relative to the RightWeapon bone (ComputeGrippedTransform rva=0x14b70f0)
    pub grip_rel: FTransform,
}

/// Original switch flags: OnBegin 165fbb0, stage transitions 1667e00.
/// Type1 retains speed1.25; Type0 transfers control to the right hand only in stage1.
pub fn mode_switch_offhand(kind: i64, stage: i64) -> (bool, f32, bool) {
    let speed = if kind == 1 { 1.25 } else if stage >= 2 { 5.0 } else if kind == 0 && stage == 1 {
        1.0 / mordhau_core::combat::modeswitch::STAGE2_DURATION as f32
    } else { 1.0 / mordhau_core::combat::modeswitch::STAGE1_DURATION as f32 };
    (stage == 0, speed, kind == 0 && stage == 1)
}

/// Target-mode ComputeGrippedTransform on identity RightWeapon and LeftHand sockets.
#[derive(Clone, Copy, Debug)]
pub struct SwitchHandGrips {
    pub right: FTransform,
    pub left: FTransform,
    pub rotation_offset: FQuat,
}

/// Native ComputeSwitchHandIK164fa10 uses different spaces for these two outputs.
#[derive(Clone, Copy, Debug, Default)]
pub struct RightHandTarget {
    pub left_hand_position: FVector,
    pub component_rotation: FQuat,
}

/// ComputeSwitchHandIK164fa10: common world placement cancels in the position;
/// the cooked world-rotation replacement becomes this rotation in component space.
/// Existing FTransform unit-scale restriction applies; no screenshot alignment constant.
pub fn switch_hand_target(rw: FTransform, rh: FTransform, lh: FTransform, rw_local: FTransform, g: &SwitchHandGrips) -> RightHandTarget {
    let gr = g.right.then(&rw);
    let gl = g.left.then(&lh);
    RightHandTarget {
        left_hand_position: lh.inverse_apply(gl.apply(gr.inverse_apply(rh.loc))),
        component_rotation: gl.rot.mul(FQuat::from_rotator(90.0, 0.0, 0.0).inverse())
            .mul(g.rotation_offset.inverse()).mul(rw_local.rot.inverse()),
    }
}

/// The current motion's offhand fields (UMordhauMotion: bDisablesOffhandIK +0x72, OffhandIKChangeSpeed +0x78,
/// OffhandIKDistanceMax / Min; the attack sequence's UMordhauAnimMetaData overrides, UAttackMotion.cpp 7210-7230)
#[derive(Clone, Copy, Debug)]
pub struct OffhandMotion {
    /// A selected original switch montage supplied metadata after motion defaults.
    pub has_switch_montage_overlay: bool,
    pub disables: bool,
    pub right_hand: bool,
    pub right_target: Option<RightHandTarget>,
    pub forces: bool,
    pub change_speed: f32,
    pub distance_max: f32,
    pub distance_min: f32,
}

impl Default for OffhandMotion {
    /// UMordhauMotion ctor: OffhandIKChangeSpeed 2.5 (decomp UMordhauMotion.cpp 199), distances 0
    fn default() -> Self {
        OffhandMotion { has_switch_montage_overlay: false, disables: false, right_hand: false, right_target: None, forces: false, change_speed: 2.5, distance_max: 0.0, distance_min: 0.0 }
    }
}

impl OffhandMotion {
    /// Native post-montage disable/speed also drive ShoulderOffset1PWeight.
    /// Preserve existing non-switch flags; forces/distances do not affect shoulder weight.
    pub fn switch_shoulder_flags(&self, mut flags: MotionFlags) -> MotionFlags {
        if self.has_switch_montage_overlay {
            flags.disables_offhand_ik = self.disables;
            flags.offhand_ik_change_speed = self.change_speed;
        }
        flags
    }
}

#[derive(Clone, Debug, Default)]
pub struct OffhandState {
    /// Native OffhandIsRightHand; left/right alpha split in UpdateBlueprintHelpers151a2b0.
    pub right_fraction: f32,
    pub right_target: RightHandTarget,
    /// OffhandIKWeight, OffhandIKSeparationSmooth, LeftHandGripPosition (RightWeapon bone space)
    pub weight: f32,
    pub separation: f32,
    pub grip: FVector,
    pub initialised: bool,
}

/// FMath::FInterpTo (engine source)
fn finterp_to_f(cur: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let d = target - cur;
    if d * d < 1e-8 {
        return target;
    }
    cur + d * (dt * speed).clamp(0.0, 1.0)
}

/// FMath::FInterpConstantTo (engine source)
fn finterp_constant_to_f(cur: f32, target: f32, dt: f32, speed: f32) -> f32 {
    let d = target - cur;
    if d * d < 1e-8 {
        return target;
    }
    let step = speed * dt;
    cur + d.clamp(-step, step)
}

impl OffhandState {
    /// UMordhauAnimInstance::NativeUpdateAnimation rva=0x1501930 (decomp 3158-3880, 3928-3936), on the game thread from
    /// the previous frame's bones `prev` (component space). Sliding target: d = the weapon-local Z of
    /// RightHandFinger03_02; separation = OffhandIKUpOffset(1P) clamped by the motion's distances, + slide compensation,
    /// FInterpTo(separation, 3.5 when closing else 10); z = d + separation (x -1 when bInvertOffhandUp), not below the
    /// GripEnd socket; P = weapon(0, 0, z); P -= (LeftHandFinger05_01 - LeftWeapon) + (LeftHandFinger04_02 -
    /// LeftHandFinger05_01) x 0.6; LeftHandGripPosition = RightWeapon^-1(P). Fixed target: the OffhandIK(1P) socket.
    /// OffhandIKWeight = FInterpConstantTo(weight, target, dt, OffhandIKChangeSpeed), target = (bUsesOffhandIK ||
    /// bForcesOffhandIK) && !bDisablesOffhandIK. UNCONFIRMED / not modelled: the low-LOD branches, the horse case, the
    /// equipment-switch motion's equipment, the DisableOffhandIK anim curve (UpdateBlueprintHelpers decomp 5176)
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, sk: &Skeleton, prev: &[FTransform], w: &OffhandWeapon, m: &OffhandMotion, first_person: bool, slide: (f32, f32), dt: f32) {
        let find = |n: &str| sk.find(n).and_then(|i| prev.get(i).copied());
        let (Some(rw), Some(f03), Some(lw), Some(l05), Some(l04)) =
            (find("RightWeapon"), find("RightHandFinger03_02"), find("LeftWeapon"), find("LeftHandFinger05_01"), find("LeftHandFinger04_02"))
        else {
            return;
        };
        let weapon = w.grip_rel.then(&rw);
        let fixed = if first_person { w.fixed_1p.or(w.fixed) } else { w.fixed };
        let p = match fixed {
            Some(s) => weapon.apply(s),
            None => {
                let d = weapon.inverse_apply(f03.loc).z;
                let mut sep = if first_person { w.up_offset_1p } else { w.up_offset };
                if m.distance_max != 0.0 && sep <= m.distance_max {
                    sep = m.distance_max;
                }
                if m.distance_min != 0.0 && m.distance_min <= sep {
                    sep = m.distance_min;
                }
                sep += slide.0 * slide.1;
                let speed = if sep <= self.separation { 3.5 } else { 10.0 };
                self.separation = if self.initialised { finterp_to_f(self.separation, sep, dt, speed) } else { sep };
                let sign = if w.invert_up { -1.0 } else { 1.0 };
                let mut z = self.separation * sign + d;
                if let Some(ge) = w.grip_end_z {
                    if z <= ge {
                        z = ge;
                    }
                }
                let p = weapon.apply(FVector::new(0.0, 0.0, z));
                let off = (l05.loc - lw.loc) + (l04.loc - l05.loc).scale(0.6);
                p - off
            }
        };
        self.grip = rw.inverse_apply(p);
        self.update_weights(m, w.uses_offhand_ik || m.forces, dt);
        self.initialised = true;
    }

    fn update_weights(&mut self, m: &OffhandMotion, left_target: bool, dt: f32) {
        if let Some(target) = m.right_target { self.right_target = target; }
        // Original virtual motion target contributes even when ordinary offhand equipment is disabled.
        let target = if (left_target || m.right_target.is_some()) && !m.disables { 1.0 } else { 0.0 };
        self.weight = if self.initialised { finterp_constant_to_f(self.weight, target, dt, m.change_speed) } else { target };
        let right = if m.right_hand { 1.0 } else { 0.0 };
        // NativeUpdateAnimation3928–3945 updates weight before the fraction; division uses the new weight.
        self.right_fraction = if self.weight >= 0.01 {
            finterp_constant_to_f(self.right_fraction, right, dt, m.change_speed / self.weight)
        } else { right };
    }

    /// TwoBoneIK_2 (548): IKBone LeftHand, EffectorLocation = LeftHandGripPosition in BCS_BoneSpace of RightWeapon,
    /// JointTarget (0, -92.38372, -38.27853) in LeftArm bone space, bAllowStretching (StartStretchRatio 0.9,
    /// MaxStretchScale 1.25), end rotation kept (bTakeRotationFromEffectorSpace / bMaintainEffectorRelRot false)
    pub fn apply(&self, pose: &mut CsPose) {
        let sk = pose.sk;
        // Cooked548 left IK, then541 right IK, then540 world rotation replacement.
        let targets = [
            ("LeftHand", "RightWeapon", self.grip, self.weight * (1.0 - self.right_fraction), false),
            ("RightHand", "LeftHand", self.right_target.left_hand_position, self.weight * self.right_fraction, true),
        ];
        for (hand, effector_bone, grip, alpha, right) in targets {
            if alpha < ZERO_ANIMWEIGHT_THRESH { continue; }
            let (Some(end), Some(eb)) = (sk.find(hand), sk.find(effector_bone)) else { continue };
            let joint = sk.parents[end];
            if joint < 0 || sk.parents[joint as usize] < 0 { continue; }
            let (joint, root) = (joint as usize, sk.parents[joint as usize] as usize);
            let r0 = pose.cs(root);
            let jt = FTransform::new(FQuat::IDENTITY, FVector::new(0.0, -92.38372, -38.27853)).then(&r0).loc;
            let eff = pose.cs(eb).apply(grip);
            let (r1, j1, e1) = solve_two_bone_ik_stretch(r0, pose.cs(joint), pose.cs(end), jt, eff, 0.9, 1.25);
            pose.blend_cs_batch(&[(root, r1), (joint, j1), (end, e1)], alpha);
            if right { pose.copy_rot(end, self.right_target.component_rotation, alpha); }
        }
    }
}

/// fp-anim r2: TwoBoneIK_7 / _8 (nodes 465 / 466, below the hips / grounding nodes, before the offhand TwoBoneIK_2):
/// LeftFoot / RightFoot to their virtual bones VB Position_LeftFoot / _RightFoot (EffectorLocation 0, BCS_BoneSpace),
/// joint target (0, -90, 100) in LeftUpLeg / RightUpLeg bone space, bAllowStretching (StartStretchRatio 0.9,
/// MaxStretchScale 1.15), Alpha IsFirstPersonFloat (AB CDO; PropertyAccess copies). `alpha` scales it (the Sim passes the
/// LowerBody machine's Ground weight: UNCONFIRMED, the airborne states copy the feet into the VBs without the offset).
pub fn first_person_feet_ik(pose: &mut CsPose, vb: (FVector, FVector), alpha: f32) {
    if alpha < ZERO_ANIMWEIGHT_THRESH {
        return;
    }
    let sk = pose.sk;
    for (foot, up, target) in [("LeftFoot", "LeftUpLeg", vb.0), ("RightFoot", "RightUpLeg", vb.1)] {
        let (Some(f), Some(u)) = (sk.find(foot), sk.find(up)) else { continue };
        let j = sk.parents[f];
        if j < 0 || sk.parents[j as usize] != u as i32 {
            continue;
        }
        let j = j as usize;
        let r0 = pose.cs(u);
        let jt = FTransform::new(FQuat::IDENTITY, FVector::new(0.0, -90.0, 100.0)).then(&r0).loc;
        let (r1, j1, e1) = solve_two_bone_ik_stretch(r0, pose.cs(j), pose.cs(f), jt, target, 0.9, 1.15);
        pose.blend_cs_batch(&[(u, r1), (j, j1), (f, e1)], alpha);
    }
}

/// AnimationCore::SolveTwoBoneIK with bAllowStretching (UE 4.26 TwoBoneIK.cpp, UNCONFIRMED as compiled): reach ratio =
/// desired length / (upper + lower); scale = (MaxStretchScale - 1) x clamp((ratio - Start) / (Max - Start), 0, 1);
/// both limb lengths x (1 + scale); then `solve_two_bone_ik` with the stretched lengths
pub fn solve_two_bone_ik_stretch(root: FTransform, joint: FTransform, end: FTransform, joint_target: FVector, effector: FVector, start: f32, max_scale: f32) -> (FTransform, FTransform, FTransform) {
    let upper = (joint.loc - root.loc).length();
    let lower = (end.loc - joint.loc).length();
    let max_len = upper + lower;
    let desired = (effector - root.loc).length();
    let range = max_scale - start;
    let mut s = 0.0;
    if range > 1e-4 && max_len > 1e-4 {
        let ratio = desired / max_len;
        s = (max_scale - 1.0) * ((ratio - start) / range).clamp(0.0, 1.0);
    }
    if s <= 1e-4 {
        return solve_two_bone_ik(root, joint, end, joint_target, effector);
    }
    let k = 1.0 + s;
    let joint_s = FTransform::new(joint.rot, root.loc + (joint.loc - root.loc).scale(k as f64));
    let end_s = FTransform::new(end.rot, joint_s.loc + (end.loc - joint.loc).scale(k as f64));
    solve_two_bone_ik(root, joint_s, end_s, joint_target, effector)
}

/// The strike's look counter-compensation (the crosshair alignment of a swing; reader method 2026-10-06 + decomp):
/// - UStrikeMotion::OnTick_Implementation rva=0x166a0b0 (decomp UStrikeMotion.cpp 907-990): once WindupEnd - LagInduction
///   has passed (bHasSetLookCounterCompensation), LookUpCompensationFactor = 1, TurnCompensationFactor =
///   ((1 - |AngleTarget|) + 1) * 0.5, ReleaseLookUpValue = LookUpValue, ReleaseTurnValue = the mesh's world yaw.
/// - UStrikeMotion::ShouldCounterCompensateLook rva=0x166d560: WindupEnd - LagInduction <= now and Stage != Recovery.
/// - UStrikeMotion::ComputeCounterCompensationRotation rva=0x164f670 (decomp 570-735): while now < WindupEnd -
///   LagInduction + CounterCompensateLookTime, from L = LookUpValue - ReleaseLookUpValue and T = ClampAngle(mesh yaw -
///   ReleaseTurnValue, left ? [0, Max] : [-Max, 0]): AngleTarget <= 0 -> L = min(L, 0) * f, yaw term = (left ? -L : L)
///   * OverheadFixupTerm, tilt = (left ? -L : L) * OverheadFixupTiltTerm; else L = max(L, 0) * f and no terms;
///   CounterCompensateRotation = (tilt, yaw term - T * TurnCompensationFactor, L) * CounterCompensateWeight; afterwards
///   the stored rotation is returned unchanged.
/// - UStrikeMotion ctor rva=0x16491c0: OverheadFixupTerm -0.65, OverheadFixupTiltTerm 0.25, Weight 0.5,
///   MaxTurnCompensation 90, CounterCompensateLookTime 0.25 (the shipped strike BPs keep them; only the
///   *_CombatTestCasual variants override).
/// - UMordhauAnimInstance::NativeUpdateAnimation (decomp UMordhauAnimInstance.cpp 4365-4377): ShouldCounterCompensateLook
///   -> CounterCompensateLookWeight = 1 and CounterCompensateRotation = Compute...; else the weight
///   FInterpConstantTo 0 at 4/s (the rotation keeps its last value).
/// - AB_MordhauCharacterAnimation node 457 ModifyBone_90: Spine1, rotation BMM_Additive BCS_ComponentSpace = the
///   rotation, Alpha = the weight; right after the look-up spine bend (463) and before ModifyBone_89 (458).
#[derive(Clone, Debug, Default)]
pub struct CounterCompensation {
    motion: Option<(u64, f64)>,
    has_set: bool,
    release_look_up: f32,
    release_turn: f32,
    look_factor: f32,
    turn_factor: f32,
    /// the motion's CounterCompensateRotation (pitch, yaw, roll)
    motion_rot: (f32, f32, f32),
    /// the anim instance's CounterCompensateRotation / CounterCompensateLookWeight
    pub rot: (f32, f32, f32),
    pub weight: f32,
}

/// the current strike as the counter-compensation reads it
pub struct StrikeView {
    pub key: (u64, f64),
    pub mv: i64,
    pub stage: i64,
    pub windup_end: f64,
    pub lag_induction: f64,
    pub angle_target: f64,
}

const CC_OVERHEAD_FIXUP: f32 = -0.65;
const CC_OVERHEAD_FIXUP_TILT: f32 = 0.25;
const CC_WEIGHT: f32 = 0.5;
const CC_MAX_TURN: f32 = 90.0;
const CC_LOOK_TIME: f64 = 0.25;

/// FMath::ClampAngle (UE4 engine source, UnrealMath.cpp; UNCONFIRMED as compiled)
pub fn clamp_angle(angle: f32, min: f32, max: f32) -> f32 {
    let max_delta = ((max - min) * 0.5).clamp(0.0, 180.0);
    let center = (min + max_delta).rem_euclid(360.0);
    let d = normalize_axis(angle - center);
    if d > max_delta {
        normalize_axis(center + max_delta)
    } else if d < -max_delta {
        normalize_axis(center - max_delta)
    } else {
        normalize_axis(angle)
    }
}

impl CounterCompensation {
    /// one frame: `strike` = the current motion when it is a UStrikeMotion; `yaw` = the mesh's world yaw (any constant
    /// offset cancels)
    pub fn update(&mut self, now: f64, dt: f32, strike: Option<&StrikeView>, look_up: f32, yaw: f32) {
        let Some(s) = strike else {
            self.motion = None;
            self.weight = crate::blendspace::finterp_constant_to(self.weight as f64, 0.0, dt as f64, 4.0) as f32;
            return;
        };
        if self.motion != Some(s.key) {
            self.motion = Some(s.key);
            self.has_set = false;
            self.look_factor = 0.0;
            self.turn_factor = 0.0;
            self.motion_rot = (0.0, 0.0, 0.0);
        }
        let release = s.windup_end - s.lag_induction;
        // OnTick: latch at release
        if release < now && !self.has_set {
            self.has_set = true;
            self.look_factor = 1.0;
            self.turn_factor = ((1.0 - s.angle_target.abs() as f32) + 1.0) * 0.5;
            self.release_look_up = look_up;
            self.release_turn = yaw;
        }
        let should = release <= now && s.stage != mordhau_core::combat::enums::stage::RECOVERY;
        if !should {
            self.weight = crate::blendspace::finterp_constant_to(self.weight as f64, 0.0, dt as f64, 4.0) as f32;
            return;
        }
        self.weight = 1.0;
        if now < release + CC_LOOK_TIME {
            let left = mordhau_core::combat::enums::is_left(s.mv);
            let mut l = look_up - self.release_look_up;
            let (yaw_term, tilt) = if s.angle_target <= 0.0 {
                l = l.min(0.0) * self.look_factor;
                let sl = if left { -l } else { l };
                (sl * CC_OVERHEAD_FIXUP, sl * CC_OVERHEAD_FIXUP_TILT)
            } else {
                l = l.max(0.0) * self.look_factor;
                (0.0, 0.0)
            };
            let d = normalize_axis(yaw - self.release_turn);
            let t = if left { clamp_angle(d, 0.0, CC_MAX_TURN) } else { clamp_angle(d, -CC_MAX_TURN, 0.0) };
            self.motion_rot = (tilt * CC_WEIGHT, (yaw_term - t * self.turn_factor) * CC_WEIGHT, l * CC_WEIGHT);
        }
        self.rot = self.motion_rot;
    }

    /// node 457 ModifyBone_90: Spine1 rotation BMM_Additive in component space at the weight
    /// 456 ModifyBone_91 (LeftShoulder, additive CS; Blueprint-bound inputs decoded from the AnimBP Kismet, ubergraph
    /// @19286: Rotation = NegateRotator(CounterCompensateRotation), Alpha = CounterCompensateLookWeight *
    /// (1 - Helper_LeftHandIKWeight)) runs right before 457 ModifyBone_90 (Spine1, additive CS, CounterCompensateRotation
    /// at CounterCompensateLookWeight): a free left arm (no offhand IK) is kept out of the spine's counter-compensation.
    /// `left_hand_ik_weight` = Helper_LeftHandIKWeight (the offhand IK weight the previous frame produced).
    pub fn apply(&self, pose: &mut CsPose, b: &ProcBones, left_hand_ik_weight: f32) {
        if let Some(ls) = b.left_shoulder {
            let neg = (-self.rot.0, -self.rot.1, -self.rot.2);
            pose.modify_add(ls, neg, None, false, (self.weight * (1.0 - left_hand_ik_weight)).clamp(0.0, 1.0));
        }
        if let Some(s1) = b.spine1 {
            pose.modify_add(s1, self.rot, None, false, self.weight);
        }
    }
}

// ---- the hit-effect IK (EVD_SWG_003): UMordhauAnimInstance HitEffect* + AB_MordhauCharacterAnimation TwoBoneIK_4 (491)
// and ModifyBone_74 (492) -----------------------------------------------------------------------------------------

/// UMordhauAnimInstance's hit-effect IK state: bIsDoingHitEffectIK, HitEffectIKWeight, HitEffectIKLocation(Start),
/// HitEffectRotation(Start) (component space), and the last evaluated pose's "VB Global_RightHand" (CopyBone_14, node
/// 488: RightHand's CS transform, copied before 550 / 491 in the ground-pose chain)
#[derive(Clone, Debug, Default)]
pub struct HitEffect {
    pub doing: bool,
    pub weight: f32,
    pub loc: FVector,
    pub loc_start: FVector,
    pub rot: (f32, f32, f32),
    pub rot_start: (f32, f32, f32),
    pub vb: Option<FTransform>,
}

/// UMordhauAnimInstance ctor (decomp 7048-7049): HitEffectLocationSlideSpeed 200, HitEffectDisableSpeed 3
pub const HIT_EFFECT_SLIDE_SPEED: f32 = 200.0;
pub const HIT_EFFECT_DISABLE_SPEED: f32 = 3.0;

/// FMath::VInterpConstantTo (engine source)
fn vinterp_constant_to(cur: FVector, target: FVector, dt: f32, speed: f32) -> FVector {
    let d = target - cur;
    let dist = d.length();
    let step = speed * dt;
    if dist > step {
        if step > 0.0 {
            let k = step / dist;
            return cur + FVector::new(d.x * k, d.y * k, d.z * k);
        }
        return cur;
    }
    target
}

impl HitEffect {
    /// NativeUpdateAnimation rva=0x1501930 (decomp UMordhauAnimInstance.cpp 4280-4364). `active` = the character is the
    /// view target, its motion is an UAttackMotion with bHasHitIncludingCosmeticHit, now < ReleaseEnd + 0.1 and its
    /// AttackInfo.HitEffectIKWeightCurve is set; `curve` = that curve. The VB is GetSocketTransform("VB
    /// Global_RightHand", RTS_Component) of the last evaluated pose. First frame: location / start = the VB location,
    /// rotation / start = its FQuat::Rotator, weight 1; then weight = min(weight, curve(|start - VB|)), location
    /// VInterpConstantTo the VB at HitEffectLocationSlideSpeed, rotation = the VB's with the start yaw. Inactive:
    /// bIsDoingHitEffectIK false and the weight FInterpConstantTo 0 at HitEffectDisableSpeed.
    pub fn update(&mut self, dt: f32, active: bool, curve: &dyn Fn(f32) -> f32) {
        match (active, self.vb) {
            (true, Some(vb)) => {
                let r = quat_rotator(vb.rot);
                if !self.doing {
                    self.doing = true;
                    self.loc = vb.loc;
                    self.loc_start = vb.loc;
                    self.rot = r;
                    self.rot_start = r;
                    self.weight = 1.0;
                } else {
                    let w = curve((self.loc_start - vb.loc).length());
                    self.weight = self.weight.min(w);
                    self.loc = vinterp_constant_to(self.loc, vb.loc, dt, HIT_EFFECT_SLIDE_SPEED);
                    self.rot = (r.0, self.rot_start.1, r.2);
                }
            }
            _ => self.doing = false,
        }
        if !self.doing {
            self.weight = finterp_constant_to_f(self.weight, 0.0, dt, HIT_EFFECT_DISABLE_SPEED);
        }
    }

    /// TwoBoneIK_4 (491): IKBone RightHand, EffectorLocation = HitEffectIKLocation in BCS_ComponentSpace, joint target
    /// (0, -92.38372, -38.27853) in RightArm bone space, bAllowStretching (StartStretchRatio 0.9, MaxStretchScale 2),
    /// no effector rotation taken / kept; then ModifyBone_74 (492): RightHand rotation BMM_Replace in BCS_ComponentSpace
    /// = HitEffectRotation. Both Alpha HitEffectIKWeight (PropertyAccess copies 460-469; AB CDO).
    pub fn apply(&self, pose: &mut CsPose, b: &ProcBones) {
        if self.weight < ZERO_ANIMWEIGHT_THRESH {
            return;
        }
        let sk = pose.sk;
        let (Some(e), Some(u)) = (b.right_hand, b.right_arm) else { return };
        let j = sk.parents[e];
        if j < 0 || sk.parents[j as usize] != u as i32 {
            return;
        }
        let j = j as usize;
        let r0 = pose.cs(u);
        let jt = FTransform::new(FQuat::IDENTITY, FVector::new(0.0, -92.38372, -38.27853)).then(&r0).loc;
        let (r1, j1, e1) = solve_two_bone_ik_stretch(r0, pose.cs(j), pose.cs(e), jt, self.loc, 0.9, 2.0);
        pose.blend_cs_batch(&[(u, r1), (j, j1), (e, e1)], self.weight);
        let mut n = pose.cs(e);
        n.rot = FQuat::from_rotator(self.rot.0, self.rot.1, self.rot.2);
        pose.blend_cs(e, n, self.weight);
    }
}


#[cfg(test)]
mod mode_switch_tests {
    use super::*;

    fn same_rotation(a: FQuat, b: FQuat) -> bool { qdot(a, b).abs() > 1.0 - 1e-5 }

    #[test]
    fn native_switch_target_preserves_grip_point_and_rotation_chain() {
        let xf = |p, y, ro, x, z| FTransform::new(FQuat::from_rotator(p, y, ro), FVector::new(x, 4.0, z));
        let rw = xf(7.0, 22.0, -11.0, 13.0, 8.0);
        let rh = xf(6.0, 15.0, 8.0, 10.0, 6.0);
        let lh = xf(-4.0, -33.0, 14.0, -19.0, 2.0);
        let local = xf(10.0, -13.0, 12.0, 3.0, -2.0);
        let g = SwitchHandGrips {
            right: xf(3.0, 9.0, -27.0, 2.0, 9.0),
            left: xf(-19.0, 21.0, 4.0, -8.0, 3.0),
            rotation_offset: FQuat::from_rotator(165.0, 15.0, 0.0),
        };
        let t = switch_hand_target(rw, rh, lh, local, &g);
        let gr = g.right.then(&rw);
        let gl = g.left.then(&lh);
        let actual_point = lh.apply(t.left_hand_position);
        let original_grip_point = gr.inverse_apply(rh.loc);
        assert!((gl.inverse_apply(actual_point) - original_grip_point).length() < 1e-3);
        // Reconstitute the desired target weapon rotation from the right-hand output.
        let reconstructed = t.component_rotation.mul(local.rot).mul(g.rotation_offset).mul(FQuat::from_rotator(90.0, 0.0, 0.0));
        assert!(same_rotation(reconstructed, gl.rot));
        // Position is LeftHand bone-space; rotation transforms once from component to world.
        let world = xf(12.0, 61.0, -9.0, 100.0, 70.0);
        let tw = switch_hand_target(rw.then(&world), rh.then(&world), lh.then(&world), local, &g);
        assert!((tw.left_hand_position - t.left_hand_position).length() < 1e-3);
        assert!(same_rotation(tw.component_rotation, world.rot.mul(t.component_rotation)));
    }

    #[test]
    fn native_switch_flags_select_right_hand_only_in_type_zero_stage_one() {
        assert_eq!(mode_switch_offhand(0, 0), (true, 4.0, false));
        assert_eq!(mode_switch_offhand(0, 1), (false, 1.0 / 0.15_f32, true));
        assert_eq!(mode_switch_offhand(0, 2), (false, 5.0, false));
        for stage in 0..3 { assert_eq!(mode_switch_offhand(1, stage), (stage == 0, 1.25, false)); }
        for kind in [2, 3] {
            assert_eq!(mode_switch_offhand(kind, 0), (true, 4.0, false));
            assert_eq!(mode_switch_offhand(kind, 1), (false, 4.0, false));
            assert_eq!(mode_switch_offhand(kind, 2), (false, 5.0, false));
        }
    }

    #[test]
    fn native_offhand_fraction_uses_new_weight_and_transfers_back_after_switch() {
        let mut s = OffhandState { weight: 1.0, initialised: true, ..Default::default() };
        let disabled = OffhandMotion { disables: true, change_speed: 4.0, ..Default::default() };
        s.update_weights(&disabled, true, 0.25);
        assert_eq!((s.weight, s.right_fraction), (0.0, 0.0));
        let right = OffhandMotion { right_hand: true, right_target: Some(RightHandTarget::default()), change_speed: 1.0 / 0.15, ..Default::default() };
        s.update_weights(&right, false, 0.0625);
        assert_eq!(s.weight, 0.0625_f32 * (1.0 / 0.15_f32));
        assert_eq!(s.right_fraction, 1.0);
        let left = OffhandMotion { change_speed: 5.0, ..Default::default() };
        let expected_weight = s.weight + 0.05_f32 * 5.0;
        let expected_fraction = 1.0_f32 - 0.05_f32 * (5.0 / expected_weight);
        s.update_weights(&left, true, 0.05);
        assert_eq!(s.weight, expected_weight);
        assert_eq!(s.right_fraction, expected_fraction);
        for _ in 0..10 { s.update_weights(&left, true, 0.05); }
        assert_eq!((s.weight, s.right_fraction), (1.0, 0.0));
    }

    #[test]
    fn cooked_right_solver_reaches_left_hand_effector_then_replaces_rotation() {
        let names = ["Root", "RightArm", "RightForearm", "RightHand", "RightWeapon", "LeftArm", "LeftForearm", "LeftHand"];
        let locs = [(0.0,0.0), (0.0,0.0), (5.0,0.0), (5.0,0.0), (0.0,0.0), (0.0,20.0), (5.0,0.0), (5.0,0.0)];
        let sk = Skeleton::from_parts(names.iter().map(|n| n.to_string()).collect(), vec![-1,0,1,2,3,0,5,6],
            locs.iter().map(|(x,y)| FTransform::new(FQuat::IDENTITY, FVector::new(*x,*y,0.0))).collect());
        let mut pose = CsPose { sk: &sk, local: sk.ref_local.clone() };
        let left = pose.cs(7);
        let rotation = FQuat::from_rotator(17.0, 20.0, 9.0);
        let want = FVector::new(5.0, 7.0, 0.0);
        let s = OffhandState { weight: 1.0, right_fraction: 1.0,
            right_target: RightHandTarget { left_hand_position: left.inverse_apply(want), component_rotation: rotation }, ..Default::default() };
        s.apply(&mut pose);
        assert!((pose.cs(3).loc - want).length() < 1e-3);
        assert!(same_rotation(pose.cs(3).rot, rotation));
        assert_eq!(pose.cs(7).loc, left.loc);
        assert!(same_rotation(pose.cs(7).rot, left.rot));
    }
}

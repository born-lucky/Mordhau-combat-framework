//! The "LowerBody" state machine of AB_MordhauCharacterAnimation (BakedStateMachines[1], state Ground; rust-combat
//! r5): the base pose LayeredBoneBlend_1 puts the upper body on. Node indices are the CDO AnimGraphNode_* order
//! (anim_graph_data.gd); bindings from the PropertyAccessLibrary copy records.
//!
//! Ground (StateResult_2, 698) from the leaves up:
//!   BlendListByBool_2 (687, bActiveValue <- bHelper_LBVelocityIsZero = anim Velocity == 0, BlendTime 0.3 / 0.3,
//!   HermiteCubic):
//!     [0] standing (677): BlendListByBool_4 (665, bHelper_LBFootShuffling: AbsoluteAngularVelocityLowerBody > 45 and
//!         AnimLOD1 == 1 -> the TurnRt/Lt90 loops; AnimLOD1 is 0 on a dedicated server, so never there) else
//!         BlendListByBool (696, bIsCurrentLowerA): SequencePlayer_4 / _5 = LowerBodyAnimationA / B = the main
//!         equipment's LowerAnimation (UMordhauAnimInstance::UpdateEquipmentData rva=0x151c4d0, decomp 6740-6812;
//!         SecondLowerAnimation in alternate mode, the shield's ShieldLowerAnimation with a left-hand shield), looping;
//!         then the DirectionOffsetSlow hips / feet nodes (666, 661, 673..674) and the feet IK offsets (678, 679).
//!     [1] moving (682): BlendListByBool_1 (688, bUseBackBlendSpace, 0.26 s): BlendSpacePlayer_3 / _2 =
//!         BS_LowerBodyLocomotion_Back / _Front at (DirectionWithOffset, ThirdPersonVelocity), rate MovementAnimRate;
//!         TwoWayBlend (684) with the 1P space at IsFirstPersonFloat (0 in third person / on the server); SpeedWarping
//!         and the feet TwoBoneIK (662, 663); ModifyBone_20 Hips (DirectionOffsetSlow).
//!   then ModifyBone_21 Hips + Helper_LBCrouchOffsetInverse, ModifyBone_19 Hips translation BMM_Replace (0, 0, 100)
//!   in component space at Alpha Helper_LBHipsZOverrideAlpha = min(IsDedicatedServer + IsFirstPersonFloat, 1) *
//!   (1 - FastSmoothedIsCrouching) (UpdateBlueprintHelpers rva=0x151a2b0, decomp 4578-4582), and the 1P-only leg
//!   offsets (alpha IsFirstPersonFloat).
//! Anim instance (NativeUpdateAnimation rva=0x1501930): IsDedicatedServer = 1 on a dedicated server, with AnimLOD0 =
//! AnimLOD1 = 0 (decomp 500-506); Velocity (anim) = 0 when the 2D speed <= 1 (decomp 2496-2499);
//! ThirdPersonVelocity = clamp((speed - 240) / 110, 0, 1) * (1 - crouch) (2592-2604); DirectionWithOffset =
//! normalize(Direction + DirectionOffset) with bUseBackBlendSpace set above |100|, cleared below |80| (2418-2433).
//! The Sim is the authority, which in multiplayer is the dedicated server: `dedicated_server` (default true) is the
//! pose the server traces with.
//! Not ported (UNCONFIRMED): the DirectionOffsetSlow helper nodes (661, 666, 670-675, 678/679, 692); DirectionOffset /
//! DirectionWithOffset are ported (EVD_MOV_019). Ported: MovementAnimRate / SpeedWarping
//! (EVD_MOV_005) and the SpeedWarping node 664 + leg IK 663 / 662 (EVD_MOV_016, fn speed_warp), the feet IK, crouching. fp-anim r1: the players' weight smoothing, the Locomotion sync group
//! (normalized time, `sync`; the upper body follows it) and the 1P branch (BS_LowerBodyLocomotion_1P) are ported.

use crate::animgraph::{alpha_blend, blend_transforms, AnimAssets};
use crate::pose::Skeleton;
use mordhau_core::data::Spec;
use mordhau_core::ue::{FTransform, FVector};
use std::rc::Rc;

pub const BS_FRONT: &str = "Mordhau/Content/Mordhau/Animations/BlendSpaces/LowerBody/BS_LowerBodyLocomotion_Front";
pub const BS_BACK: &str = "Mordhau/Content/Mordhau/Animations/BlendSpaces/LowerBody/BS_LowerBodyLocomotion_Back";
/// fp-anim r1: AB_MordhauCharacterAnimation BlendSpacePlayer_1 (node 697, TwoWayBlend 684's B pose, Alpha
/// IsFirstPersonFloat; X Direction, Y Velocity, PlayRate MovementSpeedScale: PropertyAccessLibrary copies)
/// fp-anim r2: AB_MordhauCharacterAnimation CDO AnimGraphNode_ModifyBone_31 / _32 Translation Y (VB Position_LeftFoot /
/// _RightFoot) and AnimGraphNode_ModifyBone_22 / _23 Translation Y (LeftUpLeg / RightUpLeg)
pub const VB_FOOT_OFFSET_1P: f32 = 25.0;
pub const UP_LEG_OFFSET_1P: f32 = 5.0;
pub const BS_1P: &str = "Mordhau/Content/Mordhau/Animations/BlendSpaces/LowerBody/BS_LowerBodyLocomotion_1P";

/// a BlendListByBool's crossfade state (FAnimNode_BlendListBase, engine: StandardBlend, HermiteCubic)
#[derive(Clone, Debug, Default)]
pub struct BoolBlend {
    pub active: Option<bool>,
    pub switched_at: f64,
    pub dur: f64,
    /// the weight of the inactive pose when the switch happened
    pub from_w: f64,
}

impl BoolBlend {
    /// set this frame's value; returns the weight of `true`'s pose
    pub fn update(&mut self, spec: &Spec, v: bool, now: f64, t_true: f64, t_false: f64) -> f64 {
        match self.active {
            None => {
                self.active = Some(v);
                self.switched_at = now;
                self.dur = 0.0;
                self.from_w = 0.0;
            }
            Some(a) if a != v => {
                let w = self.weight_active(spec, now);
                self.active = Some(v);
                self.switched_at = now;
                self.dur = if v { t_true } else { t_false };
                self.from_w = 1.0 - w;
            }
            _ => {}
        }
        let w = self.weight_active(spec, now);
        if self.active == Some(true) { w } else { 1.0 - w }
    }
    fn weight_active(&self, spec: &Spec, now: f64) -> f64 {
        if self.dur <= 0.0 {
            return 1.0;
        }
        let a = alpha_blend(spec, 2, ((now - self.switched_at) / self.dur).clamp(0.0, 1.0), "");
        self.from_w + (1.0 - self.from_w) * a
    }
}

#[derive(Clone, Debug)]
pub struct LowerBody {
    pub dedicated_server: bool,
    pub idle: String,
    pub front: Option<Rc<crate::blendspace::BlendSpace>>,
    pub back: Option<Rc<crate::blendspace::BlendSpace>>,
    pub still: BoolBlend,
    pub use_back: bool,
    pub back_blend: BoolBlend,
    /// (2D speed cm/s, Direction deg) of this frame (Sim::anim_inputs)
    pub input: (f64, f64),
    /// the notifies the dominant lower-body clip crossed this frame (FAnimNotifyEvent names; drained by the Sim)
    pub notifies: Vec<String>,
    last_t: Option<f64>,
    /// fp-anim r1: bIsFirstPerson (TwoWayBlend 684 Alpha IsFirstPersonFloat), the anim instance's Velocity (the 1P
    /// quantized 0 / 60 / 90, Sim::upper_bs_input) and MovementSpeedScale (BlendSpacePlayer_1 PlayRate)
    pub first_person: bool,
    pub velocity_1p: f64,
    pub movement_speed_scale: f64,
    pub fp: Option<Rc<crate::blendspace::BlendSpace>>,
    /// the blend-space players' smoothed weights (front, back, 1P) and the Locomotion sync group this machine leads
    pub players: [crate::blendspace::BsPlayer; 3],
    pub sync: crate::blendspace::SyncGroup,
    last_now: Option<f64>,
    /// fp-anim r2: VB Position_LeftFoot / _RightFoot in component space after the Ground state (virtual bones: the
    /// sequence's foot location, + ModifyBone_31 / _32's (0, 25, 0) in first person), for TwoBoneIK_7 / _8
    pub vb_feet: Option<(FVector, FVector)>,
    /// EVD_MOV_005: UMordhauAnimInstance MovementAnimRate / SpeedWarping (NativeUpdateAnimation decomp 2530-2611; not
    /// set by the ctor, so 0 until the first FInterpConstantTo) and the "Speed" anim curve of the last evaluated pose
    /// (UAnimInstance::GetCurveValue reads the previous evaluation; only the lower-body locomotion clips carry it, the
    /// upper-body clips have no Speed curve, so LayeredBoneBlend_1's Override keeps the lower body's)
    pub movement_anim_rate: f64,
    pub speed_warping: f64,
    pub speed_curve: f64,
    /// UMordhauAnimInstance AnimatedMovementDirectionInCompSpace = (-MovementY, MovementX, 0) of the last evaluated pose,
    /// clamped to unit length (NativeUpdateAnimation decomp 2264-2287); the SpeedWarping node's Axis
    pub movement_dir: (f64, f64),
    /// FAnimNode_SpeedWarping (node 664) state: HipsZOffset and its FFloatSpringState
    pub hips_z_offset: f32,
    pub hips_spring: (f32, f32),
    /// SmoothedVelocity / SmoothedVelocityChangeVelocity / OneToZeroAtWalkSpeed (NativeUpdateAnimation decomp 2612-2626:
    /// FSmoothDamp toward the anim Velocity, smooth time 0.25, only while they differ; OneToZero = 1 - clamp(Smoothed /
    /// 60); ctor OneToZero 1, decomp 7038)
    pub smoothed_velocity: f64,
    pub smoothed_velocity_change: f64,
    pub one_to_zero_at_walk_speed: f64,
    /// EVD_MOV_019 DirectionOffset (NativeUpdateAnimation decomp 2030-2112): CurrentLowerBodyIdleOffset FInterpTo (speed 5) to
    /// the lower animation's UMordhauAnimMetaData ParamA (UpdateEquipmentData decomp 6812-6841; greatsword 33);
    /// DirectionOffset = clamp(IdleOffset x OneToZeroAtWalkSpeed - Spine1 component yaw (last evaluated pose), -90, 90) x
    /// min(sum of montage instance weights, 1) (x 1 while an AdditiveOverrideType is set), 0 in first person;
    /// DirectionOffsetSlow = FInterpTo(DirectionOffset, DirectionOffsetSlowInterpSpeed 15). The Sim sets the inputs.
    pub idle_offset_target: f64,
    pub current_idle_offset: f64,
    pub spine1_yaw: f64,
    pub montage_weight: f64,
    pub additive_override_active: bool,
    pub direction_offset: f64,
    pub direction_offset_slow: f64,
    /// EVD_CAM_021 (state/proofs/crouch_anim.md): bIsCrouching (ACharacter +0x330 bit 0, the movement's is_crouched),
    /// UnclampedFastSmoothedIsCrouching / FastSmoothedIsCrouching (NativeUpdateAnimation rva=0x1501930 decomp
    /// 1880-1918: FloatSpringInterp toward bIsCrouching with CrouchSpringStiffness 15 / UncrouchSpringStiffness 30,
    /// damping 0.9, mass 1.1, clamped to CrouchSpringLimits (-1.25, 1.25), FInterpConstantTo at ServerCrouchSpeed 4 when
    /// DeltaTime > 0.1; Fast = clamp01) and the spring state
    pub crouching: bool,
    pub crouch_unclamped: f64,
    pub crouch_fast: f64,
    pub crouch_spring: (f32, f32),
    /// EVD_CAM_022 (state/proofs/foot_shuffle.md): the standing turn's foot shuffle. lb_yaw = the mesh yaw +
    /// LowerBodyRotationOffset (the Sim sets it); AngularVelocityLowerBody / AbsoluteAngularVelocityLowerBody =
    /// FSmoothDamp toward clamp(delta(lb_yaw) / dt, +-360) (its absolute value) over AngularVelocityLowerBodyWindow 0.1
    /// (NativeUpdateAnimation rva=0x1501930 decomp 2190-2214); bHelper_LBFootShuffling = Absolute > 45 and AnimLOD1 == 1,
    /// Helper_LBFootShufflingPlayRate = clamp((|AngVel| - 45) / 225, 0, 1) x 2.5 + 0.5, bHelper_LBFootShufflingRight =
    /// AngVel > 0 (UpdateBlueprintHelpers rva=0x151a2b0 decomp 4507-4524); graph: the standing branch's
    /// BlendListByBool_4 (665, blend 0.3 into / 0.4 out of the shuffle) picks BlendListByBool_3 (667, 0.2 s, right ->
    /// SequencePlayer_6 TurnRt90_LoopProper, else SequencePlayer_7 TurnLt90_LoopProper, both looping in the Turning
    /// sync group at the helper play rate) over the LowerAnimation loop
    pub lb_yaw: f64,
    pub lb_yaw_prev: Option<f64>,
    pub anim_lod1: bool,
    pub ang_vel_lb: f64,
    pub ang_vel_lb_cv: f64,
    pub abs_ang_vel_lb: f64,
    pub abs_ang_vel_lb_cv: f64,
    pub shuffle_blend: BoolBlend,
    pub shuffle_right_blend: BoolBlend,
    pub turning_time: f64,
}

pub const TURN_RT_LOOP: &str = "Mordhau/Content/Mordhau/Animations/RawClips/Locomotion/Turning/TurnRt90_LoopProper";
pub const TURN_LT_LOOP: &str = "Mordhau/Content/Mordhau/Animations/RawClips/Locomotion/Turning/TurnLt90_LoopProper";

/// EVD_CAM_021: one update of UnclampedFastSmoothedIsCrouching (decomp 1880-1918; AB_MordhauCharacterAnimation CDO
/// ServerCrouchSpeed 4, CrouchSpringLimits (-1.25, 1.25), CrouchSpringStiffness 15, UncrouchSpringStiffness 30,
/// CrouchSpringDamping 0.9, CrouchSpringMass 1.1): returns (unclamped, fast)
pub fn crouch_step(unclamped: f64, spring: &mut (f32, f32), crouching: bool, dt: f64) -> (f64, f64) {
    let target = if crouching { 1.0 } else { 0.0 };
    let v = if dt > 0.1 {
        finterp_constant_to(unclamped, target, dt, 4.0)
    } else {
        let k = if crouching { 15.0 } else { 30.0 };
        crate::procedural::float_spring_interp(unclamped as f32, target as f32, spring, k, 0.9, dt as f32, 1.1) as f64
    };
    let v = v.clamp(-1.25, 1.25);
    (v, v.clamp(0.0, 1.0))
}

/// UMordhauUtilityLibrary::FSmoothDamp rva=0x161c9b0 (the critically damped smooth damp: omega = 2 / SmoothTime, the
/// 0.48 / 0.235 polynomial for exp, change clamped to MaxSpeed x SmoothTime, overshoot snaps to the target)
pub fn fsmooth_damp(cur: &mut f64, target: f64, vel: &mut f64, smooth_time: f64, dt: f64, max_speed: f64) {
    if dt < 0.0001 {
        return;
    }
    let st = smooth_time.max(0.0001);
    let max_change = st * max_speed;
    let omega = 1.0 / st + 1.0 / st;
    let x = omega * dt;
    let exp = 1.0 / (((x * 0.235 + 0.48) * x + 1.0) * x + 1.0);
    let change = (*cur - target).clamp(-max_change, max_change);
    let temp = (change * omega + *vel) * dt;
    let mut out = (change + temp) * exp + (*cur - change);
    *vel = (*vel - temp * omega) * exp;
    if (0.0 < target - *cur) == (target < out) {
        *vel = 0.0;
        out = target;
    }
    *cur = out;
}

/// FMath::FInterpTo
pub fn finterp_to(cur: f64, target: f64, dt: f64, speed: f64) -> f64 {
    if speed <= 0.0 {
        return target;
    }
    let dist = target - cur;
    if dist * dist < 1e-8 {
        return target;
    }
    cur + dist * (dt * speed).clamp(0.0, 1.0)
}

/// the lower animation's MetaData[0] UMordhauAnimMetaData ParamA (+0x48), 0 without one (UpdateEquipmentData decomp 6812-6841)
fn idle_offset_param(a: &AnimAssets, seq: &str) -> f64 {
    if seq.is_empty() {
        return 0.0;
    }
    a.rd.read(seq)
        .and_then(|ex| ex.iter().find(|x| x["Type"].as_str() == Some("MordhauAnimMetaData")).and_then(|m| m["Properties"]["ParamA"].as_f64()))
        .unwrap_or(0.0)
}

/// FMath::FInterpConstantTo
pub fn finterp_constant_to(cur: f64, target: f64, dt: f64, speed: f64) -> f64 {
    let dist = target - cur;
    if dist * dist < 1e-8 {
        return target;
    }
    let step = speed * dt;
    cur + dist.clamp(-step, step)
}

/// NativeUpdateAnimation decomp 2538-2611: AnimSpeed = max(GetCurveValue("Speed"), 5) x mesh RelativeScale3D.Y x
/// (IsDwarfFloat == 1 ? DwarfSlowerAnimSpeedFactor 0.8 : 1); r = speed / AnimSpeed; rate = r > 1 ? 1 + 0.15 x
/// clamp(2r - 2, 0, 1) : 0.85 + 0.15 x clamp(2r - 1, 0, 1); warping target = speed / (rate x AnimSpeed), 1 in first
/// person, capped at 4. Returns (rate target, warping target). Mesh scale taken as 1 (CharacterMesh0 default scale).
pub fn movement_anim_rate_targets(speed: f64, speed_curve: f64, first_person: bool) -> (f64, f64) {
    let anim_speed = speed_curve.max(5.0);
    let r = speed / anim_speed;
    let rate = if r > 1.0 { (2.0 * r - 2.0).clamp(0.0, 1.0) * 0.14999998 + 1.0 } else { (2.0 * r - 1.0).clamp(0.0, 1.0) * 0.14999998 + 0.85 };
    let warp = if first_person { 1.0 } else { speed / (rate * anim_speed) };
    (rate, warp.min(4.0))
}

impl LowerBody {
    pub fn new(a: &AnimAssets, idle: &str) -> LowerBody {
        LowerBody {
            dedicated_server: true,
            idle: idle.to_string(),
            front: a.blend_space(BS_FRONT),
            back: a.blend_space(BS_BACK),
            still: BoolBlend::default(),
            use_back: false,
            back_blend: BoolBlend::default(),
            input: (0.0, 0.0),
            notifies: Vec::new(),
            last_t: None,
            first_person: false,
            velocity_1p: 0.0,
            movement_speed_scale: 1.0,
            fp: a.blend_space(BS_1P),
            players: Default::default(),
            sync: Default::default(),
            last_now: None,
            vb_feet: None,
            movement_anim_rate: 0.0,
            speed_warping: 0.0,
            speed_curve: 0.0,
            movement_dir: (0.0, 0.0),
            hips_z_offset: 0.0,
            hips_spring: (0.0, 0.0),
            smoothed_velocity: 0.0,
            smoothed_velocity_change: 0.0,
            one_to_zero_at_walk_speed: 1.0,
            idle_offset_target: idle_offset_param(a, idle),
            current_idle_offset: 0.0,
            spine1_yaw: 0.0,
            montage_weight: 0.0,
            additive_override_active: false,
            direction_offset: 0.0,
            direction_offset_slow: 0.0,
            crouching: false,
            crouch_unclamped: 0.0,
            crouch_fast: 0.0,
            crouch_spring: (0.0, 0.0),
            lb_yaw: 0.0,
            lb_yaw_prev: None,
            anim_lod1: true,
            ang_vel_lb: 0.0,
            ang_vel_lb_cv: 0.0,
            abs_ang_vel_lb: 0.0,
            abs_ang_vel_lb_cv: 0.0,
            shuffle_blend: BoolBlend::default(),
            shuffle_right_blend: BoolBlend::default(),
            turning_time: 0.0,
        }
    }

    /// FAnimNode_SpeedWarping::EvaluateSkeletalControl_AnyThread rva=0x146c860 (node 664: Hips; LeftLegTarget / LeftFoot
    /// = VB Position_LeftFoot, the right likewise, here the feet's own component transforms; TotalLegLength 90.76,
    /// spring 200 / 0.5 / 1, HipsOffsetRemapIn (-110, 0) -> Out (-20, 0), HipsOffsetClamp (-20, 20); Speed =
    /// SpeedWarping, Axis = AnimatedMovementDirectionInCompSpace; UpdateInternal rva=0x14a9760 stores DeltaTime), then
    /// TwoBoneIK 663 / 662 (LeftFoot / RightFoot to the warped targets, joint target (0, -92.38372, 38.27853) in UpLeg
    /// bone space, no stretching, bMaintainEffectorRelRot: the foot keeps its rotation relative to the shin). Each foot's offset from its
    /// UpLeg is scaled along Axis by Speed; above Speed 1 the horizontal reach is clamped to TotalLegLength and the
    /// hips target is the lower of the two legs' heights, remapped; HipsZOffset springs to it (dt capped 0.05) and is
    /// clamped; the UpLegs move by it and each foot is pulled back within its original leg length.
    fn speed_warp(&mut self, sk: &Skeleton, pose: Vec<FTransform>, dt: f32) -> Vec<FTransform> {
        let f = |n: &str| sk.find(n);
        let (Some(hips), Some(lu), Some(lf), Some(ru), Some(rf)) = (f("Hips"), f("LeftUpLeg"), f("LeftFoot"), f("RightUpLeg"), f("RightFoot")) else { return pose };
        let mut cp = crate::procedural::CsPose { sk, local: pose };
        let (pr, pl) = (cp.cs(rf).loc, cp.cs(lf).loc);
        let (ur, ul) = (cp.cs(ru).loc, cp.cs(lu).loc);
        let (len_r, len_l) = ((ur - pr).length(), (ul - pl).length());
        let speed = self.speed_warping as f32;
        let axis = FVector::new(self.movement_dir.0 as f32, self.movement_dir.1 as f32, 0.0);
        let (mut fr, mut fl) = (pr, pl);
        let mut target = 0.0f32;
        let k = speed - 1.0;
        if len_l > 1e-8 && len_r > 1e-8 && k.abs() > 1e-8 && (axis.x.abs() > 1e-4 || axis.y.abs() > 1e-4 || axis.z.abs() > 1e-4) {
            let inv = 1.0 / (axis.x * axis.x + axis.y * axis.y + axis.z * axis.z);
            let warp = |p: FVector, u: FVector| {
                let d = ((p.x - u.x) * axis.x + (p.y - u.y) * axis.y + (p.z - u.z) * axis.z) * inv;
                FVector::new(p.x + d * axis.x * k, p.y + d * axis.y * k, p.z + d * axis.z * k)
            };
            fr = warp(fr, ur);
            fl = warp(fl, ul);
            if speed > 1.0 {
                let l = 90.76f32;
                let reach = |p: FVector, u: FVector| {
                    let (mut dx, mut dy) = (p.x - u.x, p.y - u.y);
                    if l >= 1e-4 {
                        let d2 = dx * dx + dy * dy;
                        if l * l < d2 {
                            let s = l / d2.sqrt();
                            dx *= s;
                            dy *= s;
                        }
                    } else {
                        dx = 0.0;
                        dy = 0.0;
                    }
                    FVector::new(u.x + dx, u.y + dy, p.z)
                };
                fr = reach(fr, ur);
                fl = reach(fl, ul);
                let height = |p: FVector, u: FVector| {
                    let h2 = l * l - ((u.x - p.x) * (u.x - p.x) + (u.y - p.y) * (u.y - p.y));
                    p.z + if h2 > 1e-5 { h2.sqrt() } else { 0.0 } - u.z
                };
                let z = height(fr, ur).min(height(fl, ul));
                let (in0, in1, out0, out1) = (-110.0f32, 0.0f32, -20.0f32, 0.0f32);
                let a = if (in1 - in0).abs() > 1e-8 { ((z - in0) / (in1 - in0)).clamp(0.0, 1.0) } else if z < in1 { 0.0 } else { 1.0 };
                target = out0 + (out1 - out0) * a;
            }
        }
        let z = crate::procedural::float_spring_interp(self.hips_z_offset, target, &mut self.hips_spring, 200.0, 0.5, dt.min(0.05), 1.0);
        let z = z.clamp(-20.0, 20.0);
        self.hips_z_offset = z;
        let lift = FVector::new(0.0, 0.0, z);
        let pull = |p: FVector, u: FVector, len: f32| {
            let u2 = u + lift;
            let d = p - u2;
            let dl = d.length();
            if len >= 1e-4 && dl > len { u2 + d.scale((len / dl) as f64) } else { p }
        };
        let (tr, tl) = (pull(fr, ur, len_r), pull(fl, ul, len_l));
        // Hips raised / lowered by HipsZOffset (component space)
        let mut h = cp.cs(hips);
        h.loc = h.loc + lift;
        cp.blend_cs(hips, h, 1.0);
        // TwoBoneIK 663 (left) then 662 (right)
        for (foot, up, t) in [(lf, lu, tl), (rf, ru, tr)] {
            let j = sk.parents[foot];
            if j < 0 || sk.parents[j as usize] != up as i32 {
                continue;
            }
            let j = j as usize;
            let (r0, f0) = (cp.cs(up), cp.cs(foot));
            let jt = FTransform::new(mordhau_core::ue::FQuat::IDENTITY, FVector::new(0.0, -92.38372, 38.27853)).then(&r0).loc;
            let j0 = cp.cs(j);
            let (r1, j1, _) = crate::procedural::solve_two_bone_ik(r0, j0, f0, jt, t);
            // bMaintainEffectorRelRot (node data true): EndBoneCSTransform = EndBoneLocalTransform * LowerLimbCSTransform
            // (FAnimNode_TwoBoneIK), the foot keeps its rotation relative to the shin
            let e1 = f0.then(&j0.inverse()).then(&j1);
            cp.blend_cs(up, r1, 1.0);
            cp.blend_cs(j, j1, 1.0);
            cp.blend_cs(foot, e1, 1.0);
        }
        cp.local
    }

    fn blend(sk: &Skeleton, a: &[FTransform], b: &[FTransform], wb: f64) -> Vec<FTransform> {
        if wb <= 1e-5 {
            return a.to_vec();
        }
        if wb >= 1.0 - 1e-5 {
            return b.to_vec();
        }
        (0..sk.ref_local.len()).map(|i| blend_transforms(&[a[i], b[i]], &[1.0 - wb, wb])).collect()
    }

    /// the anim notifies of the dominant lower-body clip between the last frame and `now` (the clip plays looping
    /// on its own clock; UNCONFIRMED: only the highest-weighted blend-space sample triggers, the engine's notify
    /// mode is not read). The foot notifies are LeftFootLanded / RightFootLanded on the locomotion clips.
    fn collect_notifies(&mut self, a: &AnimAssets, clip: &str, now: f64) {
        let prev = self.last_t.replace(now);
        let Some(prev) = prev else { return };
        let Some(c) = a.clip(clip) else { return };
        let len = c.sequence_length as f64;
        if len <= 0.0 || now <= prev {
            return;
        }
        let (t0, t1) = (prev.rem_euclid(len), now.rem_euclid(len));
        for (name, t) in a.notifies(clip).iter() {
            let crossed = if t0 <= t1 { *t > t0 && *t <= t1 } else { *t > t0 || *t <= t1 };
            if crossed {
                self.notifies.push(name.clone());
            }
        }
    }

    /// the Ground state's local pose at `now`
    pub fn pose(&mut self, spec: &Spec, sk: &Skeleton, a: &AnimAssets, now: f64) -> Vec<FTransform> {
        let (speed, direction) = self.input;
        // EVD_MOV_019: DirectionWithOffset = normalize(Direction + DirectionOffset) (decomp 2420-2426); DirectionOffset is
        // last frame's here (updated below, before the graph, from this frame's inputs: the order of decomp 2044-2112)
        let dir = {
            let x = (self.direction_offset + direction).rem_euclid(360.0);
            if x > 180.0 { x - 360.0 } else { x }
        };
        // the dominant clip of this frame (notifies)
        let dom = if !(speed > 1.0) {
            self.idle.clone()
        } else {
            let y = ((speed - 240.0) * 0.009090909).clamp(0.0, 1.0) * (1.0 - self.crouch_fast);
            let bs = if self.use_back { self.back.clone() } else { self.front.clone() };
            bs.and_then(|b| b.weights(dir, y).into_iter().max_by(|x, z| x.1.partial_cmp(&z.1).unwrap()).and_then(|(k, _)| b.samples.get(k).map(|s| s.0.clone())))
                .unwrap_or_default()
        };
        if !dom.is_empty() {
            self.collect_notifies(a, &dom, now);
        }
        let still = !(speed > 1.0);
        let w_still = self.still.update(spec, still, now, 0.3, 0.3);
        let dt = self.last_now.map(|l| (now - l).max(0.0)).unwrap_or(0.0);
        self.last_now = Some(now);
        // EVD_CAM_021: the crouch smoothing (decomp 1880-1918), before the helpers that read it
        {
            let (u, f) = crouch_step(self.crouch_unclamped, &mut self.crouch_spring, self.crouching, dt);
            self.crouch_unclamped = u;
            self.crouch_fast = f;
        }
        // EVD_MOV_005: the anim instance's MovementAnimRate / SpeedWarping from last frame's Speed curve (FInterpConstantTo
        // speed 5), read by BlendSpacePlayer_3 / _2 PlayRate and the SpeedWarping node below
        {
            self.current_idle_offset = finterp_to(self.current_idle_offset, self.idle_offset_target, dt, 5.0);
            let yaw = (self.current_idle_offset * self.one_to_zero_at_walk_speed - self.spine1_yaw).clamp(-90.0, 90.0);
            let w = if self.additive_override_active { 1.0 } else { self.montage_weight.min(1.0) };
            self.direction_offset = if self.first_person { 0.0 } else { yaw * w };
            self.direction_offset_slow = finterp_to(self.direction_offset_slow, self.direction_offset, dt, 15.0);
        }
        let (rate_t, warp_t) = movement_anim_rate_targets(speed, self.speed_curve, self.first_person);
        self.movement_anim_rate = finterp_constant_to(self.movement_anim_rate, rate_t, dt, 5.0);
        self.speed_warping = finterp_constant_to(self.speed_warping, warp_t, dt, 5.0);
        if self.velocity_1p != self.smoothed_velocity {
            fsmooth_damp(&mut self.smoothed_velocity, self.velocity_1p, &mut self.smoothed_velocity_change, 0.25, dt, 1e12);
            self.one_to_zero_at_walk_speed = 1.0 - (self.smoothed_velocity * 0.016666668).clamp(0.0, 1.0);
        }
        // bUseBackBlendSpace hysteresis (decomp 2418-2433)
        if !self.use_back && dir.abs() > 100.0 {
            self.use_back = true;
        } else if self.use_back && dir.abs() < 80.0 {
            self.use_back = false;
        }
        // ThirdPersonVelocity = clamp((speed - 240) / 110, 0, 1) x (1 - FastSmoothedIsCrouching) (decomp 2592-2604)
        let y = ((speed - 240.0) * 0.009090909).clamp(0.0, 1.0) * (1.0 - self.crouch_fast);
        let wb = self.back_blend.update(spec, self.use_back, now, 0.26, 0.26);
        // fp-anim r1: the players tick every frame (their weights smooth at TargetWeightInterpolationSpeedPerSec)
        let wf = self.front.clone().map(|b| self.players[0].update(&b, dir, y, dt)).unwrap_or_default();
        let wbk = self.back.clone().map(|b| self.players[1].update(&b, dir, y, dt)).unwrap_or_default();
        let w1 = self.fp.clone().map(|b| self.players[2].update(&b, dir, self.velocity_1p, dt)).unwrap_or_default();
        // the Locomotion sync group's leader = the highest-weighted CanBeLeader player: the LowerAnimation sequence
        // (SequencePlayer_4 / _5) while standing, else the moving branch's blend space (1P: BlendSpacePlayer_1 at
        // MovementSpeedScale; 3P: Front / Back at MovementAnimRate, EVD_MOV_005)
        let (len, rate) = if w_still >= 0.5 {
            (a.play_length(&self.idle), 1.0)
        } else if self.first_person {
            (self.fp.as_ref().map(|b| a.blend_space_length(b, &w1)).unwrap_or(0.0), self.movement_speed_scale)
        } else if wb >= 0.5 {
            (self.back.as_ref().map(|b| a.blend_space_length(b, &wbk)).unwrap_or(0.0), self.movement_anim_rate)
        } else {
            (self.front.as_ref().map(|b| a.blend_space_length(b, &wf)).unwrap_or(0.0), self.movement_anim_rate)
        };
        self.sync.advance(dt, rate, len);
        let norm = self.sync.norm;
        // the Speed curve of this evaluation (weights as the pose blend below; a clip without the curve counts as 0)
        {
            let lerp_bs = |bs: &Option<Rc<crate::blendspace::BlendSpace>>, ws: &[(usize, f64)], name: &str| -> f64 {
                let Some(b) = bs else { return 0.0 };
                let tot: f64 = ws.iter().map(|x| x.1).sum();
                if tot <= 0.0 {
                    return 0.0;
                }
                ws.iter()
                    .filter_map(|(k, w)| {
                        let s = &b.samples.get(*k)?.0;
                        let len = a.clip(s)?.sequence_length as f64;
                        Some(a.curve_value(spec, s, name, norm.rem_euclid(1.0) * len).unwrap_or(0.0) * w / tot)
                    })
                    .sum()
            };
            let curve = |name: &str| {
                let idle_c = a.clip(&self.idle).map(|c| a.curve_value(spec, &self.idle, name, norm.rem_euclid(1.0) * c.sequence_length as f64).unwrap_or(0.0)).unwrap_or(0.0);
                let moving_c = if self.first_person && self.fp.is_some() { lerp_bs(&self.fp, &w1, name) } else { lerp_bs(&self.front, &wf, name) * (1.0 - wb) + lerp_bs(&self.back, &wbk, name) * wb };
                idle_c * w_still + moving_c * (1.0 - w_still)
            };
            self.speed_curve = curve("Speed");
            let (mx, my) = (curve("MovementX"), -curve("MovementY"));
            let n2 = mx * mx + my * my;
            self.movement_dir = if n2 <= 1.0 { (my, mx) } else { (my / n2.sqrt(), mx / n2.sqrt()) };
        }
        // EVD_CAM_022: the lower body's angular velocity (decomp 2190-2214) and the foot-shuffle helpers (4507-4524)
        let (w_shuffle, w_right, turning_t) = {
            let d = match self.lb_yaw_prev {
                Some(p) => {
                    let x = (self.lb_yaw - p).rem_euclid(360.0);
                    if x > 180.0 { x - 360.0 } else { x }
                }
                None => 0.0,
            };
            self.lb_yaw_prev = Some(self.lb_yaw);
            let rate = if dt > 1e-8 { (d / dt).clamp(-360.0, 360.0) } else { 0.0 };
            fsmooth_damp(&mut self.ang_vel_lb, rate, &mut self.ang_vel_lb_cv, 0.1, dt, 1e12);
            fsmooth_damp(&mut self.abs_ang_vel_lb, rate.abs(), &mut self.abs_ang_vel_lb_cv, 0.1, dt, 1e12);
            let shuffling = self.abs_ang_vel_lb > 45.0 && self.anim_lod1;
            let play_rate = ((self.ang_vel_lb.abs() - 45.0) / 225.0).clamp(0.0, 1.0) * 2.5 + 0.5;
            let right = self.ang_vel_lb > 0.0;
            let w_sh = self.shuffle_blend.update(spec, shuffling, now, 0.3, 0.4);
            let w_r = self.shuffle_right_blend.update(spec, right, now, 0.2, 0.2);
            // the Turning sync group's clock: both loops are 1.3 s, played at the helper rate
            self.turning_time += dt * play_rate;
            (w_sh, w_r, self.turning_time)
        };
        let idle = || {
            let base = if self.idle.is_empty() {
                sk.ref_local.clone()
            } else {
                let l = a.clip(&self.idle).map(|c| c.sequence_length as f64).unwrap_or(0.0);
                a.sample(sk, &self.idle, norm * l, true)
            };
            if w_shuffle <= 1e-5 {
                return base;
            }
            let loop_of = |path: &str| -> Option<Vec<FTransform>> {
                let c = a.clip(path)?;
                let l = c.sequence_length as f64;
                Some(a.sample(sk, path, if l > 0.0 { turning_t.rem_euclid(l) } else { 0.0 }, true))
            };
            match (loop_of(TURN_RT_LOOP), loop_of(TURN_LT_LOOP)) {
                (Some(rt), Some(lt)) => {
                    let turn = Self::blend(sk, &lt, &rt, w_right);
                    Self::blend(sk, &base, &turn, w_shuffle)
                }
                _ => base,
            }
        };
        let mut pose = if w_still >= 1.0 - 1e-5 {
            idle()
        } else {
            let moving = match (&self.fp, self.first_person) {
                // TwoWayBlend 684 at IsFirstPersonFloat = 1: BlendSpacePlayer_1 alone. Not ported: the 1P leg nodes
                // (ModifyBone_22 / _23 / _31 / _32, TwoBoneIK_7 / _8, Alpha IsFirstPersonFloat)
                (Some(b), true) => a.blend_space_pose_at(sk, b, &w1, norm),
                _ => {
                    let f = self.front.as_ref().map(|b| a.blend_space_pose_at(sk, b, &wf, norm)).unwrap_or_else(|| sk.ref_local.clone());
                    if wb > 1e-5 {
                        let bk = self.back.as_ref().map(|b| a.blend_space_pose_at(sk, b, &wbk, norm)).unwrap_or_else(|| sk.ref_local.clone());
                        Self::blend(sk, &f, &bk, wb)
                    } else {
                        f
                    }
                }
            };
            // node 664 FAnimNode_SpeedWarping then TwoBoneIK 663 / 662 on the moving branch, Alpha IsNotFirstPersonFloat
            let idle_pose = if w_still > 1e-5 { Some(idle()) } else { None };
            let moving = if self.first_person { moving } else { self.speed_warp(sk, moving, dt as f32) };
            match idle_pose { Some(ip) => Self::blend(sk, &moving, &ip, w_still), None => moving }
        };
        // fp-anim r2: the virtual bones VB Position_LeftFoot / _RightFoot (UMA_Master_Skeleton VirtualBones: source
        // Position, targets LeftFoot / RightFoot) take the evaluated feet (CopyBone 660 / 659 copy them again on the
        // moving branch), then ModifyBone_31 / _32 (nodes 656 / 655, last in the Ground state StateResult_2) add
        // (0, 25, 0) in component space at Alpha IsFirstPersonFloat. UNCONFIRMED: the engine's virtual-bone fill from the
        // sequence pose (engine source) is taken for the standing branch too
        self.vb_feet = None;
        if self.first_person {
            if let (Some(lf), Some(rf)) = (sk.find("LeftFoot"), sk.find("RightFoot")) {
                let cs = sk.to_component(&pose);
                let off = FVector::new(0.0, VB_FOOT_OFFSET_1P, 0.0);
                self.vb_feet = Some((cs[lf].loc + off, cs[rf].loc + off));
            }
        }
        // EVD_CAM_021: ModifyBone_21 (node 685, before ModifyBone_19): Hips translation BMM_Additive in component space
        // = Helper_LBCrouchOffsetInverse = -max(Unclamped, 0) x (0, 10, 30 + 20 x OneToZeroAtWalkSpeed) (UpdateBlueprintHelpers
        // rva=0x151a2b0 decomp 4561-4576; the CDO's Helper_LBCrouchOffset.X is 0), Alpha Helper_LBCrouchAlpha = |Unclamped| > 0.001
        if self.crouch_unclamped.abs() > 0.001 {
            if let Some(h) = sk.find("Hips") {
                let u = self.crouch_unclamped.max(0.0) as f32;
                let off = FVector::new(0.0, -10.0 * u, -(30.0 + 20.0 * self.one_to_zero_at_walk_speed as f32) * u);
                let mut cp = crate::procedural::CsPose { sk, local: pose };
                cp.modify_add(h, (0.0, 0.0, 0.0), Some(off), false, 1.0);
                pose = cp.local;
            }
        }
        // ModifyBone_19 (node 695): Hips translation replaced by (0, 0, 100) in component space at Alpha
        // Helper_LBHipsZOverrideAlpha = min(IsDedicatedServer + IsFirstPersonFloat, 1) x (1 - FastSmoothedIsCrouching)
        // (decomp 4578-4582; EVD_CAM_021: a crouching first-person character keeps its lowered hips)
        let hips_alpha = if self.dedicated_server { 1.0 - self.crouch_fast as f32 } else { 0.0 };
        if hips_alpha > 1e-4 {
            if let Some(h) = sk.find("Hips") {
                let p = sk.parents[h];
                let parent_cs = if p >= 0 { sk.to_component(&pose)[p as usize] } else { FTransform::IDENTITY };
                let cs = pose[h].then(&parent_cs);
                let target = FVector::new(0.0, 0.0, 100.0);
                let loc = if hips_alpha >= 1.0 - 1e-4 { target } else { FVector::new(cs.loc.x + (target.x - cs.loc.x) * hips_alpha, cs.loc.y + (target.y - cs.loc.y) * hips_alpha, cs.loc.z + (target.z - cs.loc.z) * hips_alpha) };
                let n = FTransform::new(cs.rot, loc);
                pose[h] = if p >= 0 { n.then(&parent_cs.inverse()) } else { n };
            }
        }
        // fp-anim r2: ModifyBone_22 / _23 (nodes 681 / 680, after ModifyBone_19): LeftUpLeg / RightUpLeg translation
        // (0, 5, 0) BMM_Additive in component space, Alpha IsFirstPersonFloat
        if self.first_person {
            let mut cp = crate::procedural::CsPose { sk, local: pose };
            for b in ["LeftUpLeg", "RightUpLeg"] {
                if let Some(i) = sk.find(b) {
                    cp.modify_add(i, (0.0, 0.0, 0.0), Some(FVector::new(0.0, UP_LEG_OFFSET_1P, 0.0)), false, 1.0);
                }
            }
            pose = cp.local;
        }
        pose
    }
}

#[cfg(test)]
mod tests {
    /// EVD_CAM_021: the crouch spring (stiffness 15 / 30, damping 0.9, mass 1.1) settles near 1 within a second at 60 Hz,
    /// the unclamped value may overshoot (limits -1.25 / 1.25) while Fast stays in [0, 1], and a long frame takes the
    /// constant 4 / s step
    #[test]
    fn crouch_smoothing_settles_and_clamps() {
        let (mut u, mut sp) = (0.0f64, (0.0f32, 0.0f32));
        let mut max_u: f64 = 0.0;
        for _ in 0..60 {
            let (nu, f) = super::crouch_step(u, &mut sp, true, 1.0 / 60.0);
            u = nu;
            max_u = max_u.max(u);
            assert!((0.0..=1.0).contains(&f) && u <= 1.25);
        }
        assert!((u - 1.0).abs() < 0.15, "u {u}");
        assert!(max_u <= 1.25);
        for _ in 0..120 {
            u = super::crouch_step(u, &mut sp, false, 1.0 / 60.0).0;
        }
        assert!(u.abs() < 0.05, "u {u}");
        let (nu, f) = super::crouch_step(0.0, &mut sp, true, 0.2);
        assert!((nu - 0.8).abs() < 1e-9 && (f - 0.8).abs() < 1e-9);
    }
    /// EVD_MOV_005: NativeUpdateAnimation decomp 2538-2611 by hand
    #[test]
    fn movement_anim_rate_matches_the_exe_formula() {
        let f = super::movement_anim_rate_targets;
        // clip authored at 400: running 400 -> r 1 -> rate 1, warp 1
        let (r, w) = f(400.0, 400.0, false);
        assert!((r - 1.0).abs() < 1e-6 && (w - 1.0).abs() < 1e-6);
        // r 1.25 -> 1 + 0.15 x 0.5; r 0.6 -> 0.85 + 0.15 x 0.2
        assert!((f(500.0, 400.0, false).0 - 1.075).abs() < 1e-6);
        assert!((f(240.0, 400.0, false).0 - 0.88).abs() < 1e-6);
        // the curve floor 5, the warp cap 4, warp 1 in first person
        assert!((f(100.0, 0.0, false).1 - 4.0).abs() < 1e-6);
        assert_eq!(f(500.0, 400.0, true).1, 1.0);
    }
}

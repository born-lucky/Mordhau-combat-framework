//! The victim's procedural (cosmetic) flinch: the body jolt on a hit (fidelity-audit r8).
//!
//! Inputs: UMordhauAnimInstance::OnTookDamage rva=0x150f7c0 (decomp UMordhauAnimInstance.cpp 5640-5870), run when the
//! character's LastHitInfo changes. It works in the mesh's component space:
//! - FlinchStartTime = now.
//! - FlinchHitSpineIdx = the first of Hips, LowerBack, Spine, Spine1, Neck, Head whose component-space Z is >= the hit
//!   point's (else 6).
//! - HitDirection = hit point - that bone's world location (the bone index clamped to 5).
//! - With a melee hit (damage type 1) whose weapon is an AMordhauWeapon:
//!   - FlinchTranslationTarget = mesh^-1(normal(LastObservedTraceDirection +0x1c80)) x TranslationNonHipsFactor.
//!   - FlinchHipsTranslationTarget.Z = min(that Z x WeaponDirHipsZFactor, 0).
//!   - For a strike, HitDirection = -LastObservedTraceDirection, and the side sign is -1 for LeftStrike, else 1.
//! - d = CalculateDirection(HitDirection with Z = 0, mesh rotator + 90 yaw).
//! - FlinchRotationTarget = Rotator(Y(d) * R(roll FlinchPitchAmount) * Y(d)^-1) (VectorQuaternionMultiply2 operand
//!   order read off the sign masks).
//! - d is folded into [-90, 90]; for a strike, d = |d| x sign; f = d x FlinchPitchYawFactor / 90; then
//!   Pitch *= 1 - |f|, Yaw = f x FlinchYawAmount + (1 - |f|) Yaw, Roll *= 1 - |f|.
//! Per update: UpdateProceduralFlinch rva=0x151ea30 (decomp 5265-5605): every value R/VInterpTo its target (module
//! `update`), the freeze / blend-in / blend-out phases, the per-spine-index target routing.
//! Apply: the AB_MordhauCharacterAnimation root chain, evaluated in this order (AnimGraphNode indices 26 .. 23,
//! component space, Alpha = Helper_IsAnyFlinchValueNonZero):
//! 1. ModifyBone_194: Hips translation += HipsFlinchTranslation.
//! 2. CopyBone_43 / _42: the feet locations go to VB LeftUpLeg_LeftLeg / VB RightUpLeg_RightLeg.
//! 3. WeightShift (FAnimNode_WeightShift rva=0x147e680): Hips CS rotated by HipsFlinchRotation about Pivot (0,0,0);
//!    the two VB bones keep their CS transforms.
//! 4. ModifyBone_203 / _202: LeftFoot / RightFoot rotation += Helper_HipsFlinchRotationInverse.
//! 5. ModifyBone_201 .. _197: LowerBack, Spine, Spine1, Neck, head, translation + rotation additive.
//! 6. ModifyBone_196 / _195 / _193 / _192: Left / RightShoulder, Left / RightArm rotation += Helper_ArmsShoulderFlinchInverse
//!    = (0, 0, -max(CurrentFlinchSpineRotationsCombined.Roll, 0)) (UpdateBlueprintHelpers decomp 5106-5110).
//! 7. TwoBoneIKOffset_7 / _6: LeftFoot / RightFoot reach VB ..._Leg + Helper_HipsFlinchTranslationInverse.
//!    UNCONFIRMED: the custom node's offset space; taken as component space, no stretching, with the
//!    TwoBoneIKOffset_3 joint target (0, -92.38, 38.28) in UpLeg bone space.
//! Parameters: the AB_MordhauCharacterAnimation CDO (FlinchFreezeBlendInDuration 0.05, ... TranslationNonHipsFactor 7.5)
//! over the UMordhauAnimInstance ctor (FlinchPitchYawFactor 0.5, decomp 7046).
//! Not ported:
//! - the kick branch (Move 4 reads the attacker's +0x398 weapon);
//! - bDisableCosmeticFlinch (no native setter found);
//! - the WorldSettings time dilation test (taken as 1).

use crate::procedural::{rinterp_to, solve_two_bone_ik, vinterp_to, CsPose};
use crate::pose::Skeleton;
use mordhau_core::combat::geometry::quat_rotator;
use mordhau_core::ue::{FQuat, FTransform, FVector};

type Rot = (f32, f32, f32); // (Pitch, Yaw, Roll)

const FREEZE_DUR: f64 = 0.05;
const FREEZE_ROT_SPEED: f32 = 100.0;
const FREEZE_TRANS_SPEED: f32 = 100.0;
const FREEZE_ALPHA: f32 = 0.5;
const BLEND_IN_DUR: f64 = 0.2;
const ROT_IN: f32 = 18.0;
const TRANS_IN: f32 = 10.0;
const ROT_OUT: f32 = 4.0;
const TRANS_OUT: f32 = 4.0;
const ROT_OUT_FAST: f32 = 10.0;
const TRANS_OUT_FAST: f32 = 10.0;
const PITCH_AMOUNT: f32 = -21.0;
const YAW_AMOUNT: f32 = 75.0;
const PITCH_YAW_FACTOR: f32 = 0.5;
const HIPS_Z_FACTOR: f32 = -0.5;
const WEAPON_DIR_HIPS_Z_FACTOR: f32 = 3.0;
const TRANSLATION_NON_HIPS_FACTOR: f32 = 7.5;

const SPINE: [&str; 6] = ["Hips", "LowerBack", "Spine", "Spine1", "Neck", "head"];

#[derive(Clone, Debug, Default)]
pub struct Flinch {
    pub start: f64,
    pub spine_idx: i32,
    pub rot_target: Rot,
    pub trans_target: FVector,
    pub hips_trans_target: FVector,
    pub hips_rot: Rot,
    pub hips_trans_internal: FVector,
    pub hips_trans: FVector,
    pub lower_back: (Rot, FVector),
    pub spine: (Rot, FVector),
    pub spine1: (Rot, FVector),
    pub neck: (Rot, FVector),
    pub head: (Rot, FVector),
    pub combined_roll: f32,
}

fn rq(r: Rot) -> FQuat {
    FQuat::from_rotator(r.0, r.1, r.2)
}
fn add(a: Rot, b: Rot) -> Rot {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}
fn scale(a: Rot, k: f32) -> Rot {
    (a.0 * k, a.1 * k, a.2 * k)
}
/// FRotator::GetInverse = Quaternion().Inverse().Rotator()
fn inverse(a: Rot) -> Rot {
    quat_rotator(rq(a).inverse())
}
fn vs(a: FVector, k: f32) -> FVector {
    FVector::new(a.x * k, a.y * k, a.z * k)
}
fn nonzero_r(a: Rot) -> bool {
    a.0 != 0.0 || a.1 != 0.0 || a.2 != 0.0
}
fn nonzero_v(a: FVector) -> bool {
    a.x != 0.0 || a.y != 0.0 || a.z != 0.0
}

impl Flinch {
    /// OnTookDamage. `cs` = the victim's component-space pose, `mesh` = the mesh component's world transform,
    /// `trace_dir` = the attacker weapon's LastObservedTraceDirection for a melee weapon hit, `mv` = the attack move
    pub fn on_took_damage(&mut self, sk: &Skeleton, cs: &[FTransform], mesh: &FTransform, impact: FVector, trace_dir: Option<FVector>, mv: i64, now: f64) {
        self.start = now;
        let hit_cs = mesh.inverse_apply(impact);
        self.spine_idx = 6;
        for (i, b) in SPINE.iter().enumerate() {
            if let Some(bi) = sk.find(b) {
                if hit_cs.z <= cs[bi].loc.z {
                    self.spine_idx = i as i32;
                    break;
                }
            }
        }
        let sb = sk.find(SPINE[self.spine_idx.min(5) as usize]).map(|b| cs[b].then(mesh).loc).unwrap_or(mesh.loc);
        let mut dir = impact - sb;
        let mut sign = 0.0f32;
        if let Some(t) = trace_dir {
            if mordhau_core::combat::enums::is_strike(mv) {
                dir = FVector::new(-t.x, -t.y, -t.z);
                sign = if mv == 1 { -1.0 } else { 1.0 };
            }
            let l = t.length_squared();
            let n = if l == 1.0 { t } else if l >= 1e-8 { let k = 1.0 / l.sqrt(); FVector::new(t.x * k, t.y * k, t.z * k) } else { FVector::ZERO };
            let local = mesh.rot.inverse().rotate(n);
            self.trans_target = FVector::new(local.x * TRANSLATION_NON_HIPS_FACTOR, local.y * TRANSLATION_NON_HIPS_FACTOR, local.z * TRANSLATION_NON_HIPS_FACTOR);
            self.hips_trans_target.z = (local.z * WEAPON_DIR_HIPS_Z_FACTOR).min(0.0);
        }
        dir.z = 0.0;
        let (mp, my, mr) = quat_rotator(mesh.rot);
        let d = mh_character::uequat::calculate_direction(dir, mp, my + 90.0, mr);
        let q = rq((0.0, d, 0.0)).mul(rq((0.0, 0.0, PITCH_AMOUNT))).mul(rq((0.0, d, 0.0)).inverse());
        let t = quat_rotator(q);
        let mut d = d;
        if d > 90.0 {
            d -= 180.0;
        } else if d < -90.0 {
            d += 180.0;
        }
        if sign != 0.0 {
            d = d.abs() * sign;
        }
        let f = d * PITCH_YAW_FACTOR * 0.011111111;
        let k = 1.0 - f.abs();
        self.rot_target = (k * t.0, f * YAW_AMOUNT + k * t.1, k * t.2);
    }

    /// UpdateProceduralFlinch. `suppress` = the current motion is an attack not in recovery (Stage != 2) or a
    /// UFeintedMotion (decomp 5300-5340)
    pub fn update(&mut self, now: f64, dt: f32, suppress: bool) {
        let mut rs = ROT_OUT;
        let mut ts = TRANS_OUT;
        let mut rot: Rot = (0.0, 0.0, 0.0);
        let mut tr = FVector::ZERO;
        let mut hips_t = FVector::ZERO;
        if suppress {
            rs = ROT_OUT_FAST;
            ts = TRANS_OUT_FAST;
            self.start = 0.0;
        } else if now <= self.start + FREEZE_DUR {
            rs = FREEZE_ROT_SPEED;
            ts = FREEZE_TRANS_SPEED;
            rot = scale(self.rot_target, FREEZE_ALPHA);
            tr = vs(self.trans_target, FREEZE_ALPHA);
            hips_t = vs(self.hips_trans_target, FREEZE_ALPHA);
        } else if now <= self.start + BLEND_IN_DUR {
            rs = ROT_IN;
            ts = TRANS_IN;
            rot = self.rot_target;
            tr = self.trans_target;
            hips_t = self.hips_trans_target;
        }
        let yaw_t: Rot = (0.0, rot.1, 0.0);
        let mut r138: Rot = (rot.0, 0.0, rot.2);
        let inv = inverse(r138);
        let idx = self.spine_idx;
        self.hips_rot = rinterp_to(self.hips_rot, yaw_t, dt, rs);
        self.hips_trans_internal = vinterp_to(self.hips_trans_internal, hips_t, dt, ts);
        if idx < 2 {
            r138 = inv;
        }
        let mut t128 = FVector::ZERO;
        self.lower_back.0 = rinterp_to(self.lower_back.0, add(yaw_t, r138), dt, rs);
        if idx == 2 {
            t128 = tr;
        }
        self.lower_back.1 = vinterp_to(self.lower_back.1, t128, dt, ts);
        if idx == 2 {
            r138 = scale(inv, 2.0);
        }
        self.spine.0 = rinterp_to(self.spine.0, add(yaw_t, r138), dt, rs);
        if idx == 2 {
            t128 = FVector::ZERO;
        } else if idx == 3 {
            t128 = tr;
        }
        self.spine.1 = vinterp_to(self.spine.1, t128, dt, ts);
        if idx == 3 {
            r138 = scale(inv, 2.0);
            if r138.2 > 0.0 {
                r138.2 += r138.2;
            }
        }
        self.spine1.0 = rinterp_to(self.spine1.0, add(yaw_t, r138), dt, rs);
        match idx {
            2 => t128 = FVector::new(-tr.x, -tr.y, -tr.z),
            3 => t128 = FVector::ZERO,
            4 => t128 = tr,
            _ => {}
        }
        self.spine1.1 = vinterp_to(self.spine1.1, t128, dt, ts);
        let yaw2;
        if idx < 4 {
            r138 = inv;
            if idx == 3 {
                if inv.2 < 0.0 {
                    r138.2 = inv.2 * 2.0;
                }
                r138.0 = inv.0 * 2.0;
            }
            yaw2 = inverse(yaw_t);
        } else if idx < 5 {
            yaw2 = inverse(yaw_t);
        } else {
            yaw2 = (0.0, 0.0, 0.0);
        }
        self.neck.0 = rinterp_to(self.neck.0, add(yaw2, r138), dt, rs);
        match idx {
            2 | 4 => t128 = FVector::ZERO,
            3 => t128 = FVector::new(-tr.x, -tr.y, -tr.z),
            5 => t128 = tr,
            _ => {}
        }
        self.neck.1 = vinterp_to(self.neck.1, t128, dt, ts);
        self.head.0 = rinterp_to(self.head.0, add(yaw2, r138), dt, rs);
        match idx {
            3 | 5 => t128 = FVector::ZERO,
            4 => t128 = FVector::new(-tr.x, -tr.y, -tr.z),
            6 => t128 = tr,
            _ => {}
        }
        self.head.1 = vinterp_to(self.head.1, t128, dt, ts);
        self.combined_roll = self.hips_rot.2 + self.lower_back.0 .2 + self.spine.0 .2 + self.spine1.0 .2;
        self.hips_trans = FVector::new(0.0, 0.0, self.hips_rot.2.abs() * HIPS_Z_FACTOR + self.hips_trans_internal.z);
    }

    /// Helper_IsAnyFlinchValueNonZero (UpdateBlueprintHelpers decomp 5040-5095)
    pub fn any(&self) -> bool {
        nonzero_r(self.hips_rot)
            || nonzero_v(self.hips_trans)
            || [self.lower_back, self.spine, self.spine1, self.neck, self.head].iter().any(|(r, t)| nonzero_r(*r) || nonzero_v(*t))
    }

    /// The root chain (module doc) on a component-space pose
    pub fn apply(&self, p: &mut CsPose) {
        if !self.any() {
            return;
        }
        let sk = p.sk;
        let f = |n: &str| sk.find(n);
        // 1. ModifyBone_194
        if let Some(h) = f("Hips") {
            p.modify_add(h, (0.0, 0.0, 0.0), Some(self.hips_trans), false, 1.0);
        }
        // 2. CopyBone_43 / _42 (translation)
        let vb = [("LeftFoot", "VB LeftUpLeg_LeftLeg"), ("RightFoot", "VB RightUpLeg_RightLeg")];
        for (src, dst) in vb {
            if let (Some(s), Some(d)) = (f(src), f(dst)) {
                let mut n = p.cs(d);
                n.loc = p.cs(s).loc;
                p.blend_cs(d, n, 1.0);
            }
        }
        let keep: Vec<(usize, FTransform)> = vb.iter().filter_map(|(_, d)| f(d).map(|i| (i, p.cs(i)))).collect();
        // 3. WeightShift: Hips about the component origin, the VB bones kept
        if let Some(h) = f("Hips") {
            let q = rq(self.hips_rot);
            let c = p.cs(h);
            p.blend_cs(h, FTransform::new(q.mul(c.rot), q.rotate(c.loc)), 1.0);
            for (i, x) in &keep {
                p.blend_cs(*i, *x, 1.0);
            }
        }
        // 4. ModifyBone_203 / _202
        let hinv = inverse(self.hips_rot);
        for b in ["LeftFoot", "RightFoot"] {
            if let Some(i) = f(b) {
                p.modify_add(i, hinv, None, false, 1.0);
            }
        }
        // 5. ModifyBone_201 .. _197
        for (b, (r, t)) in [("LowerBack", self.lower_back), ("Spine", self.spine), ("Spine1", self.spine1), ("Neck", self.neck), ("head", self.head)] {
            if let Some(i) = f(b) {
                p.modify_add(i, r, Some(t), false, 1.0);
            }
        }
        // 6. ModifyBone_196 / _195 / _193 / _192
        let arms: Rot = (0.0, 0.0, -self.combined_roll.max(0.0));
        for b in ["LeftShoulder", "RightShoulder", "LeftArm", "RightArm"] {
            if let Some(i) = f(b) {
                p.modify_add(i, arms, None, false, 1.0);
            }
        }
        // 7. TwoBoneIKOffset_7 / _6
        let off = FVector::new(-self.hips_trans.x, -self.hips_trans.y, -self.hips_trans.z);
        for (foot, vbn) in [("LeftFoot", "VB LeftUpLeg_LeftLeg"), ("RightFoot", "VB RightUpLeg_RightLeg")] {
            let (Some(e), Some(v)) = (f(foot), f(vbn)) else { continue };
            let j = sk.parents[e];
            if j < 0 || sk.parents[j as usize] < 0 {
                continue;
            }
            let (j, r) = (j as usize, sk.parents[j as usize] as usize);
            let (r0, j0, e0) = (p.cs(r), p.cs(j), p.cs(e));
            let eff = p.cs(v).loc + off;
            let jt = FTransform::new(FQuat::IDENTITY, FVector::new(0.0, -92.38372, 38.27853)).then(&r0).loc;
            let (r1, j1, e1) = solve_two_bone_ik(r0, j0, e0, jt, eff);
            p.blend_cs(r, r1, 1.0);
            p.blend_cs(j, j1, 1.0);
            p.blend_cs(e, e1, 1.0);
        }
    }
}

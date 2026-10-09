//! EVD_MOV (sheets/combat/04_movement_evidence.csv): the anim instance's foot grounding, UCreatureAnimInstance
//! (extract/native/decomp UCreatureAnimInstance.cpp, UAdvancedCharacterAnimInstance.cpp):
//!
//! - UAdvancedCharacterAnimInstance::NativeUpdateAnimation rva=0x148aa10 (decomp 223-282): GroundingWeightTarget = 1,
//!   0 when !bWantsGrounding or airborne; PreGrounding(dt, target); |GroundingWeight| > 0.001 -> UpdateGrounding(dt).
//! - UCreatureAnimInstance::PreGrounding rva=0x1512c20 (decomp 500-577): target > 0 with |weight| <= 0.001 resets the
//!   limbs' RotationOffset / InternalTranslationOffset, RootRotationOffset and RootTranslationOffsetInternal and sets
//!   the weight to the target at once; else FInterpTo(weight, target, dt, 10). (LandOffset: AB CDO
//!   bDoNotAddLandOffsetToGrounding = true keeps it out of the offsets; not modelled.)
//! - UCreatureAnimInstance::UpdateGrounding rva=0x151d660 (decomp 8-498): per GroundingLimbs entry (AB CDO: VB
//!   Global_RightFoot, VB Global_LeftFoot; TraceDistance 125, UpValueLimits (0, 50), no TraceStartBone) the last
//!   async trace's hit is read; when the end bone moved more than MinEndBone2DDistanceToRetrace (AB CDO 2) in 2D a new
//!   UWorld::AsyncLineTraceByObjectType (GroundingChannels: ctor WorldStatic + channel 7) runs from the bone + Up x 50
//!   down 125 (its result is read on the next update); with bComputeGroundingRotation (UCreatureAnimInstance ctor
//!   true) each limb's RotationOffset RInterpTo(FindBetween(Up, hit or floor normal) x LimbRotationOffsetFactor
//!   (AB 0.75), RotationInterpSpeed 4) and its root-space impact Z = UnrotateVector(RootRotationOffset, impact -
//!   pivot).Z + pivot.Z; the lowest one minus the capsule bottom (pivot Z = UpdatedComponent Z - Bounds.BoxExtent.Z,
//!   no hit: 0), clamped to RootLiftLimits (AB (-50, 0); crouched (-Y, -X)), MoveTowards by |pivot Z change| when it
//!   moves the same way, then FInterpTo(TranslationInterpSpeed 10) = RootTranslationOffsetInternal.Z;
//!   RootRotationOffset = RInterpTo(FindBetween(Up, floor normal) x RootRotationOffsetFactor 0.5, 4); per limb
//!   offsets FInterpTo(clamp(impact Z - (pivot Z + root Z), UpValueLimits), 10).
//!   RootTranslationOffset = (internal X, internal Y, internal Z).
//! UNCONFIRMED: "VB Global_<Foot>" (no AnimGraph node writes it; the engine fills virtual bones from the sequence pose)
//! is taken as the foot bone of the last evaluated pose; the trace is the Sim's collision world's line trace.

use mordhau_core::combat::geometry::quat_rotator;
use mordhau_core::ue::{FQuat, FVector};

/// AB_MordhauCharacterAnimation CDO / native ctor values (module docs)
pub const TRACE_DISTANCE: f32 = 125.0;
pub const UP_VALUE_LIMITS: (f32, f32) = (0.0, 50.0);
pub const MIN_END_BONE_2D_DISTANCE_TO_RETRACE: f32 = 2.0;
pub const ROOT_LIFT_LIMITS: (f32, f32) = (-50.0, 0.0);
pub const TRANSLATION_INTERP_SPEED: f32 = 10.0;
pub const ROTATION_INTERP_SPEED: f32 = 4.0;
pub const LIMB_ROTATION_OFFSET_FACTOR: f32 = 0.75;
pub const ROOT_ROTATION_OFFSET_FACTOR: f32 = 0.5;

#[derive(Clone, Copy, Debug, Default)]
pub struct Limb {
    /// TraceResult: bBlockingHit, ImpactPoint, ImpactNormal of the last trace issued (async: read the next update)
    pub hit: Option<(FVector, FVector)>,
    /// the trace issued this update (becomes `hit` on the next one)
    pending: Option<Option<(FVector, FVector)>>,
    pub last_end: Option<FVector>,
    pub rotation_offset: (f32, f32, f32),
    pub root_space_impact_z: f32,
    pub internal_z: f32,
    pub translation_z: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Grounding {
    pub weight: f32,
    pub limbs: [Limb; 2],
    pub root_translation_internal: FVector,
    pub root_rotation_offset: (f32, f32, f32),
    pub previous_pivot_z: f32,
    /// RootTranslationOffset (world space)
    pub root_translation_offset: FVector,
}

/// the frame's inputs
pub struct GroundingInput<'a> {
    pub dt: f32,
    pub airborne: bool,
    pub wants_grounding: bool,
    pub crouched: bool,
    /// the mesh's world translation (X, Y) and the capsule bottom (UpdatedComponent Z - Bounds.BoxExtent.Z)
    pub mesh_xy: (f32, f32),
    pub capsule_bottom: f32,
    /// CharacterMovement CurrentFloor: bWalkableFloor and ImpactNormal
    pub floor_normal: Option<FVector>,
    /// the limbs' end bones (world): VB Global_RightFoot, VB Global_LeftFoot
    pub feet: [FVector; 2],
    pub trace: &'a dyn Fn(FVector, FVector) -> Option<(FVector, FVector)>,
}

fn finterp_to(cur: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let d = target - cur;
    if d * d < 1e-8 {
        return target;
    }
    cur + d * (dt * speed).clamp(0.0, 1.0)
}

/// UMordhauUtilityLibrary::MoveTowards rva=0x162e9f0
fn move_towards(cur: f32, target: f32, max: f32) -> f32 {
    let d = target - cur;
    if d.abs() <= max {
        return target;
    }
    if d < 0.0 { cur - max } else if d > 0.0 { cur + max } else { cur }
}

/// FQuat::FindBetweenVectors (engine source) as an FRotator, times `k`
fn between_up(n: FVector, k: f32) -> (f32, f32, f32) {
    let up = FVector::new(0.0, 0.0, 1.0);
    let norm = (n.length_squared() * 1.0).sqrt();
    let w = norm + up.dot(n);
    let q = if w >= 1e-6 * norm {
        let c = FVector::new(up.y * n.z - up.z * n.y, up.z * n.x - up.x * n.z, up.x * n.y - up.y * n.x);
        let l = (c.x * c.x + c.y * c.y + c.z * c.z + w * w).sqrt();
        FQuat { x: c.x / l, y: c.y / l, z: c.z / l, w: w / l }
    } else {
        // opposite vectors: 180 degrees about X (|Up.x| <= |Up.z|)
        FQuat { x: 0.0, y: -1.0, z: 0.0, w: 0.0 }
    };
    let r = quat_rotator(q);
    (r.0 * k, r.1 * k, r.2 * k)
}

impl Grounding {
    /// PreGrounding then UpdateGrounding (module docs)
    pub fn update(&mut self, i: &GroundingInput) {
        let target = if i.wants_grounding && !i.airborne { 1.0 } else { 0.0 };
        if target > 0.0 && self.weight.abs() <= 0.001 {
            for l in self.limbs.iter_mut() {
                l.rotation_offset = (0.0, 0.0, 0.0);
                l.internal_z = 0.0;
            }
            self.root_rotation_offset = (0.0, 0.0, 0.0);
            self.root_translation_internal = FVector::ZERO;
            self.weight = target;
        } else {
            self.weight = finterp_to(self.weight, target, i.dt, 10.0);
        }
        if self.weight.abs() > 0.001 && i.dt != 0.0 {
            self.update_grounding(i);
        }
    }

    fn update_grounding(&mut self, i: &GroundingInput) {
        let dt = i.dt;
        let pivot = FVector::new(i.mesh_xy.0, i.mesh_xy.1, i.capsule_bottom);
        let dz = i.capsule_bottom - self.previous_pivot_z;
        let up = FVector::new(0.0, 0.0, 1.0);
        let floor_normal = if i.airborne { up } else { i.floor_normal.unwrap_or(up) };
        let root_q = FQuat::from_rotator(self.root_rotation_offset.0, self.root_rotation_offset.1, self.root_rotation_offset.2);
        let mut lowest = f32::MAX;
        let mut any = false;
        for (k, l) in self.limbs.iter_mut().enumerate() {
            if !i.airborne {
                // the async trace issued last update is read now
                if let Some(p) = l.pending.take() {
                    l.hit = p;
                }
                let foot = i.feet[k];
                let moved = match l.last_end {
                    Some(e) => ((foot.x - e.x).powi(2) + (foot.y - e.y).powi(2)).sqrt(),
                    None => f32::MAX,
                };
                if MIN_END_BONE_2D_DISTANCE_TO_RETRACE < moved {
                    l.last_end = Some(foot);
                    let start = foot + FVector::new(0.0, 0.0, UP_VALUE_LIMITS.1);
                    let end = start + FVector::new(0.0, 0.0, -TRACE_DISTANCE);
                    l.pending = Some((i.trace)(start, end));
                }
            } else {
                l.hit = None;
            }
            let n = l.hit.map(|h| h.1).unwrap_or(floor_normal);
            let t = between_up(n, LIMB_ROTATION_OFFSET_FACTOR);
            l.rotation_offset = crate::procedural::rinterp_to(l.rotation_offset, t, dt, ROTATION_INTERP_SPEED);
            if let Some((imp, _)) = l.hit {
                l.root_space_impact_z = root_q.inverse().rotate(imp - pivot).z + pivot.z;
                lowest = lowest.min(l.root_space_impact_z);
                any = true;
            }
        }
        if !any {
            lowest = i.capsule_bottom;
        }
        let d = lowest - i.capsule_bottom;
        let (lo, hi) = if i.crouched { (-ROOT_LIFT_LIMITS.1, -ROOT_LIFT_LIMITS.0) } else { ROOT_LIFT_LIMITS };
        let goal = d.clamp(lo, hi);
        let s_pivot = if dz > 0.0 { -1.0 } else if dz < 0.0 { 1.0 } else { 0.0 };
        let s_goal = if goal - self.root_translation_internal.z > 0.0 { 1.0 } else if goal - self.root_translation_internal.z < 0.0 { -1.0 } else { 0.0 };
        if s_goal == s_pivot {
            self.root_translation_internal.z = move_towards(self.root_translation_internal.z, goal, dz.abs());
        }
        self.root_translation_internal.z = finterp_to(self.root_translation_internal.z, goal, dt, TRANSLATION_INTERP_SPEED);
        let rt = between_up(floor_normal, ROOT_ROTATION_OFFSET_FACTOR);
        self.root_rotation_offset = crate::procedural::rinterp_to(self.root_rotation_offset, rt, dt, ROTATION_INTERP_SPEED);
        let (llo, lhi) = if i.crouched { (-UP_VALUE_LIMITS.1, -UP_VALUE_LIMITS.0) } else { UP_VALUE_LIMITS };
        let root_z = self.root_translation_internal.z;
        for l in self.limbs.iter_mut() {
            let want = match l.hit {
                Some(_) => (l.root_space_impact_z - (i.capsule_bottom + root_z)).clamp(llo, lhi),
                None => 0.0,
            };
            l.internal_z = finterp_to(l.internal_z, want, dt, TRANSLATION_INTERP_SPEED);
            l.translation_z = l.internal_z.clamp(llo, lhi);
        }
        self.root_translation_offset = self.root_translation_internal;
        self.previous_pivot_z = i.capsule_bottom;
    }
}

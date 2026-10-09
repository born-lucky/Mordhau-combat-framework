//! The parry / chamber / clash geometry of the exe (exe mode with a geometry host; the reference has none of it and
//! treats a contact as passing every geometric test). Binary32 throughout, operation order of the decomp.
//!   UParryMotion::CheckSimpleBlock rva=0x164f330 (byte-matched src/Mordhau/Private/Motions/ParryMotion.cpp)
//!   UParryMotion::CheckSimpleBlockDirectional rva=0x164f3b0 (decomp UParryMotion.cpp 1613-1711)
//!   UParryMotion::TestForwardParry rva=0x166dbd0 (decomp 1713-1843) + FMath::LineBoxIntersection (engine, exe
//!   0x140e5e9a0, disassembled with scripts/ue_dis.py)

use super::world::FighterGeom;
use crate::ue::{rotator_vector, FQuat, FTransform, FVector};

/// GetSafeNormal2D as CheckSimpleBlockDirectional inlines it: SizeSquared2D == 1 -> (X, Y) when Z == 0 else zero;
/// < 1e-8 -> zero; else * InvSqrt (rsqrtss + 2 Newton steps, crate::ue::ue_inv_sqrt)
fn safe_normal_2d(v: FVector) -> (f32, f32) {
    let sq = v.x * v.x + v.y * v.y;
    if sq == 1.0 {
        return if v.z == 0.0 { (v.x, v.y) } else { (0.0, 0.0) };
    }
    if sq < 1e-8 {
        return (0.0, 0.0);
    }
    let r = crate::ue::ue_inv_sqrt(sq);
    (v.x * r, v.y * r)
}

/// UParryMotion::CheckSimpleBlockDirectional rva=0x164f3b0: the angle between the (2D) direction and the defender's
/// CameraRotation1P forward (FRotator::Vector, 2D-normalized), acosf(clamp(-(dir . fwd), -1, 1)) * 57.295776, wrapped to
/// [-180, 180]; |angle| <= Tolerance
pub fn check_simple_block_directional(def: &FighterGeom, dir: FVector, tolerance: f32) -> bool {
    let (dx, dy) = safe_normal_2d(dir);
    let f = rotator_vector(def.camera_rot.0, def.camera_rot.1);
    let (fx, fy) = safe_normal_2d(f);
    // fVar8 = (-dy * fy - dx * fx) + -dz * fz (dz, fz = 0 after the 2D normalize)
    let d = (-dy * fy - dx * fx) + -0.0f32 * 0.0;
    let c = if d < -1.0 { -1.0 } else if d >= 1.0 { 1.0 } else { d };
    let mut a = c.acos() * 57.295776;
    while a > 180.0 {
        a -= 360.0;
    }
    while a < -180.0 {
        a += 360.0;
    }
    a.abs() <= tolerance
}

/// UParryMotion::CheckSimpleBlock rva=0x164f330: a point within 1e-4 (2D) of the defender's camera blocks; else the
/// direction camera - point through CheckSimpleBlockDirectional
pub fn check_simple_block(def: &FighterGeom, p: FVector, tolerance: f32) -> bool {
    let dx = p.x - def.camera_loc.x;
    let dy = p.y - def.camera_loc.y;
    if (dx * dx + dy * dy).sqrt() < 1e-4 {
        return true;
    }
    let dir = FVector::new(def.camera_loc.x - p.x, def.camera_loc.y - p.y, def.camera_loc.z - p.z);
    check_simple_block_directional(def, dir, tolerance)
}

/// FMath::LineBoxIntersection (exe 0x140e5e9a0): per axis, Start outside the slab needs End on the slab's side and
/// gives Time = (plane - Start) * OneOverDirection; Start inside every slab -> true; else MaxTime = max(Ty, Tx, Tz) in
/// [0, 1] and the hit point Start + Direction * MaxTime within the box grown by 0.1 (.rdata 0x143fe4dfc), strictly
pub fn line_box_intersection(min: FVector, max: FVector, start: FVector, end: FVector, dir: FVector, inv: FVector) -> bool {
    let mut outside = false;
    let mut t = [0f32; 3];
    for k in 0..3 {
        let (s, e, lo, hi) = (start.get(k), end.get(k), min.get(k), max.get(k));
        if s < lo {
            if lo > e {
                return false;
            }
            outside = true;
            t[k] = (lo - s) * inv.get(k);
        } else if s > hi {
            if hi < e {
                return false;
            }
            outside = true;
            t[k] = (hi - s) * inv.get(k);
        }
    }
    if !outside {
        return true;
    }
    let m = |a: f32, b: f32| if a > b { a } else { b }; // maxss
    let mt = m(m(t[1], t[0]), t[2]);
    if mt < 0.0 || mt > 1.0 {
        return false;
    }
    let th = 0.1f32;
    let h = [mt * dir.x + start.x, mt * dir.y + start.y, mt * dir.z + start.z];
    (0..3).all(|k| h[k] > min.get(k) - th && h[k] < max.get(k) + th)
}

/// UParryMotion::TestForwardParry rva=0x166dbd0: BlockColliderForwardParryDistance (fwd.x, fwd.y) > 0 each; the trace
/// segment in the BlockCollider's frame (inverse rotation, scale 1) against the box Min (-ExtX, -fwd.y, -ExtZ), Max
/// (fwd.x - ExtX, fwd.y, ExtZ); Direction = start - end (as the decomp passes it), OneOverDirection 3.4e38 for a zero
/// component
pub fn test_forward_parry(def: &FighterGeom, fwd: (f32, f32), start: FVector, end: FVector) -> bool {
    if fwd.0 <= 0.0 || fwd.1 <= 0.0 {
        return false;
    }
    let bc = def.block_collider;
    let ext = def.block_extent;
    let min = FVector::new(-ext.x, -fwd.1, -ext.z);
    let max = FVector::new(fwd.0 - ext.x, fwd.1, ext.z);
    let a = bc.inverse_apply(start);
    let b = bc.inverse_apply(end);
    let d = a - b;
    let r = |x: f32| if x == 0.0 { 3.4e38f32 } else { 1.0 / x };
    line_box_intersection(min, max, a, b, d, FVector::new(r(d.x), r(d.y), r(d.z)))
}

/// A stable identity for an attack motion (the TSet<TWeakObjectPtr<UAttackMotion>> / TMap keys of BlockedAttacks):
/// the owner's id, the slab slot and the attack's StartTime bits
pub fn attack_key(owner_id: u32, slot: u32, start_time: f64) -> u64 {
    ((owner_id as u64) << 48) ^ ((slot as u64) << 32) ^ (start_time as f32).to_bits() as u64
}

/// UParryMotion::ComputeParryHeight rva=0x164f930: LookUpValue >= -5 -> (1 + clamp(L/60 + 1/12, 0, 1)) / 2, else
/// clamp((L + 70) / 65, 0, 1) / 2 (0.016666668, 0.083333336, 0.015384615 from the decomp)
pub fn compute_parry_height(look_up: f32) -> f32 {
    if -5.0 <= look_up {
        let f = look_up * 0.016666668 + 0.083333336;
        let c = if f < 0.0 { 0.0 } else if 1.0 <= f { 1.0 } else { f };
        (c + 1.0) * 0.5
    } else {
        let f = (look_up + 70.0) * 0.015384615;
        if f < 0.0 {
            0.0
        } else {
            (if 1.0 <= f { 1.0 } else { f }) * 0.5
        }
    }
}

/// An FTransform with scale (the Original / Low / HighBlockColliderRelativeOffset values)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaledXf {
    pub rot: FQuat,
    pub loc: FVector,
    pub scale: FVector,
}

/// FTransform::Blend (LerpTranslationScale3D + FQuat::FastLerp with the shortest-arc sign + Normalize)
fn blend_xf(a: &ScaledXf, b: &ScaledXf, t: f32) -> ScaledXf {
    let lerp = |x: FVector, y: FVector| FVector::new(x.x + (y.x - x.x) * t, x.y + (y.y - x.y) * t, x.z + (y.z - x.z) * t);
    let d = a.rot.x * b.rot.x + a.rot.y * b.rot.y + a.rot.z * b.rot.z + a.rot.w * b.rot.w;
    let s = if d >= 0.0 { t } else { -t };
    let u = 1.0 - t;
    let q = FQuat::new(a.rot.x * u + b.rot.x * s, a.rot.y * u + b.rot.y * s, a.rot.z * u + b.rot.z * s, a.rot.w * u + b.rot.w * s);
    let l = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
    let q = if l > 1e-8 { FQuat::new(q.x / l, q.y / l, q.z / l, q.w / l) } else { FQuat::IDENTITY };
    ScaledXf { rot: q, loc: lerp(a.loc, b.loc), scale: lerp(a.scale, b.scale) }
}

/// AMordhauCharacter::UpdateBlockCollider rva=0x15700a0: the offset blended from OriginalBlockColliderRelativeOffset
/// toward Low (parry height h <= 0.5, alpha 1 - 2h) or High (alpha 2h - 1) (alpha <= 1e-5: Original, >= 0.99999: the
/// target), then put under the base 1P camera. Returns the
/// BlockCollider's world transform (scale dropped) and its world box half extent (BoxExtent * the blended Scale3D,
/// as TestForwardParry and the overlap use it). Without a ParryWeapon (update_block_collider_pb adds its
/// ParryBoxTransform). This compatibility entry takes zero camera roll; live collider setup uses the full entry below.
pub fn update_block_collider(orig: &ScaledXf, low: &ScaledXf, high: &ScaledXf, look_up: f32, cam_loc: FVector, cam_rot: (f32, f32), extent: FVector) -> (FTransform, FVector) {
    update_block_collider_pb(orig, low, high, None, look_up, cam_loc, cam_rot, extent)
}

/// FMath::FastAsin (UE 4.2x UnrealMathUtility.h; engine source, not disassembled: UNCONFIRMED as compiled)
pub fn fast_asin(v: f32) -> f32 {
    const HALF_PI: f32 = 1.570_796_3;
    let nonneg = v >= 0.0;
    let x = v.abs();
    let omx = (1.0 - x).max(0.0);
    let root = omx.sqrt();
    let r = ((((((-0.001_262_491_1 * x + 0.006_670_090_1) * x - 0.017_088_126) * x + 0.030_891_881) * x - 0.050_174_305) * x + 0.088_978_99) * x
        - 0.214_598_8)
        * x
        + HALF_PI;
    let r = r * root;
    if nonneg { HALF_PI - r } else { r - HALF_PI }
}

/// FQuat::Rotator (UE 4.2x Math/UnrealMath.cpp; engine source, UNCONFIRMED as compiled): (pitch, yaw, roll)
pub fn quat_rotator(q: FQuat) -> (f32, f32, f32) {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    let st = z * x - w * y;
    let yaw_y = 2.0 * (w * z + x * y);
    let yaw_x = 1.0 - 2.0 * (y * y + z * z);
    const T: f32 = 0.499_999_5;
    let r2d = 180.0 / std::f32::consts::PI;
    let norm = |a: f32| {
        let mut r = a % 360.0;
        if r < 0.0 {
            r += 360.0;
        }
        if r > 180.0 {
            r -= 360.0;
        }
        r
    };
    let yaw = yaw_y.atan2(yaw_x) * r2d;
    if st < -T {
        (-90.0, yaw, norm(-yaw - 2.0 * x.atan2(w) * r2d))
    } else if st > T {
        (90.0, yaw, norm(yaw - 2.0 * x.atan2(w) * r2d))
    } else {
        (fast_asin(2.0 * st) * r2d, yaw, (-2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y)) * r2d)
    }
}

/// UMordhauCameraComponent FirstPersonLookUpOffset (ctor rva=0x14af2f0: 5.2700005; no BP_CharacterCameraComponent
/// override)
pub const FIRST_PERSON_LOOK_UP_OFFSET: f32 = 5.270_000_5;

/// AMordhauCharacter::UpdateFPCamera rva=0x1571ac0 -> UMordhauCameraComponent::GetFirstPersonCameraRotation
/// rva=0x14bcfa0 and GetFirstPersonCameraLocation rva=0x14bce70:
///   Rot = Offset * BoneRotation("Position") * MakeFromEuler(Roll = -(LookUp + FirstPersonLookUpOffset))
///         * FRotator(90, 0, -90).Quaternion(), as an FRotator (Offset taken as zero: UNCONFIRMED caller values)
///   CameraLocation1P = BoneLocation("Spine1") + Rot.RotateVector((0, 0, Mesh Scale3D.Z * 41.625))
/// Returns (CameraLocation1P, CameraRotation1P (pitch, yaw)).
pub fn camera_1p(position_rot: FQuat, spine1: FVector, mesh_scale_z: f32, look_up: f32) -> (FVector, (f32, f32)) {
    let (location,(pitch,yaw,_)) = camera_1p_full(position_rot,spine1,mesh_scale_z,look_up);
    (location,(pitch,yaw))
}

/// Base CameraLocation1P and all THREE CameraRotation1P components. UpdateBlockCollider15700a0
/// reads the full stored FRotator; its roll must survive the location/rotation handoff.
/// Cosmetic FOV offsets are separate. FP lookup-collision correction is a separate host integration gate.
pub fn camera_1p_full(position_rot: FQuat, spine1: FVector, mesh_scale_z: f32, look_up: f32) -> (FVector, (f32, f32, f32)) {
    let e = FQuat::from_rotator(0.0, 0.0, -(look_up + FIRST_PERSON_LOOK_UP_OFFSET));
    let q = position_rot.mul(e).mul(FQuat::from_rotator(90.0, 0.0, -90.0));
    let (p, y, r) = quat_rotator(q);
    let off = FQuat::from_rotator(p, y, r).rotate(FVector::new(0.0, 0.0, mesh_scale_z * 41.625));
    (spine1 + off, (p, y, r))
}

/// update_block_collider with the parry weapon's ParryBoxTransform (AMordhauWeapon +0x19c0, read while a UParryMotion
/// is current with a ParryWeapon: UpdateBlockCollider decomp AMordhauCharacter.cpp 4423-4452): the box offset becomes
/// ParryBoxTransform * Offset (FTransform::Multiply: rotation Off.rot * PB.rot, translation Off.rot(Off.scale *
/// PB.loc) + Off.loc, scale PB.scale * Off.scale) before the camera
#[allow(clippy::too_many_arguments)]
pub fn update_block_collider_pb(orig: &ScaledXf, low: &ScaledXf, high: &ScaledXf, parry_box: Option<&ScaledXf>, look_up: f32, cam_loc: FVector, cam_rot: (f32, f32), extent: FVector) -> (FTransform, FVector) {
    update_block_collider_pb_full(orig,low,high,parry_box,look_up,cam_loc,(cam_rot.0,cam_rot.1,0.0),extent)
}

/// Native base-camera placement including roll (CameraRotation1P+f30 through FRotator::Quaternion).
#[allow(clippy::too_many_arguments)]
pub fn update_block_collider_pb_full(orig: &ScaledXf, low: &ScaledXf, high: &ScaledXf, parry_box: Option<&ScaledXf>, look_up: f32, cam_loc: FVector, cam_rot: (f32, f32, f32), extent: FVector) -> (FTransform, FVector) {
    let h = compute_parry_height(look_up);
    // Preserve the original binary32 instruction order, not an algebraically equivalent reassociation.
    let (target, a) = if h <= 0.5 { (low, 1.0 - (h + h)) } else { (high, ((h - 0.5) + h) - 0.5) };
    let off = if a <= 1e-5 {
        *orig
    } else if a < 0.99999 {
        blend_xf(orig, target, a)
    } else {
        *target
    };
    let off = match parry_box {
        Some(pb) => {
            let sl = FVector::new(off.scale.x * pb.loc.x, off.scale.y * pb.loc.y, off.scale.z * pb.loc.z);
            ScaledXf { rot: off.rot.mul(pb.rot), loc: off.rot.rotate(sl) + off.loc, scale: FVector::new(pb.scale.x * off.scale.x, pb.scale.y * off.scale.y, pb.scale.z * off.scale.z) }
        }
        None => off,
    };
    let cam = FTransform::new(FQuat::from_rotator(cam_rot.0, cam_rot.1, cam_rot.2), cam_loc);
    let world = FTransform::new(off.rot, off.loc).then(&cam);
    (world, FVector::new(extent.x * off.scale.x, extent.y * off.scale.y, extent.z * off.scale.z))
}

#[cfg(test)]
mod parry_camera_tests {
    use super::*;
    fn close(actual:FVector,expected:FVector) {
        assert!((actual-expected).length()<0.0001,"{actual:?} != {expected:?}");
    }
    fn offset(loc:FVector,scale:FVector)->ScaledXf {ScaledXf {rot:FQuat::IDENTITY,loc,scale}}
    #[test]
    fn rolled_camera_rotates_box_center_and_axes_without_resizing_it() {
        let off=offset(FVector::new(2.,3.,4.),FVector::new(1.,1.,1.));
        let cam=FVector::new(100.,200.,300.);let extent=FVector::new(3.,4.,5.);
        let (rolled,size)=update_block_collider_pb_full(&off,&off,&off,None,-5.,cam,(0.,0.,90.),extent);
        // UE positive roll: Y -> -Z, Z -> Y; offsets are relative to the full camera.
        close(rolled.loc,FVector::new(102.,204.,297.));
        close(rolled.rot.rotate(FVector::new(0.,1.,0.)),FVector::new(0.,0.,-1.));
        close(rolled.rot.rotate(FVector::new(0.,0.,1.)),FVector::new(0.,1.,0.));
        assert_eq!(size,extent);
        let (zero,size0)=update_block_collider_pb_full(&off,&off,&off,None,-5.,cam,(0.,0.,0.),extent);
        assert_eq!(zero.loc,FVector::new(102.,203.,304.));assert_eq!(size0,extent);
        assert_eq!(update_block_collider_pb(&off,&off,&off,None,-5.,cam,(0.,0.),extent),(zero,size0));
    }
    #[test]
    fn weapon_parry_box_scale_and_local_translation_precede_camera_roll() {
        let orig=ScaledXf {rot:FQuat::from_rotator(0.,90.,0.),loc:FVector::new(10.,20.,30.),scale:FVector::new(2.,3.,4.)};
        let pb=ScaledXf {rot:FQuat::from_rotator(0.,0.,90.),loc:FVector::new(1.,2.,3.),scale:FVector::new(0.5,2.,0.25)};
        let (world,size)=update_block_collider_pb_full(&orig,&orig,&orig,Some(&pb),-5.,FVector::new(100.,200.,300.),(0.,0.,90.),FVector::new(3.,4.,5.));
        // PB translation (1,2,3) -> authored scale (2,6,12) -> yaw (-6,2,12),
        // plus original offset (4,22,42), then camera roll (4,42,-22).
        close(world.loc,FVector::new(104.,242.,278.));assert_eq!(size,FVector::new(3.,24.,5.));
        close(world.rot.rotate(FVector::new(1.,0.,0.)),FVector::new(0.,0.,-1.));
        close(world.rot.rotate(FVector::new(0.,1.,0.)),FVector::new(0.,-1.,0.));
    }
    #[test]
    fn original_low_high_and_thresholds_preserve_the_authored_offsets() {
        let orig=offset(FVector::new(10.,0.,0.),FVector::new(1.,1.,1.));
        let low=offset(FVector::new(-10.,0.,0.),FVector::new(2.,2.,2.));
        let high=offset(FVector::new(30.,0.,0.),FVector::new(3.,3.,3.));
        let extent=FVector::new(1.,1.,1.);
        for (look,x,scale) in [(-70.,-10.,2.),(-5.,10.,1.),(55.,30.,3.),(25.,20.,2.)] {
            let (world,size)=update_block_collider_pb_full(&orig,&low,&high,None,look,FVector::ZERO,(0.,0.,0.),extent);
            close(world.loc,FVector::new(x,0.,0.));close(size,FVector::new(scale,scale,scale));
        }
        for look in [-5.00025,-4.99975] {
            let (world,size)=update_block_collider_pb_full(&orig,&low,&high,None,look,FVector::ZERO,(0.,0.,0.),extent);
            assert_eq!(world.loc,orig.loc);assert_eq!(size,extent);
        }
        for (look,target) in [(-69.9998,low),(54.9998,high)] {
            let (world,size)=update_block_collider_pb_full(&orig,&low,&high,None,look,FVector::ZERO,(0.,0.,0.),extent);
            assert_eq!(world.loc,target.loc);assert_eq!(size,target.scale);
        }
    }
    #[test]
    fn camera_location_and_rotation_share_the_full_rotator() {
        let spine=FVector::new(5.,7.,11.);
        let (loc,rot)=camera_1p_full(FQuat::from_rotator(15.,25.,35.),spine,1.5,20.);
        assert!(rot.2.abs()>1.,"Fixture must expose the previously lost roll");
        let camera=FQuat::from_rotator(rot.0,rot.1,rot.2);
        close(loc-spine,camera.rotate(FVector::new(0.,0.,62.4375)));
        let (oldloc,oldrot)=camera_1p(FQuat::from_rotator(15.,25.,35.),spine,1.5,20.);
        assert_eq!(oldloc,loc);assert_eq!(oldrot,(rot.0,rot.1));
    }
}

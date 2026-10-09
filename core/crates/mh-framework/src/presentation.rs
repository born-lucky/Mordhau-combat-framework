//! Pure presentation math for hosts; no renderer or original data payloads.
use mordhau_core::combat::enums::br::{PARRY as BR_PARRY, CHAMBER as BR_CHAMBER};

/// The unmodified character fields OnBlocked reads (UE world cm / unit UE forward).
/// bIsFirstPerson is +0xf08, CameraLocation1P +0xf18, CameraRotation1P +0xf30.
/// This is not the cosmetic POV or bIsViewTarget: native 0x1630900 tests +0xf08.
#[derive(Clone, Copy, Debug)]
pub struct BlockedCamera {
    pub location: mordhau_core::ue::FVector,
    pub forward: mordhau_core::ue::FVector,
    pub first_person: bool,
}

/// AMordhauWeapon::OnBlocked_Implementation 0x16306d0 (1404 bytes): GetTrace/OverrideTrace
/// belongs to the DEFENDER's selected weapon, not the attacker's swept impact point.
/// Parry/Chamber use 15cm along the current blade, other reasons 30cm (0x1630706/0x163073c).
/// Parry blends halfway to CameraLocation1P + forward*(bIsFirstPerson ?35:60)
/// (0x1630900..0x1630aac). Camera input must be the same-tick raw native fields.
/// An unavailable parry camera is explicit None; this helper never guesses from the impact/POV.
/// The rare parent-character-null native branch is outside this living-defender contract.
pub fn blocked_particle_point(
    reason: i64,
    trace_start: mordhau_core::ue::FVector,
    trace_end: mordhau_core::ue::FVector,
    camera: Option<BlockedCamera>,
) -> Option<mordhau_core::ue::FVector> {
    use mordhau_core::ue::FVector;
    let d = trace_end - trace_start;
    // Native FVector::GetSafeNormal gate; reciprocal sqrt is bounded f32 math here,
    // not a claim of bitwise parity with the original rsqrtss + two Newton steps.
    let l2 = d.y*d.y + d.x*d.x + d.z*d.z;
    let unit = if l2 == 1.0 { d } else if l2 < 1e-8 { FVector::ZERO } else { d.scale(l2.sqrt().recip() as f64) };
    let length = if reason == BR_PARRY || reason == BR_CHAMBER { 15.0 } else { 30.0 };
    let point = trace_start + unit.scale(length);
    if reason != BR_PARRY { return Some(point); }
    let camera = camera?;
    let near = camera.location + camera.forward.scale(if camera.first_person { 35.0 } else { 60.0 });
    Some(FVector::new((point.x-near.x)*0.5+near.x,
        (point.y-near.y)*0.5+near.y,(point.z-near.z)*0.5+near.z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mordhau_core::combat::enums::br::CLASH as BR_CLASH;
    #[test]
    fn blocked_particle_origin_uses_defender_blade_and_native_first_person_flag() {
        use mordhau_core::ue::FVector as V;
        let start=V::new(100.0,200.0,300.0);let end=V::new(100.0,220.0,300.0);
        let camera=BlockedCamera {location:V::new(10.0,20.0,30.0),forward:V::new(1.0,0.0,0.0),first_person:true};
        // Independent axis geometry: blade point(100,215,300), camera point(45,20,30).
        assert_eq!(blocked_particle_point(BR_PARRY,start,end,Some(camera)),Some(V::new(72.5,117.5,165.0)));
        // The 60cm branch changes only X; view-target does not select this constant.
        assert_eq!(blocked_particle_point(BR_PARRY,start,end,Some(BlockedCamera {first_person:false,..camera})),Some(V::new(85.0,117.5,165.0)));
        // Chamber has no camera blend; clash uses30cm and also ignores camera.
        assert_eq!(blocked_particle_point(BR_CHAMBER,start,end,None),Some(V::new(100.0,215.0,300.0)));
        assert_eq!(blocked_particle_point(BR_CLASH,start,end,Some(camera)),Some(V::new(100.0,230.0,300.0)));
        // Required native fields unavailable means no manufactured camera origin.
        assert!(blocked_particle_point(BR_PARRY,start,end,None).is_none());
    }

    #[test]
    fn blocked_particle_origin_normalizes_blade_and_handles_zero_trace() {
        use mordhau_core::ue::FVector as V;
        // 3/4/5 direction produces a15cm offset(9,12,0), irrespective of blade length.
        let start=V::new(7.0,11.0,13.0);
        let a=blocked_particle_point(BR_CHAMBER,start,start+V::new(3.0,4.0,0.0),None).unwrap();
        let b=blocked_particle_point(BR_CHAMBER,start,start+V::new(30.0,40.0,0.0),None).unwrap();
        assert!((a.x-16.0).abs()<1e-5&&(a.y-23.0).abs()<1e-5&&a.z==13.0);
        assert!((a.x-b.x).abs()<1e-5&&(a.y-b.y).abs()<1e-5);
        assert_eq!(blocked_particle_point(BR_CHAMBER,start,start,None),Some(start));
        assert_eq!(blocked_particle_point(BR_CHAMBER,V::ZERO,V::new(0.00001,0.0,0.0),None),Some(V::ZERO));
    }

}

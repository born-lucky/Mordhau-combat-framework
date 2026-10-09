//! Complete original PhysX cooked meshes, owned by the existing Scene.
//! Native execution is separately gated until the guarded original-wall fixture is accepted.
use crate::{Scene, Transform};
use std::ffi::c_void;

// Original cooked full-array/ray + body/D6/death guarded fixtures passed. The matching
// normal-mode bridge145661f0 is independently reviewed (contact-polish proofs20261008).
// Canonical contact/render QA is a separate rollout gate; diagnostic DLLs remain rejected.
pub(crate) const COOKED_ABI_VERIFIED: bool = true;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CookedSet { pub first: u32, pub normal: u32, pub mirrored: u32, pub triangles: u32, pub consumed: u32 }
impl CookedSet {
    pub fn triangle_handles(&self) -> std::ops::Range<u32> {
        let first = self.first + self.normal + self.mirrored;
        first..first + self.triangles
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CookedInfo { pub kind: u32, pub vertices: u32, pub triangles: u32, pub minimum: [f32; 3], pub maximum: [f32; 3] }
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CookedRayHit {
    pub position: [f32; 3], pub normal: [f32; 3], pub distance: f32,
    pub face: u32, pub material: u32, pub flags: u32, pub u: f32, pub v: f32,
}
type Load = unsafe extern "C" fn(*mut c_void,*const u8,u32,u32,*mut CookedSet)->i32;
type Info = unsafe extern "C" fn(*mut c_void,u32,*mut CookedInfo)->i32;
type Ray = unsafe extern "C" fn(*mut c_void,u32,*const Transform,*const f32,u32,*const f32,*const f32,f32,*mut CookedRayHit)->i32;
type Readback = unsafe extern "C" fn(*mut c_void,u32,*mut f32,u32,*mut u32,u32,*mut u32)->i32;

pub(crate) struct Api { load: Load, info: Info, ray: Ray, readback: Readback }
impl Api {
    /// Existing ragdoll exports remain usable with the previous DLL. This provider fails closed unless ALL its exports exist.
    pub(crate) unsafe fn resolve(lib: &libloading::Library) -> Option<Self> {
        Some(Self {
            load: *lib.get::<Load>(b"mh_px_cooked_load\0").ok()?,
            info: *lib.get::<Info>(b"mh_px_cooked_info\0").ok()?,
            ray: *lib.get::<Ray>(b"mh_px_cooked_ray\0").ok()?,
            readback: *lib.get::<Readback>(b"mh_px_cooked_readback\0").ok()?,
        })
    }
}

fn verify_payload(bytes: &[u8], expected_size: usize, sha1: [u8; 20]) -> Result<(), String> {
    if bytes.len() != expected_size || bytes.len() < 13 || bytes.len() > u32::MAX as usize {
        return Err("Original cooked payload length differs from its verified bulk descriptor".into());
    }
    if sha1_smol::Sha1::from(bytes).digest().bytes() != sha1 {
        return Err("Original cooked payload SHA1 differs from its verified pak bytes".into());
    }
    if bytes[0] != 1 { return Err("Unsupported cooked PhysX byte order".into()); }
    Ok(())
}

impl Scene {
    fn cooked_api(&self) -> Result<&Api, String> {
        if !COOKED_ABI_VERIFIED && !self.guarded {
            return Err("Original cooked collision native validation is pending".into());
        }
        self.cooked.as_ref().ok_or_else(|| "PhysX bridge lacks the complete reviewed cooked collision API".into())
    }
    /// Only pass a complete SHA1-verified original pak payload, with the extractor's size/hash.
    /// The hash is a consistency check, not authorization to deserialize arbitrary cooked data.
    pub fn load_cooked(&mut self, bytes: &[u8], expected_size: usize, sha1: [u8; 20]) -> Result<CookedSet, String> {
        verify_payload(bytes, expected_size, sha1)?;
        let api = self.cooked_api()?;
        let mut out = CookedSet::default();
        // Native copies/owns all mesh objects; the borrowed payload need only survive this call.
        let ok = unsafe { (api.load)(self.raw, bytes.as_ptr(), bytes.len() as u32, expected_size as u32, &mut out) };
        if ok == 0 { return Err(self.failure("Original cooked mesh loading failed")); }
        let total = out.normal.checked_add(out.mirrored).and_then(|n| n.checked_add(out.triangles));
        if total.and_then(|n| out.first.checked_add(n)).is_none() || out.consumed as usize > bytes.len() || out.consumed < 13 {
            return Err("Native cooked mesh descriptor is out of bounds".into());
        }
        Ok(out)
    }
    pub fn cooked_info(&self, handle: u32) -> Result<CookedInfo, String> {
        let mut out = CookedInfo::default();
        if unsafe { (self.cooked_api()?.info)(self.raw, handle, &mut out) } == 0 {
            Err(self.failure("Original cooked mesh metadata query failed"))
        } else { Ok(out) }
    }
    pub fn ray_cooked(&self, handle: u32, placement: &Transform, scale: [f32; 3], double_sided: bool,
                      origin: [f32; 3], unit_direction: [f32; 3], distance: f32) -> Result<Option<CookedRayHit>, String> {
        let mut out = CookedRayHit::default();
        let result = unsafe { (self.cooked_api()?.ray)(self.raw, handle, placement, scale.as_ptr(), u32::from(double_sided),
                                                     origin.as_ptr(), unit_direction.as_ptr(), distance, &mut out) };
        match result { 0 => Ok(None), 1 => Ok(Some(out)), _ => Err("Invalid original cooked geometry/ray descriptor".into()) }
    }
    /// Full native array readback exists only in a guarded build. Counts are checked BEFORE allocation and native copying.
    #[cfg(feature = "native-validation")]
    pub fn cooked_readback_for_validation(&self, handle: u32, expected_vertices: u32, expected_triangles: u32)
            -> Result<(Vec<f32>, Vec<u32>, Vec<u32>), String> {
        if !self.guarded { return Err("Cooked readback requires the reviewed guarded DLL".into()); }
        let info = self.cooked_info(handle)?;
        if info.kind != 5 || info.vertices != expected_vertices || info.triangles != expected_triangles {
            return Err("Original cooked readback counts differ from the frozen fixture".into());
        }
        let nv = (expected_vertices as usize).checked_mul(3).ok_or("Cooked vertex count overflow")?;
        let ni = (expected_triangles as usize).checked_mul(3).ok_or("Cooked triangle count overflow")?;
        let mut vertices = vec![0.; nv]; let mut indices = vec![0; ni]; let mut materials = vec![0; expected_triangles as usize];
        let ok = unsafe { (self.cooked_api()?.readback)(self.raw, handle, vertices.as_mut_ptr(), expected_vertices,
                                                     indices.as_mut_ptr(), expected_triangles, materials.as_mut_ptr()) };
        if ok == 0 { Err(self.failure("Guarded original cooked array readback failed")) } else { Ok((vertices, indices, materials)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_descriptors_match_reviewed_native_abi() {
        use std::mem::{size_of, offset_of};
        assert_eq!(size_of::<CookedSet>(),20); assert_eq!(size_of::<CookedInfo>(),36); assert_eq!(size_of::<CookedRayHit>(),48);
        assert_eq!(offset_of!(CookedInfo,minimum),12); assert_eq!(offset_of!(CookedInfo,maximum),24);
        assert_eq!(offset_of!(CookedRayHit,normal),12); assert_eq!(offset_of!(CookedRayHit,distance),24);
        assert_eq!(offset_of!(CookedRayHit,face),28); assert_eq!(offset_of!(CookedRayHit,material),32);
        assert_eq!(offset_of!(CookedRayHit,flags),36); assert_eq!(offset_of!(CookedRayHit,u),40); assert_eq!(offset_of!(CookedRayHit,v),44);
    }
    #[test]
    fn changed_or_truncated_original_payload_rejected_before_native_entry() {
        let mut data = [0u8; 13]; data[0] = 1;
        let sha1 = sha1_smol::Sha1::from(&data[..]).digest().bytes();
        assert!(verify_payload(&data,13,sha1).is_ok());
        assert!(verify_payload(&data[..12],13,sha1).is_err());
        data[12] ^= 1; assert!(verify_payload(&data,13,sha1).is_err());
        assert!(verify_payload(&data,14,sha1).is_err());
    }
}

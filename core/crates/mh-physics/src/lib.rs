//! PhysX 3.4.0 from the user's game install, with UE centimetres/Z-up throughout.
//! SDK/header pin and native build: scripts/build_physx.py. This crate never attaches to the running game.

use std::ffi::{c_char, c_void, CStr};
use std::marker::PhantomData;
use std::path::Path;
use std::rc::Rc;
use std::sync::Mutex;

pub mod cooked;

static INIT: Mutex<()> = Mutex::new(());

// The bridge uses the independently reviewed original ABI overlay. Enable only after
// the guarded production-body/D6/death fixture and teardown pass. Old or diagnostic
// DLLs still fail closed via ABI revision and instrumentation checks before creation.
const GAME_ABI_VERIFIED: bool = true;
const BRIDGE_ABI_REVISION: u32 = 0x20261008;
const ABI_BLOCKED: &str = "Original PhysX backend disabled: SDK/DLL PxSceneDesc and PxCookingParams ABI mismatch; native corpse validation pending";

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Transform { pub position: [f32; 3], pub rotation: [f32; 4] }
impl Default for Transform {
    fn default() -> Self { Self { position: [0.; 3], rotation: [0.,0.,0.,1.] } }
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    /// 0 sphere (radius), 1 box (half extents XYZ), 2 capsule (radius, cylinder half length), along local X.
    pub kind: u32, pub local: Transform, pub size: [f32; 3], pub rest_offset: f32, pub contact_offset: f32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub world: Transform, pub mass: f32, pub linear_damping: f32, pub angular_damping: f32,
    pub com_nudge: [f32; 3], pub sleep_threshold: f32,
    pub position_iterations: u32, pub velocity_iterations: u32, pub kinematic: u32,
    pub velocity: [f32; 3], pub group: u32, pub max_angular_velocity: f32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Joint {
    pub body0: u32, pub body1: u32, pub frame0: Transform, pub frame1: Transform,
    /// PhysX D6 enum: Locked0, Limited1, Free2; X/Y/Z/Twist/Swing1/Swing2.
    pub motion: [u32; 6], pub linear_limit: f32, pub swing1: f32, pub swing2: f32, pub twist: f32,
    pub restitution: [f32; 3], pub contact_distance: [f32; 3], pub stiffness: [f32; 3], pub damping: [f32; 3],
    pub soft: [u32; 3], pub disable_collision: u32, pub projection: u32, pub parent_dominates: u32,
    pub projection_linear: f32, pub projection_angular: f32,
}
type Create = unsafe extern "C" fn(f32,f32,f32,f32,f32) -> *mut c_void;
type Release = unsafe extern "C" fn(*mut c_void);
type AddBody = unsafe extern "C" fn(*mut c_void,*const Body,*const Shape,u32)->u32;
type AddJoint = unsafe extern "C" fn(*mut c_void,*const Joint)->u32;
type AddStatic = unsafe extern "C" fn(*mut c_void,*const f32,u32,f32,f32)->i32;
type Step = unsafe extern "C" fn(*mut c_void,f32)->i32;
type Pose = unsafe extern "C" fn(*mut c_void,u32,*mut Transform)->i32;
type Remove = unsafe extern "C" fn(*mut c_void,*const u32,u32);
type DisablePairs = unsafe extern "C" fn(*mut c_void,*const u32,u32);
type Impulse = unsafe extern "C" fn(*mut c_void,u32,*const f32,*const f32)->i32;
type Error = unsafe extern "C" fn()->*const c_char;

pub struct Scene {
    raw: *mut c_void, release: Release, body: AddBody, joint: AddJoint, static_shape: AddStatic,
    step: Step, pose: Pose, remove: Remove, disable_pairs: DisablePairs, impulse: Impulse, error: Error,
    // Every symbol belongs to these loaded modules. The scene is released before their handles are dropped.
    _modules: Vec<libloading::Library>,
    cooked: Option<cooked::Api>,
    guarded: bool,
    // PhysX scenes and the owning sim remain on their creating thread.
    _thread: PhantomData<Rc<()>>,
}
impl Scene {
    pub fn open(bridge: &Path, dll_dir: &Path, gravity: f32, friction: f32, restitution: f32,
                length_scale: f32, speed_scale: f32) -> Result<Self,String> {
        if !GAME_ABI_VERIFIED { return Err(ABI_BLOCKED.into()); }
        Self::open_native(bridge,dll_dir,gravity,friction,restitution,length_scale,speed_scale,false)
    }
    /// Diagnostic-only native entry. Production `open` remains blocked.
    /// Call only in an isolated process after exact bridge/input review, never from runtime setup.
    #[cfg(feature = "native-validation")]
    pub fn open_for_validation(bridge: &Path, dll_dir: &Path, gravity: f32, friction: f32, restitution: f32,
                               length_scale: f32, speed_scale: f32) -> Result<Self,String> {
        Self::open_native(bridge,dll_dir,gravity,friction,restitution,length_scale,speed_scale,true)
    }
    fn open_native(bridge: &Path, dll_dir: &Path, gravity: f32, friction: f32, restitution: f32,
                   length_scale: f32, speed_scale: f32, validation: bool) -> Result<Self,String> {
        if !cfg!(windows) { return Err("The installed game's PhysX DLL backend requires Windows".into()); }
        let _init = INIT.lock().map_err(|_| "PhysX initialization mutex poisoned")?;
        // SAFETY: each absolute path is supplied by the host. The bridge ABI is our C++ source, not inferred UE vtables.
        unsafe {
            let mut modules = Vec::new();
            for name in ["PxFoundation_x64", "PxPvdSDK_x64", "PhysX3Common_x64", "PhysX3_x64", "PhysX3Cooking_x64"] {
                modules.push(load(&dll_dir.join(format!("{name}.dll")))?);
            }
            let lib = load(bridge)?;
            let version = *lib.get::<unsafe extern "C" fn()->u32>(b"mh_px_version\0").map_err(|e|e.to_string())?;
            if version()!=0x03040000 { return Err("PhysX bridge version must be 3.4.0".into()); }
            let revision=*lib.get::<unsafe extern "C" fn()->u32>(b"mh_px_abi_revision\0").map_err(|e|e.to_string())?;
            if revision()!=BRIDGE_ABI_REVISION { return Err("PhysX bridge ABI revision differs from the reviewed repair".into()); }
            let guarded=*lib.get::<unsafe extern "C" fn()->u32>(b"mh_px_validation\0").map_err(|e|e.to_string())?;
            if guarded()!=u32::from(validation) { return Err("PhysX bridge validation instrumentation differs from the requested mode".into()); }
            let create = *lib.get::<Create>(b"mh_px_scene\0").map_err(|e|e.to_string())?;
            let release = *lib.get::<Release>(b"mh_px_release\0").map_err(|e|e.to_string())?;
            let body = *lib.get::<AddBody>(b"mh_px_body\0").map_err(|e|e.to_string())?;
            let joint = *lib.get::<AddJoint>(b"mh_px_joint\0").map_err(|e|e.to_string())?;
            let static_shape = *lib.get::<AddStatic>(b"mh_px_static\0").map_err(|e|e.to_string())?;
            let step = *lib.get::<Step>(b"mh_px_step\0").map_err(|e|e.to_string())?;
            let pose = *lib.get::<Pose>(b"mh_px_pose\0").map_err(|e|e.to_string())?;
            let remove = *lib.get::<Remove>(b"mh_px_remove\0").map_err(|e|e.to_string())?;
            let disable_pairs = *lib.get::<DisablePairs>(b"mh_px_disable_pairs\0").map_err(|e|e.to_string())?;
            let impulse = *lib.get::<Impulse>(b"mh_px_impulse\0").map_err(|e|e.to_string())?;
            let error = *lib.get::<Error>(b"mh_px_error\0").map_err(|e|e.to_string())?;
            let cooked = cooked::Api::resolve(&lib);
            let raw=create(gravity,friction,restitution,length_scale,speed_scale);
            if raw.is_null() { return Err(message(error,"PhysX scene initialization failed")); }
            modules.push(lib);
            Ok(Self {raw,release,body,joint,static_shape,step,pose,remove,disable_pairs,impulse,error,cooked,guarded:validation,_modules:modules,_thread:PhantomData})
        }
    }
    pub fn add_body(&mut self, b: &Body, shapes: &[Shape]) -> Result<u32,String> {
        if shapes.is_empty() || !b.mass.is_finite() || b.mass<=0. || b.position_iterations==0 || b.velocity_iterations==0 {
            return Err("Invalid rigid body mass/shapes/solver counts".into());
        }
        // SAFETY: native copies the descriptors before returning; the scene remains alive and thread confined.
        let id=unsafe{(self.body)(self.raw,b,shapes.as_ptr(),shapes.len() as u32)};
        if id==u32::MAX { Err(self.failure("Rigid body creation failed")) } else { Ok(id) }
    }
    pub fn add_joint(&mut self, d: &Joint) -> Result<u32,String> {
        let id=unsafe{(self.joint)(self.raw,d)};
        if id==u32::MAX {Err(self.failure("Joint creation failed"))} else {Ok(id)}
    }
    pub fn add_static(&mut self, points:&[[f32;3]], radius:f32, contact_offset:f32)->Result<(),String> {
        if points.is_empty() {return Err("Static shape has no points".into());}
        let ok=unsafe{(self.static_shape)(self.raw,points.as_ptr().cast(),points.len() as u32,radius,contact_offset)};
        if ok==0 {Err(self.failure("Static shape creation failed"))} else {Ok(())}
    }
    pub fn step(&mut self,dt:f32)->Result<(),String> {
        let ok=unsafe{(self.step)(self.raw,dt)};
        if ok==0 {Err(self.failure("Physics simulation/fetch failed"))} else {Ok(())}
    }
    pub fn pose(&self,id:u32)->Option<Transform> {
        let mut out=Transform::default();
        (unsafe{(self.pose)(self.raw,id,&mut out)}!=0).then_some(out)
    }
    pub fn remove_bodies(&mut self,ids:&[u32]) {unsafe{(self.remove)(self.raw,ids.as_ptr(),ids.len() as u32)}}
    pub fn disable_pairs(&mut self,pairs:&[[u32;2]]) {
        if !pairs.is_empty() {unsafe{(self.disable_pairs)(self.raw,pairs.as_ptr().cast(),pairs.len() as u32)}}
    }
    pub fn impulse_at(&mut self,id:u32,impulse:[f32;3],point:[f32;3])->Result<(),String> {
        if unsafe{(self.impulse)(self.raw,id,impulse.as_ptr(),point.as_ptr())}==0 {Err(self.failure("Impulse failed"))} else {Ok(())}
    }
    fn failure(&self,fallback:&str)->String {unsafe{message(self.error,fallback)}}
}
unsafe fn message(error:Error,fallback:&str)->String {
    let p=error(); if p.is_null(){return fallback.into();}
    let s=CStr::from_ptr(p).to_string_lossy(); if s.is_empty(){fallback.into()} else {s.into_owned()}
}
unsafe fn load(path:&Path)->Result<libloading::Library,String> {
    #[cfg(windows)] {
        // Dependency lookup stays in this module's directory/system paths; no process PATH mutation.
        libloading::os::windows::Library::load_with_flags(path,0x100|0x1000)
            .map(Into::into).map_err(|e|format!("{}: {e}",path.display()))
    }
    #[cfg(not(windows))] {Err(format!("Windows DLL cannot be loaded: {}",path.display()))}
}
impl Drop for Scene {
    fn drop(&mut self) {
        // SAFETY: exactly one owner; every native body/joint is released with the scene before unloading code.
        if let Ok(_init)=INIT.lock() {unsafe{(self.release)(self.raw)}}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_libraries_never_create_a_native_scene() {
        let result = Scene::open(Path::new("missing-bridge.dll"), Path::new("missing-game-dlls"), -980., 0.7, 0.3, 100., 1000.);
        assert!(result.is_err());
        if !GAME_ABI_VERIFIED { assert!(matches!(result, Err(ref error) if error == ABI_BLOCKED)); }
    }

    /// Run only after activation to prove ordinary runtime entry refuses the diagnostic DLL.
    #[test]
    #[cfg(feature = "native-validation")]
    #[ignore = "Post-safety activation diagnostic; guarded DLL path and root dispatch required"]
    fn production_open_rejects_guarded_bridge_before_creation() {
        assert!(GAME_ABI_VERIFIED, "Run only after native safety activation");
        let bridge=std::path::PathBuf::from(std::env::var_os("MH_PHYSX_VALIDATION_BRIDGE")
            .expect("Explicit reviewed guarded DLL path required"));
        assert!(bridge.is_absolute());
        let game=std::path::PathBuf::from(std::env::var_os("MORDHAU_DIR")
            .unwrap_or_else(||"C:\\Program Files (x86)\\Steam\\steamapps\\common\\Mordhau".into()));
        let result=Scene::open(&bridge,&game.join("Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015"),
                               -980.,0.7,0.3,100.,1000.);
        assert!(matches!(result, Err(ref error) if error.contains("validation instrumentation differs")));
    }

    /// First exercise the production geometry/mass/COM path without introducing D6 callbacks.
    #[test]
    #[cfg(feature = "native-validation")]
    #[ignore = "Explicit isolated native validation: reviewed guarded DLL and root dispatch required"]
    fn production_body_geometry_mass_and_teardown() {
        let bridge=std::path::PathBuf::from(std::env::var_os("MH_PHYSX_VALIDATION_BRIDGE")
            .expect("Explicit reviewed staged DLL path required"));
        assert!(bridge.is_absolute());
        let game=std::path::PathBuf::from(std::env::var_os("MORDHAU_DIR")
            .unwrap_or_else(||"C:\\Program Files (x86)\\Steam\\steamapps\\common\\Mordhau".into()));
        let dlls=game.join("Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015");
        for kind in 0..3 {
            let mut scene=Scene::open_for_validation(&bridge,&dlls,-980.,0.7,0.3,100.,1000.).unwrap();
            let body=Body {world:Transform {position:[0.,0.,100.],..Default::default()},mass:2.,
                linear_damping:0.01,angular_damping:0.5,com_nudge:[1.,2.,3.],sleep_threshold:50.,
                position_iterations:8,velocity_iterations:1,kinematic:0,velocity:[0.;3],group:1,
                max_angular_velocity:std::f32::consts::PI*20.};
            let shape=Shape {kind,local:Transform {position:[3.,4.,5.],..Default::default()},
                size:if kind==1 {[10.,10.,10.]} else {[10.,10.,0.]},rest_offset:0.,contact_offset:0.1};
            let id=scene.add_body(&body,&[shape]).unwrap();
            assert_eq!(id,0);
            for _ in 0..120 {scene.step(1./120.).unwrap();}
            let pose=scene.pose(id).unwrap();
            assert!(pose.position.iter().chain(&pose.rotation).all(|v|v.is_finite()));
            assert!(pose.position[2]<80.,"Production dynamic body did not respond to original gravity");
            scene.remove_bodies(&[id]);
            assert!(scene.pose(id).is_none());
            scene.step(1./120.).unwrap();
            // Drop performs guarded full native release; require_empty is enforced in the diagnostic DLL.
            eprintln!("PROBE production geometry kind{kind} teardown");
        }
    }
}

//! armory_natives.rs - the natives the Armory / Mercenaries Blueprints call (BP_MainMenu's armory tab, BP_ProfileCustomization,
//! BP_LoadoutPicker, the customization platform / observer / doll actors). natives::call falls through to `call` first;
//! None = not an armory native. Rules and data live in armory.rs (cited there); actor spawning and transforms follow
//! UE 4.26 GameplayStatics / AActor / KismetMathLibrary, rotation math as the exe does it (mh-character uequat.rs:
//! FRotator::Quaternion rva 0x18b8f90, FQuat::Rotator rva=0x18bdc80, FRotator::Vector rva 0x18c3050).

use crate::armory;
use crate::kismet::ObjRef;
use crate::model::*;
use crate::natives::NRet;
use crate::vm::{Ctx, Vm};
use mh_character::ue::FVector;
use mh_character::uequat as uq;
use std::sync::Arc;

fn r(v: V) -> Option<NRet> {
    Some(NRet { ret: v, outs: vec![] })
}

fn outs(ret: V, o: Vec<(usize, V)>) -> Option<NRet> {
    Some(NRet { ret, outs: o })
}

/// a Blueprint class reference value for a class package ("Mordhau/Content/X/BP_Y" -> BP_Y_C)
pub fn class_ref(pkg: &str) -> V {
    let n = pkg.rsplit('/').next().unwrap_or(pkg);
    V::Asset(Arc::new(ObjRef { package: pkg.to_string(), name: format!("{n}_C"), class: "BlueprintGeneratedClass".into(), outer: String::new() }))
}

// ---- value <-> UE math ------------------------------------------------------------------------------------------

pub fn vec_of(v: &V) -> FVector {
    FVector::new(v.field("X").f() as f32, v.field("Y").f() as f32, v.field("Z").f() as f32)
}
pub fn vec_v(p: FVector) -> V {
    V::st(&[("X", V::Float(p.x as f64)), ("Y", V::Float(p.y as f64)), ("Z", V::Float(p.z as f64))])
}
pub fn rot_of(v: &V) -> (f32, f32, f32) {
    (v.field("Pitch").f() as f32, v.field("Yaw").f() as f32, v.field("Roll").f() as f32)
}
pub fn rot_v(r: (f32, f32, f32)) -> V {
    V::st(&[("Pitch", V::Float(r.0 as f64)), ("Yaw", V::Float(r.1 as f64)), ("Roll", V::Float(r.2 as f64))])
}
fn quat_v(q: uq::Quat) -> V {
    V::st(&[("X", V::Float(q[0] as f64)), ("Y", V::Float(q[1] as f64)), ("Z", V::Float(q[2] as f64)), ("W", V::Float(q[3] as f64))])
}
fn quat_of(v: &V) -> uq::Quat {
    if matches!(v.field("W"), V::None) {
        // a rotator literal where a quaternion is expected (VM struct defaults): identity / from the rotator
        let (p, y, r) = rot_of(v);
        return uq::rotator_quaternion(p, y, r);
    }
    [v.field("X").f() as f32, v.field("Y").f() as f32, v.field("Z").f() as f32, v.field("W").f() as f32]
}
/// FQuat::operator* (Hamilton product; (a * b) rotates by b then a). UNCONFIRMED: the exe's SIMD lane order
/// (VectorQuaternionMultiply2) is not matched bit for bit.
fn qmul(a: uq::Quat, b: uq::Quat) -> uq::Quat {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]
}

#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub t: FVector,
    pub q: uq::Quat,
    pub s: FVector,
}

impl Xf {
    pub const IDENTITY: Xf = Xf { t: FVector::ZERO, q: [0.0, 0.0, 0.0, 1.0], s: FVector { x: 1.0, y: 1.0, z: 1.0 } };
    pub fn from_v(v: &V) -> Xf {
        let s = match v.field("Scale3D") {
            V::None => FVector::new(1.0, 1.0, 1.0),
            x => vec_of(x),
        };
        Xf { t: vec_of(v.field("Translation")), q: match v.field("Rotation") {
            V::None => [0.0, 0.0, 0.0, 1.0],
            x => quat_of(x),
        }, s }
    }
    pub fn to_v(self) -> V {
        V::st(&[("Translation", vec_v(self.t)), ("Rotation", quat_v(self.q)), ("Scale3D", vec_v(self.s))])
    }
    pub fn from_loc_rot(l: FVector, r: (f32, f32, f32)) -> Xf {
        Xf { t: l, q: uq::rotator_quaternion(r.0, r.1, r.2), s: FVector::new(1.0, 1.0, 1.0) }
    }
    pub fn rotator(self) -> (f32, f32, f32) {
        uq::quat_rotator(self.q)
    }
    /// FTransform::operator* (A * B = A then B): rotation B.q * A.q, scale A.s * B.s, translation
    /// B.q.Rotate(B.s * A.t) + B.t
    pub fn compose(self, b: Xf) -> Xf {
        let st = FVector::new(b.s.x * self.t.x, b.s.y * self.t.y, b.s.z * self.t.z);
        let t = uq::quat_rotate(b.q, st) + b.t;
        Xf { t, q: qmul(b.q, self.q), s: FVector::new(self.s.x * b.s.x, self.s.y * b.s.y, self.s.z * b.s.z) }
    }
    /// FTransform::TransformPosition
    pub fn apply(self, p: FVector) -> FVector {
        uq::quat_rotate(self.q, FVector::new(p.x * self.s.x, p.y * self.s.y, p.z * self.s.z)) + self.t
    }
    /// FTransform::InverseTransformPosition: (Q^-1 (P - T)) / S
    pub fn inverse_apply(self, p: FVector) -> FVector {
        let d = uq::quat_rotate(uq::quat_inverse(self.q), p - self.t);
        let sd = |a: f32, b: f32| if b == 0.0 { 0.0 } else { a / b };
        FVector::new(sd(d.x, self.s.x), sd(d.y, self.s.y), sd(d.z, self.s.z))
    }
}

// ---- actors spawned by the UI Blueprints ---------------------------------------------------------------------------

const ACTORS: &str = "__armory_actors";
const FRAME: &str = "__armory_frame";

pub fn actors(vm: &Vm) -> Vec<Id> {
    vm.world.get("map").map(|&m| vm.prop(m, ACTORS).arr().iter().filter_map(V::obj).collect()).unwrap_or_default()
}

fn register(vm: &mut Vm, id: Id) {
    if let Some(&m) = vm.world.get("map") {
        let mut l = vm.prop(m, ACTORS).arr().to_vec();
        l.push(V::Obj(id));
        vm.set(m, ACTORS, V::Array(l));
    }
}

/// AActor ticking for actors the UI Blueprints spawned (the customization platform / observer / doll): ReceiveTick
/// (DeltaSeconds) while PrimaryActorTick is enabled (SetActorTickEnabled; the CDO's PrimaryActorTick
/// .bStartWithTickEnabled, AActor default true). Also GFrameCounter for GetCurrentFrameBP. Called by host.rs per UI frame.
pub fn tick(vm: &mut Vm, dt: f64) {
    let Some(&m) = vm.world.get("map") else { return };
    // once per UI frame (host.rs and the Bevy ArmoryPlugin may both call it): keyed on the VM clock
    if matches!(vm.prop(m, "__armory_last"), V::Float(t) if t == vm.time) {
        return;
    }
    let now = vm.time;
    vm.set(m, "__armory_last", V::Float(now));
    let f = vm.prop(m, FRAME).i();
    vm.set(m, FRAME, V::Int(f + 1));
    for a in actors(vm) {
        if vm.alive(a) && vm.prop(a, "__tick").truthy() {
            vm.event(a, "ReceiveTick", vec![V::Float(dt)]);
        }
    }
}

/// the actor's (or component's) world transform: own location / rotation, or attached: relative composed with the
/// parent component's world (AActor::K2_AttachToComponent with KeepRelative rules, the BP passes 1,1,1 = KeepWorld
/// UNCONFIRMED; the Blueprints set the relative location / rotation right after attaching, so relative is what counts)
pub fn world_xf(vm: &Vm, id: Id) -> Xf {
    world_xf_d(vm, id, 0)
}

fn world_xf_d(vm: &Vm, id: Id, d: u32) -> Xf {
    if d > 16 {
        // an attachment cycle (UE refuses those in AttachToComponent); stop at the local transform
        return Xf::IDENTITY;
    }
    if let V::Struct(_) = vm.prop(id, "__world") {
        return Xf::from_v(&vm.prop(id, "__world"));
    }
    if let Some(p) = vm.prop(id, "__parent").obj().filter(|&p| vm.alive(p)) {
        let rel = Xf::from_loc_rot(vec_of(&vm.prop(id, "__rel_loc")), rot_of(&vm.prop(id, "__rel_rot")));
        return rel.compose(world_xf_d(vm, p, d + 1));
    }
    if let Some(o) = vm.prop(id, "__owner").obj().filter(|&o| vm.alive(o) && o != id) {
        return world_xf_d(vm, o, d + 1);
    }
    match vm.prop(id, "__transform") {
        V::Struct(_) => Xf::from_v(&vm.prop(id, "__transform")),
        _ => Xf::IDENTITY,
    }
}

fn set_world(vm: &mut Vm, id: Id, x: Xf) {
    vm.set(id, "__transform", x.to_v());
    // a world placement detaches the relative pair from the parent: store as relative to the parent's world
    if let Some(p) = vm.prop(id, "__parent").obj().filter(|&p| vm.alive(p)) {
        let pw = world_xf(vm, p);
        let l = pw.inverse_apply(x.t);
        let q = qmul(uq::quat_inverse(pw.q), x.q);
        vm.set(id, "__rel_loc", vec_v(l));
        vm.set(id, "__rel_rot", rot_v(uq::quat_rotator(q)));
    }
}

/// instance the actor's components: every CDO member that references a component template of the class's package
/// becomes an object owned by the actor (component relative transforms: UNCONFIRMED, taken as identity)
fn instance_components(vm: &mut Vm, id: Id) {
    let cdo: Vec<(String, V)> = vm.o(id).class.cdo.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (k, v) in cdo {
        if let V::Asset(a) = &v {
            if a.class.ends_with("Component") && !a.package.is_empty() {
                let c = vm.native_class(&a.class);
                let cid = vm.new_obj(c, &k);
                vm.set(cid, "__owner", V::Obj(id));
                vm.set(id, &k, V::Obj(cid));
            }
        }
    }
}

/// USimpleConstructionScript::ExecuteScriptOnActor (UE 4.26 SimpleConstructionScript.cpp, run by
/// AActor::ExecuteConstruction for a Blueprint class, parents' scripts first): every SCS_Node creates its component
/// (ComponentClass, named InternalVariableName, the ComponentTemplate's properties) on the actor, attached to the
/// component of the node that lists it in ChildNodes (root nodes: the actor's root); the class member of that name
/// refers to it. Relative location / rotation come from the template. A CameraComponent's FieldOfView is the
/// UCameraComponent ctor's 90 unless the template sets it.
fn instance_scs(vm: &mut Vm, id: Id) {
    let mut chain = vec![];
    let mut c = Some(vm.o(id).class.clone());
    while let Some(k) = c {
        if !k.package.is_empty() {
            chain.push(k.package.clone());
        }
        c = k.parent.clone();
    }
    let idx = |v: Option<&serde_json::Value>| -> Option<usize> { v?.get("ObjectPath")?.as_str()?.rsplit_once('.')?.1.parse().ok() };
    for pkg in chain.into_iter().rev() {
        let Some(ex) = vm.json(&pkg) else { continue };
        // node export index -> (variable name, component class, template export index)
        let mut nodes: Vec<(usize, String, String, Option<usize>)> = vec![];
        let mut parent_of: std::collections::HashMap<usize, usize> = Default::default();
        for (i, e) in ex.iter().enumerate() {
            if e.get("Type").and_then(|t| t.as_str()) != Some("SCS_Node") {
                continue;
            }
            let p = e.get("Properties").cloned().unwrap_or_default();
            let var = p.get("InternalVariableName").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let cls = p.pointer("/ComponentClass/ObjectName").and_then(|v| v.as_str()).unwrap_or("").trim_start_matches("Class'").trim_end_matches('\'').to_string();
            for ch in p.get("ChildNodes").and_then(|c| c.as_array()).into_iter().flatten() {
                if let Some(ci) = idx(Some(ch)) {
                    parent_of.insert(ci, i);
                }
            }
            nodes.push((i, var, cls, idx(p.get("ComponentTemplate"))));
        }
        let mut made: std::collections::HashMap<usize, Id> = Default::default();
        for (i, var, cls, tpl) in &nodes {
            if var.is_empty() || vm.prop(id, var).obj().is_some_and(|o| vm.alive(o)) {
                continue;
            }
            let k = vm.native_class(cls.trim_end_matches("_C"));
            let cid = vm.new_obj(k, var);
            vm.set(cid, "__owner", V::Obj(id));
            if cls == "CameraComponent" {
                vm.set(cid, "FieldOfView", V::Float(90.0));
            }
            if let Some(props) = tpl.and_then(|t| ex.get(t)).and_then(|t| t.get("Properties")).and_then(|p| p.as_object()) {
                for (pk, pv) in props {
                    vm.set(cid, pk, crate::model::json_v(pv, &|_| None));
                }
                if let Some(l) = props.get("RelativeLocation") {
                    vm.set(cid, "__rel_loc", crate::model::json_v(l, &|_| None));
                }
                if let Some(r) = props.get("RelativeRotation") {
                    vm.set(cid, "__rel_rot", crate::model::json_v(r, &|_| None));
                }
            }
            vm.set(id, var, V::Obj(cid));
            made.insert(*i, cid);
        }
        for (child, parent) in parent_of {
            if let (Some(&c), Some(&p)) = (made.get(&child), made.get(&parent)) {
                vm.set(c, "__parent", V::Obj(p));
            }
        }
    }
}

/// host input: a level actor of `class_name` (e.g. "BP_CharacterCustomizationSpot_C") at `actor` with its
/// AttachComponent at `attach` (world transforms from the level: mh-level placements). GetAllActorsOfClass returns
/// these to the platform Blueprint (UpdateCamera / SpawnCharacterDollIfNone / SpawnEquipment).
pub fn set_level_actor(vm: &mut Vm, class_name: &str, actor: Xf, attach: Xf) {
    let key = format!("__level:{class_name}");
    let id = match vm.world.get("map").map(|&m| vm.prop(m, &key)).and_then(|v| v.obj()) {
        Some(i) => i,
        None => {
            let c = vm.native_class(class_name.trim_end_matches("_C"));
            let i = vm.new_obj(c, class_name);
            let ac = vm.native_class("SceneComponent");
            let comp = vm.new_obj(ac, "AttachComponent");
            vm.set(i, "AttachComponent", V::Obj(comp));
            if let Some(&m) = vm.world.get("map") {
                vm.set(m, &key, V::Obj(i));
            }
            i
        }
    };
    vm.set(id, "__world", actor.to_v());
    if let Some(c) = vm.prop(id, "AttachComponent").obj() {
        vm.set(c, "__world", attach.to_v());
    }
}

/// host input: a socket's world location on the preview doll's mesh (the renderer has the posed skeleton; the
/// platform's UpdateCamera / UpdateCharacterDollRotation read "Mandible")
pub fn set_doll_socket(vm: &mut Vm, socket: &str, p: FVector) {
    if let Some(&m) = vm.world.get("map") {
        vm.set(m, &format!("__socket:{socket}"), vec_v(p));
    }
}

/// host input: an equipment class's skeletal mesh bounds in component space (FBoxSphereBounds Origin / BoxExtent /
/// SphereRadius, UE cm) for AMordhauEquipment::ComputeAccurateBounds; `pkg` is the Blueprint package
/// ("Mordhau/Content/.../BP_Longsword")
pub fn set_equipment_bounds(vm: &mut Vm, pkg: &str, origin: FVector, extent: FVector, radius: f32) {
    if let Some(&m) = vm.world.get("map") {
        vm.set(m, &format!("__bounds:{pkg}"), V::st(&[("Origin", vec_v(origin)), ("BoxExtent", vec_v(extent)), ("SphereRadius", V::Float(radius as f64))]));
    }
}

/// UMordhauSingleton::SpawnEquipmentFromClass (UMordhauSingleton.cpp 4519-4657): an actor of the item's class with
/// AssignedCustomization set, registered with the UI's actors (no tick); its meshes are the renderer's
fn spawn_equipment(vm: &mut Vm, pkg: &str, cust: V) -> Option<Id> {
    let cls = vm.bp_class(pkg)?;
    let id = vm.new_obj(cls, pkg.rsplit('/').next().unwrap_or("Equipment"));
    vm.set(id, "AssignedCustomization", cust);
    instance_components(vm, id);
    instance_scs(vm, id);
    vm.set(id, "__tick", V::Bool(false));
    register(vm, id);
    Some(id)
}

/// the CRT rand() the exe calls (MSVC: state = state * 214013 + 2531011, result (state >> 16) & 0x7fff). The seed is
/// the engine's (FMath::RandInit with the startup time): UNCONFIRMED, seeded here from the clock once per VM
fn crt_rand(vm: &mut Vm) -> u32 {
    let Some(&m) = vm.world.get("map") else { return 0 };
    let mut s = match vm.prop(m, "__crt_rand") {
        V::Int(s) => s as u32,
        _ => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1),
    };
    s = s.wrapping_mul(214013).wrapping_add(2531011);
    vm.set(m, "__crt_rand", V::Int(s as i64));
    (s >> 16) & 0x7fff
}

/// UMordhauSingleton::MakeEmptyProfile(out Profile, Character, bRandomizeVoice) (UMordhauSingleton.cpp 3589-3797):
/// FCharacterProfile() with PlayFabID = GetPlayFabID() (offline: empty); with the character's
/// FaceCustomizationComponent: Face Translate / Rotate / Scale = its BakedDefaultFaceValues*; Wearables[Head /
/// UpperChest / Legs].Id = the singleton's DefaultHead / DefaultUpperChest / DefaultLegs, every other slot (mask 0x85
/// skips 0, 2, 7) = its GetWearableClass default; bRandomizeVoice: VoicePitch = min(255, int(rand() * 0.007812738)),
/// Voice = a random index among the MaleVoices the inventory owns (offline: all) =
/// min(n - 1, int(rand() * 3.051851e-05 * n)).
/// The starting value is BP_MordhauSingleton's CDO CharacterProfiles[0] (the shipped "Unnamed" profile, which carries
/// the FCharacterProfile defaults and the baked default face: UNCONFIRMED that the CDO's face equals
/// BP_MordhauCharacter's BakedDefaultFaceValues)
pub fn make_empty_profile(vm: &mut Vm, randomize_voice: bool) -> V {
    let Some(so) = vm.world.get("singleton").copied() else { return V::None };
    let mut p = vm.o(so).class.cdo.get("CharacterProfiles").and_then(|v| v.arr().first().cloned()).unwrap_or_default();
    *field_mut(&mut p, "PlayFabID") = V::Str(String::new());
    let mut gear = p.field("GearCustomization").clone();
    for (s, k) in [(armory::HEAD, "DefaultHead"), (armory::UPPER_CHEST, "DefaultUpperChest"), (armory::LEGS, "DefaultLegs")] {
        let d = vm.prop(so, k).i();
        if let Some(w) = elem_mut(field_mut(&mut gear, "Wearables"), s) {
            *field_mut(w, "Id") = V::Int(d);
        }
    }
    for s in 0..armory::SLOTS {
        if (0x85u32 >> s) & 1 != 0 {
            continue;
        }
        let (_, d) = armory::wearable_class(vm, &gear, s);
        if let Some(w) = elem_mut(field_mut(&mut gear, "Wearables"), s) {
            *field_mut(w, "Id") = V::Int(d);
        }
    }
    *field_mut(&mut p, "GearCustomization") = gear;
    if randomize_voice {
        let pitch = ((crt_rand(vm) as f32 * 0.007812738) as i64).min(255);
        *field_mut(field_mut(&mut p, "AppearanceCustomization"), "VoicePitch") = V::Int(pitch);
        let n = vm.prop(so, "MaleVoices").arr().iter().filter(|v| armory::class_pkg(v).is_some()).count() as i64;
        if n > 0 {
            let k = ((crt_rand(vm) as f32 * 3.051851e-05 * n as f32) as i64).min(n - 1);
            // the owned list holds the MaleVoices indices in order; offline every valid class is owned
            let idx = vm.prop(so, "MaleVoices").arr().iter().enumerate().filter(|(_, v)| armory::class_pkg(v).is_some()).nth(k as usize).map(|(i, _)| i as i64).unwrap_or(0);
            *field_mut(field_mut(&mut p, "AppearanceCustomization"), "Voice") = V::Int(idx);
        }
    }
    p
}

/// a class default object as JSON (the shape mh-character's from_defaults readers take)
fn cdo_json(vm: &mut Vm, pkg: &str) -> serde_json::Value {
    let m = armory::cdo(vm, pkg).unwrap_or_default();
    armory::to_json(&V::Struct(Box::new(m.into_iter().collect())))
}

/// UMordhauMovementComponent::GetSpeedFactor for a character wearing / carrying `profile`:
/// - UpdateEquipmentSpeedAndAcceleration rva 0x14dcd30 (mh_character ExeMovement port): the right hand's
///   SpeedOverrideEquipped / SpeedBonusPercentageEquipped / SubSprintSpeedBonusEquipped, every other carried item's
///   SpeedBonusPercentageHolstered. Which item the doll holds: Equipment[0] in the right hand, the rest holstered
///   (UNCONFIRMED: AMordhauCharacter's spawn equip order for the doll);
/// - UpdateArmorSpeedAndAcceleration rva 0x14dc0c0 (mh_character::equipment::ExeMovement::update_armor_speed_and_
///   acceleration): 1 - (the sum of the Head / UpperChest / Legs wearables' SpeedFactor minus the smallest); no game
///   override, no Tank perk check here (the Armory has no game-state override; Tank = perk bit 6: UNCONFIRMED);
/// - GetSpeedFactor: s = bonus + armor + (1 - t) * sub-sprint bonus, min(override) when the override > 0, max 0.
pub fn armory_speed_factor(vm: &mut Vm, profile: &V, t: f32) -> f32 {
    use mh_character::equipment::{ArmorWearable, EquipmentMovement};
    let gear = profile.field("GearCustomization").clone();
    let mut armor = [0.0f32; 3];
    for (k, s) in [armory::HEAD, armory::UPPER_CHEST, armory::LEGS].into_iter().enumerate() {
        if let (Some(c), _) = armory::wearable_class(vm, &gear, s) {
            armor[k] = ArmorWearable::from_defaults(&cdo_json(vm, &c)).speed_factor;
        }
    }
    let (h, u, l) = (armor[0], armor[1], armor[2]);
    let minf = |a: f32, b: f32| if a < b { a } else { b };
    let armor_speed = 1.0 - (((u + h) + l) - minf(minf(u, h), l));
    let (mut cap, mut add, mut sub) = (0.0f32, 0.0f32, 0.0f32);
    for (i, e) in gear.field("Equipment").arr().iter().enumerate() {
        let Some(c) = armory::equipment_class(vm, e.field("Id").i()) else { continue };
        let m = EquipmentMovement::from_defaults(&c, &cdo_json(vm, &c));
        if i == 0 {
            if m.speed_override_equipped > 0.0 {
                cap = m.speed_override_equipped;
            }
            add += m.speed_bonus_percentage_equipped;
            sub += m.sub_sprint_speed_bonus_equipped;
        } else {
            add += m.speed_bonus_percentage_holstered;
        }
    }
    let mut s = add + armor_speed + (1.0 - t) * sub;
    if cap > 0.0 && cap < s {
        s = cap;
    }
    if s <= 0.0 {
        0.0
    } else {
        s
    }
}

fn this(ctx: Ctx) -> Option<Id> {
    match ctx {
        Ctx::Obj(i) => Some(i),
        _ => None,
    }
}

/// the actor a component / actor id belongs to
fn actor_of(vm: &Vm, id: Id) -> Id {
    vm.prop(id, "__owner").obj().filter(|&o| vm.alive(o)).unwrap_or(id)
}

/// mutable access to a nested struct field / array element of a profile value (created when absent)
fn field_mut<'a>(v: &'a mut V, k: &str) -> &'a mut V {
    if !matches!(v, V::Struct(_)) {
        *v = V::Struct(Box::default());
    }
    match v {
        V::Struct(m) => m.entry(k.to_string()).or_insert(V::None),
        _ => unreachable!(),
    }
}
fn elem_mut(v: &mut V, i: usize) -> Option<&mut V> {
    match v {
        V::Array(a) => a.get_mut(i),
        _ => None,
    }
}

/// UCharacterProfileBPWrapper::TryCarryOverEquipmentData (UCharacterProfileBPWrapper.cpp 143-202): when both items'
/// Skins[Skin] exist, New.Pattern = the index in the new skin's Patterns whose Texture equals the old pattern's, and each
/// colour index whose ColorTables entry is the same table carries over
fn carry_over_equipment(vm: &mut Vm, old: &V, new: &mut V) {
    let (Some(oc), Some(nc)) = (armory::equipment_class(vm, old.field("Id").i()), armory::equipment_class(vm, new.field("Id").i())) else { return };
    let Some(od) = armory::cdo(vm, &oc) else { return };
    let Some(nd) = armory::cdo(vm, &nc) else { return };
    let skins = |d: &std::collections::HashMap<String, V>, i: i64| d.get("Skins").and_then(|s| s.arr().get(i.max(0) as usize).cloned());
    let (Some(os), Some(ns)) = (skins(&od, old.field("Skin").i()), skins(&nd, new.field("Skin").i())) else { return };
    if let Some(op) = os.field("Patterns").arr().get(old.field("Pattern").i().max(0) as usize).map(|p| p.field("Texture").clone()) {
        if let Some(i) = ns.field("Patterns").arr().iter().position(|p| p.field("Texture").same(&op)) {
            *field_mut(new, "Pattern") = V::Int(i as i64);
        }
    }
    for c in 0..3 {
        let (a, b) = (os.field("ColorTables").arr().get(c).map(V::i), ns.field("ColorTables").arr().get(c).map(V::i));
        if a.is_some() && a == b {
            let ov = old.field("Colors").arr().get(c).cloned().unwrap_or(V::Int(0));
            if let Some(e) = elem_mut(field_mut(new, "Colors"), c) {
                *e = ov;
            }
        }
    }
}

/// FEquipmentCustomization::FEquipmentCustomization rva 0x1527320: Id 0, Colors / Parts three 0 bytes, Pattern 0, Skin 0
fn empty_equipment() -> V {
    let z = || V::Array(vec![V::Int(0), V::Int(0), V::Int(0)]);
    V::st(&[("Id", V::Int(0)), ("Colors", z()), ("Parts", z()), ("Pattern", V::Int(0)), ("Skin", V::Int(0))])
}

/// the wrapper's Profile.GearCustomization.Equipment[slot] replaced by `f(old)` (the exec thunks' shared shape)
fn edit_equipment(vm: &mut Vm, w: Id, slot: usize, f: impl FnOnce(&mut Vm, &V) -> Option<V>) {
    let mut p = vm.prop(w, "Profile");
    let old = p.field("GearCustomization").field("Equipment").arr().get(slot).cloned();
    let Some(old) = old else { return };
    let Some(nv) = f(vm, &old) else { return };
    if let Some(e) = elem_mut(field_mut(field_mut(&mut p, "GearCustomization"), "Equipment"), slot) {
        *e = nv;
    }
    vm.set(w, "Profile", p);
}

fn edit_profile(vm: &mut Vm, w: Id, f: impl FnOnce(&mut V)) {
    let mut p = vm.prop(w, "Profile");
    f(&mut p);
    vm.set(w, "Profile", p);
}

/// UCharacterProfileBPWrapper::TryCarryOverWearableData (UCharacterProfileBPWrapper.cpp 8-138) on gear values:
/// bMatchId: if the old slot's class is in the new gear's list for the slot (GetWearableArray), the whole old item
/// carries over with Id = that index; otherwise (or not found), with both classes present: the pattern whose Texture
/// matches the old one (Patterns +0x1c8) and Colors[0] / [1] when ColorTables[0] / [1] (+0x1f0) are equal
fn carry_over_wearable(vm: &mut Vm, slot: usize, old: &V, new: &mut V, match_id: bool) {
    let (oc, _) = armory::wearable_class(vm, old, slot);
    let (nc, _) = armory::wearable_class(vm, new, slot);
    if match_id {
        if let (Some(o), Some((list, _))) = (&oc, armory::wearable_list(vm, new, slot)) {
            if let Some(i) = list.iter().position(|c| armory::class_pkg(c).as_deref() == Some(o.as_str())) {
                let mut w = old.field("Wearables").arr().get(slot).cloned().unwrap_or_default();
                *field_mut(&mut w, "Id") = V::Int(i as i64);
                if let Some(e) = elem_mut(field_mut(new, "Wearables"), slot) {
                    *e = w;
                }
                return;
            }
        }
    }
    let (Some(oc), Some(nc)) = (oc, nc) else { return };
    let Some(od) = armory::cdo(vm, &oc) else { return };
    let Some(nd) = armory::cdo(vm, &nc) else { return };
    let ow = old.field("Wearables").arr().get(slot).cloned().unwrap_or_default();
    let arr = |d: &std::collections::HashMap<String, V>, k: &str| d.get(k).map(|v| v.arr().to_vec()).unwrap_or_default();
    let (op, np) = (arr(&od, "Patterns"), arr(&nd, "Patterns"));
    let pat = ow.field("Pattern").i().max(0) as usize;
    let Some(nw) = elem_mut(field_mut(new, "Wearables"), slot) else { return };
    if let Some(t) = op.get(pat).map(|p| p.field("Texture").clone()) {
        if let Some(i) = np.iter().position(|p| p.field("Texture").same(&t)) {
            *field_mut(nw, "Pattern") = V::Int(i as i64);
        }
    }
    let (oct, nct) = (arr(&od, "ColorTables"), arr(&nd, "ColorTables"));
    for c in 0..2 {
        // the byte compare reads index c of each ColorTables array (absent = the zeroed byte: UNCONFIRMED for empty arrays)
        if oct.get(c).map(V::i).unwrap_or(0) == nct.get(c).map(V::i).unwrap_or(0) {
            let v = ow.field("Colors").arr().get(c).cloned().unwrap_or(V::Int(0));
            if let Some(e) = elem_mut(field_mut(nw, "Colors"), c) {
                *e = v;
            }
        }
    }
}

/// a dependent slot reset after its parent changed (SetWearableId's per-slot block / HandleSubSlotWearableIdChanged
/// rva 0x1687bf0): GetWearableClass(new gear, Slot, &Default); Wearables[Slot] = FWearableCustomization() (0x1528610)
/// with Id = Default; TryCarryOverWearableData(Slot, Old, New, true)
fn reset_sub_slot(vm: &mut Vm, slot: usize, old: &V, new: &mut V) {
    let (_, def) = armory::wearable_class(vm, new, slot);
    if let Some(e) = elem_mut(field_mut(new, "Wearables"), slot) {
        *e = armory::empty_wearable(def);
    }
    carry_over_wearable(vm, slot, old, new, true);
}

/// SListView::SetSelection + UListView::OnSelectionChangedInternal (see the BP_SetSelectedItem arm)
fn list_select(vm: &mut Vm, l: Id, old: V, new: V) {
    if old.same(&new) {
        return;
    }
    vm.set(l, "__selected", new.clone());
    // re-entrancy guard: a selection handler that reselects (Slate's would recurse until the stack ends) stops here
    let depth = vm.prop(l, "__select_depth").i();
    if depth >= 8 {
        eprintln!("armory: ListView {} selection re-entered {depth} deep; not broadcasting", vm.o(l).name);
        return;
    }
    vm.set(l, "__select_depth", V::Int(depth + 1));
    list_select_notify(vm, l, old, new);
    vm.set(l, "__select_depth", V::Int(depth));
}

fn list_select_notify(vm: &mut Vm, l: Id, old: V, new: V) {
    let items = vm.prop(l, "ListItems").arr().to_vec();
    let entries = vm.prop(l, "__entries").arr().to_vec();
    for (it, e) in items.iter().zip(entries.iter()) {
        let Some(e) = e.obj().filter(|&e| vm.alive(e)) else { continue };
        if !matches!(old, V::None) && it.same(&old) {
            vm.event(e, "BP_OnItemSelectionChanged", vec![V::Bool(false)]);
        }
        if !matches!(new, V::None) && it.same(&new) {
            vm.event(e, "BP_OnItemSelectionChanged", vec![V::Bool(true)]);
        }
    }
    if let V::Multi(m) = vm.prop(l, "BP_OnItemSelectionChanged") {
        let sel = !matches!(new, V::None);
        for (o, f) in m {
            vm.call_named(o, &f, vec![new.clone(), V::Bool(sel)]);
        }
    }
}

pub fn call(vm: &mut Vm, ctx: Ctx, class: &str, name: &str, a: &[V]) -> Option<NRet> {
    let a0 = a.first().cloned().unwrap_or_default();
    let a1 = a.get(1).cloned().unwrap_or_default();
    let a2 = a.get(2).cloned().unwrap_or_default();
    let t = this(ctx).filter(|&i| vm.alive(i));
    let is = |vm: &Vm, n: &str| t.is_some_and(|i| vm.o(i).class.isa(n));
    let actorish = |vm: &Vm| t.is_some_and(|i| !vm.o(i).class.isa("Widget") && !vm.o(i).class.isa("Visual"));
    match name {
        // ---- UKismetMathLibrary: vectors, rotators, transforms ----
        "MakeTransform" => r(Xf { t: vec_of(&a0), q: { let (p, y, r) = rot_of(&a1); uq::rotator_quaternion(p, y, r) }, s: match &a2 { V::None => FVector::new(1.0, 1.0, 1.0), s => vec_of(s) } }.to_v()),
        "BreakTransform" => {
            let x = Xf::from_v(&a0);
            outs(V::None, vec![(1, vec_v(x.t)), (2, rot_v(x.rotator())), (3, vec_v(x.s))])
        }
        "ComposeTransforms" => r(Xf::from_v(&a0).compose(Xf::from_v(&a1)).to_v()),
        "InverseTransformLocation" => r(vec_v(Xf::from_v(&a0).inverse_apply(vec_of(&a1)))),
        "TransformLocation" => r(vec_v(Xf::from_v(&a0).apply(vec_of(&a1)))),
        // UKismetMathLibrary::MakeRotator(Roll, Pitch, Yaw)
        "MakeRotator" => r(rot_v((a1.f() as f32, a2.f() as f32, a0.f() as f32))),
        // ComposeRotators(A, B) = FRotator(FQuat(B) * FQuat(A))
        "ComposeRotators" => {
            let (p, y, rr) = rot_of(&a0);
            let (p2, y2, r2) = rot_of(&a1);
            r(rot_v(uq::quat_rotator(qmul(uq::rotator_quaternion(p2, y2, r2), uq::rotator_quaternion(p, y, rr)))))
        }
        // GetForwardVector = FRotator::Vector; GetRightVector / GetUpVector = FRotationMatrix axes Y / Z
        "GetForwardVector" if class == "KismetMathLibrary" || !actorish(vm) => {
            let (p, y, _) = rot_of(&a0);
            r(vec_v(uq::rotator_vector(p, y)))
        }
        "GetRightVector" if class == "KismetMathLibrary" || !actorish(vm) => {
            let (p, y, rr) = rot_of(&a0);
            r(vec_v(uq::rotation_matrix(p, y, rr)[1]))
        }
        "GetUpVector" if class == "KismetMathLibrary" || !actorish(vm) => {
            let (p, y, rr) = rot_of(&a0);
            r(vec_v(uq::rotation_matrix(p, y, rr)[2]))
        }
        "Add_VectorVector" => r(vec_v(vec_of(&a0) + vec_of(&a1))),
        "Subtract_VectorVector" => r(vec_v(vec_of(&a0) - vec_of(&a1))),
        "Multiply_VectorFloat" => {
            let (p, f) = (vec_of(&a0), a1.f() as f32);
            r(vec_v(FVector::new(p.x * f, p.y * f, p.z * f)))
        }
        "Divide_VectorFloat" => {
            let (p, f) = (vec_of(&a0), a1.f() as f32);
            r(vec_v(if f == 0.0 { FVector::ZERO } else { FVector::new(p.x / f, p.y / f, p.z / f) }))
        }
        // VLerp(A, B, Alpha) = A + Alpha * (B - A)
        "VLerp" => {
            let (x, y, al) = (vec_of(&a0), vec_of(&a1), a2.f() as f32);
            r(vec_v(FVector::new(x.x + al * (y.x - x.x), x.y + al * (y.y - x.y), x.z + al * (y.z - x.z))))
        }
        "Conv_BoolToFloat" => r(V::Float(if a0.truthy() { 1.0 } else { 0.0 })),
        // ---- GameplayStatics actor spawning ----
        // BeginDeferredActorSpawnFromClass(WorldContext, ActorClass, SpawnTransform, CollisionHandling, Owner): the actor
        // exists (ExposeOnSpawn / SetXPropertyByName may set members) but BeginPlay waits for FinishSpawningActor
        "BeginDeferredActorSpawnFromClass" => {
            let V::Asset(c) = a1 else { return r(V::None) };
            let cls = vm.class(&c);
            let id = vm.new_obj(cls, c.name.trim_end_matches("_C"));
            vm.set(id, "__transform", a2);
            instance_components(vm, id);
            instance_scs(vm, id);
            r(V::Obj(id))
        }
        // FinishSpawningActor(Actor, SpawnTransform): AActor::FinishSpawning -> PostActorConstruction -> BeginPlay (the
        // world has begun play) -> ReceiveBeginPlay; ticking from now on as PrimaryActorTick says
        "FinishSpawningActor" => {
            let Some(id) = a0.obj().filter(|&i| vm.alive(i)) else { return r(V::None) };
            vm.set(id, "__transform", a1);
            let start = vm.prop(id, "PrimaryActorTick").field("bStartWithTickEnabled").clone();
            vm.set(id, "__tick", V::Bool(!matches!(start, V::Bool(false))));
            register(vm, id);
            vm.event(id, "ReceiveBeginPlay", vec![]);
            r(V::Obj(id))
        }
        // GetAllActorsOfClass(WorldContext, ActorClass, out Actors): the level's customization spots (host input) and
        // the actors the UI spawned; any other class falls through to natives.rs
        "GetAllActorsOfClass" => {
            let V::Asset(c) = &a1 else { return None };
            let key = format!("__level:{}", c.name);
            let mut found: Vec<V> = vm.world.get("map").map(|&m| vm.prop(m, &key)).into_iter().filter(|v| v.obj().is_some()).collect();
            for x in actors(vm) {
                if vm.alive(x) && (vm.o(x).class.isa(&c.name) || vm.o(x).class.isa(c.name.trim_end_matches("_C"))) {
                    found.push(V::Obj(x));
                }
            }
            if found.is_empty() && !c.name.contains("Customization") {
                return None;
            }
            outs(V::None, vec![(2, V::Array(found))])
        }
        "SetActorTickEnabled" if t.is_some() => {
            vm.set(t.unwrap(), "__tick", V::Bool(a0.truthy()));
            r(V::None)
        }
        "IsActorTickEnabled" if t.is_some() => r(V::Bool(vm.prop(t.unwrap(), "__tick").truthy())),
        // AActor::K2_DestroyActor: EndPlay then pending kill (IsValid false from now on)
        "K2_DestroyActor" if actorish(vm) => {
            let id = t.unwrap();
            vm.event(id, "ReceiveEndPlay", vec![V::Int(0)]);
            vm.om(id).alive = false;
            r(V::None)
        }
        // ---- actor / component transforms ----
        "K2_GetActorLocation" | "K2_GetComponentLocation" if t.is_some() => r(vec_v(world_xf(vm, t.unwrap()).t)),
        "K2_GetActorRotation" | "K2_GetComponentRotation" if t.is_some() => r(rot_v(world_xf(vm, t.unwrap()).rotator())),
        "GetTransform" | "K2_GetComponentToWorld" | "GetActorTransform" if t.is_some() => r(world_xf(vm, t.unwrap()).to_v()),
        "K2_SetActorLocationAndRotation" if t.is_some() => {
            let id = actor_of(vm, t.unwrap());
            let s = world_xf(vm, id).s;
            set_world(vm, id, Xf { s, ..Xf::from_loc_rot(vec_of(&a0), rot_of(&a1)) });
            outs(V::Bool(true), vec![(3, V::Struct(Box::default()))])
        }
        "K2_SetActorLocation" if t.is_some() => {
            let id = actor_of(vm, t.unwrap());
            let w = world_xf(vm, id);
            set_world(vm, id, Xf { t: vec_of(&a0), ..w });
            outs(V::Bool(true), vec![(2, V::Struct(Box::default()))])
        }
        "K2_SetActorRotation" if t.is_some() => {
            let id = actor_of(vm, t.unwrap());
            let w = world_xf(vm, id);
            let (p, y, rr) = rot_of(&a0);
            set_world(vm, id, Xf { q: uq::rotator_quaternion(p, y, rr), ..w });
            r(V::Bool(true))
        }
        "K2_SetActorTransform" if t.is_some() => {
            let id = actor_of(vm, t.unwrap());
            set_world(vm, id, Xf::from_v(&a0));
            outs(V::Bool(true), vec![(2, V::Struct(Box::default()))])
        }
        "K2_AddActorWorldOffset" if t.is_some() => {
            let id = actor_of(vm, t.unwrap());
            let w = world_xf(vm, id);
            set_world(vm, id, Xf { t: w.t + vec_of(&a0), ..w });
            outs(V::None, vec![(2, V::Struct(Box::default()))])
        }
        "K2_SetActorRelativeLocation" if t.is_some() => {
            let id = actor_of(vm, t.unwrap());
            vm.set(id, "__rel_loc", a0);
            outs(V::None, vec![(2, V::Struct(Box::default()))])
        }
        "K2_SetActorRelativeRotation" if t.is_some() => {
            let id = actor_of(vm, t.unwrap());
            vm.set(id, "__rel_rot", a0);
            outs(V::None, vec![(2, V::Struct(Box::default()))])
        }
        // K2_AttachToComponent(Parent, SocketName, LocationRule, RotationRule, ScaleRule, bWeldSimulatedBodies)
        "K2_AttachToComponent" | "K2_AttachToActor" if t.is_some() && actorish(vm) => {
            let id = actor_of(vm, t.unwrap());
            let parent = a0.obj();
            let w = world_xf(vm, id);
            vm.set(id, "__parent", parent.map(V::Obj).unwrap_or_default());
            set_world(vm, id, w);
            r(V::Bool(true))
        }
        "K2_DetachFromActor" if t.is_some() && actorish(vm) => {
            let id = actor_of(vm, t.unwrap());
            let w = world_xf(vm, id);
            vm.set(id, "__parent", V::None);
            vm.set(id, "__transform", w.to_v());
            r(V::None)
        }
        // USkinnedMeshComponent::GetSocketLocation: the renderer's posed socket (set_doll_socket), else the component's
        // location (UNCONFIRMED stand-in until the host reports the socket)
        "GetSocketLocation" if t.is_some() => {
            let s = a0.s();
            let host = vm.world.get("map").map(|&m| vm.prop(m, &format!("__socket:{s}")));
            match host {
                Some(v @ V::Struct(_)) => r(v),
                _ => r(vec_v(world_xf(vm, t.unwrap()).t)),
            }
        }
        // GFrameCounter (UMordhauUtilityLibrary::GetCurrentFrameBP / AMordhauGameState::GetCurrentFrame)
        "GetCurrentFrameBP" | "GetCurrentFrame" => r(V::Int(vm.world.get("map").map(|&m| vm.prop(m, FRAME).i()).unwrap_or(0))),
        "GetWorldOf" => r(V::None),
        // ---- the singleton (UMordhauSingleton) ----
        "SaveToConfig" if is(vm, "MordhauSingleton") => {
            if let Err(e) = armory::save_config(vm, t.unwrap()) {
                eprintln!("armory: SaveToConfig {}: {e}", armory::game_ini().display());
            }
            r(V::None)
        }
        "LoadFromConfig" if is(vm, "MordhauSingleton") => {
            armory::load_config(vm, t.unwrap());
            r(V::None)
        }
        // UMordhauSingleton::ApplyProfileTo(Profile, Character, ...) rva 0x15c4910 (UMordhauSingleton.cpp 4138-4251):
        // the character's Profile is the applied one; building the meshes is the renderer's (armory::preview)
        // ApplyProfileTo(Profile, Char, Team, bAddEquipment): Char.Equipment[0..3] dropped (DropSlot) and destroyed,
        // AssignProfile, SetQuiver(None); with bAddEquipment, for each Profile.GearCustomization.Equipment[i] whose Id
        // indexes the singleton's Equipment: SpawnEquipmentFromClass, bForceInstantMeshUpdate, AssignCustomization(item,
        // Emblem, EmblemColors[0], EmblemColors[1]), its Quiver class onto the character when it has none, then
        // PickUp(item, i) -> Char.Equipment[i]
        "ApplyProfileTo" if is(vm, "MordhauSingleton") => {
            let Some(c) = a1.obj().filter(|&c| vm.alive(c)) else { return r(V::None) };
            let mut held = vm.prop(c, "Equipment").arr().to_vec();
            held.resize(3, V::None);
            for h in held.iter_mut() {
                if let Some(e) = h.obj().filter(|&e| vm.alive(e)) {
                    vm.event(e, "ReceiveEndPlay", vec![V::Int(0)]);
                    vm.om(e).alive = false;
                }
                *h = V::None;
            }
            vm.set(c, "Profile", a0.clone());
            vm.set(c, "__applied", V::Bool(true));
            vm.set(c, "Quiver", V::None);
            if a.get(3).map(V::truthy).unwrap_or(true) {
                let ap = a0.field("AppearanceCustomization").clone();
                let cols = ap.field("EmblemColors").arr().to_vec();
                for (i, cust) in a0.field("GearCustomization").field("Equipment").arr().to_vec().into_iter().enumerate().take(3) {
                    let Some(pkg) = armory::equipment_class(vm, cust.field("Id").i()) else { continue };
                    let Some(e) = spawn_equipment(vm, &pkg, cust) else { continue };
                    vm.set(e, "bForceInstantMeshUpdate", V::Bool(true));
                    vm.set(e, "__emblem", V::st(&[("Emblem", ap.field("Emblem").clone()), ("Color1", cols.first().cloned().unwrap_or(V::Int(0))), ("Color2", cols.get(1).cloned().unwrap_or(V::Int(0)))]));
                    let q = vm.prop(e, "Quiver");
                    if !matches!(q, V::None) && matches!(vm.prop(c, "Quiver"), V::None) {
                        vm.set(c, "Quiver", q);
                    }
                    // AMordhauCharacter::PickUp(item, slot): the item is the character's, in that slot
                    vm.set(e, "__owner_char", V::Obj(c));
                    held[i] = V::Obj(e);
                }
            }
            vm.set(c, "Equipment", V::Array(held));
            r(V::None)
        }
        // UMordhauSingleton::MakeEmptyProfile(out Profile, Character, bRandomize) rva (UMordhauSingleton.cpp 3589-3797):
        // the singleton CDO's CharacterProfiles[0] shape with the default character's wearables (UNCONFIRMED: the body
        // walks the character CDO's default wearables; here the shipped "Unnamed" profile, which is that result)
        "MakeEmptyProfile" if is(vm, "MordhauSingleton") => {
            let p = make_empty_profile(vm, a.get(2).map(V::truthy).unwrap_or(false));
            outs(V::None, vec![(0, p)])
        }
        // UMordhauUtilityLibrary::MakeEmptyProfile(CharacterClass, bRandomizeVoice) (UMordhauUtilityLibrary.cpp
        // 16398-16457) -> UMordhauSingleton::MakeEmptyProfile with the class's default object
        "MakeEmptyProfile" => r(make_empty_profile(vm, a1.truthy())),
        // AMordhauPlayerController::PrepareAndSendCustomizationIfChanged (AMordhauPlayerController.cpp 22517-22629): with
        // bSendsDefaultCustomization false, SelectedDefaultProfile >= 0 sends ServerRequestSetDefaultProfile when it
        // changed; else a valid SelectedCharacterProfile sends the custom profile (Validate'd) when it differs from the
        // last sent; neither selected -> SelectedDefaultProfile 0. Ctor defaults -1 (AMordhauPlayerController ctor
        // decomp 190-195). The "server" here is the local game: armory::spawn_queue -> SpawnProfile message.
        "PrepareAndSendCustomizationIfChanged" if t.is_some() => {
            let pc = t.unwrap();
            if vm.prop(pc, "bSendsDefaultCustomization").truthy() {
                return r(V::None);
            }
            let geti = |vm: &Vm, k: &str| match vm.prop(pc, k) {
                V::None => -1,
                v => v.i(),
            };
            let mut def = geti(vm, "SelectedDefaultProfile");
            if def == -1 {
                let cus = geti(vm, "SelectedCharacterProfile");
                if cus != -1 {
                    let list = vm.world.get("singleton").map(|&s| vm.prop(s, "CharacterProfiles")).unwrap_or_default();
                    let Some(p) = (cus >= 0).then(|| list.arr().get(cus as usize).cloned()).flatten() else { return r(V::None) };
                    let (_, p) = armory::force_valid(vm, &p);
                    if !vm.prop(pc, "bHasEverSentCustomProfile").truthy() || !p.same(&vm.prop(pc, "LastSentCharacterProfile")) {
                        vm.set(pc, "LastSentDefaultProfile", V::Int(-1));
                        vm.set(pc, "bHasEverSentCustomProfile", V::Bool(true));
                        vm.set(pc, "LastSentCharacterProfile", p.clone());
                        let n = vm.world.get("singleton").map(|&s| vm.prop(s, "DefaultProfiles").arr().len()).unwrap_or(0);
                        armory::queue_spawn(vm, n + cus as usize, p);
                    }
                    return r(V::None);
                }
                vm.set(pc, "SelectedDefaultProfile", V::Int(0));
                def = 0;
            }
            if geti(vm, "LastSentDefaultProfile") != def {
                let p = vm.world.get("singleton").and_then(|&s| vm.prop(s, "DefaultProfiles").arr().get(def as usize).cloned()).unwrap_or_default();
                armory::queue_spawn(vm, def as usize, p);
                vm.set(pc, "LastSentDefaultProfile", V::Int(def));
                vm.set(pc, "bHasEverSentCustomProfile", V::Bool(false));
            }
            r(V::None)
        }
        // UMordhauUtilityLibrary::FilterArrayByFunction(Array, FuncDel, Class) (UMordhauUtilityLibrary.cpp): the items
        // that are a Class and for which FuncDel(Item) sets its bool result (ProcessEvent parms: object then bool)
        "FilterArrayByFunction" => {
            let V::Delegate(o, f) = a1.clone() else { return r(V::Array(vec![])) };
            let cn = match &a2 {
                V::Asset(c) => c.name.clone(),
                _ => String::new(),
            };
            let mut keep = vec![];
            for it in a0.arr() {
                let Some(id) = it.obj().filter(|&i| vm.alive(i)) else { continue };
                if !cn.is_empty() && !vm.o(id).class.isa(&cn) && !vm.o(id).class.isa(cn.trim_end_matches("_C")) {
                    continue;
                }
                if !vm.alive(o) {
                    continue;
                }
                // FFilterByFunctionDelegate = void(UObject*, bool&) (extract/native/types/FFilterByFunctionDelegate.h): the
                // one out parameter is the verdict, whatever the Blueprint named it ("Ret Val" in BP_SelectionMenu filters)
                let (ret, outs_) = vm.call_named(o, &f, vec![it.clone()]);
                let ok = match ret {
                    V::None => outs_.values().any(V::truthy),
                    v => v.truthy(),
                };
                if ok {
                    keep.push(it.clone());
                }
            }
            r(V::Array(keep))
        }
        // UListView selection (UE 4.26 ListView.cpp / ListViewBase.cpp / SListView): BP_SetSelectedItem / SetSelectedItem
        // -> SListView::SetSelection(Item) (single selection) -> Private_SignalSelectionChanged -> the rows whose state
        // changed get IUserObjectListEntry::BP_OnItemSelectionChanged(bIsSelected), then UListView
        // OnSelectionChangedInternal broadcasts BP_OnItemSelectionChanged(Item, bIsSelected = Item != null).
        // BP_SetItemSelection(Item, bSelected) selects / deselects one item; BP_ClearSelection clears.
        "BP_SetSelectedItem" | "SetSelectedItem" | "BP_SetItemSelection" | "SetItemSelection" | "BP_ClearSelection" | "ClearSelection"
            if t.is_some_and(|i| matches!(vm.o(i).class.native(), "ListView" | "TileView" | "TreeView")) =>
        {
            let l = t.unwrap();
            let old = vm.prop(l, "__selected");
            let new = match name {
                "BP_ClearSelection" | "ClearSelection" => V::None,
                "BP_SetItemSelection" | "SetItemSelection" if !a1.truthy() => {
                    if old.same(&a0) {
                        V::None
                    } else {
                        return r(V::None);
                    }
                }
                _ => a0.clone(),
            };
            list_select(vm, l, old, new);
            r(V::None)
        }
        // UListView::BP_SetListItems(InListItems) (UMG ListView.cpp): SetListItems = ClearListItems then the items
        // (through natives.rs' AddItem, which makes the entry widgets)
        "BP_SetListItems" if t.is_some_and(|i| matches!(vm.o(i).class.native(), "ListView" | "TileView" | "TreeView")) => {
            let ctx2 = Ctx::Obj(t.unwrap());
            crate::natives::call(vm, ctx2, "ListView", "ClearListItems", &[]);
            let sel = vm.prop(t.unwrap(), "__selected");
            for it in a0.arr().to_vec() {
                crate::natives::call(vm, ctx2, "ListView", "AddItem", &[it]);
                // UListViewBase::HandleGenerateRow -> OnEntryWidgetGenerated -> BP_OnEntryGenerated(Widget)
                let e = vm.prop(t.unwrap(), "__entries").arr().last().cloned().unwrap_or_default();
                if let V::Multi(m) = vm.prop(t.unwrap(), "BP_OnEntryGenerated") {
                    for (o, f) in m {
                        vm.call_named(o, &f, vec![e.clone()]);
                    }
                }
            }
            // ClearListItems keeps no selection (UListView::ClearListItems -> ClearSelection); re-select a surviving item
            vm.set(t.unwrap(), "__selected", V::None);
            if a0.arr().iter().any(|x| x.same(&sel)) {
                list_select(vm, t.unwrap(), V::None, sel);
            }
            r(V::None)
        }
        // UMordhauSingleton::GetEquipment(Id) (UMordhauSingleton.cpp 5282-5317): Equipment[Id] class (None out of
        // range); GetEquipmentNum (6642-6646): Equipment.Num()
        "GetEquipment" if is(vm, "MordhauSingleton") => r(armory::equipment_class(vm, a0.i()).map(|p| class_ref(&p)).unwrap_or(V::None)),
        "GetEquipmentNum" if is(vm, "MordhauSingleton") => r(V::Int(vm.prop(t.unwrap(), "Equipment").arr().len() as i64)),
        // UMordhauUtilityLibrary::GetWearableClasses (UMordhauUtilityLibrary.cpp 22472-22480) =
        // FCharacterGearCustomization::GetWearableArray(Gear, Slot): the class list the slot's Id indexes
        "GetWearableClasses" => {
            let l = armory::wearable_list(vm, &a0, a1.i().max(0) as usize).map(|x| x.0).unwrap_or_default();
            r(V::Array(l.iter().map(|c| armory::class_pkg(c).map(|p| class_ref(&p)).unwrap_or(V::None)).collect()))
        }
        // UMordhauUtilityLibrary::GetDefaultActor(FromClass) (UMordhauUtilityLibrary.cpp 21109-21129): the class default
        // object (one VM object per class package, its properties the CDO's)
        "GetDefaultActor" | "GetDefaultActorCopy" => {
            let pkg = match &a0 {
                V::Asset(c) => Some(c.package.clone()),
                v => armory::class_pkg(v),
            };
            r(pkg.and_then(|p| vm.library(&p)).map(V::Obj).unwrap_or(V::None))
        }
        // banned items come from the server's AMordhauGameState lists (DoesProfileContainBannedEquipment
        // UMordhauUtilityLibrary.cpp 15698-15763, GetBannedEquipmentArray 8581-8791): a local game bans nothing
        "DoesProfileContainBannedEquipment" | "DoesProfileContainBannedPerks" => r(V::Bool(false)),
        "GetBannedEquipmentArray" | "GetBannedPerksArray" | "GetBannedEquipmentNames" | "GetBannedPerkNames" => r(V::Array(vec![])),
        // UMordhauInventory (PlayFab) ownership, offline: everything is owned and available (UNCONFIRMED stand-in, the
        // same offline backend natives.rs reports for the menus); gold / XP 0
        "HasItem" | "HasSkin" | "IsSkinAvailable" | "IsItemPlatformAvailable" if is(vm, "MordhauInventory") => r(V::Bool(true)),
        "GetGold" | "GetXP" if is(vm, "MordhauInventory") => r(V::Int(0)),
        // UMordhauUtilityLibrary::SortArrayByFunction(Array, FuncDel) (UMordhauUtilityLibrary.cpp 19692-19721): a copy
        // sorted by Algo::IntroSort with FuncDel(A, B) as the less-than (UNCONFIRMED: a stable insertion sort stands in
        // for IntroSort, so equal elements keep their order where IntroSort may not)
        "SortArrayByFunction" => {
            let V::Delegate(o, f) = a1.clone() else { return r(a0) };
            let mut items = a0.arr().to_vec();
            if !vm.alive(o) {
                return r(V::Array(items));
            }
            let less = |vm: &mut Vm, x: &V, y: &V| -> bool {
                let (ret, outs_) = vm.call_named(o, &f, vec![x.clone(), y.clone()]);
                match ret {
                    V::None => outs_.values().any(|v| matches!(v, V::Bool(true))),
                    v => v.truthy(),
                }
            };
            // insertion sort (stable) with the Blueprint predicate
            for i in 1..items.len() {
                let mut j = i;
                while j > 0 {
                    let (a_, b_) = (items[j].clone(), items[j - 1].clone());
                    if less(vm, &a_, &b_) {
                        items.swap(j, j - 1);
                        j -= 1;
                    } else {
                        break;
                    }
                }
            }
            r(V::Array(items))
        }
        // UBlueprintSetLibrary::Set_Contains(TargetSet, ItemToFind) (UE 4.26 BlueprintSetLibrary.cpp): sets are held as
        // arrays here; membership by value
        "Set_Contains" => r(V::Bool(a0.arr().iter().any(|x| x.same(&a1)))),
        // UGameplayStatics::GetObjectClass(Object): the object's class (a class reference of its Blueprint package)
        "GetObjectClass" => {
            let Some(o) = a0.obj().filter(|&o| vm.alive(o)) else { return r(V::None) };
            let c = vm.o(o).class.clone();
            if c.package.is_empty() {
                return r(V::Asset(Arc::new(ObjRef { package: "/Script/Mordhau".into(), name: c.name.clone(), class: "Class".into(), outer: String::new() })));
            }
            r(class_ref(&c.package))
        }
        // UMordhauSingleton::SpawnEquipment(World, Customization, Emblem, EmblemColor1, EmblemColor2, bDeferred?, bForce?)
        // (UMordhauSingleton.cpp 5241-5277) -> SpawnEquipmentFromClass(Equipment[Customization.Id], ...) (4519-4657): an
        // actor of the item's class with AssignedCustomization set; its meshes are the renderer's (ArmoryPreview.equipment)
        "SpawnEquipment" | "SpawnEquipmentFromClass" if is(vm, "MordhauSingleton") => {
            let (pkg, cust) = if name == "SpawnEquipment" {
                (armory::equipment_class(vm, a1.field("Id").i()), a1.clone())
            } else {
                (match &a1 {
                    V::Asset(c) => Some(c.package.clone()),
                    v => armory::class_pkg(v),
                }, a.get(2).cloned().unwrap_or_default())
            };
            let Some(pkg) = pkg else { return r(V::None) };
            r(spawn_equipment(vm, &pkg, cust).map(V::Obj).unwrap_or(V::None))
        }
        // UKismetTextLibrary::EqualEqual_IgnoreCase_TextText(A, B) (UE 4.26 KismetTextLibrary.cpp): A.EqualToCaseIgnored(B)
        // (case-folded compare of the display strings; ASCII/Unicode lower-casing stands in for ICU's fold: UNCONFIRMED for
        // non-ASCII)
        "EqualEqual_IgnoreCase_TextText" => r(V::Bool(a0.s().to_lowercase() == a1.s().to_lowercase())),
        // UKismetSystemLibrary::IsValidSoftClassReference(SoftClassReference) (UE 4.26): !SoftClassReference.IsNull(),
        // i.e. a non-empty asset path
        "IsValidSoftClassReference" | "IsValidSoftObjectReference" => r(V::Bool(armory::class_pkg(&a0).is_some() || matches!(&a0, V::Obj(_)))),
        // UUserListEntryLibrary::GetOwningListView(UserListEntry) (UE 4.26 IUserListEntry::GetOwningListView): the list view
        // that generated this entry widget (natives.rs keeps each list's generated entries in "__entries")
        "GetOwningListView" => {
            let Some(e) = a0.obj() else { return r(V::None) };
            let owner = (1..vm.objs.len() as u32).find(|&l| vm.alive(l) && matches!(vm.o(l).class.native(), "ListView" | "TileView" | "TreeView") && vm.prop(l, "__entries").arr().iter().any(|x| x.obj() == Some(e)));
            r(owner.map(V::Obj).unwrap_or(V::None))
        }
        // APawn::GetMovementComponent (UE 4.26; ACharacter: its CharacterMovement, a UMordhauMovementComponent for
        // AMordhauCharacter, created in the ctor so the Blueprint CDO does not list it): made on first use for a
        // VM-spawned character (the Armory's BP_CustomizationCharacterDoll), None for other actors. BP_LoadoutPicker:
        // Update Loadout Breakdown@43-154 needs it before it updates the breakdown, the top bar's name and points.
        "GetMovementComponent" if actorish(vm) => {
            let id = actor_of(vm, t.unwrap());
            if !vm.o(id).class.isa("Character") && !vm.o(id).class.isa("MordhauCharacter") {
                return r(V::None);
            }
            if let Some(c) = vm.prop(id, "CharacterMovement").obj().filter(|&c| vm.alive(c)) {
                return r(V::Obj(c));
            }
            let cls = vm.native_class("MordhauMovementComponent");
            let c = vm.new_obj(cls, "CharMoveComp");
            vm.set(c, "__owner", V::Obj(id));
            vm.set(id, "CharacterMovement", V::Obj(c));
            r(V::Obj(c))
        }
        // UMordhauMovementComponent::GetSpeedFactor(PartialSprintToSprintWeight) rva 0x14bf9a0 (UMordhauMovementComponent
        // .cpp 2417-2451) for the owner's applied profile: armor_speed_factor (below) + the equipment terms, capped by
        // the equipment override, times MotionSpeedFactor (1: no motion on the doll; the emote factor at owner +0xcc /
        // +300 is not active on the doll: UNCONFIRMED), floored at 0
        "GetSpeedFactor" if is(vm, "MordhauMovementComponent") => {
            let owner = vm.prop(t.unwrap(), "__owner").obj().filter(|&o| vm.alive(o));
            let prof = owner.map(|o| vm.prop(o, "Profile")).unwrap_or_default();
            r(V::Float(armory_speed_factor(vm, &prof, a0.f() as f32) as f64))
        }
        // UKismetInputLibrary pointer-event accessors (UE 4.26 KismetInputLibrary.cpp) over the host's FPointerEvent
        // values (host.rs pointer: CursorDelta, PressedButtons, WheelDelta): GetCursorDelta, IsMouseButtonDown(Key) =
        // PressedButtons contains the key, GetWheelDelta
        "PointerEvent_GetCursorDelta" => r(match a0.field("CursorDelta") {
            V::None => V::st(&[("X", V::Float(0.0)), ("Y", V::Float(0.0))]),
            v => v.clone(),
        }),
        "PointerEvent_IsMouseButtonDown" => {
            let k = a1.field("KeyName").s();
            r(V::Bool(a0.field("PressedButtons").arr().iter().any(|b| b.s() == k)))
        }
        "PointerEvent_GetWheelDelta" => r(V::Float(a0.field("WheelDelta").f())),
        // AMordhauCameraManager::EnterCustomization(CustomizationTarget) rva 0x14f7880: SetViewTarget(the observer) and
        // bIsInCustomization = true (BP_ProfileCustomization @105); LeaveCustomization rva 0x14ff680: back to the queued
        // view target, bIsInCustomization = false (BP_MainMenu @187). The host views through the observer only while
        // it is set (armory::preview)
        "EnterCustomization" => {
            if let Some(&m) = vm.world.get("map") {
                vm.set(m, "__in_customization", V::Bool(true));
            }
            r(V::None)
        }
        "LeaveCustomization" => {
            if let Some(&m) = vm.world.get("map") {
                vm.set(m, "__in_customization", V::Bool(false));
            }
            r(V::None)
        }
        // UNetPushModelHelpers::MarkPropertyDirtyFromRepIndex (UE 4.26 NetPushModelHelpers.cpp): flags a replicated
        // property for the push-model replication graph; the local UI game replicates nothing
        "MarkPropertyDirtyFromRepIndex" | "MarkPropertyDirty" => r(V::None),
        // UVirtualCursorFunctionLibrary::IsCursorOverInteractableWidget rva 0xebad10 (GamepadUMGPlugin, disassembled): the
        // virtual-cursor input processor's weak hovered-widget pointer (+0x170/+0x178) pinned; false when it is null,
        // which it is while the gamepad virtual cursor is not in use (mouse and keyboard)
        "IsCursorOverInteractableWidget" => r(V::Bool(false)),
        // UMordhauUtilityLibrary::execSLessThan(A, B) rva 0x17081a0 (497 bytes, disassembled with scripts/ue_dis.py): an
        // inline TCHAR loop comparing UTF-16 code units (case-sensitive; FString operator< would be Stricmp), result =
        // first differing unit below the other's. Which operand is on the left: UNCONFIRMED (A < B taken, the name's
        // reading; the sort predicates in BP_SelectionMenu:SortByName@516 pass (other, self))
        "SLessThan" => {
            let (x, y): (Vec<u16>, Vec<u16>) = (a0.s().encode_utf16().collect(), a1.s().encode_utf16().collect());
            r(V::Bool(x < y))
        }
        // AMordhauEquipment::GetWasSeen (AMordhauEquipment.cpp 7571-7617) -> UMordhauInventoryItem::GetWasSeen
        // (UMordhauInventoryItem.cpp 122-188): seen unless the item class is in UMordhauSingleton::UnseenInventoryItems
        // (+0x7e0). That set is filled by inventory grants (the PlayFab backend); the offline game grants nothing, so it is
        // empty: everything counts as seen. MarkSeen (8448-8589) removes the class from that set: nothing to remove.
        "GetWasSeen" if actorish(vm) || is(vm, "MordhauInventoryItem") => r(V::Bool(true)),
        "MarkSeen" if actorish(vm) || is(vm, "MordhauInventoryItem") => r(V::None),
        // AMordhauEquipment::ComputeAccurateBounds (AMordhauEquipment.cpp 8597-8643): the SkeletalMeshComponent's Bounds
        // (computed with the physics asset set) as FBoxSphereBounds. The mesh lives in the renderer, which reports the
        // component-space bounds with set_equipment_bounds; without them the origin is the component's location, so
        // BP_MordhauCustomizationPlatform:SpawnEquipment@1589 adds no centring offset (UNCONFIRMED until the host reports)
        "ComputeAccurateBounds" if actorish(vm) => {
            let id = actor_of(vm, t.unwrap());
            let pkg = vm.o(id).class.package.clone();
            let w = world_xf(vm, id);
            let host = vm.world.get("map").map(|&m| vm.prop(m, &format!("__bounds:{pkg}"))).filter(|v| matches!(v, V::Struct(_)));
            let (o, e, rad) = match host {
                Some(b) => (w.apply(vec_of(b.field("Origin"))), vec_of(b.field("BoxExtent")), b.field("SphereRadius").f()),
                None => {
                    if !vm.prop(id, "__bounds_warned").truthy() {
                        vm.set(id, "__bounds_warned", V::Bool(true));
                        eprintln!("armory: ComputeAccurateBounds({pkg}): no mesh bounds from the host (set_equipment_bounds); using the component location");
                    }
                    (w.t, FVector::ZERO, 0.0)
                }
            };
            r(V::st(&[("Origin", vec_v(o)), ("BoxExtent", vec_v(e)), ("SphereRadius", V::Float(rad))]))
        }
        // AMordhauEquipment::UpdateEquipmentState_Implementation rva (AMordhauEquipment.cpp 4394-4702): re-applies the
        // equipment's mesh / attachment / visibility state from its flags. The meshes are the renderer's
        // (ArmoryPreview.equipment); here the state change is recorded for it
        "UpdateEquipmentState" if actorish(vm) => {
            let id = actor_of(vm, t.unwrap());
            let n = vm.prop(id, "__state_serial").i();
            vm.set(id, "__state_serial", V::Int(n + 1));
            r(V::None)
        }
        // AMordhauEquipment::AssignCustomization(Customization, Emblem, EmblemColor1, EmblemColor2)
        "AssignCustomization" if t.is_some() && actorish(vm) => {
            vm.set(t.unwrap(), "AssignedCustomization", a0);
            r(V::None)
        }
        // ---- UMordhauUtilityLibrary statics on profiles ----
        // GetWearableClass(CharacterGearCustomization, Slot) -> TSubclassOf<UMordhauWearable>
        "GetWearableClass" => {
            let (c, _) = armory::wearable_class(vm, &a0, a1.i().max(0) as usize);
            r(c.map(|p| class_ref(&p)).unwrap_or(V::None))
        }
        // GetPerks(Profile) -> TArray<UPerk*>: the set perks' default objects
        "GetPerks" => {
            let perks = a0.field("SkillsCustomization").field("Perks").i();
            let list = armory::perk_classes(vm, perks);
            let mut v = vec![];
            // low bit first (UNCONFIRMED order; GetPerksCost's walk is high to low, sum-invariant)
            for (_, c) in list.into_iter().rev() {
                if let Some(o) = vm.library(&c) {
                    v.push(V::Obj(o));
                }
            }
            r(V::Array(v))
        }
        "GetPerksCost" => {
            let p = a0.field("SkillsCustomization").field("Perks").i();
            r(V::Int(armory::perks_cost(vm, p)))
        }
        "ComputePointsLeft" => r(V::Int(armory::points_left(vm, &a0))),
        // ForceValidCharacterProfile(Profile, out ForceValidatedProfile, bValidateInventory) -> bool
        "ForceValidCharacterProfile" => {
            let (ok, p) = armory::force_valid(vm, &a0);
            outs(V::Bool(ok), vec![(1, p)])
        }
        // UCharacterProfileBPWrapper::ForceValidate (UCharacterProfileBPWrapper.cpp 261-550): Profile = the
        // force-validated copy (the wrapper's own extra pass: UNCONFIRMED beyond ForceValidCharacterProfile's rules)
        "ForceValidate" if is(vm, "CharacterProfileBPWrapper") => {
            let id = t.unwrap();
            let p = vm.prop(id, "Profile");
            let (_, v) = armory::force_valid(vm, &p);
            vm.set(id, "Profile", v);
            r(V::None)
        }
        // ---- UCharacterProfileBPWrapper setters (exec thunks disassembled with scripts/ue_dis.py; Profile at +0x28,
        // Equipment array at +0x50, Skills at +0xb8) ----
        // execSetEquipmentId rva 0x1689160: if Equipment[Slot].Id != Id: Old = copy, item = FEquipmentCustomization()
        // (0x1527320), item.Id = Id, TryCarryOverEquipmentData(Old, item) (0x14a7910)
        "SetEquipmentId" if is(vm, "CharacterProfileBPWrapper") => {
            let id = a1.i();
            edit_equipment(vm, t.unwrap(), a0.i().max(0) as usize, |vm, old| {
                if old.field("Id").i() == id {
                    return None;
                }
                let mut n = empty_equipment();
                *field_mut(&mut n, "Id") = V::Int(id);
                carry_over_equipment(vm, old, &mut n);
                Some(n)
            });
            r(V::None)
        }
        // execSetEquipmentSkin rva 0x16894c0: if Skin differs: Old = copy, item = FEquipmentCustomization(), item.Id =
        // Old.Id, item.Skin = Skin (+0x29), TryCarryOverEquipmentData(Old, item)
        "SetEquipmentSkin" if is(vm, "CharacterProfileBPWrapper") => {
            let sk = a1.i();
            edit_equipment(vm, t.unwrap(), a0.i().max(0) as usize, |vm, old| {
                if old.field("Skin").i() == sk {
                    return None;
                }
                let mut n = empty_equipment();
                *field_mut(&mut n, "Id") = V::Int(old.field("Id").i());
                *field_mut(&mut n, "Skin") = V::Int(sk);
                carry_over_equipment(vm, old, &mut n);
                Some(n)
            });
            r(V::None)
        }
        // execSetEquipmentPartId rva 0x16892e0: Equipment[Slot].Parts[Part] = Id (no range check);
        // execSetEquipmentColor rva 0x1688e20: Colors[Index] = Color (same 272-byte shape: UNCONFIRMED twin reading)
        "SetEquipmentPartId" | "SetEquipmentColor" if is(vm, "CharacterProfileBPWrapper") => {
            let (k, i, v) = (if name == "SetEquipmentPartId" { "Parts" } else { "Colors" }, a1.i().max(0) as usize, a2.i());
            edit_equipment(vm, t.unwrap(), a0.i().max(0) as usize, |_, old| {
                let mut n = old.clone();
                *elem_mut(field_mut(&mut n, k), i)? = V::Int(v);
                Some(n)
            });
            r(V::None)
        }
        // execSetEquipmentPattern rva 0x16893f0: Equipment[Slot].Pattern = v
        "SetEquipmentPattern" if is(vm, "CharacterProfileBPWrapper") => {
            let v = a1.i();
            edit_equipment(vm, t.unwrap(), a0.i().max(0) as usize, |_, old| {
                let mut n = old.clone();
                *field_mut(&mut n, "Pattern") = V::Int(v);
                Some(n)
            });
            r(V::None)
        }
        // execSetEquipmentCustomizationDirect rva 0x1688f30: Equipment[Slot] = the given item
        "SetEquipmentCustomizationDirect" if is(vm, "CharacterProfileBPWrapper") => {
            let v = a1.clone();
            edit_equipment(vm, t.unwrap(), a0.i().max(0) as usize, |_, _| Some(v));
            r(V::None)
        }
        // execRemoveAllEquipment rva 0x1688c00: every equipment item = FEquipmentCustomization()
        "RemoveAllEquipment" if is(vm, "CharacterProfileBPWrapper") => {
            edit_profile(vm, t.unwrap(), |p| {
                if let V::Array(a) = field_mut(field_mut(p, "GearCustomization"), "Equipment") {
                    for e in a.iter_mut() {
                        *e = empty_equipment();
                    }
                }
            });
            r(V::None)
        }
        // UCharacterProfileBPWrapper::SetWearableId rva 0x1687d00 (1043 bytes, disassembled): same Id -> nothing; else
        // Old = copy of the profile, Wearables[Slot] = FWearableCustomization() with Id; the dependent slots reset to
        // their defaults with carry-over (Head -> Coif; UpperChest -> LowerChest, Shoulders, Arms, Hands; Arms -> Hands;
        // Legs -> Feet via HandleSubSlotWearableIdChanged), then TryCarryOverWearableData(Slot, Old, New, false)
        "SetWearableId" if is(vm, "CharacterProfileBPWrapper") => {
            let (slot, id) = (a0.i().max(0) as usize, a1.i());
            let w = t.unwrap();
            let p = vm.prop(w, "Profile");
            let old = p.field("GearCustomization").clone();
            if old.field("Wearables").arr().get(slot).map(|x| x.field("Id").i()) == Some(id) {
                return r(V::None);
            }
            let mut g = old.clone();
            if let Some(e) = elem_mut(field_mut(&mut g, "Wearables"), slot) {
                *e = armory::empty_wearable(id);
            }
            let subs: &[usize] = match slot {
                armory::HEAD => &[armory::COIF],
                armory::UPPER_CHEST => &[armory::LOWER_CHEST, armory::SHOULDERS, armory::ARMS, armory::HANDS],
                armory::ARMS => &[armory::HANDS],
                armory::LEGS => &[armory::FEET],
                _ => &[],
            };
            for &s2 in subs {
                reset_sub_slot(vm, s2, &old, &mut g);
            }
            carry_over_wearable(vm, slot, &old, &mut g, false);
            edit_profile(vm, w, |p| *field_mut(p, "GearCustomization") = g);
            r(V::None)
        }
        // execSetWearableColor rva 0x168a160 / execSetWearablePattern rva 0x168a270: Wearables[Slot].Colors[Index] / Pattern
        "SetWearableColor" if is(vm, "CharacterProfileBPWrapper") => {
            let (s, i, v) = (a0.i().max(0) as usize, a1.i().max(0) as usize, a2.i());
            edit_profile(vm, t.unwrap(), |p| {
                if let Some(w) = elem_mut(field_mut(field_mut(p, "GearCustomization"), "Wearables"), s) {
                    if let Some(c) = elem_mut(field_mut(w, "Colors"), i) {
                        *c = V::Int(v);
                    }
                }
            });
            r(V::None)
        }
        "SetWearablePattern" if is(vm, "CharacterProfileBPWrapper") => {
            let (s, v) = (a0.i().max(0) as usize, a1.i());
            edit_profile(vm, t.unwrap(), |p| {
                if let Some(w) = elem_mut(field_mut(field_mut(p, "GearCustomization"), "Wearables"), s) {
                    *field_mut(w, "Pattern") = V::Int(v);
                }
            });
            r(V::None)
        }
        // execTogglePerk rva 0x168a600: SetPerk(Perk, !HasPerk(Perk)) (FSkillsCustomization::HasPerk 0x1548140 /
        // SetPerk 0x156c240: bit Perk & 31 of Perks)
        "TogglePerk" if is(vm, "CharacterProfileBPWrapper") => {
            let b = 1i64 << (a0.i() & 31);
            edit_profile(vm, t.unwrap(), |p| {
                let s = field_mut(field_mut(p, "SkillsCustomization"), "Perks");
                *s = V::Int((s.i() ^ b) & 0xffff_ffff);
            });
            r(V::None)
        }
        "HasPerk" if is(vm, "CharacterProfileBPWrapper") => {
            let p = vm.prop(t.unwrap(), "Profile").field("SkillsCustomization").field("Perks").i();
            r(V::Bool((p >> (a0.i() & 31)) & 1 != 0))
        }
        // execSetProfileName rva 0x1689d10 / execSetProfileCategory rva 0x1689bf0
        "SetProfileName" if is(vm, "CharacterProfileBPWrapper") => {
            let n = a0.s();
            edit_profile(vm, t.unwrap(), |p| *field_mut(p, "Name") = V::Text(n));
            r(V::None)
        }
        "SetProfileCategory" if is(vm, "CharacterProfileBPWrapper") => {
            let n = a0.s();
            edit_profile(vm, t.unwrap(), |p| *field_mut(p, "Category") = V::Str(n));
            r(V::None)
        }
        // appearance setters (execSetFat rva 0x1689850 ... 118-130 bytes each: the byte stored into the field;
        // execSetFace rva 0x1689750 calls SetFace 0x14a0890, whose face-dependent hair resets are UNCONFIRMED here)
        "SetFat" | "SetSkinny" | "SetStrong" | "SetVoice" | "SetVoicePitch" | "SetAge" | "SetSkinColor" | "SetFace" | "SetEyeColor" | "SetHairColor" | "SetHair" | "SetFacialHair" | "SetEyebrows" | "SetEmblem" | "SetMetalTint" | "SetMetalRoughnessScale" | "SetIsFemale"
            if is(vm, "CharacterProfileBPWrapper") =>
        {
            let k = if name == "SetIsFemale" { "bIsFemale".to_string() } else { name[3..].to_string() };
            let v = if name == "SetIsFemale" { V::Bool(a0.truthy()) } else { V::Int(a0.i()) };
            edit_profile(vm, t.unwrap(), |p| *field_mut(field_mut(p, "AppearanceCustomization"), &k) = v);
            r(V::None)
        }
        // execSetEmblemColor rva 0x1688d60: EmblemColors[Index] = Color
        "SetEmblemColor" if is(vm, "CharacterProfileBPWrapper") => {
            let (i, v) = (a0.i().max(0) as usize, a1.i());
            edit_profile(vm, t.unwrap(), |p| {
                if let Some(c) = elem_mut(field_mut(field_mut(p, "AppearanceCustomization"), "EmblemColors"), i) {
                    *c = V::Int(v);
                }
            });
            r(V::None)
        }
        // FCharacterProfile::Compare (FCharacterProfile.cpp 308-489) and the per-part compares: member-wise equality
        "AreProfilesEqual" | "CompareGearCustomization" | "CompareEquipmentCustomization" | "CompareAppearanceCustomization" | "CompareFaceCustomization" | "CompareSkillsCustomization" | "CompareWearableCustomization" => r(V::Bool(a0.same(&a1))),
        _ => {
            let _ = class;
            None
        }
    }
}

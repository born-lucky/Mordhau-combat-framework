//! Collision data of the melee trace in UE space (cm, Z up), read from the paks through mh-pak; the Rust form of
//! godot/components/ue/records/ue_physics.gd without its Godot conversion:
//!   - PhysicsAsset bodies: SkeletalBodySetup.AggGeom (BoxElems, SphylElems, SphereElems) per bone, the shapes a
//!     weapon trace hits on a character. The character mesh's PhysicsAssetOverride (BP_MordhauCharacter
//!     CharacterMesh0.PhysicsAssetOverride = UMA_Master_PhysicsAsset: 16 bodies, boxes only). Elements with
//!     CollisionEnabled NoCollision are skipped (UePhysics.body_boxes).
//!   - skeletal mesh / skeleton sockets (SkeletalMeshSocket: SocketName, BoneName, RelativeLocation, RelativeRotation):
//!     the weapon's TraceStart / TraceEnd (AMordhauWeapon::GetTrace_Implementation rva=0x1629520 reads the FNames at
//!     .rdata 0x144361d18 "TraceStart" and 0x144361d40 "TraceEnd").
//!   - BP_MordhauCharacter's BlockCollider (UBoxComponent BoxExtent = half extents) and CharacterMesh0 placement.
//! Geometry: segment vs oriented box (slab test), vs capsule and sphere, in the shape's local frame, f32 inputs.
//! UNCONFIRMED (engine, PhysX): FKBoxElem X/Y/Z are full side lengths (half = X/2); FKSphylElem Length is the
//! cylinder length between the cap centres along local Z; the trace channel 0xf's responses.

use mh_pak::Reader;
use mordhau_core::ue::{FQuat, FTransform, FVector};
use serde_json::Value;

pub const CHARACTER: &str = "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter";
pub const TRACE_START: &str = "TraceStart";
pub const TRACE_END: &str = "TraceEnd";
/// AMordhauWeapon::GetTrace_Implementation rva=0x1629520: bIsUsingAlternateMode -> these sockets instead
pub const SECOND_TRACE_START: &str = "SecondTraceStart";
pub const SECOND_TRACE_END: &str = "SecondTraceEnd";

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// oriented box: half extents in the element frame
    Box { half: FVector },
    /// capsule along the element's local Z: radius, half length of the cylinder part
    Capsule { radius: f32, half_len: f32 },
    Sphere { radius: f32 },
}

/// One collision element of a body: bone, element frame relative to the bone (Center + Rotation), shape
#[derive(Clone, Debug, PartialEq)]
pub struct BodyShape {
    pub bone: String,
    pub xf: FTransform,
    pub shape: Shape,
}

fn vec3(v: &Value) -> FVector {
    let g = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    FVector::new(g("X"), g("Y"), g("Z"))
}
fn rot(v: &Value) -> FQuat {
    let g = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    FQuat::from_rotator(g("Pitch"), g("Yaw"), g("Roll"))
}

/// CUE4Parse / UePkg.strip: "Pkg/Path/Name.0" -> "Pkg/Path/Name"
pub fn strip(p: &str) -> String {
    let base = p.rsplit('/').next().unwrap_or(p);
    match base.rfind('.') {
        Some(i) => p[..p.len() - base.len() + i].to_string(),
        None => p.to_string(),
    }
}

fn exports(rd: &Reader, path: &str) -> Result<Vec<Value>, String> {
    rd.read(path).ok_or_else(|| format!("pak: no package {path}"))
}

fn export_named<'a>(ex: &'a [Value], name: &str) -> Option<&'a Value> {
    ex.iter().find(|e| e["Name"].as_str() == Some(name))
}

/// The character mesh's physics asset (CharacterMesh0.PhysicsAssetOverride) and skeletal mesh
pub fn character_mesh(rd: &Reader) -> Result<(String, String, FTransform), String> {
    let ex = exports(rd, CHARACTER)?;
    let cm = export_named(&ex, "CharacterMesh0").ok_or("BP_MordhauCharacter: no CharacterMesh0")?;
    let p = &cm["Properties"];
    let pa = strip(p["PhysicsAssetOverride"]["ObjectPath"].as_str().unwrap_or(""));
    let mesh = strip(p["SkeletalMesh"]["ObjectPath"].as_str().unwrap_or(""));
    // a component template holds only deltas: absent RelativeLocation / RelativeRotation = zero (CharacterData.mesh)
    let xf = FTransform::new(rot(&p["RelativeRotation"]), vec3(&p["RelativeLocation"]));
    Ok((pa, mesh, xf))
}

/// Every body shape of a physics asset (UePhysics.body_boxes, all element kinds), decoded by mh-assets::physics
/// (owner rust-assets; SkeletalBodySetups in body-index order); NoCollision elements skipped
pub fn body_shapes(rd: &Reader, pa: &str) -> Result<Vec<BodyShape>, String> {
    let asset = mh_assets::physics::read(rd, pa).ok_or_else(|| format!("pak: no physics asset {pa}"))?;
    let v = |c: [f64; 3]| FVector::new(c[0] as f32, c[1] as f32, c[2] as f32);
    let r = |d: [f64; 3]| FQuat::from_rotator(d[0] as f32, d[1] as f32, d[2] as f32);
    let mut out = Vec::new();
    for b in &asset.bodies {
        for e in b.geom.boxes.iter().filter(|e| e.shape.collides()) {
            let half = FVector::new((e.x * 0.5) as f32, (e.y * 0.5) as f32, (e.z * 0.5) as f32);
            out.push(BodyShape { bone: b.bone.clone(), xf: FTransform::new(r(e.rotation_deg), v(e.center)), shape: Shape::Box { half } });
        }
        for e in b.geom.sphyls.iter().filter(|e| e.shape.collides()) {
            let shape = Shape::Capsule { radius: e.radius as f32, half_len: (e.length * 0.5) as f32 };
            out.push(BodyShape { bone: b.bone.clone(), xf: FTransform::new(r(e.rotation_deg), v(e.center)), shape });
        }
        for e in b.geom.spheres.iter().filter(|e| e.shape.collides()) {
            out.push(BodyShape { bone: b.bone.clone(), xf: FTransform::new(FQuat::IDENTITY, v(e.center)), shape: Shape::Sphere { radius: e.radius as f32 } });
        }
    }
    Ok(out)
}

/// BlockCollider template: (BoxExtent half extents, RelativeLocation), UE cm
pub fn block_collider_ue(rd: &Reader) -> Result<([f32; 3], [f32; 3]), String> {
    let ex = exports(rd, CHARACTER)?;
    let b = export_named(&ex, "BlockColliderBP_GEN_VARIABLE").ok_or("BP_MordhauCharacter: no BlockCollider template")?;
    let (h, c) = (vec3(&b["Properties"]["BoxExtent"]), vec3(&b["Properties"]["RelativeLocation"]));
    Ok(([h.x, h.y, h.z], [c.x, c.y, c.z]))
}

/// BP_MordhauCharacter's BlockCollider offsets (AMordhauCharacter::UpdateBlockCollider rva=0x15700a0):
/// OriginalBlockColliderRelativeOffset = the BlockCollider template's relative transform (UNCONFIRMED: set at
/// BeginPlay from the component, not disassembled), Low / HighBlockColliderRelativeOffset from the CDO (absent: the
/// ctor's identity, AMordhauCharacter.cpp 3006-3011)
pub fn block_collider_offsets(rd: &Reader) -> Result<[mordhau_core::combat::geometry::ScaledXf; 3], String> {
    use mordhau_core::combat::geometry::ScaledXf;
    let ex = exports(rd, CHARACTER)?;
    let b = &export_named(&ex, "BlockColliderBP_GEN_VARIABLE").ok_or("BP_MordhauCharacter: no BlockCollider template")?["Properties"];
    let one = FVector::new(1.0, 1.0, 1.0);
    let s3 = |v: &Value| if v.is_null() { one } else { vec3(v) };
    let orig = ScaledXf {
        rot: if b["RelativeRotation"].is_null() { FQuat::IDENTITY } else { rot(&b["RelativeRotation"]) },
        loc: vec3(&b["RelativeLocation"]),
        scale: s3(&b["RelativeScale3D"]),
    };
    let d = rd.defaults(CHARACTER);
    let xf = |k: &str| -> ScaledXf {
        match d.get(k) {
            Some(v) => {
                let r = &v["Rotation"];
                let g = |k: &str| r[k].as_f64().unwrap_or(0.0) as f32;
                ScaledXf { rot: if r.is_null() { FQuat::IDENTITY } else { FQuat::new(g("X"), g("Y"), g("Z"), r["W"].as_f64().unwrap_or(1.0) as f32) }, loc: vec3(&v["Translation"]), scale: s3(&v["Scale3D"]) }
            }
            None => ScaledXf { rot: FQuat::IDENTITY, loc: FVector::ZERO, scale: one },
        }
    };
    Ok([orig, xf("LowBlockColliderRelativeOffset"), xf("HighBlockColliderRelativeOffset")])
}

/// A socket: bone + transform relative to it (UE cm)
#[derive(Clone, Debug, PartialEq)]
pub struct Socket {
    pub bone: String,
    pub xf: FTransform,
}

/// UePhysics.sockets: the skeleton's SkeletalMeshSockets, then the mesh's own on top (UNCONFIRMED override order)
pub fn sockets(rd: &Reader, mesh: &str) -> Result<Vec<(String, Socket)>, String> {
    let ex = exports(rd, mesh)?;
    let mut out: Vec<(String, Socket)> = Vec::new();
    let mut add = |ex: &[Value]| {
        for e in ex {
            if e["Type"].as_str() != Some("SkeletalMeshSocket") {
                continue;
            }
            let p = &e["Properties"];
            let name = p["SocketName"].as_str().unwrap_or("").to_string();
            let s = Socket { bone: p["BoneName"].as_str().unwrap_or("").to_string(), xf: FTransform::new(rot(&p["RelativeRotation"]), vec3(&p["RelativeLocation"])) };
            out.retain(|(n, _)| *n != name);
            out.push((name, s));
        }
    };
    let skel = ex.iter().find(|e| e["Type"].as_str() == Some("SkeletalMesh")).and_then(|e| e["Properties"]["Skeleton"]["ObjectPath"].as_str()).map(strip);
    if let Some(s) = skel.filter(|s| !s.is_empty()) {
        add(&exports(rd, &s)?);
    }
    add(&ex);
    Ok(out)
}

/// UePhysics.weapon_mesh: the weapon's SkeletalMesh, else the first skin's first part with a mesh
pub fn weapon_mesh(rd: &Reader, weapon: &str) -> Option<String> {
    let d = rd.defaults(weapon);
    if let Some(m) = d.get("SkeletalMesh").and_then(|v| v["ObjectPath"].as_str()) {
        return Some(strip(m));
    }
    for skin in d.get("Skins").and_then(|v| v.as_array()).into_iter().flatten() {
        for pt in skin["PartTypes"].as_array().into_iter().flatten() {
            for part in pt["Parts"].as_array().into_iter().flatten() {
                let cls = strip(part["ObjectPath"].as_str().unwrap_or(""));
                if cls.is_empty() {
                    continue;
                }
                let pd = rd.defaults(&cls);
                if let Some(m) = pd.get("SkeletalMesh").and_then(|v| v["ObjectPath"].as_str()) {
                    return Some(strip(m));
                }
            }
        }
    }
    None
}

/// TraceStart / TraceEnd socket locations (bone space = actor space for the weapon's root bone, UE cm)
pub fn weapon_sockets_ue(rd: &Reader, weapon: &str) -> Result<Option<([f32; 3], [f32; 3])>, String> {
    let Some(mesh) = weapon_mesh(rd, weapon) else { return Ok(None) };
    let so = sockets(rd, &mesh)?;
    let get = |n: &str| so.iter().find(|(k, _)| k == n).map(|(_, s)| s.xf.loc);
    Ok(match (get(TRACE_START), get(TRACE_END)) {
        (Some(a), Some(b)) => Some(([a.x, a.y, a.z], [b.x, b.y, b.z])),
        _ => None,
    })
}

/// The weapon mesh's SecondTraceStart / SecondTraceEnd socket locations (alternate mode), each None when absent
pub fn weapon_second_sockets_ue(rd: &Reader, weapon: &str) -> Result<(Option<FVector>, Option<FVector>), String> {
    let Some(mesh) = weapon_mesh(rd, weapon) else { return Ok((None, None)) };
    let so = sockets(rd, &mesh)?;
    let get = |n: &str| so.iter().find(|(k, _)| k == n).map(|(_, s)| s.xf.loc);
    Ok((get(SECOND_TRACE_START), get(SECOND_TRACE_END)))
}

/// The weapon's clash data: AMordhauWeapon ClashNormal / SecondClashNormal from the class defaults (absent: the ctor's
/// -TrailRight = (0, -1, 0), decomp AMordhauWeapon.cpp 2040-2048) and the BP_MordhauWeapon "ClashCapsuleBP" capsule
/// template's CapsuleRadius (SCS_Node_1: CapsuleComponent under SkeletalMeshComponent). UNCONFIRMED: where
/// ClashCollider (+0x1a50) is pointed at ClashCapsuleBP (no native or bytecode write found); per-weapon capsule
/// overrides (InheritableComponentHandler) are not read.
pub fn weapon_clash_ue(rd: &Reader, weapon: &str) -> (FVector, FVector, f32) {
    let d = rd.defaults(weapon);
    let n = |k: &str| d.get(k).map(vec3).unwrap_or(FVector::new(0.0, -1.0, 0.0));
    let mut radius = 0.0;
    if let Ok(ex) = exports(rd, WEAPON_BASE) {
        for e in &ex {
            if e["Type"].as_str() == Some("CapsuleComponent") && e["Name"].as_str().map(|s| s.starts_with("Capsule")).unwrap_or(false) {
                radius = e["Properties"]["CapsuleRadius"].as_f64().unwrap_or(22.0) as f32; // UCapsuleComponent ctor 22
            }
        }
    }
    (n("ClashNormal"), n("SecondClashNormal"), radius)
}

/// A shield's BlockCollider (AMordhauWeapon +0x1a58): the "BlockColliderBP" BoxComponent of the class's SCS
/// (BP_MordhauShield SCS_Node_2, parent RootSceneComponent; profile WeaponOnly), its template properties merged
/// down the Blueprint chain (BP_KiteShield's InheritableComponentTemplate overrides RelativeLocation (0, 0, -13) /
/// RelativeScale3D (0.070759, 0.9, 2.2)). Returns (relative location, half extents = BoxExtent * scale). BoxExtent: the
/// template's, else UBoxComponent's ctor default (32, 32, 32) (UE 4.26 BoxComponent.cpp, engine source, UNCONFIRMED as
/// compiled). The template's rotation is not set in any shield (identity). None for a class without the component.
pub fn weapon_block_box(rd: &Reader, weapon: &str) -> Option<(FVector, FVector)> {
    let mut props: Option<serde_json::Map<String, Value>> = None;
    for p in rd.chain(weapon).iter().rev() {
        let Ok(ex) = exports(rd, p) else { continue };
        for e in &ex {
            if e["Type"].as_str() == Some("BoxComponent") && e["Name"].as_str() == Some("BlockColliderBP_GEN_VARIABLE") {
                if let Some(o) = e["Properties"].as_object() {
                    let m = props.get_or_insert_with(serde_json::Map::new);
                    mh_pak::pkg::merge_into(m, o.clone());
                }
            }
        }
    }
    let m = props?;
    let loc = m.get("RelativeLocation").map(vec3).unwrap_or(FVector::ZERO);
    let scale = m.get("RelativeScale3D").map(vec3).unwrap_or(FVector::new(1.0, 1.0, 1.0));
    let ext = m.get("BoxExtent").map(vec3).unwrap_or(FVector::new(32.0, 32.0, 32.0));
    Some((loc, FVector::new(ext.x * scale.x, ext.y * scale.y, ext.z * scale.z)))
}

/// AMordhauWeapon ParryBoxTransform (+0x19c0) from the class defaults chain; None when absent (identity)
pub fn weapon_parry_box(rd: &Reader, weapon: &str) -> Option<mordhau_core::combat::geometry::ScaledXf> {
    let d = rd.defaults(weapon);
    let v = d.get("ParryBoxTransform")?;
    let r = &v["Rotation"];
    let g = |k: &str| r[k].as_f64().unwrap_or(0.0) as f32;
    let rot = if r.is_null() { FQuat::IDENTITY } else { FQuat::new(g("X"), g("Y"), g("Z"), r["W"].as_f64().unwrap_or(1.0) as f32) };
    let scale = if v["Scale3D"].is_null() { FVector::new(1.0, 1.0, 1.0) } else { vec3(&v["Scale3D"]) };
    Some(mordhau_core::combat::geometry::ScaledXf { rot, loc: vec3(&v["Translation"]), scale })
}

/// BP_MordhauWeapon: the base weapon Blueprint (holds the ClashCapsuleBP template)
pub const WEAPON_BASE: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/BP_MordhauWeapon";

// ---- segment vs shape --------------------------------------------------------------------------------------------

/// Entry fraction in [0, 1] of segment [a, b] into the shape placed at `world` (bone world * element frame), or None
pub fn segment_shape(a: FVector, b: FVector, world: &FTransform, shape: &Shape) -> Option<f32> {
    let p = world.inverse_apply(a);
    let q = world.inverse_apply(b);
    let d = q - p;
    match shape {
        Shape::Box { half } => slab(p, d, *half),
        Shape::Sphere { radius } => ray_sphere(p, d, FVector::ZERO, *radius),
        Shape::Capsule { radius, half_len } => {
            // the infinite cylinder along Z clipped to |z| <= half_len, else the cap spheres
            let mut best: Option<f32> = None;
            let take = |t: Option<f32>, best: &mut Option<f32>| {
                if let Some(t) = t {
                    if best.map(|b| t < b).unwrap_or(true) {
                        *best = Some(t);
                    }
                }
            };
            let a2 = d.x * d.x + d.y * d.y;
            let b2 = p.x * d.x + p.y * d.y;
            let c2 = p.x * p.x + p.y * p.y - radius * radius;
            if c2 <= 0.0 && p.z.abs() <= *half_len {
                return Some(0.0);
            }
            if a2 > 0.0 {
                let disc = b2 * b2 - a2 * c2;
                if disc >= 0.0 {
                    let t = (-b2 - disc.sqrt()) / a2;
                    if (0.0..=1.0).contains(&t) && (p.z + d.z * t).abs() <= *half_len {
                        take(Some(t), &mut best);
                    }
                }
            }
            take(ray_sphere(p, d, FVector::new(0.0, 0.0, *half_len), *radius), &mut best);
            take(ray_sphere(p, d, FVector::new(0.0, 0.0, -*half_len), *radius), &mut best);
            best
        }
    }
}

fn slab(p: FVector, d: FVector, half: FVector) -> Option<f32> {
    let (mut t0, mut t1) = (0f32, 1f32);
    for k in 0..3 {
        let (dk, pk, hk) = (d.get(k), p.get(k), half.get(k));
        if dk.abs() < 1e-12 {
            if pk.abs() > hk {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((-hk - pk) / dk, (hk - pk) / dk);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

fn ray_sphere(p: FVector, d: FVector, c: FVector, r: f32) -> Option<f32> {
    let m = p - c;
    let cc = m.dot(m) - r * r;
    if cc <= 0.0 {
        return Some(0.0);
    }
    let a = d.dot(d);
    if a == 0.0 {
        return None;
    }
    let b = m.dot(d);
    let disc = b * b - a * cc;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    (0.0..=1.0).contains(&t).then_some(t)
}

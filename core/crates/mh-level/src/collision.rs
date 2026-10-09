//! An engine-neutral collision world for a map, and a CPU reference implementation of the queries the character
//! movement makes (mh-character `World`: sweep_capsule / line_trace / overlap_capsule) over it.
//!
//! What is collected (all UE space, cm):
//! - every placed static mesh's UBodySetup simple collision (`mh_pak::aggeom`: convex hulls, boxes, sphyls, spheres;
//!   tapered capsules as sphyls of the larger radius), or its LOD0 triangles when the BodySetup is
//!   CTF_UseComplexAsSimple (through a caller-supplied triangle provider: mh-level does not decode meshes);
//! - landscape collision heightfields (landscape::LandscapeCollision::triangles, holes left out);
//! - brush volumes' convex elements (BlockingVolume, CameraBlockingVolume, ... with their collision settings);
//! - each body's collision settings: profile, CollisionEnabled, object type, per-channel responses, CanCharacterStepUpOn,
//!   physical material; resolved from the collision profiles of BaseEngine.ini + DefaultEngine.ini
//!   ([/Script/Engine.CollisionProfile] +Profiles / -Profiles / +EditProfiles / +DefaultChannelResponses) and the
//!   component's BodyInstance (Template chain merged per field);
//! - the WorldSettings KillZ and the gravity, and the damage volumes (PainCausingVolume, KillZVolume).
//!
//! Defaults with their sources:
//! - a StaticMeshComponent without a BodyInstance profile uses BlockAllDynamic: UStaticMeshComponent::
//!   UStaticMeshComponent (rva 0x2feb430) loads UCollisionProfile::BlockAllDynamic_ProfileName (rva 0x58fa370) at
//!   0x2feb4c0 and calls UPrimitiveComponent::SetCollisionProfileName at 0x2feb4ca;
//! - class defaults of the other colliding components, all read from their constructors (`class_default`); brush
//!   vtable slots +0x640 / +0x820 are SetCollisionProfileName / SetCollisionResponseToChannel (the UBrushComponent
//!   vftable symbol rva 0x4977fa0 is 0x1b8 below the address point: its slots 0x7f8 / 0x9d8 hold
//!   UPrimitiveComponent::SetCollisionProfileName rva 0x337ec30 / SetCollisionResponseToChannel rva 0x337ed50);
//! - KillZ -1048575 (AWorldSettings::AWorldSettings rva 0x3576250 stores 0xc97ffff0 at +0x244, 0x3576423), unless the
//!   level's WorldSettings serializes KillZ; gravity = WorldSettings GlobalGravityZ when set, else
//!   [/Script/Engine.PhysicsSettings] DefaultGravityZ (DefaultEngine.ini: -980; UWorld::GetDefaultGravityZ);
//! - a "Custom" BodyInstance (no profile name, or "Custom") = its CollisionEnabled / ObjectType / ResponseArray over
//!   the channel defaults; the ResponseArray lists only responses that differ from the channel default
//!   (FCollisionResponse). E.g. Arena's CameraBlockingVolumes serialize only Visibility and Projectile Ignore, so they
//!   block the Pawn as the data reads [UNCONFIRMED: the class-default responses are not in the cook];
//! - channels: the 8 engine channels respond Block by default (FCollisionResponseContainer), game channels per
//!   +DefaultChannelResponses; a response entry without Response= is ECR_Block (FResponseChannel default).
//!
//! The queries are a reference, not PhysX: every shape is a convex point set plus a radius and all queries are GJK
//! distances with conservative advancement (gjk.rs), exact for flat-faced shapes up to `TOL`; start-penetration depth
//! for intersecting cores is an approximation (gjk::penetration). PhysX is a closed engine dependency of the exe; the
//! character movement code consumes its results as given (mh-character world.rs header).

use crate::gjk::{self, add, dot, len, mul, norm, sub, V};
use crate::level::{pkg_of, LevelData, Pkgs};
use crate::xf::Xf;
use mh_pak::aggeom;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};

/// distance below which a sweep counts as touching (cm)
pub const TOL: f64 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resp {
    Ignore,
    Overlap,
    Block,
}

fn resp(s: &str) -> Resp {
    if s.ends_with("Ignore") {
        Resp::Ignore
    } else if s.ends_with("Overlap") {
        Resp::Overlap
    } else {
        Resp::Block
    }
}

/// The 8 engine collision channels (ECollisionChannel 0-7 display names)
pub const ENGINE_CHANNELS: [&str; 8] = ["WorldStatic", "WorldDynamic", "Pawn", "Visibility", "Camera", "PhysicsBody", "Vehicle", "Destructible"];

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub collision_enabled: String,
    pub object_type: String,
    pub responses: HashMap<String, Resp>,
}

/// Collision profiles and channels of the shipped config
#[derive(Clone, Debug, Default)]
pub struct Profiles {
    /// channel display name -> default response
    pub channel_defaults: Vec<(String, Resp)>,
    /// ECC_GameTraceChannelN -> display name
    pub game_channels: HashMap<String, String>,
    pub profiles: HashMap<String, Profile>,
    /// +ProfileRedirects OldName -> NewName (BaseEngine.ini: BlockingVolume -> InvisibleWall, ...)
    pub redirects: HashMap<String, String>,
}

/// `key="v"` or `key=v` inside a parenthesised ini value
fn field<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let i = s.find(&format!("{key}="))? + key.len() + 1;
    let r = &s[i..];
    if let Some(q) = r.strip_prefix('"') {
        return q.find('"').map(|e| &q[..e]);
    }
    Some(&r[..r.find([',', ')']).unwrap_or(r.len())])
}

/// (Channel="X",Response=ECR_Y) entries of a CustomResponses=(...) list
fn custom_responses(s: &str) -> Vec<(String, Resp)> {
    let Some(i) = s.find("CustomResponses=") else { return vec![] };
    let r = &s[i..];
    let mut out = vec![];
    for part in r.split("(Channel=").skip(1) {
        let ch = part.trim_start_matches('"');
        let ch = &ch[..ch.find(['"', ',', ')']).unwrap_or(ch.len())];
        let end = part.find(')').unwrap_or(part.len());
        let rs = field(&part[..end], "Response").map(resp).unwrap_or(Resp::Block);
        out.push((ch.to_string(), rs));
    }
    out
}

impl Profiles {
    pub fn load(vfs: &mh_pak::Vfs) -> Profiles {
        let mut p = Profiles::default();
        for c in ENGINE_CHANNELS {
            p.channel_defaults.push((c.to_string(), Resp::Block));
        }
        let mut edits = vec![];
        for file in ["BaseEngine.ini", "DefaultEngine.ini"] {
            let Some(t) = crate::config::text(vfs, file) else { continue };
            let mut in_sec = false;
            for raw in t.lines() {
                let l = raw.trim();
                if l.starts_with('[') {
                    in_sec = l == "[/Script/Engine.CollisionProfile]";
                    continue;
                }
                if !in_sec {
                    continue;
                }
                if let Some(v) = l.strip_prefix("+DefaultChannelResponses=") {
                    let (Some(ch), Some(name)) = (field(v, "Channel"), field(v, "Name")) else { continue };
                    let d = field(v, "DefaultResponse").map(resp).unwrap_or(Resp::Block);
                    p.game_channels.insert(ch.to_string(), name.to_string());
                    p.channel_defaults.retain(|(n, _)| n != name);
                    p.channel_defaults.push((name.to_string(), d));
                } else if let Some(v) = l.strip_prefix("-Profiles=") {
                    if let Some(n) = field(v, "Name") {
                        p.profiles.remove(n);
                    }
                } else if let Some(v) = l.strip_prefix("+Profiles=").or_else(|| l.strip_prefix("Profiles=")) {
                    let Some(n) = field(v, "Name") else { continue };
                    p.profiles.insert(
                        n.to_string(),
                        Profile {
                            name: n.to_string(),
                            collision_enabled: field(v, "CollisionEnabled").unwrap_or("QueryAndPhysics").to_string(),
                            object_type: field(v, "ObjectTypeName").unwrap_or("WorldStatic").to_string(),
                            responses: custom_responses(v).into_iter().collect(),
                        },
                    );
                } else if let Some(v) = l.strip_prefix("+ProfileRedirects=") {
                    if let (Some(o), Some(n)) = (field(v, "OldName"), field(v, "NewName")) {
                        p.redirects.insert(o.to_string(), n.to_string());
                    }
                } else if let Some(v) = l.strip_prefix("+EditProfiles=") {
                    if let Some(n) = field(v, "Name") {
                        edits.push((n.to_string(), custom_responses(v)));
                    }
                }
            }
        }
        for (n, rs) in edits {
            if let Some(pr) = p.profiles.get_mut(&n) {
                pr.responses.extend(rs);
            }
        }
        p
    }

    /// a profile name through +ProfileRedirects
    pub fn redirect(&self, name: &str) -> String {
        self.redirects.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    /// full response table of a profile (channel defaults, then its custom responses)
    pub fn responses(&self, profile: &str) -> Option<(String, String, HashMap<String, Resp>)> {
        let pr = self.profiles.get(profile)?;
        let mut r: HashMap<String, Resp> = self.channel_defaults.iter().cloned().collect();
        r.extend(pr.responses.iter().map(|(k, v)| (k.clone(), *v)));
        Some((pr.collision_enabled.clone(), pr.object_type.clone(), r))
    }

    /// "ECC_WorldStatic" / "ECC_GameTraceChannel3" / "WorldStatic" -> channel display name
    pub fn channel_name(&self, ecc: &str) -> String {
        let s = ecc.trim_start_matches("ECollisionChannel::");
        if let Some(n) = self.game_channels.get(s) {
            return n.clone();
        }
        s.trim_start_matches("ECC_").to_string()
    }
}

/// Collision settings of one body (component)
#[derive(Clone, Debug)]
pub struct Body {
    pub name: String,
    /// "mesh", "landscape", "volume"
    pub kind: &'static str,
    /// the component export "pkg.N" (meshes, volumes) or the landscape collision component name
    pub source: String,
    pub mesh: String,
    pub profile: String,
    /// QueryAndPhysics / QueryOnly / PhysicsOnly / NoCollision
    pub collision_enabled: String,
    pub object_type: String,
    pub responses: HashMap<String, Resp>,
    pub can_step_up: bool,
    pub physical_materials: Vec<String>,
    /// EPhysicalSurface per physical material slot (`surface_at`): a mesh body has one (its simple physical material),
    /// a landscape one per entry of `physical_materials` (the cooked heightfield's material indices)
    pub surfaces: Vec<u8>,
}

impl Body {
    pub fn queries(&self) -> bool {
        self.collision_enabled.contains("Query")
    }
    pub fn response(&self, channel: &str) -> Resp {
        self.responses.get(channel).copied().unwrap_or(Resp::Block)
    }
}

/// One convex shape: hull of `pts` (world, cm) rounded by `radius`
#[derive(Clone, Debug)]
pub struct Shape {
    pub body: u32,
    pub pts: Vec<V>,
    pub radius: f64,
    pub min: V,
    pub max: V,
}

impl Shape {
    fn new(body: u32, pts: Vec<V>, radius: f64) -> Shape {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for p in &pts {
            for k in 0..3 {
                min[k] = min[k].min(p[k] - radius);
                max[k] = max[k].max(p[k] + radius);
            }
        }
        Shape { body, pts, radius, min, max }
    }
}

/// A damage volume (PainCausingVolume / KillZVolume): convex elements + its properties
#[derive(Clone, Debug)]
pub struct DamageVolume {
    pub class: String,
    pub name: String,
    pub elems: Vec<Vec<V>>,
    pub props: Map<String, Value>,
}

#[derive(Default)]
struct Node {
    min: V,
    max: V,
    /// leaf: shape index range in `order`; inner: children
    left: u32,
    right: u32,
    start: u32,
    count: u32,
}

/// The collision world of a map
#[derive(Default)]
pub struct CollisionWorld {
    pub bodies: Vec<Body>,
    pub shapes: Vec<Shape>,
    pub damage_volumes: Vec<DamageVolume>,
    /// WorldSettings KillZ (cm)
    pub kill_z: f64,
    /// gravity Z (cm/s^2)
    pub gravity_z: f64,
    /// the querying object's profile (the character capsule: "Pawn") and channel
    pub query_profile: String,
    pub query_channel: String,
    query_responses: HashMap<String, Resp>,
    /// per body: blocks the query (channel response Block both ways, query enabled)
    pub blocks: Vec<bool>,
    /// meshes skipped: complex-as-simple without a triangle provider, or no BodySetup
    pub skipped: BTreeMap<String, usize>,
    nodes: Vec<Node>,
    /// landscape heightfields queried directly (cell walk): (body, data, world -> local, world box)
    pub heightfields: Vec<Heightfield>,
    order: Vec<u32>,
}

/// A landscape collision heightfield in the world, queried by its cells under the query box
pub struct Heightfield {
    pub body: u32,
    pub data: crate::landscape::LandscapeCollision,
    pub inv: Xf,
    pub min: V,
    pub max: V,
}

impl Heightfield {
    /// The cooked material index of the heightfield triangle under world `point` (XY), None over a hole / outside
    pub fn material_at(&self, point: V) -> Option<u8> {
        let e = 1.0;
        let (r0, r1, c0, c1) = self.data.cells_in(&self.inv, [point[0] - e, point[1] - e, point[2] - e], [point[0] + e, point[1] + e, point[2] + e])?;
        let mut tris = vec![];
        for r in r0..=r1 {
            for c in c0..=c1 {
                self.data.cell_triangles(r, c, &mut tris);
            }
        }
        let inside = |t: &[[f64; 3]; 3]| {
            let d = |a: [f64; 3], b: [f64; 3], p: V| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
            let (d0, d1, d2) = (d(t[0], t[1], point), d(t[1], t[2], point), d(t[2], t[0], point));
            let eps = 1e-6;
            (d0 >= -eps && d1 >= -eps && d2 >= -eps) || (d0 <= eps && d1 <= eps && d2 <= eps)
        };
        tris.iter().find(|(t, _)| inside(t)).map(|(_, m)| *m)
    }
}

/// Build options
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildOptions {
    /// put every landscape triangle in the BVH instead of querying heightfields by cell (the reference path the
    /// fast path is tested against)
    pub landscape_as_triangles: bool,
}

/// Triangle provider for CTF_UseComplexAsSimple meshes: mesh package -> LOD0 triangle soup (mesh space, cm, 3 points
/// per triangle). mh-assets static_mesh::lod0 positions + indices give it (tests/collision.rs).
pub type TriProvider<'a> = &'a dyn Fn(&str) -> Option<Vec<V>>;

/// Deep merge of a component's BodyInstance over its Template chain (a cooked component serializes only the struct
/// fields that differ from its archetype)
fn body_instance(pk: &Pkgs, e: &Value, depth: usize) -> Map<String, Value> {
    let mut base = match (e.get("Template"), depth < 16) {
        (Some(t), true) => pk.obj(Some(t)).map(|o| body_instance(pk, &o, depth + 1)).unwrap_or_default(),
        _ => Map::new(),
    };
    if let Some(Value::Object(own)) = e.get("Properties").and_then(|p| p.get("BodyInstance")) {
        mh_pak::pkg::merge_into(&mut base, own.clone());
    }
    base
}

/// EPhysicalSurface of a UPhysicalMaterial package: its SurfaceType ("SurfaceTypeN" -> N; absent = SurfaceType_Default
/// 0, the UPhysicalMaterial zero value; Engine DefaultPhysicalMaterial serializes none). Mordhau's names
/// (DefaultEngine.ini [/Script/Engine.PhysicsSettings] PhysicalSurfaces): 1 Flesh, 2 Wood, 3 Metal, 4 Stone, 5 Dirt,
/// 6 Grass, 7 Pebbles, 8 Wet, 9 Sand, 10 Snow
pub fn phys_material_surface(pk: &Pkgs, cache: &mut HashMap<String, u8>, pkg: &str) -> u8 {
    if pkg.is_empty() {
        return 0;
    }
    if let Some(v) = cache.get(pkg) {
        return *v;
    }
    let v = pk
        .load_pkg(pkg)
        .iter()
        .filter(|e| e.get("Type").and_then(|t| t.as_str()).is_some_and(|t| t.contains("PhysicalMaterial")))
        .find_map(|e| e.get("Properties").and_then(|p| p.get("SurfaceType")).and_then(|v| v.as_str()))
        .map(|t| t.rsplit("SurfaceType").next().and_then(|n| n.parse::<u8>().ok()).unwrap_or(0))
        .unwrap_or(0);
    cache.insert(pkg.to_string(), v);
    v
}

/// UMaterialInterface::GetPhysicalMaterial of a material object "pkg.N": UMaterialInstance rva 0x31eb1d0 = its
/// PhysMaterial (+0x88), else Parent (+0xd0)->GetPhysicalMaterial(); UMaterial rva 0x31d4480 = PhysMaterial, else
/// GEngine->DefaultPhysMaterial (+0x678; "" here, surface 0)
fn material_phys_material(pk: &Pkgs, path: &str, depth: usize) -> String {
    if path.is_empty() || depth > 16 {
        return String::new();
    }
    let Some(o) = pk.obj(Some(&serde_json::json!({ "ObjectPath": path }))) else { return String::new() };
    let p = pk.props(&o);
    if let Some(pm) = p.get("PhysMaterial").and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()) {
        return pkg_of(pm).to_string();
    }
    match p.get("Parent").and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()) {
        Some(parent) => material_phys_material(pk, parent, depth + 1),
        None => String::new(),
    }
}

/// FBodyInstance::GetSimplePhysicalMaterial rva 0x32ec370 for a static mesh body: the BodyInstance's
/// PhysMaterialOverride (+0x108), [the owner component's override (+0x3c0): read as the component's
/// PhysMaterialOverride property, UNCONFIRMED field name], the BodySetup's PhysMaterial (+0xa8), the component's
/// GetMaterial(0) (vtable +0x568: OverrideMaterials[0], else the mesh's StaticMaterials[0]) -> GetPhysicalMaterial
/// (vtable +0x298), else GEngine->DefaultPhysMaterial (+0x678). A complex-as-simple mesh's hits use the hit
/// triangle's section material (FBodyInstance::GetComplexPhysicalMaterials rva 0x32eabc0); the triangles carry no
/// section here, so those use the same slot-0 answer [UNCONFIRMED for multi-material complex meshes]
fn mesh_surface(pk: &Pkgs, cache: &mut HashMap<String, u8>, bi: &Map<String, Value>, comp: &Map<String, Value>, setup_pm: &str, m: &crate::level::MeshPlacement) -> u8 {
    let objpkg = |v: Option<&Value>| v.and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()).map(|s| pkg_of(s).to_string()).filter(|s| !s.is_empty());
    let pm = objpkg(bi.get("PhysMaterialOverride")).or_else(|| objpkg(comp.get("PhysMaterialOverride"))).or_else(|| (!setup_pm.is_empty()).then(|| setup_pm.to_string())).or_else(|| {
        let slot0 = m.material_paths.first().filter(|s| !s.is_empty()).cloned().or_else(|| {
            pk.load_pkg(m.mesh_pkg())
                .iter()
                .find(|e| e.get("Type").and_then(|t| t.as_str()) == Some("StaticMesh"))
                .and_then(|e| e.get("Properties").and_then(|p| p.get("StaticMaterials")).and_then(|a| a.as_array()).and_then(|a| a.first().cloned()))
                .and_then(|s0| s0.get("MaterialInterface").and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()).map(|s| s.to_string()))
        })?;
        Some(material_phys_material(pk, &slot0, 0)).filter(|s| !s.is_empty())
    });
    pm.map(|p| phys_material_surface(pk, cache, &p)).unwrap_or(0)
}

fn col(x: &Xf, c: usize) -> V {
    [x.m[0][c], x.m[1][c], x.m[2][c]]
}

impl CollisionWorld {
    /// Build the collision world of a read map (`level::read`). `tris` supplies complex-as-simple meshes.
    pub fn build(pk: &Pkgs, d: &LevelData, tris: Option<TriProvider>) -> CollisionWorld {
        Self::build_with(pk, d, tris, BuildOptions::default())
    }

    pub fn build_with(pk: &Pkgs, d: &LevelData, tris: Option<TriProvider>, opt: BuildOptions) -> CollisionWorld {
        let prof = Profiles::load(&pk.rd.vfs);
        let mut w = CollisionWorld { query_profile: "Pawn".into(), query_channel: "Pawn".into(), kill_z: -1048575.0, ..Default::default() };
        w.query_responses = prof.responses("Pawn").map(|r| r.2).unwrap_or_default();
        // gravity: [/Script/Engine.PhysicsSettings] DefaultGravityZ (UWorld::GetDefaultGravityZ)
        w.gravity_z = crate::config::float_value(&pk.rd.vfs, "DefaultEngine.ini", "/Script/Engine.PhysicsSettings", "DefaultGravityZ")
            .or_else(|| crate::config::float_value(&pk.rd.vfs, "BaseEngine.ini", "/Script/Engine.PhysicsSettings", "DefaultGravityZ"))
            .unwrap_or(-980.0);
        // WorldSettings of the persistent level
        if let Some(lv) = d.levels.first() {
            for e in pk.load_pkg(&lv.pkg).iter() {
                if e.get("Type").and_then(|t| t.as_str()).is_some_and(|t| t.contains("WorldSettings")) {
                    let p = pk.props(e);
                    if let Some(k) = p.get("KillZ").and_then(|v| v.as_f64()) {
                        w.kill_z = k;
                    }
                    if p.get("bGlobalGravitySet").and_then(|v| v.as_bool()) == Some(true) {
                        if let Some(g) = p.get("GlobalGravityZ").and_then(|v| v.as_f64()) {
                            w.gravity_z = g;
                        }
                    }
                }
            }
        }
        let settings = |bi: &Map<String, Value>, class: &str| -> (String, String, String, HashMap<String, Resp>) {
            let cd = class_default(class);
            // a serialized name wins; else the class default's (a Custom class default = its base profile + the
            // constructor's SetCollisionResponseToChannel calls, with the profile name invalidated to "Custom")
            let own = bi.get("CollisionProfileName").and_then(|v| v.as_str()).map(|n| prof.redirect(n));
            let name = own.clone().unwrap_or_else(|| if cd.extra.is_empty() { cd.profile.to_string() } else { "Custom".into() });
            if name != "Custom" {
                if let Some((en, ot, r)) = prof.responses(&name) {
                    return (name, en, ot, r);
                }
            }
            // Custom: the class default's settings, then the BodyInstance's own fields. A ResponseArray lists every
            // channel whose response differs from the channel default (FCollisionResponse::UpdateArrayFromResponseContainer),
            // so a serialized one replaces the whole table
            let (mut en, mut ot, mut r) = prof.responses(cd.profile).unwrap_or_else(|| ("QueryAndPhysics".into(), "WorldStatic".into(), prof.channel_defaults.iter().cloned().collect()));
            for (ch, rs) in cd.extra {
                r.insert(ch.to_string(), *rs);
            }
            if let Some(arr) = bi.get("CollisionResponses").and_then(|c| c.get("ResponseArray")).and_then(|a| a.as_array()) {
                r = prof.channel_defaults.iter().cloned().collect();
                for e in arr {
                    if let Some(ch) = e.get("Channel").and_then(|v| v.as_str()) {
                        r.insert(ch.to_string(), resp(e.get("Response").and_then(|v| v.as_str()).unwrap_or("ECR_Block")));
                    }
                }
            }
            if let Some(e) = bi.get("CollisionEnabled").and_then(|v| v.as_str()) {
                en = e.trim_start_matches("ECollisionEnabled::").to_string();
            }
            if let Some(o) = bi.get("ObjectType").and_then(|v| v.as_str()) {
                ot = prof.channel_name(o);
            }
            ("Custom".into(), en, ot, r)
        };
        let mut surf_cache: HashMap<String, u8> = HashMap::new();
        let mut body_cache: HashMap<String, Option<(aggeom::AggGeom, bool, String, Map<String, Value>)>> = HashMap::new();
        for m in d.meshes.iter() {
            let Some(comp) = pk.obj(Some(&serde_json::json!({ "ObjectPath": m.component_path }))) else { continue };
            let p = pk.props(&comp);
            let bi = body_instance(pk, &comp, 0);
            let mesh_pkg = m.mesh_pkg().to_string();
            let bs = body_cache
                .entry(m.mesh.clone())
                .or_insert_with(|| {
                    // UStaticMesh::BodySetup is serialized natively (UStaticMesh::Serialize), not as a tagged property:
                    // the BodySetup is the export of the mesh package whose Outer is the StaticMesh
                    let exps = pk.load_pkg(&mesh_pkg);
                    let bi_ = exps.iter().position(|e| {
                        e.get("Type").and_then(|t| t.as_str()) == Some("BodySetup")
                            && e.get("Outer").and_then(|o| o.get("ObjectPath")).and_then(|v| v.as_str()) == Some(m.mesh.as_str())
                    })?;
                    let bs = crate::level::Obj(exps.clone(), bi_);
                    let bp = pk.props(&bs);
                    let complex = bp.get("CollisionTraceFlag").and_then(|v| v.as_str()).is_some_and(|s| s.ends_with("UseComplexAsSimple"));
                    let pm = bp.get("PhysMaterial").and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()).map(|s| pkg_of(s).to_string()).unwrap_or_default();
                    let di = bp.get("DefaultInstance").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                    Some((aggeom::agg_geom(bp.get("AggGeom").unwrap_or(&Value::Null)), complex, pm, di))
                })
                .clone();
            let Some((agg, complex, pm, di)) = bs else {
                *w.skipped.entry("no BodySetup".into()).or_default() += 1;
                continue;
            };
            // UStaticMeshComponent::bUseDefaultCollision: the component's collision is its mesh's BodySetup
            // DefaultInstance (FBodyInstance::UseExternalCollisionProfile). Its default: false in the
            // UStaticMeshComponent ctor (and cl, 0xf7 on +0x485 at 0x2feb4d6), true for AStaticMeshActor's component
            // (or byte [comp+0x485], 8 at 0x3471e24; bit 3 of +0x485 = NewProp_bUseDefaultCollision_SetBit 0x36ed090);
            // a serialized value (component or Template chain) wins
            let owner = pk.obj(comp.get("Outer")).and_then(|o| o.get("Type").and_then(|t| t.as_str()).map(|t| t.to_string())).unwrap_or_default();
            let use_default = p.get("bUseDefaultCollision").and_then(|v| v.as_bool()).unwrap_or(owner == "StaticMeshActor");
            let (profile, en, ot, responses) = if use_default {
                // an FBodyInstance with no fields serialized = its ctor defaults: QueryAndPhysics, WorldStatic (UBodySetup
                // ctor 0x32fec75 SetObjectType), responses = channel defaults; every DefaultInstance read names a profile
                let mut d2 = di.clone();
                d2.entry("CollisionProfileName").or_insert(Value::from("Custom"));
                d2.entry("ObjectType").or_insert(Value::from("ECC_WorldStatic"));
                d2.entry("CollisionEnabled").or_insert(Value::from("ECollisionEnabled::QueryAndPhysics"));
                let (pf, en, ot, r) = settings(&d2, "StaticMeshComponent");
                (format!("{pf} (mesh default)"), en, ot, r)
            } else {
                let class = if owner == "StaticMeshActor" { "StaticMeshActor.StaticMeshComponent" } else { m.component.as_str() };
                settings(&bi, class)
            };
            if en.contains("NoCollision") {
                *w.skipped.entry("NoCollision".into()).or_default() += 1;
                continue;
            }
            let bid = w.bodies.len() as u32;
            let surface = mesh_surface(pk, &mut surf_cache, &bi, &p, &pm, &m);
            let mut pms: Vec<String> = p
                .get("PhysMaterialOverride")
                .and_then(|v| v.get("ObjectPath"))
                .and_then(|v| v.as_str())
                .map(|s| vec![pkg_of(s).to_string()])
                .unwrap_or_default();
            if pms.is_empty() && !pm.is_empty() {
                pms.push(pm);
            }
            w.bodies.push(Body {
                name: m.name.clone(),
                kind: "mesh",
                source: m.component_path.clone(),
                mesh: m.mesh.clone(),
                profile,
                collision_enabled: en,
                object_type: ot,
                responses,
                // UPrimitiveComponent::CanCharacterStepUpOn (default ECB_Yes)
                can_step_up: !p.get("CanCharacterStepUpOn").and_then(|v| v.as_str()).is_some_and(|s| s.ends_with("ECB_No")),
                surfaces: vec![surface],
                physical_materials: pms,
            });
            let xfs: Vec<Xf> = if m.instances.is_empty() { vec![m.xf] } else { m.instances.iter().map(|i| m.xf * *i).collect() };
            if complex {
                match tris.and_then(|f| f(&mesh_pkg)) {
                    Some(t) => {
                        for x in &xfs {
                            for tri in t.chunks_exact(3) {
                                w.shapes.push(Shape::new(bid, tri.iter().map(|p| x.apply(*p)).collect(), 0.0));
                            }
                        }
                    }
                    None => *w.skipped.entry("complex-as-simple without triangles".into()).or_default() += 1,
                }
                continue;
            }
            for x in &xfs {
                add_agg(&mut w.shapes, bid, x, &agg);
            }
        }
        // spline meshes: NoCollision by class default (USplineMeshComponent ctor 0x2feb2f7, bUseDefaultCollision cleared
        // at 0x2feb306); a colliding one (Blueprint templates wall_castlestn_spline / Overhead_Spline_flag set BlockAll)
        // gets its mesh's collision bent along the spline, as USplineMeshComponent::RecreateCollision does: convex hull
        // points deformed one by one; boxes as their 8 corners; spheres / sphyls deformed at their points with the radius
        // kept [UNCONFIRMED: the engine's handling of non-convex elements]; complex-as-simple triangles deformed
        for sp in &d.splines {
            let Some(comp) = pk.obj(Some(&serde_json::json!({ "ObjectPath": sp.component_path }))) else { continue };
            let bi = body_instance(pk, &comp, 0);
            let (profile, en, ot, responses) = settings(&bi, "SplineMeshComponent");
            if en.contains("NoCollision") {
                continue;
            }
            let mesh_pkg = pkg_of(&sp.mesh).to_string();
            let exps = pk.load_pkg(&mesh_pkg);
            let Some(smi) = sp.mesh.rsplit_once('.').and_then(|(_, i)| i.parse::<usize>().ok()) else { continue };
            let Some(bsi) = exps.iter().position(|e| {
                e.get("Type").and_then(|t| t.as_str()) == Some("BodySetup")
                    && e.get("Outer").and_then(|o| o.get("ObjectPath")).and_then(|v| v.as_str()) == Some(sp.mesh.as_str())
            }) else {
                continue;
            };
            let bp = pk.props(&exps[bsi]);
            let complex = bp.get("CollisionTraceFlag").and_then(|v| v.as_str()).is_some_and(|s| s.ends_with("UseComplexAsSimple"));
            let agg = aggeom::agg_geom(bp.get("AggGeom").unwrap_or(&Value::Null));
            // render bounds along the forward axis: the mesh's ExtendedBounds (Origin -+ BoxExtent)
            let eb = exps.get(smi).and_then(|m| m.get("Properties")).and_then(|p| p.get("ExtendedBounds")).cloned().unwrap_or(Value::Null);
            let ax = ["X", "Y", "Z"][sp.forward_axis as usize];
            let o = eb.get("Origin").and_then(|v| v.get(ax)).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let e = eb.get("BoxExtent").and_then(|v| v.get(ax)).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let bounds = (o - e, o + e);
            let bid = w.bodies.len() as u32;
            w.bodies.push(Body {
                name: sp.name.clone(),
                kind: "spline",
                source: sp.component_path.clone(),
                mesh: sp.mesh.clone(),
                profile,
                collision_enabled: en,
                object_type: ot,
                responses,
                can_step_up: true,
                physical_materials: vec![],
                surfaces: vec![],
            });
            let bend = |pts: Vec<V>| -> Vec<V> { pts.into_iter().map(|p| sp.xf.apply(sp.deform(p, bounds))).collect() };
            if complex {
                if let Some(t) = tris.and_then(|f| f(&mesh_pkg)) {
                    for tri in t.chunks_exact(3) {
                        w.shapes.push(Shape::new(bid, bend(tri.to_vec()), 0.0));
                    }
                }
                continue;
            }
            let mut tmp = vec![];
            add_agg(&mut tmp, bid, &Xf::trs([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]), &agg);
            for s0 in tmp {
                let r = s0.radius;
                w.shapes.push(Shape::new(bid, bend(s0.pts), r));
            }
        }
        for c in &d.landscape_collision {
            let bid = w.bodies.len() as u32;
            let (profile, en, ot, responses) = settings(&Map::new(), "LandscapeHeightfieldCollisionComponent");
            w.bodies.push(Body {
                name: c.name.clone(),
                kind: "landscape",
                source: c.name.clone(),
                mesh: String::new(),
                profile,
                collision_enabled: en,
                object_type: ot,
                responses,
                can_step_up: true,
                surfaces: c.physical_materials.iter().map(|pmp| phys_material_surface(pk, &mut surf_cache, pmp)).collect(),
                physical_materials: c.physical_materials.clone(),
            });
            if opt.landscape_as_triangles {
                for (t, _) in c.triangles() {
                    w.shapes.push(Shape::new(bid, t.to_vec(), 0.0));
                }
            } else if let Some(inv) = c.xf.inverse() {
                let mut min = [f64::INFINITY; 3];
                let mut max = [f64::NEG_INFINITY; 3];
                let n = c.verts;
                for y in 0..n {
                    for x in 0..n {
                        let p = c.vertex_world(x, y);
                        for k in 0..3 {
                            min[k] = min[k].min(p[k]);
                            max[k] = max[k].max(p[k]);
                        }
                    }
                }
                w.heightfields.push(Heightfield { body: bid, data: c.clone(), inv, min, max });
            }
        }
        for v in &d.volumes {
            if v.class == "PainCausingVolume" || v.class == "KillZVolume" {
                let props = pk.obj(Some(&serde_json::json!({ "ObjectPath": v.actor_path }))).map(|o| pk.props(&o)).unwrap_or_default();
                w.damage_volumes.push(DamageVolume { class: v.class.clone(), name: v.name.clone(), elems: v.elems.clone(), props });
                continue;
            }
            let Some(comp) = pk.obj(Some(&serde_json::json!({ "ObjectPath": v.component_path }))) else { continue };
            let bi = body_instance(pk, &comp, 0);
            let (profile, en, ot, responses) = settings(&bi, &v.class);
            if en.contains("NoCollision") {
                continue;
            }
            let bid = w.bodies.len() as u32;
            w.bodies.push(Body {
                name: format!("{}.{}", v.class, v.name),
                kind: "volume",
                source: v.component_path.clone(),
                mesh: String::new(),
                profile,
                collision_enabled: en,
                object_type: ot,
                responses,
                can_step_up: true,
                physical_materials: vec![],
                surfaces: vec![],
            });
            for el in &v.elems {
                if !el.is_empty() {
                    w.shapes.push(Shape::new(bid, el.clone(), 0.0));
                }
            }
        }
        w.blocks = w
            .bodies
            .iter()
            .map(|b| {
                b.queries()
                    && b.response(&w.query_channel) == Resp::Block
                    && w.query_responses.get(&b.object_type).copied().unwrap_or(Resp::Block) == Resp::Block
            })
            .collect();
        w.build_bvh();
        w
    }

    /// first-person r1 (combat test level, marked addition): a world with no map, only a flat floor: one
    /// "BlockAll" WorldStatic box body, top face at z = 0, `half` cm across each side (the test level is not a game
    /// map; its floor is rendering / testing glue). Gravity and the Pawn query profile as `build_with` reads them.
    pub fn flat_floor(vfs: &mh_pak::Vfs, half: f64) -> CollisionWorld {
        let prof = Profiles::load(vfs);
        let mut w = CollisionWorld { query_profile: "Pawn".into(), query_channel: "Pawn".into(), kill_z: -1048575.0, ..Default::default() };
        w.query_responses = prof.responses("Pawn").map(|r| r.2).unwrap_or_default();
        w.gravity_z = crate::config::float_value(vfs, "DefaultEngine.ini", "/Script/Engine.PhysicsSettings", "DefaultGravityZ")
            .or_else(|| crate::config::float_value(vfs, "BaseEngine.ini", "/Script/Engine.PhysicsSettings", "DefaultGravityZ"))
            .unwrap_or(-980.0);
        let (profile, en, ot, responses) = match prof.responses("BlockAll") {
            Some((en, ot, r)) => ("BlockAll".to_string(), en, ot, r),
            None => ("BlockAll".to_string(), "QueryAndPhysics".to_string(), "WorldStatic".to_string(), HashMap::new()),
        };
        w.bodies.push(Body {
            name: "TestLevelFloor".into(),
            kind: "mesh",
            source: "TestLevelFloor".into(),
            mesh: String::new(),
            profile,
            collision_enabled: en,
            object_type: ot,
            responses,
            can_step_up: true,
            physical_materials: vec![],
            surfaces: vec![],
        });
        let (h, d) = (half, 200.0);
        let mut pts = Vec::new();
        for x in [-h, h] {
            for y in [-h, h] {
                for z in [-d, 0.0] {
                    pts.push([x, y, z]);
                }
            }
        }
        w.shapes.push(Shape::new(0, pts, 0.0));
        w.blocks = w
            .bodies
            .iter()
            .map(|b| b.queries() && b.response(&w.query_channel) == Resp::Block && w.query_responses.get(&b.object_type).copied().unwrap_or(Resp::Block) == Resp::Block)
            .collect();
        w.build_bvh();
        w
    }

    fn build_bvh(&mut self) {
        self.order = (0..self.shapes.len() as u32).collect();
        self.nodes.clear();
        if self.shapes.is_empty() {
            return;
        }
        self.nodes.push(Node::default());
        let n = self.order.len();
        self.split(0, 0, n);
    }

    fn split(&mut self, node: usize, start: usize, end: usize) {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for &i in &self.order[start..end] {
            let s = &self.shapes[i as usize];
            for k in 0..3 {
                min[k] = min[k].min(s.min[k]);
                max[k] = max[k].max(s.max[k]);
            }
        }
        self.nodes[node].min = min;
        self.nodes[node].max = max;
        if end - start <= 4 {
            self.nodes[node].start = start as u32;
            self.nodes[node].count = (end - start) as u32;
            return;
        }
        let ext = sub(max, min);
        let ax = if ext[0] >= ext[1] && ext[0] >= ext[2] { 0 } else if ext[1] >= ext[2] { 1 } else { 2 };
        let shapes = &self.shapes;
        self.order[start..end].sort_by(|a, b| {
            let ca = shapes[*a as usize].min[ax] + shapes[*a as usize].max[ax];
            let cb = shapes[*b as usize].min[ax] + shapes[*b as usize].max[ax];
            ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mid = (start + end) / 2;
        let l = self.nodes.len();
        self.nodes.push(Node::default());
        self.nodes.push(Node::default());
        self.nodes[node].left = l as u32;
        self.nodes[node].right = l as u32 + 1;
        self.split(l, start, mid);
        self.split(l + 1, mid, end);
    }

    /// Accepted shapes overlapping the box: BVH shapes plus the landscape cells under it (as temporary triangles)
    pub fn shapes_in(&self, min: V, max: V, filter: &dyn Fn(u32) -> bool) -> Vec<std::borrow::Cow<'_, Shape>> {
        let mut out: Vec<std::borrow::Cow<'_, Shape>> = self
            .candidates(min, max)
            .into_iter()
            .map(|i| &self.shapes[i as usize])
            .filter(|s| filter(s.body))
            .map(std::borrow::Cow::Borrowed)
            .collect();
        let mut tris = vec![];
        for h in &self.heightfields {
            if !filter(h.body) || (0..3).any(|k| h.max[k] < min[k] || h.min[k] > max[k]) {
                continue;
            }
            let Some((r0, r1, c0, c1)) = h.data.cells_in(&h.inv, min, max) else { continue };
            for row in r0..=r1 {
                for col in c0..=c1 {
                    tris.clear();
                    h.data.cell_triangles(row, col, &mut tris);
                    for (t, _) in &tris {
                        let sh = Shape::new(h.body, t.to_vec(), 0.0);
                        if (0..3).all(|k| sh.max[k] >= min[k] && sh.min[k] <= max[k]) {
                            out.push(std::borrow::Cow::Owned(sh));
                        }
                    }
                }
            }
        }
        out
    }

    /// Shapes whose box overlaps [min, max]
    pub fn candidates(&self, min: V, max: V) -> Vec<u32> {
        let mut out = vec![];
        if self.nodes.is_empty() {
            return out;
        }
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let nd = &self.nodes[n];
            if (0..3).any(|k| nd.max[k] < min[k] || nd.min[k] > max[k]) {
                continue;
            }
            if nd.count > 0 {
                for &i in &self.order[nd.start as usize..(nd.start + nd.count) as usize] {
                    let s = &self.shapes[i as usize];
                    if (0..3).all(|k| s.max[k] >= min[k] && s.min[k] <= max[k]) {
                        out.push(i);
                    }
                }
            } else {
                stack.push(nd.left as usize);
                stack.push(nd.right as usize);
            }
        }
        out
    }

    /// Every hit of a vertical capsule (radius, half height incl. caps; the core segment is half_height - radius) swept
    /// from `start` to `end` against the shapes `filter` accepts, earliest first
    pub fn sweep(&self, start: V, end: V, radius: f64, half_height: f64, filter: &dyn Fn(u32) -> bool) -> Vec<Hit> {
        let h = (half_height - radius).max(0.0);
        let dvec = sub(end, start);
        let mut min = [0.0; 3];
        let mut max = [0.0; 3];
        for k in 0..3 {
            let ext = if k == 2 { h + radius } else { radius };
            min[k] = start[k].min(end[k]) - ext - TOL;
            max[k] = start[k].max(end[k]) + ext + TOL;
        }
        let core = [add(start, [0.0, 0.0, -h]), add(start, [0.0, 0.0, h])];
        let mut hits = vec![];
        for s in self.shapes_in(min, max, filter) {
            let s = &*s;
            if let Some(mut hit) = toi(&core, radius, dvec, s) {
                hit.body = s.body;
                hit.location = add(start, mul(dvec, hit.time));
                hits.push(hit);
            }
        }
        hits.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        hits
    }

    /// First hit of a ray
    pub fn trace(&self, start: V, end: V, filter: &dyn Fn(u32) -> bool) -> Option<Hit> {
        self.sweep(start, end, 0.0, 0.0, filter).into_iter().find(|h| !h.start_penetrating || h.time >= 0.0)
    }

    /// Whether a vertical capsule at `centre` overlaps an accepted shape
    pub fn overlap(&self, centre: V, radius: f64, half_height: f64, filter: &dyn Fn(u32) -> bool) -> bool {
        let h = (half_height - radius).max(0.0);
        let min = [centre[0] - radius, centre[1] - radius, centre[2] - h - radius];
        let max = [centre[0] + radius, centre[1] + radius, centre[2] + h + radius];
        let core = [add(centre, [0.0, 0.0, -h]), add(centre, [0.0, 0.0, h])];
        self.shapes_in(min, max, filter).iter().any(|s| gjk::distance(&core, &s.pts).0 < radius + s.radius - TOL)
    }

    /// Per body: the gameplay actor (index into `d.gameplay`) whose component it is, or None for level geometry.
    /// Matched by the body's placement name in the actor's `components` within the same level package (the body's
    /// `source` "pkg.N" and the actor's `path` share the package). Build once, index by body id.
    pub fn body_actors(&self, d: &LevelData) -> Vec<Option<usize>> {
        let mut by: HashMap<(&str, &str), usize> = HashMap::new();
        for (i, a) in d.gameplay.iter().enumerate() {
            let pkg = a.path.rsplit_once('.').map(|p| p.0).unwrap_or("");
            for c in &a.components {
                by.entry((pkg, c.as_str())).or_insert(i);
            }
        }
        self.bodies
            .iter()
            .map(|b| {
                let pkg = b.source.rsplit_once('.').map(|p| p.0).unwrap_or("");
                by.get(&(pkg, b.name.as_str())).copied()
            })
            .collect()
    }

    /// The gameplay actor a body belongs to (one lookup; use `body_actors` for many)
    pub fn actor_of(&self, body: u32, d: &LevelData) -> Option<usize> {
        let b = self.bodies.get(body as usize)?;
        let pkg = b.source.rsplit_once('.').map(|p| p.0).unwrap_or("");
        d.gameplay.iter().position(|a| a.path.rsplit_once('.').map(|p| p.0) == Some(pkg) && a.components.iter().any(|c| *c == b.name))
    }

    /// The default filter: bodies that block the Pawn capsule
    pub fn blocks_pawn(&self, body: u32) -> bool {
        self.blocks.get(body as usize).copied().unwrap_or(false)
    }
}

/// A raw sweep hit (UE space)
#[derive(Clone, Debug, Default)]
pub struct Hit {
    pub body: u32,
    pub time: f64,
    /// capsule centre at the hit
    pub location: V,
    pub impact_point: V,
    /// from the hit shape towards the capsule
    pub normal: V,
    pub start_penetrating: bool,
    pub penetration_depth: f64,
}

/// Time of impact of the core segment (rounded by r) moving by d against a shape (conservative advancement)
fn toi(core: &[V; 2], r: f64, d: V, s: &Shape) -> Option<Hit> {
    let rr = r + s.radius;
    let seg0: Vec<V> = if core[0] == core[1] { vec![core[0]] } else { core.to_vec() };
    let (dist, pa, pb) = gjk::distance(&seg0, &s.pts);
    if dist < rr - TOL {
        // start penetrating: depenetration direction and depth
        let (depth, n) = if dist > 1e-9 {
            (rr - dist, norm(sub(pa, pb)))
        } else {
            let (dep, n) = gjk::penetration(&seg0, &s.pts, &[[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
            (dep + rr, n)
        };
        let ip = if dist > 1e-9 { add(pb, mul(n, s.radius)) } else { pa };
        return Some(Hit { time: 0.0, impact_point: ip, normal: n, start_penetrating: true, penetration_depth: depth, ..Default::default() });
    }
    let dl = len(d);
    if dl == 0.0 {
        return None;
    }
    let mut t = 0.0f64;
    for _ in 0..96 {
        let seg: Vec<V> = seg0.iter().map(|p| add(*p, mul(d, t))).collect();
        let (dist, pa, pb) = gjk::distance(&seg, &s.pts);
        let gap = dist - rr;
        let n = norm(sub(pa, pb));
        if gap <= TOL {
            return Some(Hit { time: t, impact_point: add(pb, mul(n, s.radius)), normal: n, ..Default::default() });
        }
        let closing = -dot(d, n);
        if closing <= 1e-12 {
            return None;
        }
        // advance to TOL/2 short of contact, so the next closest-point pair is still separated and its direction
        // (the hit normal) well defined
        t += (gap - 0.5 * TOL) / closing;
        if t > 1.0 {
            return None;
        }
    }
    None
}

/// Add an AggGeom's elements, through the body -> world transform `x`, to `out`, in the order UE adds a body's shapes:
/// spheres, boxes, sphyls, convex, tapered capsules (FBodySetup::AddShapesToRigidActor_AssumesLocked) [UNCONFIRMED:
/// UE 4.26 source recalled]; the order matters to `encroaching` (FBodyInstance::OverlapTest stops at the first
/// overlapping shape)
fn add_agg(out: &mut Vec<Shape>, bid: u32, x: &Xf, a: &aggeom::AggGeom) {
    let scale_min = |m: &Xf| (0..3).map(|c| len(col(m, c))).fold(f64::INFINITY, f64::min);
    for s in &a.spheres {
        if !s.shape.queries() {
            continue;
        }
        // FKSphereElem scaled by the smallest absolute scale [UNCONFIRMED: UE 4.26 GetFinalScaled recalled]
        out.push(Shape::new(bid, vec![x.apply(s.center)], s.radius * scale_min(x)));
    }
    for b in &a.boxes {
        if !b.shape.queries() {
            continue;
        }
        let (t, q) = aggeom::elem_to_body(b.center, b.rotation_deg);
        let m = *x * Xf::trs(t, q, [1.0; 3]);
        let pts = (0..8)
            .map(|i| m.apply([if i & 1 == 0 { -b.x } else { b.x } * 0.5, if i & 2 == 0 { -b.y } else { b.y } * 0.5, if i & 4 == 0 { -b.z } else { b.z } * 0.5]))
            .collect();
        out.push(Shape::new(bid, pts, 0.0));
    }
    for s in &a.sphyls {
        if !s.shape.queries() {
            continue;
        }
        let (t, q) = aggeom::elem_to_body(s.center, s.rotation_deg);
        let m = *x * Xf::trs(t, q, [1.0; 3]);
        let rs = len(col(&m, 0)).min(len(col(&m, 1)));
        let pts = vec![m.apply([0.0, 0.0, -s.length * 0.5]), m.apply([0.0, 0.0, s.length * 0.5])];
        out.push(Shape::new(bid, pts, s.radius * rs));
    }
    for c in &a.convex {
        if !c.shape.queries() || c.verts.is_empty() {
            continue;
        }
        let e = Xf::trs(c.transform.translation, c.transform.rotation, c.transform.scale);
        let m = *x * e;
        out.push(Shape::new(bid, c.verts.iter().map(|p| m.apply(*p)).collect(), 0.0));
    }
    for s in &a.tapered {
        if !s.shape.queries() {
            continue;
        }
        // a tapered capsule bounded by the sphyl of its larger radius [approximation]
        let (t, q) = aggeom::elem_to_body(s.center, s.rotation_deg);
        let m = *x * Xf::trs(t, q, [1.0; 3]);
        let rs = len(col(&m, 0)).min(len(col(&m, 1)));
        let pts = vec![m.apply([0.0, 0.0, -s.length * 0.5]), m.apply([0.0, 0.0, s.length * 0.5])];
        out.push(Shape::new(bid, pts, s.radius0.max(s.radius1) * rs));
    }
}

/// The collision settings a component / volume class's constructor gives its primitive, read from the exe
pub struct ClassDefault {
    pub profile: &'static str,
    /// SetCollisionResponseToChannel calls after the profile (make the class default "Custom")
    pub extra: &'static [(&'static str, Resp)],
}

/// Constructor defaults by export Type (component class for meshes, actor class for volumes):
/// - StaticMeshComponent and subclasses (InstancedStaticMeshComponent rva 0x312bf90, HierarchicalInstancedStaticMesh
///   0x30f8370, FoliageInstancedStaticMesh 0x29760e0 set nothing of their own): BlockAllDynamic, UStaticMeshComponent
///   ctor 0x2feb4c0;
/// - AStaticMeshActor's StaticMeshComponent: BlockAll, AStaticMeshActor ctor (rva 0x3471d90) at 0x3471df0 (and
///   bUseDefaultCollision = true, see `build_with`);
/// - SplineMeshComponent: NoCollision, USplineMeshComponent ctor 0x2feb2f7 (after the inlined UStaticMeshComponent
///   ctor's BlockAllDynamic at 0x2feb1c0);
/// - LandscapeHeightfieldCollisionComponent: BlockAll, ctor 0x29830a4 (ALandscapeProxy's BodyInstance BlockAll too,
///   ctor 0x2982a61 FBodyInstance::SetCollisionProfileName);
/// - volumes (AVolume ctor rva 0x354eba0): the brush gets OverlapAll, a function-static FName(L"OverlapAll")
///   initialised at 0x354ec5d (string ??_C@_1BG@FJHDHCKK@ "OverlapAll") and set at 0x354ec10;
/// - BlockingVolume: InvisibleWall, ABlockingVolume ctor 0x2f05f6f passes the static FName at rva 0x58f9c90, whose
///   dynamic initializer at rva 0x73fab0 is FName(L"InvisibleWall") (??_C@_1BM@FKCAPGGD@);
/// - CameraBlockingVolume: AVolume's OverlapAll, then SetCollisionResponseToChannel(ECC_Camera = 4, ECR_Block = 2)
///   at 0x2f27152 (ACameraBlockingVolume ctor 0x2f27120);
/// - volume subclasses re-profile the brush after AVolume's ctor (brush vtable +0x640 SetCollisionProfileName):
///   NoCollision (UCollisionProfile::NoCollision_ProfileName, FName global rva 0x58fa358) in APostProcessVolume
///   0x3358a00, ALightmassImportanceVolume 0x31b3ff0, ALightmassCharacterIndirectDetailVolume 0x31b3f90,
///   ANavMeshBoundsVolume 0x38012b0, ANavModifierVolume 0x385f760, AAudioVolume 0x2f05e20, ACullDistanceVolume
///   0x3015810, ALevelStreamingVolume 0x31951e0, APrecomputedVisibilityVolume 0x3358b30,
///   APrecomputedVisibilityOverrideVolume 0x3358aa0, AVolumetricLightmapDensityVolume 0x354ec90;
///   APhysicsVolume 0x332e830: function-static FName(L"OverlapAllDynamic") (string at 0x46b2578), inherited by
///   ADefaultPhysicsVolume 0x3045d60, APainCausingVolume 0x324cf70 and AKillZVolume 0x3166740 (each calls it);
///   ATriggerVolume 0x34e5db0: function-static FName(L"Trigger") (string at 0x4a8db90).
pub fn class_default(class: &str) -> ClassDefault {
    const CAMERA_BLOCK: &[(&str, Resp)] = &[("Camera", Resp::Block)];
    match class {
        "SplineMeshComponent" => ClassDefault { profile: "NoCollision", extra: &[] },
        "StaticMeshActor.StaticMeshComponent" => ClassDefault { profile: "BlockAll", extra: &[] },
        "LandscapeHeightfieldCollisionComponent" => ClassDefault { profile: "BlockAll", extra: &[] },
        "BlockingVolume" => ClassDefault { profile: "InvisibleWall", extra: &[] },
        "CameraBlockingVolume" => ClassDefault { profile: "OverlapAll", extra: CAMERA_BLOCK },
        "PostProcessVolume" | "LightmassImportanceVolume" | "LightmassCharacterIndirectDetailVolume" | "NavMeshBoundsVolume" | "NavModifierVolume"
        | "AudioVolume" | "CullDistanceVolume" | "LevelStreamingVolume" | "PrecomputedVisibilityVolume" | "PrecomputedVisibilityOverrideVolume"
        | "VolumetricLightmapDensityVolume" => ClassDefault { profile: "NoCollision", extra: &[] },
        "PhysicsVolume" | "DefaultPhysicsVolume" | "PainCausingVolume" | "KillZVolume" => ClassDefault { profile: "OverlapAllDynamic", extra: &[] },
        "TriggerVolume" => ClassDefault { profile: "Trigger", extra: &[] },
        c if c.ends_with("Volume") => ClassDefault { profile: "OverlapAll", extra: &[] },
        _ => ClassDefault { profile: "BlockAllDynamic", extra: &[] },
    }
}

/// p.EncroachEpsilon default 0.15 (TAutoConsoleVariable registered at rva 0x75182e with __real@3e19999a)
pub const ENCROACH_EPSILON: f64 = 0.15;
/// KINDA_SMALL_NUMBER (__real@38d1b717 = 1e-4) as FindTeleportSpot compares the adjustment against it
pub const KINDA_SMALL: f64 = 1e-4;

impl CollisionWorld {
    /// The EPhysicalSurface a hit on `body` at world `point` reports (FHitResult PhysMaterial -> SurfaceType, what
    /// AAdvancedCharacter footsteps / world hits read): a mesh body's simple physical material; a landscape's cooked
    /// heightfield triangle under the point -> its material index -> that physical material (the PhysX hit's
    /// face material, PxHeightFieldSample materialIndex0 / 1)
    pub fn surface_at(&self, body: u32, point: V) -> u8 {
        let Some(b) = self.bodies.get(body as usize) else { return 0 };
        if b.kind == "landscape" {
            if let Some(h) = self.heightfields.iter().find(|h| h.body == body) {
                if let Some(i) = h.material_at(point) {
                    return b.surfaces.get(i as usize).copied().unwrap_or(0);
                }
            }
        }
        b.surfaces.first().copied().unwrap_or(0)
    }

    /// MTD (minimum translation, from the shape towards the capsule) of a vertical capsule against one shape, None when
    /// they do not overlap
    pub fn capsule_mtd(&self, centre: V, radius: f64, half_height: f64, s: &Shape) -> Option<V> {
        let h = (half_height - radius).max(0.0);
        let core: Vec<V> = if h > 0.0 { vec![add(centre, [0.0, 0.0, -h]), add(centre, [0.0, 0.0, h])] } else { vec![centre] };
        let rr = radius + s.radius;
        let (dist, pa, pb) = gjk::distance(&core, &s.pts);
        if dist >= rr {
            return None;
        }
        if dist > 1e-9 {
            return Some(mul(norm(sub(pa, pb)), rr - dist));
        }
        let (dep, n) = gjk::penetration(&core, &s.pts, &[[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        Some(mul(n, dep + rr))
    }

    /// UWorld::EncroachingBlockingGeometry (rva 0x31a1990) for the character: an actor with a movement component tests
    /// only its UpdatedPrimitive (the capsule; GetAllChildActors + ComponentEncroachesBlockingGeometry_WithAdjustment
    /// rva 0x319d320 at 0x31a2581). ComponentEncroachesBlockingGeometry_WithAdjustment: OverlapMultiByChannel (0x319d6d9)
    /// on the capsule's object channel with the capsule shrunk by p.EncroachEpsilon (GetCollisionShape(-Epsilon), vtable
    /// +0x740 with the negated epsilon at 0x319d3cc); for each blocking overlapping component (response to that channel
    /// Block) the body's OverlapTest MTD with the unshrunk capsule (0x319d7e0) is added to the adjustment (0x319d812);
    /// an MTD of distance 0 is retried with the shrunk capsule and the hit dropped when that misses (0x319d896); no
    /// remaining blocking hit = no encroachment and a zero adjustment. A body's MTD is its first overlapping shape's;
    /// for landscape / complex triangle bodies (one PhysX heightfield / trimesh shape) the deepest triangle's
    /// [UNCONFIRMED: PhysX computePenetration on a heightfield / trimesh].
    pub fn encroaching(&self, centre: V, radius: f64, half_height: f64, adjust: Option<&mut V>) -> bool {
        let filter = |b: u32| self.blocks_pawn(b);
        let (rs, hs) = ((radius - ENCROACH_EPSILON).max(0.0), (half_height - ENCROACH_EPSILON).max(0.0));
        let hh = (hs - rs).max(0.0);
        let min = [centre[0] - radius, centre[1] - radius, centre[2] - half_height];
        let max = [centre[0] + radius, centre[1] + radius, centre[2] + half_height];
        let shapes = self.shapes_in(min, max, &filter);
        let core = [add(centre, [0.0, 0.0, -hh]), add(centre, [0.0, 0.0, hh])];
        // overlapping bodies, in first-shape order
        let mut bodies: Vec<u32> = vec![];
        for sh in &shapes {
            if !bodies.contains(&sh.body) && gjk::distance(&core, &sh.pts).0 < rs + sh.radius {
                bodies.push(sh.body);
            }
        }
        if bodies.is_empty() {
            return false;
        }
        let Some(out) = adjust else { return true };
        let mut total = [0.0; 3];
        let mut blocking = 0;
        for b in bodies {
            let tri_body = matches!(self.bodies[b as usize].kind, "landscape") || shapes.iter().filter(|s| s.body == b).count() > 64;
            let mtd = |r: f64, h: f64| -> Option<V> {
                let mut best: Option<V> = None;
                for sh in shapes.iter().filter(|s| s.body == b) {
                    if let Some(m) = self.capsule_mtd(centre, r, h, sh) {
                        if !tri_body {
                            return Some(m);
                        }
                        if best.is_none_or(|bm| len(m) > len(bm)) {
                            best = Some(m);
                        }
                    }
                }
                best
            };
            blocking += 1;
            match mtd(radius, half_height) {
                Some(m) if len(m) > 0.0 => total = add(total, m),
                Some(_) => match mtd(rs, hs) {
                    Some(m) => total = add(total, m),
                    None => blocking -= 1,
                },
                None => {
                    // "OverlapTest says we are overlapping, yet MTD says we're not": zero adjustment, still encroaching
                    *out = [0.0; 3];
                    return true;
                }
            }
        }
        if blocking == 0 {
            *out = [0.0; 3];
            return false;
        }
        *out = total;
        true
    }

    /// UWorld::FindTeleportSpot (rva 0x31a3b60), read from the disassembly: fits as is -> true; else with the
    /// encroachment adjustment A (all components <= KINDA_SMALL -> false): try loc + (0, 0, A.z) (when A.z is not
    /// small); then from the original location a table of XY offsets (both A.x and A.y non-zero: (x, y), (-x, y),
    /// (x, -y), (-x, -y), (y, x), (-y, x), (y, -x), (-y, -x); one of them zero: (x, y), (-x, -y), (y, x), (-y, -x),
    /// (a, a), (a, -a), (-a, a), (-a, -a) with a the non-zero one, 0x31a3d65 / 0x31a3dd6), each tried at the original
    /// height (loop 0x31a3ea0) and then, when A.z is not small, with A.z added (loop 0x31a3f30); only the first entry
    /// when the actor's 2-bit field at +0x5c is 2 (cmove at 0x31a3e8f; the spawn template has not begun play, so a
    /// spawn tries all 8) [UNCONFIRMED: that field read as ActorHasBegunPlay]. Nothing fits -> the original location,
    /// false. `loc` is updated on success.
    pub fn find_teleport_spot(&self, loc: &mut V, radius: f64, half_height: f64) -> bool {
        let mut adj = [0.0; 3];
        if !self.encroaching(*loc, radius, half_height, Some(&mut adj)) {
            return true;
        }
        let small = |x: f64| x.abs() <= KINDA_SMALL;
        if small(adj[0]) && small(adj[1]) && small(adj[2]) {
            return false;
        }
        let orig = *loc;
        let zero_z = small(adj[2]);
        if !zero_z {
            let t = [orig[0], orig[1], orig[2] + adj[2]];
            if !self.encroaching(t, radius, half_height, None) {
                *loc = t;
                return true;
            }
        }
        let (zx, zy) = (small(adj[0]), small(adj[1]));
        if zx && zy {
            return false;
        }
        let (mut x, mut y) = (adj[0], adj[1]);
        if zx {
            x = 0.0;
        } else if zy {
            y = 0.0;
        }
        let table: Vec<[f64; 2]> = if !zx && !zy {
            vec![[x, y], [-x, y], [x, -y], [-x, -y], [y, x], [-y, x], [y, -x], [-y, -x]]
        } else {
            let a = if zy { x } else { y };
            vec![[x, y], [-x, -y], [y, x], [-y, -x], [a, a], [a, -a], [-a, a], [-a, -a]]
        };
        for dz in [0.0, adj[2]] {
            if dz != 0.0 && zero_z {
                break;
            }
            for e in &table {
                let t = [orig[0] + e[0], orig[1] + e[1], orig[2] + dz];
                if !self.encroaching(t, radius, half_height, None) {
                    *loc = t;
                    return true;
                }
            }
            if zero_z {
                break;
            }
        }
        false
    }

    /// Where UWorld::SpawnActor puts a pawn whose class uses ESpawnActorCollisionHandlingMethod::
    /// AdjustIfPossibleButAlwaysSpawn (BP_MordhauCharacter's class default; AGameModeBase::
    /// SpawnDefaultPawnAtTransform_Implementation rva 0x30ed030 leaves FActorSpawnParameters::SpawnCollisionHandlingOverride
    /// at the ctor's Undefined, FActorSpawnParameters::FActorSpawnParameters rva 0x354ee70 +0x28 = 0, so the class
    /// default applies): FindTeleportSpot, and the actor spawns either way (at the found spot, else at the original
    /// location). Returns (location, adjusted ok)
    pub fn spawn_adjust(&self, start: V, radius: f64, half_height: f64) -> (V, bool) {
        let mut l = start;
        let ok = self.find_teleport_spot(&mut l, radius, half_height);
        (l, ok)
    }
}

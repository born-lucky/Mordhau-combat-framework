//! FKAggregateGeom (a UBodySetup's simple collision: spheres, boxes, sphyls, convex hulls, tapered capsules) as
//! engine-neutral data, from the "AggGeom" tagged-property value of mh-pak's export JSON (the shape extract/json has).
//! Shared by mh_assets::physics (PhysicsAsset bodies; re-exported as mh_assets::collision) and mh-level (static-mesh
//! UBodySetup collision). Lives in mh-pak (owner rust-pak, module written by rust-assets) so both can depend on it.
//! UE space: centimetres, Z up, FRotator degrees (pitch, yaw, roll).
//!
//! Element layouts: UE 4.26 Runtime/Engine/Classes/PhysicsEngine/{SphereElem,BoxElem,SphylElem,ConvexElem,
//! TaperedCapsuleElem}.h as CUE4Parse reads them (tagged structs; the JSON fields are their UPROPERTY names).
//! Field defaults when a field is absent from the cooked tags (= equal to the struct's constructor value):
//!   FKSphereElem(): Center 0, Radius 1; FKBoxElem(): Center 0, Rotation 0, X = Y = Z = 1; FKSphylElem(): Center 0,
//!   Rotation 0, Radius 1, Length 1; FKTaperedCapsuleElem(): Radius0 = Radius1 = 1, Length 1; FKShapeElem:
//!   bContributeToMass true, CollisionEnabled QueryAndPhysics, RestOffset 0. UNCONFIRMED: recalled from the UE 4.26
//!   headers (engine constructors are not in the decomp); every Mordhau element read so far serializes these fields.
//! Semantics (UE 4.26 FKBoxElem / FKSphylElem::GetTransform): a box's X / Y / Z are full side lengths; a sphyl is a
//! capsule along its local Z with cylinder `Length` between the two hemisphere centres (total height Length + 2 r).

use serde_json::Value;

/// UE FRotator (degrees) -> quaternion (x, y, z, w): FRotator::Quaternion, as CUE4Parse FRotator.cs:88-111 and
/// godot/components/ue/records/ue_physics.gd rot_quat
pub fn rotator_quat(r: [f64; 3]) -> [f64; 4] {
    let h = std::f64::consts::PI / 360.0;
    let (sp, cp) = (r[0] * h).sin_cos();
    let (sy, cy) = (r[1] * h).sin_cos();
    let (sr, cr) = (r[2] * h).sin_cos();
    [cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub name: String,
    /// ECollisionEnabled::Type as stored ("ECollisionEnabled::QueryAndPhysics")
    pub collision_enabled: String,
    pub contribute_to_mass: bool,
    pub rest_offset: f64,
}

impl Shape {
    /// false for ECollisionEnabled::NoCollision (ue_physics.gd body_boxes skips those)
    pub fn collides(&self) -> bool {
        !self.collision_enabled.contains("NoCollision")
    }
    pub fn queries(&self) -> bool {
        self.collides() && !self.collision_enabled.ends_with("PhysicsOnly")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sphere {
    pub center: [f64; 3],
    pub radius: f64,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxElem {
    pub center: [f64; 3],
    /// FRotator (pitch, yaw, roll) degrees
    pub rotation_deg: [f64; 3],
    /// full side lengths
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sphyl {
    pub center: [f64; 3],
    pub rotation_deg: [f64; 3],
    pub radius: f64,
    /// cylinder length between the hemisphere centres, along local Z
    pub length: f64,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tapered {
    pub center: [f64; 3],
    pub rotation_deg: [f64; 3],
    pub radius0: f64,
    pub radius1: f64,
    pub length: f64,
    pub shape: Shape,
}

/// FTransform: rotation quaternion (x, y, z, w), translation (cm), scale
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xform {
    pub rotation: [f64; 4],
    pub translation: [f64; 3],
    pub scale: [f64; 3],
}

impl Default for Xform {
    fn default() -> Self {
        Xform { rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3], scale: [1.0; 3] }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Convex {
    /// hull points (VertexData, element space, cm); the cooked hull is the same point set
    pub verts: Vec<[f64; 3]>,
    /// FKConvexElem::Transform (element -> body)
    pub transform: Xform,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AggGeom {
    pub spheres: Vec<Sphere>,
    pub boxes: Vec<BoxElem>,
    pub sphyls: Vec<Sphyl>,
    pub convex: Vec<Convex>,
    pub tapered: Vec<Tapered>,
}

impl AggGeom {
    pub fn is_empty(&self) -> bool {
        self.spheres.is_empty() && self.boxes.is_empty() && self.sphyls.is_empty() && self.convex.is_empty()
            && self.tapered.is_empty()
    }
}

fn num(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn vec3(v: Option<&Value>, d: [f64; 3]) -> [f64; 3] {
    match v {
        Some(v) if v.is_object() => [num(v, "X", d[0]), num(v, "Y", d[1]), num(v, "Z", d[2])],
        _ => d,
    }
}
fn rot(v: Option<&Value>) -> [f64; 3] {
    match v {
        Some(v) if v.is_object() => [num(v, "Pitch", 0.0), num(v, "Yaw", 0.0), num(v, "Roll", 0.0)],
        _ => [0.0; 3],
    }
}
fn shape(v: &Value) -> Shape {
    Shape {
        name: v.get("Name").and_then(Value::as_str).unwrap_or("None").to_string(),
        collision_enabled: v.get("CollisionEnabled").and_then(Value::as_str).unwrap_or("ECollisionEnabled::QueryAndPhysics").to_string(),
        contribute_to_mass: v.get("bContributeToMass").and_then(Value::as_bool).unwrap_or(true),
        rest_offset: num(v, "RestOffset", 0.0),
    }
}
fn elems<'a>(g: &'a Value, k: &str) -> impl Iterator<Item = &'a Value> {
    g.get(k).and_then(Value::as_array).into_iter().flatten()
}

/// FTransform from its tagged JSON (Rotation quat, Translation, Scale3D)
pub fn xform(v: Option<&Value>) -> Xform {
    let Some(v) = v.filter(|v| v.is_object()) else { return Xform::default() };
    let q = v.get("Rotation");
    Xform {
        rotation: q.map_or([0.0, 0.0, 0.0, 1.0], |q| [num(q, "X", 0.0), num(q, "Y", 0.0), num(q, "Z", 0.0), num(q, "W", 1.0)]),
        translation: vec3(v.get("Translation"), [0.0; 3]),
        scale: vec3(v.get("Scale3D"), [1.0; 3]),
    }
}

/// The "AggGeom" value of a BodySetup's properties (`props["AggGeom"]`); an absent / null value gives an empty AggGeom
pub fn agg_geom(g: &Value) -> AggGeom {
    AggGeom {
        spheres: elems(g, "SphereElems")
            .map(|e| Sphere { center: vec3(e.get("Center"), [0.0; 3]), radius: num(e, "Radius", 1.0), shape: shape(e) })
            .collect(),
        boxes: elems(g, "BoxElems")
            .map(|e| BoxElem {
                center: vec3(e.get("Center"), [0.0; 3]),
                rotation_deg: rot(e.get("Rotation")),
                x: num(e, "X", 1.0),
                y: num(e, "Y", 1.0),
                z: num(e, "Z", 1.0),
                shape: shape(e),
            })
            .collect(),
        sphyls: elems(g, "SphylElems")
            .map(|e| Sphyl {
                center: vec3(e.get("Center"), [0.0; 3]),
                rotation_deg: rot(e.get("Rotation")),
                radius: num(e, "Radius", 1.0),
                length: num(e, "Length", 1.0),
                shape: shape(e),
            })
            .collect(),
        convex: elems(g, "ConvexElems")
            .map(|e| Convex {
                verts: elems(e, "VertexData").map(|p| vec3(Some(p), [0.0; 3])).collect(),
                transform: xform(e.get("Transform")),
                shape: shape(e),
            })
            .collect(),
        tapered: elems(g, "TaperedCapsuleElems")
            .map(|e| Tapered {
                center: vec3(e.get("Center"), [0.0; 3]),
                rotation_deg: rot(e.get("Rotation")),
                radius0: num(e, "Radius0", 1.0),
                radius1: num(e, "Radius1", 1.0),
                length: num(e, "Length", 1.0),
                shape: shape(e),
            })
            .collect(),
    }
}

/// Element -> body transform (translation cm, rotation quaternion x, y, z, w) of a centred, rotated element
/// (FKBoxElem / FKSphylElem / FKTaperedCapsuleElem::GetTransform = FTransform(Rotation, Center))
pub fn elem_to_body(center: [f64; 3], rotation_deg: [f64; 3]) -> ([f64; 3], [f64; 4]) {
    (center, rotator_quat(rotation_deg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_rotation() {
        let g: Value = serde_json::json!({"BoxElems": [{"Center": {"X": 1.0}}], "SphylElems": [{"Radius": 5.0}]});
        let a = agg_geom(&g);
        assert_eq!((a.boxes[0].x, a.boxes[0].center), (1.0, [1.0, 0.0, 0.0]));
        assert_eq!((a.sphyls[0].radius, a.sphyls[0].length), (5.0, 1.0));
        assert!(a.boxes[0].shape.collides());
        // yaw 90: rotation of 90 degrees about Z
        let q = rotator_quat([0.0, 90.0, 0.0]);
        assert!((q[2] - (0.5f64).sqrt()).abs() < 1e-12 && (q[3] - (0.5f64).sqrt()).abs() < 1e-12);
        assert!(agg_geom(&Value::Null).is_empty());
    }
}

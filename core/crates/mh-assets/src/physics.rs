//! UPhysicsAsset as engine-neutral data: per-bone bodies (USkeletalBodySetup: FKAggregateGeom elements with their
//! transforms, physics type, collision settings) and the ragdoll constraints (UPhysicsConstraintTemplate
//! DefaultInstance: FConstraintInstance frames + limits), read from the paks through mh-pak's export JSON (the shape
//! extract/json has). The character's asset is UMA_Master_PhysicsAsset (BP_MordhauCharacter CDO,
//! CharacterMesh0.PhysicsAssetOverride; ue_physics.gd), 16 bodies / 15 constraints. UE space (cm, Z up, FRotator
//! degrees); `godot_box` converts a box the way godot/components/ue/records/ue_physics.gd body_boxes does.
//!
//! Defaults for fields absent from the cooked tags (= the struct constructor's value), UE 4.26 headers
//! (PhysicsEngine/BodySetup.h, BodyInstance.h, ConstraintInstance.h, ConstraintTypes.h), UNCONFIRMED (engine
//! constructors are not in the decomp; the shipped exe's are not disassembled yet):
//!   USkeletalBodySetup: PhysicsType PhysType_Default, CollisionTraceFlag CTF_UseDefault, bConsiderForBounds true;
//!   FBodyInstance: CollisionEnabled QueryAndPhysics, CollisionProfileName "" (= the component's), ObjectType
//!     ECC_WorldStatic (unset);
//!   FConstraintInstance: Pos1 = Pos2 = 0, PriAxis1 = PriAxis2 = (1, 0, 0), SecAxis1 = SecAxis2 = (0, 1, 0);
//!   FLinearConstraint: Limit 0, X/Y/ZMotion LCM_Locked; FConeConstraint: Swing1/Swing2LimitDegrees 45,
//!   Swing1/Swing2Motion ACM_Free; FTwistConstraint: TwistLimitDegrees 45, TwistMotion ACM_Free;
//!   FConstraintProfileProperties: bDisableCollision false, bParentDominates false.

use crate::collision::{agg_geom, rotator_quat, AggGeom, BoxElem};
use crate::material::strip_index;
use mh_pak::Reader;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub bone: String,
    pub physics_material: Option<String>,
    pub geom: AggGeom,
    /// EPhysicsType as stored ("PhysType_Default" / "PhysType_Kinematic" / "PhysType_Simulated")
    pub physics_type: String,
    /// ECollisionTraceFlag ("CTF_UseSimpleAsComplex" ...)
    pub collision_trace_flag: String,
    pub consider_for_bounds: bool,
    /// DefaultInstance (FBodyInstance) collision: enabled state, profile name, object type, per-channel responses
    /// (channel, response) as stored
    pub collision_enabled: String,
    pub collision_profile: String,
    pub object_type: String,
    pub responses: Vec<(String, String)>,
    /// Stored rigid-body settings, needed by the corpse and knockdown assets (EVD_RAG_019).
    pub dynamics: BodyDynamics,
}

/// FBodyInstance's cooked overrides. None means the tag is absent, not zero: these engine constructor defaults
/// have not been verified against the shipped exe. Units remain UE cm / kg / seconds, as in DefaultInstance.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BodyDynamics {
    pub override_mass: Option<bool>,
    pub mass_kg: Option<f64>,
    pub mass_scale: Option<f64>,
    pub linear_damping: Option<f64>,
    pub angular_damping: Option<f64>,
    pub com_nudge_cm: Option<[f64; 3]>,
    pub sleep_family: Option<String>,
    pub custom_sleep_threshold_multiplier: Option<f64>,
    pub position_solver_iterations: Option<u64>,
    pub velocity_solver_iterations: Option<u64>,
}

/// Stored FConstraintDrive fields. An omitted enable flag must be resolved from the engine constructor before
/// enabling a drive: the corpse Head stores Stiffness=70 without bEnablePositionDrive; knockdown stores both.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Drive {
    pub stiffness: Option<f64>,
    pub damping: Option<f64>,
    pub force_limit: Option<f64>,
    pub position_enabled: Option<bool>,
    pub velocity_enabled: Option<bool>,
}

/// The soft-constraint settings in FLinearConstraint / FConeConstraint / FTwistConstraint (EVD_RAG_020).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LimitDynamics {
    pub soft: Option<bool>,
    pub stiffness: Option<f64>,
    pub damping: Option<f64>,
    pub restitution: Option<f64>,
    pub contact_distance: Option<f64>,
}

/// FConstraintProfileProperties dynamics, including PA_FallingRagdoll's Hips -> Position tether and pose drives.
/// Preserve absent tags explicitly; geometry / motion limits continue to live in Constraint::limits.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConstraintDynamics {
    pub projection_enabled: Option<bool>,
    pub projection_linear_cm: Option<f64>,
    pub projection_angular_deg: Option<f64>,
    pub linear_limit: LimitDynamics,
    pub cone_limit: LimitDynamics,
    pub twist_limit: LimitDynamics,
    pub linear_drives: [Drive; 3],
    pub twist_drive: Drive,
    pub swing_drive: Drive,
    pub slerp_drive: Drive,
    pub angular_drive_mode: Option<String>,
    pub orientation_target_deg: Option<[f64; 3]>,
    pub angular_velocity_target: Option<[f64; 3]>,
    pub linear_position_target_cm: Option<[f64; 3]>,
    pub linear_velocity_target: Option<[f64; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub linear_limit: f64,
    /// ELinearConstraintMotion per axis
    pub linear_motion: [String; 3],
    pub swing1_deg: f64,
    pub swing2_deg: f64,
    /// EAngularConstraintMotion
    pub swing1_motion: String,
    pub swing2_motion: String,
    pub twist_deg: f64,
    pub twist_motion: String,
    pub disable_collision: bool,
    pub parent_dominates: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    pub joint: String,
    /// child (Bone1) and parent (Bone2) bodies
    pub bone1: String,
    pub bone2: String,
    /// frames in each bone's space (cm): position, primary (twist) axis, secondary axis
    pub pos1: [f64; 3],
    pub pri_axis1: [f64; 3],
    pub sec_axis1: [f64; 3],
    pub pos2: [f64; 3],
    pub pri_axis2: [f64; 3],
    pub sec_axis2: [f64; 3],
    pub limits: Limits,
    pub dynamics: ConstraintDynamics,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhysicsAsset {
    pub package: String,
    /// in SkeletalBodySetups order (the body index UE uses)
    pub bodies: Vec<Body>,
    pub constraints: Vec<Constraint>,
    /// The native serialized map, in asset body-index order. Preserve values as well as keys.
    pub collision_disable_pairs: Vec<([usize; 2], bool)>,
}

impl PhysicsAsset {
    pub fn body(&self, bone: &str) -> Option<&Body> {
        self.bodies.iter().find(|b| b.bone.eq_ignore_ascii_case(bone))
    }
}

fn st<'a>(v: &'a Value, k: &str, d: &'a str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn f(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn b(v: &Value, k: &str, d: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn v3(v: &Value, k: &str, d: [f64; 3]) -> [f64; 3] {
    match v.get(k) {
        Some(x) if x.is_object() => [f(x, "X", d[0]), f(x, "Y", d[1]), f(x, "Z", d[2])],
        _ => d,
    }
}

fn stored_vector(v: &Value, k: &str, axes: [&str; 3]) -> Option<[f64; 3]> {
    let x = v.get(k).filter(|x| x.is_object())?;
    // A stored FVector / FRotator is a complete struct. Missing components of an object are zero; an absent
    // object remains None so callers can distinguish an engine default from a serialized zero vector.
    Some(axes.map(|axis| f(x, axis, 0.0)))
}

fn body_dynamics(v: &Value) -> BodyDynamics {
    BodyDynamics {
        override_mass: v.get("bOverrideMass").and_then(Value::as_bool),
        mass_kg: v.get("MassInKgOverride").and_then(Value::as_f64),
        mass_scale: v.get("MassScale").and_then(Value::as_f64),
        linear_damping: v.get("LinearDamping").and_then(Value::as_f64),
        angular_damping: v.get("AngularDamping").and_then(Value::as_f64),
        com_nudge_cm: stored_vector(v, "COMNudge", ["X", "Y", "Z"]),
        sleep_family: v.get("SleepFamily").and_then(Value::as_str).map(str::to_string),
        custom_sleep_threshold_multiplier: v.get("CustomSleepThresholdMultiplier").and_then(Value::as_f64),
        position_solver_iterations: v.get("PositionSolverIterationCount").and_then(Value::as_u64),
        velocity_solver_iterations: v.get("VelocitySolverIterationCount").and_then(Value::as_u64),
    }
}

fn drive(v: &Value) -> Drive {
    Drive {
        stiffness: v.get("Stiffness").and_then(Value::as_f64),
        damping: v.get("Damping").and_then(Value::as_f64),
        force_limit: v.get("MaxForce").and_then(Value::as_f64),
        position_enabled: v.get("bEnablePositionDrive").and_then(Value::as_bool),
        velocity_enabled: v.get("bEnableVelocityDrive").and_then(Value::as_bool),
    }
}

fn limit_dynamics(v: &Value) -> LimitDynamics {
    LimitDynamics {
        soft: v.get("bSoftConstraint").and_then(Value::as_bool),
        stiffness: v.get("Stiffness").and_then(Value::as_f64),
        damping: v.get("Damping").and_then(Value::as_f64),
        restitution: v.get("Restitution").and_then(Value::as_f64),
        contact_distance: v.get("ContactDistance").and_then(Value::as_f64),
    }
}

fn constraint_dynamics(v: &Value) -> ConstraintDynamics {
    let angular = &v["AngularDrive"];
    let linear = &v["LinearDrive"];
    ConstraintDynamics {
        projection_enabled: v.get("bEnableProjection").and_then(Value::as_bool),
        projection_linear_cm: v.get("ProjectionLinearTolerance").and_then(Value::as_f64),
        projection_angular_deg: v.get("ProjectionAngularTolerance").and_then(Value::as_f64),
        linear_limit: limit_dynamics(&v["LinearLimit"]),
        cone_limit: limit_dynamics(&v["ConeLimit"]),
        twist_limit: limit_dynamics(&v["TwistLimit"]),
        linear_drives: ["XDrive", "YDrive", "ZDrive"].map(|k| drive(&linear[k])),
        twist_drive: drive(&angular["TwistDrive"]),
        swing_drive: drive(&angular["SwingDrive"]),
        slerp_drive: drive(&angular["SlerpDrive"]),
        angular_drive_mode: angular.get("AngularDriveMode").and_then(Value::as_str).map(str::to_string),
        orientation_target_deg: stored_vector(angular, "OrientationTarget", ["Pitch", "Yaw", "Roll"]),
        angular_velocity_target: stored_vector(angular, "AngularVelocityTarget", ["X", "Y", "Z"]),
        linear_position_target_cm: stored_vector(linear, "PositionTarget", ["X", "Y", "Z"]),
        linear_velocity_target: stored_vector(linear, "VelocityTarget", ["X", "Y", "Z"]),
    }
}

/// export index of an object reference "pkg.N"
fn index(r: &Value) -> Option<usize> {
    let op = r.get("ObjectPath")?.as_str()?;
    op.rsplit_once('.')?.1.parse().ok()
}

/// The PhysicsAsset of a package
pub fn read(rd: &Reader, pkg: &str) -> Option<PhysicsAsset> {
    let pkg = strip_index(pkg);
    let ex = rd.read(pkg)?;
    let pa = ex.iter().find(|e| st(e, "Type", "") == "PhysicsAsset")?;
    let null = Value::Null;
    let pp = pa.get("Properties").unwrap_or(&null);
    let mut out = PhysicsAsset { package: pkg.to_string(), ..Default::default() };
    out.collision_disable_pairs = pa.get("CollisionDisableTable").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|p| Some(([p["Key"]["Indices"][0].as_u64()? as usize, p["Key"]["Indices"][1].as_u64()? as usize], p["Value"].as_bool()?))).collect();
    for r in pp.get("SkeletalBodySetups").and_then(Value::as_array).into_iter().flatten() {
        let Some(e) = index(r).and_then(|i| ex.get(i)) else { continue };
        let p = e.get("Properties").unwrap_or(&null);
        let di = p.get("DefaultInstance").unwrap_or(&null);
        let resp = di
            .get("CollisionResponses")
            .and_then(|c| c.get("ResponseArray"))
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|x| (st(x, "Channel", "").to_string(), st(x, "Response", "").to_string())).collect())
            .unwrap_or_default();
        out.bodies.push(Body {
            bone: st(p, "BoneName", "None").to_string(),
            physics_material: p.get("PhysMaterial").and_then(|m| m.get("ObjectPath")).and_then(Value::as_str).map(|s| strip_index(s).to_string()),
            geom: agg_geom(p.get("AggGeom").unwrap_or(&null)),
            physics_type: st(p, "PhysicsType", "PhysType_Default").trim_start_matches("EPhysicsType::").to_string(),
            collision_trace_flag: st(p, "CollisionTraceFlag", "CTF_UseDefault").trim_start_matches("ECollisionTraceFlag::").to_string(),
            consider_for_bounds: b(p, "bConsiderForBounds", true),
            collision_enabled: st(di, "CollisionEnabled", "ECollisionEnabled::QueryAndPhysics").to_string(),
            collision_profile: di.get("CollisionProfileName").and_then(|n| n.as_str().or_else(|| n.get("Name").and_then(Value::as_str))).unwrap_or("").to_string(),
            object_type: st(di, "ObjectType", "").to_string(),
            responses: resp,
            dynamics: body_dynamics(di),
        });
    }
    for r in pp.get("ConstraintSetup").and_then(Value::as_array).into_iter().flatten() {
        let Some(e) = index(r).and_then(|i| ex.get(i)) else { continue };
        let ci = e.get("Properties").and_then(|p| p.get("DefaultInstance")).unwrap_or(&null);
        let pi = ci.get("ProfileInstance").unwrap_or(&null);
        let lin = pi.get("LinearLimit").unwrap_or(&null);
        let cone = pi.get("ConeLimit").unwrap_or(&null);
        let tw = pi.get("TwistLimit").unwrap_or(&null);
        let m = |v: &Value, k: &str| st(v, k, "LCM_Locked").trim_start_matches("ELinearConstraintMotion::").to_string();
        let a = |v: &Value, k: &str| st(v, k, "ACM_Free").trim_start_matches("EAngularConstraintMotion::").to_string();
        out.constraints.push(Constraint {
            joint: st(ci, "JointName", "None").to_string(),
            bone1: st(ci, "ConstraintBone1", "None").to_string(),
            bone2: st(ci, "ConstraintBone2", "None").to_string(),
            pos1: v3(ci, "Pos1", [0.0; 3]),
            pri_axis1: v3(ci, "PriAxis1", [1.0, 0.0, 0.0]),
            sec_axis1: v3(ci, "SecAxis1", [0.0, 1.0, 0.0]),
            pos2: v3(ci, "Pos2", [0.0; 3]),
            pri_axis2: v3(ci, "PriAxis2", [1.0, 0.0, 0.0]),
            sec_axis2: v3(ci, "SecAxis2", [0.0, 1.0, 0.0]),
            limits: Limits {
                linear_limit: f(lin, "Limit", 0.0),
                linear_motion: [m(lin, "XMotion"), m(lin, "YMotion"), m(lin, "ZMotion")],
                swing1_deg: f(cone, "Swing1LimitDegrees", 45.0),
                swing2_deg: f(cone, "Swing2LimitDegrees", 45.0),
                swing1_motion: a(cone, "Swing1Motion"),
                swing2_motion: a(cone, "Swing2Motion"),
                twist_deg: f(tw, "TwistLimitDegrees", 45.0),
                twist_motion: a(tw, "TwistMotion"),
                disable_collision: b(pi, "bDisableCollision", false),
                parent_dominates: b(pi, "bParentDominates", false),
            },
            dynamics: constraint_dynamics(pi),
        });
    }
    Some(out)
}

/// A box in the Godot port's bone space (ue_physics.gd body_boxes): centre (m, Y up), rotation quaternion
/// SwapYZ(FRotator.Quaternion) = (x, z, y, -w) normalized, half extents (X, Z, Y) x 0.005 m
pub fn godot_box(bx: &BoxElem) -> ([f64; 3], [f64; 4], [f64; 3]) {
    let q = rotator_quat(bx.rotation_deg);
    let g = [q[0], q[2], q[1], -q[3]];
    let l = g.iter().map(|x| x * x).sum::<f64>().sqrt();
    let c = bx.center;
    ([c[0] * 0.01, c[2] * 0.01, c[1] * 0.01], g.map(|x| x / l), [bx.x * 0.005, bx.z * 0.005, bx.y * 0.005])
}

/// USkeletalMeshSocket (SocketName, BoneName, RelativeLocation cm, RelativeRotation deg, RelativeScale)
#[derive(Debug, Clone, PartialEq)]
pub struct Socket {
    pub bone: String,
    pub location: [f64; 3],
    pub rotation_deg: [f64; 3],
    pub scale: [f64; 3],
}

fn add_sockets(ex: &[Value], out: &mut std::collections::BTreeMap<String, Socket>) {
    let null = Value::Null;
    for e in ex.iter().filter(|e| st(e, "Type", "") == "SkeletalMeshSocket") {
        let p = e.get("Properties").unwrap_or(&null);
        let r = p.get("RelativeRotation").unwrap_or(&null);
        out.insert(st(p, "SocketName", "None").to_string(), Socket {
            bone: st(p, "BoneName", "None").to_string(),
            location: v3(p, "RelativeLocation", [0.0; 3]),
            rotation_deg: [f(r, "Pitch", 0.0), f(r, "Yaw", 0.0), f(r, "Roll", 0.0)],
            scale: v3(p, "RelativeScale", [1.0; 3]),
        });
    }
}

/// Sockets of a skeletal mesh: its Skeleton's, then the mesh's own on top (ue_physics.gd sockets; UNCONFIRMED there:
/// mesh sockets override skeleton sockets of the same name, USkeletalMesh::FindSocket). The melee trace reads
/// "TraceStart" / "TraceEnd" (Second* in alternate mode): AMordhauWeapon::GetTrace_Implementation rva=0x1629520,
/// .rdata 0x144361d18 / 0x144361d40 / 0x144361d28 / 0x144361d50.
pub fn sockets(rd: &Reader, mesh: &str) -> std::collections::BTreeMap<String, Socket> {
    let mut out = std::collections::BTreeMap::new();
    let Some(ex) = rd.read(strip_index(mesh)) else { return out };
    let skel = ex
        .iter()
        .find(|e| st(e, "Type", "") == "SkeletalMesh")
        .and_then(|e| e.get("Properties")?.get("Skeleton")?.get("ObjectPath")?.as_str())
        .map(|s| strip_index(s).to_string());
    if let Some(sk) = skel.and_then(|s| rd.read(&s)) {
        add_sockets(&sk, &mut out);
    }
    add_sockets(&ex, &mut out);
    out
}

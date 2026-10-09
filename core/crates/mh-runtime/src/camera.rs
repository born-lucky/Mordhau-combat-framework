//! camera.rs - CameraPlugin. R1: a fly camera (the Godot level walker's role, godot/game/world/level_walker.gd).
//! `--cam=x,y,z,pitch,yaw` / script `cam` use the walker's convention: UE cm and degrees, placed through ue.rs xf and
//! looking down UE +X (ue_light.gd light_basis). The original player controller uses MaintainYFOV; its current
//! POV FOV scalar is stored directly as Bevy's vertical FOV and is also used by player mouse scaling.
//! Controls (windowed): hold right mouse to look, WASD/QE to move, Shift = x4.

use crate::level::{ue_look_basis, LevelState};
use crate::ue;
use bevy::camera::{PerspectiveProjection, Projection};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use serde_json::json;

#[derive(Component)]
pub struct FlyCam {
    pub yaw: f32,
    pub pitch: f32,
}

/// A pending camera placement in UE terms [x, y, z, pitch, yaw].
#[derive(Resource, Default)]
pub struct CamRequest(pub Option<[f32; 5]>);

pub struct CameraPlugin;

/// Consumers of raw camera fields must run after this set, not before the camera's pose update.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraSet { PlayerRig }

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CamRequest>()
            .init_resource::<CamMode>()
            .init_resource::<FpInFly>()
            .add_systems(Startup, (spawn_cam, load_rig))
            .add_systems(Update, (place_cam, auto_place, toggle_mode, fly, apply_look, height_fog).chain())
            // the player camera is built from THIS frame's posed skeleton: GetFirstPersonCameraLocation rva=0x14bce70 /
            // GetFirstPersonCameraRotation rva=0x14bcfa0 read Spine1 / Position off the mesh the frame draws. The joints'
            // GlobalTransforms are only refreshed by TransformSystems::Propagate (PostUpdate); reading them in Update gave
            // last frame's skeleton, so while turning the camera trailed the 1P arms by a frame (user 22:01: "when you
            // move the camera ... the arms and the sword perfectly track with the camera" in the real game)
            .add_systems(PostUpdate, (player_rig, publish_other_cameras).chain()
                .in_set(CameraSet::PlayerRig).after(bevy::transform::TransformSystems::Propagate));
    }
}

/// LateTick 0x154d190 updates the raw first-person camera of living characters even
/// when they are not the view target. The non-first-person (+0xf08=false) branch
/// does not apply lookup collision or cosmetic offsets. Reconstruct it from the
/// current drawn pose, using the same camera component defaults as player_rig.
/// Missing bones/unsupported perspectives have no synthetic actor-position fallback.
fn publish_other_cameras(
    mut sim: NonSendMut<crate::sim::Sim>, rig: Res<Rig>,
    bodies: Query<(&crate::fighter::BodyJoints, &GlobalTransform, &ChildOf)>,
    fighters: Query<&crate::fighter::Fighter>, joints: Query<&GlobalTransform>,
) {
    let views = sim.0.fighters();
    let rq = |p: f32, y: f32, r: f32| ue::rot_quat(Some(&json!({"Pitch": p, "Yaw": y, "Roll": r})));
    let ue_v = |v: Vec3| [v.x * 100.0, v.z * 100.0, v.y * 100.0];
    let tick = sim.0.ticks();
    let world_generation = sim.0.world_generation();
    for (body, mesh, parent) in &bodies {
        let Ok(fighter) = fighters.get(parent.parent()) else { continue };
        if sim.0.raw_camera_1p(fighter.id).is_some() || sim.0.first_person(fighter.id) != Some(false) { continue; }
        let Some(view) = views.iter().find(|v| v.id == fighter.id && v.health > 0.0) else { continue };
        let bone = |name: &str| body.names.iter().position(|n| n.eq_ignore_ascii_case(name))
            .and_then(|i| body.joints.get(i)).and_then(|e| joints.get(*e).ok());
        let (Some(position), Some(spine)) = (bone("Position"), bone("Spine1")) else { continue };
        let pq = to_ue_q(position.compute_transform().rotation);
        let rot = qmul(qmul(pq, rq(0.0, 0.0, -(view.look_up + rig.fp_lookup_offset))), rq(90.0, 0.0, -90.0));
        let scale_z = mesh.compute_transform().scale.y;
        if !scale_z.is_finite() || scale_z <= 0.0 { continue; }
        let loc = spine.translation() + Vec3::from(ue::xf(None, rot, None).matrix3
            * bevy::math::Vec3A::new(0.0, 41.625 * scale_z, 0.0)) * 0.01;
        let angles = mordhau_core::combat::geometry::quat_rotator(mordhau_core::ue::FQuat::new(rot[0], rot[1], rot[2], rot[3]));
        let forward = mordhau_core::ue::rotator_vector(angles.0, angles.1);
        sim.0.publish_raw_camera_1p(fighter.id, crate::sim::RawCamera1P {
            location_ue_cm: ue_v(loc), forward_ue: [forward.x, forward.y, forward.z],
            first_person: false, tick, world_generation,
        });
    }
}

/// Evidence only (first-person r1, script `view fly1p`): keep fighter 0 in first person (1P pose + 1P part set)
/// while the fly camera looks at it from outside; not game behaviour.
#[derive(Resource, Default, Clone, Copy)]
pub struct FpInFly(pub bool);

/// Which camera drives the view: the fly / level-walker camera, or the player rig (3P / 1P, PlayerControl.third_person).
/// Windowed runs start on the player once a fighter exists; `cam` (script / --cam) switches to Fly, F1 toggles.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub enum CamMode {
    #[default]
    Fly,
    Player,
}

/// The character camera (UMordhauCameraComponent; BP_CharacterCameraComponent -> BP_MordhauCameraComponent class
/// defaults over the native ctor UMordhauCameraComponent::UMordhauCameraComponent, extract/native/decomp
/// UMordhauCameraComponent.cpp 2110-2196). UE FieldOfView is horizontal.
#[derive(Resource, Clone, Debug, serde::Serialize)]
pub struct Rig {
    pub fov_h: f32,
    /// MaxThirdPersonFOV (+0x820, ctor 78): third person caps the FOV at it (ComputeCameraPOV, `CameraStyle == 0 &&
    /// MaxThirdPersonFOV <= FOV -> FOV = MaxThirdPersonFOV`)
    pub max_3p_fov: f32,
    pub offset_ue: [f32; 3],
    pub rotation_offset: [f32; 3],
    /// ThirdPersonHipsSmoothSpeed (+0x854, ctor 0.5), ThirdPersonHipsSmoothLimits (+0x858, ctor (0, 50, 25)),
    /// ThirdPersonTeleportThreshold (+0x864, ctor 20)
    pub hips_speed: f32,
    pub hips_limits: [f32; 3],
    pub teleport: f32,
    /// FirstPersonLookUpOffset (+0x7e4, ctor 5.27)
    pub fp_lookup_offset: f32,
    /// ThirdPersonAimingCameraOffset / ThirdPersonAimingCameraRotationOffset (+0x848 / +0x83c), AimingCameraActivate /
    /// DisableChangeSpeed (+0x7e8 / +0x7ec)
    pub aim_offset_ue: [f32; 3],
    pub aim_rotation_offset: [f32; 3],
    pub aim_on_speed: f32,
    pub aim_off_speed: f32,
    pub source: String,
}

/// ComputeCameraPOV state (SmoothedHipsOffset +0x?, PreviousHipsLocation), UE cm, component space
#[derive(Resource, Default, Clone, Debug, serde::Serialize)]
pub struct RigState {
    pub smoothed_hips: [f32; 3],
    pub prev_hips: Option<[f32; 3]>,
    /// AimingBlendWeight (+0x7f0): FInterpTo toward 1 while aiming (ranged), 0 otherwise
    pub aiming: f32,
    pub collision_hits: u64,
    pub last_hit: Option<String>,
    /// evidence: the first-person view (UE cm location, UE forward axis, Position socket quat in UE)
    pub fp_view: Option<[f32; 10]>,
    /// evidence (first-person r1): bone -> (forward, right, up) cm in the final 1P camera frame
    pub fp_probe: Vec<(String, [f32; 3])>,
    /// APlayerCameraManager's current FOV (the last POV's, read by GetFirstPersonCameraCosmeticLocationOffset)
    pub last_fov: f32,
    /// CameraLocation1PCosmeticOffset (UE cm, world) last applied
    pub fp_cosmetic: [f32; 3],
    /// EVD_CAM_010: LookUpCollisionOffset (+0x7d0) and CameraCollisionLocationOffset (UE cm, world) of the last
    /// UpdateFirstPersonCamera
    pub look_up_collision_offset: f32,
    pub camera_collision_location_offset: [f32; 3],
}

/// FC_CameraLocationScale (Mordhau/Content/Mordhau/Blueprints/FC_CameraLocationScale, the BP_MordhauCharacter
/// Camera_GEN_VARIABLE CameraFOVToLocationOffsetScaleCurve): linear keys (79, 0), (101, 7), (110, 7); UE
/// FRichCurve::Eval holds the end values outside the key range
pub fn fov_to_location_offset(fov: f32) -> f32 {
    const K: [(f32, f32); 3] = [(79.0, 0.0), (101.0, 7.0), (110.0, 7.0)];
    if fov <= K[0].0 {
        return K[0].1;
    }
    for w in K.windows(2) {
        if fov <= w[1].0 {
            return w[0].1 + (w[1].1 - w[0].1) * (fov - w[0].0) / (w[1].0 - w[0].0);
        }
    }
    K[2].1
}

pub const CAMERA_BP: &str = "Mordhau/Content/Mordhau/Blueprints/Characters/BP_CharacterCameraComponent";

fn load_rig(mut commands: Commands, spec: Option<Res<crate::specdata::SpecData>>, src: Res<crate::source::Source>) {
    let s = spec.as_deref();
    let f = |fld: &str, d: f64| s.map(|s| s.f("ENT_CAM_CAMERA", fld, d)).unwrap_or(d) as f32;
    let v = |fld: &str, d: [f64; 3]| s.map(|s| s.v3("ENT_CAM_CAMERA", fld, d)).unwrap_or(d).map(|x| x as f32);
    // native ctor values (rva 0x14af2f0 UMordhauCameraComponent::UMordhauCameraComponent, decomp lines 2174-2194), overridden by the Blueprint chain's class defaults when present
    let mut rig = Rig {
        fov_h: f("FLD_CAM_FIELD_OF_VIEW", 110.0),
        max_3p_fov: 78.0,
        offset_ue: v("FLD_CAM_THIRD_PERSON_CAMERA_OFFSET", [-150.0, 0.0, 80.0]),
        rotation_offset: v("FLD_CAM_THIRD_PERSON_ROTATION_OFFSET", [-5.0, 0.0, 0.0]),
        hips_speed: 0.5,
        hips_limits: [0.0, 50.0, 25.0],
        teleport: 20.0,
        fp_lookup_offset: 5.2700005,
        // ThirdPersonAiming* are not written by the ctor (rva 0x14af2f0 UMordhauCameraComponent::UMordhauCameraComponent):
        // zero-initialised UObject memory unless the Blueprint chain sets them
        aim_offset_ue: [0.0, 0.0, 0.0],
        aim_rotation_offset: [0.0, 0.0, 0.0],
        // ctor rva 0x14af2f0: AimingCameraActivateChangeSpeed = 7.5, AimingCameraDisableChangeSpeed = 2.0
        aim_on_speed: 7.5,
        aim_off_speed: 2.0,
        source: "spec ENT_CAM_CAMERA + native ctor".into(),
    };
    if let Some(vfs) = &src.vfs {
        let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
        let d = pk.defaults(CAMERA_BP);
        let g = |k: &str| d.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
        let gv = |k: &str| d.get(k).map(|x| ["X", "Y", "Z"].map(|c| x.get(c).and_then(|y| y.as_f64()).unwrap_or(0.0) as f32));
        if let Some(x) = g("MaxThirdPersonFOV") {
            rig.max_3p_fov = x;
        }
        if let Some(x) = g("ThirdPersonHipsSmoothSpeed") {
            rig.hips_speed = x;
        }
        if let Some(x) = gv("ThirdPersonHipsSmoothLimits") {
            rig.hips_limits = x;
        }
        if let Some(x) = g("ThirdPersonTeleportThreshold") {
            rig.teleport = x;
        }
        if let Some(x) = g("FirstPersonLookUpOffset") {
            rig.fp_lookup_offset = x;
        }
        // evidence only (first-person r2): MH_FP_LOOKUP_OFFSET overrides FirstPersonLookUpOffset to test the 1P framing
        // against footage; not game behaviour
        if let Some(x) = std::env::var("MH_FP_LOOKUP_OFFSET").ok().and_then(|v| v.parse::<f32>().ok()) {
            rig.fp_lookup_offset = x;
        }
        if let Some(x) = gv("ThirdPersonAimingCameraOffset") {
            rig.aim_offset_ue = x;
        }
        if let Some(r) = d.get("ThirdPersonAimingCameraRotationOffset") {
            let f = |k: &str| r.get(k).and_then(|y| y.as_f64()).unwrap_or(0.0) as f32;
            rig.aim_rotation_offset = [f("Pitch"), f("Yaw"), f("Roll")];
        }
        if let Some(x) = g("AimingCameraActivateChangeSpeed") {
            rig.aim_on_speed = x;
        }
        if let Some(x) = g("AimingCameraDisableChangeSpeed") {
            rig.aim_off_speed = x;
        }
        if !d.is_empty() {
            rig.source.push_str(" + BP_CharacterCameraComponent defaults (paks)");
        }
    }
    commands.insert_resource(rig);
    commands.insert_resource(RigState::default());
}

/// UE Hamilton product a * b, quaternions (x, y, z, w) (FQuat::operator*)
fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

/// Bevy quaternion <-> UE quaternion (the Y/Z swap of a rotation is (x, z, y, -w), an involution)
fn to_ue_q(q: Quat) -> [f32; 4] {
    [q.x, q.z, q.y, -q.w]
}

/// EVD_CAM_010: one UpdateFirstPersonCamera step of LookUpCollisionOffset (rva=0x14dcf80, machine code
/// 0x1414dd41f-0x1414dd488): `hit_time` = the ECC_Camera sweep's Hit.Distance / |End - Start| (None: no hit or
/// |End - Start| <= 1); target = ((t L - L) 0.5 + L) - L; FInterpTo(cur, target, speed, dt) with the speed (20, or 3 when
/// |target| < |cur|) in the DeltaTime slot and dt as InterpSpeed, as the exe calls it
pub fn look_up_collision_step(cur: f32, look_up: f32, hit_time: Option<f32>, dt: f32) -> f32 {
    let target = match hit_time {
        Some(t) => ((t * look_up - look_up) * 0.5 + look_up) - look_up,
        None => 0.0,
    };
    let speed = if target.abs() < cur.abs() { 3.0 } else { 20.0 };
    finterp_to(cur, target, speed, dt)
}

/// FMath::FInterpTo
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

/// FMath::VInterpTo
fn vinterp_to(cur: [f32; 3], target: [f32; 3], dt: f32, speed: f32) -> [f32; 3] {
    if speed <= 0.0 {
        return target;
    }
    let d = [target[0] - cur[0], target[1] - cur[1], target[2] - cur[2]];
    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 1e-8 {
        return target;
    }
    let a = (dt * speed).clamp(0.0, 1.0);
    [cur[0] + d[0] * a, cur[1] + d[1] * a, cur[2] + d[2] * a]
}

pub(crate) fn toggle_mode(keys: Option<Res<ButtonInput<KeyCode>>>, mut mode: ResMut<CamMode>, sc: Res<crate::script::Script>, sim: NonSend<crate::sim::Sim>, mut started: Local<bool>) {
    if let Some(k) = keys {
        if k.just_pressed(KeyCode::F1) {
            *mode = if *mode == CamMode::Fly { CamMode::Player } else { CamMode::Fly };
            *started = true;
        }
    }
    // a windowed run hands the view to the player as soon as fighter 0 exists
    if !*started && sc.mode == "windowed" && !sim.0.fighters().is_empty() {
        *mode = CamMode::Player;
        *started = true;
    }
}

/// UE rotator (pitch, yaw) -> camera rotation in Bevy space (looks down the rotator's +X, ue_light.gd light_basis)
pub fn ue_view_rotation(pitch: f32, yaw: f32) -> Quat {
    let xf = ue::xf(None, ue::rot_quat(Some(&json!({"Pitch": pitch, "Yaw": yaw}))), None);
    ue_look_basis(&xf)
}

/// The player rig, a port of UMordhauCameraComponent::ComputeCameraPOV_Implementation (rva 0x14b5520; decomp UMordhauCameraComponent.cpp
/// 9-1112) for CameraStyle 0 (third person) and the first-person head view:
///   rotation = Quat(CameraRotation1P) * Quat(ThirdPersonRotationOffset)   (lines 546-571; CameraRotation1P = the
///              control rotation here: UpdateFirstPersonCamera's look-up / turn caps are not ported)
///   location = Spine1 world location (GetSocketTransform(Spine1, RTS_World), line 397)
///            + rotation * (ThirdPersonCameraOffset x mesh scale)
///            + mesh rotation * (SmoothedHipsOffset x mesh scale)                       (lines 598-631)
///   SmoothedHipsOffset += PreviousHips - Hips (component space, when the jump is under ThirdPersonTeleportThreshold),
///   VInterpTo(0, ThirdPersonHipsSmoothSpeed), clamped to +-ThirdPersonHipsSmoothLimits (lines 227-299)
///   FOV: FieldOfView, capped at MaxThirdPersonFOV in third person (line 210)
///   aiming: Slerp(rotation, Quat(CameraRotation1P) * Quat(ThirdPersonAimingCameraRotationOffset), AimingBlendWeight)
///           and Lerp(location, CameraLocation1P + aim rotation * ThirdPersonAimingCameraOffset, AimingBlendWeight)
///           (lines 147-182, 575-718); collision sweep (lines 1022-1106)
/// 1P = GetFirstPersonCameraLocation / Rotation (rva 0x14bce70 / 0x14bcfa0). Cinematic / vehicle / dead cameras and
/// UpdateFirstPersonCamera's turn caps (rva 0x14dcf80) are not ported (UNCONFIRMED).
#[allow(clippy::too_many_arguments)]
pub fn player_rig(
    mode: Res<CamMode>,
    rig: Option<Res<Rig>>,
    mut st: Option<ResMut<RigState>>,
    pc: Option<Res<crate::input::PlayerControl>>,
    mut sim: NonSendMut<crate::sim::Sim>,
    time: Res<Time>,
    fighters: Query<(&crate::fighter::Fighter, &Children)>,
    // Without<FlyCam>: the camera's GlobalTransform is written below (Bevy B0001 otherwise: a startup panic)
    bodies: Query<(&crate::fighter::BodyJoints, &GlobalTransform), Without<FlyCam>>,
    joints: Query<&GlobalTransform, Without<FlyCam>>,
    windows: Query<&Window>,
    mut cam: Query<(&mut Transform, &mut GlobalTransform, &mut Projection), With<FlyCam>>,
    mut shakes: Option<ResMut<crate::shake::CameraShakes>>,
    us: Option<Res<crate::usersettings::UserSettings>>,
    mut speed_fov: Local<f32>,
    held: Query<(&crate::weapon::HeldWeapon, &GlobalTransform), Without<FlyCam>>,
) {
    // Never expose a preceding-frame/local-player sample as a remote or current camera.
    sim.0.clear_raw_cameras_1p();
    if *mode != CamMode::Player {
        return;
    }
    let (Some(rig), Some(pc), Some(st)) = (rig, pc, st.as_mut()) else { return };
    let views = sim.0.fighters();
    let Some(v) = views.iter().find(|v| v.id == pc.id) else { return };
    let (yaw, pitch) = if pc.yaw_initialised { (pc.yaw, pc.pitch) } else { (v.yaw, 0.0) };
    // the fighter's mesh root and joints
    let mut body: Option<(&crate::fighter::BodyJoints, &GlobalTransform)> = None;
    for (f, ch) in fighters.iter() {
        if f.id == pc.id {
            for c in ch.iter() {
                if let Ok(b) = bodies.get(c) {
                    body = Some(b);
                }
            }
        }
    }
    let joint = |name: &str| -> Option<GlobalTransform> {
        let (b, _) = body?;
        let j = b.names.iter().position(|n| n.eq_ignore_ascii_case(name))?;
        joints.get(b.joints[j]).ok().copied()
    };
    let centre = Vec3::new(v.loc[0], v.loc[2], v.loc[1]) * 0.01;
    let rq = |p: f32, y: f32, r: f32| ue::rot_quat(Some(&json!({"Pitch": p, "Yaw": y, "Roll": r})));
    let ctrl = ue_view_rotation(pitch, yaw);
    // AimingBlendWeight: only ranged weapons aim (UMordhauCameraComponent ComputeCameraPOV rva 0x14b5520 decomp 147-182: bUsesRangedCamera + URangedDrawMotion / URangedReleaseMotion -> 1, FMath::FInterpTo(AimingBlendWeight, target, DeltaTime, Activate / DisableChangeSpeed)); the melee
    // loadouts here never aim, so it eases to 0 (target 1 would need the ranged motion's IsAiming, not ported)
    let aiming_target = 0.0;
    let sp = if aiming_target > st.aiming { rig.aim_on_speed } else { rig.aim_off_speed };
    st.aiming = finterp_to(st.aiming, aiming_target, time.delta_secs(), sp);
    // first-person view (also the aiming target of 3P):
    // GetFirstPersonCameraRotation (rva 0x14bcfa0, decomp 1884-1995): (Offset * Socket("Position") world rotation) *
    // MakeFromEuler(-(LookUp + FirstPersonLookUpOffset), 0, 0) * FRotator(90, 0, -90), in UE FQuat products (the
    // VectorQuaternionMultiply2 operand order of the three products read off the masks); Offset = UpdateFirstPersonCamera's
    // (rva 0x14dcf80) Offset argument: FRotator::ZeroRotator from both callers (AMordhauCharacter::LateTick rva=0x154d190
    // decomp 12036, UMordhauAnimInstance::NativeUpdateAnimation decomp 720), confirmed zero (fidelity-audit r2).
    // GetFirstPersonCameraLocation (rva 0x14bce70, decomp 2268-2312): Spine1 world location + Rot.RotateVector((0, 0,
    // 41.625 x mesh scale Z)).
    let fp: Option<(Vec3, [f32; 4])> = match (joint("Position").map(|g| to_ue_q(g.compute_transform().rotation)), joint("Spine1")) {
        (Some(pq), Some(sp1)) => {
            let rot_at = |look: f32| qmul(qmul(qmul([0.0, 0.0, 0.0, 1.0], pq), rq(0.0, 0.0, -(look + rig.fp_lookup_offset))), rq(90.0, 0.0, -90.0));
            let r = rot_at(pitch);
            let xf = ue::xf(None, r, None);
            let scale_z = body.map(|(_, m)| m.compute_transform().scale.y).unwrap_or(1.0);
            // GetFirstPersonCameraLocation(Offset, look) without the cosmetic offset
            let loc_at = |look: f32| sp1.translation() + Vec3::from(ue::xf(None, rot_at(look), None).matrix3 * bevy::math::Vec3A::new(0.0, 41.625 * scale_z, 0.0)) * 0.01;
            let fwd = Vec3::from(xf.matrix3 * bevy::math::Vec3A::X);
            // CameraLocation1PCosmeticOffset: UMordhauCameraComponent::GetFirstPersonCameraCosmeticLocationOffset
            // rva=0x14bcdb0 = CameraRotation1P.RotateVector((FC_CameraLocationScale(CurrentFOV), 0, 0)), CurrentFOV =
            // the AMordhauCameraManager's last FOV (90 without one); set by UpdateFirstPersonCamera rva=0x14dcf80 when
            // the character flag it tests is set (UNCONFIRMED which flag: taken as the local view target) and added to
            // CameraLocation1P by ComputeCameraPOV rva=0x14b5520 (CameraStyle 1 branch, decomp ~816-835)
            let cur_fov = if st.last_fov > 0.0 { st.last_fov } else { 90.0 };
            let cos = fwd * fov_to_location_offset(cur_fov) * 0.01;
            st.fp_cosmetic = [cos.x * 100.0, cos.z * 100.0, cos.y * 100.0];
            // MH_FP_CAM_DOWN=<cm>: test-only lowering of the 1P camera (default 0). The reader method (2026-10-06,
            // state/live_view_1p.json) measured the real camera at Spine1 + 41.6 cm, matching this exe formula; the
            // footage-matched 8 cm default is removed (the low sword was GripLocationLocal, mh-sim load.rs)
            let down = std::env::var("MH_FP_CAM_DOWN").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
            // EVD_CAM_010: UMordhauCameraComponent::UpdateFirstPersonCamera rva=0x14dcf80 (decomp
            // UMordhauCameraComponent.cpp 1700-1878), the cosmetic-offset branch (the same character flag as above):
            // End = CameraLocation1P (look L) + C, Start = GetFirstPersonCameraLocation(Offset, 0) + C (C = the cosmetic
            // offset); |End - Start| > 1 -> UWorld::SweepSingleByChannel(Start, End, identity, ECC_Camera (4),
            // FCollisionShape Sphere 30 (0x41f0000000000002 at 0x1414dd3b6)); a hit -> target = ((Hit.Distance / |End -
            // Start|) L - L) 0.5 + L - L, else 0. LookUpCollisionOffset = FMath::FInterpTo(cur, target, 20 (3 when
            // |target| < |cur|), dt): the machine code passes the speed in the DeltaTime slot and dt as InterpSpeed
            // (0x1414dd479-0x1414dd483; FInterpTo uses their product). CameraLocation1P = GetFirstPersonCameraLocation(
            // Offset, L + LookUpCollisionOffset); CameraCollisionLocationOffset = that - the look-L location.
            let ue_v = |v: Vec3| [v.x * 100.0, v.z * 100.0, v.y * 100.0];
            let (start, end) = (loc_at(0.0) + cos, loc_at(pitch) + cos);
            let len = (end - start).length() * 100.0;
            let mut hit = None;
            if len > 1.0 {
                if let Some((t, what)) = sim.0.camera_channel_sweep(ue_v(start), ue_v(end), 30.0) {
                    hit = Some(t);
                    st.last_hit = Some(what);
                }
            }
            st.look_up_collision_offset = look_up_collision_step(st.look_up_collision_offset, pitch, hit, time.delta_secs());
            let base = loc_at(pitch + st.look_up_collision_offset);
            if let Some(first_person) = sim.0.first_person(pc.id) {
                // Native UpdateFirstPersonCamera stores base location separately from cosmetic. Its
                // bIsFirstPerson branch applies lookup collision only in FP; rotation remains at L.
                let raw_loc = if first_person { base } else { loc_at(pitch) };
                let qr = mordhau_core::combat::geometry::quat_rotator(mordhau_core::ue::FQuat::new(r[0], r[1], r[2], r[3]));
                let forward = mordhau_core::ue::rotator_vector(qr.0, qr.1);
                let tick = sim.0.ticks();
                let world_generation = sim.0.world_generation();
                sim.0.publish_raw_camera_1p(pc.id, crate::sim::RawCamera1P {
                    location_ue_cm: ue_v(raw_loc), forward_ue: [forward.x, forward.y, forward.z],
                    first_person, tick, world_generation,
                });
            }
            st.camera_collision_location_offset = ue_v(base - loc_at(pitch));
            let p = base + cos - Vec3::Y * down * 0.01;
            st.fp_view = Some([p.x * 100.0, p.z * 100.0, p.y * 100.0, fwd.x, fwd.z, fwd.y, pq[0], pq[1], pq[2], pq[3]]);
            Some((p, r))
        }
        _ => None,
    };
    // EVD_CAM_010: the anim instance reads CameraCollisionLocationOffset on its next update (UMordhauAnimInstance
    // decomp 1482-1504); without the first-person branch it is zeroed (UpdateFirstPersonCamera decomp 1729-1733)
    if fp.is_none() || pc.third_person {
        st.look_up_collision_offset = 0.0;
        st.camera_collision_location_offset = [0.0; 3];
    }
    sim.0.set_camera_collision_offset(pc.id, st.camera_collision_location_offset);
    let (pos, rot, fov_h, ue_q) = if pc.third_person {
        let base_q = rq(pitch, yaw, 0.0);
        let q_n = qmul(base_q, rq(rig.rotation_offset[0], rig.rotation_offset[1], rig.rotation_offset[2]));
        let q_a = qmul(base_q, rq(rig.aim_rotation_offset[0], rig.aim_rotation_offset[1], rig.aim_rotation_offset[2]));
        // FQuat::Slerp_NotNormalized(normal, aiming, AimingBlendWeight) (decomp 643-644)
        let qn = Quat::from_xyzw(q_n[0], q_n[1], q_n[2], q_n[3]);
        let qa = Quat::from_xyzw(q_a[0], q_a[1], q_a[2], q_a[3]);
        let qb = qn.slerp(qa, st.aiming);
        let q = [qb.x, qb.y, qb.z, qb.w];
        let xf = ue::xf(None, q, None);
        // the normal location uses the normal rotation q_n and ThirdPersonCameraOffset (ComputeCameraPOV rva 0x14b5520,
        // decomp 612-626)
        let xf_n = ue::xf(None, q_n, None);
        let o = rig.offset_ue;
        let off = Vec3::from(xf_n.matrix3 * bevy::math::Vec3A::new(o[0], o[2], o[1])) * 0.01;
        let base = joint("Spine1").map(|g| g.translation()).unwrap_or(centre);
        // hips smoothing in component space (UE cm)
        let mut hips_off = Vec3::ZERO;
        if let (Some(h), Some((_, mroot))) = (joint("Hips"), body) {
            let lp = mroot.affine().inverse().transform_point3(h.translation());
            let hips = [lp.x * 100.0, lp.z * 100.0, lp.y * 100.0];
            if let Some(prev) = st.prev_hips {
                let d = [prev[0] - hips[0], prev[1] - hips[1], prev[2] - hips[2]];
                if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < rig.teleport {
                    for i in 0..3 {
                        st.smoothed_hips[i] += d[i];
                    }
                }
            }
            let mut sm = vinterp_to(st.smoothed_hips, [0.0; 3], time.delta_secs(), rig.hips_speed);
            for i in 0..3 {
                sm[i] = sm[i].clamp(-rig.hips_limits[i], rig.hips_limits[i]);
            }
            st.smoothed_hips = sm;
            st.prev_hips = Some(hips);
            hips_off = mroot.affine().transform_vector3(Vec3::new(sm[0], sm[2], sm[1]) * 0.01);
        }
        let normal_pos = base + off + hips_off;
        // aiming location (decomp 687-718): CameraLocation1P + CameraLocation1PCosmeticOffset (zero, UNCONFIRMED: the
        // cosmetic offset is not ported) + q_aim.RotateVector(ThirdPersonAimingCameraOffset), lerped from the normal
        // location by AimingBlendWeight
        let aim_pos = {
            let xf_a = ue::xf(None, q_a, None);
            let a = rig.aim_offset_ue;
            let fp_pos = fp.map(|f| f.0).or_else(|| joint("Head").map(|g| g.translation())).unwrap_or(centre);
            fp_pos + Vec3::from(xf_a.matrix3 * bevy::math::Vec3A::new(a[0], a[2], a[1])) * 0.01
        };
        let mut cam_pos = normal_pos.lerp(aim_pos, st.aiming);
        // collision (ComputeCameraPOV rva 0x14b5520, decomp 1022-1106, CameraStyle 0 only): UWorld::SweepSingleByObjectType
        // of FCollisionShape {Sphere, 20} (0x41a0000000000002) with ObjectTypesToQuery 1 (ECC_WorldStatic) from the Head
        // socket location (SmoothedHeadLocation, decomp 318-324) + 40 x dir(head -> camera) to the camera; a hit puts the
        // camera at Hit.Location, bStartPenetrating (flag bit 2) at the sweep start
        if let Some(head) = joint("Head").map(|g| g.translation()) {
            let to_cam = (cam_pos - head).normalize_or_zero();
            let start = head + to_cam * 0.40;
            let ue = |v: Vec3| [v.x * 100.0, v.z * 100.0, v.y * 100.0];
            if let Some((loc, pen, what)) = sim.0.camera_sweep(ue(start), ue(cam_pos), 20.0) {
                cam_pos = if pen { start } else { Vec3::new(loc[0], loc[2], loc[1]) * 0.01 };
                st.collision_hits += 1;
                st.last_hit = Some(what);
            }
        }
        (cam_pos, ue_look_basis(&xf), rig.fov_h.min(rig.max_3p_fov), q)
    } else {
        match fp {
            Some((p, r)) => (p, ue_look_basis(&ue::xf(None, r, None)), rig.fov_h, r),
            None => (joint("Head").map(|g| g.translation()).unwrap_or(centre), ctrl, rig.fov_h, rq(pitch, yaw, 0.0)),
        }
    };
    // camera shakes (APlayerCameraManager modifiers after ComputeCameraPOV; shake.rs, fidelity-audit r2)
    let (pos, rot, fov_h) = match shakes.as_mut() {
        Some(sh) if !sh.active.is_empty() => {
            let (sq, l, df) = sh.apply(time.delta_secs(), mordhau_core::ue::FQuat::from_array(ue_q), pitch);
            (pos + Vec3::new(l.x, l.z, l.y) * 0.01, ue_look_basis(&ue::xf(None, [sq.x, sq.y, sq.z, sq.w], None)), fov_h + df)
        }
        _ => (pos, rot, fov_h),
    };
    // the POV FOV from the player's FieldOfView setting (fidelity-audit r2): ComputeCameraPOV rva=0x14b5520 decomp
    // 196-220 + the sprint FOV offset AMordhauCharacter::LODTick rva=0x154c390 (decomp 3638-3651: FInterpTo to
    // MaxSprintFOVOffset 5 at MaxSprintFOVOffsetInterpSpeed 4, BP_MordhauCharacter CDO); CurrentMotionFOVOffset is only
    // set by URangedDrawMotion::UpdateFOVOffset (rva 0x166eec0) and eases to 0 otherwise (not ported: melee only)
    let fov_h = match us.as_deref() {
        Some(u) => {
            let target = if sim.0.sprint_fov(pc.id) { 5.0 } else { 0.0 };
            *speed_fov = crate::usersettings::finterp_to(*speed_fov, target, time.delta_secs(), 4.0);
            u.pov_fov(*speed_fov, 0.0, pc.third_person, rig.max_3p_fov) + (fov_h - if pc.third_person { rig.fov_h.min(rig.max_3p_fov) } else { rig.fov_h })
        }
        None => fov_h,
    };
    if !pc.third_person {
        // evidence (first-person r1): where the hands / weapon bones sit in the 1P view
        let inv = Transform::from_translation(pos).with_rotation(rot).compute_affine().inverse();
        st.fp_probe = ["RightHand", "LeftHand", "RightWeapon", "LeftWeapon", "Head", "Spine1", "Spine2", "Position", "RightForeArm", "Hips", "RightUpLeg", "RightLeg", "RightFoot", "RightToeBase", "LeftFoot"]
            .iter()
            .filter_map(|n| joint(n).map(|g| {
                let l = inv.transform_point3(g.translation()) * 100.0;
                (n.to_string(), [-l.z, l.x, l.y])
            }))
            .collect();
        for (h, g) in held.iter().filter(|(h, _)| h.fighter == pc.id) {
            let l = inv.transform_point3(g.translation()) * 100.0;
            st.fp_probe.push((if h.left { "weapon_left".into() } else { "weapon".into() }, [-l.z, l.x, l.y]));
        }
    }
    st.last_fov = fov_h;
    let aspect = windows.iter().next().map(|w| w.width() / w.height().max(1.0)).unwrap_or(OFFSCREEN_SIZE.0 as f32 / OFFSCREEN_SIZE.1 as f32);
    for (mut t, mut g, mut p) in cam.iter_mut() {
        *t = Transform::from_translation(pos).with_rotation(rot);
        // after Propagate: the camera (a root entity) gets its GlobalTransform now, for this frame's render
        *g = GlobalTransform::from(*t);
        if let Projection::Perspective(pp) = &mut *p {
            // camera1p gauntlet piece 1 (2026-10-07, state/proofs/fp_render_offset.md round 3): the real game applies the
            // FOV value on the VERTICAL axis. FMinimalViewInfo::CalculateProjectionMatrixGivenView rva 0x2f2e570 takes
            // the Y-FOV branch (XAxisMultiplier = SizeY / SizeX), so the pixel focal at 1920x1080 is 540 / tan(FOV/2) =
            // 621 px for 82, not 1104: measured twice (the recorded blade line and finger bones land on the drawn sword and
            // hands only at 621 px in the same-instant captures fp_dump_1p / sync1; central-strip landmark shifts in the
            // motion1 video give 600-760 px). AMordhauPlayerController::BeginPlay rva 0x15c7ed0 (decomp 20575-20578) writes
            // GetLocalPlayer()->AspectRatioAxisConstraint = AspectRatio_MaintainYFOV (0) unconditionally, overriding
            // BaseEngine.ini's MaintainXFOV; ULocalPlayer::GetProjectionData 0x31bd200 passes that byte (+0x94). (Was: 2 atan(tan(FOV/2) / aspect), the horizontal reading.)
            let _ = aspect;
            pp.fov = fov_h.to_radians();
            // GNearClippingPlane: DefaultEngine.ini [/Script/Engine.Engine] NearClipPlane=3.0 cm (extract/config
            // DefaultEngine.ini:140, over BaseEngine's 10); the own head / body in first person is hidden by the 1P
            // part sets (fighter.rs first_person_parts, UpdateMeshVisibility 0x14ddc50), not by the near plane
            pp.near = 0.03;
        }
    }
}

/// `--offscreen`: the camera renders into this image (GPU, no window); screenshots read it.
#[derive(Resource, Clone)]
pub struct Offscreen(pub Handle<Image>);

pub const OFFSCREEN_SIZE: (u32, u32) = (1280, 720);

pub fn offscreen_image(images: &mut Assets<Image>) -> Handle<Image> {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
    let size = Extent3d { width: OFFSCREEN_SIZE.0, height: OFFSCREEN_SIZE.1, depth_or_array_layers: 1 };
    let mut img = Image::new_fill(size, TextureDimension::D2, &[0, 0, 0, 255], TextureFormat::Rgba8UnormSrgb, bevy::asset::RenderAssetUsages::default());
    img.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    images.add(img)
}

/// The camera entity, reserved in main.rs before Startup (mh-ui reads UiTarget at setup)
#[derive(Resource, Clone, Copy)]
pub struct CamEntity(pub Entity);

fn spawn_cam(mut commands: Commands, off: Option<Res<Offscreen>>, ce: Option<Res<CamEntity>>) {
    let mut e = match ce {
        Some(c) => commands.entity(c.0),
        None => commands.spawn_empty(),
    };
    e.insert((
        Camera3d::default(),
        // horizontal 90 deg FOV at 16:9 -> vertical 2*atan(tan(45 deg) * 9/16) (Bevy's fov is vertical)
        Projection::Perspective(PerspectiveProjection { fov: 2.0 * (9.0f32 / 16.0).atan(), far: 6000.0, ..default() }),
        Transform::from_xyz(0.0, 5.0, 0.0),
        FlyCam { yaw: 0.0, pitch: 0.0 },
        // UE 4.26 filmic tonemapper -> Bevy's closest curve (lighting.rs header, UNCONFIRMED approximation)
        bevy::core_pipeline::tonemapping::Tonemapping::AcesFitted,
        bevy::camera::Hdr,
    ));
    if let Some(o) = off {
        e.insert(bevy::camera::RenderTarget::Image(o.0.clone().into()));
    }
}

/// The map's look (lighting.rs Look) onto the camera when a level loads. rust-render r1: with the level's UE post
/// settings present, Bevy's tonemapper / bloom / fixed exposure / distance fog are off and UE's own chain runs
/// (uepost_render.rs UePostCam); the sky is the BP_Sky_Sphere cube (Skybox) and the stationary SkyLight's diffuse goes
/// on the environment light (uesky.rs). Without them (no paks) the r2 look stays (Bevy AcesFitted + Bloom).
fn apply_look(
    mut commands: Commands,
    look: Res<crate::lighting::Look>,
    mut clear: ResMut<ClearColor>,
    cams: Query<Entity, With<FlyCam>>,
    dbg: Res<crate::uepost_render::PostDebug>,
    mut seen: Local<u32>,
) {
    if look.version == *seen || look.version == 0 {
        return;
    }
    *seen = look.version;
    clear.0 = Color::linear_rgb(look.clear[0], look.clear[1], look.clear[2]);
    for e in cams.iter() {
        let mut c = commands.entity(e);
        c.insert(bevy::camera::Exposure { ev100: look.ev100 });
        c.remove::<bevy::pbr::DistanceFog>();
        match (&look.post, &look.lut) {
            (Some(ps), Some(lut)) => {
                c.remove::<bevy::post_process::bloom::Bloom>();
                c.insert((
                    bevy::core_pipeline::tonemapping::Tonemapping::None,
                    bevy::core_pipeline::tonemapping::DebandDither::Disabled,
                    crate::uepost_render::UePostCam::new(ps, lut.clone(), look.version, &dbg),
                ));
            }
            _ => {
                c.insert(bevy::core_pipeline::tonemapping::Tonemapping::AcesFitted);
                c.remove::<crate::uepost_render::UePostCam>();
                match look.bloom {
                    Some(i) => {
                        c.insert(bevy::post_process::bloom::Bloom { intensity: i, ..bevy::post_process::bloom::Bloom::NATURAL });
                    }
                    None => {
                        c.remove::<bevy::post_process::bloom::Bloom>();
                    }
                }
            }
        }
        match &look.sky_cube {
            Some(sky) => {
                c.insert(bevy::light::Skybox { image: Some(sky.clone()), brightness: 1.0, ..default() });
            }
            None => {
                c.remove::<bevy::light::Skybox>();
            }
        }
        match &look.env {
            Some((spec, diff, i)) => {
                c.insert(bevy::light::EnvironmentMapLight {
                    diffuse_map: look.sky_diffuse.clone().unwrap_or_else(|| diff.clone()),
                    specular_map: spec.clone(),
                    intensity: *i,
                    ..default()
                });
            }
            None => {
                c.remove::<bevy::light::EnvironmentMapLight>();
            }
        }
    }
}

/// UE ExponentialHeightFog evaluated at the camera height each frame as Bevy exponential distance fog (lighting.rs
/// header). Colour = FogInscatteringColor x the exposure (Bevy fogs the already-exposed lit colour).
fn height_fog(mut commands: Commands, look: Res<crate::lighting::Look>, cams: Query<(Entity, &GlobalTransform), With<FlyCam>>) {
    let Some(f) = &look.fog else { return };
    for (e, t) in cams.iter() {
        let d = f.density_at(t.translation().y);
        let c = Color::linear_rgb(f.inscatter[0] * look.exposure, f.inscatter[1] * look.exposure, f.inscatter[2] * look.exposure);
        commands.entity(e).insert(bevy::pbr::DistanceFog { color: c, falloff: bevy::pbr::FogFalloff::Exponential { density: d }, ..default() });
    }
}

pub fn ue_cam_transform(c: [f32; 5]) -> Transform {
    let xf = ue::xf(
        Some(&json!({"X": c[0], "Y": c[1], "Z": c[2]})),
        ue::rot_quat(Some(&json!({"Pitch": c[3], "Yaw": c[4]}))),
        None,
    );
    Transform::from_translation(xf.translation.into()).with_rotation(ue_look_basis(&xf))
}

fn place_cam(mut req: ResMut<CamRequest>, mut mode: ResMut<CamMode>, mut q: Query<(&mut Transform, &mut FlyCam)>) {
    let Some(c) = req.0.take() else { return };
    *mode = CamMode::Fly;
    for (mut t, mut f) in q.iter_mut() {
        *t = ue_cam_transform(c);
        let (y, p, _) = t.rotation.to_euler(EulerRot::YXZ);
        f.yaw = y;
        f.pitch = p;
    }
}

/// When a level finishes loading and no camera was asked for: stand behind the first PlayerStart, 1.7 m up,
/// looking along it (UNCONFIRMED framing, debugging aid only).
fn auto_place(lvl: Res<LevelState>, mut done: Local<String>, req: Res<CamRequest>, mut q: Query<(&mut Transform, &mut FlyCam)>) {
    if !lvl.loaded || *done == lvl.map || req.0.is_some() {
        return;
    }
    *done = lvl.map.clone();
    let Some(s) = lvl.starts.first() else { return };
    let rot = ue_look_basis(&s.xf);
    let fwd = rot * Vec3::NEG_Z;
    let pos = Vec3::from(s.xf.translation) - fwd * 4.0 + Vec3::Y * 0.8;
    for (mut t, mut f) in q.iter_mut() {
        *t = Transform::from_translation(pos).with_rotation(rot);
        let (y, p, _) = rot.to_euler(EulerRot::YXZ);
        f.yaw = y;
        f.pitch = p;
    }
}

fn fly(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    motion: Option<Res<AccumulatedMouseMotion>>,
    time: Res<Time>,
    mut q: Query<(&mut Transform, &mut FlyCam)>,
    mode: Res<CamMode>,
    screen: Option<Res<mh_ui::Screen>>,
) {
    // the front end owns the keyboard / mouse while it shows a menu (the backdrop camera stays put)
    if *mode != CamMode::Fly || screen.as_deref().is_some_and(|s| *s == mh_ui::Screen::MainMenu) {
        return;
    }
    let (Some(keys), Some(mouse), Some(motion)) = (keys, mouse, motion) else { return };
    for (mut t, mut f) in q.iter_mut() {
        if mouse.pressed(MouseButton::Right) {
            f.yaw -= motion.delta.x * 0.003;
            f.pitch = (f.pitch - motion.delta.y * 0.003).clamp(-1.54, 1.54);
            t.rotation = Quat::from_euler(EulerRot::YXZ, f.yaw, f.pitch, 0.0);
        }
        let mut d = Vec3::ZERO;
        let fwd = *t.forward();
        let right = *t.right();
        if keys.pressed(KeyCode::KeyW) { d += fwd; }
        if keys.pressed(KeyCode::KeyS) { d -= fwd; }
        if keys.pressed(KeyCode::KeyD) { d += right; }
        if keys.pressed(KeyCode::KeyA) { d -= right; }
        if keys.pressed(KeyCode::KeyE) { d += Vec3::Y; }
        if keys.pressed(KeyCode::KeyQ) { d -= Vec3::Y; }
        let speed = if keys.pressed(KeyCode::ShiftLeft) { 20.0 } else { 5.0 };
        t.translation += d.normalize_or_zero() * speed * time.delta_secs();
    }
}

#[cfg(test)]
mod fp_tests {
    /// FC_CameraLocationScale keys (79, 0) / (101, 7) / (110, 7), held outside the range
    /// EVD_MOV_013 regression: player_rig writes the camera's GlobalTransform (PostUpdate) and reads the joints' /
    /// bodies' / held weapons' GlobalTransforms, so those queries must exclude the camera (Bevy B0001 panicked at the
    /// first frame of every run with the PostUpdate change before Without<FlyCam>)
    #[test]
    fn player_rig_queries_do_not_conflict() {
        use bevy::ecs::system::System;
        let mut world = bevy::prelude::World::new();
        let mut sys = bevy::prelude::IntoSystem::into_system(super::player_rig);
        let _ = sys.initialize(&mut world);
    }

    #[test]
    fn look_up_collision_follows_the_exe_step() {
        // looking down 60 with the sweep blocked at 40 %: target 0.5 x -60 x (0.4 - 1) = +18, approached at
        // 20 / s; once clear it returns at 3 / s
        let a = super::look_up_collision_step(0.0, -60.0, Some(0.4), 1.0 / 60.0);
        assert!((a - 6.0).abs() < 1e-4, "{a}");
        let b = super::look_up_collision_step(a, -60.0, None, 1.0 / 60.0);
        assert!((b - 5.7).abs() < 1e-4, "{b}");
        // no hit from rest stays 0; a start-penetrating hit (t = 0) targets -L / 2
        assert_eq!(super::look_up_collision_step(0.0, -60.0, None, 1.0 / 60.0), 0.0);
        let c = super::look_up_collision_step(0.0, -60.0, Some(0.0), 1.0);
        assert!((c - 30.0).abs() < 1e-4, "{c}");
    }

    #[test]
    fn cosmetic_offset_follows_fc_camera_location_scale() {
        assert_eq!(super::fov_to_location_offset(78.0), 0.0);
        assert_eq!(super::fov_to_location_offset(79.0), 0.0);
        assert!((super::fov_to_location_offset(90.0) - 3.5).abs() < 1e-5);
        assert!((super::fov_to_location_offset(93.0) - 7.0 * 14.0 / 22.0).abs() < 1e-5);
        assert_eq!(super::fov_to_location_offset(101.0), 7.0);
        assert_eq!(super::fov_to_location_offset(120.0), 7.0);
    }
}

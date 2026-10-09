//! sim.rs - SimPlugin: the bridge between Bevy and the engine-neutral sims. The sim runs in UE units (cm, Z up,
//! degrees) at a fixed step in `FixedUpdate`; the bridge converts its fighter views to Bevy transforms (ue.rs rules).
//! Backends: `StubSim` (idle fighters), `CoreSim` (mordhau-core combat only, sim_core.rs) and `MhSim` (rust-combat's
//! mh-sim facade: exe-exact combat + mh-character movement on the map's collision + weapon traces, sim_mh.rs; the
//! default when its data is present).

use bevy::prelude::*;
use std::collections::HashMap;

/// Fixed step: the server's tick rate, DefaultEngine.ini [/Script/OnlineSubsystemUtils.IpNetDriver]
/// NetServerMaxTickRate=60 (extract/config/DefaultEngine.ini:188; mh-net node.rs NET_SERVER_MAX_TICK_RATE). A local
/// player's client frame rate is uncapped in the engine; one shared fixed step here (UNCONFIRMED for a listen client).
pub const TICK_HZ: f64 = 60.0;

/// One fighter as the sim sees it (UE space).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct FighterView {
    pub id: u32,
    pub team: Option<i64>,
    /// UE world location of the capsule centre, cm
    pub loc: [f32; 3],
    /// UE velocity, cm/s
    pub vel: [f32; 3],
    /// UE yaw, degrees
    pub yaw: f32,
    /// capsule half height, cm (the mesh origin sits at the capsule bottom)
    pub half_height: f32,
    pub falling: bool,
    pub crouched: bool,
    /// the control rotation pitch (LookUpValue, degrees, + up) the sim holds for this fighter; the camera reads it when
    /// no input system runs (offscreen scripted replays: camera1p gauntlet 2026-10-07)
    pub look_up: f32,
    /// current motion / animation state name (the reference's MotionSystem state names)
    pub state: String,
    pub health: f32,
    pub stamina: i64,
}

/// A one-off command, in the reference's input vocabulary (godot/game/actor/melee_input.gd -> MotionSystem requests;
/// mordhau-core combat::Input). `mv`: EAttackMove (0 RightStrike, 1 LeftStrike, 2 Stab, 3 AltStab, 4 Kick;
/// core/tests/scenarios/combat.json _doc), `angle` degrees.
#[derive(Clone, Debug, serde::Serialize)]
#[allow(dead_code)] // ToggleMode: script / bot use
pub enum SimInput {
    Attack { mv: i64, angle: f64 },
    Feint,
    Parry,
    ReleaseBlock,
    ToggleMode,
}

/// One frame of a fighter's input (mh_sim::SimInput's shape): axes and held keys persist, one-offs are consumed by the
/// next fixed step.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct FrameInput {
    pub fwd: f32,
    pub right: f32,
    pub jump: bool,
    pub sprint: bool,
    pub crouch: bool,
    pub yaw: Option<f32>,
    pub look_up: Option<f64>,
    pub attack: Option<(i64, f64)>,
    pub feint: bool,
    pub parry: Option<i64>,
    pub release_block: bool,
    pub switch_mode: bool,
    pub toggle_mode: bool,
}

impl FrameInput {
    pub fn add_event(&mut self, ev: &SimInput) {
        match ev {
            SimInput::Attack { mv, angle } => self.attack = Some((*mv, *angle)),
            SimInput::Feint => self.feint = true,
            // EBlockType 0 = Regular (scenario default, export_golden.gd header)
            SimInput::Parry => self.parry = Some(0),
            SimInput::ReleaseBlock => self.release_block = true,
            SimInput::ToggleMode => self.toggle_mode = true,
        }
    }
    /// clear the one-offs after a step consumed them
    pub fn consumed(&mut self) {
        self.jump = false;
        self.attack = None;
        self.feint = false;
        self.parry = None;
        self.release_block = false;
        self.switch_mode = false;
        self.toggle_mode = false;
    }
}

/// Raw character camera fields reconstructed by the current player camera update. UE world cm,
/// binary32. CameraLocation1P (+0xf18) excludes cosmetic location (+0xf24); forward is
/// CameraRotation1P (+0xf30).Vector(). bIsFirstPerson (+0xf08) is distinct from IsViewTarget.
/// This is a sampled runtime reconstruction, not an original-process memory read.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct RawCamera1P {
    pub location_ue_cm: [f32; 3],
    pub forward_ue: [f32; 3],
    pub first_person: bool,
    pub tick: u64,
    pub world_generation: u64,
}

/// Current held-mesh GetTrace, independent of the cached attack tracer history. Unit-scale
/// weapon transform in UE world cm; absent socket/pose/weapon is explicitly unsupported.
#[derive(Clone, Copy, Debug)]
pub struct CurrentWeaponTrace {
    /// Actual active-mode GripLocationLocal; independent of the collision trace start.
    pub grip_local_ue_cm: [f32; 3],
    pub start_local_ue_cm: [f32; 3],
    pub end_local_ue_cm: [f32; 3],
    pub start_ue_cm: [f32; 3],
    pub end_ue_cm: [f32; 3],
    pub weapon_world: mordhau_core::ue::FTransform,
    pub alternate_mode: bool,
    /// Current right-hand equipment generation; absent means an attachment cannot be identified.
    pub owner_equipment: Option<mordhau_core::combat::EquipmentId>,
}

/// What the runtime needs from a sim. The cores keep Rc-shared spec data, so the sim lives on the main thread (a Bevy
/// non-send resource).
#[allow(dead_code)] // combat / fighter_index / set_pose / weapon_world: the host API for mh-ui / mh-fx and weapon rendering
pub trait SimBackend: 'static {
    fn name(&self) -> &'static str;
    /// Changes when the backend replaces its world, even if elapsed clocks do not rewind.
    fn world_generation(&self) -> u64 { 0 }
    /// the map is loaded: build what the sim needs from it (collision); called before the first spawn of a map
    fn attach_level(&mut self, _vfs: Option<&std::sync::Arc<mh_pak::Vfs>>, _map: &str) -> Result<(), String> {
        Ok(())
    }
    fn spawn(&mut self, loc: [f32; 3], yaw: f32, team: Option<i64>) -> u32;
    /// a fighter driven by the backend's own AI; None = this backend has no bots
    fn spawn_bot(&mut self, _loc: [f32; 3], _yaw: f32, _team: Option<i64>) -> Option<u32> {
        None
    }
    /// the map's collision world (shared: camera sweep, surfaces, audio occlusion)
    fn collision(&self) -> Option<std::sync::Arc<mh_level::collision::CollisionWorld>> {
        None
    }
    /// the BotCharacterProfiles index the next spawn_bot will give its bot (the roster pick on the current rand()
    /// stream), None = no roster
    fn preview_bot_loadout(&self) -> Option<usize> {
        None
    }
    /// seed the world's rand() stream (the bots' and every other game-thread consumer's; sim.rs session_seed)
    fn set_rand_seed(&mut self, _seed: u32) {}
    /// step with the caller's dt (one rendered frame) instead of the fixed TICK_HZ step; false = not supported
    fn set_variable_dt(&mut self, _on: bool) -> bool {
        false
    }
    /// the weapon class the next spawn holds (the loadout's); backends without per-fighter weapons ignore it
    fn set_next_weapon(&mut self, _weapon: &str) {}
    /// the wearable classes (EWearableSlot order) the next spawn wears: their armor SpeedFactor / AccelerationFactor
    /// feed its movement (rust-character r9); backends without movement gear ignore it
    fn set_next_wearables(&mut self, _classes: &[String]) {}
    /// one fixed step with these inputs (fighter id -> input)
    fn tick(&mut self, dt: f32, inputs: &HashMap<u32, FrameInput>);
    fn ticks(&self) -> u64;
    fn fighters(&self) -> Vec<FighterView>;
    /// the combat world and a fighter id's index in it (animation reads motions from it)
    fn combat(&self) -> Option<&mordhau_core::combat::World> {
        None
    }
    fn export_combat_timings(&self) -> Result<mordhau_core::timing::TimingEdits, String> {
        Err("combat timing authoring is unavailable in this backend".into())
    }
    /// None restores stock timing data. False means a validated edit is queued until idle.
    fn apply_combat_timings(&mut self, _edits: Option<&mordhau_core::timing::TimingEdits>) -> Result<bool, String> {
        Err("combat timing authoring is unavailable in this backend".into())
    }
    fn combat_timing_status(&self) -> serde_json::Value { serde_json::Value::Null }
    fn fighter_index(&self, id: u32) -> Option<usize> {
        Some(id as usize)
    }
    /// bone names of the pose `set_pose` takes (the sim's trace skeleton order); None = poses not used
    fn pose_bones(&self) -> Option<Vec<String>> {
        None
    }
    /// the pose the sim stepped with last (component space, UE, `pose_bones` order): what the body renders
    fn pose_now(&self, _id: u32) -> Option<Vec<mordhau_core::ue::FTransform>> {
        None
    }
    /// skeletal mesh component transform relative to the actor (UE; the character BP's Mesh RelativeLocation/Rotation)
    fn mesh_xf(&self) -> Option<mordhau_core::ue::FTransform> {
        None
    }
    /// component-space UE bones for the coming step (weapon traces run against them)
    fn set_pose(&mut self, _id: u32, _bones: Vec<mordhau_core::ue::FTransform>) {}
    /// the weapon class fighter id holds
    fn weapon_path(&self, _id: u32) -> Option<String> {
        None
    }
    /// the fighter id of a sim fighter name (event "who" / "victim"); MhSim / CoreSim name fighters "F<id>"
    fn id_of(&self, name: &str) -> Option<u32> {
        name.strip_prefix('F').and_then(|n| n.parse().ok())
    }
    /// EPhysicalSurface of a floor body (the "foot_landed" event's floor_component); 0 = SurfaceType_Default
    fn floor_surface(&mut self, _component: u32) -> usize {
        0
    }
    /// move a fighter by an offset (UE cm): mh-world's MoveCharacter (riders carried by a pushable)
    fn move_by(&mut self, _id: u32, _offset: [f32; 3]) {}
    /// the left-hand equipment the next spawn holds ("" none; fidelity-audit r5)
    fn set_next_left(&mut self, _left: &str) {}
    /// the left-hand equipment class of fighter id and its world transform (UE)
    fn left_path(&self, _id: u32) -> Option<String> {
        None
    }
    fn left_world(&self, _id: u32) -> Option<mordhau_core::ue::FTransform> {
        None
    }
    /// AMordhauCharacter::bIsFirstPerson of fighter id (CameraStyle 1 && view target; mh-sim Sim::set_first_person)
    fn set_first_person(&mut self, _id: u32, _first_person: bool) {}
    /// AAdvancedCharacter::bIsViewTarget: the local player camera targets this fighter in either perspective.
    /// `debug_override` labels the external fly1p observer's synthetic target in attack evidence.
    fn set_view_target(&mut self, _id: u32, _is_view_target: bool, _debug_override: bool) {}
    /// Actual animation/perspective flag; None means this backend does not model it.
    fn first_person(&self, _id: u32) -> Option<bool> { None }
    /// Same-tick raw camera fields, populated after player_rig. Remote/unupdated cameras return None.
    fn raw_camera_1p(&self, _id: u32) -> Option<RawCamera1P> { None }
    fn clear_raw_cameras_1p(&mut self) {}
    fn publish_raw_camera_1p(&mut self, _id: u32, _camera: RawCamera1P) {}
    /// respawn fighter `id` as a new character at a UE location / yaw (the combat test level's round restart)
    fn respawn(&mut self, _id: u32, _loc: [f32; 3], _yaw: f32) {}
    /// (first-person r3) AAdvancedCharacter::RequestSuicide -> ServerSuicide_Implementation rva=0x14a02a0 (allowed unless
    /// the game mode's bSuicideAllowed is false; ctor true, AMordhauGameMode.cpp 1992) -> Suicide rva=0x14a3860:
    /// TakeDamage(10000) while alive
    fn suicide(&mut self, _id: u32) {}
    /// AMordhauCharacter::LODTick rva=0x154c390 (decomp 3638-3651) sprint FOV condition: the movement component's
    /// bWantsSprint (+0xd19) && MovementModifier (+0xd18) > 2 (fidelity-audit r2)
    fn sprint_fov(&self, _id: u32) -> bool {
        false
    }
    /// EVD_CAM_010: UWorld::SweepSingleByChannel of a sphere (UE cm) on ECC_Camera (channel 4) with the default query
    /// params: the first body that blocks the Camera channel; (Hit.Time, body name). Hit.Distance = Time x length
    fn camera_channel_sweep(&self, _start: [f32; 3], _end: [f32; 3], _radius: f32) -> Option<(f32, String)> {
        None
    }
    /// EVD_CAM_010: fighter id's camera CameraCollisionLocationOffset (world, UE cm) for its next anim update
    fn set_camera_collision_offset(&mut self, _id: u32, _offset: [f32; 3]) {}
    /// camera collision: sweep a sphere (UE cm) against WorldStatic bodies; (location, start_penetrating, body name)
    fn camera_sweep(&self, _start: [f32; 3], _end: [f32; 3], _radius: f32) -> Option<([f32; 3], bool, String)> {
        None
    }
    /// Last prepared collision trace (CurrentTraceStart / CurrentTraceEnd, UE world cm); can be old in idle.
    fn trace_sockets(&self, _id: u32) -> Option<([f32; 3], [f32; 3])> {
        None
    }
    /// GetTrace_Implementation 0x1629520 local normal/alternate mesh sockets for the current held weapon.
    /// Material updates transform these through their current component; motion OverrideTrace remains unsupported.
    fn weapon_trace_local(&self, _id: u32) -> Option<([f32; 3], [f32; 3])> { None }
    fn current_weapon_trace(&self, _id: u32) -> Option<CurrentWeaponTrace> { None }
    /// Actual SampleTracers segments from every step since the host last consumed them.
    fn drain_tracers(&mut self) -> Vec<TracerSegment> { Vec::new() }
    /// UAttackMotion::TrailWeight of the fighter's current motion (0 without one); None = the sim does not model it
    fn trail_weight(&self, _id: u32) -> Option<f32> {
        None
    }
    /// the fighter's weapon is in its alternate mode (the Second* trace / trail data apply)
    fn alternate_mode(&self, _id: u32) -> bool {
        false
    }
    fn weapon_world(&self, _id: u32) -> Option<mordhau_core::ue::FTransform> {
        None
    }
    /// combat events since the last call (mordhau-core emit_event objects; "pos_ue" / "attacker_weapon" added by the
    /// backend when it knows them)
    fn drain_events(&mut self) -> Vec<serde_json::Value> {
        Vec::new()
    }
    /// backend-specific detail for dump_state (events, motion states)
    fn debug(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TracerSegment {
    pub fighter: u32,
    pub tick: u64,
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub environment_only: bool,
}

/// Stand-in: fighters idle in place. No rules are invented here: nothing moves, nothing takes damage.
#[derive(Default)]
pub struct StubSim {
    ticks: u64,
    fighters: Vec<FighterView>,
}

impl SimBackend for StubSim {
    fn name(&self) -> &'static str {
        "stub"
    }
    fn spawn(&mut self, loc: [f32; 3], yaw: f32, team: Option<i64>) -> u32 {
        let id = self.fighters.len() as u32;
        // health 100 / half height 96: UNCONFIRMED placeholders of the stub (mh-sim gives the real values)
        self.fighters.push(FighterView { id, team, loc, yaw, half_height: 96.0, state: "Idle".into(), health: 100.0, ..Default::default() });
        id
    }
    fn tick(&mut self, _dt: f32, _inputs: &HashMap<u32, FrameInput>) {
        self.ticks += 1;
    }
    fn ticks(&self) -> u64 {
        self.ticks
    }
    fn fighters(&self) -> Vec<FighterView> {
        self.fighters.clone()
    }
}

/// Non-send resource (see SimBackend).
pub struct Sim(pub Box<dyn SimBackend>);

/// Spawn requests from the script / default flow: n fighters at the level's starts (player-driven), then n bots.
#[derive(Resource, Default)]
pub struct SpawnQueue(pub usize);

#[derive(Resource, Default)]
pub struct BotQueue(pub usize);

/// Inputs for the next fixed step, by fighter id (player input, scripts, bots).
#[derive(Resource, Default)]
pub struct Inputs(pub HashMap<u32, FrameInput>);

/// The camera sweep's filter (ComputeCameraPOV rva 0x14b5520: UWorld::SweepSingleByObjectType, ObjectTypesToQuery =
/// ECC_WorldStatic): bodies that answer queries with object type WorldStatic. An object-type query blocks on every
/// matching object, so the info volumes mh-level gives AVolume's OverlapAll (LightmassImportanceVolume,
/// PostProcessVolume, NavMeshBoundsVolume: WorldStatic, QueryOnly) would hold the camera at the sweep start everywhere
/// inside them (seen: state/runtime_evidence/2026-10-05/115817-r5_camera, 548 hits on LightmassImportanceVolume_1).
/// The exe gives those classes NoCollision (ctors APostProcessVolume 0x3358a00, ALightmassImportanceVolume 0x31b3ff0,
/// ANavMeshBoundsVolume 0x38012b0: AVolume's OverlapAll, then SetCollisionProfileName(NoCollision, FName global rva
/// 0x58fa358); mh-world r1 moved that into mh-level class_default). Kept as a guard for other info volumes: a volume
/// only counts when it blocks Pawn or Camera (BlockingVolume InvisibleWall, CameraBlockingVolume) (UNCONFIRMED for
/// volume classes whose ctors were not checked).
pub fn camera_blocker(b: &mh_level::collision::Body) -> bool {
    use mh_level::collision::Resp;
    b.queries()
        && b.object_type.ends_with("WorldStatic")
        && (b.kind != "volume" || b.response("Pawn") == Resp::Block || b.response("Camera") == Resp::Block)
}

/// The session's FMath::Rand seed: FEngineLoop::PreInitPreStartupScreen calls srand(FPlatformTime::Cycles()) (the call
/// at rva 0x7ce0dd), or srand(0) with -FIXEDSEED (mh-world spawner.rs header; state/rand_callers.txt). On Windows
/// FPlatformTime::Cycles() is the low 32 bits of QueryPerformanceCounter. `--seed N` = a fixed seed (evidence runs,
/// the -FIXEDSEED analogue with N = 0).
#[derive(Resource, Clone, Copy, Debug, serde::Serialize)]
pub struct SessionSeed(pub u32);

pub fn session_seed(fixed: Option<u32>) -> u32 {
    if let Some(s) = fixed {
        return s;
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn QueryPerformanceCounter(lp: *mut i64) -> i32;
        }
        let mut c: i64 = 0;
        // SAFETY: QueryPerformanceCounter writes one i64 through the pointer
        if unsafe { QueryPerformanceCounter(&mut c) } != 0 {
            return c as u32;
        }
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0)
}

/// The map the sim is attached to, and the error if attaching failed (evidence).
#[derive(Resource, Default)]
pub struct SimLevel {
    /// per fighter id: the DefaultProfiles index it was spawned with (look + weapon)
    pub profiles: HashMap<u32, usize>,
    /// bots spawned so far (`--bot-profile all` cycles the profiles by it)
    pub bots_spawned: usize,
    pub map: String,
    pub error: Option<String>,
    pub secs: f32,
    /// pawns no controller owns any more: the player's old character after a respawn as a new one
    /// (armory_host::restart_player); its corpse stays, nothing respawns it
    pub retired: std::collections::HashSet<u32>,
}

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnQueue>()
            .init_resource::<BotQueue>()
            .init_resource::<Inputs>()
            .init_resource::<SimLevel>()
            .init_resource::<SimFrames>()
            .init_resource::<FrameMotion>()
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .init_resource::<StepMode>()
            .add_systems(FixedUpdate, tick_sim.run_if(|m: Res<StepMode>| !m.per_frame))
            .add_systems(
                Update,
                tick_sim_frame
                    .run_if(|m: Res<StepMode>| m.per_frame)
                    .after(crate::input::player_input)
                    .before(crate::fighter::spawn_from_queue)
                    .before(crate::camera::player_rig)
                    .before(crate::bridge::forward_events),
            )
            .add_systems(PostUpdate, record_motion.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// How the sim steps. `per_frame` (default with mh-sim, `--step frame`): once per rendered frame with the frame's
/// DeltaTime, as the exe's owning client ticks its character: UWorld::Tick runs the tick groups once per frame, the
/// CharacterMovementComponent substeps by MaxSimulationTimeStep 0.05 / MaxSimulationIterations 8 (CMC ctor 0x142f6d094),
/// AMordhauCharacter::LODTick rva 0x154c390 -> UMotionSystemComponent::OnLODTick rva 0x14cb3e0 tick the motions with the
/// same DeltaTime (mh-sim Sim::step_dt). The frame time is clamped as AWorldSettings::FixupDeltaSeconds does
/// (MinUndilatedFrameTime 0.0005 / MaxUndilatedFrameTime 0.4: stock UE AWorldSettings defaults, the engine module is
/// not in the decomp corpus: UNCONFIRMED). `--step fixed`: TICK_HZ fixed steps + render interpolation (SimFrames).
#[derive(Resource, Clone, Copy, Debug, serde::Serialize)]
pub struct StepMode {
    pub per_frame: bool,
}

impl Default for StepMode {
    fn default() -> Self {
        StepMode { per_frame: false }
    }
}

pub const MIN_FRAME_TIME: f32 = 0.0005;
pub const MAX_FRAME_TIME: f32 = 0.4;

pub fn tick_sim_frame(mut sim: NonSendMut<Sim>, time: Res<Time>, mut inputs: ResMut<Inputs>, mut frames: ResMut<SimFrames>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    sim.0.tick(dt.clamp(MIN_FRAME_TIME, MAX_FRAME_TIME), &inputs.0);
    for i in inputs.0.values_mut() {
        i.consumed();
    }
    // nothing to interpolate: the drawn state is this frame's
    frames.cur = snapshot(&sim);
    frames.prev = frames.cur.clone();
    frames.steps += 1;
}

fn tick_sim(mut sim: NonSendMut<Sim>, time: Res<Time<Fixed>>, mut inputs: ResMut<Inputs>, mut frames: ResMut<SimFrames>) {
    sim.0.tick(time.delta_secs(), &inputs.0);
    for i in inputs.0.values_mut() {
        i.consumed();
    }
    frames.prev = std::mem::replace(&mut frames.cur, snapshot(&sim));
    frames.steps += 1;
}

/// One fixed step's render state: fighter views, component-space poses, held-weapon world transforms (UE).
#[derive(Clone, Default)]
pub struct Snap {
    pub views: HashMap<u32, FighterView>,
    pub poses: HashMap<u32, Vec<mordhau_core::ue::FTransform>>,
    pub weapons: HashMap<u32, mordhau_core::ue::FTransform>,
}

/// Render interpolation (smooth motion at any frame rate). The sim steps at the fixed TICK_HZ (its combat schedule
/// and the golden traces are per tick), while the engine's local client ticks UCharacterMovementComponent once per
/// rendered frame with the frame's DeltaTime, substepped by MaxSimulationTimeStep 0.05 / MaxSimulationIterations 8
/// (CMC ctor 0x142f6d094 / +0x2a0; mh-character exe_cmc.rs GetSimulationTimeStep). Drawing the last fixed step as is
/// shows 60 Hz steps on a 144 Hz display (the stutter); here every frame draws prev + (cur - prev) x
/// Time<Fixed>::overstep_fraction (one step of latency; a per-frame client movement tick is the exe's way and
/// needs a variable-dt movement step in mh-sim: UNCONFIRMED stand-in, reported to rust-combat).
#[derive(Resource)]
pub struct SimFrames {
    pub prev: Snap,
    pub cur: Snap,
    pub steps: u64,
    /// `debug interp_off` draws the latest step only (before/after evidence)
    pub enabled: bool,
}

impl Default for SimFrames {
    fn default() -> Self {
        SimFrames { prev: Snap::default(), cur: Snap::default(), steps: 0, enabled: true }
    }
}

fn lerp_f(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// shortest-way angle lerp (degrees)
pub fn lerp_yaw(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    a + d * t
}

pub fn lerp_xf(a: &mordhau_core::ue::FTransform, b: &mordhau_core::ue::FTransform, t: f32) -> mordhau_core::ue::FTransform {
    let mut o = *b;
    o.loc.x = lerp_f(a.loc.x, b.loc.x, t);
    o.loc.y = lerp_f(a.loc.y, b.loc.y, t);
    o.loc.z = lerp_f(a.loc.z, b.loc.z, t);
    let qa = Quat::from_xyzw(a.rot.x, a.rot.y, a.rot.z, a.rot.w);
    let qb = Quat::from_xyzw(b.rot.x, b.rot.y, b.rot.z, b.rot.w);
    let q = qa.slerp(qb, t);
    o.rot.x = q.x;
    o.rot.y = q.y;
    o.rot.z = q.z;
    o.rot.w = q.w;
    o
}

impl SimFrames {
    fn alpha(&self, time: &Time<Fixed>) -> f32 {
        if self.enabled { time.overstep_fraction().clamp(0.0, 1.0) } else { 1.0 }
    }
    /// the drawn view of fighter id (None before its first step)
    pub fn view(&self, id: u32, time: &Time<Fixed>) -> Option<FighterView> {
        let b = self.cur.views.get(&id)?;
        let Some(a) = self.prev.views.get(&id) else { return Some(b.clone()) };
        let t = self.alpha(time);
        let mut v = b.clone();
        for i in 0..3 {
            v.loc[i] = lerp_f(a.loc[i], b.loc[i], t);
        }
        v.yaw = lerp_yaw(a.yaw, b.yaw, t);
        Some(v)
    }
    pub fn pose(&self, id: u32, time: &Time<Fixed>) -> Option<Vec<mordhau_core::ue::FTransform>> {
        let b = self.cur.poses.get(&id)?;
        let t = self.alpha(time);
        match self.prev.poses.get(&id) {
            Some(a) if a.len() == b.len() => Some(a.iter().zip(b).map(|(x, y)| lerp_xf(x, y, t)).collect()),
            _ => Some(b.clone()),
        }
    }
    pub fn weapon(&self, id: u32, time: &Time<Fixed>) -> Option<mordhau_core::ue::FTransform> {
        let b = self.cur.weapons.get(&id)?;
        let t = self.alpha(time);
        Some(match self.prev.weapons.get(&id) {
            Some(a) => lerp_xf(a, b, t),
            None => *b,
        })
    }
}

fn snapshot(sim: &Sim) -> Snap {
    let mut s = Snap::default();
    for v in sim.0.fighters() {
        if let Some(p) = sim.0.pose_now(v.id) {
            s.poses.insert(v.id, p);
        }
        if let Some(w) = sim.0.weapon_world(v.id) {
            s.weapons.insert(v.id, w);
        }
        s.views.insert(v.id, v);
    }
    s
}

/// per-frame motion of the view and of the player fighter (stutter evidence): frame dt, camera / fighter position
/// deltas (m) over the last frames
#[derive(Resource, Default, Clone, serde::Serialize)]
pub struct FrameMotion {
    pub frames: Vec<[f32; 3]>,
    #[serde(skip)]
    pub last_cam: Option<Vec3>,
    #[serde(skip)]
    pub last_fighter: Option<Vec3>,
}

impl FrameMotion {
    /// speed (m/s) statistics of the drawn fighter / camera over the recorded frames: mean, coefficient of
    /// variation, and the share of frames where the fighter did not move although it moves on average
    pub fn stats(&self) -> serde_json::Value {
        let st = |k: usize| {
            let v: Vec<f32> = self.frames.iter().filter(|f| f[0] > 0.0).map(|f| f[k] / f[0]).collect();
            if v.is_empty() {
                return serde_json::json!(null);
            }
            let m = v.iter().sum::<f32>() / v.len() as f32;
            let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt();
            let still = v.iter().filter(|x| **x < 0.05 * m).count() as f32 / v.len() as f32;
            serde_json::json!({"mean_m_s": m, "cv": if m > 0.0 { sd / m } else { 0.0 }, "still_frames": still, "n": v.len()})
        };
        let dts: Vec<f32> = self.frames.iter().map(|f| f[0]).collect();
        serde_json::json!({"fighter_speed": st(1), "camera_speed": st(2), "dt_min": dts.iter().cloned().fold(f32::MAX, f32::min), "dt_max": dts.iter().cloned().fold(0.0, f32::max)})
    }
}

fn record_motion(
    time: Res<Time>,
    mut fm: ResMut<FrameMotion>,
    pc: Option<Res<crate::input::PlayerControl>>,
    fighters: Query<(&crate::fighter::Fighter, &GlobalTransform)>,
    cam: Query<&GlobalTransform, With<crate::camera::FlyCam>>,
) {
    let me = pc.map(|p| p.id).unwrap_or(0);
    let f = fighters.iter().find(|(f, _)| f.id == me).map(|(_, g)| g.translation());
    let c = cam.iter().next().map(|g| g.translation());
    if let (Some(f), Some(c)) = (f, c) {
        let df = fm.last_fighter.map(|l| (f - l).length()).unwrap_or(0.0);
        let dc = fm.last_cam.map(|l| (c - l).length()).unwrap_or(0.0);
        fm.frames.push([time.delta_secs(), df, dc]);
        let n = fm.frames.len();
        if n > 600 {
            fm.frames.drain(..n - 600);
        }
    }
    fm.last_fighter = f;
    fm.last_cam = c;
}

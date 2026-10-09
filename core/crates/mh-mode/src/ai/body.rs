//! What the bot code reads from a character (its own pawn or an enemy) and what it asks of the world
//! (godot/game/ai/bot_body.gd BotBody + the MotionSystem fields the tasks read), in UE units: centimetres, Z up, yaw
//! in degrees (DESIGN.md §4). The host fills these every frame from its fighters (the combat core's motion state) and
//! implements `BotHost` for the engine queries (navmesh, traces, stimuli) and the fighter input entry points.
//!
//! Field sources (AMordhauCharacter layout extract/native/types/AMordhauCharacter.h + engine parents), as bot_body.gd:
//!   location / yaw     RootComponent (+0x130) ComponentToWorld translation (+0x1d0) / GetForwardVector
//!   velocity           APawn::GetVelocity (vtable +0x2f8)
//!   capsule            CapsuleRadius +0x45c = 50, CapsuleHalfHeight +0x458 = 96 (AMordhauCharacter ctor rva=0x1524d90);
//!                      scaled radius = CapsuleRadius * min(ComponentToWorld scale X, Y)
//!   max_walk_speed     CharacterMovement (+0x288) +0x18c MaxWalkSpeed (UCharacterMovementComponent::GetMaxSpeed
//!                      rva=0x2f79a50); 308 = BP_MordhauCharacter CharMoveComp.MaxWalkSpeed
//!   mesh_scale_x       Mesh (+0x280) RelativeScale3D.X (+0x134), USceneComponent default 1.0 (UNCONFIRMED)
//!   weapon_length      AMordhauWeapon::Length (+0x1bec) = |tracer span| / 15 (RecalculateTracerPoints rva=0x163a940)
//!   trace_*            AMordhauWeapon CurrentTraceStart +0xd48, CurrentTraceEnd +0xd54, PreviousTraceEnd +0xd6c
//!   ignore_cache       AMordhauWeapon ActorIgnoreCache +0xe88: bodies this swing already hit
//!   facing_bone_height GetSocketLocation(<FName at 0x145720590>).Z - Mesh Z (bone UNCONFIRMED; the host supplies it)
//! MotionView (the pawn's MotionSystemComponent +0x688 Motion): the UAttackMotion / UParryMotion / UFeintedMotion fields
//! the tasks read (offsets on each field).

use mordhau_core::ue::FVector;
use serde::{Deserialize, Serialize};

pub type BodyId = usize;

// ---- combat enums (godot/game/combat/combat_enums.gd: PDB LF_ENUM records / UHT name tables) ---------------------
pub mod enums {
    /// EAttackMove
    pub const RIGHT_STRIKE: i64 = 0;
    pub const LEFT_STRIKE: i64 = 1;
    pub const STAB: i64 = 2;
    pub const ALT_STAB: i64 = 3;
    pub const KICK: i64 = 4;
    /// EAttackType
    pub const ATTACK_MORPH: i64 = 4;
    /// EAttackStage
    pub const WINDUP: i64 = 0;
    pub const RELEASE: i64 = 1;
    pub const RECOVERY: i64 = 2;
    /// EBlockType
    pub const BLOCK_REGULAR: i64 = 0;
    pub const BLOCK_ALT_REGULAR: i64 = 1;
    pub const MOVE_NAMES: [&str; 8] = ["RightStrike", "LeftStrike", "Stab", "AltStab", "Kick", "Bash", "Couch", "Ranged"];
    pub const STAGE_NAMES: [&str; 3] = ["Windup", "Release", "Recovery"];
    /// UAttackMotion::IsStrike: move < 2
    pub fn is_strike(m: i64) -> bool {
        m < 2
    }
    /// UAttackMotion::IsLeft rva=0x162c140: ((move - 1) & 0xfd) == 0, i.e. LeftStrike or AltStab
    pub fn is_left(m: i64) -> bool {
        m == 1 || m == 3
    }
    /// UAttackMotion::FlipSide rva=0x161dc80
    pub fn flip_side(m: i64) -> i64 {
        match m {
            0 => 1,
            1 => 0,
            2 => 3,
            3 => 2,
            _ => m,
        }
    }
    pub fn move_name(m: i64) -> &'static str {
        MOVE_NAMES.get(m as usize).copied().unwrap_or("?")
    }
    pub fn stage_name(s: i64) -> &'static str {
        STAGE_NAMES.get(s as usize).copied().unwrap_or("?")
    }
}

/// the pawn's current motion as the bot tasks read it
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionView {
    /// Motion.kind() of the combat port ("Idle", "Attack", "Parry", "Feinted", "Flinch...", "Blocked...", "Equip...",
    /// "Climbing"); "" = no motion
    pub kind: String,
    /// identity of the motion object (UMordhauMotion instance): two reads of the same motion compare equal
    pub id: i64,
    /// identity of the replicated NetMotion (MotionSystem +0xc4 Id, incremented on every AssignNetMotion); -1 = none
    pub net_id: i64,
    /// a UAttackMotion (or URangedDrawMotion, not ported)
    pub is_attack: bool,
    /// a UParryMotion
    pub is_parry: bool,
    /// a UFeintedMotion (StaticClass 0x14168e7c0)
    pub is_feinted: bool,
    /// UMordhauMotion StartTime (+0x4c)
    pub start_time: f64,
    /// UAttackMotion Stage (+0x10e9)
    pub stage: i64,
    /// ReleaseEnd (+0x1094)
    pub release_end: f64,
    /// WindupEnd
    pub windup_end: f64,
    /// bHasChambered (+0x10f4)
    pub has_chambered: bool,
    /// bHasHit (+0x10ea)
    pub has_hit: bool,
    /// Type (+0x108d) EAttackType
    pub attack_type: i64,
    /// Move EAttackMove
    pub mv: i64,
    /// AngleTarget
    pub angle_target: f64,
    /// GetEarlyReleaseDuration()
    pub early_release_duration: f64,
    /// UParryMotion riposte window: RiposteWindowStart, ParryData RiposteWindowBase,
    /// NonHeldParryExtensionAndRiposteWindowExtra
    pub riposte_window_start: f64,
    pub riposte_window_base: f64,
    pub riposte_window_extra: f64,
}

/// the fighter state the tasks read besides the motion: the motion, Stamina / Health replicated bytes
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PawnView {
    pub motion: MotionView,
    /// AMordhauCharacter Stamina byte (the replicated 0..100 value)
    pub stamina_byte: i64,
    /// AMordhauCharacter Health byte
    pub health_byte: i64,
}

/// the held weapon's data the tasks read (WeaponData of the combat port)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeaponView {
    pub id: String,
    pub chain: Vec<String>,
    pub native_class: String,
    /// AMordhauEquipment +0xcb9 bCanAttack
    pub b_can_attack: bool,
    /// StabAttack.Damage[0] (weapon +0xfa0)
    pub stab_damage0: f64,
    /// StrikeAttack.Damage[0] (weapon +0x1440)
    pub strike_damage0: f64,
}

/// an inventory slot for UBTTask_SwitchEquipment (AMordhauCharacter +0x11e8)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InventoryItem {
    pub weapon: Option<WeaponView>,
    pub is_fists: bool,
    /// the ranged branch's qualification (header of the task: UNCONFIRMED)
    pub ranged: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BotBody {
    pub name: String,
    /// the character has a MotionSystemComponent (+0x688)
    pub has_sys: bool,
    /// AAdvancedCharacter +0x660 Team
    pub team: i64,
    /// +0x504 bIsDead
    pub is_dead: bool,
    /// destroyed / no longer in the world (the reference's `bodies.has(b)` false)
    pub gone: bool,
    pub location: FVector,
    pub yaw: f64,
    pub velocity: FVector,
    pub capsule_radius: f64,
    pub capsule_half_height: f64,
    pub capsule_scale_min: f64,
    pub max_walk_speed: f64,
    pub mesh_scale_x: f64,
    pub weapon_length: f64,
    pub trace_start: FVector,
    pub trace_end: FVector,
    pub prev_trace_end: FVector,
    pub ignore_cache: Vec<BodyId>,
    pub facing_bone_height: f64,
    /// outputs the bot writes (the host applies them): AMordhauCharacter +0xde4 bWantsCrouch; StartSprinting
    /// rva=0x156dcc0 sets CharacterMovement +0xd19
    pub wants_crouch: bool,
    pub wants_sprint: bool,
    pub inventory: Vec<InventoryItem>,
    pub right_hand: usize,
    /// the inventory slot in the left hand (AMordhauCharacter +0x11f8 LeftHandEquipment), None = empty
    pub left_hand: Option<usize>,
    /// BP_HordeEnemy RageTarget (Enrage@4795 sets it for a Duration; the host's skill effects), read by
    /// GetEnragedTarget: valid and alive -> the target
    pub rage_target: Option<BodyId>,
    /// BP_HordeEnemy CurrentTask's GetLocationTarget (a map's HordeTasks entry chosen at ReceivePossessed@261-@827;
    /// the host resolves it), None = no CurrentTask
    pub horde_task_target: Option<FVector>,
    /// APawn::GetActorEyesViewPoint height above the root (BaseEyeHeight; UNCONFIRMED until the host sets it)
    pub eye_height: f64,
    /// APawn Controller (+0x258) when it is an AMordhauAIController: the bot's index in the bot world
    pub ai: Option<usize>,
    /// the fighter state (MotionSystem)
    pub pawn: PawnView,
    /// RightHandEquipment (+0x11f8) as AMordhauWeapon (sys.weapon)
    pub weapon: Option<WeaponView>,
}

impl BotBody {
    pub fn new(name: &str) -> BotBody {
        BotBody {
            name: name.to_string(),
            has_sys: true,
            team: 0,
            is_dead: false,
            gone: false,
            location: FVector::ZERO,
            yaw: 0.0,
            velocity: FVector::ZERO,
            capsule_radius: 50.0,
            capsule_half_height: 96.0,
            capsule_scale_min: 1.0,
            max_walk_speed: 308.0,
            mesh_scale_x: 1.0,
            weapon_length: 0.0,
            trace_start: FVector::ZERO,
            trace_end: FVector::ZERO,
            prev_trace_end: FVector::ZERO,
            ignore_cache: Vec::new(),
            facing_bone_height: 0.0,
            wants_crouch: false,
            wants_sprint: false,
            inventory: Vec::new(),
            right_hand: 0,
            left_hand: None,
            rage_target: None,
            horde_task_target: None,
            eye_height: 0.0,
            ai: None,
            pawn: PawnView::default(),
            weapon: None,
        }
    }

    /// USceneComponent::GetForwardVector of an upright root: (cos yaw, sin yaw, 0)
    pub fn forward(&self) -> FVector {
        let r = self.yaw.to_radians();
        FVector::new(r.cos() as f32, r.sin() as f32, 0.0)
    }

    /// UCapsuleComponent::GetScaledCapsuleRadius as inlined in the tasks: min(scale X, scale Y) * CapsuleRadius
    pub fn scaled_radius(&self) -> f64 {
        self.capsule_scale_min * self.capsule_radius
    }

    /// the speed term the tasks use: max(|GetVelocity().XY|, CharacterMovement +0x18c MaxWalkSpeed)
    pub fn threat_speed(&self) -> f64 {
        let v = self.velocity;
        let s = (v.x * v.x + v.y * v.y).sqrt() as f64;
        if self.max_walk_speed > s {
            self.max_walk_speed
        } else {
            s
        }
    }

    pub fn motion(&self) -> &MotionView {
        &self.pawn.motion
    }
    pub fn stamina_byte(&self) -> i64 {
        self.pawn.stamina_byte
    }
    pub fn health_byte(&self) -> i64 {
        self.pawn.health_byte
    }
}

/// a sensed stimulus (FAIStimulus) the host's perception system reports for a character
#[derive(Clone, Debug, PartialEq)]
pub struct Stimulus {
    pub sense: Sense,
    pub age: f64,
    pub strength: f64,
    pub receiver: FVector,
    pub location: FVector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sense {
    Sight,
    Hearing,
    Damage,
}

/// The world the bots live in: engine queries (the reference's adapter callables, BotWorld in Godot today; mh-runtime
/// later) and the fighters' input entry points. Defaults = the reference's behaviour when the callable is not set.
pub trait BotHost {
    /// AMordhauCharacter::RequestAttack rva=0x15644c0 (EAttackMove, float angle): the fighter's input entry point.
    /// Returns the pawn's state right after (the tasks read it in the same evaluation).
    fn request_attack(&mut self, body: BodyId, mv: i64, angle: f64) -> PawnView;
    /// AMordhauCharacter::RequestParry rva=0x1564860 (EBlockType, bool: always 1 from the bot tasks)
    fn request_parry(&mut self, body: BodyId, bt: i64) -> PawnView;
    /// AMordhauCharacter::RequestFeint rva=0x15646b0
    fn request_feint(&mut self, body: BodyId) -> PawnView;
    /// UNavigationSystemV1::NavigationRaycast: blocked between a and b (default: no navmesh, never blocked)
    fn nav_raycast(&mut self, _a: FVector, _b: FVector) -> bool {
        false
    }
    /// a UNavigationSystemV1 exists (GetRandomReachablePointInRadius callable set)
    fn has_nav(&self) -> bool {
        false
    }
    /// UNavigationSystemV1::GetRandomReachablePointInRadius: None when no point is found
    fn random_reachable_point(&mut self, _origin: FVector, _radius: f64) -> Option<FVector> {
        None
    }
    /// the perception system's last sensed stimuli of `target` for `receiver`; None = the reference's stand-in (a
    /// fresh sight stimulus from every other live character: UNCONFIRMED)
    fn stimuli(&mut self, _receiver: BodyId, _target: BodyId) -> Option<Vec<Stimulus>> {
        None
    }
    /// UWorld::LineTraceTestByChannel(ECC 3) for hearing beyond 400 cm (None = not set)
    fn hearing_trace_blocked(&mut self, _a: FVector, _b: FVector) -> Option<bool> {
        None
    }
    /// UWorld::LineTraceSingleByChannel(0x12) ignoring `ignore`: the body hit first (outer None = not set)
    /// UNavigationSystemV1::FindPathToLocationSynchronously(a, b): Some(true) = a valid, complete path; Some(false) =
    /// no path or a partial one; None = not answered (no navigation: the caller's stand-in)
    fn path_complete(&mut self, _a: FVector, _b: FVector) -> Option<bool> {
        None
    }
    /// the sight sense's line of sight (UAISense_Sight: a Visibility trace from the receiver's eyes to the target):
    /// Some(true) = blocked by the static world; None = not answered (taken as clear)
    fn sight_blocked(&mut self, _a: FVector, _b: FVector) -> Option<bool> {
        None
    }
    fn trace_first_hit(&mut self, _from: FVector, _to: FVector, _ignore: BodyId) -> Option<Option<BodyId>> {
        None
    }
}

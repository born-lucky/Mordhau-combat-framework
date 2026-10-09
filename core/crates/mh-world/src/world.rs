//! The world of gameplay actors: built from mh-level's level data, driven by the host, emitting events.

use crate::capture::CapturePoint;
use crate::destructible::{Destructible, Overlaps};
use crate::door::Door;
use crate::frontline::{KillObjective, Objective, ObjectiveKind};
use crate::hazards::{Cauldron, Trigger, TriggerKind};
use crate::pickups::Pickup;
use crate::ladder::Ladder;
use crate::progress::{Driver, ProgressActor};
use crate::props::Props;
use crate::pushable::Pushable;
use crate::spawner::{CrtRand, EquipmentSpawner, VehicleSpawner};
use crate::V;
use mh_level::xf::Xf;
use std::collections::BTreeMap;

/// Index into `LevelData::gameplay` (and `World::actors`)
pub type ActorId = usize;
/// The host's character id
pub type CharId = u32;

/// What the Blueprints / natives read of a character
#[derive(Clone, Debug, Default)]
pub struct CharView {
    pub id: CharId,
    /// actor (capsule centre) location
    pub location: V,
    /// actor forward (unit, world)
    pub forward: V,
    /// team (EMordhauTeam byte; AMordhauPlayerState::Team = AAdvancedCharacter::Team +0x660 for a possessed pawn);
    /// None = no team
    pub team: Option<u8>,
    /// AAdvancedCharacter::bIsDead (+0x504)
    pub dead: bool,
    pub ragdoll_falling: bool,
    pub in_knockback: bool,
    /// in a vehicle (horse, ladder, siege engine): CurrentVehicle valid
    pub in_vehicle: bool,
    /// ACharacter::bAllowVehicles (BP_Ladder:CanInteract@88)
    pub allow_vehicles: bool,
    /// the current motion is an attack / blocked / feinted motion (BP_Door:ExecuteUbergraph@238..@1003)
    pub attacking_or_blocked_or_feinted: bool,
    /// controlled by a MordhauPlayerController (scores go to player controllers only)
    pub has_player_controller: bool,
    /// UAdvancedCharacterMovement PreviousVelocity (the impalement volumes read it)
    pub velocity: V,
    /// the placed actor this character is (a BP_FrontlineKillObjective / placed character), if any
    pub actor: Option<ActorId>,
    /// an AHorse (BP_Horse) rather than a MordhauCharacter
    pub is_horse: bool,
    /// a BP_DemonHordeCharacter (Demon Invasion enemies and allies)
    pub is_demon_horde_character: bool,
    /// a BP_DemonHordeEnemy
    pub is_demon_horde_enemy: bool,
    /// a BP_FrontlineKillObjective (the Frontline nobles)
    pub is_kill_objective: bool,
    /// the right hand holds a BP_DeliverableEquipment
    pub holding_deliverable: bool,
    /// RightHandEquipment's class package path ("pkg.N" as the actor properties hold class refs), empty for none
    pub right_hand_class: String,
}

/// One damage application (AActor::TakeDamage from UAttackMotion::ProcessHitForDamage rva 0x1638a60's
/// FMordhauDamageEvent, or UGameplayStatics::ApplyDamage from another Blueprint)
#[derive(Clone, Debug, Default)]
pub struct DamageInfo {
    /// the damage causer when it is a character (melee: the attacker)
    pub causer: Option<CharId>,
    /// the instigating controller's character (InstigatedBy's pawn), the receiver of scores
    pub instigator: Option<CharId>,
    /// InstigatedBy is a MordhauPlayerController (BP_DestroyableActor:SetHealth@144, BP_FrontlineDestroyable@1953)
    pub instigator_player: bool,
    /// InstigatedBy's MordhauPlayerState Team (players and bots); None = no controller / player state
    pub instigator_team: Option<u8>,
    /// FMordhauDamageEvent Move (EAttackMove); 4 = kick (BP_Door:ExecuteUbergraph@6626)
    pub attack_move: Option<u8>,
    /// the causing character's forward (the kick's door-to-actor angle, BP_Door:HandleFastOpen@168)
    pub causer_forward: Option<V>,
    /// the causer is BP_CircleSawProgressActor (BP_FrontlineDestroyable:ExecuteUbergraph@2407)
    pub circle_saw: bool,
}

/// What the host must do
#[derive(Clone, Debug, PartialEq)]
pub enum WorldEvent {
    /// AMordhauCharacter::Knockback(impulse)
    Knockback { char: CharId, impulse: V },
    /// AMordhauCharacter::SetIsRagdollFalling(true)
    Ragdoll { char: CharId },
    /// AMordhauCharacter::Trip()
    Trip { char: CharId },
    /// MordhauTakeDamage on a character (BP_Ladder fall: DropDamage)
    DamageCharacter { char: CharId, amount: f64 },
    /// K2_TeleportTo(own location): push a character out of geometry it overlaps (UE resolves the encroachment)
    Depenetrate { char: CharId },
    /// K2_AddActorWorldOffset on a character (carried by a spline pushable, BP_SplinePushableActor@644)
    MoveCharacter { char: CharId, offset: V },
    /// UGameplayStatics::ApplyDamage on another world actor (host feeds it back through `apply_damage`)
    DamageActor { actor: ActorId, amount: f64 },
    /// a component's relative rotation (door leaf yaw, degrees; MakeRotator(0, 0, Yaw))
    ComponentYaw { actor: ActorId, component: &'static str, yaw_deg: f64 },
    /// a component's relative transform (ladder mesh while raising / dropping)
    ComponentTransform { actor: ActorId, component: &'static str, rel: Xf },
    /// K2_SetActorTransform (a spline pushable on its spline)
    ActorTransform { actor: ActorId, xf: Xf },
    /// SetActorEnableCollision
    CollisionEnabled { actor: ActorId, on: bool },
    /// StaticMesh.SetStaticMesh (damage-state mesh)
    MeshSwapped { actor: ActorId, mesh: String },
    /// hidden in game (destroyed with DeleteWhenDestroyed)
    Hidden { actor: ActorId },
    /// SetLifeSpan expiry: the actor is gone
    Destroyed { actor: ActorId },
    /// OnDeath broadcast (destructibles)
    Died { actor: ActorId },
    /// GiveClientScoreBP(kind, amount) on the player controller of `char` (EScoreFeedReason byte)
    Score { char: Option<CharId>, team: Option<u8>, kind: u8, amount: i64 },
    /// CapturePoint.ObjectivesChanged() was called (the world runs it before returning; informational)
    ObjectivesChanged { capture_point: ActorId },
    /// BP_CapturePoint: the objectives reached 1 (bObjectivesCompleted, OnObjectivesCompleted)
    ObjectivesCompleted { capture_point: ActorId },
    /// AControlPoint::SetCaptureProgress(progress, team, false) on mh-mode's control point (BP_CapturePoint)
    CaptureProgress { capture_point: ActorId, progress: f64, team: Option<u8> },
    /// SetTeamScore(team, score) on the game mode (BP_CapturePoint TriggerWinDelayed)
    TeamScore { team: u8, score: f64 },
    /// BP_FrontlineGameState.PushWinDelay = delay (push points)
    PushWinDelay { delay: f64 },
    /// APushableActor Progress changed (OnProgressUpdated)
    PushableProgress { actor: ActorId, progress: f64 },
    /// spawn an actor of a class at a transform (vehicle spawners)
    Spawn { by: ActorId, class: String, xf: Xf },
    /// BP_EquipmentSpawner:SpawnEquipment: Equipment class at xf, then AssignCustomization(Customization, Emblem,
    /// EmblemColor1, EmblemColor2) (the spawner's values in `customization`)
    SpawnEquipment { by: ActorId, class: String, xf: Xf, customization: serde_json::Value },
    /// K2_TeleportTo(xf) of the spawner's vehicle (horse onto its spawner); the host reports the result with
    /// `World::spawn_teleport_result`
    TeleportSpawned { by: ActorId, xf: Xf },
    /// K2_DestroyActor of the spawner's vehicle (a horse that could not be teleported)
    DestroySpawned { by: ActorId },
    /// BP_DestroyableActor:DetachAttachedProjectiles: destroy the MordhauProjectiles attached to the actor, detach
    /// (K2_DetachFromActor(KeepWorld x3)) its attached MordhauEquipment
    DetachAttached { actor: ActorId },
    /// BP_DestroyableActor's unstuck pass (OnRep_ReplicatedHealth@3606..@4921) for one overlapping character: capsule
    /// trace (object type 0, radius x0.8, half height x0.9) from its location + 500 Z down to it; if it hits `actor`,
    /// K2_TeleportTo(actor StaticMesh's closest collision point to (location + 100 Z) + unscaled half height)
    Unstuck { char: CharId, actor: ActorId },
    /// AAdvancedCharacter::MordhauTakeDamage rva 0x148a3f0 (Amount, empty FHitResult, DamageType, DamageSubType,
    /// Source, Agent = this world actor, EventInstigator null): the combat side's damage path
    TakeDamage { char: CharId, amount: f64, damage_type: u8, sub_type: u8, source: Option<CharId>, agent: Option<ActorId> },
    /// UGameplayStatics::ApplyDamage(character, amount, null, null, DamageType) (AActor::TakeDamage on a character)
    ApplyDamageChar { char: CharId, amount: f64 },
    /// the vehicle's driver: GetDriver().MordhauTakeDamage(amount, hit, damage_type, 0, null, null, null)
    DamageDriver { vehicle: CharId, amount: f64, damage_type: u8 },
    /// AAdvancedCharacter bForceRagdollIfDmgAgent = true, RagdollForceMultIfDmgAgent = force_mult
    ForceRagdollIfDmgAgent { char: CharId, force_mult: f64 },
    /// AMordhauCharacter::QueueDismember(bone, false, false, (0,0,0), null)
    QueueDismember { char: CharId, bone: &'static str },
    /// IBounds EnteredOutOfBoundsArea(volume) / LeftOutOfBoundsArea (entered = false)
    OutOfBounds { char: CharId, volume: ActorId, entered: bool },
    /// AMordhauCharacter EnteredTeamArea / LeftTeamArea(team)
    TeamArea { char: CharId, team: i64, entered: bool },
    /// fall damage overrides: UAdvancedCharacterMovement FallDamageOffset / FallDamageFactor / MinVelocityForFallDamage,
    /// AMordhauCharacter ReceivedFallDamageModifier (MordhauCharacters only)
    SetFallDamage { char: CharId, received_fall_damage_modifier: Option<f64>, fall_damage_offset: f64, fall_damage_factor: f64, min_velocity: f64 },
    /// BP_DemonHordeCharacter IsImmortal
    SetImmortal { char: CharId, immortal: bool },
    /// BurnableComponent.StartBurning(a, b, c, null, null)
    StartBurning { char: CharId, a: f64, b: f64, c: f64 },
    /// BP_DemonHordeEnemy IsLedgePushing = true, LedgePushRotation = yaw
    LedgePushing { char: CharId, yaw: f64 },
    /// BP_DemonHordeEnemy LedgePusherFallDamage()
    LedgePusherFallDamage { char: CharId },
    /// Knockback(impulse) unless the preceding TakeDamage killed the character (GetIsDead check)
    KnockbackUnlessDead { char: CharId, impulse: V },
    /// BP_AmmoBox Restock: NextAmmoBoxAvailableTime check, Character.RestockEquipmentFromAmmoBox(), the controller's
    /// ReplicatedAmmoBoxCooldown + 1 on success
    RestockFromAmmoBox { char: CharId, ammo_box: ActorId },
    /// AAdvancedCharacter::OffsetHealth(amount, true) / OffsetStamina(amount, true)
    OffsetHealth { char: CharId, amount: i64 },
    OffsetStamina { char: CharId, amount: i64 },
    /// spawn `class` at `xf` with InstigatorController = the character's (BP_OilCauldron's BP_OilFire_C)
    SpawnAt { by: ActorId, class: String, xf: Xf, instigator: Option<CharId> },
    /// BP_ItemDeliverySpawn:OnInteractionStart (@15..@1053): a `class` deliverable at the spawn, UsableByTeam =
    /// `usable_by_team`, Type = `item_type`, Character.PickUp(it, -1); not picked up -> destroyed. The host keeps the
    /// spawn's SpawnedByCharacter map: the character's previous deliverable from this spawn, when no longer held
    /// (GetParentCharacter invalid), is Break()-ed and destroyed (@1113..@1367)
    GiveDeliverable { by: ActorId, char: CharId, class: String, usable_by_team: Option<u8>, item_type: i64 },
    /// BP_ItemDeliverySpawn:Disable (@61..@444): every SpawnedDeliverable destroyed
    DestroySpawnedDeliverables { by: ActorId },
    /// spawn `class` at the character and Character.PickUp(it, -1) (BP_RestockableEquipmentSpawn)
    GiveEquipment { by: ActorId, char: CharId, class: String },
    /// visible again (SetHiddenInGame(false))
    Shown { actor: ActorId },
    /// the vehicle a spawner spawned should die (BP_VehicleSpawner:Deactivate: ApplyDamage 1e9)
    KillSpawned { by: ActorId },
    /// ladder climb: spawn BP_LadderMover_C at `mover_xf` and StartDriving(char) (BP_Ladder:ExecuteUbergraph@7232);
    /// `info` builds the mover (mh_character::ExeMovement::new_ladder_mover)
    LadderMount { char: CharId, ladder: ActorId, mover_xf: Xf, start: V, end: V, exit: V, mover_class: &'static str, info: mh_character::exe_ladder::LadderInfo },
}

/// The overlap tests the Blueprints / natives make (ComponentOverlapActors on a component at its current transform).
/// The host answers them from its collision (mh-level CollisionWorld + the components' current transforms).
pub trait Queries {
    /// characters overlapping the actor's moving component (door leaf / ladder mesh / destructible mesh / pushable mesh)
    fn overlapping_chars(&self, actor: ActorId) -> Vec<CharId>;
    /// world actors (MordhauActor) overlapping it, excluding itself
    fn overlapping_actors(&self, actor: ActorId) -> Vec<ActorId>;
    /// is the spawned item / vehicle of a spawner still present, and its distance to the spawner (cm)
    fn spawned_status(&self, spawner: ActorId) -> SpawnedStatus;
    /// characters overlapping an area component (APushableActor PushArea's OverlappingComponents)
    fn area_chars(&self, actor: ActorId) -> Vec<CharId> {
        self.overlapping_chars(actor)
    }
    /// characters overlapping a named trigger component of the actor ("Box", "Sphere", "Kill", "OOB", "Flame",
    /// "Capsule"): the hazard volumes' begin / end overlaps
    fn component_chars(&self, actor: ActorId, _component: &str) -> Vec<CharId> {
        self.area_chars(actor)
    }
    /// BP_BallistaSpawner:SpawnVehicle@1064 CapsuleOverlapActors at its Capsule found something
    fn spawn_blocked(&self, _spawner: ActorId) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpawnedStatus {
    /// IsValid(instance)
    pub exists: bool,
    /// picked up (GetParentCharacter valid)
    pub held: bool,
    pub distance: f64,
    /// a vehicle spawner's vehicle is dead / gone
    pub dead: bool,
}

/// Which behaviour an actor has (from its Blueprint chain)
#[derive(Clone, Debug)]
pub enum Kind {
    Door(Door),
    Destructible(Destructible),
    Objective(Objective),
    /// a pushable that is not a Frontline objective (BP_SplinePushableActor)
    Pushable(Pushable),
    Ladder(Ladder),
    /// BP_ProgressDriver (BP_SwitchInteractable) and its variants: levers, wall cranks, wheels
    ProgressDriver(Driver),
    /// BP_SlaveProgressDriver: forwards use to its MasterCrank
    SlaveDriver { master: Option<ActorId> },
    /// BP_StaticMeshProgressActor / BP_SceneProgressActor (castle gates, portcullises, trap doors, ...)
    ProgressActor(ProgressActor),
    /// BP_FeitoriaDoorLocker: 1 s after BeginPlay locks its "Doors To Lock"
    DoorLocker { doors: Vec<ActorId>, at: Option<f64> },
    /// BP_FrontlineKillObjective (a placed character's objective layer)
    KillObjective(KillObjective),
    /// ammo boxes, rock piles, food, random weapon pickups (pickups.rs)
    Pickup(Pickup),
    /// BP_OilCauldron
    Cauldron(Cauldron),
    /// BP_ItemDeliverySpawn (torch / explosive barrel / treasure chest / rock / weapon bundle spawns): `active` =
    /// bIsInteractable (Activate@0 / Disable@5); `capture_point` set by its spot's Initialize (@370)
    DeliverySpawn { active: bool, capture_point: Option<ActorId> },
    EquipmentSpawner(EquipmentSpawner),
    VehicleSpawner(VehicleSpawner),
    /// no ported behaviour (yet): kept as data
    Inert,
}

#[derive(Clone, Debug)]
pub struct WorldActor {
    pub id: ActorId,
    pub name: String,
    pub class: String,
    /// Blueprint chain names, child first ("BP_DestroyableWoodenDoor", "BP_Door", ...)
    pub chain: Vec<String>,
    pub xf: Xf,
    pub props: Props,
    pub kind: Kind,
}

pub struct World {
    pub actors: Vec<WorldActor>,
    /// capture point actor -> its objective actors (the capture point's `Objectives` property, resolvable ones)
    pub objectives: BTreeMap<ActorId, Vec<ActorId>>,
    /// the BP_CapturePoint layer of every capture point with objectives
    pub cps: BTreeMap<ActorId, CapturePoint>,
    /// the game mode is BP_FrontlineGameMode and IsMatchInProgress (BP_CapturePoint@15..@157); the host sets it
    pub fl_match_in_progress: bool,
    /// the game mode's (HorseRespawnTime, BallistaRespawnTime, CatapultRespawnTime) when it is a MordhauGameMode
    pub game_mode_respawn: Option<(f64, f64, f64)>,
    /// the engine's global FMath::Rand stream
    pub rng: CrtRand,
    pub now: f64,
    /// what the construction scripts set (damage-state meshes); `begin_play` returns them first
    pub construction_events: Vec<WorldEvent>,
    /// the server (true, default) or a client (`set_authority(false)`, replication.rs)
    pub authority: bool,
    /// overlap triggers of any actor (hazard volumes, the spikes / swinging logs that also move): hazards.rs
    pub triggers: BTreeMap<ActorId, Trigger>,
    /// events from calls that return none (capture_point_prerequisites); the next `tick` returns them first
    pub pending: Vec<WorldEvent>,
}

fn path_of(v: &serde_json::Value) -> Option<&str> {
    v.get("ObjectPath").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

impl World {
    /// Build from a read map (mh_level::read); `pk` resolves Blueprint component templates (door / ladder points) and
    /// placed sub-objects (spline components)
    pub fn from_level(pk: &mh_level::Pkgs, d: &mh_level::LevelData) -> World {
        let mut actors = vec![];
        let mut triggers = BTreeMap::new();
        let path_to_id: BTreeMap<String, ActorId> = d.gameplay.iter().enumerate().map(|(i, a)| (a.path.clone(), i)).collect();
        for (i, a) in d.gameplay.iter().enumerate() {
            let c = &d.gameplay_classes[&a.class];
            let chain: Vec<String> = c.chain.iter().map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).collect();
            let props = Props { inst: a.props.clone(), defaults: c.defaults.clone() };
            let xf = a.xf.unwrap_or(mh_level::xf::IDENTITY);
            let has = |n: &str| chain.iter().any(|c| c == n);
            let pushable = || {
                let mut p = Pushable::new(pk, &props, Self::spline_of(pk, d, &props));
                p.xf = Some(xf);
                p
            };
            let kind = if has("BP_Door") {
                Kind::Door(Door::new(&props, pk, &c.chain, &xf))
            } else if has("BP_FrontlineDestroyable") {
                Kind::Objective(Objective::destroyable(&props))
            } else if has("BP_ItemDeliverySpot") {
                Kind::Objective(Objective::delivery(&props))
            } else if has("BP_FrontlinePushable") {
                Kind::Objective(Objective::pushable(&props, pushable()))
            } else if has("BP_FrontlineKillObjectiveWrapper") {
                Kind::Objective(Objective::kill_wrapper(&props))
            } else if has("BP_SplinePushableActor") || c.native == "PushableActor" {
                Kind::Pushable(pushable())
            } else if has("BP_DestroyableActor") {
                Kind::Destructible(Destructible::new(&props))
            } else if has("BP_ProgressDriver") {
                Kind::ProgressDriver(Driver::new(&props))
            } else if has("BP_SlaveProgressDriver") {
                Kind::SlaveDriver { master: None }
            } else if has("BP_StaticMeshProgressActor") {
                Kind::ProgressActor(ProgressActor::new(pk, &props, &c.chain, "StaticMesh"))
            } else if has("BP_BaseProgressActor") {
                // BP_SceneProgressActor (@92..@149) and the trap door / iron maiden classes that re-implement it on
                // BP_BaseProgressActor (BP_SceneProgressActor_TrapDoor_01:ExecuteUbergraph@44..@101, the same TLerp
                // on Holder behind HasAuthority @10) move their Holder
                Kind::ProgressActor(ProgressActor::new(pk, &props, &c.chain, "Holder"))
            } else if has("BP_ItemDeliverySpawn") {
                Kind::DeliverySpawn { active: props.b("bIsInteractable"), capture_point: None }
            } else if let Some(pu) = Pickup::of(&chain, &props) {
                Kind::Pickup(pu)
            } else if has("BP_OilCauldron") {
                Kind::Cauldron(Cauldron::new(pk, &c.chain, &xf))
            } else if has("BP_FeitoriaDoorLocker") {
                Kind::DoorLocker { doors: vec![], at: None }
            } else if has("BP_FrontlineKillObjective") {
                Kind::KillObjective(KillObjective::new(&props))
            } else if has("BP_Ladder") {
                Kind::Ladder(Ladder::new(&props, pk, &c.chain))
            } else if has("BP_EquipmentSpawner") {
                Kind::EquipmentSpawner(EquipmentSpawner::new(&props))
            } else if has("BP_VehicleSpawner") {
                Kind::VehicleSpawner(VehicleSpawner::new(&props, &chain, pk, &c.chain))
            } else {
                Kind::Inert
            };
            if let Some(t) = Trigger::of(&chain, &props, &xf, pk, &c.chain, &|p: &str| path_to_id.get(p).copied()) {
                triggers.insert(i, t);
            }
            actors.push(WorldActor { id: i, name: a.name.clone(), class: a.class.clone(), chain, xf, props, kind });
        }
        // capture points and their objectives / delivery spots
        let mut objectives = BTreeMap::new();
        let mut cps = BTreeMap::new();
        for a in &actors {
            if !a.chain.iter().any(|c| c.contains("CapturePoint")) {
                continue;
            }
            let resolve = |k: &str| -> Vec<Option<ActorId>> { a.props.arr(k).iter().map(|o| path_of(o).and_then(|p| path_to_id.get(p).copied())).collect() };
            let os = resolve("Objectives");
            if os.is_empty() {
                continue;
            }
            objectives.insert(a.id, os.iter().flatten().copied().collect::<Vec<_>>());
            cps.insert(a.id, CapturePoint::new(a.id, &a.props, os, resolve("DeliverySpots")));
        }
        let mut w = World { actors, objectives, cps, fl_match_in_progress: false, game_mode_respawn: None, rng: CrtRand::default(), now: 0.0, construction_events: vec![], authority: true, triggers, pending: vec![] };
        // OnInitialize(self) of each objective (BP_CapturePoint@1277..@1629: Initialize stores CapturePoint); a kill
        // wrapper forwards it to its KillObjective (BP_FrontlineKillObjectiveWrapper@360..@403 -> Point @3848)
        let links: Vec<(ActorId, ActorId)> = w.objectives.iter().flat_map(|(cp, os)| os.iter().map(move |o| (*o, *cp))).collect();
        for (o, cp) in links {
            let kt = w.actors[o].props.get("KillObjective").and_then(path_of).and_then(|p| path_to_id.get(p).copied());
            if let Kind::Objective(ob) = &mut w.actors[o].kind {
                ob.capture_point = Some(cp);
                if ob.kind == ObjectiveKind::KillWrapper {
                    ob.kill_target = kt;
                }
            }
            if let Some(k) = kt {
                if let Kind::KillObjective(ko) = &mut w.actors[k].kind {
                    ko.point = Some(cp);
                }
            }
        }
        // delivery spots' DeliverySpawns (BP_ItemDeliverySpot:Initialize@213..@370: each spawn's CapturePoint)
        for i in 0..w.actors.len() {
            let spawns: Vec<ActorId> = w.actors[i].props.arr("DeliverySpawns").iter().filter_map(|o| path_of(o).and_then(|p| path_to_id.get(p).copied())).collect();
            let cp = match &mut w.actors[i].kind {
                Kind::Objective(o) if o.kind == ObjectiveKind::Delivery => {
                    o.delivery_spawns = spawns.clone();
                    o.capture_point
                }
                _ => continue,
            };
            for s in spawns {
                if let Kind::DeliverySpawn { capture_point, .. } = &mut w.actors[s].kind {
                    *capture_point = cp;
                }
            }
        }
        // drivers: TargetProgressActors get Driver = self, SlaveCranks MasterCrank = self (BP_ProgressDriver@2052 /
        // @1695); the door locker's soft references (by actor name, "Doors To Lock")
        let by_name: BTreeMap<String, ActorId> = w.actors.iter().map(|a| (a.name.clone(), a.id)).collect();
        for i in 0..w.actors.len() {
            let res = |k: &str| -> Vec<ActorId> { w.actors[i].props.arr(k).iter().filter_map(|o| path_of(o).and_then(|p| path_to_id.get(p).copied())).collect() };
            let (targets, slaves) = (res("TargetProgressActors"), res("SlaveCranks"));
            let doors: Vec<ActorId> = w.actors[i]
                .props
                .arr("Doors To Lock")
                .iter()
                .filter_map(|v| {
                    let s = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                        let a = v.get("AssetPathName").and_then(|x| x.as_str()).unwrap_or("");
                        let sub = v.get("SubPathString").and_then(|x| x.as_str()).unwrap_or("");
                        format!("{a}:{sub}")
                    });
                    by_name.get(s.rsplit(['.', ':']).next()?).copied()
                })
                .collect();
            match &mut w.actors[i].kind {
                Kind::ProgressDriver(d) => {
                    d.targets = targets;
                    d.slaves = slaves.clone();
                }
                Kind::DoorLocker { doors: ds, .. } => *ds = doors,
                _ => {}
            }
            for s in slaves {
                if let Kind::SlaveDriver { master } = &mut w.actors[s].kind {
                    *master = Some(i);
                }
            }
        }
        // construction scripts: BP_DestroyableActor's ReplicatedHealth / damage mesh (UserConstructionScript@20..@211)
        let mut cev = vec![];
        for a in w.actors.iter_mut() {
            let p = a.props.clone();
            match &mut a.kind {
                Kind::Destructible(x) => x.construct(a.id, &p, &mut cev),
                Kind::Door(d) => d.health.construct(a.id, &p, &mut cev),
                Kind::Objective(o) => o.base.construct(a.id, &p, &mut cev),
                _ => {}
            }
        }
        w.construction_events = cev;
        w
    }

    /// BP_SplinePushableActor TargetSplineActor -> its USplineComponent (the actor's `Spline` / `SplineComponent` /
    /// `Path` component property) with its world transform
    fn spline_of(pk: &mh_level::Pkgs, d: &mh_level::LevelData, p: &Props) -> Option<crate::spline::Spline> {
        let target = p.get("TargetSplineActor").and_then(path_of)?.to_string();
        let (pkg, idx) = target.rsplit_once('.')?;
        let exps = pk.load_pkg(pkg);
        let actor = exps.get(idx.parse::<usize>().ok()?)?;
        let ap = pk.props(actor);
        let comp_ref = ["Spline", "SplineComponent", "Path"].iter().find_map(|k| ap.get(*k)).cloned();
        let comp = pk.obj(comp_ref.as_ref())?;
        let lv = d.levels.iter().find(|l| pkg.eq_ignore_ascii_case(&l.pkg)).map(|l| l.xf).unwrap_or(mh_level::xf::IDENTITY);
        let cprops = pk.props(&comp);
        Some(crate::spline::Spline::from_props(&cprops, lv * pk.world_xf(&comp)))
    }

    /// ReceiveBeginPlay of every actor (construction-script state, spawners' first spawn), then each capture point's
    /// BeginPlay objective pass (BP_CapturePoint@2650..@1742: OnInitialize loop, ObjectivesChanged)
    pub fn begin_play(&mut self, q: &dyn Queries) -> Vec<WorldEvent> {
        let mut ev = std::mem::take(&mut self.construction_events);
        let now = self.now;
        let gm = self.game_mode_respawn;
        for a in self.actors.iter_mut() {
            match &mut a.kind {
                Kind::Door(d) => {
                    d.begin_play(a.id, &mut ev);
                    d.health.begin_play(&a.props, now);
                }
                Kind::Destructible(x) => x.begin_play(&a.props, now),
                Kind::Pickup(pu) => pu.begin_play(a.id, &a.props, &a.xf, now, self.authority, &mut ev),
                Kind::ProgressDriver(d) => d.begin_play(&a.props, now),
                // BP_FeitoriaDoorLocker ReceiveBeginPlay@1594: Delay(1.0)
                Kind::DoorLocker { at, .. } => *at = Some(now + 1.0),
                Kind::EquipmentSpawner(s) => s.begin_play(now),
                Kind::VehicleSpawner(s) => s.begin_play(a.id, &a.xf, now, gm, q, &mut self.rng, &mut ev),
                Kind::Pushable(p) => Self::place_on_spline(a.id, p, &mut ev),
                Kind::Objective(o) => {
                    o.base.begin_play(&a.props, now);
                    if let Some(p) = o.push.as_deref_mut() {
                        Self::place_on_spline(a.id, p, &mut ev)
                    }
                }
                _ => {}
            }
        }
        for cp in self.cps.keys().copied().collect::<Vec<_>>() {
            self.objectives_changed(cp, &mut ev);
        }
        ev
    }

    /// BP_SplinePushableActor ReceiveBeginPlay @1094..@1239: with a TargetSplineActor, the actor onto its spline
    fn place_on_spline(id: ActorId, p: &mut Pushable, ev: &mut Vec<WorldEvent>) {
        if let Some(x) = p.spline_transform() {
            p.xf = Some(x);
            ev.push(WorldEvent::ActorTransform { actor: id, xf: x });
        }
    }

    /// OnInteractionStart (a character's use key on the actor)
    pub fn interact(&mut self, actor: ActorId, ch: &CharView) -> Vec<WorldEvent> {
        let mut ev = vec![];
        // the interaction component starts an interaction only with a target whose CanInteract holds
        // (ValidateInteractionTarget rva 0x14e1780)
        if !self.can_interact(actor, ch).0 {
            return ev;
        }
        let now = self.now;
        // BP_SlaveProgressDriver:OnInteractionStart@223..@314: MasterCrank.ToggleValue (authority)
        if let Kind::SlaveDriver { master: Some(m) } = self.actors[actor].kind {
            let a = &mut self.actors[m];
            if let Kind::ProgressDriver(d) = &mut a.kind {
                d.toggle(&a.props, now);
            }
            return ev;
        }
        let a = &mut self.actors[actor];
        match &mut a.kind {
            // BP_SwitchInteractable:OnInteractionStart@218: ToggleValue
            Kind::ProgressDriver(d) => d.toggle(&a.props, now),
            Kind::Pickup(pu) => pu.interact(actor, &a.props, ch, now, &mut self.rng, &mut ev),
            Kind::DeliverySpawn { capture_point, .. } => {
                // UsableByTeam: the point's OwningTeam 0 -> 1, 1 -> 0, else the select default (@261..@350)
                let owner = capture_point.and_then(|c| self.cps.get(&c)).and_then(|c| c.owning_team);
                let usable = match owner {
                    Some(0) => Some(1),
                    Some(1) => Some(0),
                    _ => None,
                };
                ev.push(WorldEvent::GiveDeliverable { by: actor, char: ch.id, class: a.props.obj("DeliverableClass"), usable_by_team: usable, item_type: a.props.i("Type") });
            }
            // BP_OilCauldron:OnInteractionStart@1351 -> Activate
            Kind::Cauldron(c) => c.activate(&a.props, ch, now),
            Kind::Door(d) => d.interact(&a.xf, ch, self.now, &mut ev),
            Kind::Ladder(l) => {
                if l.can_interact(&a.xf, ch) {
                    l.mount(actor, &a.xf, ch, &mut ev)
                }
            }
            _ => {}
        }
        ev
    }

    /// OnHeldInteractionStart (held use: ladder drop / raise)
    pub fn held_interact(&mut self, actor: ActorId, _ch: &CharView) -> Vec<WorldEvent> {
        if let Kind::Ladder(l) = &mut self.actors[actor].kind {
            l.toggle(self.now);
        }
        vec![]
    }

    /// IInteractable CanInteract / CanHeldInteract of an actor for a character (BP_MordhauActor: bIsInteractable;
    /// BP_Ladder:CanInteract; held: a ladder that can drop / raise and is not animating [BP_Ladder:CanHeldInteract
    /// not read, UNCONFIRMED])
    pub fn can_interact(&self, actor: ActorId, ch: &CharView) -> (bool, bool) {
        let a = &self.actors[actor];
        match &a.kind {
            Kind::Door(_) => (a.props.b("bIsInteractable"), false),
            Kind::Ladder(l) => (l.can_interact(&a.xf, ch), l.can_drop_and_raise && !l.animating),
            Kind::ProgressDriver(d) => (d.can_interact(&a.props), false),
            // BP_ItemDeliverySpawn:CanInteract (@0..@328): a capture point, the character's team not its OwningTeam and
            // the right hand not already a BP_DeliverableEquipment, then the parent's (bIsInteractable)
            Kind::DeliverySpawn { active, capture_point } => {
                let owner = capture_point.and_then(|c| self.cps.get(&c)).map(|c| c.owning_team);
                (*active && owner.is_some() && owner.flatten() != ch.team && !ch.holding_deliverable, false)
            }
            Kind::Pickup(pu) => (a.props.get("bIsInteractable").and_then(|v| v.as_bool()).unwrap_or(true) && pu.can_interact(&a.props, ch), false),
            // OnRep_State@14..@89: bIsInteractable while State 0
            Kind::Cauldron(c) => (c.state == 0, false),
            // BP_SlaveProgressDriver:CanInteract: MasterCrank valid and its CanInteract, then its own (bIsInteractable)
            Kind::SlaveDriver { master } => {
                let own = a.props.get("bIsInteractable").and_then(|v| v.as_bool()).unwrap_or(true);
                (own && master.map(|m| self.can_interact(m, ch).0).unwrap_or(false), false)
            }
            _ => (false, false),
        }
    }

    /// The use-key target (UInteractionSystemComponent::GetInteractionTarget rva 0x14bd220) from the host's hits of
    /// `interaction::sweeps(eye, view_dir, ..)` (ValidateInteractionTarget rva 0x14e1780: CanInteract or
    /// CanHeldInteract)
    pub fn interaction_target(&self, ch: &CharView, eye: V, view_dir: V, hits: &[crate::interaction::SweepHit]) -> Option<ActorId> {
        let sw = crate::interaction::sweeps(eye, view_dir, [1.0; 3]);
        let valid = |a: ActorId| a < self.actors.len() && {
            let (i, h) = self.can_interact(a, ch);
            i || h
        };
        crate::interaction::choose(eye, view_dir, &sw, hits, &valid)
    }

    /// AActor::TakeDamage -> ReceiveAnyDamage. `amount` is the raw float the engine passes (ProcessHitForDamage's
    /// value after the caller's modifiers); the Blueprint math runs in f32 in the engine, here in f64 [rounding
    /// differences below 1e-6 relative]
    pub fn apply_damage(&mut self, actor: ActorId, amount: f32, d: &DamageInfo, q: &dyn Queries, chars: &[CharView]) -> Vec<WorldEvent> {
        let amount = amount as f64;
        let mut ev = vec![];
        let now = self.now;
        // the objective's capture point: OwningTeam (mh-mode's) and bObjectivesCompleted (BP_CapturePoint's)
        let cp = match &self.actors[actor].kind {
            Kind::Objective(o) => o.capture_point.and_then(|c| self.cps.get(&c)).map(|c| (c.owning_team, c.objectives_completed)),
            _ => None,
        };
        // AActor::TakeDamage does nothing while bCanBeDamaged is false (UE 4.26 Actor.cpp; the Frontline objectives
        // keep their own flag, `Objective::can_be_damaged`)
        if !matches!(self.actors[actor].kind, Kind::Objective(_)) && self.actors[actor].props.get("bCanBeDamaged").and_then(|v| v.as_bool()) == Some(false) {
            return ev;
        }
        let o = self.overlaps_of(actor, q, chars);
        let ovf = move || o.clone();
        let ov: crate::destructible::Ov = &ovf;
        let mut a = std::mem::replace(&mut self.actors[actor].kind, Kind::Inert);
        let (xf, props) = (self.actors[actor].xf, self.actors[actor].props.clone());
        match &mut a {
            // BP_OilCauldron:ReceiveAnyDamage (authority, @683..@1006): a MordhauCharacter causer kicking (AttackMotion
            // Move 4) while interactable -> Activate
            Kind::Cauldron(c) => {
                if d.attack_move == Some(4) && c.state == 0 {
                    if let Some(cv) = d.causer.and_then(|id| chars.iter().find(|v| v.id == id)) {
                        c.activate(&props, cv, now);
                    }
                }
            }
            Kind::Door(door) => door.receive_any_damage(actor, &xf, amount, d, now, ov, &mut ev),
            Kind::Destructible(x) => x.receive_any_damage(actor, &props, amount, d, now, ov, &mut ev),
            Kind::Objective(o) => {
                let (cp_team, cp_done) = cp.unwrap_or((None, false));
                o.receive_any_damage(actor, &props, amount, d, cp_team, cp_done, now, ov, &mut ev)
            }
            _ => {}
        }
        self.actors[actor].kind = a;
        self.settle(&mut ev);
        ev
    }

    /// What an actor's StaticMesh overlaps for the destructible unstuck pass
    fn overlaps_of(&self, actor: ActorId, q: &dyn Queries, chars: &[CharView]) -> Overlaps {
        Overlaps {
            chars: q.overlapping_chars(actor).into_iter().map(|c| (c, chars.iter().any(|v| v.id == c && v.in_vehicle))).collect(),
            actors: q
                .overlapping_actors(actor)
                .into_iter()
                .map(|a| (a, a < self.actors.len() && self.actors[a].chain.iter().any(|c| c == "BP_DestroyableActor")))
                .collect(),
        }
    }

    /// The kill objective (a placed BP_FrontlineKillObjective character) took `damage` (ReceiveAnyDamage's Damage);
    /// `health_after` / `dead_after` are the character's after the combat side applied it
    pub fn kill_objective_damaged(&mut self, actor: ActorId, damage: f32, d: &DamageInfo, health_after: u8, dead_after: bool) -> Vec<WorldEvent> {
        let mut ev = vec![];
        let p = self.actors[actor].props.clone();
        let owner = match &self.actors[actor].kind {
            Kind::KillObjective(k) => k.point.and_then(|c| self.cps.get(&c)).and_then(|c| c.owning_team),
            _ => return ev,
        };
        if let Kind::KillObjective(k) = &mut self.actors[actor].kind {
            k.damaged(&p, damage as f64, d, health_after, dead_after, owner, &mut ev);
        }
        self.settle(&mut ev);
        ev
    }

    /// The kill objective character was destroyed (ReceiveDestroyed)
    pub fn kill_objective_destroyed(&mut self, actor: ActorId) -> Vec<WorldEvent> {
        let mut ev = vec![];
        if let Kind::KillObjective(k) = &mut self.actors[actor].kind {
            k.destroyed(&mut ev);
        }
        self.settle(&mut ev);
        ev
    }

    /// mh-mode's capture point changed owner (AControlPoint OwningTeam; the Blueprint reads it in ObjectivesChanged,
    /// TriggerWinDelayed and the destroyables' team filter)
    pub fn set_capture_point_owner(&mut self, cp: ActorId, owning_team: Option<u8>) {
        if let Some(c) = self.cps.get_mut(&cp) {
            c.owning_team = owning_team;
        }
    }

    /// mh-mode's control point raised enemy_gained_prerequisites / enemy_lost_prerequisites (AControlPoint::Tick ->
    /// EnemyGainedPrerequisites / EnemyLostPrerequisites): BP_CapturePoint loops OnEnemyGainedPrerequisites
    /// (@7572..@7943) / OnEnemyLostPrerequisites (@817..@1165) over its objectives
    pub fn capture_point_prerequisites(&mut self, cp: ActorId, gained: bool) {
        let Some(c) = self.cps.get(&cp) else { return };
        let done = c.objective_progress == 1.0;
        for o in c.objectives.clone().into_iter().flatten() {
            let a = &mut self.actors[o];
            if let Kind::Objective(ob) = &mut a.kind {
                ob.set_prerequisites(&a.props, gained, done);
            }
        }
        let mut ev = vec![];
        self.sync_delivery_spawns(&mut ev);
        self.pending.extend(ev);
    }

    /// One objective's OnEnemyGained / LostPrerequisites (tests, objectives outside a capture point)
    pub fn set_prerequisites(&mut self, actor: ActorId, gained: bool) {
        let done = self.cp_of(actor).map(|c| self.cps[&c].objective_progress == 1.0).unwrap_or(false);
        let a = &mut self.actors[actor];
        if let Kind::Objective(o) = &mut a.kind {
            o.set_prerequisites(&a.props, gained, done);
        }
    }

    fn cp_of(&self, actor: ActorId) -> Option<ActorId> {
        match &self.actors[actor].kind {
            Kind::Objective(o) => o.capture_point.filter(|c| self.cps.contains_key(c)),
            _ => None,
        }
    }

    /// A BP_DeliverableEquipment of `item_type` (its Type) entered a delivery spot's Area; `scorer` = its
    /// LastEquippedByPlayerController's character; true = consumed
    pub fn deliver(&mut self, actor: ActorId, item_type: i64, scorer: Option<CharId>, scorer_team: Option<u8>) -> (bool, Vec<WorldEvent>) {
        let mut ev = vec![];
        let a = &mut self.actors[actor];
        let ok = match &mut a.kind {
            Kind::Objective(o) => o.deliver(&a.props, item_type, scorer, scorer_team, &mut ev),
            _ => false,
        };
        self.settle(&mut ev);
        (ok, ev)
    }

    /// The host's report of a kill wrapper's KillObjective (its GetObjectiveProgress / IsCompleted), or a forced
    /// progress; then the capture point's ObjectivesChanged
    pub fn set_objective_progress(&mut self, actor: ActorId, progress: f64, completed: bool) -> Vec<WorldEvent> {
        let mut ev = vec![];
        if let Kind::Objective(o) = &mut self.actors[actor].kind {
            o.set_progress(progress);
            o.kill_done = completed;
            if let Some(cp) = o.capture_point {
                ev.push(WorldEvent::ObjectivesChanged { capture_point: cp });
            }
        }
        self.settle(&mut ev);
        ev
    }

    /// The host's K2_TeleportTo result for a horse spawner's horse
    pub fn spawn_teleport_result(&mut self, spawner: ActorId, ok: bool) {
        let now = self.now;
        if let Kind::VehicleSpawner(s) = &mut self.actors[spawner].kind {
            s.teleport_result(ok, now, &mut self.rng);
        }
    }

    /// BP_CapturePoint:ObjectivesChanged: ObjectiveProgress, OnAnyObjectiveProgressChanged on every objective, then
    /// the authority part (capture.rs)
    pub fn objectives_changed(&mut self, cp: ActorId, ev: &mut Vec<WorldEvent>) {
        let Some(mut c) = self.cps.remove(&cp) else { return };
        c.compute_progress(&|o| self.objective_progress_of(o));
        let p = c.objective_progress;
        // every valid DeliverySpot's progress == 1 (BP_ItemDeliverySpot:AnyObjectiveProgressChanged@197..@742)
        let spots_done = c.delivery_spots.iter().flatten().all(|s| self.objective_progress_of(*s) == 1.0);
        for o in c.objectives.iter().flatten() {
            let a = &mut self.actors[*o];
            let mut kt = None;
            if let Kind::Objective(ob) = &mut a.kind {
                ob.on_any_objective_progress_changed(&a.props, p, spots_done);
                kt = ob.kill_target;
            }
            // the wrapper forwards OnAnyObjectiveProgressChanged to its KillObjective (@95..@138)
            if let Some(k) = kt {
                if let Kind::KillObjective(ko) = &mut self.actors[k].kind {
                    ko.on_any_objective_progress_changed(p);
                }
            }
        }
        if self.authority {
            c.authority_changed(self.now, ev);
        }
        self.cps.insert(cp, c);
    }

    fn objective_progress_of(&self, o: ActorId) -> f64 {
        match &self.actors[o].kind {
            // BP_FrontlineKillObjectiveWrapper:GetObjectiveProgress@43..@134: KillObjective's progress x Weight
            Kind::Objective(ob) if ob.kill_target.is_some() => match &self.actors[ob.kill_target.unwrap()].kind {
                Kind::KillObjective(k) => k.progress() * self.actors[o].props.f("Weight"),
                _ => ob.progress(&self.actors[o].props),
            },
            Kind::Objective(ob) => ob.progress(&self.actors[o].props),
            // an objective that is not one of the ported kinds [UNCONFIRMED: none on the probed maps]
            _ => 0.0,
        }
    }

    /// Run the capture points' ObjectivesChanged for the ObjectivesChanged events emitted so far (each once, in order)
    /// The delivery spots' ActivateSpawns / DisableSpawns onto their BP_ItemDeliverySpawns
    fn sync_delivery_spawns(&mut self, ev: &mut Vec<WorldEvent>) {
        let mut todo = vec![];
        for a in &self.actors {
            if let Kind::Objective(o) = &a.kind {
                for s in &o.delivery_spawns {
                    todo.push((*s, o.spawns_active));
                }
            }
        }
        for (s, on) in todo {
            if let Kind::DeliverySpawn { active, .. } = &mut self.actors[s].kind {
                if *active != on {
                    *active = on;
                    if !on {
                        ev.push(WorldEvent::DestroySpawnedDeliverables { by: s });
                    }
                }
            }
        }
    }

    fn settle(&mut self, ev: &mut Vec<WorldEvent>) {
        self.sync_delivery_spawns(ev);
        if !self.authority {
            return self.settle_client(ev);
        }
        let cps: Vec<ActorId> = ev.iter().filter_map(|e| if let WorldEvent::ObjectivesChanged { capture_point } = e { Some(*capture_point) } else { None }).collect();
        for cp in cps {
            self.objectives_changed(cp, ev);
        }
    }

    pub fn tick(&mut self, dt: f64, q: &dyn Queries, chars: &[CharView]) -> Vec<WorldEvent> {
        self.now += dt;
        let now = self.now;
        let mut ev = std::mem::take(&mut self.pending);
        let mut carried: Vec<(ActorId, V, V)> = vec![];
        let mut driven: Vec<(Vec<ActorId>, f64)> = vec![];
        let mut locks: Vec<ActorId> = vec![];
        let destroyable: Vec<bool> = self.actors.iter().map(|a| a.chain.iter().any(|c| c == "BP_DestroyableActor")).collect();
        let ovf = |id: ActorId| Overlaps {
            chars: q.overlapping_chars(id).into_iter().map(|c| (c, chars.iter().any(|v| v.id == c && v.in_vehicle))).collect(),
            actors: q.overlapping_actors(id).into_iter().map(|a| (a, destroyable.get(a).copied().unwrap_or(false))).collect(),
        };
        let auth = self.authority;
        for a in self.actors.iter_mut() {
            let id = a.id;
            let ov = || ovf(id);
            match &mut a.kind {
                Kind::Door(d) => d.tick(a.id, &a.xf, dt, now, q, chars, &ov, &mut ev),
                Kind::Destructible(x) => x.tick(a.id, &a.props, now, &ov, &mut ev),
                Kind::ProgressDriver(d) => {
                    if let Some(v) = d.tick(&a.props, dt, now) {
                        driven.push((d.targets.clone(), v));
                    }
                }
                Kind::DoorLocker { doors, at } => {
                    if at.is_some_and(|t| now >= t) {
                        *at = None;
                        locks.extend(doors.iter().copied());
                    }
                }
                Kind::Objective(o) => {
                    o.base.tick(a.id, &a.props, now, &ov, &mut ev);
                    if let Some(p) = o.push.as_deref_mut() {
                        let (changed, m) = Self::tick_pushable(a.id, p, dt, now, auth, q, chars, &mut ev);
                        carried.extend(m);
                        o.progress = p.progress;
                        if let (true, Some(cp)) = (changed, o.capture_point) {
                            // BP_FrontlinePushable:OnProgressUpdated@148..@211: CapturePoint.ObjectivesChanged
                            ev.push(WorldEvent::ObjectivesChanged { capture_point: cp });
                        }
                    }
                }
                Kind::Pushable(p) => {
                    let (_, m) = Self::tick_pushable(a.id, p, dt, now, auth, q, chars, &mut ev);
                    carried.extend(m);
                }
                Kind::Ladder(l) => l.tick(a.id, &a.xf, now, q, chars, &mut ev),
                // spawners and the capture points' latent delays are server-only (HasAuthority / TrySpawnVehicle @0)
                Kind::EquipmentSpawner(s) if auth => s.tick(a.id, &a.props, &a.xf, now, q, &mut self.rng, &mut ev),
                Kind::VehicleSpawner(s) if auth => s.tick(a.id, &a.xf, now, q, &mut self.rng, &mut ev),
                Kind::EquipmentSpawner(_) | Kind::VehicleSpawner(_) => {}
                Kind::Pickup(pu) if auth => pu.tick(a.id, &a.props, now, &mut ev),
                Kind::Cauldron(c) if auth => c.tick(a.id, &a.props, now, &mut ev),
                Kind::DeliverySpawn { .. } | Kind::Pickup(_) | Kind::Cauldron(_) | Kind::SlaveDriver { .. } | Kind::ProgressActor(_) | Kind::KillObjective(_) | Kind::Inert => {}
            }
        }
        // overlap triggers (hazards.rs); the tree breaker / door activator act on the actors their queue yields
        let mut trigs = std::mem::take(&mut self.triggers);
        for (id, t) in trigs.iter_mut() {
            let (id, want) = (*id, match t.kind {
                TriggerKind::TreeBreaker => "BP_DestroyableTree",
                TriggerKind::DoorActivator => "BP_DestroyableCastleDoor",
                _ => "",
            });
            let actors = &self.actors;
            let sphere = || q.overlapping_actors(id).into_iter().filter(|a| *a < actors.len() && actors[*a].chain.iter().any(|c| c == want)).collect::<Vec<_>>();
            let comp = |c: &str| q.component_chars(id, c);
            let todo = t.tick(id, now, auth, &comp, chars, &sphere, &mut self.rng, &mut ev);
            for a in todo {
                match t.kind {
                    // BP_TreeBreaker@464: SetHealth(0, null)
                    TriggerKind::TreeBreaker => {
                        let x = &mut self.actors[a];
                        if let Kind::Destructible(dx) = &mut x.kind {
                            dx.set_health(a, &x.props, 0.0, None, None, now, &Overlaps::default, &mut ev);
                        }
                    }
                    // BP_DoorActivator@551: bCanBeDamaged = true
                    _ => {
                        let x = &mut self.actors[a];
                        x.props.inst.insert("bCanBeDamaged".into(), true.into());
                        if let Kind::Door(dd) = &mut x.kind {
                            dd.props.inst.insert("bCanBeDamaged".into(), true.into());
                        }
                    }
                }
            }
        }
        self.triggers = trigs;
        // BP_ProgressDriver -> TargetProgressActors.ProgressUpdatedInternal(SmoothedValue) (@1547)
        for (targets, v) in driven {
            for t in targets {
                if let Kind::ProgressActor(pa) = &mut self.actors[t].kind {
                    pa.progress_updated_internal(t, v, &mut ev);
                }
            }
        }
        // BP_FeitoriaDoorLocker@684..@1403: each door bCanBeDamaged = false, bIsInteractable = false, DamageFactor = 0,
        // SetHealth(255, null)
        for door in locks {
            self.lock_door(door, &mut ev);
        }
        // BP_SplinePushableActor:OnProgressUpdated (authority): carry / crush what the moved mesh overlaps
        for (id, prev, cur) in carried.into_iter().filter(|_| auth) {
            let destroyable = |a: ActorId| self.actors[a].chain.iter().any(|c| c == "BP_FrontlineDestroyable");
            let in_vehicle = |c: CharId| chars.iter().any(|v| v.id == c && v.in_vehicle);
            crate::pushable::carry(id, prev, cur, &q.overlapping_chars(id), &q.overlapping_actors(id), &destroyable, &in_vehicle, &mut ev);
        }
        let fl = self.fl_match_in_progress;
        for c in self.cps.values_mut().filter(|_| auth) {
            c.tick(now, fl, &mut ev);
        }
        self.settle(&mut ev);
        ev
    }

    /// A component hit on a world actor by a character (UPrimitiveComponent OnComponentHit; the impalement volumes'
    /// Box). Authority only
    pub fn component_hit(&mut self, actor: ActorId, ch: &CharView) -> Vec<WorldEvent> {
        let mut ev = vec![];
        if !self.authority {
            return ev;
        }
        let now = self.now;
        if let Some(t) = self.triggers.get_mut(&actor) {
            t.hit(actor, ch, now, &mut ev);
        }
        ev
    }

    /// BP_FeitoriaDoorLocker's lock of one BP_Door
    pub fn lock_door(&mut self, door: ActorId, ev: &mut Vec<WorldEvent>) {
        let now = self.now;
        let a = &mut self.actors[door];
        let Kind::Door(d) = &mut a.kind else { return };
        for p in [&mut a.props, &mut d.props] {
            p.inst.insert("bCanBeDamaged".into(), false.into());
            p.inst.insert("bIsInteractable".into(), false.into());
            p.inst.insert("DamageFactor".into(), 0.0.into());
        }
        let p = d.props.clone();
        d.health.set_health(door, &p, 255.0, None, None, now, &|| Overlaps::default(), ev);
    }

    /// APushableActor::Tick + (spline pushables) the move; returns whether Progress changed and (actor, previous
    /// location, new location) to carry
    #[allow(clippy::too_many_arguments)]
    fn tick_pushable(id: ActorId, p: &mut Pushable, dt: f64, now: f64, auth: bool, q: &dyn Queries, chars: &[CharView], ev: &mut Vec<WorldEvent>) -> (bool, Option<(ActorId, V, V)>) {
        let moved = if auth {
            let area: Vec<&CharView> = q.area_chars(id).iter().filter_map(|c| chars.iter().find(|v| v.id == *c)).collect();
            p.tick(id, dt, &area, ev)
        } else {
            p.client_tick(id, dt, now, ev)
        };
        if !moved {
            return (false, None);
        }
        let Some(x) = p.spline_transform() else { return (true, None) };
        let prev = p.xf.map(|x| x.translation()).unwrap_or(x.translation());
        p.xf = Some(x);
        ev.push(WorldEvent::ActorTransform { actor: id, xf: x });
        (true, Some((id, prev, x.translation())))
    }

    pub fn ladder(&self, actor: ActorId) -> Option<crate::ladder::LadderInfo> {
        match &self.actors[actor].kind {
            Kind::Ladder(l) => Some(l.info(&self.actors[actor].xf)),
            _ => None,
        }
    }

    /// A capture point's objectives: (ObjectiveProgress as BP_CapturePoint computes it, completed count, total)
    pub fn objective_progress(&self, capture_point: ActorId) -> Option<(f64, usize, usize)> {
        let c = self.cps.get(&capture_point)?;
        let mut p = 0.0;
        let mut done = 0;
        for o in &c.objectives {
            match o {
                Some(o) => {
                    p += self.objective_progress_of(*o);
                    if let Kind::Objective(ob) = &self.actors[*o].kind {
                        // the wrapper's IsCompleted@43: KillObjective.IsCompleted = GetIsDead
                        done += match ob.kill_target.map(|k| &self.actors[k].kind) {
                            Some(Kind::KillObjective(k)) => k.dead,
                            _ => ob.completed(),
                        } as usize;
                    }
                }
                None => p += 1.0,
            }
        }
        let n = c.objectives.len();
        Some(((p / n as f64).clamp(0.0, 1.0), done, n))
    }

    /// Is this objective a pushable one (for hosts listing objectives)
    pub fn is_pushable_objective(&self, actor: ActorId) -> bool {
        matches!(&self.actors[actor].kind, Kind::Objective(o) if o.kind == ObjectiveKind::Pushable)
    }
}

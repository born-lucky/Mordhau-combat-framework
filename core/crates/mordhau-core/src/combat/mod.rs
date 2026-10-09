//! Combat: the deterministic fixed-step melee simulation, ported 1:1 from the GDScript reference
//! godot/game/combat/*.gd (which ports UMotionSystemComponent, UMordhauMotion and its subclasses, UAttackMotion's
//! hit processing, the stamina / health stat components from extract/native/decomp; every function below keeps the
//! reference's `rva=` + PDB-name citation).
//!
//! Layout (mirrors game/combat/):
//!   world.rs     CombatState (clock, schedule, input, tick order, events, trace memory)
//!   system.rs    MotionSystem (one fighter: motion switching, NetMotion, stamina, health, input requests)
//!   motion.rs    UMordhauMotion base + UIdleMotion + virtual dispatch
//!   attack.rs    UAttackMotion / UStrikeMotion / UStabMotion / UKickMotion
//!   parry.rs     UParryMotion       feinted.rs  UFeintedMotion     blocked.rs  UBlockedMotion
//!   react.rs     UFlinchMotion / UStunMotion / UDisarmedMotion
//!   melee_hit.rs ExecuteAttackTracingAndLogic, ProcessHitForBlocking / ForDamage, CheckChamber, HandleWasParried
//!   damage.rs    AMordhauCharacter::ComputeMeleeDamage / GetArmorTierForBone
//!   tracer.rs    AMordhauWeapon tracers (PrepareForTracing, SampleTracers)
//!   ranged.rs    AMordhauProjectile::ProcessProjectileHit, the character branch (ranged damage / parry)
//!   horse.rs     AHorse combat: DoKnockback (trample), mounted rider gates
//!
//! Ownership: the World owns its fighters (Vec, insertion order = tick order); a fighter owns its motions in a slab
//! (`MotionId` indices). Every cross-object reference is an index (fighter index or `Fighter::id`, `MotionId`), so
//! the rules can be written as `&mut World` methods that re-read state after every call that may switch motions,
//! exactly where the GDScript re-reads `sys.motion`.

pub mod enums;
pub mod exe;
pub mod geometry;
pub mod equipment;
pub mod world;
pub mod system;
pub mod motion;
pub mod attack;
pub mod parry;
pub mod feinted;
pub mod blocked;
pub mod climb;
pub mod react;
pub mod melee_hit;
pub mod worldhit;
pub mod damage;
pub mod tracer;
pub mod net;
pub mod ranged;
pub mod rangedmotion;
pub mod turncap;
pub mod horse;
pub mod modeswitch;

pub use motion::{Motion, MotionId, MotionKind};
pub use system::{Fighter, NetMotion};
pub use world::{Input, World};
pub use equipment::{EquipmentId, EquipmentInstance, EquipmentPlacement, EquipmentCounts};
pub use net::{NetCtx, NetHooks, NetSlot};

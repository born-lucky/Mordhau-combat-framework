//! Bots (godot/game/ai/**): AMordhauAIController (controller.rs), UBotBehaviorProfile (profile.rs), the behavior
//! tree runner (bt.rs) and the BT tasks (tasks.rs). World queries (navmesh, traces, stimuli) and the fighter input
//! entry points are the `BotHost` trait the host implements (BotWorld in Godot today; mh-runtime later).

pub mod body;
pub mod bot_profiles;
pub mod bt;
pub mod bt_read;
pub mod combat_host;
pub mod controller;
pub mod nav;
pub mod profile;
pub mod tasks;

pub use body::{BodyId, BotBody, BotHost, InventoryItem, MotionView, PawnView, Sense, Stimulus, WeaponView};
pub use bt::{BbVal, BtTree, TreeDef};
pub use controller::{BotController, Bots, Ctx, Facing, PathStatus, Request, WorldState};
pub use profile::{BehaviorProfile, Profile};
pub use nav::{NavQueries, PathFollower, WithNav, WithSight};

//! Interactable pickups and supplies: BP_AmmoBox (BP_LocalInteractableChest > BP_LocalCooldownInteractable), the
//! rock / brick piles (BP_RestockableEquipmentSpawn), BP_FoodConsumable, BP_FoodConsumableRespawn and the random
//! weapon pickups (BP_RandomEquipment). Bytecode: state/world_kismet/<class>.txt.

use crate::props::Props;
use crate::spawner::{range_i, CrtRand};
use crate::world::{ActorId, CharView, WorldEvent};
use mh_level::xf::Xf;

#[derive(Clone, Debug)]
pub enum Pickup {
    /// BP_AmmoBox: OnInteractionStart -> AvailableInteractionStart (the server never marks it depleted: IsDepleted is
    /// the local client's, BP_LocalCooldownInteractable:UpdateValue reads GetPlayerController(0)) -> Restock (@0..@481):
    /// a BP_MordhauPlayerController whose NextAmmoBoxAvailableTime has passed gets
    /// Character.RestockEquipmentFromAmmoBox(), and on success ReplicatedAmmoBoxCooldown + 1 (the host's)
    AmmoBox,
    /// BP_RestockableEquipmentSpawn: Ammo (BeginPlay MaxAmmo, @908..@15); use (CanInteract: Ammo > 0 = bIsInteractable
    /// from OnRep_Ammo@1157 and the right hand not already an `Equipment`): Ammo - 1 (@827), the replenish timer when
    /// not already replenishing (AmmoReplenishInterval, @701..@735), an `Equipment` at the character's transform
    /// picked up (@389..@644); ReplenishAmmo (@274): Ammo + 1, until MaxAmmo the timer again
    Restockable { ammo: u8, replenishing: bool, replenish_at: Option<f64> },
    /// BP_FoodConsumable: use (authority, @43..@409): OffsetHealth(20), OffsetStamina(10), RandomIntegerInRange(0, 1)
    /// (a sound pick), K2_DestroyActor
    Food { eaten: bool },
    /// BP_FoodConsumableRespawn: use: OffsetHealth(HealthRegen), OffsetStamina(StamRegen), ReplicatedConsumptionCounter +
    /// 1 -> OnRep (after 3 s of game time): food hidden, Replenishing, RetriggerableDelay(RespawnTimer) -> shown again
    /// (@236..@599); CanInteract: not Replenishing, KingExclusive -> only a BP_FrontlineKillObjective
    FoodRespawn { replenishing: bool, back_at: Option<f64>, counter: u8 },
    /// BP_RandomEquipment: BeginPlay (authority): EquipmentClass spawned at the actor with a random skin / parts /
    /// pattern / emblem (GetRandomSkin + RandomInteger over the singleton's tables: the host's equipment system),
    /// bAllowCleanup false; the spawner destroys itself 1 s later (@1841..@2884)
    RandomEquipment { done_at: Option<f64> },
}

impl Pickup {
    pub fn of(chain: &[String], p: &Props) -> Option<Pickup> {
        let has = |n: &str| chain.iter().any(|c| c == n);
        Some(if has("BP_AmmoBox") {
            Pickup::AmmoBox
        } else if has("BP_RestockableEquipmentSpawn") {
            Pickup::Restockable { ammo: p.i("MaxAmmo").clamp(0, 255) as u8, replenishing: false, replenish_at: None }
        } else if has("BP_FoodConsumableRespawn") {
            Pickup::FoodRespawn { replenishing: false, back_at: None, counter: 0 }
        } else if has("BP_FoodConsumable") {
            Pickup::Food { eaten: false }
        } else if has("BP_RandomEquipment") {
            Pickup::RandomEquipment { done_at: None }
        } else {
            return None;
        })
    }

    pub fn begin_play(&mut self, id: ActorId, p: &Props, xf: &Xf, now: f64, authority: bool, ev: &mut Vec<WorldEvent>) {
        match self {
            Pickup::Restockable { ammo, .. } if authority => *ammo = p.i("MaxAmmo").clamp(0, 255) as u8,
            Pickup::RandomEquipment { done_at } if authority => {
                let class = p.obj("EquipmentClass");
                if !class.is_empty() {
                    ev.push(WorldEvent::SpawnEquipment { by: id, class, xf: *xf, customization: serde_json::json!({ "random": true, "bAllowCleanup": false }) });
                }
                *done_at = Some(now + 1.0);
            }
            _ => {}
        }
    }

    pub fn can_interact(&self, p: &Props, c: &CharView) -> bool {
        match self {
            Pickup::AmmoBox => true,
            Pickup::Restockable { ammo, .. } => *ammo > 0 && c.right_hand_class != p.obj("Equipment"),
            Pickup::Food { eaten } => !eaten,
            Pickup::FoodRespawn { replenishing, .. } => !replenishing && (!p.b("KingExclusive") || c.is_kill_objective),
            Pickup::RandomEquipment { .. } => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn interact(&mut self, id: ActorId, p: &Props, c: &CharView, now: f64, rng: &mut CrtRand, ev: &mut Vec<WorldEvent>) {
        match self {
            Pickup::AmmoBox => ev.push(WorldEvent::RestockFromAmmoBox { char: c.id, ammo_box: id }),
            Pickup::Restockable { ammo, replenishing, replenish_at } => {
                *ammo = ammo.wrapping_sub(1);
                if !*replenishing {
                    *replenishing = true;
                    *replenish_at = Some(now + p.f("AmmoReplenishInterval"));
                }
                ev.push(WorldEvent::GiveEquipment { by: id, char: c.id, class: p.obj("Equipment") });
            }
            Pickup::Food { eaten } => {
                ev.push(WorldEvent::OffsetHealth { char: c.id, amount: 20 });
                ev.push(WorldEvent::OffsetStamina { char: c.id, amount: 10 });
                let _sound = range_i(rng, 0, 1); // the eat sound pick (@153), a rand() draw
                *eaten = true;
                ev.push(WorldEvent::Destroyed { actor: id });
            }
            Pickup::FoodRespawn { replenishing, back_at, counter } => {
                ev.push(WorldEvent::OffsetHealth { char: c.id, amount: p.i("HealthRegen") });
                ev.push(WorldEvent::OffsetStamina { char: c.id, amount: p.i("StamRegen") });
                *counter = counter.wrapping_add(1);
                if now > 3.0 {
                    *replenishing = true;
                    *back_at = Some(now + p.f("RespawnTimer"));
                    ev.push(WorldEvent::Hidden { actor: id });
                }
            }
            Pickup::RandomEquipment { .. } => {}
        }
    }

    pub fn tick(&mut self, id: ActorId, p: &Props, now: f64, ev: &mut Vec<WorldEvent>) {
        match self {
            Pickup::Restockable { ammo, replenishing, replenish_at } => {
                if replenish_at.is_some_and(|t| now >= t) {
                    *ammo = ammo.wrapping_add(1);
                    if *ammo as i64 == p.i("MaxAmmo") {
                        *replenishing = false;
                        *replenish_at = None;
                    } else {
                        *replenish_at = Some(now + p.f("AmmoReplenishInterval"));
                    }
                }
            }
            Pickup::FoodRespawn { replenishing, back_at, .. } => {
                if back_at.is_some_and(|t| now >= t) {
                    *back_at = None;
                    *replenishing = false;
                    ev.push(WorldEvent::Shown { actor: id });
                }
            }
            Pickup::RandomEquipment { done_at } => {
                if done_at.is_some_and(|t| now >= t) {
                    *done_at = None;
                    ev.push(WorldEvent::Destroyed { actor: id });
                }
            }
            _ => {}
        }
    }
}

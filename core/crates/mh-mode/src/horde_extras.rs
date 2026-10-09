//! Horde's actors and player progression beyond the wave loop (rust-mode-ai r3), ported from the Blueprint bytecode
//! (scripts/kismet on the paks' packages; `Function@N` = the in-memory statement index): graves and revive
//! (BP_HordePlayerGrave), chests (BP_HordeChestBase : BP_BattleRoyaleChest : BP_BattleRoyaleBaseItemSpawn), the
//! placed purchasables (BP_HordePurchasable), the buy menu (BP_HordeGameState / BP_DemonHordeGamestate
//! BuyMenuEntries), the skill tree (BP_HordePlayerController SkillInfo / SkillPrerequisites, E_HordeSkill) and the
//! grave spawn of BP_HordeCharacter's OnKilled. Rule ids (data_gen/spec/rules.json): RULE_HRD_HordeGameState_
//! IsHordePurchaseAllowed, _PurchaseHordeBuyMenuEntry, _ModifyBuyMenuEntryPrice, _Spawn_Purchased_Actor_For_Player,
//! _Is_Valid_Wearable_For_Player, RULE_HRD_HordeGameMode_TriggerDefeat (the Graves term), RULE_HRD_HORDE_SPEC.
//!
//! Everything spatial stays with the host (line traces, spawn transforms, physics impulses, ammo counts, the
//! equipment a character holds): the host passes what the Blueprint read and applies what it spawned, through the
//! returned records / events. Floats are binary32 (Kismet float).

use crate::game_mode::{CtrlId, GameMode, ModeExt};
use crate::kismet_rand::{frand, random_integer};
use mordhau_core::ue::FVector;
use serde::{Deserialize, Serialize};

// ---- records (the data_hrd_br header, godot/tools/golden/mode.gd _horde_extras) ------------------------------------

/// BP_HordePlayerGrave class defaults (RevivePeriod 60, AutoReviveTime 30, MaxInteractionHoldTime 1.5)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraveDef {
    pub revive_period: f32,
    pub auto_revive_time: f32,
    pub max_interaction_hold_time: f32,
}

/// one TMap<UClass*, float> entry of a chest's item lists, with the casts SpawnContents makes on the spawned actor
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChanceItem {
    pub class: String,
    pub chance: f32,
    /// "equipment" (MordhauEquipment), "wearable" (BP_WearablePickup_C) or "other"
    pub kind: String,
}

/// a chest class's defaults (BP_HordeChestBase Cost 20 / RespawnTime 5; Tier1-3 Cost 300 / 1000 / 1750)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChestDef {
    pub class: String,
    pub cost: i64,
    pub respawn_time: f32,
    pub item_list: Vec<ChanceItem>,
    pub second_item_list: Vec<ChanceItem>,
    pub third_item_list: Vec<ChanceItem>,
    pub original_second_item_list: Vec<ChanceItem>,
    pub original_third_item_list: Vec<ChanceItem>,
}

/// a BP_HordePurchasable class's defaults
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PurchasableDef {
    pub class: String,
    pub cost: i64,
    pub purchasable_class: String,
    pub is_ammo_restock: bool,
    pub ammo_restock_amount: i64,
}

/// STRUCT_HordeBuyMenuEntry (BuyMenuEntries value)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BuyMenuEntry {
    pub id: i64,
    pub purchased_actor: String,
    pub base_cost: i64,
    pub is_ammo_restock: bool,
    pub ammo_restock_amount: i64,
    /// the PurchasedActor class's casts: "equipment" / "wearable" (BP_WearablePickup_C) / "other"
    #[serde(default)]
    pub kind: String,
}

/// STRUCT_HordeSkillInfo (SkillInfo value); `skill` = the E_HordeSkill enumerator name
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillInfo {
    pub skill: String,
    pub name: String,
    pub ultimate: bool,
    pub percent_a: f32,
    pub percent_b: f32,
    pub integer_a: i64,
    pub time_a: f32,
}

/// a BP_DemonHordeGamestate StageSettings row (STRUCT_DemonHordeStageSettings; the CDO holds a placeholder stage 0,
/// the maps' placed game state overrides it: the host passes the map's rows)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DemonStage {
    pub stage: i64,
    pub max_total_enemies: i64,
    pub wave_spawn_interval: f32,
    pub stage_spawn_delay: f32,
    pub skill_points_granted: i64,
    /// ActorArray length (the stage's objective actors)
    pub objectives: i64,
    /// HordeEnemies: EnemyDatabase key -> weight, in map order
    pub horde_enemies: Vec<(String, f32)>,
}

/// BP_DemonHordeGamestate class defaults and curves
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DemonData {
    pub stages: Vec<DemonStage>,
    pub stage_activation_delay: f32,
    /// DifficultyScaling (FC_DemonHordeDifficultyScaling) at alive-player counts 0, 1, 2, ...
    pub difficulty_scaling: Vec<f32>,
    /// FC_DemonHordeJIPCoins at float(byte(CurrentStage - 1)) / StageCount: [StageCount - 1][CurrentStage]
    pub jip_coins: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HordeExtras {
    pub grave: GraveDef,
    pub chests: Vec<ChestDef>,
    pub purchasables: Vec<PurchasableDef>,
    pub buy_menu: Vec<BuyMenuEntry>,
    pub di_buy_menu: Vec<BuyMenuEntry>,
    pub skills: Vec<SkillInfo>,
    /// SkillPrerequisites: skill -> prerequisite (enumerator names)
    pub skill_prerequisites: Vec<(String, String)>,
    /// BP_HordePlayerController MerchantPurchasables (class names)
    pub merchant_purchasables: Vec<String>,
    pub demon: DemonData,
}

/// E_HordeSkill: the enumerators in UEnum::Names order = their byte values (E_HordeSkill.uexp, the Names array at
/// export offset +1703: NewEnumerator2 = 1, NewEnumerator1 = 2, ..., NewEnumerator4 = 53 "Total", E_MAX = 54)
pub const E_HORDE_SKILL: [&str; 54] = [
    "NewEnumerator0", "NewEnumerator2", "NewEnumerator1", "NewEnumerator3", "NewEnumerator5", "NewEnumerator6",
    "NewEnumerator7", "NewEnumerator8", "NewEnumerator9", "NewEnumerator10", "NewEnumerator12", "NewEnumerator13",
    "NewEnumerator14", "NewEnumerator15", "NewEnumerator16", "NewEnumerator17", "NewEnumerator18", "NewEnumerator19",
    "NewEnumerator20", "NewEnumerator21", "NewEnumerator22", "NewEnumerator23", "NewEnumerator24", "NewEnumerator25",
    "NewEnumerator26", "NewEnumerator27", "NewEnumerator28", "NewEnumerator29", "NewEnumerator30", "NewEnumerator31",
    "NewEnumerator32", "NewEnumerator33", "NewEnumerator34", "NewEnumerator35", "NewEnumerator36", "NewEnumerator37",
    "NewEnumerator38", "NewEnumerator39", "NewEnumerator40", "NewEnumerator41", "NewEnumerator42", "NewEnumerator43",
    "NewEnumerator44", "NewEnumerator45", "NewEnumerator46", "NewEnumerator47", "NewEnumerator48", "NewEnumerator49",
    "NewEnumerator50", "NewEnumerator51", "NewEnumerator52", "NewEnumerator53", "NewEnumerator54", "NewEnumerator4",
];

/// the byte value of an E_HordeSkill enumerator name
pub fn skill_value(name: &str) -> Option<u8> {
    E_HORDE_SKILL.iter().position(|n| *n == name).map(|i| i as u8)
}

/// ReplicatedSkills length: the player state's construction loop 0..52 (one byte per skill below Total = 53)
pub const SKILL_COUNT: usize = 53;
/// E_HordeSkill LastChance (UpgradeSkill@968: Skill == 3 at level 0 -> the pawn's bHasLastChance)
pub const SKILL_LAST_CHANCE: u8 = 3;
/// GetMerchantDuplicateChance@194: GetScaledSkillLevelParams(39, 0) (NewEnumerator41)
pub const SKILL_MERCHANT: u8 = 39;
/// UpgradeSkill@759: level < 5
pub const SKILL_MAX_LEVEL: u8 = 5;

// ---- state ------------------------------------------------------------------------------------------------------

pub type GraveId = u64;

/// a BP_HordePlayerGrave
#[derive(Clone, Debug, PartialEq)]
pub struct Grave {
    pub id: GraveId,
    /// AssociatedPlayerState's owner
    pub owner: CtrlId,
    pub location: FVector,
    pub revive_period: f32,
    pub being_interacted_with: bool,
    pub expired: bool,
    pub will_auto_revive_at: f32,
    pub replicated_life_span_seconds: u8,
    /// StartAutoReviveTimer's K2_SetTimerDelegate(AutoRevive, AutoReviveTime) due time
    pub auto_revive_at: Option<f32>,
}

/// a placed chest
#[derive(Clone, Debug)]
pub struct Chest {
    pub def: ChestDef,
    pub destroyed: bool,
    /// RespawnChest's Delay(RespawnTime) due time
    pub respawn_at: Option<f32>,
    /// SetLifeSpan(2): gone at this time
    pub gone_at: Option<f32>,
    pub gone: bool,
    pub item_list: Vec<ChanceItem>,
    pub second_item_list: Vec<ChanceItem>,
    pub third_item_list: Vec<ChanceItem>,
    pub original_first_item_list: Vec<ChanceItem>,
    pub original_second_item_list: Vec<ChanceItem>,
    pub original_third_item_list: Vec<ChanceItem>,
    /// the BP_HordeGameState Purchasables slot (ReceiveBeginPlay@30 AddUnique)
    pub available: bool,
}

/// one SpawnRandomItem result of SpawnContents
#[derive(Clone, Debug, PartialEq)]
pub struct ChestSpawn {
    pub class: String,
    pub kind: String,
    /// SpawnPoint1..3 (TransformToUse[TransformIdx])
    pub spawn_point: usize,
    /// the RandomFloatInRange(-180, 180) rolls of the impulse (AddImpulse) and angular impulse
    /// (AddAngularImpulseInDegrees) directions: equipment (a skeletal PhysicsProxy root, UNCONFIRMED always) and
    /// wearables roll both, other actors none
    pub impulse_yaw: Option<(f32, f32)>,
}

/// what a placed purchasable dispensed (the host applies it to the buyer's pawn)
#[derive(Clone, Debug, PartialEq)]
pub enum Purchase {
    /// FindEquipmentToRestock([PurchasableClass]) gets SetAmmo(BMin(byte(ammo + byte(amount)), max)) (restock_ammo)
    AmmoRestock { class: String, amount: i64 },
    /// SpawnItem: PurchasableClass at the purchasable's transform (GetRandomCustomization + the buyer's emblem)
    SpawnItem { class: String },
}

/// what the buy menu dispensed
#[derive(Clone, Debug, PartialEq)]
pub enum BuyOutcome {
    AmmoRestock { class: String, amount: i64 },
    /// Spawn Purchased Actor For Player, twice when GetMerchantDuplicateChance hit (the bonus re-runs the spawn)
    Spawn { class: String, count: u8 },
}

/// the buyer's pawn facts IsHordePurchaseAllowed reads (host side)
#[derive(Clone, Copy, Debug, Default)]
pub struct BuyerView {
    /// GetControlledMordhauCharacter is valid
    pub has_character: bool,
    /// some Equipment[i] of exactly the entry's class with GetAmmo < GetCurrentMaxAmmo (@1122-@1956)
    pub can_restock: bool,
    /// Is Valid Wearable For Player (wearable entries)
    pub wearable_ok: bool,
}

/// SetAmmo(BMin(Add_ByteByte(GetAmmo, Conv_IntToByte(AmmoRestockAmount)), GetCurrentMaxAmmo)): the byte add wraps
pub fn restock_ammo(ammo: u8, amount: i64, max: u8) -> u8 {
    ammo.wrapping_add(amount as u8).min(max)
}

/// UKismetMathLibrary::RandomFloatInRange = Min + (Max - Min) * FRand (UE 4.26 FMath::FRandRange, UNCONFIRMED)
pub fn random_float_in_range(rng: &mut mordhau_core::ue::CrtRand, min: f32, max: f32) -> f32 {
    min + (max - min) * frand(rng)
}

/// UKismetMathLibrary::RandomBoolWithWeight: Weight <= 0 -> false without a draw, else Weight >= FRandRange(0, 1)
/// (UE 4.26 source, UNCONFIRMED)
pub fn random_bool_with_weight(rng: &mut mordhau_core::ue::CrtRand, w: f32) -> bool {
    if w <= 0.0 {
        return false;
    }
    w >= random_float_in_range(rng, 0.0, 1.0)
}

/// BP_BattleRoyaleBaseItemSpawn RenormalizeChances@0-@1446: TotalChances = sum |chance|, MaxChance = max |chance|
/// (both locals, zero each call); MaxChance > 0 -> every chance = chance / TotalChances (in place, Map_Add)
pub fn renormalize(list: &mut [ChanceItem]) {
    if list.is_empty() {
        return;
    }
    let (mut total, mut max) = (0.0f32, 0.0f32);
    for it in list.iter() {
        total = it.chance.abs() + total;
        max = max.max(it.chance.abs());
    }
    if max > 0.0 {
        for it in list.iter_mut() {
            it.chance /= total;
        }
    }
}

/// GetRandomItem@0-@970: RenormalizeChances, r = RandomFloat, walk the keys accumulating chance: BestId = the index,
/// break once Current >= r (no break: the last index); an empty list -> None (no draw is skipped: RandomFloat runs
/// even for an empty list)
pub fn get_random_item(rng: &mut mordhau_core::ue::CrtRand, list: &mut [ChanceItem]) -> Option<usize> {
    renormalize(list);
    let r = frand(rng);
    let mut cur = 0.0f32;
    let mut best = 0usize;
    for (i, it) in list.iter().enumerate() {
        best = i;
        cur = it.chance + cur;
        if cur >= r {
            break;
        }
    }
    if list.is_empty() {
        None
    } else {
        Some(best)
    }
}

impl Chest {
    pub fn new(def: &ChestDef) -> Chest {
        Chest {
            def: def.clone(),
            destroyed: false,
            respawn_at: None,
            gone_at: None,
            gone: false,
            item_list: def.item_list.clone(),
            second_item_list: def.second_item_list.clone(),
            third_item_list: def.third_item_list.clone(),
            original_first_item_list: Vec::new(),
            original_second_item_list: def.original_second_item_list.clone(),
            original_third_item_list: def.original_third_item_list.clone(),
            available: false,
        }
    }

    /// BP_BattleRoyaleChest SpawnContents@0-@3153. OriginalFirstItemList = ItemList; the Second / Third originals
    /// are self-assigned (@67 / @94, a Blueprint bug: they keep their class default, empty on every chest class), so
    /// the restore at the end (@677-@731) empties Second / Third after the first opening. For i in 0..=2: ItemList =
    /// [ItemList, SecondItemList, ThirdItemList][i] (the switch @580-@662, @3097 / @3125), SpawnRandomItem at
    /// TransformToUse[TransformIdx]; a valid spawn: TransformIdx + 1, the impulse rolls, and its class removed from
    /// Second and Third (@2918-@3036).
    pub fn spawn_contents(&mut self, rng: &mut mordhau_core::ue::CrtRand) -> Vec<ChestSpawn> {
        self.original_first_item_list = self.item_list.clone();
        let mut out = Vec::new();
        let mut idx = 0usize;
        for i in 0..=2 {
            match i {
                1 => self.item_list = self.second_item_list.clone(),
                2 => self.item_list = self.third_item_list.clone(),
                _ => {}
            }
            // SpawnRandomItem@0: GetRandomItem, IsValidClass -> spawn (equipment: ArmoryTransformOffset composed and
            // pitched 90; GetRandomCustomization(true): its own randomness UNCONFIRMED, not drawn here)
            let Some(k) = get_random_item(rng, &mut self.item_list) else { continue };
            let it = self.item_list[k].clone();
            let impulse_yaw = match it.kind.as_str() {
                "equipment" | "wearable" => {
                    let a = random_float_in_range(rng, -180.0, 180.0); // @1464 / @2228
                    let b = random_float_in_range(rng, -180.0, 180.0); // @1789 / @2575
                    Some((a, b))
                }
                _ => None,
            };
            out.push(ChestSpawn { class: it.class.clone(), kind: it.kind.clone(), spawn_point: idx, impulse_yaw });
            idx += 1;
            self.second_item_list.retain(|x| x.class != it.class);
            self.third_item_list.retain(|x| x.class != it.class);
        }
        self.item_list = self.original_first_item_list.clone();
        self.second_item_list = self.original_second_item_list.clone();
        self.third_item_list = self.original_third_item_list.clone();
        out
    }
}

impl GameMode {
    pub fn horde_extras(&self) -> &HordeExtras {
        &self.horde_data().extras
    }

    // ---- graves ---------------------------------------------------------------------------------------------------

    /// BP_HordeCharacter OnKilled (ubergraph @5251-@5583), before the parent OnKilled: the controller (vehicle
    /// included) is a BP_HordePlayerController with a BP_HordePlayerState -> SaveEquipmentToPlayerState; NewHorde
    /// (true on the CDO) and a BP_HordeGameState: a Demon Invasion game state always (@2912), classic only while
    /// !bAllowSpawning (@3965) -> LineTraceSingleForObjects(actor location, location - (0, 0, 10000), WorldStatic):
    /// `ground` = the hit location (None: no hit, no grave). The grave is spawned there with AssociatedPlayerState.
    /// The host calls this when a player's pawn dies.
    pub fn horde_character_killed(&mut self, c: CtrlId, ground: Option<FVector>) -> Option<GraveId> {
        if self.ctrls[c].is_bot {
            return None; // not a BP_HordePlayerController
        }
        let who = self.name(c);
        self.ev_args("save_equipment_to_player_state", vec![("who", who.into())]); // @5457
        let demon = self.horde().demon.is_some();
        if !demon && self.allow_spawning {
            return None;
        }
        let at = ground?;
        Some(self.spawn_grave(c, at))
    }

    /// a grave's BeginPlay (@1644): HordeGameMode.Graves.Add(self); OnRep_AssociatedPlayerState: the player state's Grave
    pub fn spawn_grave(&mut self, owner: CtrlId, at: FVector) -> GraveId {
        let g = self.horde_extras().grave.clone();
        let h = self.horde_mut();
        h.next_grave_id += 1;
        let id = h.next_grave_id;
        h.graves.push(Grave {
            id,
            owner,
            location: at,
            revive_period: g.revive_period,
            being_interacted_with: false,
            expired: false,
            will_auto_revive_at: 0.0,
            replicated_life_span_seconds: 0,
            auto_revive_at: None,
        });
        let who = self.name(owner);
        self.ev_args("grave_spawned", vec![("who", who.into()), ("grave", (id as i64).into()), ("xf", crate::event::Arg::V([at.x, at.y, at.z]))]);
        id
    }

    fn grave_index(&self, id: GraveId) -> Option<usize> {
        self.horde().graves.iter().position(|g| g.id == id)
    }

    /// K2_DestroyActor -> ReceiveDestroyed@1823: the owner's HUD hides the respawn timer, Graves.RemoveItem(self)
    pub fn destroy_grave(&mut self, id: GraveId) {
        if let Some(i) = self.grave_index(id) {
            let g = self.horde_mut().graves.remove(i);
            let who = self.name(g.owner);
            self.ev_args("grave_destroyed", vec![("who", who.into()), ("grave", (id as i64).into())]);
        }
    }

    /// OnInteractionMaintained@1232 (a player holding the revive): BeingInteractedWith = true
    pub fn grave_interaction_maintained(&mut self, id: GraveId) {
        if let Some(i) = self.grave_index(id) {
            self.horde_mut().graves[i].being_interacted_with = true;
        }
    }

    /// OnHeldInteractionStart@1183 (the hold of MaxInteractionHoldTime completed; `reviver` = the reviving
    /// character's controller): the reviver's MordhauPlayerController gets GiveClientScoreBP(2, 250) (@433); the dead
    /// player: UnpossessAndDestroyPawn(c, false), RestartPlayerAtTransform(c, grave location + (0, 0, 100)) (@488-@717);
    /// then the grave is destroyed (@473)
    pub fn grave_revive(&mut self, id: GraveId, reviver: Option<CtrlId>) {
        let Some(i) = self.grave_index(id) else { return };
        let g = self.horde().graves[i].clone();
        if let Some(r) = reviver.filter(|&r| !self.ctrls[r].is_bot) {
            let who = self.name(r);
            self.ev_args("client_score", vec![("who", who.into()), ("reason", 2i64.into()), ("score", 250i64.into())]);
        }
        self.unpossess_and_destroy_pawn(g.owner, false);
        self.restart_player_at(g.owner, g.location + FVector::new(0.0, 0.0, 100.0));
        self.destroy_grave(id);
    }

    /// ForceRespawn (ubergraph @2329 -> @15): the revive path with no reviver (no score)
    pub fn grave_force_respawn(&mut self, id: GraveId) {
        self.grave_revive(id, None);
    }

    /// AGameModeBase::RestartPlayerAtTransform (engine, UNCONFIRMED: spawns and possesses the default pawn at the
    /// transform, no spawn queue)
    pub fn restart_player_at(&mut self, c: CtrlId, at: FVector) {
        self.ctrls[c].location = at;
        self.ctrls[c].has_pawn = true;
        self.ctrls[c].alive = true;
        self.ctrls[c].damage_history.clear();
        let who = self.name(c);
        self.ev_args("restart_at", vec![("who", who.into()), ("xf", crate::event::Arg::V([at.x, at.y, at.z]))]);
    }

    /// every grave's ReceiveTick@988 (authority; tick order vs the game mode UNCONFIRMED: after it) and its
    /// AutoRevive timer: !BeingInteractedWith -> RevivePeriod -= dt (@1109); BeingInteractedWith = false (@1097);
    /// ReplicatedLifeSpanSeconds = byte(FCeil(RevivePeriod)) (@782); RevivePeriod <= 0 -> Expired = true, OnRep_Expired
    /// (@962-@913): WillAutoReviveAtTime = GetTimeSeconds + AutoReviveTime, not interactable, hidden, tick off,
    /// StartAutoReviveTimer (@1563: AutoRevive in AutoReviveTime). AutoRevive@1259: UnpossessAndDestroyPawn(owner,
    /// true) (a restart through the queue), the grave destroyed. An expired grave stays in Graves until then.
    pub(crate) fn horde_graves_tick(&mut self, dt: f32) {
        let now = self.now as f32;
        let auto = self.horde_extras().grave.auto_revive_time;
        let ids: Vec<GraveId> = self.horde().graves.iter().map(|g| g.id).collect();
        for id in ids {
            let Some(i) = self.grave_index(id) else { continue };
            let mut expired_now = false;
            {
                let g = &mut self.horde_mut().graves[i];
                if !g.expired {
                    if !g.being_interacted_with {
                        g.revive_period -= dt;
                    }
                    g.being_interacted_with = false;
                    g.replicated_life_span_seconds = (g.revive_period.ceil() as i32) as u8;
                    if g.revive_period <= 0.0 {
                        g.expired = true;
                        g.will_auto_revive_at = now + auto;
                        g.auto_revive_at = Some(now + auto);
                        expired_now = true;
                    }
                }
            }
            if expired_now {
                let who = self.name(self.horde().graves[i].owner);
                self.ev_args("grave_expired", vec![("who", who.into()), ("grave", (id as i64).into())]);
            }
            let g = self.horde().graves[i].clone();
            if let Some(t) = g.auto_revive_at {
                if !expired_now && now >= t {
                    self.unpossess_and_destroy_pawn(g.owner, true);
                    self.destroy_grave(id);
                }
            }
        }
    }

    // ---- chests -----------------------------------------------------------------------------------------------------

    /// a placed chest of class `class` (ReceiveBeginPlay@272 -> @30: SetAvailability(false), Purchasables.AddUnique)
    pub fn add_chest(&mut self, class: &str) -> Option<usize> {
        let def = self.horde_extras().chests.iter().find(|c| c.class == class)?.clone();
        let h = self.horde_mut();
        h.chests.push(Chest::new(&def));
        Some(h.chests.len() - 1)
    }

    /// BP_HordeChestBase OnInteractionStart@287: the character's BP_HordePlayerState; Coins >= GetDiscountedPrice(Cost)
    /// (BP_HordePlayerState.GetDiscountedPrice -> BP_HordePlayerController.GetDiscountedPrice = BasePrice: no
    /// discount) -> Coins -= price (@715, no clamp), BreakChest(the BP_HordePlayerController, else null @944)
    pub fn chest_interact(&mut self, chest: usize, buyer: CtrlId) -> Vec<ChestSpawn> {
        if self.horde().chests[chest].gone {
            return Vec::new();
        }
        let price = self.horde().chests[chest].def.cost;
        let Some(p) = self.horde_mut().players.get_mut(&buyer) else { return Vec::new() };
        if p.coins < price {
            return Vec::new();
        }
        p.coins -= price;
        self.break_chest(chest, Some(buyer))
    }

    /// BP_BattleRoyaleChest BreakChest@0: already destroyed -> nothing; else SpawnContents(DestroyerPlayerController),
    /// ChestDestroyed = true, OnRep_Destroyed (@253-@370: authority, RespawnTime > 0 -> RespawnChest = Delay(RespawnTime)
    /// then ChestDestroyed = false (@215, @41); else SetLifeSpan(2)). ReceiveAnyDamage (@96 / @199) breaks it too.
    pub fn break_chest(&mut self, chest: usize, by: Option<CtrlId>) -> Vec<ChestSpawn> {
        if self.horde().chests[chest].destroyed {
            return Vec::new();
        }
        let now = self.now as f32;
        let mut ch = self.horde().chests[chest].clone();
        let out = ch.spawn_contents(&mut self.rng);
        ch.destroyed = true;
        if ch.def.respawn_time > 0.0 {
            ch.respawn_at = Some(now + ch.def.respawn_time);
        } else {
            ch.gone_at = Some(now + 2.0);
        }
        self.horde_mut().chests[chest] = ch;
        let who = by.map(|b| self.name(b)).unwrap_or_default();
        for s in &out {
            self.ev_args(
                "chest_item",
                vec![("chest", (chest as i64).into()), ("by", who.clone().into()), ("class", s.class.clone().into()), ("point", (s.spawn_point as i64).into())],
            );
        }
        out
    }

    pub(crate) fn horde_chests_tick(&mut self) {
        let now = self.now as f32;
        for ch in self.horde_mut().chests.iter_mut() {
            if let Some(t) = ch.respawn_at {
                if now >= t {
                    ch.respawn_at = None;
                    ch.destroyed = false; // @41 ChestDestroyed = false, OnRep_Destroyed (interactable again)
                }
            }
            if let Some(t) = ch.gone_at {
                if now >= t {
                    ch.gone = true;
                }
            }
        }
    }

    // ---- placed purchasables ----------------------------------------------------------------------------------------

    /// BP_HordePurchasable OnInteractionStart@245: the character's BP_HordePlayerState and its owner a
    /// BP_HordePlayerController; Coins >= GetDiscountedPrice(Cost, class) (= Cost) -> Coins -= Cost (@836, no clamp);
    /// then only for a BP_BattleRoyaleCharacter (`br_character`, @958): IsAmmoRestock -> the restock (@1052-@1474)
    /// and the controller's PurchaseTrigger + 1 (@1669, whether or not a restock target existed); else SpawnItem(@1853)
    pub fn purchasable_interact(&mut self, def: &PurchasableDef, buyer: CtrlId, br_character: bool) -> Option<Purchase> {
        if self.ctrls[buyer].is_bot {
            return None;
        }
        let p = self.horde_mut().players.get_mut(&buyer)?;
        if p.coins < def.cost {
            return None;
        }
        p.coins -= def.cost;
        if !br_character {
            return None;
        }
        if def.is_ammo_restock {
            p.purchase_trigger = p.purchase_trigger.wrapping_add(1);
            return Some(Purchase::AmmoRestock { class: def.purchasable_class.clone(), amount: def.ammo_restock_amount });
        }
        if def.purchasable_class.is_empty() {
            return None; // SpawnItem@5: IsValidClass(PurchasableClass)
        }
        Some(Purchase::SpawnItem { class: def.purchasable_class.clone() })
    }

    // ---- the buy menu -----------------------------------------------------------------------------------------------

    /// the active game state's BuyMenuEntries (BP_DemonHordeGamestate overrides the map's 75 entries)
    pub fn buy_menu(&self) -> &[BuyMenuEntry] {
        let x = self.horde_extras();
        if self.horde().demon.is_some() {
            &x.di_buy_menu
        } else {
            &x.buy_menu
        }
    }

    /// ModifyBuyMenuEntryPrice@0: Abs(BaseCost) of the entry, Abs(0) when the id is missing
    pub fn buy_menu_price(&self, id: i64) -> i64 {
        self.buy_menu().iter().find(|e| e.id == id).map(|e| e.base_cost.abs()).unwrap_or(0)
    }

    /// IsHordePurchaseAllowed@0-@2462: a BP_HordePlayerState, the entry exists, Coins >= GetDiscountedPrice(BaseCost,
    /// MerchantClassLinkMap class) (= BaseCost) and a controlled character; equipment entries: an ammo restock needs
    /// a restock target (@1122-@1956), else allowed (@2348); BP_WearablePickup entries: Is Valid Wearable For Player;
    /// anything else allowed (@2249 "what are you trying to sell??")
    pub fn is_horde_purchase_allowed(&self, c: CtrlId, id: i64, v: BuyerView) -> bool {
        let Some(p) = self.horde().players.get(&c) else { return false };
        let Some(e) = self.buy_menu().iter().find(|e| e.id == id) else { return false };
        if !(p.coins >= e.base_cost && v.has_character) {
            return false;
        }
        match e.kind.as_str() {
            "equipment" => !e.is_ammo_restock || v.can_restock,
            "wearable" => v.wearable_ok,
            _ => true,
        }
    }

    /// PurchaseHordeBuyMenuEntry@0-@1778 (authority): IsHordePurchaseAllowed; Coins = Clamp(Coins - price, 0, INT_MAX)
    /// (@256-@371); an ammo restock entry -> the restock and PurchaseTrigger + 1 (@1289); else
    /// GetMerchantDuplicateChance(PurchasedActor) and Spawn Purchased Actor For Player (twice on the bonus, @1605-@1736)
    pub fn purchase_buy_menu_entry(&mut self, c: CtrlId, id: i64, v: BuyerView) -> Option<BuyOutcome> {
        if !self.is_horde_purchase_allowed(c, id, v) {
            return None;
        }
        let e = self.buy_menu().iter().find(|e| e.id == id)?.clone();
        let price = self.buy_menu_price(id);
        let p = self.horde_mut().players.get_mut(&c)?;
        p.coins = (p.coins - price).clamp(0, i32::MAX as i64);
        if e.is_ammo_restock {
            p.purchase_trigger = p.purchase_trigger.wrapping_add(1);
            return Some(BuyOutcome::AmmoRestock { class: e.purchased_actor, amount: e.ammo_restock_amount });
        }
        let bonus = self.merchant_duplicate_chance(c, &e.purchased_actor);
        Some(BuyOutcome::Spawn { class: e.purchased_actor, count: if bonus { 2 } else { 1 } })
    }

    /// BP_HordePlayerController GetMerchantDuplicateChance@0-@666: for each MerchantPurchasables entry, in order:
    /// PercentA of GetScaledSkillLevelParams(39, 0), RandomBoolWithWeight(PercentA) (a draw only with the skill),
    /// (class == entry || class is a child of it, UNCONFIRMED: by name here) && the roll -> true
    pub fn merchant_duplicate_chance(&mut self, c: CtrlId, class: &str) -> bool {
        let list = self.horde_extras().merchant_purchasables.clone();
        for m in list {
            let (pa, _, _, _) = self.scaled_skill_level_params(c, SKILL_MERCHANT, 0);
            let roll = random_bool_with_weight(&mut self.rng, pa);
            if m == class && roll {
                return true;
            }
        }
        false
    }

    // ---- skills -----------------------------------------------------------------------------------------------------

    fn skill_info(&self, skill: u8) -> Option<SkillInfo> {
        self.horde_extras().skills.iter().find(|s| skill_value(&s.skill) == Some(skill)).cloned()
    }

    /// GetSkillLevel: ReplicatedSkills[Skill], 0 for a missing player state or an invalid index
    pub fn skill_level(&self, c: CtrlId, skill: u8) -> u8 {
        self.horde().players.get(&c).and_then(|p| p.skills.get(skill as usize).copied()).unwrap_or(0)
    }

    /// GetScaledSkillLevelParams(Skill, Bias)@0-@1193: n = level + Bias > 0 -> (PercentA * float(n), PercentB *
    /// float(n), IntegerA * n, TimeA * float(n)) of SkillInfo[Skill]; else zeros
    pub fn scaled_skill_level_params(&self, c: CtrlId, skill: u8, bias: i64) -> (f32, f32, i64, f32) {
        let Some(p) = self.horde().players.get(&c) else { return (0.0, 0.0, 0, 0.0) };
        let Some(&lv) = p.skills.get(skill as usize) else { return (0.0, 0.0, 0, 0.0) };
        let n = bias + lv as i64;
        if n <= 0 {
            return (0.0, 0.0, 0, 0.0);
        }
        let i = self.skill_info(skill).unwrap_or_default();
        (i.percent_a * n as f32, i.percent_b * n as f32, i.integer_a * n, i.time_a * n as f32)
    }

    /// HasPrerequisite@0-@325: no SkillPrerequisites entry -> true; else level(prerequisite) > 0 || prerequisite == 0
    pub fn has_prerequisite(&self, c: CtrlId, skill: u8) -> bool {
        let pre = self
            .horde_extras()
            .skill_prerequisites
            .iter()
            .find(|(k, _)| skill_value(k) == Some(skill))
            .and_then(|(_, v)| skill_value(v));
        match pre {
            None => true,
            Some(p) => self.skill_level(c, p) > 0 || p == 0,
        }
    }

    /// UpgradeSkill@0-@1653 (authority, a BP_HordePlayerState): a valid index and SkillPoints > 0, HasPrerequisite,
    /// not (SpecialSkill != 0 && SpecialSkill != Skill && SkillInfo[Skill].Ultimate) (one ultimate), level < 5;
    /// level 0 of LastChance (3) -> the pawn's bHasLastChance = true (@968-@1104); level + 1, SkillsUpdated,
    /// SkillPoints - 1 (byte)
    pub fn upgrade_skill(&mut self, c: CtrlId, skill: u8) -> bool {
        let Some(p) = self.horde().players.get(&c) else { return false };
        if (skill as usize) >= p.skills.len() || p.skill_points <= 0 || !self.has_prerequisite(c, skill) {
            return false;
        }
        let ult = self.skill_info(skill).map(|i| i.ultimate).unwrap_or(false);
        let p = &self.horde().players[&c];
        if p.special_skill != 0 && p.special_skill != skill && ult {
            return false;
        }
        let lv = p.skills[skill as usize];
        if lv >= SKILL_MAX_LEVEL {
            return false;
        }
        if lv == 0 && skill == SKILL_LAST_CHANCE && self.ctrls[c].has_pawn {
            let who = self.name(c);
            self.ev_args("has_last_chance", vec![("who", who.into())]);
        }
        self.horde_mut().players.get_mut(&c).unwrap().skills[skill as usize] = lv + 1;
        self.skills_updated(c);
        let p = self.horde_mut().players.get_mut(&c).unwrap();
        p.skill_points = (p.skill_points - 1) & 0xff;
        true
    }

    /// SkillsUpdated@0-@1440: SpecialSkill = 0, then the first index with level > 0 whose SkillInfo entry is Ultimate
    /// (GetValidValue(E_HordeSkill, i)); the pawn copies ReplicatedSkills (@1005)
    pub fn skills_updated(&mut self, c: CtrlId) {
        let skills = self.horde().players.get(&c).map(|p| p.skills.clone()).unwrap_or_default();
        let mut special = 0u8;
        for (i, &lv) in skills.iter().enumerate() {
            if lv > 0 && self.skill_info(i as u8).map(|s| s.ultimate).unwrap_or(false) {
                special = i as u8;
                break;
            }
        }
        if let Some(p) = self.horde_mut().players.get_mut(&c) {
            p.special_skill = special;
        }
    }

    /// RandomInteger over a list, for hosts (Array_Random: RandomIntegerInRange(0, Num - 1), UNCONFIRMED)
    pub fn array_random(&mut self, n: usize) -> Option<usize> {
        if n == 0 {
            return None;
        }
        Some(random_integer(&mut self.rng, n as i64) as usize)
    }

    pub(crate) fn horde_actors_tick(&mut self, dt: f64) {
        if !matches!(self.ext, ModeExt::Horde(_)) {
            return;
        }
        self.horde_graves_tick(dt as f32);
        self.horde_chests_tick();
        self.demon_tick();
    }
}

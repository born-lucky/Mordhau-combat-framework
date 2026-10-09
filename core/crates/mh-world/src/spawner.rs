//! Spawners: BP_EquipmentSpawner (weapon racks / pickups: Blueprints/Equipment/BP_EquipmentSpawner and its presets)
//! and BP_VehicleSpawner with BP_HorseSpawner / BP_BallistaSpawner. Bytecode: state/world_kismet/BP_EquipmentSpawner.txt,
//! BP_VehicleSpawner.txt, BP_HorseSpawner.txt, BP_BallistaSpawner.txt.
//! Equipment goes through the host's equipment system (`WorldEvent::SpawnEquipment`, the class + AssignCustomization
//! values); horses / ballistas are rust-character's vehicles (`WorldEvent::Spawn`).
//!
//! Random numbers: RandomFloatInRange / RandomIntegerInRange (UKismetMathLibrary::execRandomFloatInRange /
//! execRandomIntegerInRange) and the K2_SetTimerDelegate start variance draw from FMath::Rand = the UCRT rand(),
//! `CrtRand`. That stream is seeded once in FEngineLoop::PreInitPreStartupScreen (srand call at rva 0x7ce0dd) with
//! FPlatformTime::Cycles() unless -FIXEDSEED (then 0), and the UCRT keeps it per thread. So on the game thread every
//! other rand() consumer between two spawner draws advances it too: 388 call sites in 146 functions
//! (state/rand_callers.txt, from scripts/rand_callers.py). Among them are the bots' UBotBehaviorProfile::Randomize,
//! UBTTask_MeleeAttack / UBTTask_Wait, USoundNodeRandom::ChooseNodeIndex, UCharacterVoiceComponent::OnTakeDamage,
//! UCharacterMeshComponent::AddWound, AMordhauGameMode::OnKilled, the AMordhauEquipment mode / fire handlers, the
//! loading screen, UKismetArrayLibrary shuffle / random, and every Blueprint RandomFloat / RandomInteger node. A
//! cycle-seeded stream shared with sound and bot AI cannot be replayed exactly, so the host owns `World::rng` and
//! should share one CrtRand with its other game-thread consumers (the order is the host's tick order).

use crate::props::Props;
use crate::world::{ActorId, Queries, WorldEvent};
use mh_level::xf::Xf;
use serde_json::{json, Value};

/// AMordhauGameMode::BallistaRespawnTime / CatapultRespawnTime / HorseRespawnTime, all 30 in the ctor (rva 0x157a930,
/// AMordhauGameMode.cpp:492 / :493 / :494); a game mode Blueprint may override them (`World::game_mode_respawn`)
pub const DEFAULT_VEHICLE_RESPAWN: f64 = 30.0;

/// The engine's FMath::Rand stream: the UCRT rand() as mordhau-core models it (state * 214013 + 2531011,
/// (state >> 16) & 0x7fff; mordhau_core::ue::CrtRand, the one type mh-mode's bots share), so a host can hand mh-world
/// and mh-mode the same stream
pub type CrtRand = mordhau_core::ue::CrtRand;

/// FMath::FRand = Rand() * (1 / RAND_MAX) in f32 (FGenericPlatformMath::FRand rva 0x15da470)
pub fn frand(rng: &mut CrtRand) -> f64 {
    rng.frand((1.0f32 / 32767.0) as f64)
}
/// UKismetMathLibrary::RandomFloatInRange -> FMath::FRandRange(min, max) = min + (max - min) * FRand()
pub fn range_f(rng: &mut CrtRand, min: f64, max: f64) -> f64 {
    min + (max - min) * frand(rng)
}
/// UKismetMathLibrary::RandomIntegerInRange -> FMath::RandRange(min, max) = min + RandHelper(max - min + 1),
/// RandHelper(a) = a > 0 ? min(trunc(FRand() * a), a - 1) : 0
pub fn range_i(rng: &mut CrtRand, min: i64, max: i64) -> i64 {
    let a = max - min + 1;
    min + if a > 0 { ((frand(rng) * a as f64) as i64).min(a - 1) } else { 0 }
}

#[derive(Clone, Debug)]
pub struct EquipmentSpawner {
    pub respawning: bool,
    pub spawn_at: Option<f64>,
    pub next_check: Option<f64>,
    pub spawned: u32,
    /// the game mode's NoEquipmentSpawners (BP_MordhauGameMode variable; @141)
    pub disabled: bool,
}

impl EquipmentSpawner {
    pub fn new(_p: &Props) -> EquipmentSpawner {
        EquipmentSpawner { respawning: false, spawn_at: None, next_check: None, spawned: 0, disabled: false }
    }

    /// ReceiveBeginPlay @308: Delay 0.5, then (authority, a BP_MordhauGameMode without NoEquipmentSpawners, a valid
    /// Equipment class, @15..@207) SpawnEquipment and CheckEquipment on a looping 1 s timer started with
    /// K2_SetTimerDelegate(1.0, true, InitialStartDelay 1.0, variance 1.0) @254: first call after Time + InitialStartDelay
    /// + FRandRange(-variance, variance) (UE 4.26 UKismetSystemLibrary::K2_SetTimerDelegate)
    pub fn begin_play(&mut self, now: f64) {
        self.spawn_at = Some(now + 0.5);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tick(&mut self, id: ActorId, p: &Props, xf: &Xf, now: f64, q: &dyn Queries, rng: &mut CrtRand, ev: &mut Vec<WorldEvent>) {
        let class = p.obj("Equipment");
        if let Some(t) = self.spawn_at {
            if now >= t {
                self.spawn_at = None;
                let first = self.spawned == 0 && !self.respawning;
                if first && (self.disabled || class.is_empty()) {
                    return;
                }
                // SpawnEquipment @621: Respawning = false; Equipment at the spawner's transform, AssignCustomization @787
                self.respawning = false;
                self.spawned += 1;
                let customization = json!({
                    "Customization": p.get("Customization").cloned().unwrap_or(Value::Null),
                    "Emblem": p.get("Emblem").cloned().unwrap_or(Value::Null),
                    "EmblemColor1": p.get("EmblemColor1").cloned().unwrap_or(Value::Null),
                    "EmblemColor2": p.get("EmblemColor2").cloned().unwrap_or(Value::Null),
                });
                ev.push(WorldEvent::SpawnEquipment { by: id, class: class.clone(), xf: *xf, customization });
                if first {
                    self.next_check = Some(now + 1.0 + 1.0 + range_f(rng, -1.0, 1.0)); // @254
                }
            }
        }
        if let Some(t) = self.next_check {
            if now >= t {
                self.next_check = Some(t + 1.0);
                // CheckEquipment @363: not while respawning; the instance gone, held (GetParentCharacter valid), or
                // > 200 cm away -> BeginRespawnProcess (@860: SpawnEquipment after RespawnInterval, Respawning = true)
                if !self.respawning {
                    let s = q.spawned_status(id);
                    if !s.exists || s.held || s.distance > 200.0 {
                        self.respawning = true;
                        self.spawn_at = Some(now + p.f("RespawnInterval"));
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum VehicleKind {
    Horse,
    Ballista,
    /// BP_CatapultSpawner: BP_Catapult_C spawned at (0, 0, -3000) and teleported like a horse
    Catapult,
    /// BP_VehicleSpawner itself: SpawnVehicle is empty (@34)
    Other,
}

#[derive(Clone, Debug)]
pub struct VehicleSpawner {
    pub kind: VehicleKind,
    pub active: bool,
    pub respawn_time: f64,
    /// pending TrySpawnVehicle (Delay continuation)
    pub spawn_at: Option<f64>,
    pub has_vehicle: bool,
    /// OnCharacterDied bound (only when RespawnTime > 0 at the spawn, Horse @308 / Ballista @518)
    pub died_bound: bool,
    /// the horse classes (TSet Horses, insertion order = Set_ToArray order)
    pub horses: Vec<String>,
    /// the Capsule component's relative transform (ballista spawn / overlap capsule)
    pub capsule_rel: Xf,
    /// horse: the teleport onto the spawner after RandomFloatInRange(0, 0.1) (@2204)
    pub teleport_at: Option<f64>,
    /// horse: a failed teleport destroys the horse after RandomFloatInRange(0.1, 0.5) (@417)
    pub destroy_at: Option<f64>,
    pub spawned_class: String,
}

fn class_ref(v: &Value) -> Option<String> {
    v.get("ObjectName").and_then(|n| n.as_str()).map(|n| n.split('\'').nth(1).unwrap_or(n).to_string()).filter(|n| !n.is_empty())
}

impl VehicleSpawner {
    pub fn new(p: &Props, chain: &[String], pk: &mh_level::Pkgs, class_chain: &[String]) -> VehicleSpawner {
        let kind = if chain.iter().any(|c| c == "BP_HorseSpawner") {
            VehicleKind::Horse
        } else if chain.iter().any(|c| c == "BP_BallistaSpawner") {
            VehicleKind::Ballista
        } else if chain.iter().any(|c| c == "BP_CatapultSpawner") {
            VehicleKind::Catapult
        } else {
            VehicleKind::Other
        };
        // BP_HorseSpawner ReceiveBeginPlay @574..@985 (authority): Set_AddItems of RegularHorses / MediumHorses /
        // ArmoredHorses / Camels / Hogs per bAllowRegular / bAllowMedium / bAllowArmored / bAllowCamel / bAllowHog
        let mut horses: Vec<String> = vec![];
        for (flag, set) in [("bAllowRegular", "RegularHorses"), ("bAllowMedium", "MediumHorses"), ("bAllowArmored", "ArmoredHorses"), ("bAllowCamel", "Camels"), ("bAllowHog", "Hogs")] {
            if p.b(flag) {
                for c in p.arr(set).iter().filter_map(class_ref) {
                    if !horses.contains(&c) {
                        horses.push(c);
                    }
                }
            }
        }
        let capsule_rel = class_chain
            .iter()
            .find_map(|cls| {
                pk.load_pkg(cls).iter().find(|e| e.get("Name").and_then(|v| v.as_str()) == Some("Capsule_GEN_VARIABLE")).map(|e| {
                    let pr = e.get("Properties").and_then(|p| p.as_object()).cloned().unwrap_or_default();
                    mh_level::xf::rel_xf(&pr)
                })
            })
            .unwrap_or(mh_level::xf::IDENTITY);
        VehicleSpawner {
            kind,
            active: p.b("Active"),
            respawn_time: p.f("RespawnTime"),
            spawn_at: None,
            has_vehicle: false,
            died_bound: false,
            horses,
            capsule_rel,
            teleport_at: None,
            destroy_at: None,
            spawned_class: String::new(),
        }
    }

    /// ReceiveBeginPlay: RespawnTime = the game mode's HorseRespawnTime (Horse @864) / BallistaRespawnTime (Ballista
    /// @176) / CatapultRespawnTime (Catapult @650) when the game mode is a MordhauGameMode (`game_mode` Some: horse,
    /// ballista, catapult), then BP_VehicleSpawner's ReceiveBeginPlay (@29) -> TrySpawnVehicle
    #[allow(clippy::too_many_arguments)]
    pub fn begin_play(&mut self, id: ActorId, xf: &Xf, now: f64, game_mode: Option<(f64, f64, f64)>, q: &dyn Queries, rng: &mut CrtRand, ev: &mut Vec<WorldEvent>) {
        if let Some((horse, ballista, catapult)) = game_mode {
            match self.kind {
                VehicleKind::Horse => self.respawn_time = horse,
                VehicleKind::Ballista => self.respawn_time = ballista,
                VehicleKind::Catapult => self.respawn_time = catapult,
                VehicleKind::Other => {}
            }
        }
        self.try_spawn(id, xf, now, q, rng, ev);
    }

    /// TrySpawnVehicle: authority and Active -> SpawnVehicle
    fn try_spawn(&mut self, id: ActorId, xf: &Xf, now: f64, q: &dyn Queries, rng: &mut CrtRand, ev: &mut Vec<WorldEvent>) {
        if !self.active {
            return;
        }
        match self.kind {
            // BP_BallistaSpawner:SpawnVehicle @886: CapsuleOverlapActors at the Capsule (object types [0, 4, 2]) ->
            // retry after 3 s (@627); else BP_Ballista_C at the Capsule's world transform (@732), OnCharacterDied
            // bound when RespawnTime > 0 (@518)
            VehicleKind::Ballista => {
                if q.spawn_blocked(id) {
                    self.spawn_at = Some(now + 3.0);
                } else {
                    self.has_vehicle = true;
                    self.died_bound = self.respawn_time > 0.0;
                    self.spawned_class = "BP_Ballista_C".into();
                    ev.push(WorldEvent::Spawn { by: id, class: self.spawned_class.clone(), xf: *xf * self.capsule_rel });
                }
            }
            // BP_HorseSpawner:SpawnVehicle @2360..@1513: with any horse class, RandomIntegerInRange(0, n - 1) of the
            // set, spawned at world (0, 0, -3000) (MakeTransform @1299; the colour / unused-dying overrides are the
            // spawner's properties for the host), teleported onto the spawner after RandomFloatInRange(0, 0.1) (@2204)
            VehicleKind::Horse => {
                if self.horses.is_empty() {
                    return;
                }
                let i = range_i(rng, 0, self.horses.len() as i64 - 1) as usize;
                self.spawned_class = self.horses[i].clone();
                self.has_vehicle = true;
                ev.push(WorldEvent::Spawn { by: id, class: self.spawned_class.clone(), xf: Xf::trs([0.0, 0.0, -3000.0], [0.0, 0.0, 0.0, 1.0], [1.0; 3]) });
                self.teleport_at = Some(now + range_f(rng, 0.0, 0.1));
            }
            // BP_CatapultSpawner:SpawnVehicle @770..@1041: BP_Catapult_C at world (0, 0, -3000), teleported onto the
            // spawner after RandomFloatInRange(0, 0.1) (@50..@106; the failure path @289..@460 is the horse's)
            VehicleKind::Catapult => {
                self.spawned_class = "BP_Catapult_C".into();
                self.has_vehicle = true;
                ev.push(WorldEvent::Spawn { by: id, class: self.spawned_class.clone(), xf: Xf::trs([0.0, 0.0, -3000.0], [0.0, 0.0, 0.0, 1.0], [1.0; 3]) });
                self.teleport_at = Some(now + range_f(rng, 0.0, 0.1));
            }
            VehicleKind::Other => {}
        }
    }

    /// The host's K2_TeleportTo result for the horse (@294): success binds OnCharacterDied when RespawnTime > 0;
    /// failure destroys it after RandomFloatInRange(0.1, 0.5) and retries after RandomFloatInRange(1, 3) (@417, @45)
    pub fn teleport_result(&mut self, ok: bool, now: f64, rng: &mut CrtRand) {
        if ok {
            self.died_bound = self.respawn_time > 0.0;
        } else {
            self.destroy_at = Some(now + range_f(rng, 0.1, 0.5));
        }
    }

    /// OnHorseDied / OnBallistaDied -> Delay(RespawnTime) -> TrySpawnVehicle (only when bound)
    pub fn on_vehicle_died(&mut self, now: f64) {
        self.has_vehicle = false;
        if self.died_bound {
            self.died_bound = false;
            self.spawn_at = Some(now + self.respawn_time);
        }
    }

    /// Deactivate: Active = false; (authority) DestroyVehicleOnDeactivate kills a live vehicle that is not dead
    /// (ApplyDamage 1e9 @177)
    pub fn deactivate(&mut self, id: ActorId, p: &Props, ev: &mut Vec<WorldEvent>) {
        if !self.active {
            return;
        }
        self.active = false;
        if p.b("DestroyVehicleOnDeactivate") && self.has_vehicle {
            ev.push(WorldEvent::KillSpawned { by: id });
        }
    }

    /// Activate: as shipped it sets Active = false when inactive (@19) before TrySpawnVehicle, so it never activates a
    /// spawner (ported as is)
    pub fn activate(&mut self) {
        if self.active {
            return;
        }
        self.active = false;
    }

    pub fn tick(&mut self, id: ActorId, xf: &Xf, now: f64, q: &dyn Queries, rng: &mut CrtRand, ev: &mut Vec<WorldEvent>) {
        if self.has_vehicle && q.spawned_status(id).dead {
            self.on_vehicle_died(now);
        }
        if let Some(t) = self.teleport_at {
            if now >= t {
                self.teleport_at = None;
                ev.push(WorldEvent::TeleportSpawned { by: id, xf: *xf }); // K2_TeleportTo(actor location, rotation) @234
            }
        }
        if let Some(t) = self.destroy_at {
            if now >= t {
                self.destroy_at = None;
                self.has_vehicle = false;
                ev.push(WorldEvent::DestroySpawned { by: id }); // K2_DestroyActor @45
                self.spawn_at = Some(now + range_f(rng, 1.0, 3.0)); // @81
            }
        }
        if let Some(t) = self.spawn_at {
            if now >= t {
                self.spawn_at = None;
                self.try_spawn(id, xf, now, q, rng, ev);
            }
        }
    }
}

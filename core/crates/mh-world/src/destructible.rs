//! BP_DestroyableActor (Mordhau/Content/Mordhau/Blueprints/Interactables/Environment/BP_DestroyableActor; native
//! parent AMordhauActor): health, damage / repair, team rules, damage-state meshes, regeneration, death, the
//! attached-projectile detach and the upgrade "unstuck" pass.
//! Bytecode: state/world_kismet/BP_DestroyableActor.txt (functions cited by name and @statement).

use crate::props::Props;
use crate::world::{ActorId, CharId, DamageInfo, WorldEvent};

/// EScoreFeedReason values the Blueprint awards (SetHealth@719 repair = 15, @1195 damage = 6)
pub const SCORE_REPAIR: u8 = 15;
pub const SCORE_STRUCTURE_DAMAGE: u8 = 6;

#[derive(Clone, Debug)]
pub struct Destructible {
    pub health: f64,
    pub replicated: u8,
    pub last_replicated: u8,
    pub max_repairable: u8,
    /// SpawnHealth (ReceiveBeginPlay@795)
    pub spawn_health: f64,
    pub regenerating: bool,
    /// next RegenTickEvent (K2_SetTimerDelegate(RegenTick, non-looping) @1119 / @1250)
    pub next_regen: Option<f64>,
    /// ReceiveBeginPlay's Delay(RegenStartDelay) before StartRegenerating (@135..@165 -> @105)
    pub regen_start_at: Option<f64>,
    pub mesh_index: Option<usize>,
    pub collision: bool,
    pub hidden: bool,
    /// SetLifeSpan(1.0) after destruction with DeleteWhenDestroyed (UpdateReplicatedHealth@358)
    pub expire_at: Option<f64>,
    /// DetachAfterShortDelay's Delay(0.01) -> DetachAttachedProjectiles (@1308 -> @15)
    pub detach_at: Option<f64>,
    pub begun_play: bool,
    pub destroyed: bool,
    /// HasAuthority (OnRep's @1297 detach / unstuck, @2389 regeneration and @2774 stop branches are the server's)
    pub authority: bool,
}

impl Destructible {
    pub fn new(p: &Props) -> Destructible {
        let health = p.f("Health");
        Destructible {
            health,
            replicated: 0,
            last_replicated: 0,
            max_repairable: p.i("MaxHealthRepairableTo").clamp(0, 255) as u8,
            spawn_health: health,
            regenerating: p.b("Regenerating"),
            next_regen: None,
            regen_start_at: None,
            mesh_index: None,
            collision: true,
            hidden: false,
            expire_at: None,
            detach_at: None,
            begun_play: false,
            destroyed: false,
            authority: true,
        }
    }

    /// UserConstructionScript (placed actors run it before BeginPlay): ReplicatedHealth = clamp(floor(Health), 0, 255)
    /// (@20..@141, FFloor here, FCeil in UpdateReplicatedHealth), OnRep_ReplicatedHealth (@197: segments, the
    /// damage-state mesh, a 0-health actor's collision), LastReplicatedHealth = ReplicatedHealth (@211)
    pub fn construct(&mut self, id: ActorId, p: &Props, ev: &mut Vec<WorldEvent>) {
        self.replicated = self.health.floor().clamp(0.0, 255.0) as u8;
        self.on_rep_replicated_health(id, p, 0.0, &|| Overlaps::default(), ev);
        self.last_replicated = self.replicated;
    }

    /// ReceiveBeginPlay@795..@862: SpawnHealth = Health; (authority) a Regenerating actor starts regenerating after
    /// RegenStartDelay (Delay@165 -> @105 StartRegenerating when still Regenerating)
    pub fn begin_play(&mut self, p: &Props, now: f64) {
        self.spawn_health = self.health;
        self.begun_play = true;
        if self.regenerating && self.authority {
            self.regen_start_at = Some(now + p.f("RegenStartDelay"));
        }
    }

    /// ReceiveAnyDamage (ExecuteUbergraph@293..@762): damage >= 0 or Repairable; new health =
    /// min(Health - Damage * DamageFactor * (Damage < 0 ? RepairFactor : 1), max(MaxHealthRepairableTo, Health))
    #[allow(clippy::too_many_arguments)]
    pub fn receive_any_damage(&mut self, id: ActorId, p: &Props, damage: f64, d: &DamageInfo, now: f64, ov: Ov, ev: &mut Vec<WorldEvent>) {
        if self.destroyed || !(damage >= 0.0 || p.b("Repairable")) {
            return;
        }
        let factor = if damage < 0.0 { p.f("RepairFactor") } else { 1.0 };
        let cap = (self.max_repairable as f64).max(self.health);
        let new = (self.health - damage * p.f("DamageFactor") * factor).min(cap);
        // SetHealth's instigator: the InstigatedBy controller, a team only through a MordhauPlayerController (@144)
        let team = if d.instigator_player { d.instigator_team } else { None };
        self.set_health(id, p, new, team, d.instigator, now, ov, ev);
    }

    /// SetHealth: clamp 0..255; with an OwningTeam (!= 255) and a player instigator: the owning team only repairs
    /// (score 15, min(trunc |dH|, RepairScoreMax)) and cannot damage; another team only damages (score 6,
    /// min(trunc dH, DamageScoreMax)) and cannot repair (@103..@1215); then Health, UpdateReplicatedHealth, and
    /// OnDeath whenever Health <= 0 (@781..@825)
    #[allow(clippy::too_many_arguments)]
    pub fn set_health(&mut self, id: ActorId, p: &Props, new: f64, instigator_team: Option<u8>, instigator: Option<CharId>, now: f64, ov: Ov, ev: &mut Vec<WorldEvent>) {
        let new = new.clamp(0.0, 255.0);
        let owning = p.i("OwningTeam");
        let mut score = None;
        if owning != 255 {
            if let Some(team) = instigator_team {
                let delta = self.health - new;
                if team as i64 == owning {
                    if delta >= 0.0 {
                        return; // own team: no damage (@512)
                    }
                    score = Some((SCORE_REPAIR, (delta.abs().trunc() as i64).min(p.i("RepairScoreMax"))));
                } else {
                    if delta < 0.0 {
                        return; // enemy: no repair (@1020)
                    }
                    score = Some((SCORE_STRUCTURE_DAMAGE, (delta.trunc() as i64).min(p.i("DamageScoreMax"))));
                }
            }
        }
        self.health = new;
        self.update_replicated_health(id, p, now, ov, ev);
        if self.health <= 0.0 {
            ev.push(WorldEvent::Died { actor: id }); // OnDeath.Broadcast @825
        }
        if let Some((kind, amount)) = score {
            if amount != 0 {
                ev.push(WorldEvent::Score { char: instigator, team: instigator_team, kind, amount });
            }
        }
    }

    /// UpdateReplicatedHealth: ReplicatedHealth = clamp(ceil(Health), 0, 255); on change OnRep_ReplicatedHealth, and at
    /// 0 with DeleteWhenDestroyed SetLifeSpan(1.0)
    fn update_replicated_health(&mut self, id: ActorId, p: &Props, now: f64, ov: Ov, ev: &mut Vec<WorldEvent>) {
        let rep = (self.health.ceil() as i64).clamp(0, 255) as u8;
        if rep == self.replicated {
            return;
        }
        self.replicated = rep;
        self.on_rep_replicated_health(id, p, now, ov, ev);
        if rep == 0 && p.b("DeleteWhenDestroyed") {
            self.expire_at = Some(now + 1.0);
        }
    }

    /// OnRep_ReplicatedHealth (authority side). `chars` / `actors`: what StaticMesh overlaps (the unstuck pass)
    pub fn on_rep_replicated_health(&mut self, id: ActorId, p: &Props, now: f64, ov: Ov, ev: &mut Vec<WorldEvent>) {
        let rep = self.replicated;
        // health segments (@44..@443): MaxHealthRepairableTo = min(ceil(rep / seg) * seg, 100), seg = ceil(100 / n)
        let segs = p.i("RepairableHealthSegments");
        if segs > 1 {
            let seg = (100.0 / segs as f64).ceil();
            let m = ((rep as f64 / seg).ceil() * seg) as i64;
            self.max_repairable = m.clamp(0, 100) as u8;
        }
        // damage-state mesh (@471..@1211): the last index i with rep <= DamageMeshesHealth[i] (BestIdx, a zeroed
        // local), when DamageMeshes has it and it differs from the current mesh
        let hs = p.arr("DamageMeshesHealth");
        if !hs.is_empty() {
            let mut best = 0usize;
            for (i, h) in hs.iter().enumerate() {
                if (rep as i64) <= h.as_i64().unwrap_or(0) {
                    best = i;
                }
            }
            let meshes = p.arr("DamageMeshes");
            if let Some(m) = meshes.get(best) {
                if self.mesh_index != Some(best) {
                    self.mesh_index = Some(best);
                    let mesh = m.get("ObjectPath").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    ev.push(WorldEvent::MeshSwapped { actor: id, mesh });
                    // OnMeshChanged @1211, then once begun play (@1225): DetachAfterShortDelay (@1332) and, when the
                    // health went up (an upgrade / repair) with PerformsUnstuckProcess, the unstuck pass over what
                    // StaticMesh overlaps (object types [0, 2, 4], @1433..@1523)
                    if self.begun_play && self.authority {
                        self.detach_at = Some(now + 0.009999999776482582);
                        if self.last_replicated < rep && p.b("PerformsUnstuckProcess") {
                            let o = ov();
                            let (chars, actors) = (&o.chars, &o.actors);
                            // other BP_DestroyableActors: ApplyDamage 1e10 with DestroysOtherWhenUpgrading (@1922)
                            for (a, is_destroyable) in actors {
                                if *a != id && *is_destroyable && p.b("DestroysOtherWhenUpgrading") {
                                    ev.push(WorldEvent::DamageActor { actor: *a, amount: 1e10 });
                                }
                            }
                            // characters not in a vehicle (@3606..@4921): the host's capsule trace from 500 cm above
                            // down to the character (radius x0.8, half height x0.9, object type 0); when it hits this
                            // actor, K2_TeleportTo(StaticMesh closest point to (location + 100 Z) + unscaled half height)
                            for (c, in_vehicle) in chars {
                                if !in_vehicle {
                                    ev.push(WorldEvent::Unstuck { char: *c, actor: id });
                                }
                            }
                        }
                    }
                }
            }
        }
        // regeneration (@2389..@2700, authority)
        let auth = self.authority;
        let stop = auth && ((p.b("StopRegeneratingOnDamage") && rep < self.last_replicated) || rep >= self.max_repairable);
        if stop {
            self.stop_regenerating();
        }
        if auth && p.b("AutoResumeRegenerating") && rep < self.max_repairable {
            self.start_regenerating(p, now);
        }
        self.last_replicated = rep;
        // destroyed (@2728..@2903): stop regen; DeleteWhenDestroyed -> StaticMesh hidden + no actor collision; else
        // DisableCollisionWhenDestroyed -> no collision; OnDeath broadcast (@2949). The Blueprint never re-enables
        // collision when a destroyed actor is repaired (no SetActorEnableCollision(true) node in the class)
        if rep == 0 {
            if auth {
                self.stop_regenerating(); // @2774
            }
            if p.b("DeleteWhenDestroyed") {
                if !self.hidden {
                    self.hidden = true;
                    ev.push(WorldEvent::Hidden { actor: id });
                }
                self.set_collision(id, false, ev);
            } else if p.b("DisableCollisionWhenDestroyed") {
                self.set_collision(id, false, ev);
            }
            ev.push(WorldEvent::Died { actor: id });
        }
    }

    fn set_collision(&mut self, id: ActorId, on: bool, ev: &mut Vec<WorldEvent>) {
        if self.collision != on {
            self.collision = on;
            ev.push(WorldEvent::CollisionEnabled { actor: id, on });
        }
    }

    /// StartRegenerating (@1177, authority): Regenerating = true (@79), RegenTickEvent after RegenTick (@1250)
    pub fn start_regenerating(&mut self, p: &Props, now: f64) {
        self.regenerating = true;
        self.next_regen = Some(now + p.f("RegenTick"));
    }
    /// StopRegenerating: Regenerating = false (the pending timer finds Regenerating false and stops, @877)
    pub fn stop_regenerating(&mut self) {
        self.regenerating = false;
        self.next_regen = None;
    }

    /// timers: the begin-play regen delay, RegenTickEvent (@877: Health = min(Health + RegenPerTick,
    /// max(MaxHealthRepairableTo, Health)) through SetHealth without an instigator, the timer again while
    /// Regenerating @1086), the projectile detach, the 1 s life span
    pub fn tick(&mut self, id: ActorId, p: &Props, now: f64, ov: Ov, ev: &mut Vec<WorldEvent>) {
        if let Some(t) = self.expire_at {
            if now >= t && !self.destroyed {
                self.destroyed = true;
                ev.push(WorldEvent::Destroyed { actor: id });
            }
        }
        if let Some(t) = self.detach_at {
            if now >= t {
                self.detach_at = None;
                // DetachAttachedProjectiles: attached MordhauProjectiles destroyed, MordhauEquipment detached
                ev.push(WorldEvent::DetachAttached { actor: id });
            }
        }
        if let Some(t) = self.regen_start_at {
            if now >= t {
                self.regen_start_at = None;
                if self.regenerating {
                    self.start_regenerating(p, now);
                }
            }
        }
        if let Some(t) = self.next_regen {
            if now >= t {
                self.next_regen = None;
                if self.regenerating {
                    let cap = (self.max_repairable as f64).max(self.health);
                    let new = (self.health + p.f("RegenPerTick")).min(cap);
                    self.set_health(id, p, new, None, None, now, ov, ev);
                    if self.regenerating {
                        self.next_regen = Some(now + p.f("RegenTick"));
                    }
                }
            }
        }
    }
}

/// The overlaps of the unstuck pass, asked for only when it runs
pub type Ov<'a> = &'a dyn Fn() -> Overlaps;

/// What the actor's StaticMesh overlaps when its health replicates (the unstuck pass): characters (id, in a vehicle)
/// and world actors (id, is a BP_DestroyableActor)
#[derive(Clone, Debug, Default)]
pub struct Overlaps {
    pub chars: Vec<(CharId, bool)>,
    pub actors: Vec<(ActorId, bool)>,
}

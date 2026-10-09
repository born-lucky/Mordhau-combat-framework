//! Overlap-driven placed actors: hazard and trigger volumes (out-of-bounds boxes, death / OOB / flame barriers,
//! force-fall boxes, spawn protection boxes, kill-objective containment, ledge pushers, directional ragdoll boxes,
//! tree breakers, the Demon Horde door activator) and the moving hazards' damage (dungeon spikes, swinging logs),
//! plus the impalement volumes (component hit). Bytecode: state/world_kismet/<class>.txt for every class named here.
//!
//! The host answers which characters overlap each trigger component (`Queries::component_chars`; begin / end
//! overlaps are the frame-to-frame differences, as UPrimitiveComponent::UpdateOverlaps reports them) and reports
//! component hits for the impalement volumes (`World::component_hit`). Damage to characters is the combat side's
//! (`WorldEvent::TakeDamage` = AAdvancedCharacter::MordhauTakeDamage rva 0x148a3f0 (Amount, Hit, DamageType,
//! DamageSubType, Source, Agent, EventInstigator); the Blueprints pass an empty FHitResult with ImpactNormal (0,0,1)).

use crate::props::Props;
use crate::spawner::{range_f, CrtRand};
use crate::world::{ActorId, CharId, CharView, WorldEvent};
use crate::V;
use mh_level::xf::Xf;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// UKismetMathLibrary::RandomBoolWithWeight (UE 4.26 KismetMathLibrary.cpp; a rand() caller, state/rand_callers.txt):
/// Weight <= 0 -> false, else Weight >= FMath::FRandRange(0, 1)
pub fn random_bool_with_weight(rng: &mut CrtRand, w: f64) -> bool {
    if w <= 0.0 {
        return false;
    }
    w >= range_f(rng, 0.0, 1.0)
}

/// The 13 / 7 bones the spike / swinging log dismember (BP_SpikeProgressActor@200, BP_SwingingLogMovable@55)
pub const DISMEMBER_13: [&str; 13] = ["head", "RightArm", "RightForearm", "RightHand", "LeftArm", "LeftForearm", "LeftHand", "RightUpLeg", "RightLeg", "RightFoot", "LeftUpLeg", "LeftLeg", "LeftFoot"];
pub const DISMEMBER_7: [&str; 7] = ["head", "RightArm", "RightForearm", "RightHand", "LeftArm", "LeftForearm", "LeftHand"];

#[derive(Clone, Debug, PartialEq)]
pub enum TriggerKind {
    /// BP_OutOfBoundsBox: Bounds actors entering / leaving (EnteredOutOfBoundsArea(self) / LeftOutOfBoundsArea, @10..@211)
    OutOfBounds,
    /// BP_ForceFallDeathBox (@15..@352): AdvancedCharacters entering get FallDamageOffset 1e10, FallDamageFactor 1,
    /// MinVelocityForFallDamage 100, MordhauCharacters ReceivedFallDamageModifier 1
    ForceFallDeath,
    /// BP_DeathBarrier ("Kill" begin overlap, @15..@765)
    DeathBarrier,
    /// BP_OOBBarrier ("OOB" begin / end, "Kill" begin, every tick the Kill box, @15..@1695)
    OobBarrier,
    /// BP_FlameBarrier ("OOB", "Flame", "Kill"; @15..@3186)
    FlameBarrier,
    /// BP_SpawnProtectionBox / BP_PushSpawnProtectionBox: EnteredTeamArea / LeftTeamArea(AllowedTeam) (@10..@214)
    SpawnProtection { team: i64 },
    /// BP_KillObjectiveContainmentBox: its KillObjectiveToContain leaving is out of bounds (@10..@400)
    Containment { kill_objective: Option<ActorId> },
    /// BP_LedgePusher (authority, @10..@406): Demon Horde enemies entering are ledge-pushed towards PushDirection's
    /// yaw; leaving deals LedgePusherFallDamage
    LedgePusher { yaw: f64 },
    /// BP_ForceRagdoll_Directional: every tick Knockback(50, -200, 30) + Trip on the AdvancedCharacters in Box (@370..@668)
    ForceRagdollDirectional,
    /// BP_TreeBreaker: a BP_MordhauCharacter entering Sphere (authority) breaks the BP_DestroyableTrees in Sphere one by
    /// one, RandomFloatInRange(1, 5) s apart (@15..@759)
    TreeBreaker,
    /// BP_DoorActivator: a BP_DemonHordeCharacter entering Sphere (authority) makes the BP_DestroyableCastleDoors in
    /// Sphere damageable one by one, RandomFloatInRange(1, 5) s apart (@15..@994)
    DoorActivator,
    /// BP_SpikeProgressActor "Box" / BP_SwingingLogMovable "Capsule" begin overlap: ProcessHit + the dismember FX
    Impaler { force_mult: f64, dismember_weight: f64, bones: &'static [&'static str], skip_dead: bool },
    /// BP_ImpalementVolume / BP_ImpalementVolumeInstantKill: component hits (`World::component_hit`), no overlaps
    Impalement { instant: bool, velocity_to_kill: f64 },
}

#[derive(Clone, Debug)]
pub struct Trigger {
    pub kind: TriggerKind,
    /// characters inside each watched component last frame
    pub inside: BTreeMap<&'static str, BTreeSet<CharId>>,
    /// tree breaker / door activator: actors left to handle and the time of the next
    pub queue: VecDeque<ActorId>,
    pub next_at: Option<f64>,
    /// BP_ImpalementVolume LastDealtDamageTime
    pub last_damage: f64,
    /// the actor's world transform (impalement Box rotation; ledge push direction)
    pub xf: Xf,
}

/// The components a trigger watches
fn components(k: &TriggerKind) -> &'static [&'static str] {
    match k {
        TriggerKind::OobBarrier => &["OOB", "Kill"],
        TriggerKind::FlameBarrier => &["OOB", "Flame", "Kill"],
        TriggerKind::DeathBarrier => &["Kill"],
        TriggerKind::TreeBreaker | TriggerKind::DoorActivator => &["Sphere"],
        TriggerKind::Impalement { .. } => &[],
        TriggerKind::Impaler { bones, .. } if bones.len() == 7 => &["Capsule"],
        _ => &["Box"],
    }
}

fn yaw_of(x: &Xf) -> f64 {
    x.m[1][0].atan2(x.m[0][0]).to_degrees()
}

impl Trigger {
    /// The trigger behaviour of a placed actor's Blueprint chain, if any
    pub fn of(chain: &[String], p: &Props, xf: &Xf, pk: &mh_level::Pkgs, class_chain: &[String], resolve: &dyn Fn(&str) -> Option<ActorId>) -> Option<Trigger> {
        let has = |n: &str| chain.iter().any(|c| c == n);
        let kind = if has("BP_OutOfBoundsBox") {
            TriggerKind::OutOfBounds
        } else if has("BP_ForceFallDeathBox") {
            TriggerKind::ForceFallDeath
        } else if has("BP_DeathBarrier") {
            TriggerKind::DeathBarrier
        } else if has("BP_OOBBarrier") {
            TriggerKind::OobBarrier
        } else if has("BP_FlameBarrier") {
            TriggerKind::FlameBarrier
        } else if has("BP_SpawnProtectionBox") {
            TriggerKind::SpawnProtection { team: p.i("AllowedTeam") }
        } else if has("BP_KillObjectiveContainmentBox") {
            let k = p.get("KillObjectiveToContain").and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()).and_then(resolve);
            TriggerKind::Containment { kill_objective: k }
        } else if has("BP_LedgePusher") {
            // PushDirection.K2_GetComponentRotation().Yaw (@310..@406): the component's world yaw
            let rel = crate::ladder::comp_rel_xf(pk, class_chain, "PushDirection").unwrap_or(mh_level::xf::IDENTITY);
            TriggerKind::LedgePusher { yaw: yaw_of(&(*xf * rel)) }
        } else if has("BP_ForceRagdoll_Directional") {
            TriggerKind::ForceRagdollDirectional
        } else if has("BP_TreeBreaker") {
            TriggerKind::TreeBreaker
        } else if has("BP_DoorActivator") {
            TriggerKind::DoorActivator
        } else if has("BP_SpikeProgressActor") {
            // ProcessHit: RagdollForceMultIfDmgAgent 10, dead characters skipped (@157..@218); FX: 13 bones x
            // RandomBoolWithWeight(0.4) (@160..@439)
            TriggerKind::Impaler { force_mult: 10.0, dismember_weight: 0.4000000059604645, bones: &DISMEMBER_13, skip_dead: true }
        } else if has("BP_SwingingLogMovable") {
            // ProcessHit: RagdollForceMultIfDmgAgent 17, no dead check (@79..@157); FX: 7 bones x RandomBoolWithWeight(0.25)
            TriggerKind::Impaler { force_mult: 17.0, dismember_weight: 0.25, bones: &DISMEMBER_7, skip_dead: false }
        } else if has("BP_ImpalementVolumeInstantKill") {
            TriggerKind::Impalement { instant: true, velocity_to_kill: p.f("VelocityToKill") }
        } else if has("BP_ImpalementVolume") {
            TriggerKind::Impalement { instant: false, velocity_to_kill: p.f("VelocityToKill") }
        } else {
            return None;
        };
        Some(Trigger { kind, inside: BTreeMap::new(), queue: VecDeque::new(), next_at: None, last_damage: f64::NEG_INFINITY, xf: *xf })
    }

    /// One frame: the begin / end overlaps per component and the per-tick parts. `authority`: HasAuthority.
    /// `trees` / `doors`: the actors in Sphere of the class the tree breaker / door activator looks for
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        id: ActorId,
        now: f64,
        authority: bool,
        comp_chars: &dyn Fn(&str) -> Vec<CharId>,
        chars: &[CharView],
        sphere_actors: &dyn Fn() -> Vec<ActorId>,
        rng: &mut CrtRand,
        ev: &mut Vec<WorldEvent>,
    ) -> Vec<ActorId> {
        let view = |c: CharId| chars.iter().find(|v| v.id == c);
        let mut out = vec![];
        for comp in components(&self.kind) {
            let now_in: BTreeSet<CharId> = comp_chars(comp).into_iter().collect();
            let before = self.inside.get(comp).cloned().unwrap_or_default();
            for c in now_in.difference(&before) {
                if let Some(v) = view(*c) {
                    self.begin(id, comp, v, now, authority, sphere_actors, rng, ev);
                }
            }
            for c in before.difference(&now_in) {
                let fallback = CharView { id: *c, ..Default::default() };
                let v = view(*c).unwrap_or(&fallback);
                self.end(id, comp, v, authority, ev);
            }
            self.inside.insert(comp, now_in);
        }
        // per-tick parts
        let inside = |c: &str| self.inside.get(c).cloned().unwrap_or_default();
        match &self.kind {
            // OOB / flame barrier ReceiveTick: every not-dead Demon Horde character in Kill dies (IsImmortal false,
            // MordhauTakeDamage(99999, type 4)); flame barrier: the ones in Flame burn (StartBurning(0.3, 6, 0.1))
            TriggerKind::OobBarrier | TriggerKind::FlameBarrier => {
                if self.kind == TriggerKind::FlameBarrier {
                    for c in inside("Flame") {
                        if view(c).is_some_and(|v| v.is_demon_horde_character && !v.dead) {
                            ev.push(WorldEvent::StartBurning { char: c, a: 0.30000001192092896, b: 6.0, c: 0.10000000149011612 });
                        }
                    }
                }
                for c in inside("Kill") {
                    if view(c).is_some_and(|v| v.is_demon_horde_character && !v.dead) {
                        ev.push(WorldEvent::SetImmortal { char: c, immortal: false });
                        ev.push(WorldEvent::TakeDamage { char: c, amount: 99999.0, damage_type: 4, sub_type: 0, source: None, agent: None });
                    }
                }
            }
            TriggerKind::ForceRagdollDirectional => {
                for c in inside("Box") {
                    // MakeVector(50, -200, 30) as is, world space (@570..@668)
                    ev.push(WorldEvent::Knockback { char: c, impulse: [50.0, -200.0, 30.0] });
                    ev.push(WorldEvent::Trip { char: c });
                }
            }
            _ => {}
        }
        // the sequential tree / door queue (Delay(RandomFloatInRange(1, 5)) between items)
        if let Some(t) = self.next_at {
            if now >= t {
                if let Some(a) = self.queue.pop_front() {
                    out.push(a);
                    self.next_at = Some(now + range_f(rng, 1.0, 5.0));
                } else {
                    self.next_at = None;
                }
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn begin(&mut self, id: ActorId, comp: &str, v: &CharView, now: f64, authority: bool, sphere_actors: &dyn Fn() -> Vec<ActorId>, rng: &mut CrtRand, ev: &mut Vec<WorldEvent>) {
        let c = v.id;
        match (&self.kind, comp) {
            (TriggerKind::OutOfBounds, _) => ev.push(WorldEvent::OutOfBounds { char: c, volume: id, entered: true }),
            (TriggerKind::ForceFallDeath, _) => ev.push(WorldEvent::SetFallDamage {
                char: c,
                received_fall_damage_modifier: (!v.is_horse).then_some(1.0),
                fall_damage_offset: 10000000000.0,
                fall_damage_factor: 1.0,
                min_velocity: 100.0,
            }),
            (TriggerKind::DeathBarrier, "Kill") => {
                if v.is_demon_horde_character {
                    ev.push(WorldEvent::SetImmortal { char: c, immortal: false }); // @100
                }
                if !v.is_horse {
                    ev.push(WorldEvent::TakeDamage { char: c, amount: 99999.0, damage_type: 0, sub_type: 0, source: None, agent: None }); // @353
                } else {
                    // the driver (type 4) and the horse (ApplyDamage 99999), @648..@765
                    ev.push(WorldEvent::DamageDriver { vehicle: c, amount: 99999.0, damage_type: 4 });
                    ev.push(WorldEvent::ApplyDamageChar { char: c, amount: 99999.0 });
                }
            }
            (TriggerKind::OobBarrier | TriggerKind::FlameBarrier, "OOB") => {
                if v.is_demon_horde_character {
                    ev.push(WorldEvent::OutOfBounds { char: c, volume: id, entered: true });
                }
            }
            (TriggerKind::FlameBarrier, "Flame") => {
                if v.is_demon_horde_character {
                    ev.push(WorldEvent::StartBurning { char: c, a: 0.30000001192092896, b: 6.0, c: 0.10000000149011612 });
                }
            }
            (TriggerKind::OobBarrier | TriggerKind::FlameBarrier, "Kill") => {
                if v.is_demon_horde_character {
                    ev.push(WorldEvent::SetImmortal { char: c, immortal: false }); // the RetriggerableDelay(0) kill runs in tick
                } else if v.is_horse {
                    ev.push(WorldEvent::ApplyDamageChar { char: c, amount: 99999.0 });
                }
            }
            (TriggerKind::SpawnProtection { team }, _) => {
                if !v.is_horse {
                    ev.push(WorldEvent::TeamArea { char: c, team: *team, entered: true });
                }
            }
            (TriggerKind::Containment { kill_objective }, _) => {
                if v.actor.is_some() && v.actor == *kill_objective {
                    ev.push(WorldEvent::OutOfBounds { char: c, volume: id, entered: false }); // LeftOutOfBoundsArea @184
                }
            }
            (TriggerKind::LedgePusher { yaw }, _) => {
                if authority && v.is_demon_horde_enemy {
                    ev.push(WorldEvent::LedgePushing { char: c, yaw: *yaw });
                }
            }
            (TriggerKind::TreeBreaker, _) => {
                if authority && !v.is_horse && self.next_at.is_none() {
                    self.queue = sphere_actors().into();
                    self.next_at = Some(now);
                }
            }
            (TriggerKind::DoorActivator, _) => {
                if authority && v.is_demon_horde_character && self.next_at.is_none() {
                    self.queue = sphere_actors().into();
                    self.next_at = Some(now);
                }
            }
            (TriggerKind::Impaler { force_mult, dismember_weight, bones, skip_dead }, _) => {
                if authority {
                    // ProcessHit: bForceRagdollIfDmgAgent, RagdollForceMultIfDmgAgent, DealDamage(1000):
                    // MordhauTakeDamage(1000, hit, 0, 0, Source = the character, Agent = self)
                    ev.push(WorldEvent::ForceRagdollIfDmgAgent { char: c, force_mult: *force_mult });
                    if !(*skip_dead && v.dead) {
                        ev.push(WorldEvent::TakeDamage { char: c, amount: 1000.0, damage_type: 0, sub_type: 0, source: Some(c), agent: Some(id) });
                    }
                }
                // FX (runs on every machine; the dismember draws use the shared rand stream): MordhauCharacters only
                if !v.is_horse {
                    for b in bones.iter() {
                        if random_bool_with_weight(rng, *dismember_weight) {
                            ev.push(WorldEvent::QueueDismember { char: c, bone: b });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, id: ActorId, comp: &str, v: &CharView, authority: bool, ev: &mut Vec<WorldEvent>) {
        let c = v.id;
        match (&self.kind, comp) {
            (TriggerKind::OutOfBounds, _) => ev.push(WorldEvent::OutOfBounds { char: c, volume: id, entered: false }),
            (TriggerKind::OobBarrier | TriggerKind::FlameBarrier, "OOB") => ev.push(WorldEvent::OutOfBounds { char: c, volume: id, entered: false }),
            (TriggerKind::SpawnProtection { team }, _) => {
                if !v.is_horse {
                    ev.push(WorldEvent::TeamArea { char: c, team: *team, entered: false });
                }
            }
            (TriggerKind::Containment { kill_objective }, _) => {
                if v.actor.is_some() && v.actor == *kill_objective {
                    ev.push(WorldEvent::OutOfBounds { char: c, volume: id, entered: true }); // EnteredOutOfBoundsArea @400
                }
            }
            (TriggerKind::LedgePusher { .. }, _) => {
                if authority && v.is_demon_horde_enemy {
                    ev.push(WorldEvent::LedgePusherFallDamage { char: c });
                }
            }
            _ => {}
        }
    }

    /// BP_ImpalementVolume Box hit (authority; @1336..@1490, ProcessHit / DealDamage). The instant-kill variant deals
    /// 1000 to any AdvancedCharacter. The other: not dead, the character's PreviousVelocity (in knockback: its 2D
    /// direction x 1200) in the Box's frame; X <= -VelocityToKill -> 1000, X < -320 -> 10. At most once per 0.5 s
    /// (LastDealtDamageTime). DealDamage: MordhauTakeDamage(Amount, hit, 0, 0, Source = the character, Agent = self),
    /// then while alive Knockback(forward 2D x 600)
    pub fn hit(&mut self, id: ActorId, v: &CharView, now: f64, ev: &mut Vec<WorldEvent>) {
        let TriggerKind::Impalement { instant, velocity_to_kill } = self.kind.clone() else { return };
        if self.last_damage + 0.5 > now {
            return;
        }
        let amount = if instant {
            Some(1000.0)
        } else {
            if v.dead {
                return;
            }
            let mut vel = v.velocity;
            if v.in_knockback && !v.is_horse {
                let l = (vel[0] * vel[0] + vel[1] * vel[1]).sqrt();
                vel = if l > 1e-4 { [vel[0] / l * 1200.0, vel[1] / l * 1200.0, 0.0] } else { [0.0; 3] };
            }
            // LessLess_VectorRotator: the inverse rotation of the Box's world rotation
            let m = &self.xf.m;
            let len = |c: usize| (m[0][c] * m[0][c] + m[1][c] * m[1][c] + m[2][c] * m[2][c]).sqrt().max(1e-12);
            let x = (m[0][0] * vel[0] + m[1][0] * vel[1] + m[2][0] * vel[2]) / len(0);
            if x <= -velocity_to_kill {
                Some(1000.0)
            } else if x < -320.0 {
                Some(10.0)
            } else {
                None
            }
        };
        let Some(a) = amount else { return };
        self.last_damage = now;
        ev.push(WorldEvent::TakeDamage { char: v.id, amount: a, damage_type: 0, sub_type: 0, source: Some(v.id), agent: Some(id) });
        let f = [self.xf.m[0][0], self.xf.m[1][0]];
        let l = (f[0] * f[0] + f[1] * f[1]).sqrt().max(1e-12);
        ev.push(WorldEvent::KnockbackUnlessDead { char: v.id, impulse: [f[0] / l * 600.0, f[1] / l * 600.0, 0.0] });
    }
}

/// BP_OilCauldron: State 0 idle (interactable) -> 1 pouring (use, or a kick: an AttackMotion with Move 4 by a
/// MordhauCharacter while interactable, @683..@1006) -> after SpawnFireTime a BP_OilFire_C at the Fire component
/// with InstigatorController = the last user's PlayerController, State 2 (@15..@439) -> after ReloadTime State 0
/// (@454..@584)
#[derive(Clone, Debug)]
pub struct Cauldron {
    pub state: u8,
    pub fire_at: Option<f64>,
    pub idle_at: Option<f64>,
    pub last_user: Option<CharId>,
    pub fire_xf: Xf,
}

impl Cauldron {
    pub fn new(pk: &mh_level::Pkgs, class_chain: &[String], xf: &Xf) -> Cauldron {
        let rel = crate::ladder::comp_rel_xf(pk, class_chain, "Fire").unwrap_or(mh_level::xf::IDENTITY);
        Cauldron { state: 0, fire_at: None, idle_at: None, last_user: None, fire_xf: *xf * rel }
    }
    /// Activate(Char): LastUserController (a PlayerController), State = 1
    pub fn activate(&mut self, p: &Props, c: &CharView, now: f64) {
        if self.state != 0 {
            return;
        }
        if c.has_player_controller {
            self.last_user = Some(c.id);
        }
        self.state = 1;
        self.fire_at = Some(now + p.f("SpawnFireTime"));
    }
    pub fn tick(&mut self, id: ActorId, p: &Props, now: f64, ev: &mut Vec<WorldEvent>) {
        if self.fire_at.is_some_and(|t| now >= t) {
            self.fire_at = None;
            ev.push(WorldEvent::SpawnAt { by: id, class: "BP_OilFire_C".into(), xf: self.fire_xf, instigator: self.last_user });
            self.state = 2;
            self.idle_at = Some(now + p.f("ReloadTime"));
        }
        if self.idle_at.is_some_and(|t| now >= t) {
            self.idle_at = None;
            self.state = 0;
        }
    }
}

/// a 2D unit vector helper for hosts
pub fn dir2(v: V) -> V {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if l > 1e-8 { [v[0] / l, v[1] / l, 0.0] } else { [0.0; 3] }
}

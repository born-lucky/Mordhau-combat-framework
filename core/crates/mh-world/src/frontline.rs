//! Frontline objectives (game mode Battle): BP_FrontlineDestroyable (Blueprints/GameModes/Battle/; parent
//! BP_DestroyableActor), the delivery spots BP_ItemDeliverySpot (+ BP_MorphTargetDeliverySpot / the cog / burnable
//! spots), BP_FrontlinePushable (native APushableActor, `pushable.rs`) and BP_FrontlineKillObjectiveWrapper, as the
//! capture point (`capture.rs`) sees them: GetObjectiveProgress, IsCompleted, OnAnyObjectiveProgressChanged,
//! prerequisites, damage rules, scores.
//! Bytecode: state/world_kismet/BP_FrontlineDestroyable.txt, BP_ItemDeliverySpot.txt, BP_FrontlinePushable.txt,
//! BP_FrontlineKillObjectiveWrapper.txt.

use crate::destructible::{Destructible, Ov};
use crate::props::{nearly_equal, Props};
use crate::pushable::Pushable;
use crate::world::{ActorId, CharId, DamageInfo, WorldEvent};

/// EScoreFeedReason the Frontline objectives award (GiveClientScoreBP(12, ...): destroyable @1301 / @1430, delivery
/// @490)
pub const SCORE_OBJECTIVE: u8 = 12;

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectiveKind {
    /// BP_FrontlineDestroyable: health is progress
    Destroyable,
    /// BP_ItemDeliverySpot: Progress = Deliverables / RequiredDeliveries (OnRep_Deliverables@30..@150), fed by `deliver`
    Delivery,
    /// BP_FrontlinePushable: Progress from APushableActor::Tick (`push`)
    Pushable,
    /// BP_FrontlineKillObjectiveWrapper: KillObjective (a BP_FrontlineKillObjective character, the character / mode
    /// side's) progress x Weight; no KillObjective = progress Weight and completed (GetObjectiveProgress@166,
    /// IsCompleted@113); the host reports it with `World::set_objective_progress`
    KillWrapper,
}

#[derive(Clone, Debug)]
pub struct Objective {
    pub kind: ObjectiveKind,
    pub base: Destructible,
    pub capture_point: Option<ActorId>,
    /// AActor::bCanBeDamaged: false on the CDO, true while the enemy has its prerequisites
    /// (EnemyGainedPrerequisites@20 / EnemyLostPrerequisites@20)
    pub can_be_damaged: bool,
    /// LastDamagedByPlayerController (@2032 / @2052): the instigating player, None for AI / no controller
    pub last_damager: Option<Option<CharId>>,
    pub last_damager_team: Option<u8>,
    /// CapturePointObjectivesCompleted (OnAnyObjectiveProgressChanged @2935)
    pub cp_completed: bool,
    pub score_per_destroy: i64,
    /// Progress (delivery / kill wrapper progress 0..1)
    pub progress: f64,
    /// delivery spots: Deliverables delivered so far; Area collision on (Activate / Disable)
    pub deliverables: u8,
    pub active: bool,
    /// kill wrapper: the KillObjective is gone or its IsCompleted
    pub kill_done: bool,
    /// kill wrapper: its KillObjective when it is a placed actor of this world (else the host reports progress)
    pub kill_target: Option<ActorId>,
    pub push: Option<Box<Pushable>>,
    /// delivery spot: its DeliverySpawns (BP_ItemDeliverySpawn actors) and whether they are active (ActivateSpawns on
    /// EnemyGainedPrerequisites@56 unless the point is done, DisableSpawns on EnemyLostPrerequisites@15 / a done
    /// point after a delivery, OnRep_Deliverables@350)
    pub delivery_spawns: Vec<ActorId>,
    pub spawns_active: bool,
}

impl Objective {
    pub fn destroyable(p: &Props) -> Objective {
        Objective {
            kind: ObjectiveKind::Destroyable,
            base: Destructible::new(p),
            capture_point: None,
            can_be_damaged: p.b("bCanBeDamaged"),
            last_damager: None,
            last_damager_team: None,
            cp_completed: false,
            score_per_destroy: p.i("ScoreAwardedPerDestroy"),
            progress: 0.0,
            deliverables: 0,
            active: false,
            kill_done: false,
            kill_target: None,
            push: None,
            delivery_spawns: vec![],
            spawns_active: false,
        }
    }
    pub fn pushable(p: &Props, push: Pushable) -> Objective {
        Objective { kind: ObjectiveKind::Pushable, progress: push.progress, push: Some(Box::new(push)), ..Objective::destroyable(p) }
    }
    pub fn delivery(p: &Props) -> Objective {
        let n = p.i("Deliverables").clamp(0, 255) as u8;
        let mut o = Objective { kind: ObjectiveKind::Delivery, deliverables: n, ..Objective::destroyable(p) };
        o.progress = Self::delivery_progress(n, p);
        o
    }
    pub fn kill_wrapper(p: &Props) -> Objective {
        let none = p.obj("KillObjective").is_empty();
        Objective { kind: ObjectiveKind::KillWrapper, kill_done: none, progress: if none { 1.0 } else { 0.0 }, ..Objective::destroyable(p) }
    }
    fn delivery_progress(n: u8, p: &Props) -> f64 {
        let r = p.i("RequiredDeliveries") as f64;
        // UKismetMathLibrary::Divide_FloatFloat returns 0 for a 0 divisor
        if r > 0.0 { n as f64 / r } else { 0.0 }
    }

    /// BP_ItemDeliverySpot Area overlap (ExecuteUbergraph@159..@556), authority: while Deliverables < RequiredDeliveries,
    /// an unconsumed BP_DeliverableEquipment of the spot's Type is consumed, its LastEquippedByPlayerController (when
    /// valid) scores (12, ScoreAwardPerDelivery), Deliverables += 1; OnRep_Deliverables: Progress, Disable at the
    /// requirement, CapturePoint.ObjectivesChanged. Returns whether the item was consumed (the host destroys it: Consume)
    pub fn deliver(&mut self, p: &Props, item_type: i64, scorer: Option<CharId>, scorer_team: Option<u8>, ev: &mut Vec<WorldEvent>) -> bool {
        if self.kind != ObjectiveKind::Delivery || !self.active {
            return false; // Area collision off: no overlap events (Activate@52 / Disable@0)
        }
        if (self.deliverables as i64) >= p.i("RequiredDeliveries") || item_type != p.i("Type") {
            return false;
        }
        if scorer.is_some() {
            ev.push(WorldEvent::Score { char: scorer, team: scorer_team, kind: SCORE_OBJECTIVE, amount: p.i("ScoreAwardPerDelivery") });
        }
        self.deliverables = self.deliverables.saturating_add(1); // Add_ByteByte @88
        self.progress = Self::delivery_progress(self.deliverables, p);
        if (self.deliverables as i64) >= p.i("RequiredDeliveries") {
            self.active = false; // Disable(true) @226
        }
        if let Some(cp) = self.capture_point {
            ev.push(WorldEvent::ObjectivesChanged { capture_point: cp });
        }
        true
    }

    /// OnEnemyGainedPrerequisites / OnEnemyLostPrerequisites (the capture point's loops @7572 / @817). `cp_done` =
    /// CapturePoint.ObjectiveProgress == 1. Destroyables toggle bCanBeDamaged (@20); delivery spots Activate
    /// (IsCapturePointDone false and Deliverables < RequiredDeliveries, Activate@0) / Disable; pushables
    /// bIsPushingAllowed (EnemyGainedPrerequisites@118 unless cp_done / EnemyLostPrerequisites@0)
    pub fn set_prerequisites(&mut self, p: &Props, gained: bool, cp_done: bool) {
        match self.kind {
            ObjectiveKind::Destroyable => self.can_be_damaged = gained,
            ObjectiveKind::Delivery => {
                if !gained {
                    self.active = false;
                    self.spawns_active = false;
                } else if !cp_done {
                    if (self.deliverables as i64) < p.i("RequiredDeliveries") {
                        self.active = true;
                    }
                    self.spawns_active = true;
                }
            }
            ObjectiveKind::Pushable => {
                if let Some(pu) = self.push.as_mut() {
                    if !gained {
                        pu.pushing_allowed = false;
                    } else if !cp_done {
                        pu.pushing_allowed = true;
                    }
                }
            }
            // forwarded to the KillObjective (@180 / @265): the character side's
            ObjectiveKind::KillWrapper => {}
        }
    }

    /// OnAnyObjectiveProgressChanged, after the capture point recomputed ObjectiveProgress (`cp_progress`):
    /// destroyable: at 1 CapturePointObjectivesCompleted and no destroy score (@2869..@2946); delivery spot
    /// (AnyObjectiveProgressChanged@0..@752): at 1, or once every one of the point's DeliverySpots reports progress 1,
    /// EnemyLostPrerequisites; pushable (@706..@811): at 1 EnemyLostPrerequisites. `spots_done` = every valid
    /// DeliverySpot's GetObjectiveProgress == 1
    pub fn on_any_objective_progress_changed(&mut self, p: &Props, cp_progress: f64, spots_done: bool) {
        match self.kind {
            ObjectiveKind::Destroyable => {
                if cp_progress == 1.0 {
                    self.cp_completed = true;
                    self.score_per_destroy = 0;
                }
            }
            ObjectiveKind::Delivery => {
                if cp_progress == 1.0 || spots_done {
                    self.set_prerequisites(p, false, cp_progress == 1.0);
                }
            }
            ObjectiveKind::Pushable => {
                if cp_progress == 1.0 {
                    self.set_prerequisites(p, false, true);
                }
            }
            ObjectiveKind::KillWrapper => {}
        }
    }

    /// GetObjectiveProgress: destroyable (1 - ReplicatedHealth / MaxHealth) * ObjectiveWeight; delivery / pushable
    /// Progress * ObjectiveWeight; kill wrapper progress * Weight
    pub fn progress(&self, p: &Props) -> f64 {
        let w = p.f("ObjectiveWeight");
        match self.kind {
            ObjectiveKind::KillWrapper => self.progress * p.f("Weight"),
            ObjectiveKind::Destroyable => {
                let m = p.f("MaxHealth");
                // Divide_FloatFloat: 0 for a 0 divisor
                (1.0 - if m != 0.0 { self.base.replicated as f64 / m } else { 0.0 }) * w
            }
            ObjectiveKind::Pushable => self.push.as_ref().map(|x| x.progress).unwrap_or(self.progress) * w,
            ObjectiveKind::Delivery => self.progress * w,
        }
    }

    /// IsCompleted: ReplicatedHealth == 0 / Progress == 1
    pub fn completed(&self) -> bool {
        match self.kind {
            ObjectiveKind::Destroyable => self.base.replicated == 0,
            ObjectiveKind::Pushable => self.push.as_ref().map(|x| x.progress).unwrap_or(self.progress) == 1.0,
            ObjectiveKind::Delivery => self.progress == 1.0,
            ObjectiveKind::KillWrapper => self.kill_done,
        }
    }

    pub fn set_progress(&mut self, v: f64) {
        self.progress = v.clamp(0.0, 1.0);
        if let Some(pu) = self.push.as_mut() {
            pu.progress = self.progress;
        }
    }

    /// ReceiveAnyDamage (ExecuteUbergraph@1938..@2746): only while bCanBeDamaged; LastDamagedByPlayerController =
    /// the instigator when it is a MordhauPlayerController (@1953..@2052); with a capture point whose
    /// bObjectivesCompleted is false (`cp_done`, @2107) the damage needs a BP_CircleSawProgressActor causer or an
    /// instigator with a MordhauPlayerState whose Team is not the point's OwningTeam (@2407..@2746; no instigator /
    /// player state = ignored); each hit is capped at MaxDamagePerHit / DamageFactor (0 when DamageFactor ~ 0, @2143..
    /// @2270); then the BP_DestroyableActor path, and OnReplicatedHealthChanged's scores to the last damaging player
    /// (FTrunc((LastReplicatedHealth - ReplicatedHealth) * ScoreDamageMultiplier) when > 0, @851..@1301;
    /// ScoreAwardedPerDestroy at 0 when > 0, @1430) and CapturePoint.ObjectivesChanged (@1513)
    #[allow(clippy::too_many_arguments)]
    pub fn receive_any_damage(
        &mut self,
        id: ActorId,
        p: &Props,
        amount: f64,
        d: &DamageInfo,
        cp_team: Option<u8>,
        cp_done: bool,
        now: f64,
        ov: Ov,
        ev: &mut Vec<WorldEvent>,
    ) {
        if self.kind != ObjectiveKind::Destroyable || !self.can_be_damaged {
            return;
        }
        self.last_damager = Some(if d.instigator_player { d.instigator } else { None });
        self.last_damager_team = d.instigator_team;
        if self.capture_point.is_some() && !cp_done && !d.circle_saw {
            match d.instigator_team {
                None => return, // @2520 / @2617
                // Conv_ByteToInt(OwningTeam) == Team (255 for no owner never matches a real team)
                Some(t) if Some(t) == cp_team => return, // @2746
                _ => {}
            }
        }
        let f = p.f("DamageFactor");
        let cap = if nearly_equal(f, 0.0, 9.999999974752427e-07) { 0.0 } else { p.f("MaxDamagePerHit") / f };
        let before = self.base.replicated;
        self.base.receive_any_damage(id, p, amount.min(cap), d, now, ov, ev);
        let after = self.base.replicated;
        if after != before {
            if let Some(Some(c)) = self.last_damager {
                let pts = ((before as f64 - after as f64) * p.f("ScoreDamageMultiplier")).trunc() as i64;
                if pts > 0 {
                    ev.push(WorldEvent::Score { char: Some(c), team: self.last_damager_team, kind: SCORE_OBJECTIVE, amount: pts });
                }
                if after == 0 && self.score_per_destroy > 0 {
                    ev.push(WorldEvent::Score { char: Some(c), team: self.last_damager_team, kind: SCORE_OBJECTIVE, amount: self.score_per_destroy });
                }
            }
            if let Some(cp) = self.capture_point {
                ev.push(WorldEvent::ObjectivesChanged { capture_point: cp });
            }
        }
    }
}

/// BP_FrontlineKillObjective (Blueprints/GameModes/Battle/; a BP_MordhauCharacter: the Frontline nobles / commanders a
/// BP_FrontlineKillObjectiveWrapper points at). The character itself (health, death) is the combat side's; this is
/// its objective layer, from state/world_kismet/BP_FrontlineKillObjective.txt:
/// GetObjectiveProgress = Destroyed or dead ? 1 : 1 - Health / 100 (@0..@221); IsCompleted = dead (@0);
/// ReceiveAnyDamage scores (@3021..@3428), OnHealthChanged / OnDied / ReceiveDestroyed call Point.ObjectivesChanged
/// (@4569 / @4164 / @5887).
#[derive(Clone, Debug)]
pub struct KillObjective {
    /// AAdvancedCharacter::Health (byte; the ctor's 100, AAdvancedCharacter::AAdvancedCharacter rva 0x144b0e0)
    pub health: u8,
    pub dead: bool,
    pub destroyed: bool,
    pub awarded_kill_points: bool,
    pub objective_was_completed: bool,
    /// Point (OnInitialize from the wrapper, @3848)
    pub point: Option<ActorId>,
}

impl KillObjective {
    pub fn new(p: &Props) -> KillObjective {
        let h = p.get("Health").map(|_| p.i("Health")).unwrap_or(100);
        KillObjective { health: h.clamp(0, 255) as u8, dead: false, destroyed: false, awarded_kill_points: false, objective_was_completed: false, point: None }
    }

    /// GetObjectiveProgress (@0..@221)
    pub fn progress(&self) -> f64 {
        if self.destroyed || self.dead {
            1.0
        } else {
            1.0 - self.health as f64 / 100.0
        }
    }

    /// AwardScorePointsIfApplicable(Instigator, Points) (@0..@452): Points > 0, Point valid, an instigator with a
    /// MordhauPlayerState whose Team is not Point.OwningTeam, a MordhauPlayerController -> GiveClientScoreBP(12, Points)
    fn award(&self, points: i64, d: &DamageInfo, point_owner: Option<u8>, ev: &mut Vec<WorldEvent>) {
        if points <= 0 || self.point.is_none() {
            return;
        }
        let Some(t) = d.instigator_team else { return };
        if Some(t) == point_owner || !d.instigator_player {
            return;
        }
        ev.push(WorldEvent::Score { char: d.instigator, team: Some(t), kind: SCORE_OBJECTIVE, amount: points });
    }

    /// ReceiveAnyDamage (@3021..@3428) around the character's own damage: Damage >= 1 and no kill points yet ->
    /// FTrunc(ScorePerDamageMultiplier * Damage); the parent applies it (`health_after` / `dead_after`, the combat
    /// side's result); dead and no kill points yet -> ScorePerKill, AwardedKillPoints. Then OnHealthChanged (alive:
    /// Point.ObjectivesChanged @4569) or OnDied (@4164)
    #[allow(clippy::too_many_arguments)]
    pub fn damaged(&mut self, p: &Props, damage: f64, d: &DamageInfo, health_after: u8, dead_after: bool, point_owner: Option<u8>, ev: &mut Vec<WorldEvent>) {
        if damage >= 1.0 && !self.awarded_kill_points {
            self.award((p.f("ScorePerDamageMultiplier") * damage).trunc() as i64, d, point_owner, ev);
        }
        let was_dead = self.dead;
        self.health = health_after;
        self.dead = dead_after;
        if self.dead && !self.awarded_kill_points {
            self.award(p.i("ScorePerKill"), d, point_owner, ev);
            self.awarded_kill_points = true;
        }
        if let Some(cp) = self.point {
            if !was_dead {
                ev.push(WorldEvent::ObjectivesChanged { capture_point: cp });
            }
        }
    }

    /// OnAnyObjectiveProgressChanged (@5619..@5815): alive and Point.ObjectiveProgress == 1 -> ObjectiveWasCompleted,
    /// AwardedKillPoints (no more score)
    pub fn on_any_objective_progress_changed(&mut self, cp_progress: f64) {
        if !self.dead && cp_progress == 1.0 {
            self.objective_was_completed = true;
            self.awarded_kill_points = true;
        }
    }

    /// ReceiveDestroyed (@5832): Destroyed = true, Point.ObjectivesChanged
    pub fn destroyed(&mut self, ev: &mut Vec<WorldEvent>) {
        self.destroyed = true;
        if let Some(cp) = self.point {
            ev.push(WorldEvent::ObjectivesChanged { capture_point: cp });
        }
    }
}

//! AMordhauAIController (godot/game/ai/bot_controller.gd): owns the bot's BehaviorProfile, its behavior tree and
//! blackboard, and turns the tasks' decisions into fighter input (BotHost::request_attack / request_parry /
//! request_feint, the same entry points a player's input uses) plus movement / facing / crouch requests the host applies.
//! Source: extract/native/decomp/AMordhauAIController.cpp, layout extract/native/types/AMordhauAIController.h.
//! `Bots` is the reference's BotDuel hookup (godot/game/ai/bot_duel.gd): bodies + bots + one CRT rand() stream.

use super::body::{BodyId, BotBody, BotHost, PawnView, Sense};
use super::bt::{BbVal, BtTree, TreeDef};
use super::profile::{BehaviorProfile, Profile};
use crate::consts::bot as K;
use crate::game_mode::Precision;
use crate::kismet::Kismet;
use std::sync::Arc;
use mordhau_core::ue::{CrtRand, FVector};
use std::collections::BTreeMap;

/// +0x36c, set by the StartFacing* functions
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    Movement = 0,
    Location = 1,
    Actor = 2,
    Actor2D = 3,
    Bone = 4,
}

/// EPathFollowingStatus (UE 4.26)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStatus {
    Idle = 0,
    Waiting = 1,
    Paused = 2,
    Moving = 3,
}

/// BP_MordhauAIController CDO BehaviorTree (extract/json), used by make()
pub const DEFAULT_TREE: &str = "BT_Deathmatch";

// Sense configs, AMordhauAIController ctor rva=0x14e3bf0 (stores into the SenseConfigSight / SenseConfigHearing /
// SenseConfigDamage subobjects; BP_MordhauAIController's subobjects override none; UE 4.26 field names UNCONFIRMED):
//   sight   MaxAge (+0x2c) 15, SightRadius (+0x50) 2600, LoseSightRadius (+0x54) 3000, PeripheralVisionAngle (+0x58) 65
//   hearing MaxAge 15, HearingRange (+0x50) 2000, LoSHearingRange (+0x54) 400
//   damage  MaxAge 30
//   PerceptionUpdateInterval (+0x334) 1, NotPerceivedTimeToForget (+0x338) 5
pub const SIGHT_RADIUS: f64 = 2600.0;
pub const LOSE_SIGHT_RADIUS: f64 = 3000.0;
pub const PERIPHERAL_VISION_ANGLE: f64 = 65.0;
pub const SIGHT_MAX_AGE: f64 = 15.0;
pub const HEARING_MAX_AGE: f64 = 15.0;
pub const HEARING_LOS_RANGE: f64 = 400.0;
/// HearingConfig HearingRange (ctor, decomp AMordhauAIController.cpp:1883: 0x44fa0000 = 2000)
pub const HEARING_RANGE: f32 = 2000.0;
pub const DAMAGE_MAX_AGE: f64 = 30.0;
pub const PERCEPTION_UPDATE_INTERVAL: f64 = 1.0;
/// AMordhauAIController ctor rva=0x14e3bf0 MidPointAcceptanceRadius (+0x600)
pub const MID_POINT_ACCEPTANCE_RADIUS: f64 = 10.0;

/// FPerceptionInfo {bSight +0, bHearing +1, bDamage +2, Team +3, UpdateTime +4}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PerceptionInfo {
    pub sight: bool,
    pub hearing: bool,
    pub damage: bool,
    pub team: i64,
    pub update_time: f64,
}

impl PerceptionInfo {
    pub fn sensed(&self) -> bool {
        self.sight || self.hearing || self.damage
    }
}

/// a voice / emote / equip request for the host
#[derive(Clone, Debug, PartialEq)]
pub struct BotEvent {
    pub t: f64,
    pub kind: &'static str,
    pub id: i64,
    pub forced: bool,
}

/// the last fighter input the bot sent
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Attack { t: f64, mv: i64, angle: f64 },
    Parry { t: f64, bt: i64 },
    Feint { t: f64 },
}

/// AAIController::MoveToLocation(dest, acceptance): recorded for the host's path following
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveRequest {
    pub dest: FVector,
    pub acceptance: f64,
    pub t: f64,
}

/// a decision-trace row {t, task, step, fn}
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub t: f64,
    pub task: &'static str,
    pub step: String,
    pub func: &'static str,
}

/// shared between the controllers of one world (VoiceOrEmote cooldowns: game mode +0x770 voice, +0x774 emote)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldState {
    pub last_voice: f64,
    pub last_emote: f64,
    /// BP_HordeGameMode KillObjectiveCached (the kill-objective actor's body), read by BTService_HordePerceptionUpdate
    pub kill_objective: Option<BodyId>,
}

impl Default for WorldState {
    fn default() -> Self {
        WorldState { last_voice: f64::NEG_INFINITY, last_emote: f64::NEG_INFINITY, kill_objective: None }
    }
}

/// what a controller's frame may touch: every body, the other bots (an ally's LastClosestEnemy), the world's rand()
/// stream, the shared cooldowns, the host, the world time (GetWorld()->TimeSeconds)
pub struct Ctx<'a> {
    pub bodies: &'a mut [BotBody],
    pub others: &'a [Option<BotController>],
    pub rng: &'a mut CrtRand,
    pub ws: &'a mut WorldState,
    pub host: &'a mut dyn BotHost,
    /// the Blueprint bytecode literals (BTTask_BackOff_C / FindRandomLocation_C / MoveToDestination_C: ai_*)
    pub k: &'a Kismet,
    pub now: f64,
    /// the float rounding of the numeric model (binary32 in exe mode, identity for the reference)
    pub q: fn(f64) -> f64,
    /// Precision::Exe: the exe's own trig where the call site is disassembled (ue_math)
    pub exe: bool,
}

#[derive(Clone, Debug)]
pub struct BotController {
    pub body: BodyId,
    pub profile: BehaviorProfile,
    pub tree: Option<BtTree>,
    pub blackboard: BTreeMap<String, BbVal>,
    /// the world's AMordhauGameState bIsTeamMode (+0x6b0), read by perception
    pub team_mode: bool,
    /// +0x328 last seen NetMotion Id (-1 = none seen)
    pub net_seen: i64,
    /// +0x32c
    pub motion_random: f64,
    pub facing_mode: Facing,
    /// +0x33c weak ptr (StartFacingActor / StartFacingBone)
    pub facing_actor: Option<BodyId>,
    /// +0x35c
    pub facing_location: FVector,
    /// +0x354 (yaw, pitch offset for StartFacingBone / Actor), Vector2
    pub facing_offset: [f32; 2],
    /// +0x368: StartFacingMovement / StartFacingActor argument, StartFacingBone bone height
    pub facing_param: f64,
    /// +0x404
    pub facing_since: f64,
    /// RotationTargetInterpolated (pitch, yaw, roll), LODTick's smoothed facing rotation (initial zero: UNCONFIRMED)
    pub rotation_target_interpolated: [f32; 3],
    pub move_request: Option<MoveRequest>,
    /// AAIController::GetMoveStatus: the host sets Idle when a move completes (on_move_completed)
    pub path_status: PathStatus,
    /// SetClosestEnemyOverride rva=0x1515b50
    pub closest_enemy_override: Option<BodyId>,
    /// voice component +0xe8 (initial value UNCONFIRMED)
    pub last_voice_time: f64,
    pub events: Vec<BotEvent>,
    pub last_request: Option<Request>,
    pub trace: Vec<Note>,
    pub record_trace: bool,
    /// PerceivedCharacters +0x390 (TMap; insertion order)
    pub perceived: Vec<(BodyId, PerceptionInfo)>,
    /// the sight sense's state per target (Precision::Exe without a host stimulus source): (target, currently seen,
    /// time last seen)
    pub sight_state: Vec<(BodyId, bool, f64)>,
    /// the hearing sense's last stimulus per instigator (Bots::report_noise): (instigator, world time, noise location)
    pub heard: Vec<(BodyId, f64, FVector)>,
    /// OnStartedPerceivingCharacter / OnStoppedPerceivingCharacter (BP events): (started, who)
    pub perception_events: Vec<(bool, BodyId)>,
    died_seen: Vec<BodyId>,
    /// +0x3f4
    pub last_closest_enemy: Option<BodyId>,
    /// +0x400 (UWorld RealTimeSeconds base)
    pub last_closest_enemy_changed_time: f64,
    /// RealTimeSeconds = now + real_time_offset (port adaptation, offset = the 1 s recheck interval so a bot sees
    /// enemies from its first frame; the real level time at the first spawn is UNCONFIRMED)
    pub real_time_offset: f64,
    /// +0x3ec ReallyCloseEnemyCached
    pub really_close_enemy: Option<BodyId>,
    /// +0x3fc bIsClosestEnemySaturated
    pub closest_enemy_saturated: bool,
    /// +0x410 ClosestEnemyIgnoreSet (never filled by the exe outside the ctor: UNCONFIRMED)
    pub closest_enemy_ignore: Vec<BodyId>,
    /// the host's MordhauPlayerState Team (None = none)
    pub player_state_team: Option<i64>,
    /// bMovePending (+0x5b8)
    pub move_pending: bool,
    pub logic_paused: bool,
    /// RandomFloat (+0x468) / NextRandomFloatAssignment (+0x46c)
    pub random_float: f64,
    pub next_random_float_assignment: f64,
    /// PendingReq (+0x5c0): (dest, acceptance)
    pub pending_move: Option<(FVector, f64)>,
}

impl BotController {
    /// plain construction (the reference's _init); make() adds BeginPlay
    pub fn new(body: BodyId, profile: BehaviorProfile, tree: Option<BtTree>) -> BotController {
        BotController {
            body,
            profile,
            tree,
            blackboard: BTreeMap::new(),
            team_mode: false,
            net_seen: -1,
            motion_random: 0.0,
            facing_mode: Facing::Movement,
            facing_actor: None,
            facing_location: FVector::ZERO,
            facing_offset: [0.0, 0.0],
            facing_param: 0.0,
            facing_since: 0.0,
            rotation_target_interpolated: [0.0; 3],
            move_request: None,
            path_status: PathStatus::Idle,
            closest_enemy_override: None,
            last_voice_time: 0.0,
            events: Vec::new(),
            last_request: None,
            trace: Vec::new(),
            record_trace: true,
            perceived: Vec::new(),
            sight_state: Vec::new(),
            heard: Vec::new(),
            perception_events: Vec::new(),
            died_seen: Vec::new(),
            last_closest_enemy: None,
            last_closest_enemy_changed_time: 0.0,
            real_time_offset: K::CLOSEST_ENEMY_RECHECK_S,
            really_close_enemy: None,
            closest_enemy_saturated: false,
            closest_enemy_ignore: Vec::new(),
            player_state_team: None,
            move_pending: false,
            logic_paused: false,
            random_float: 0.0,
            next_random_float_assignment: 0.0,
            pending_move: None,
        }
    }

    pub fn note(&mut self, cx: &Ctx, task: &'static str, step: String, func: &'static str) {
        if self.record_trace {
            self.trace.push(Note { t: cx.now, task, step, func });
        }
    }

    fn me<'a>(&self, cx: &'a Ctx) -> &'a BotBody {
        &cx.bodies[self.body]
    }

    /// one AI frame: the BT component tick (BtTree::tick follows UBehaviorTreeComponent::TickComponent's order)
    pub fn tick(&mut self, dt: f64, cx: &mut Ctx) {
        if self.me(cx).is_dead || self.tree.is_none() || self.logic_paused {
            return;
        }
        self.random_float_tick(cx);
        self.update_perception(cx);
        let mut t = self.tree.take().unwrap();
        t.tick(self, cx, dt);
        self.tree = Some(t);
    }

    // ---- AMordhauAIController --------------------------------------------------------------------------------------
    /// from AMordhauAIController::GetMotionBasedRandom rva=0x14fc660: when the pawn's MotionSystem NetMotion Id (+0xc4)
    /// differs from the one seen last (+0x328): remember it, MotionRandom (+0x32c) = FRand, BehaviorProfile->
    /// RerollRandomInstanceValues(). Returns +0x32c (disasm 0x1414fc6ef). The Id is incremented on every
    /// AssignNetMotion / AssignNetMotionSimple (UMotionSystemComponent.cpp: `+0xc4 + 1`), not on the return to idle.
    pub fn get_motion_based_random(&mut self, cx: &mut Ctx) -> f64 {
        let me = &cx.bodies[self.body];
        if me.has_sys && !me.is_dead {
            let n = me.pawn.motion.net_id;
            if n != self.net_seen {
                self.net_seen = n;
                self.motion_random = cx.rng.frand(K::FRAND_SCALE);
                self.profile.reroll_q(cx.rng, cx.q);
                let s = format!("new own motion -> reroll (random {:.4})", self.motion_random);
                self.note(cx, "Controller", s, "AMordhauAIController::GetMotionBasedRandom rva=0x14fc660");
            }
        }
        self.motion_random
    }

    // ---- perception ------------------------------------------------------------------------------------------------
    // The engine's UAIPerceptionComponent calls OnPerceptionUpdated with the actors whose stimuli changed;
    // UpdatePerceptionInfo reads each actor's last sensed stimuli. The engine side (UAISense_Sight / Hearing / Damage
    // producing stimuli) is not ported: the stimulus source is the host (BotHost::stimuli), default a fresh sight
    // stimulus from every other live character (UNCONFIRMED stand-in = everyone in sight), and tick() reports every
    // character as updated each AI frame (UNCONFIRMED: the engine reports only changes; the 1 s
    // PerceptionUpdateInterval gate keeps the per-entry refresh rate the same).

    /// FAIStimulus age test as UpdatePerceptionInfo rva=0x151e630 inlines it: age = Strength (+8) > 0 ? Age (+0) :
    /// FLT_MAX; sensed when age < MaxAge (MaxAge 0 -> FLT_MAX)
    fn stim_sensed(age: f64, strength: f64, max_age: f64) -> bool {
        let a = if strength > 0.0 { age } else { f64::INFINITY };
        a < if max_age != 0.0 { max_age } else { f64::INFINITY }
    }

    /// AMordhauAIController::UpdatePerceptionInfo rva=0x151e630: for each of the character's last sensed stimuli, by
    /// sense: sight -> bSight = sensed(SightConfig MaxAge); hearing -> bHearing = sensed(HearingConfig MaxAge), and a
    /// heard stimulus farther than LoSHearingRange (squared distance receiver -> stimulus location) whose visibility
    /// trace is blocked is not heard; damage -> bDamage = sensed(DamageConfig MaxAge). Then Team = the character's Team
    /// byte, UpdateTime = world TimeSeconds. A sense with no stimulus keeps its old flag.
    fn update_perception_info(&mut self, b: BodyId, info: &mut PerceptionInfo, cx: &mut Ctx) {
        let sts = match cx.host.stimuli(self.body, b) {
            Some(s) => s,
            None if cx.exe => {
                let mut v = vec![self.sight_stimulus(b, cx)];
                // the hearing sense's last stimulus of b (Bots::report_noise), aged since it was heard
                if let Some(&(_, t, loc)) = self.heard.iter().find(|h| h.0 == b) {
                    let me = &cx.bodies[self.body];
                    let eye = FVector::new(me.location.x, me.location.y, me.location.z + me.eye_height as f32);
                    v.push(super::body::Stimulus { sense: Sense::Hearing, age: self.world_time(cx) - t, strength: 1.0, receiver: eye, location: loc });
                }
                v
            }
            None => vec![super::body::Stimulus {
                sense: Sense::Sight,
                age: 0.0,
                strength: 1.0,
                receiver: cx.bodies[self.body].location,
                location: cx.bodies[b].location,
            }],
        };
        for st in sts {
            match st.sense {
                Sense::Sight => info.sight = Self::stim_sensed(st.age, st.strength, SIGHT_MAX_AGE),
                Sense::Hearing => {
                    info.hearing = Self::stim_sensed(st.age, st.strength, HEARING_MAX_AGE);
                    if info.hearing {
                        let far = ((st.receiver - st.location).length_squared() as f64) > HEARING_LOS_RANGE * HEARING_LOS_RANGE;
                        if far && cx.host.hearing_trace_blocked(st.receiver, st.location) == Some(true) {
                            info.hearing = false;
                        }
                    }
                }
                Sense::Damage => info.damage = Self::stim_sensed(st.age, st.strength, DAMAGE_MAX_AGE),
            }
        }
        info.team = cx.bodies[b].team & 0xff;
        info.update_time = self.world_time(cx);
    }

    /// the sight sense (UAISense_Sight, UE 4.26 engine code, not disassembled: UNCONFIRMED in detail) with
    /// Mordhau's SightConfig: a target is seen when within SightRadius (LoseSightRadius while already seen), inside
    /// the PeripheralVisionAngle half-angle of the pawn's view direction (yaw), with a clear line of sight from the
    /// eyes (BotBody eye_height above the location) to the target (BotHost::sight_blocked; None = clear). A lost
    /// target's stimulus ages from its last sighting (UpdatePerceptionInfo then keeps bSight while age < MaxAge 15).
    fn sight_stimulus(&mut self, b: BodyId, cx: &mut Ctx) -> super::body::Stimulus {
        let now = self.world_time(cx);
        let me = &cx.bodies[self.body];
        let eye = FVector::new(me.location.x, me.location.y, me.location.z + me.eye_height as f32);
        let t = cx.bodies[b].location;
        let i = match self.sight_state.iter().position(|s| s.0 == b) {
            Some(i) => i,
            None => {
                self.sight_state.push((b, false, f64::NEG_INFINITY));
                self.sight_state.len() - 1
            }
        };
        let was = self.sight_state[i].1;
        let d = FVector::new(t.x - eye.x, t.y - eye.y, t.z - eye.z);
        let d2 = d.x * d.x + d.y * d.y + d.z * d.z;
        // the ctor's floats (decomp AMordhauAIController.cpp:1867-1869: 0x45228000 / 0x453b8000 / 0x42820000)
        let r = if was { LOSE_SIGHT_RADIUS } else { SIGHT_RADIUS } as f32;
        let fwd = mordhau_core::ue::ue_actor_forward_yaw(me.yaw as f32);
        let l = d2.sqrt().max(1e-4);
        let in_cone = (fwd.x * d.x + fwd.y * d.y + fwd.z * d.z) / l >= (PERIPHERAL_VISION_ANGLE as f32).to_radians().cos();
        let seen = d2 <= r * r && in_cone && cx.host.sight_blocked(eye, t) != Some(true);
        self.sight_state[i].1 = seen;
        if seen {
            self.sight_state[i].2 = now;
        }
        let age = if seen { 0.0 } else { now - self.sight_state[i].2 };
        super::body::Stimulus { sense: Sense::Sight, age, strength: if age.is_finite() { 1.0 } else { 0.0 }, receiver: eye, location: t }
    }

    fn world_time(&self, cx: &Ctx) -> f64 {
        (cx.q)(cx.now + self.real_time_offset)
    }

    fn perceived_index(&self, b: BodyId) -> Option<usize> {
        self.perceived.iter().position(|(o, _)| *o == b)
    }

    pub fn perception(&self, b: BodyId) -> Option<&PerceptionInfo> {
        self.perceived.iter().find(|(o, _)| *o == b).map(|(_, i)| i)
    }

    /// AMordhauAIController::OnPerceptionUpdated rva=0x150cdd0:
    ///   1. each updated actor that is an AAdvancedCharacter, not our pawn, bCanBeDamaged and not yet in the map: bind
    ///      its OnCharacterDied / OnCharacterDestroyed to OnCharacterDiedOrDestroyed, add {false x3, Team 255,
    ///      UpdateTime 0}, remember it as newly perceived
    ///   2. every entry: a gone character -> removed; else when PerceptionUpdateInterval <= now - UpdateTime:
    ///      UpdatePerceptionInfo; no sense left -> unbind, removed, OnStoppedPerceivingCharacter unless newly perceived;
    ///      still sensed and newly perceived -> OnStartedPerceivingCharacter
    /// bCanBeDamaged stand-in: the character is alive (UNCONFIRMED: the flag's writers are not ported).
    pub fn on_perception_updated(&mut self, updated: &[BodyId], cx: &mut Ctx) {
        let mut fresh = Vec::new();
        for &b in updated {
            if b == self.body || cx.bodies[b].is_dead || cx.bodies[b].gone || self.perceived_index(b).is_some() {
                continue;
            }
            self.perceived.push((b, PerceptionInfo { team: 0xff, ..Default::default() }));
            fresh.push(b);
        }
        let t = self.world_time(cx);
        let keys: Vec<BodyId> = self.perceived.iter().map(|(b, _)| *b).collect();
        for b in keys {
            let Some(i) = self.perceived_index(b) else { continue };
            if cx.bodies[b].gone {
                self.perceived.remove(i);
                continue;
            }
            let mut info = self.perceived[i].1;
            if PERCEPTION_UPDATE_INTERVAL <= t - info.update_time {
                self.update_perception_info(b, &mut info, cx);
                self.perceived[i].1 = info;
                if !info.sensed() {
                    self.perceived.remove(i);
                    if !fresh.contains(&b) {
                        self.perception_events.push((false, b));
                    }
                } else if fresh.contains(&b) {
                    self.perception_events.push((true, b));
                }
            }
        }
    }

    /// AMordhauAIController::OnCharacterDiedOrDestroyed rva=0x1508160 (bound to the character's OnCharacterDied /
    /// OnCharacterDestroyed): unbind both, and if the character is in the map: remove it, OnStoppedPerceivingCharacter
    pub fn on_character_died_or_destroyed(&mut self, b: BodyId) {
        if let Some(i) = self.perceived_index(b) {
            self.perceived.remove(i);
            self.perception_events.push((false, b));
        }
    }

    /// one perception frame: the death delegates of perceived characters, then the stand-in perception update
    pub fn update_perception(&mut self, cx: &mut Ctx) {
        let keys: Vec<BodyId> = self.perceived.iter().map(|(b, _)| *b).collect();
        for b in keys {
            if cx.bodies[b].is_dead && !self.died_seen.contains(&b) {
                self.died_seen.push(b);
                self.on_character_died_or_destroyed(b);
            }
        }
        self.died_seen.retain(|&b| cx.bodies[b].is_dead);
        let all: Vec<BodyId> = (0..cx.bodies.len()).filter(|&b| !cx.bodies[b].gone).collect();
        self.on_perception_updated(&all, cx);
    }

    /// PerceivedCharacters entries that are valid and sensed (bSight || bHearing || bDamage), map order
    fn perceived_characters(&self, cx: &Ctx) -> Vec<BodyId> {
        self.perceived.iter().filter(|(b, i)| !cx.bodies[*b].gone && i.sensed()).map(|(b, _)| *b).collect()
    }

    fn perceives(&self, b: BodyId, cx: &Ctx) -> bool {
        self.perceived_characters(cx).contains(&b)
    }

    /// GetPerceivedEnemies rva=0x14fd110 / GetPerceivedAllies rva=0x14fce70: a walk over PerceivedCharacters, keeping
    /// a perceived key, then by team:
    ///   enemies (disasm 0x1414fd2fa-0x1414fd30f): GameState bIsTeamMode (+0x6b0) == 0 -> every perceived character;
    ///            else only info.Team (+3) != our pawn's Team (AAdvancedCharacter +0x660)
    ///   allies  (disasm 0x1414fcf05 / 0x1414fd06a-0x1414fd075): empty unless bIsTeamMode; else info.Team == our Team
    pub fn perceived_enemies(&self, cx: &Ctx) -> Vec<BodyId> {
        let my = cx.bodies[self.body].team & 0xff;
        self.perceived_characters(cx)
            .into_iter()
            .filter(|&b| !self.team_mode || self.perception(b).unwrap().team != my)
            .collect()
    }

    pub fn perceived_allies(&self, cx: &Ctx) -> Vec<BodyId> {
        if !self.team_mode {
            return Vec::new();
        }
        let my = cx.bodies[self.body].team & 0xff;
        self.perceived_characters(cx).into_iter().filter(|&b| self.perception(b).unwrap().team == my).collect()
    }

    /// AMordhauAIController::GetClosestEnemy rva=0x14fa570 (Ghidra C + disassembly for the return paths):
    ///   1. ClosestEnemyOverride (+0x408) set, alive (+0x504 bIsDead clear) and not in ClosestEnemyIgnoreSet (+0x410) -> it
    ///   2. no pawn -> null. LastClosestEnemy (+0x3f4) gone or dead -> cleared, LastClosestEnemyChangedTime (+0x400) = 0
    ///   3. our pawn's motion is an UAttackMotion, or (ChangedTime + 1 > RealTimeSeconds and LastClosestEnemy not
    ///      ignored) -> LastClosestEnemy unchanged (re-evaluated at most once a second, never mid-attack)
    ///   4. otherwise, over the perceived characters: in team mode an ally whose controller is an AMordhauAIController
    ///      contributes its own LastClosestEnemy (when we perceive that one too) together with the ally's distance to
    ///      it; any other perceived character contributes itself. Each live, not ignored candidate gets an entry in
    ///      EnemyWithAllyCountMap (+1 when the ally is closer to it than we are) and the nearest one (squared distance
    ///      between root locations) is the closest enemy. If the closest has more allies on it than BehaviorProfile
    ///      IgnoreEnemiesWithAllyCount, the nearest entry with <= that many replaces it, unless it is 160000 (400 cm
    ///      squared) or more farther (bIsClosestEnemySaturated +0x3fc = true: keep the closest). ReallyCloseEnemyCached
    ///      (+0x3ec) = the result when its distance < 160000 else null; LastClosestEnemy = result (ChangedTime = now on
    ///      a change). Returns the result.
    pub fn closest_enemy(&mut self, cx: &mut Ctx) -> Option<BodyId> {
        if let Some(ov) = self.closest_enemy_override {
            if !cx.bodies[ov].is_dead && !self.closest_enemy_ignore.contains(&ov) {
                return Some(ov);
            }
        }
        if let Some(l) = self.last_closest_enemy {
            if cx.bodies[l].is_dead {
                self.last_closest_enemy = None;
                self.last_closest_enemy_changed_time = 0.0;
            }
        } else {
            self.last_closest_enemy_changed_time = 0.0;
        }
        let q = cx.q;
        let t = q(cx.now + self.real_time_offset);
        let me = &cx.bodies[self.body];
        if me.has_sys && me.pawn.motion.is_attack {
            return self.last_closest_enemy;
        }
        let ignored = |c: &Option<BodyId>, s: &Vec<BodyId>| c.map(|c| s.contains(&c)).unwrap_or(false);
        if q(self.last_closest_enemy_changed_time + K::CLOSEST_ENEMY_RECHECK_S) > t
            && !ignored(&self.last_closest_enemy, &self.closest_enemy_ignore)
        {
            return self.last_closest_enemy;
        }
        let my_team = me.team & 0xff;
        let my_loc = me.location;
        let mut counts: Vec<(BodyId, i64)> = Vec::new(); // EnemyWithAllyCountMap (insertion order)
        let mut best: Option<BodyId> = None;
        let mut best_d = f64::INFINITY;
        for b in self.perceived_characters(cx) {
            let mut cand = Some(b);
            let mut ally_d = f64::INFINITY;
            if self.team_mode && self.perception(b).unwrap().team == my_team {
                cand = None;
                if let Some(ac) = cx.bodies[b].ai.and_then(|i| cx.others.get(i)).and_then(|o| o.as_ref()) {
                    if let Some(ae) = ac.last_closest_enemy {
                        if self.perceives(ae, cx) {
                            cand = Some(ae);
                            ally_d = (cx.bodies[b].location - cx.bodies[ae].location).length_squared() as f64;
                        }
                    }
                }
            }
            let Some(cand) = cand else { continue };
            if cx.bodies[cand].is_dead || self.closest_enemy_ignore.contains(&cand) {
                continue;
            }
            if !counts.iter().any(|(c, _)| *c == cand) {
                counts.push((cand, 0));
            }
            let dd = (my_loc - cx.bodies[cand].location).length_squared() as f64;
            if ally_d < dd {
                counts.iter_mut().find(|(c, _)| *c == cand).unwrap().1 += 1;
            }
            if dd < best_d {
                best = Some(cand);
                best_d = dd;
            }
        }
        let max_allies = self.profile.p.ignore_enemies_with_ally_count;
        self.closest_enemy_saturated = false;
        let mut pick = best;
        let mut pick_d = best_d;
        let count_of = |e: BodyId| counts.iter().find(|(c, _)| *c == e).map(|x| x.1).unwrap_or(0);
        if let Some(b) = best {
            if count_of(b) > max_allies {
                pick = None;
                pick_d = f64::INFINITY;
                for &(e, n) in &counts {
                    if n <= max_allies {
                        let de = (my_loc - cx.bodies[e].location).length_squared() as f64;
                        if de < pick_d {
                            pick = Some(e);
                            pick_d = de;
                        }
                    }
                }
                if q(best_d + K::CLOSEST_ENEMY_SATURATION_SQ) <= pick_d {
                    self.closest_enemy_saturated = true;
                    pick = best;
                    pick_d = best_d;
                }
            }
        }
        self.really_close_enemy = if pick_d < K::CLOSEST_ENEMY_SATURATION_SQ { pick } else { None };
        if self.last_closest_enemy != pick {
            self.last_closest_enemy = pick;
            self.last_closest_enemy_changed_time = t;
        }
        pick
    }

    /// AMordhauAIController::GetClosestAlly rva=0x14fa3b0: the perceived ally (GetPerceivedAllies) nearest by squared
    /// root-location distance. Dead allies are not skipped (no bIsDead test).
    pub fn closest_ally(&self, cx: &Ctx) -> Option<BodyId> {
        let me = cx.bodies[self.body].location;
        let mut best = None;
        let mut bd = f64::INFINITY;
        for b in self.perceived_allies(cx) {
            let d = (cx.bodies[b].location - me).length_squared() as f64;
            if d < bd {
                bd = d;
                best = Some(b);
            }
        }
        best
    }

    /// AMordhauAIController::GetKthClosestOfThree rva=0x14fb780: logs "Call to unimplemented function
    /// GetKthClosestOfThree()" and returns null
    pub fn kth_closest_of_three(&self, _idx: i64) -> Option<BodyId> {
        None
    }

    /// AMordhauAIController::GetTeam rva=0x14fd6b0: the MordhauPlayerState's Team, else the AAdvancedCharacter pawn's
    /// Team byte (+0x660), else -2
    pub fn get_team(&self, cx: &Ctx) -> i64 {
        if let Some(t) = self.player_state_team {
            return t;
        }
        cx.bodies[self.body].team & 0xff
    }

    /// AMordhauAIController::PerceivesAlly rva=0x15122b0 / PerceivesEnemy rva=0x15122e0
    pub fn perceives_ally(&self, cx: &Ctx) -> bool {
        !self.perceived_allies(cx).is_empty()
    }
    pub fn perceives_enemy(&self, cx: &Ctx) -> bool {
        !self.perceived_enemies(cx).is_empty()
    }

    /// AMordhauAIController::GetAllyClearanceSides rva=0x14fa080: for each perceived ally (GetPerceivedAllies) within
    /// 300 cm in 2D (d^2 <= 90000, me - ally), the ally's offset in my root's frame (VectorQuaternionInverseRotateVector
    /// of ally - me by my ComponentToWorld rotation: QINV_SIGN_MASK conjugate, inlined) inside the box |Z| <= 100,
    /// -75 <= X <= 200, -200 <= Y <= 200 blocks the left side (Y <= 0) or the right side (Y > 0). Left clear only -> 1,
    /// right clear only -> 0, both or neither -> min(int(f32(rand() & 0x7fff) * 6.103702e-05), 1) == 1
    /// (_DAT_14432474c = 2/32767). No pawn -> 2 (not reachable here: a controller always has its body).
    pub fn ally_clearance_sides(&mut self, cx: &mut Ctx) -> i64 {
        let me = &cx.bodies[self.body];
        let (loc, yaw) = (me.location, me.yaw as f32);
        let q = mordhau_core::ue::FQuat::from_rotator(0.0, yaw, 0.0);
        let (qx, qy, qz, qw) = (-q.x, -q.y, -q.z, q.w);
        let (mut left, mut right) = (true, true);
        // the GDScript reference (Precision::Reference, the golden traces) has no box test: allies count as clear
        let allies = if cx.exe { self.perceived_allies(cx) } else { Vec::new() };
        for a in allies {
            let al = cx.bodies[a].location;
            let (dx, dy) = (loc.x - al.x, loc.y - al.y);
            if dy * dy + dx * dx > 90000.0 {
                continue;
            }
            let (vx, vy, vz) = (al.x - loc.x, al.y - loc.y, al.z - loc.z);
            let t0 = vz * qy - vy * qz;
            let t1 = vx * qz - vz * qx;
            let t2 = vy * qx - vx * qy;
            let (t0, t1, t2) = (t0 + t0, t1 + t1, t2 + t2);
            let x = (t2 * qy - t1 * qz) + t0 * qw + vx;
            let y = (t0 * qz - t2 * qx) + t1 * qw + vy;
            let z = (t1 * qx - t0 * qy) + t2 * qw + vz;
            if z.abs() <= 100.0 && x <= 200.0 && -75.0 <= x && y <= 200.0 && -200.0 <= y {
                if y <= 0.0 {
                    left = false;
                } else {
                    right = false;
                }
            }
        }
        if left && !right {
            return 1;
        }
        if !left && right {
            return 0;
        }
        let r = ((cx.rng.rand() as f64 * K::ALLY_CLEARANCE_RAND_SCALE) as i64).min(1);
        if r == 1 {
            1
        } else {
            0
        }
    }

    pub fn get_move_status(&self) -> PathStatus {
        self.path_status
    }

    pub fn currently_facing_actor(&self) -> Option<BodyId> {
        self.facing_actor
    }

    /// StartFacingMovement rva=0x1516640: mode 0, actor cleared, offset zero, +0x368 = arg
    pub fn start_facing_movement(&mut self, arg: f64) {
        self.facing_mode = Facing::Movement;
        self.facing_actor = None;
        self.facing_offset = [0.0, 0.0];
        self.facing_param = arg;
    }

    /// StartFacingLocation rva=0x15165c0: mode 1, actor cleared, +0x35c = location, offset zero, +0x368 = 0
    pub fn start_facing_location(&mut self, p: FVector) {
        self.facing_mode = Facing::Location;
        self.facing_actor = None;
        self.facing_location = p;
        self.facing_offset = [0.0, 0.0];
        self.facing_param = 0.0;
    }

    /// StartFacingActor rva=0x15164a0: mode 2, +0x368 = arg, +0x404 = now, actor, offset
    pub fn start_facing_actor(&mut self, a: BodyId, arg: f64, offset: [f32; 2], now: f64) {
        self.facing_mode = Facing::Actor;
        self.facing_actor = Some(a);
        self.facing_param = arg;
        self.facing_offset = offset;
        self.facing_since = now;
    }

    /// StartFacingBone rva=0x1516520: mode 4, actor = the mesh's owner, bone height, offset, +0x404 = now
    pub fn start_facing_bone(&mut self, a: BodyId, height: f64, offset: [f32; 2], now: f64) {
        self.facing_mode = Facing::Bone;
        self.facing_actor = Some(a);
        self.facing_param = height;
        self.facing_offset = offset;
        self.facing_since = now;
    }

    /// StartFacingActor2D rva=0x1516430: FacingUpOffset (+0x368) = offset, LastFacingActorChangeTime (+0x404) = world
    /// TimeSeconds, FacingActor = actor, FacingOffset zero, mode 3
    pub fn start_facing_actor_2d(&mut self, a: BodyId, up_offset: f64, now: f64) {
        self.facing_param = up_offset;
        self.facing_since = now;
        self.facing_actor = Some(a);
        self.facing_offset = [0.0, 0.0];
        self.facing_mode = Facing::Actor2D;
    }

    /// StopMovement rva=0x1516810: bMovePending (+0x5b8) = false, PathFollowingComponent->AbortMove (IDLE)
    pub fn stop_movement(&mut self) {
        self.move_pending = false;
        self.path_status = PathStatus::Idle;
    }

    /// PauseLogic rva=0x1512210 / ResumeLogic rva=0x1515100: forward to the BrainComponent's virtuals at vtable +0x428 /
    /// +0x430 (names UNCONFIRMED); the port pauses the behavior tree tick. No brain component (no tree) -> nothing.
    pub fn pause_logic(&mut self) {
        if self.tree.is_some() {
            self.logic_paused = true;
        }
    }
    pub fn resume_logic(&mut self) {
        if self.tree.is_some() {
            self.logic_paused = false;
        }
    }

    /// AAIController::MoveToLocation(dest, acceptance, ...): recorded for the host's path following
    pub fn move_to_location(&mut self, dest: FVector, acceptance: f64, now: f64) {
        self.move_request = Some(MoveRequest { dest, acceptance, t: now });
        self.path_status = PathStatus::Moving;
    }

    pub fn nav_blocked(&self, a: FVector, b: FVector, cx: &mut Ctx) -> bool {
        cx.host.nav_raycast(a, b)
    }

    // ---- RandomFloat (+0x468) / NextRandomFloatAssignment (+0x46c) -------------------------------------------------
    /// AMordhauAIController::BeginPlay rva=0x14f0ba0: RandomFloat = FRand (rand() & 0x7fff) * (1/32767), then
    /// NextRandomFloatAssignment = rand * (60/32767) + 120 + TimeSeconds.
    pub fn begin_play(&mut self, cx: &mut Ctx) {
        let q = cx.q;
        self.random_float = q((cx.rng.rand() & 0x7fff) as f64 * K::FRAND_SCALE);
        let r = (cx.rng.rand() & 0x7fff) as f64;
        self.next_random_float_assignment = q(q(q(r * K::RANDOM_FLOAT_NEXT_SCALE) + K::RANDOM_FLOAT_FIRST_DELAY) + self.world_time(cx));
    }

    /// AMordhauAIController::LODTick rva=0x14fe7c0, first thing: when Next < TimeSeconds (strictly): RandomFloat = FRand,
    /// Next = rand * (60/32767) + 60 + TimeSeconds (LODTick's call site and LOD gating not ported: UNCONFIRMED every frame)
    fn random_float_tick(&mut self, cx: &mut Ctx) {
        let t = self.world_time(cx);
        if self.next_random_float_assignment <= t && t != self.next_random_float_assignment {
            let q = cx.q;
            self.random_float = q((cx.rng.rand() & 0x7fff) as f64 * K::FRAND_SCALE);
            let r = (cx.rng.rand() & 0x7fff) as f64;
            self.next_random_float_assignment = q(q(q(r * K::RANDOM_FLOAT_NEXT_SCALE) + K::RANDOM_FLOAT_DELAY) + t);
        }
    }

    // ---- moves with a random midpoint ------------------------------------------------------------------------------
    pub fn random_reachable_point(&self, origin: FVector, radius: f64, cx: &mut Ctx) -> Option<FVector> {
        if cx.host.has_nav() {
            cx.host.random_reachable_point(origin, radius)
        } else {
            None
        }
    }

    /// AMordhauAIController::GetMoveMidpoint_Implementation rva=0x14fc710: the pawn-goal midpoint, then a random
    /// reachable point within half the pawn-midpoint distance of it; no navigation system / no point -> the goal
    pub fn get_move_midpoint(&self, goal: FVector, cx: &mut Ctx) -> FVector {
        let here = cx.bodies[self.body].location;
        let mid = (goal - here).scale(K::MOVE_MIDPOINT_HALF) + here;
        let r = (here - mid).length() as f64 * K::MOVE_MIDPOINT_HALF;
        if let Some(p) = self.random_reachable_point(mid, r, cx) {
            return p;
        }
        goal
    }

    /// AMordhauAIController::MoveToLocationWithRandomMidpoint rva=0x14ffea0: abort the current move; build the query
    /// with a midpoint (BuildPathfindingQueryWithMidpoint rva=0x14f1be0, not ported: taken as found when a navigation
    /// system is set, UNCONFIRMED); none -> bMovePending = false and a plain MoveTo(dest); else bMovePending = true,
    /// PendingReq = the goal request, and MoveTo(midpoint) with MidPointAcceptanceRadius. Returns the request result
    /// (EPathFollowingRequestResult: 0 Failed, 1 AlreadyAtGoal, 2 RequestSuccessful; the port's moves always succeed).
    pub fn move_to_location_with_random_midpoint(&mut self, dest: FVector, acceptance: f64, cx: &mut Ctx) -> i64 {
        if self.path_status != PathStatus::Idle {
            self.path_status = PathStatus::Idle; // PathFollowingComponent->AbortMove
        }
        if !cx.host.has_nav() {
            self.move_pending = false;
            self.move_to_location(dest, acceptance, cx.now);
            return 2;
        }
        let mid = self.get_move_midpoint(dest, cx);
        self.move_pending = true;
        self.pending_move = Some((dest, acceptance));
        self.move_to_location(mid, MID_POINT_ACCEPTANCE_RADIUS, cx.now);
        2
    }

    /// AMordhauAIController::OnMoveCompleted rva=0x150cd90: the base OnMoveCompleted, then a pending goal request is
    /// issued (bMovePending = false, MoveTo(PendingReq)). The host's path following calls this when it arrives.
    pub fn on_move_completed(&mut self, now: f64) {
        self.path_status = PathStatus::Idle;
        if self.move_pending {
            self.move_pending = false;
            if let Some((d, a)) = self.pending_move {
                self.move_to_location(d, a, now);
            }
        }
    }

    /// AMordhauAIController::CanSee rva=0x14f2530 (return paths from the disassembly, 0x1414f2976 / 0x1414f297a):
    /// trace (channel 0x12, ignoring the pawn and its two held items) from the pawn's eyes toward the target's root,
    /// `distance` long; the first hit actor being the target -> true; otherwise a second trace to that end point
    /// raised 100 cm (UpVector * 100); its hit being the target -> true; else false. distance < 0: an AMordhauActor
    /// target's +0x364 + 50 (not ported: such targets are interactables).
    pub fn can_see(&self, target: BodyId, distance: f64, cx: &mut Ctx) -> bool {
        if distance < 0.0 {
            return false;
        }
        let me = &cx.bodies[self.body];
        let eye = me.location + FVector::new(0.0, 0.0, me.eye_height as f32);
        let dir = super::tasks::safe_normal(cx.bodies[target].location - eye);
        let end = eye + dir.scale(distance);
        match cx.host.trace_first_hit(eye, end, self.body) {
            None => false,
            Some(h) => {
                if h == Some(target) {
                    return true;
                }
                let end2 = end + FVector::new(0.0, 0.0, K::CAN_SEE_RAISE as f32);
                cx.host.trace_first_hit(eye, end2, self.body).flatten() == Some(target)
            }
        }
    }

    // ---- the pawn (AMordhauCharacter) ------------------------------------------------------------------------------
    fn apply(&self, v: PawnView, cx: &mut Ctx) {
        cx.bodies[self.body].pawn = v;
    }

    /// AMordhauCharacter::RequestAttack rva=0x15644c0 (EAttackMove, float angle) -> the fighter's input entry point
    pub fn request_attack(&mut self, mv: i64, angle: f64, cx: &mut Ctx) {
        self.last_request = Some(Request::Attack { t: cx.now, mv, angle });
        let v = cx.host.request_attack(self.body, mv, angle);
        self.apply(v, cx);
    }

    /// AMordhauCharacter::RequestParry rva=0x1564860 (EBlockType, bool). The bool (always 1 from the bot tasks) is not
    /// modelled by the combat port.
    pub fn request_parry(&mut self, bt: i64, cx: &mut Ctx) {
        self.last_request = Some(Request::Parry { t: cx.now, bt });
        let v = cx.host.request_parry(self.body, bt);
        self.apply(v, cx);
    }

    /// AMordhauCharacter::RequestFeint rva=0x15646b0
    pub fn request_feint(&mut self, cx: &mut Ctx) {
        self.last_request = Some(Request::Feint { t: cx.now });
        let v = cx.host.request_feint(self.body);
        self.apply(v, cx);
    }

    pub fn start_sprinting(&self, cx: &mut Ctx) {
        cx.bodies[self.body].wants_sprint = true;
    }
    pub fn start_crouching(&self, cx: &mut Ctx) {
        cx.bodies[self.body].wants_crouch = true;
    }
    pub fn stop_crouching(&self, cx: &mut Ctx) {
        cx.bodies[self.body].wants_crouch = false;
    }

    /// AMordhauAIController::LODTick rva=0x14fe7c0, the turning part: the bot's (yaw, pitch) input for this frame
    /// (AddControllerYawInput vfptr+0xa10 / AddControllerPitchInput vfptr+0xa00 on the character; the host turns the
    /// pawn by them). None for a dead character, a facing actor that is gone, or facing movement with no path
    /// direction. `move_dir` = UPathFollowingComponent::GetCurrentDirection (the host's path following).
    ///   view = the root's ComponentToWorld (location, rotation (0, yaw, 0)); the first-person camera branch and
    ///     UAdvancedCharacterMovement's yaw term are not modelled: UNCONFIRMED for bots (taken absent / 0).
    ///   target: Location -> FacingLocation; Actor / Actor2D -> the actor's root location; Bone -> the bone (here:
    ///     body location + facing_bone_height, UNCONFIRMED); Movement -> view + dir * 200 (every |dir component| <=
    ///     1e-4 -> None); Actor2D and Movement use the view's Z. d = (target + (0, 0, FacingUpOffset)) - view,
    ///     normalized (== 1 kept, >= 1e-8 -> rsqrtss + 2 Newton steps, else zero); FRotationMatrix::MakeFromXZ(d,
    ///     Up).Rotator(); RotationTargetInterpolated += Normalize(target - RTI) * min(dt / RotationInterpolationTime
    ///     (0.1, ctor: decomp AMordhauAIController.cpp:1984), 1).
    ///   input: FMath::GetAzimuthAndElevation(RTI.Vector(), the view quaternion's X / Y / Z axes) * 57.295776 +
    ///     FacingOffset, each clamped to +-dt * MaxTurnRate / MaxLookUpRate when |.| > 1e-8 (else no input).
    pub fn lod_turn(&mut self, cx: &Ctx, dt: f32, move_dir: Option<FVector>) -> Option<(f32, f32)> {
        use mordhau_core::ue::{ue_inv_sqrt, ue_safe_normal, FQuat};
        let me = &cx.bodies[self.body];
        if me.is_dead {
            return None;
        }
        let view = me.location;
        let q = FQuat::from_rotator(0.0, me.yaw as f32, 0.0);
        let mut t = match self.facing_mode {
            Facing::Actor | Facing::Actor2D => cx.bodies[self.facing_actor?].location,
            Facing::Bone => {
                let b = &cx.bodies[self.facing_actor?];
                FVector::new(b.location.x, b.location.y, b.location.z + b.facing_bone_height as f32)
            }
            Facing::Movement => {
                let d = move_dir?;
                if d.x.abs() <= 1e-4 && d.y.abs() <= 1e-4 && d.z.abs() <= 1e-4 {
                    return None;
                }
                FVector::new(d.x * 200.0 + view.x, d.y * 200.0 + view.y, d.z * 200.0 + view.z)
            }
            Facing::Location => self.facing_location,
        };
        if matches!(self.facing_mode, Facing::Actor2D | Facing::Movement) {
            t.z = view.z;
        }
        let (dx, dy, dz) = (t.x - view.x, t.y - view.y, (t.z + self.facing_param as f32) - view.z);
        let ss = dy * dy + dx * dx + dz * dz;
        let d = if ss == 1.0 {
            FVector::new(dx, dy, dz)
        } else if ss >= 1e-8 {
            let r = ue_inv_sqrt(ss);
            FVector::new(r * dx, r * dy, dz * r)
        } else {
            FVector::ZERO
        };
        let target = make_from_xz_rotator(d);
        let alpha = (dt / 0.1f32).min(1.0);
        let rti = &mut self.rotation_target_interpolated;
        for k in 0..3 {
            // FRotator::GetNormalized of the difference, the SSE sequence: x - trunc(x / 360) * 360 (|x/360| >=
            // 2^23 kept), clamped to [-360, 360], < 0 -> + 360, > 180 -> - 360
            let mut x = target[k] - rti[k];
            let qd = x / 360.0;
            let tr = if qd.abs() >= 8388608.0 { qd } else { qd.trunc() };
            x -= tr * 360.0;
            x = x.min(360.0).max(-360.0);
            if x < 0.0 {
                x += 360.0;
            }
            if x > 180.0 {
                x -= 360.0;
            }
            rti[k] = x * alpha + rti[k];
        }
        let dir = mordhau_core::ue::rotator_vector(rti[0], rti[1]);
        // the view quaternion's axes, as LODTick inlines them
        let (x2, y2, z2) = (q.x + q.x, q.y + q.y, q.z + q.z);
        let ax = FVector::new((q.y * -y2 - q.z * z2) + 1.0, q.w * z2 - q.x * -y2, q.x * z2 + q.w * -y2);
        let ay = FVector::new(q.y * x2 + q.w * -z2, (q.z * -z2 - q.x * x2) + 1.0, q.w * x2 - q.y * -z2);
        let az = FVector::new(q.w * y2 - q.z * -x2, q.z * y2 + q.w * -x2, (q.x * -x2 - q.y * y2) + 1.0);
        // FMath::GetAzimuthAndElevation (UE 4.26 UnrealMath.cpp; the call's own instruction order: UNCONFIRMED)
        let n = ue_safe_normal(dir);
        let nz = n.dot(az);
        let p = ue_safe_normal(FVector::new(n.x - nz * az.x, n.y - nz * az.y, n.z - nz * az.z));
        let sign = if p.dot(ay) < 0.0 { -1.0 } else { 1.0 };
        let azimuth = p.dot(ax).clamp(-1.0, 1.0).acos() * sign;
        let elevation = nz.clamp(-1.0, 1.0).asin();
        let mut yi = azimuth * 57.295776 + self.facing_offset[0];
        let mut pi = elevation * 57.295776 + self.facing_offset[1];
        let prof = &self.profile.p;
        if yi.abs() > 1e-8 {
            let m = dt * prof.max_turn_rate as f32;
            yi = if -m <= yi { if m <= yi { m } else { yi } } else { -m };
        } else {
            yi = 0.0;
        }
        if pi.abs() > 1e-8 {
            let m = dt * prof.max_look_up_rate as f32;
            pi = if -m <= pi { if pi <= m { pi } else { m } } else { -m };
        } else {
            pi = 0.0;
        }
        Some((yi, pi))
    }

    /// Where the host should turn the bot (AMordhauAIController::LODTick rva=0x14fe7c0 does this in the game; it is not
    /// ported, so this is a plain reading of the facing state: UNCONFIRMED). A UE-space point, or None to face the
    /// movement direction.
    pub fn facing_target(&self, bodies: &[BotBody]) -> Option<FVector> {
        match self.facing_mode {
            Facing::Location => Some(self.facing_location),
            Facing::Actor | Facing::Actor2D | Facing::Bone => self.facing_actor.map(|a| {
                let h = if self.facing_mode == Facing::Bone { self.facing_param } else { 0.0 };
                bodies[a].location + FVector::new(0.0, 0.0, h as f32)
            }),
            Facing::Movement => None,
        }
    }
}

/// the bots of one world (godot/game/ai/bot_duel.gd BotDuel, without the combat step: the host steps its fighters,
/// refreshes every body's PawnView, then calls tick). Order inside a frame: UE ticks the AI controller / BT component
/// and the character's motion component in tick groups whose relative order is not taken from decompiled code
/// (UNCONFIRMED).
#[derive(Clone, Debug, Default)]
pub struct Bots {
    pub bodies: Vec<BotBody>,
    pub bots: Vec<Option<BotController>>,
    /// one CRT rand() stream for the whole world (ue_rand.gd header)
    pub rng: CrtRand,
    pub ws: WorldState,
    /// GameState bIsTeamMode for every bot's perception
    pub team_mode: bool,
    pub k: Arc<Kismet>,
    /// the numeric model (crate::game_mode::Precision; default Exe)
    pub precision: Precision,
}

impl Bots {
    pub fn add_body(&mut self, b: BotBody) -> BodyId {
        self.bodies.push(b);
        self.bodies.len() - 1
    }

    /// BotDuel.add_bot -> MordhauBotController.make (profile copy, tree, BeginPlay) then every bot's perception sees
    /// the bodies present now (a fresh entry refreshes at once)
    pub fn add_bot(&mut self, body: BodyId, profile: Profile, tree: Option<&TreeDef>, now: f64, host: &mut dyn BotHost) -> usize {
        let mut c = BotController::new(body, BehaviorProfile::new(profile), tree.map(BtTree::new));
        c.team_mode = self.team_mode;
        let idx = self.bots.len();
        self.bodies[body].ai = Some(idx);
        {
            let mut cx = Ctx { bodies: &mut self.bodies, others: &self.bots, rng: &mut self.rng, ws: &mut self.ws, host, k: &self.k, now, q: self.precision.q(), exe: self.precision == Precision::Exe };
            c.begin_play(&mut cx);
        }
        self.bots.push(Some(c));
        for i in 0..self.bots.len() {
            self.with_bot(i, now, host, |c, cx| c.update_perception(cx));
        }
        idx
    }

    /// the bot BeginPlay would make next (bot_profiles.rs): the world stream after the controller's two RandomFloat
    /// draws, without consuming it. The host spawns the pawn with `weapons` before calling add_bot_from_roster.
    pub fn preview_bot_choice(&self, roster: &super::bot_profiles::BotRoster) -> Option<super::bot_profiles::BotChoice> {
        let mut r = self.rng.clone();
        r.rand();
        r.rand();
        roster.choose(&mut r)
    }

    /// AMordhauAIController::BeginPlay rva=0x14f0ba0 with the game's bot roster: RandomFloat / Next draws, then the
    /// BotProfile pick, loadout and voice draws (bot_profiles.rs), then the behaviour: the BehaviorProfile class's
    /// defaults (`profile_of`, the spec's ENT_BOT_* record) or a plain UBotBehaviorProfile (`profile_of("")`) +
    /// Randomize rva=0x149c190. Returns the bot index and what was chosen.
    pub fn add_bot_from_roster(
        &mut self,
        body: BodyId,
        roster: &super::bot_profiles::BotRoster,
        profile_of: &dyn Fn(&str) -> Option<Profile>,
        tree: Option<&TreeDef>,
        now: f64,
        host: &mut dyn BotHost,
    ) -> Option<(usize, super::bot_profiles::BotChoice)> {
        let mut c = BotController::new(body, BehaviorProfile::new(Profile::default()), tree.map(BtTree::new));
        c.team_mode = self.team_mode;
        let idx = self.bots.len();
        self.bodies[body].ai = Some(idx);
        let choice;
        {
            let mut cx = Ctx { bodies: &mut self.bodies, others: &self.bots, rng: &mut self.rng, ws: &mut self.ws, host, k: &self.k, now, q: self.precision.q(), exe: self.precision == Precision::Exe };
            c.begin_play(&mut cx);
            choice = roster.choose(cx.rng)?;
            if choice.randomize_behavior {
                let mut b = BehaviorProfile::new(profile_of("").unwrap_or_default());
                b.randomize(cx.rng);
                c.profile = b;
            } else {
                c.profile = BehaviorProfile::new(profile_of(&choice.behavior)?);
            }
        }
        self.bots.push(Some(c));
        for i in 0..self.bots.len() {
            self.with_bot(i, now, host, |c, cx| c.update_perception(cx));
        }
        Some((idx, choice))
    }

    /// UAISense_Hearing for one noise (AMordhauCharacter::PlayCharacterSound rva=0x155dac0 ->
    /// UAISense_Hearing::ReportNoiseEvent(this, root location, Loudness = Sound->GetVolumeMultiplier() (vtable +0x2a0:
    /// USoundCue's slot, read from the exe's vtable), Instigator = this, MaxRange 0, Tag None)): every bot whose pawn is
    /// not the instigator hears it when |noise - eyes|^2 <= (HearingRange 2000 * Loudness)^2 (and within MaxRange when
    /// > 0); every affiliation is detected (HearingConfig +0x5c bits 1|2|4, ctor). The engine's processing (UE 4.26
    /// AISense_Hearing.cpp, not disassembled) is UNCONFIRMED in detail; the stimulus (strength 1) is aged by
    /// UpdatePerceptionInfo against HearingConfig MaxAge 15.
    pub fn report_noise(&mut self, instigator: BodyId, location: FVector, loudness: f32, max_range: f32, now: f64) {
        for c in self.bots.iter_mut().flatten() {
            if c.body == instigator {
                continue;
            }
            let me = &self.bodies[c.body];
            let eye = FVector::new(me.location.x, me.location.y, me.location.z + me.eye_height as f32);
            let d = FVector::new(location.x - eye.x, location.y - eye.y, location.z - eye.z);
            let d2 = d.x * d.x + d.y * d.y + d.z * d.z;
            let r = HEARING_RANGE * loudness;
            if d2 > r * r || (max_range > 0.0 && d2 > max_range * max_range) {
                continue;
            }
            let t = (if self.precision == Precision::Exe { mordhau_core::ue::f32r } else { |x| x })(now + c.real_time_offset);
            match c.heard.iter_mut().find(|h| h.0 == instigator) {
                Some(h) => {
                    h.1 = t;
                    h.2 = location;
                }
                None => c.heard.push((instigator, t, location)),
            }
        }
    }

    /// LODTick's turning for bot i (BotController::lod_turn): the (yaw, pitch) input of this frame
    pub fn lod_turn(&mut self, i: usize, dt: f32, move_dir: Option<FVector>, now: f64, host: &mut dyn BotHost) -> Option<(f32, f32)> {
        self.with_bot(i, now, host, |c, cx| c.lod_turn(cx, dt, move_dir))
    }

    /// run `f` on bot i with the world as its context (the bot is taken out of the list meanwhile)
    pub fn with_bot<R>(&mut self, i: usize, now: f64, host: &mut dyn BotHost, f: impl FnOnce(&mut BotController, &mut Ctx) -> R) -> R {
        let mut c = self.bots[i].take().expect("bot is ticking");
        let r = {
            let mut cx = Ctx { bodies: &mut self.bodies, others: &self.bots, rng: &mut self.rng, ws: &mut self.ws, host, k: &self.k, now, q: self.precision.q(), exe: self.precision == Precision::Exe };
            f(&mut c, &mut cx)
        };
        self.bots[i] = Some(c);
        r
    }

    /// every bot's AI frame at `now`, reading the fighters as they stand after the host's combat step
    pub fn tick(&mut self, dt: f64, now: f64, host: &mut dyn BotHost) {
        for i in 0..self.bots.len() {
            self.with_bot(i, now, host, |c, cx| c.tick(dt, cx));
        }
    }

    pub fn bot(&self, i: usize) -> &BotController {
        self.bots[i].as_ref().unwrap()
    }
}

/// FRotationMatrix::MakeFromXZ(X, FVector::UpVector).Rotator() as (pitch, yaw, roll): NewX = X.GetSafeNormal, Norm =
/// Up; |NewX . Norm| within KINDA_SMALL of 1 -> Norm = Up if |NewX.Z| < 1 - KINDA_SMALL else (1, 0, 0); NewY = (Norm ^
/// NewX).GetSafeNormal, NewZ = NewX ^ NewY; FMatrix::Rotator rva=0x18bdb00: Pitch = Atan2(X.Z, Sqrt(X.X^2 + X.Y^2)),
/// Yaw = Atan2(X.Y, X.X), Roll = Atan2(Z . SYAxis, Y . SYAxis), SYAxis = the Y axis of FRotationMatrix(Pitch, Yaw, 0)
/// (UE 4.26 source; MakeFromXZ is a call in LODTick, its instruction order UNCONFIRMED)
pub fn make_from_xz_rotator(x: FVector) -> [f32; 3] {
    use mordhau_core::ue::{ue_atan2, ue_safe_normal, ue_yaw_axes, UE_KINDA_SMALL_NUMBER, UE_RAD_TO_DEG};
    let nx = ue_safe_normal(x);
    let mut norm = FVector::new(0.0, 0.0, 1.0);
    if (nx.dot(norm).abs() - 1.0).abs() <= UE_KINDA_SMALL_NUMBER {
        norm = if nx.z.abs() < 1.0 - UE_KINDA_SMALL_NUMBER { FVector::new(0.0, 0.0, 1.0) } else { FVector::new(1.0, 0.0, 0.0) };
    }
    let cr = |a: FVector, b: FVector| FVector::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x);
    let ny = ue_safe_normal(cr(norm, nx));
    let nz = cr(nx, ny);
    let pitch = ue_atan2(nx.z, (nx.x * nx.x + nx.y * nx.y).sqrt()) * UE_RAD_TO_DEG;
    let yaw = ue_atan2(nx.y, nx.x) * UE_RAD_TO_DEG;
    let (_, sy_axis) = ue_yaw_axes(yaw);
    let roll = ue_atan2(nz.dot(sy_axis), ny.dot(sy_axis)) * UE_RAD_TO_DEG;
    [pitch, yaw, roll]
}
